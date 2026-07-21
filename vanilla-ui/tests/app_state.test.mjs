import test from "node:test";
import assert from "node:assert/strict";

import {
  createInitialState,
  createTableSortState,
  createEventLogState,
  STATIC_TABS,
  EVENT_LOG_MAX
} from "../app_state.mjs";
import { DEFAULT_ANALYSIS_BPM_RANGE } from "../components/library/actions.mjs";

test("createInitialState returns expected default values", () => {
  const state = createInitialState();

  assert.deepEqual(state.sourceRoots, []);
  assert.equal(state.usbRoot, null);
  assert.equal(state.exportPruneStale, true);
  assert.equal(state.analysisBpmRange, DEFAULT_ANALYSIS_BPM_RANGE);
  assert.equal(state.activeTab, "library");
  assert.equal(state.progressBaseText, "Idle");
  assert.equal(state.startupPhase, true);
  assert.deepEqual(state.mockPlayback, {
    path: null,
    playing: false,
    startedAtMs: 0,
    startOffsetMs: 0,
    durationMs: 240000
  });
});

test("createInitialState creates fresh mutable containers", () => {
  const a = createInitialState();
  const b = createInitialState();

  a.selectedTrackIds.add("1");
  a.usbKnownPlaylistNames.add("A");
  a.trackPreviewHydrateInFlight.add("t1");
  a.analyzingTrackIds.add("t2");
  a.analysisPatchQueue.add("t3");
  a.selectedRepairFixIds.add("fix1");
  a.missingSourceRoots.add("/missing");
  a.sourceRoots.push("/music");

  assert.equal(b.selectedTrackIds.size, 0);
  assert.equal(b.usbKnownPlaylistNames.size, 0);
  assert.equal(b.trackPreviewHydrateInFlight.size, 0);
  assert.equal(b.analyzingTrackIds.size, 0);
  assert.equal(b.analysisPatchQueue.size, 0);
  assert.equal(b.selectedRepairFixIds.size, 0);
  assert.equal(b.missingSourceRoots.size, 0);
  assert.deepEqual(b.sourceRoots, []);
});

test("createTableSortState returns an empty object", () => {
  assert.deepEqual(createTableSortState(), {});
});

test("createEventLogState uses configured max entries", () => {
  const store = createEventLogState();

  for (let i = 0; i < EVENT_LOG_MAX + 5; i += 1) {
    store.push({ message: `entry-${i}` });
  }

  assert.equal(store.list().length, EVENT_LOG_MAX);
  assert.equal(store.list()[0].message, "entry-5");
});

test("STATIC_TABS lists the static navigation ids", () => {
  assert.deepEqual(STATIC_TABS, ["library", "usb", "usb-playlists", "usb-history", "usb-player-menu", "event-log"]);
});
