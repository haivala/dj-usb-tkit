import test from "node:test";
import assert from "node:assert/strict";
import { addTracksToCurrentPlaylist } from "../components/playlist/actions.mjs";
import { makeTestCtx } from "./test_helpers.mjs";

// An app ctx with `playlist` open. `onAdd` answers add_track_candidates_to_playlist;
// the playlist re-fetch after a resolved add is counted in `refreshes`.
function addCtx({ playlist = { id: "pl-1", name: "Main" }, onAdd, usbRoot = null, usbRootValid = false } = {}) {
  const out = { status: "", refreshes: 0 };
  out.ctx = makeTestCtx({
    withProgress: async (_label, run) => run(() => {}),
    pushEventLog: () => {},
    emitStatus: (text) => { out.status = text; },
    command: async (name, payload) => {
      if (name === "get_playlist_tracks") {
        out.refreshes += 1;
        return { items: [], total: 0 };
      }
      return onAdd(name, payload);
    }
  });
  Object.assign(out.ctx.state, {
    playlists: playlist ? [playlist] : [],
    currentPlaylistId: playlist?.id ?? null,
    usbRoot,
    usbRootValid
  });
  return out;
}

test("addTracksToCurrentPlaylist requires active playlist", async () => {
  const harness = addCtx({
    playlist: null,
    onAdd: () => { throw new Error("nothing should be sent without an active playlist"); }
  });

  await addTracksToCurrentPlaylist(harness.ctx, [{ id: "1" }]);

  assert.equal(harness.status, "Create and activate a playlist first");
});

test("addTracksToCurrentPlaylist sends row candidates to backend and reports result", async () => {
  let capturedCommand = null;

  const harness = addCtx({
    onAdd: async (name, payload) => {
      capturedCommand = { name, payload };
      return {
        playlistId: "pl-1",
        requested: 1,
        resolved: 1,
        unresolved: 0,
        added: 1,
        skipped: 0,
        resolutions: [{ previousId: "1", trackId: "local-1", resolvedBy: "localTrackId", materialized: false }]
      };
    },
    usbRoot: "/media/USB",
    usbRootValid: true
  });
  await addTracksToCurrentPlaylist(harness.ctx, [{
    id: "1",
    localTrackId: "local-1",
    title: "Track One",
    artist: "Artist",
    album: "Album",
    bpm: 128,
    filePath: "/music/one.mp3",
    fileSizeBytes: 1234
  }]);

  assert.equal(capturedCommand.name, "add_track_candidates_to_playlist");
  assert.equal(capturedCommand.payload.playlistId, "pl-1");
  assert.equal(capturedCommand.payload.dedupe, "skip");
  assert.equal(capturedCommand.payload.usbRoot, "/media/USB");
  assert.equal(capturedCommand.payload.usbRootValid, true);
  assert.deepEqual(capturedCommand.payload.tracks[0], {
    trackId: "1",
    localTrackId: "local-1",
    title: "Track One",
    artist: "Artist",
    album: "Album",
    bpm: 128,
    filePath: "/music/one.mp3",
    fileSizeBytes: 1234,
    trackNumber: null,
    key: null,
    formatExt: null,
    sampleRateHz: null,
    bitDepth: null,
    bitrateKbps: null,
    usbAnalysisPath: null,
    usbRoot: null,
    usbRootValid: false
  });
  assert.equal(harness.refreshes, 1);
  assert.match(harness.status, /Added 1 tracks \(skipped 0\) to Main/);
});

test("addTracksToCurrentPlaylist never sends both id and trackId (backend aliases them to the same field)", async () => {
  // Regression test: AddTrackCandidate.track_id (backend/src/models.rs) declares `id` as a
  // serde alias of `trackId`. Sending both keys at once -- even with one set to null -- makes
  // serde reject the whole request with "duplicate field trackId", which broke every bulk add
  // from the library (the exact bug the id/trackId split payload used to trigger).
  let capturedCommand = null;

  const harness = addCtx({
    onAdd: async (name, payload) => {
      capturedCommand = { name, payload };
      return { playlistId: "pl-1", requested: 1, resolved: 1, unresolved: 0, added: 1, skipped: 0, resolutions: [] };
    }
  });
  await addTracksToCurrentPlaylist(harness.ctx, [{ id: "1", title: "Track One", artist: "Artist" }]);

  assert.equal("id" in capturedCommand.payload.tracks[0], false);
  assert.equal(capturedCommand.payload.tracks[0].trackId, "1");
});

test("addTracksToCurrentPlaylist never sends a non-numeric bpm, even for an unanalyzed track", async () => {
  let capturedCommand = null;

  // Shaped like normalizeTrack() would (pre-fix) produce for a track that hasn't been
  // BPM-analyzed yet -- this is exactly the payload that used to crash the real backend
  // with "invalid type: string \"\", expected f64".
  const harness = addCtx({
    onAdd: async (name, payload) => {
      capturedCommand = { name, payload };
      return { playlistId: "pl-1", requested: 1, resolved: 1, unresolved: 0, added: 1, skipped: 0, resolutions: [] };
    }
  });
  await addTracksToCurrentPlaylist(harness.ctx, [{
    id: "1",
    localTrackId: null,
    title: "Unanalyzed Track",
    artist: "Artist",
    album: "",
    bpm: "",
    filePath: "/music/unanalyzed.mp3"
  }]);

  assert.notEqual(typeof capturedCommand.payload.tracks[0].bpm, "string");
  assert.equal(capturedCommand.payload.tracks[0].bpm, null);
});

test("addTracksToCurrentPlaylist reports unresolved backend candidates without refreshing", async () => {

  const harness = addCtx({
    onAdd: async () => ({
      playlistId: "pl-1",
      requested: 1,
      resolved: 0,
      unresolved: 1,
      added: 0,
      skipped: 0,
      resolutions: [{ previousId: "usb-1", trackId: null, resolvedBy: "usbOrigin", materialized: false }]
    })
  });
  await addTracksToCurrentPlaylist(harness.ctx, [{ id: "usb-1", title: "USB Track", usbAnalysisPath: "/USB/PIONEER/USBANLZ/P001/A/ANLZ0000.DAT" }]);

  assert.equal(harness.refreshes, 0);
  assert.equal(harness.status, "No imported track IDs found to add");
});

test("addTracksToCurrentPlaylist clears exported-to-USB status after a resolved add", async () => {
  const playlist = {
    id: "pl-1",
    name: "Main",
    lastExportedAt: "2026-01-01T00:00:00Z",
    lastExportedUsbRoot: "/mnt/usb1",
    lastExportedTrackCount: 5
  };

  const harness = addCtx({
    playlist: playlist,
    onAdd: async () => ({ playlistId: "pl-1", requested: 1, resolved: 1, unresolved: 0, added: 1, skipped: 0, resolutions: [] })
  });
  await addTracksToCurrentPlaylist(harness.ctx, [{ id: "1" }]);

  assert.equal(playlist.lastExportedAt, null);
  assert.equal(playlist.lastExportedUsbRoot, null);
  assert.equal(playlist.lastExportedTrackCount, null);
});
