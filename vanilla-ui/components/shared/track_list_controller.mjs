import { loadMoreIfNearBottom } from "../../track_utils.mjs";

// One data layer shared by every track-list view (library, app playlist, USB
// playlist, USB history). Rendering (`renderTrackTable`), the scroll-load check
// (`loadMoreIfNearBottom`), the duration footer, sorting (column headers) and
// row-action dispatch were already shared; this unifies the last piece that
// each view used to reimplement -- fetch + pagination + search + sort +
// in-place patching.
//
// Every backing command returns the same envelope:
//   { items, total, nextCursor, hasMore, totalDurationMs?, durationKnownCount? }
// with `query` / `sortBy` / `sortDir` / `cursor` / `limit` request params, so
// filtering and sorting run server-side over the whole list (not just the
// loaded page). Changing the search text or the column sort reloads from
// page 1.
//
// A view supplies a small config; the controller owns all the mechanics.

const DEFAULT_PAGE_SIZE = 150;
const SCROLL_THRESHOLD_PX = 120;
// A fetch that answers within this long never shows the loading indicator, so
// fast views (local library, app playlists) don't flash it on every search
// keystroke / sort click. Slow ones (USB pages hydrating from the stick) do.
const LOADING_INDICATOR_DELAY_MS = 200;

export function createTrackListController(config = {}) {
  const {
    bodyId,
    pageSize = DEFAULT_PAGE_SIZE,
    // () => ({ body, wrap, durationTarget }) -- resolved lazily so the
    // controller can be built before the DOM elements exist.
    getElements = () => ({}),
    // async ({ scopeId, query, sortBy, sortDir, cursor, limit }) -> envelope
    fetchPage,
    normalize = (track) => track,
    // () => renderTrackTable options for this view
    rowOptions = () => ({}),
    // (tbody, tracks, options) -> Promise
    renderTrackTable = async () => {},
    // (target, { totalDurationMs, durationKnownCount, trackCount }) -> void
    renderDurationSummary = () => {},
    // (pageTracks, { first }) -> void  (e.g. library kicks off bg preview hydration)
    onPage = () => {},
    // (envelope) -> void  (e.g. library consumes sourceRootAnalysis)
    onResponse = () => {},
    // (scopeId) -> void  -- the shown list changed (load with a new scopeId,
    // or clear); e.g. the USB playlist view's heading.
    onScopeChange = () => {},
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
    nextCursor: null,
    hasMore: false,
    loading: false,
    query: "",
    sortBy: null,
    sortDir: null,
    // Bumped on every fresh (non-append) load; an in-flight response for a
    // superseded seq is dropped -- the analogue of every view's old
    // per-request "seq" / "hydration token" bookkeeping, in one place.
    seq: 0,
    _scrollBound: false,
  };

  // Loading indicator: `data-loading` on the body -- "page" hides the current
  // rows (a page-1 load replaces them), "more" keeps them (an append). The
  // CSS shows the `.track-grid-loading` element that follows the body.
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
  function hideLoadingIndicator() {
    clearTimeout(loadingTimer);
    loadingTimer = null;
    setLoadingIndicator(null);
  }
  function scheduleLoadingIndicator(mode, { immediate = false } = {}) {
    hideLoadingIndicator();
    if (immediate) {
      setLoadingIndicator(mode);
      return;
    }
    loadingTimer = setTimeout(() => {
      loadingTimer = null;
      setLoadingIndicator(mode);
    }, LOADING_INDICATOR_DELAY_MS);
  }
  Object.defineProperty(ctl, "items", {
    get: readItems,
    set: writeItems,
    enumerable: true,
  });

  // What the table currently shows / row actions resolve against. Every view
  // renders `items` in exact order (search + sort are backend query params),
  // so this is just `items`.
  Object.defineProperty(ctl, "view", {
    get: readItems,
    enumerable: true,
  });

  async function run(cursor, { append, limit, showLoadingNow = false }) {
    const seq = append ? ctl.seq : (ctl.seq += 1);
    ctl.loading = true;
    scheduleLoadingIndicator(append ? "more" : "page", { immediate: showLoadingNow });
    try {
      const data = (await fetchPage({
        scopeId: ctl.scopeId,
        query: ctl.query,
        sortBy: ctl.sortBy,
        sortDir: ctl.sortDir,
        cursor,
        limit: Number.isFinite(limit) && limit > 0 ? limit : pageSize,
      })) || {};
      if (seq !== ctl.seq) return;
      hideLoadingIndicator();

      onResponse(data);
      const page = (data.items || []).map((item) => normalize(item));
      ctl.items = append ? readItems().concat(page) : page;
      ctl.total = Number.isFinite(Number(data.total)) ? Number(data.total) : readItems().length;
      ctl.nextCursor = data.nextCursor ?? null;
      ctl.hasMore = !!data.hasMore;

      const { body, durationTarget } = getElements();
      const indexOffset = append ? readItems().length - page.length : 0;
      await renderTrackTable(
        body,
        page,
        append ? { ...rowOptions(), append: true, indexOffset } : rowOptions()
      );
      if (seq !== ctl.seq) return;

      renderDurationSummary(durationTarget, {
        totalDurationMs: data.totalDurationMs,
        durationKnownCount: data.durationKnownCount,
        trackCount: ctl.total,
      });
      onPage(page, { first: !append });
    } finally {
      if (seq === ctl.seq) {
        ctl.loading = false;
        // A failed fetch: drop the indicator so a "page" load doesn't leave
        // the old rows hidden behind "Loading..." forever.
        hideLoadingIndicator();
      }
    }
  }

  // Load a (possibly different) list from page 1. Pass `{ scopeId }` when the
  // selected playlist/history/root changed. Switching to a different list
  // shows the loading indicator at once: the rows on screen belong to the
  // previous list and must not linger under the new one's name.
  ctl.load = async (opts = {}) => {
    let scopeChanged = false;
    if (Object.prototype.hasOwnProperty.call(opts, "scopeId")) {
      scopeChanged = opts.scopeId !== ctl.scopeId;
      ctl.scopeId = opts.scopeId;
      if (scopeChanged) onScopeChange(ctl.scopeId);
    }
    ctl.items = [];
    ctl.nextCursor = null;
    ctl.hasMore = false;
    // Optional one-shot page-size override for this first fetch (e.g. the
    // library loads a bigger first page right after a scan). loadMore() keeps
    // using the configured pageSize.
    await run(null, { append: false, limit: opts.limit, showLoadingNow: scopeChanged });
  };

  // Re-fetch page 1 with the current scope/query/sort (search or sort change).
  ctl.reload = () => run(null, { append: false });

  // Re-render the currently-loaded rows without a fetch -- used when a row's
  // data changed in place (a row-click re-hydrate, a cross-view analysis
  // patch) but the in-place row patch couldn't find its DOM node.
  ctl.rerender = async () => {
    const { body } = getElements();
    if (body) await renderTrackTable(body, ctl.view, rowOptions());
  };

  // Drop the current list without fetching (view deselected / USB disconnected).
  // Bumps `seq` so any in-flight response is ignored, and renders an empty table.
  ctl.clear = async () => {
    ctl.seq += 1;
    ctl.scopeId = null;
    onScopeChange(null);
    ctl.items = [];
    ctl.total = 0;
    ctl.nextCursor = null;
    ctl.hasMore = false;
    ctl.loading = false;
    hideLoadingIndicator();
    const { body, durationTarget } = getElements();
    if (body) await renderTrackTable(body, [], rowOptions());
    renderDurationSummary(durationTarget, { totalDurationMs: 0, durationKnownCount: 0, trackCount: 0 });
  };

  ctl.loadMore = async () => {
    if (ctl.loading || !ctl.hasMore) return;
    await run(ctl.nextCursor, { append: true });
  };

  ctl.setSearch = (value) => {
    const next = String(value || "");
    if (next === ctl.query) return Promise.resolve();
    ctl.query = next;
    return ctl.reload();
  };

  // Wired as this view's entry in `bodyToRendererMap` -- the shell's sort
  // header click updates `tableSortState[bodyId]` then calls this. Reloads
  // page 1 with the new sortBy (so the sort spans the whole list).
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

  ctl.attachScroll = () => {
    const { wrap } = getElements();
    if (!wrap || ctl._scrollBound) return;
    ctl._scrollBound = true;
    wrap.addEventListener(
      "scroll",
      () => {
        loadMoreIfNearBottom(
          wrap,
          SCROLL_THRESHOLD_PX,
          () => ctl.loading,
          () => ctl.hasMore,
          () => ctl.loadMore().catch((err) => console.warn(`${bodyId} page load failed:`, err))
        );
      },
      { passive: true }
    );
  };

  return ctl;
}
