import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { isTransportButtonPlaying, shouldToggleStop } = require("../playback_ui_state.js");

test("USB row stays active when playback was redirected to local track id", () => {
  const state = {
    playbackActive: true,
    playbackRowKey: "usb:741",
    playbackTrackId: "local-123",
  };

  const activeUsbButton = isTransportButtonPlaying(state, { rowKey: "usb:741", trackId: "741" });
  const inactiveOtherButton = isTransportButtonPlaying(state, { rowKey: "usb:742", trackId: "742" });

  assert.equal(activeUsbButton, true);
  assert.equal(inactiveOtherButton, false);
});

test("falls back to track id when row key is missing", () => {
  const state = {
    playbackActive: true,
    playbackRowKey: "",
    playbackTrackId: "local-22",
  };

  assert.equal(isTransportButtonPlaying(state, { rowKey: "", trackId: "local-22" }), true);
  assert.equal(isTransportButtonPlaying(state, { rowKey: "", trackId: "local-23" }), false);
});

test("shouldToggleStop uses row key first", () => {
  const state = {
    playbackActive: true,
    playbackRowKey: "usb:11",
    playbackTrackId: "local-11",
  };

  assert.equal(shouldToggleStop(state, "usb:11", false), true);
  assert.equal(shouldToggleStop(state, "usb:12", false), false);
});

test("shouldToggleStop falls back to currently-playing track check", () => {
  const state = {
    playbackActive: true,
    playbackRowKey: "",
    playbackTrackId: "local-11",
  };

  assert.equal(shouldToggleStop(state, "", true), true);
  assert.equal(shouldToggleStop(state, "", false), false);
});

test("inactive playback never marks button playing or stop", () => {
  const state = {
    playbackActive: false,
    playbackRowKey: "usb:9",
    playbackTrackId: "local-9",
  };

  assert.equal(isTransportButtonPlaying(state, { rowKey: "usb:9", trackId: "local-9" }), false);
  assert.equal(shouldToggleStop(state, "usb:9", true), false);
});

test("a pending stop immediately reads as not-playing even while playbackActive is still true", () => {
  const state = {
    playbackActive: true,
    playbackRowKey: "row:1",
    playbackTrackId: "local-1",
    playbackPendingKind: "stop",
  };

  assert.equal(isTransportButtonPlaying(state, { rowKey: "row:1", trackId: "local-1" }), false);
  assert.equal(shouldToggleStop(state, "row:1", true), false);
});

test("a pending play immediately reads as playing for its target row before playbackActive flips", () => {
  const state = {
    playbackActive: false,
    playbackRowKey: null,
    playbackTrackId: null,
    playbackPendingKind: "play",
    playbackPendingRowKey: "row:2",
    playbackPendingTrackId: "local-2",
  };

  assert.equal(isTransportButtonPlaying(state, { rowKey: "row:2", trackId: "local-2" }), true);
  assert.equal(isTransportButtonPlaying(state, { rowKey: "row:3", trackId: "local-3" }), false);
  assert.equal(shouldToggleStop(state, "row:2", false), true);
  assert.equal(shouldToggleStop(state, "row:3", false), false);
});
