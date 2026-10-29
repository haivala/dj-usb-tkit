// One data layer shared by every track-list view (library, app playlist, USB
// playlist, USB history): fetch + pagination + search + sort + windowed
// rendering + in-place patching. Row building (`buildRow`), the duration
// footer, sorting (column headers) and row-action dispatch are shared too.
//
// Every backing command returns the same envelope:
//   { items, total, totalDurationMs?, durationUnknownCount? }
// for `query` / `sortBy` / `sortDir` / `offset` / `limit` request params, so
// filtering and sorting run server-side over the whole list. Changing the
// search text or the column sort reloads from the top.
//
// The list is positional: `items` has one slot per track in the whole list
// (`total`), and a slot stays empty until its page is loaded. Pages are
// fetched by offset for whatever part of the list is on screen, so dragging
// the scrollbar anywhere loads that part directly. Only the rows around the
// viewport are in the DOM; padding on the body stands in for the rest, so the
// scrollbar is sized and placed for the whole list from the start.
//
// A view supplies a small config; the controller owns all the mechanics.

const DEFAULT_PAGE_SIZE = 150;
// Rows built beyond each edge of the viewport, so a short scroll shows rows
// that are already there.
const OVERSCAN_ROWS = 20;
// Rows built while the scroller has no size (its panel is hidden); the resize
// observer builds the real window once it's shown.
const HIDDEN_WINDOW_ROWS = 60;
// The pages under the viewport are fetched once scrolling has paused this
// long, so dragging the scrollbar across a long list doesn't fetch every page
// it passes.
const FETCH_SETTLE_MS = 100;
// A fetch that answers within this long never shows the loading indicator, so
// fast views (local library, app playlists) don't flash it on every search
// keystroke / sort click. Slow ones (USB pages hydrating from the stick) do.
const LOADING_INDICATOR_DELAY_MS = 200;
// Until the CSS row height can be read (no layout, e.g. unit tests).
const FALLBACK_ROW_HEIGHT_PX = 47;

export function createTrackListController(config = {}) {
  const {
    bodyId,
    pageSize = DEFAULT_PAGE_SIZE,
    // () => ({ body, wrap, durationTarget }) -- resolved lazily so the
    // controller can be built before the DOM elements exist. `wrap` is the
    // element that scrolls the table.
    getElements = () => ({}),
    // async ({ scopeId, query, sortBy, sortDir, offset, limit }) -> envelope
    fetchPage,
    normalize = (track) => track,
    // () => buildRow options for this view
    rowOptions = () => ({}),
    // (track | undefined, { ...rowOptions, index }) -> row element; a
    // skeleton row for an empty slot.
    buildRow = () => null,
    // (body) -> void  -- the "no tracks" table
    renderEmpty = () => {},
    // (rows) -> Promise  -- wire up rows just put in the DOM
    finishRows = async () => {},
    // (target, { totalDurationMs, durationUnknownCount }) -> void
    renderDurationSummary = () => {},
    // (pageTracks, { first }) -> void  (e.g. library kicks off bg preview hydration)
    onPage = () => {},
    // (envelope) -> void  (e.g. the library's track-count heading)
    onResponse = () => {},
    // (scopeId) -> void  -- the shown list changed (load with a new scopeId,
    // or clear); e.g. the USB playlist view's heading.
    onScopeChange = () => {},
    // (error) -> void  -- a page fetched for scrolling failed
    onError = (err) => console.warn(`${bodyId} page load failed:`, err),
    // () => tableSortState  (the shell's per-body { key, dir } sort map)
    getTableSortState = () => ({}),
    // Optional external backing for `ctl.items` -- the library and app playlist
    // point these at `state.tracks` / `playlist.tracks` so playback resolution
    // / analysis patching / selection, which read those, stay consistent.
    // Default: internal array.
    getItems = null,
    setItems = null,
  } = config;

  let internalItems = [];
  const readItems = getItems || (() => internalItems);
  const writeItems = setItems || ((value) => { internalItems = value; });

  const ctl = {
    bodyId,
    scopeId: null,
    total: 0,
    // A load from the top (new list, search, sort, refresh) is in flight.
    loading: false,
    query: "",
    sortBy: null,
    sortDir: null,
    // Bumped on every fresh load; an in-flight response for a superseded seq
    // is dropped.
    seq: 0,
    _scrollBound: false,
  };

  Object.defineProperty(ctl, "items", {
    get: readItems,
    set: writeItems,
    enumerable: true,
  });

  // What the table shows / row actions resolve against: positional, so a
  // row's `data-index` is its slot.
  Object.defineProperty(ctl, "view", {
    get: readItems,
    enumerable: true,
  });

  // The rendered window: rows for slots [start, end), keyed by slot, with the
  // track each was built from (a different track object in the slot means
  // the row is stale -- e.g. a skeleton whose page has arrived).
  let win = { start: 0, end: 0 };
  const rowsBySlot = new Map();
  // Page numbers (offset / pageSize) being fetched for scrolling.
  const inFlight = new Set();
  // While a drag-reorder moves rows itself, the window only grows: no row is
  // dropped or rebuilt under the drag. Afterwards the DOM order may not match
  // the slots until it's put back.
  let frozen = false;
  let rebuildWhenThawed = false;
  let orderDirty = false;
  let frameHandle = null;
  let fetchTimer = null;

  // Loading state: `data-loading` + `aria-busy` on the body. "page" hides
  // the rows and shows the `.track-grid-loading` element that follows the
  // body (a new list replaces them); "more" keeps them (pages loading for the
  // rows on screen, which show as skeletons meanwhile).
  let freshLoadMode = null;
  let loadingTimer = null;
  function setLoadingIndicator(mode) {
    const { body } = getElements();
    if (!body?.dataset) return;
    if (mode) {
      body.dataset.loading = mode;
      body.setAttribute("aria-busy", "true");
    } else {
      delete body.dataset.loading;
      body.removeAttribute("aria-busy");
    }
  }
  function currentLoadingMode() {
    return freshLoadMode || (inFlight.size ? "more" : null);
  }
  function updateLoadingIndicator({ immediate = false } = {}) {
    const mode = currentLoadingMode();
    const shown = !!getElements().body?.dataset?.loading;
    if (!mode || immediate || shown) {
      clearTimeout(loadingTimer);
      loadingTimer = null;
      setLoadingIndicator(mode);
      return;
    }
    if (loadingTimer) return;
    loadingTimer = setTimeout(() => {
      loadingTimer = null;
      setLoadingIndicator(currentLoadingMode());
    }, LOADING_INDICATOR_DELAY_MS);
  }

  function rowHeight() {
    const { body } = getElements();
    const view = body?.ownerDocument?.defaultView;
    const css = view?.getComputedStyle
      ? parseFloat(view.getComputedStyle(body).getPropertyValue("--track-row-height"))
      : NaN;
    return css > 0 ? css : FALLBACK_ROW_HEIGHT_PX;
  }

  // The first slot at (or above) the top of the scroller's viewport.
  function firstVisibleSlot() {
    const { body, wrap } = getElements();
    if (!wrap?.clientHeight || !body?.getBoundingClientRect) return win.start;
    const top = body.getBoundingClientRect().top - wrap.getBoundingClientRect().top;
    return Math.max(0, Math.floor(-top / rowHeight()));
  }

  function visibleWindow() {
    const { body, wrap } = getElements();
    const total = ctl.total;
    if (!wrap?.clientHeight || !body?.getBoundingClientRect) {
      const start = Math.max(0, Math.min(win.start, total - HIDDEN_WINDOW_ROWS));
      return { start, end: Math.min(total, start + HIDDEN_WINDOW_ROWS) };
    }
    const h = rowHeight();
    const top = body.getBoundingClientRect().top - wrap.getBoundingClientRect().top;
    const first = Math.floor(-top / h) - OVERSCAN_ROWS;
    const last = Math.ceil((wrap.clientHeight - top) / h) + OVERSCAN_ROWS;
    const start = Math.max(0, Math.min(first, total));
    return { start, end: Math.max(start, Math.min(last, total)) };
  }

  // Bring the DOM in line with the viewport: drop rows that left the window,
  // build the ones that entered it (a skeleton for an empty slot), rebuild
  // stale ones. `rebuild` rebuilds every row (the loaded tracks changed in
  // place).
  function renderWindow({ rebuild = false } = {}) {
    const { body } = getElements();
    if (!body) return Promise.resolve();
    const items = readItems();
    const total = ctl.total;
    if (!total) {
      rowsBySlot.clear();
      win = { start: 0, end: 0 };
      setPadding(body, 0, 0);
      renderEmpty(body);
      return Promise.resolve();
    }
    let { start, end } = visibleWindow();
    if (frozen) {
      if (rowsBySlot.size) {
        start = Math.min(start, win.start);
        end = Math.max(end, win.end);
      }
      rebuildWhenThawed ||= rebuild;
      rebuild = false;
    }
    if (!rowsBySlot.size) {
      body.replaceChildren(); // the empty-state row, if any
    } else if (orderDirty && !frozen) {
      for (const slot of [...rowsBySlot.keys()].sort((a, b) => a - b)) body.append(rowsBySlot.get(slot).row);
      orderDirty = false;
    }

    let options = null;
    const build = (slot) => buildRow(items[slot], { ...(options ||= rowOptions()), index: slot });
    const built = [];
    for (const [slot, entry] of rowsBySlot) {
      if (entry.row.parentNode !== body) {
        // Removed from outside: built again below.
        rowsBySlot.delete(slot);
        continue;
      }
      if (!frozen && (slot < start || slot >= end)) {
        entry.row.remove();
        rowsBySlot.delete(slot);
        continue;
      }
      const track = items[slot];
      // During a drag only skeletons are swapped: the dragged row must stay.
      const stale = frozen ? (!entry.track && !!track) : (rebuild || entry.track !== track);
      if (!stale) continue;
      const row = build(slot);
      entry.row.replaceWith(row);
      rowsBySlot.set(slot, { row, track });
      built.push(row);
    }
    let next = null;
    for (let slot = end - 1; slot >= start; slot -= 1) {
      let entry = rowsBySlot.get(slot);
      if (!entry) {
        entry = { row: build(slot), track: items[slot] };
        body.insertBefore(entry.row, next);
        rowsBySlot.set(slot, entry);
        built.push(entry.row);
      }
      next = entry.row;
    }
    win = { start, end };
    const h = rowHeight();
    setPadding(body, start * h, (total - end) * h);
    return built.length ? finishRows(built) : Promise.resolve();
  }

  function setPadding(body, before, after) {
    body.style?.setProperty?.("--track-grid-before", `${before}px`);
    body.style?.setProperty?.("--track-grid-after", `${after}px`);
  }

  // Drop every row. The body keeps the whole list's height, so the browser
  // doesn't clamp the scroll position before the new window is laid out.
  function resetRows() {
    rowsBySlot.clear();
    win = { start: 0, end: 0 };
    orderDirty = false;
    const { body } = getElements();
    body?.replaceChildren?.();
    if (body) setPadding(body, 0, ctl.total * rowHeight());
  }

  function requestParams(offset, limit) {
    return {
      scopeId: ctl.scopeId,
      query: ctl.query,
      sortBy: ctl.sortBy,
      sortDir: ctl.sortDir,
      offset,
      limit,
    };
  }

  function storePage(offset, rawItems) {
    const items = readItems();
    const page = (rawItems || []).map((item) => normalize(item));
    page.forEach((track, i) => { items[offset + i] = track; });
    return page;
  }

  function renderSummary(data) {
    renderDurationSummary(getElements().durationTarget, {
      totalDurationMs: data.totalDurationMs,
      durationUnknownCount: data.durationUnknownCount,
    });
  }

  // Fetch every page with an empty slot in the rendered window.
  function fetchWindowPages() {
    const items = readItems();
    for (let page = Math.floor(win.start / pageSize); page * pageSize < win.end; page += 1) {
      if (inFlight.has(page)) continue;
      const from = Math.max(win.start, page * pageSize);
      const to = Math.min(win.end, (page + 1) * pageSize);
      for (let slot = from; slot < to; slot += 1) {
        if (items[slot] === undefined) {
          void fetchWindowPage(page);
          break;
        }
      }
    }
  }

  async function fetchWindowPage(page) {
    const seq = ctl.seq;
    inFlight.add(page);
    updateLoadingIndicator();
    try {
      const data = (await fetchPage(requestParams(page * pageSize, pageSize))) || {};
      if (seq !== ctl.seq) return;
      if (Number.isFinite(Number(data.total)) && Number(data.total) !== ctl.total) {
        // The list changed underneath (a rescan, an edit elsewhere): reload
        // what's on screen instead of mixing two versions of it.
        void ctl.load().catch(onError);
        return;
      }
      onResponse(data);
      const tracks = storePage(page * pageSize, data.items);
      await renderWindow();
      if (seq !== ctl.seq) return;
      renderSummary(data);
      onPage(tracks, { first: false });
    } catch (err) {
      if (seq === ctl.seq) onError(err);
    } finally {
      if (seq === ctl.seq) {
        inFlight.delete(page);
        updateLoadingIndicator();
      }
    }
  }

  // Load the list from scratch. `keepPosition`: the same list again (a
  // refresh) -- stay where the user is and keep the old rows up until the new
  // ones arrive. Otherwise start at the top.
  async function loadFresh({ limit, keepPosition = false, showLoadingNow = false }) {
    const seq = (ctl.seq += 1);
    inFlight.clear();
    clearTimeout(fetchTimer);
    fetchTimer = null;
    ctl.loading = true;
    freshLoadMode = keepPosition ? "more" : "page";
    updateLoadingIndicator({ immediate: showLoadingNow });
    try {
      const want = Number.isFinite(limit) && limit > 0 ? limit : pageSize;
      let offset = keepPosition ? Math.floor(firstVisibleSlot() / pageSize) * pageSize : 0;
      let data = (await fetchPage(requestParams(offset, want))) || {};
      if (seq !== ctl.seq) return;
      const totalOf = (d) => (Number.isFinite(Number(d.total)) ? Number(d.total) : (d.items || []).length);
      if (offset > 0 && offset >= totalOf(data)) {
        // The list shrank past where the user was: load its last page.
        offset = Math.max(0, Math.floor((totalOf(data) - 1) / pageSize) * pageSize);
        data = (await fetchPage(requestParams(offset, want))) || {};
        if (seq !== ctl.seq) return;
      }

      onResponse(data);
      ctl.total = totalOf(data);
      writeItems(new Array(ctl.total));
      const tracks = storePage(offset, data.items);
      freshLoadMode = null;
      updateLoadingIndicator();
      resetRows();
      const { wrap } = getElements();
      if (!keepPosition && wrap) wrap.scrollTop = 0;
      await renderWindow();
      if (seq !== ctl.seq) return;
      renderSummary(data);
      onPage(tracks, { first: true });
      fetchWindowPages();
    } finally {
      if (seq === ctl.seq) {
        ctl.loading = false;
        // A failed fetch: drop the indicator so a "page" load doesn't leave
        // the old rows hidden behind "Loading..." forever.
        freshLoadMode = null;
        updateLoadingIndicator();
      }
    }
  }

  // Scroll / resize: move the window on the next frame, fetch what it shows
  // once scrolling settles.
  function onViewportChange() {
    if (frameHandle === null) {
      const raf = globalThis.requestAnimationFrame || ((cb) => setTimeout(cb, 16));
      frameHandle = raf(() => {
        frameHandle = null;
        void renderWindow();
      });
    }
    clearTimeout(fetchTimer);
    fetchTimer = setTimeout(() => {
      fetchTimer = null;
      fetchWindowPages();
    }, FETCH_SETTLE_MS);
  }

  // Load a (possibly different) list. Pass `{ scopeId }` when the selected
  // playlist/history/root changed: that starts at the top and shows the
  // loading indicator at once, since the rows on screen belong to the
  // previous list. The same list again keeps its scroll position. `limit`
  // is a one-shot size for the first fetch (e.g. the library after a scan).
  ctl.load = (opts = {}) => {
    let scopeChanged = false;
    if (Object.prototype.hasOwnProperty.call(opts, "scopeId")) {
      scopeChanged = opts.scopeId !== ctl.scopeId;
      ctl.scopeId = opts.scopeId;
      if (scopeChanged) onScopeChange(ctl.scopeId);
    }
    return loadFresh({ limit: opts.limit, keepPosition: !scopeChanged, showLoadingNow: scopeChanged });
  };

  // Re-fetch from the top with the current scope/query/sort (search or sort change).
  ctl.reload = () => loadFresh({});

  // Rebuild the rendered rows without a fetch -- the loaded tracks changed in
  // place (a row-click re-hydrate, a cross-view analysis patch) and the
  // in-place row patch couldn't find its DOM node.
  ctl.rerender = () => renderWindow({ rebuild: true });

  // Drop the current list without fetching (view deselected / USB disconnected).
  // Bumps `seq` so any in-flight response is ignored, and renders an empty table.
  ctl.clear = async () => {
    ctl.seq += 1;
    ctl.scopeId = null;
    onScopeChange(null);
    writeItems([]);
    ctl.total = 0;
    ctl.loading = false;
    inFlight.clear();
    freshLoadMode = null;
    updateLoadingIndicator();
    resetRows();
    await renderWindow();
    renderDurationSummary(getElements().durationTarget, { totalDurationMs: 0, durationUnknownCount: 0 });
  };

  ctl.setSearch = (value) => {
    const next = String(value || "");
    if (next === ctl.query) return Promise.resolve();
    ctl.query = next;
    return ctl.reload();
  };

  // Wired as this view's entry in `bodyToRendererMap` -- the shell's sort
  // header click updates `tableSortState[bodyId]` then calls this. Reloads
  // with the new sortBy (so the sort spans the whole list).
  ctl.applyHeaderSort = () => {
    const st = getTableSortState()[bodyId] || null;
    const by = st?.key || null;
    const dir = st?.dir || null;
    if (by === ctl.sortBy && dir === ctl.sortDir) return Promise.resolve();
    ctl.sortBy = by;
    ctl.sortDir = dir;
    return ctl.reload();
  };

  // Patch a single already-rendered row in place after an out-of-band update
  // (a row-click re-hydration, a cross-view analysis event). Returns whatever
  // the injected patcher returns (false ⇒ row gone ⇒ caller may full-rerender).
  ctl.patchRow = (track, patchRowInContainer) => {
    const { body } = getElements();
    return body ? patchRowInContainer(body, track) : false;
  };

  // Drag-reorder: hold the rendered rows still while rows are dragged around.
  // On release the dropped row stays where it was dropped (the refresh after
  // the reorder replaces the rows); the next window move puts the DOM back
  // in slot order first.
  ctl.freezeWindow = (on) => {
    frozen = !!on;
    if (frozen) return;
    orderDirty = true;
    if (rebuildWhenThawed) {
      rebuildWhenThawed = false;
      void renderWindow({ rebuild: true });
    }
  };

  ctl.attachScroll = () => {
    const { wrap } = getElements();
    if (!wrap || ctl._scrollBound) return;
    ctl._scrollBound = true;
    wrap.addEventListener("scroll", onViewportChange, { passive: true });
    if (typeof ResizeObserver === "function") new ResizeObserver(onViewportChange).observe(wrap);
  };

  return ctl;
}
