import test from "node:test";
import assert from "node:assert/strict";
import { createTrackListController } from "../components/shared/track_list_controller.mjs";

function harness(overrides = {}) {
  const calls = { fetch: [], renders: [], duration: [], pages: [] };
  const body = { id: "body" };
  const wrap = { addEventListener: () => {} };
  const pages = overrides.pages || [
    { items: [{ id: "1" }, { id: "2" }], total: 3, nextCursor: "c1", hasMore: true, totalDurationMs: 10, durationKnownCount: 1 },
    { items: [{ id: "3" }], total: 3, nextCursor: null, hasMore: false, totalDurationMs: 10, durationKnownCount: 1 },
  ];
  let pageIdx = 0;
  const ctl = createTrackListController({
    bodyId: "body",
    pageSize: 2,
    getElements: () => ({ body, wrap, durationTarget: "dur" }),
    fetchPage: async (params) => {
      calls.fetch.push(params);
      const idx = pageIdx++;
      return overrides.fetchPage
        ? await overrides.fetchPage(params, idx)
        : pages[Math.min(idx, pages.length - 1)];
    },
    rowOptions: () => ({ origin: "usb" }),
    renderTrackTable: async (_body, tracks, options) => { calls.renders.push({ n: tracks.length, options }); },
    renderDurationSummary: (_t, summary) => { calls.duration.push(summary); },
    onPage: (page, meta) => { calls.pages.push({ n: page.length, ...meta }); },
    getTableSortState: () => overrides.tableSortState || {},
    normalize: overrides.normalize,
  });
  return { ctl, calls, resetPageIdx: () => { pageIdx = 0; } };
}

test("load fetches page 1, replaces items, renders, and reports the whole-list total", async () => {
  const { ctl, calls } = harness();
  await ctl.load({ scopeId: "pl-1" });
  assert.deepEqual(ctl.items.map((t) => t.id), ["1", "2"]);
  assert.equal(ctl.total, 3);
  assert.equal(ctl.hasMore, true);
  assert.equal(calls.fetch[0].scopeId, "pl-1");
  assert.equal(calls.fetch[0].cursor, null);
  assert.equal(calls.renders[0].options.append, undefined);
  assert.deepEqual(calls.duration[0], { totalDurationMs: 10, durationKnownCount: 1, trackCount: 3 });
  assert.deepEqual(calls.pages[0], { n: 2, first: true });
});

test("loadMore appends the next page with the cursor and an index offset", async () => {
  const { ctl, calls } = harness();
  await ctl.load({ scopeId: "pl-1" });
  await ctl.loadMore();
  assert.deepEqual(ctl.items.map((t) => t.id), ["1", "2", "3"]);
  assert.equal(ctl.hasMore, false);
  assert.equal(calls.fetch[1].cursor, "c1");
  assert.equal(calls.renders[1].options.append, true);
  assert.equal(calls.renders[1].options.indexOffset, 2);
  assert.deepEqual(calls.pages[1], { n: 1, first: false });
});

test("loadMore is a no-op while loading or when hasMore is false", async () => {
  const { ctl, calls } = harness();
  await ctl.load({ scopeId: "pl-1" });
  await ctl.loadMore();
  const fetchCount = calls.fetch.length;
  await ctl.loadMore(); // hasMore now false
  assert.equal(calls.fetch.length, fetchCount);
});

test("setSearch reloads from page 1 with the query; a no-change search does nothing", async () => {
  const { ctl, calls } = harness();
  await ctl.load({ scopeId: "pl-1" });
  await ctl.setSearch("house");
  assert.equal(calls.fetch.at(-1).query, "house");
  assert.equal(calls.fetch.at(-1).cursor, null);
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
    fetchPage: (params, idx) => {
      if (idx === 0) return new Promise((r) => { resolveFirst = () => r({ items: [{ id: "stale" }], total: 1, hasMore: false }); });
      return Promise.resolve({ items: [{ id: "fresh" }], total: 1, hasMore: false });
    },
  });
  const first = ctl.load({ scopeId: "a" });
  const second = ctl.load({ scopeId: "b" });
  await second;
  resolveFirst();
  await first;
  assert.deepEqual(ctl.items.map((t) => t.id), ["fresh"]);
});

test("normalize is applied to every fetched item", async () => {
  const { ctl } = harness({ normalize: (t) => ({ ...t, tag: `n-${t.id}` }) });
  await ctl.load({ scopeId: "pl-1" });
  assert.deepEqual(ctl.items.map((t) => t.tag), ["n-1", "n-2"]);
});

test("clear empties the list, renders an empty table, and zeroes the footer", async () => {
  const { ctl, calls } = harness();
  await ctl.load({ scopeId: "pl-1" });
  await ctl.clear();
  assert.deepEqual(ctl.items, []);
  assert.equal(ctl.scopeId, null);
  assert.equal(ctl.hasMore, false);
  assert.equal(calls.renders.at(-1).n, 0);
  assert.deepEqual(calls.duration.at(-1), { totalDurationMs: 0, durationKnownCount: 0, trackCount: 0 });
});
