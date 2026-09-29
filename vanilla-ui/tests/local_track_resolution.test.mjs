import test from "node:test";
import assert from "node:assert/strict";
import {
  resolveLocalTrackIdAsync,
  isTrackCurrentlyPlaying
} from "../components/playback/actions.mjs";

test("resolveLocalTrackIdAsync returns an existing localTrackId without hitting the backend", async () => {
  let called = false;
  const id = await resolveLocalTrackIdAsync({
    state: {},
    command: async () => { called = true; return {}; }
  }, { id: "usb-1", localTrackId: "local-7" });
  assert.equal(id, "local-7");
  assert.equal(called, false);
});

test("resolveLocalTrackIdAsync resolves via the backend and promotes identity", async () => {
  const calls = [];
  let promoted = null;
  const track = { id: "usb-1", filePath: "/music/a.mp3", title: "A", artist: "AA" };

  const id = await resolveLocalTrackIdAsync({
    state: { usbRoot: null },
    command: async (name, payload) => {
      calls.push({ name, payload });
      if (name === "resolve_track_identity") {
        return { trackId: "local-99", resolvedBy: "materialized", materialized: true };
      }
      throw new Error(`unexpected command ${name}`);
    },
    promoteTrackIdentity: (from, to) => { promoted = { from, to }; }
  }, track);

  assert.equal(id, "local-99");
  assert.equal(track.localTrackId, "local-99");
  assert.deepEqual(promoted, { from: "usb-1", to: "local-99" });
  assert.equal(calls[0].name, "resolve_track_identity");
  assert.equal(calls[0].payload.trackId, "usb-1");
  assert.equal(calls[0].payload.filePath, "/music/a.mp3");
});

test("resolveLocalTrackIdAsync returns null when the backend can't resolve", async () => {
  const id = await resolveLocalTrackIdAsync({
    state: {},
    command: async () => ({ trackId: null, resolvedBy: "none", materialized: false })
  }, { id: "usb-9", title: "X" });
  assert.equal(id, null);
});

test("isTrackCurrentlyPlaying matches on the backend-resolved localTrackId", () => {
  const state = { playbackActive: true, playbackTrackId: "local-3" };
  assert.equal(isTrackCurrentlyPlaying({ state }, { id: "usb-1", localTrackId: "local-3" }), true);
  assert.equal(isTrackCurrentlyPlaying({ state }, { id: "usb-1", localTrackId: "local-4" }), false);
});

test("isTrackCurrentlyPlaying falls back to the row id when there is no localTrackId", () => {
  const state = { playbackActive: true, playbackTrackId: "local-3" };
  assert.equal(isTrackCurrentlyPlaying({ state }, { id: "local-3" }), true);
});

test("isTrackCurrentlyPlaying is false when nothing is playing", () => {
  assert.equal(isTrackCurrentlyPlaying({ state: { playbackActive: false, playbackTrackId: "local-3" } }, { id: "local-3" }), false);
});

test("isTrackCurrentlyPlaying honors a pending stop / pending play", () => {
  assert.equal(
    isTrackCurrentlyPlaying({ state: { playbackPendingKind: "stop", playbackActive: true, playbackTrackId: "local-3" } }, { id: "local-3" }),
    false
  );
  assert.equal(
    isTrackCurrentlyPlaying({ state: { playbackPendingKind: "play", playbackPendingTrackId: "local-3" } }, { id: "local-3" }),
    true
  );
});
