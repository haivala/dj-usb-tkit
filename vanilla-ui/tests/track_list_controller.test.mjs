import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { createTrackListController } from "../components/shared/track_list_controller.mjs";

const ROW_HEIGHT = 47; // the controller's fallback when there's no CSS layout
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
// One animation frame (the window move) + the scroll-settle delay (the fetch).
const settle = () => sleep(200);

// A list of `total` tracks served by offset, like the backend commands.
function listResponder(total, { idPrefix = "" } = {}) {
  return ({ offset, limit }) => {
    const items = [];
    for (let i = offset; i < Math.min(total, offset + limit); i += 1) items.push({ id: `${idPrefix}${i}` });
    return { items, total, totalDurationMs: 10, durationUnknownCount: 2 };
  };
}

// `viewportHeight`: the scroller's height in px; 0 = hidden (no layout).
function harness(overrides = {}) {
  const calls = { fetch: [], built: [], duration: [], pages: [] };
  const { document } = new JSDOM("<!doctype html><body><div id='body'></div></body>").window;
  const body = document.getElementById("body");
  const listeners = {};
  const wrap = {
    clientHeight: overrides.viewportHeight ?? 470, // 10 rows
    scrollTop: 0,
    addEventListener: (type, fn) => { listeners[type] = fn; },
    getBoundingClientRect: () => ({ top: 0 }),
  };
  body.getBoundingClientRect = () => ({ top: -wrap.scrollTop });
  const respond = overrides.respond || listResponder(overrides.total ?? 3);
  let fetchIdx = 0;
  const ctl = createTrackListController({
    bodyId: "body",
    pageSize: overrides.pageSize ?? 2,
    getElements: () => ({ body, wrap, durationTarget: "dur" }),
    fetchPage: async (params) => {
      calls.fetch.push(params);
      return respond(params, fetchIdx++);
    },
    rowOptions: () => ({ origin: "usb" }),
    buildRow: (track, options) => {
      calls.built.push(options.index);
      const row = document.createElement("div");
      row.className = track ? "row" : "row skeleton";
      row.dataset.slot = String(options.index);
      if (track) row.dataset.id = track.id;
      return row;
    },
    renderEmpty: (b) => { b.innerHTML = "<div class='empty'></div>"; },
    renderDurationSummary: (_t, summary) => { calls.duration.push(summary); },
    onPage: (page, meta) => { calls.pages.push({ n: page.length, ...meta }); },
    getTableSortState: () => overrides.tableSortState || {},
    normalize: overrides.normalize,
    getItems: overrides.getItems,
    setItems: overrides.setItems,
  });
  ctl.attachScroll();
  const scrollTo = (slot) => {
    wrap.scrollTop = slot * ROW_HEIGHT;
    listeners.scroll();
  };
  const rows = () => [...body.children].map((row) => (row.classList.contains("skeleton") ? `~${row.dataset.slot}` : row.dataset.id));
  return { ctl, calls, body, wrap, scrollTo, rows };
}

test("load fetches the top page, keeps one slot per track, and renders the loaded rows", async () => {
  const { ctl, calls, rows } = harness();
  await ctl.load({ scopeId: "pl-1" });
  assert.equal(ctl.total, 3);
  assert.equal(ctl.items.length, 3);
  assert.deepEqual(calls.fetch[0], { scopeId: "pl-1", query: "", sortBy: null, sortDir: null, offset: 0, limit: 2 });
  assert.deepEqual(calls.duration[0], { totalDurationMs: 10, durationUnknownCount: 2 });
  assert.deepEqual(calls.pages[0], { n: 2, first: true });
  // The rest of the window is fetched straight away, not on a scroll.
  await settle();
  assert.deepEqual(calls.fetch.map((f) => f.offset), [0, 2]);
  assert.deepEqual(rows(), ["0", "1", "2"]);
});

test("an empty list renders the empty table", async () => {
  const { ctl, body } = harness({ total: 0 });
  await ctl.load({ scopeId: "pl-1" });
  assert.equal(body.querySelectorAll(".empty").length, 1);
});

test("load({ limit }) sizes the first fetch only", async () => {
  const { ctl, calls } = harness({ total: 1000, pageSize: 100 });
  await ctl.load({ scopeId: "pl-1", limit: 500 });
  assert.equal(calls.fetch[0].limit, 500);
});

test("only the rows around the viewport are in the DOM; padding stands in for the rest", async () => {
  const { ctl, body, rows } = harness({ total: 1000, pageSize: 100 });
  await ctl.load({ scopeId: "pl-1" });
  // 10 visible rows + 20 overscan below.
  assert.equal(rows().length, 30);
  assert.equal(body.style.getPropertyValue("--track-grid-before"), "0px");
  assert.equal(body.style.getPropertyValue("--track-grid-after"), `${(1000 - 30) * ROW_HEIGHT}px`);
});

test("jumping into the list renders skeletons, then fetches just that part once scrolling settles", async () => {
  const { ctl, calls, scrollTo, rows, body } = harness({ total: 1000, pageSize: 100 });
  await ctl.load({ scopeId: "pl-1" });
  calls.fetch.length = 0;

  scrollTo(300);
  scrollTo(500); // a drag across the list: nothing is fetched on the way
  await sleep(40);
  assert.deepEqual(calls.fetch, []);
  assert.equal(rows()[0], "~480");
  assert.equal(body.style.getPropertyValue("--track-grid-before"), `${480 * ROW_HEIGHT}px`);

  await settle();
  // Slots 480..529 span pages 4 and 5; pages 1..3 are never fetched.
  assert.deepEqual(calls.fetch.map((f) => f.offset).sort((a, b) => a - b), [400, 500]);
  assert.equal(rows()[0], "480");
  assert.equal(rows().at(-1), "529");
  assert.equal(ctl.items[200], undefined);
  assert.equal(ctl.items[480].id, "480");
});

test("scrolling away drops the rows that left the window", async () => {
  const { ctl, scrollTo, rows } = harness({ total: 1000, pageSize: 100 });
  await ctl.load({ scopeId: "pl-1" });
  scrollTo(60);
  await settle();
  assert.equal(rows()[0], "40");
  assert.equal(rows().includes("0"), false);
});

test("a refresh of the same list stays where the user is", async () => {
  const { ctl, calls, scrollTo, wrap } = harness({ total: 1000, pageSize: 100 });
  await ctl.load({ scopeId: "pl-1" });
  scrollTo(500);
  await settle();
  calls.fetch.length = 0;
  await ctl.load({ scopeId: "pl-1" });
  assert.equal(calls.fetch[0].offset, 500);
  assert.equal(wrap.scrollTop, 500 * ROW_HEIGHT);
});

test("a different list starts at the top", async () => {
  const { ctl, calls, scrollTo, wrap } = harness({ total: 1000, pageSize: 100 });
  await ctl.load({ scopeId: "pl-1" });
  scrollTo(500);
  await settle();
  calls.fetch.length = 0;
  await ctl.load({ scopeId: "pl-2" });
  assert.equal(calls.fetch[0].offset, 0);
  assert.equal(wrap.scrollTop, 0);
});

test("setSearch reloads from the top with the query; a no-change search does nothing", async () => {
  const { ctl, calls, scrollTo, wrap } = harness({ total: 1000, pageSize: 100 });
  await ctl.load({ scopeId: "pl-1" });
  scrollTo(500);
  await settle();
  await ctl.setSearch("house");
  const reload = calls.fetch.find((f) => f.query === "house");
  assert.equal(reload.offset, 0);
  assert.equal(wrap.scrollTop, 0);
  const n = calls.fetch.length;
  await ctl.setSearch("house");
  assert.equal(calls.fetch.length, n);
});

test("applyHeaderSort reads tableSortState and reloads when it changes", async () => {
  const sortState = {};
  const { ctl, calls } = harness({ tableSortState: sortState });
  await ctl.load({ scopeId: "pl-1" });
  sortState.body = { key: "title", dir: "desc" };
  await ctl.applyHeaderSort();
  assert.equal(calls.fetch.at(-1).sortBy, "title");
  assert.equal(calls.fetch.at(-1).sortDir, "desc");
  const n = calls.fetch.length;
  await ctl.applyHeaderSort(); // unchanged
  assert.equal(calls.fetch.length, n);
});

test("a superseded load's late response is dropped", async () => {
  let resolveFirst;
  const { ctl } = harness({
    respond: (_params, idx) => {
      if (idx === 0) return new Promise((r) => { resolveFirst = () => r({ items: [{ id: "stale" }], total: 1 }); });
      return { items: [{ id: "fresh" }], total: 1 };
    },
  });
  const first = ctl.load({ scopeId: "a" });
  const second = ctl.load({ scopeId: "b" });
  await second;
  resolveFirst();
  await first;
  assert.deepEqual(ctl.items.map((t) => t.id), ["fresh"]);
});

test("a page whose total changed reloads the list instead of mixing versions", async () => {
  let total = 1000;
  const { ctl, scrollTo } = harness({ pageSize: 100, respond: (params) => listResponder(total)(params) });
  await ctl.load({ scopeId: "pl-1" });
  total = 900;
  scrollTo(500);
  await settle();
  await settle();
  assert.equal(ctl.total, 900);
  assert.equal(ctl.items.length, 900);
  assert.equal(ctl.items[480]?.id, "480");
});

test("normalize is applied to every fetched item", async () => {
  const { ctl } = harness({ normalize: (t) => ({ ...t, tag: `n-${t.id}` }) });
  await ctl.load({ scopeId: "pl-1" });
  assert.deepEqual(ctl.items.slice(0, 2).map((t) => t.tag), ["n-0", "n-1"]);
});

test("rerender rebuilds the rendered rows without fetching", async () => {
  const { ctl, calls } = harness();
  await ctl.load({ scopeId: "pl-1" });
  await settle();
  const fetchCount = calls.fetch.length;
  calls.built.length = 0;
  await ctl.rerender();
  assert.equal(calls.fetch.length, fetchCount);
  assert.deepEqual(calls.built.sort(), [0, 1, 2]);
});

test("a frozen window (drag-reorder) keeps every row and only swaps in arrived skeletons", async () => {
  const { ctl, scrollTo, rows } = harness({ total: 1000, pageSize: 100 });
  await ctl.load({ scopeId: "pl-1" });
  ctl.freezeWindow(true);
  scrollTo(60);
  await settle();
  // Rows 0.. are still there, and the window grew to cover the new viewport.
  assert.equal(rows()[0], "0");
  assert.equal(rows().at(-1), "89");
  assert.equal(rows().some((r) => r.startsWith("~")), false);
  ctl.freezeWindow(false);
  scrollTo(60);
  await settle();
  assert.equal(rows()[0], "40");
});

test("a hidden list still renders a first window", async () => {
  const { ctl, rows } = harness({ total: 1000, pageSize: 100, viewportHeight: 0 });
  await ctl.load({ scopeId: "pl-1" });
  assert.equal(rows().length, 60);
});

test("clear empties the list, renders an empty table, and zeroes the footer", async () => {
  const { ctl, calls, body } = harness();
  await ctl.load({ scopeId: "pl-1" });
  await ctl.clear();
  assert.deepEqual(ctl.items, []);
  assert.equal(ctl.scopeId, null);
  assert.equal(ctl.total, 0);
  assert.equal(body.querySelectorAll(".empty").length, 1);
  assert.deepEqual(calls.duration.at(-1), { totalDurationMs: 0, durationUnknownCount: 0 });
});

test("items are backed by an external store when getItems/setItems are given", async () => {
  const store = { list: [] };
  const { ctl } = harness({
    getItems: () => store.list,
    setItems: (v) => { store.list = v; },
  });
  await ctl.load({ scopeId: "x" });
  assert.deepEqual(store.list.slice(0, 2).map((t) => t.id), ["0", "1"]);
  assert.equal(ctl.items, store.list);
  // an in-place mutation of the external store is visible through ctl.items
  store.list[0].tag = "patched";
  assert.equal(ctl.items[0].tag, "patched");
});
