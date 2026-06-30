import test from "node:test";
import assert from "node:assert/strict";
import { addTracksToCurrentPlaylist } from "../components/playlist/actions.mjs";

test("addTracksToCurrentPlaylist requires active playlist", async () => {
  let status = "";

  await addTracksToCurrentPlaylist([{ id: "1" }], {
    requireCurrentPlaylist: () => null,
    resolveLocalTrackId: () => "t1",
    withProgress: async () => ({ added: 0, skipped: 0 }),
    command: async () => ({ added: 0, skipped: 0 }),
    refreshCurrentPlaylistTracks: async () => {},
    setStatus: (text) => { status = text; }
  });

  assert.equal(status, "");
});

test("addTracksToCurrentPlaylist skips when no resolvable track ids", async () => {
  let status = "";

  await addTracksToCurrentPlaylist([{ id: "1" }], {
    requireCurrentPlaylist: () => ({ id: "pl-1", name: "Main" }),
    resolveLocalTrackId: () => null,
    withProgress: async () => {
      throw new Error("withProgress should not run when no trackIds");
    },
    command: async () => ({ added: 0, skipped: 0 }),
    refreshCurrentPlaylistTracks: async () => {},
    setStatus: (text) => { status = text; }
  });

  assert.equal(status, "No imported track IDs found to add");
});

test("addTracksToCurrentPlaylist sends multiple track IDs with dedupe=skip and reports result", async () => {
  let capturedCommand = null;
  let refreshed = 0;
  let status = "";

  await addTracksToCurrentPlaylist([{ id: "1" }, { id: "2" }, { id: "3" }], {
    requireCurrentPlaylist: () => ({ id: "pl-1", name: "Main" }),
    resolveLocalTrackId: (track) => track.id,
    withProgress: async (_label, run) => run(() => {}),
    command: async (name, payload) => {
      capturedCommand = { name, payload };
      return { added: 3, skipped: 1 };
    },
    refreshCurrentPlaylistTracks: async () => { refreshed += 1; },
    setStatus: (text) => { status = text; }
  });

  assert.equal(capturedCommand.name, "add_tracks_to_playlist");
  assert.equal(capturedCommand.payload.playlistId, "pl-1");
  assert.deepEqual(capturedCommand.payload.trackIds, ["1", "2", "3"]);
  assert.equal(capturedCommand.payload.dedupe, "skip");
  assert.equal(refreshed, 1);
  assert.match(status, /Added 3 tracks \(skipped 1\) to Main/);
});

test("addTracksToCurrentPlaylist uses existing localTrackId for usb-origin tracks", async () => {
  let capturedCommand = null;

  await addTracksToCurrentPlaylist([{
    id: "usb-1",
    localTrackId: "local-usb-1",
    usbAnalysisPath: "/USB/PIONEER/USBANLZ/P001/TEST/ANLZ0000.DAT",
    filePath: "/USB/Contents/Test/track.mp3"
  }], {
    requireCurrentPlaylist: () => ({ id: "pl-1", name: "Main" }),
    resolveLocalTrackId: (track) => track.localTrackId || null,
    resolveLocalTrackIdAsync: async () => {
      throw new Error("async fallback should not run when localTrackId exists");
    },
    shouldAllowResolvedFallback: () => false,
    withProgress: async (_label, run) => run(() => {}),
    command: async (name, payload) => {
      capturedCommand = { name, payload };
      return { added: 1, skipped: 0 };
    },
    refreshCurrentPlaylistTracks: async () => {},
    setStatus: () => {}
  });

  assert.equal(capturedCommand.name, "add_tracks_to_playlist");
  assert.deepEqual(capturedCommand.payload.trackIds, ["local-usb-1"]);
});

test("addTracksToCurrentPlaylist does not fuzzy-resolve usb-origin tracks without localTrackId", async () => {
  let status = "";

  await addTracksToCurrentPlaylist([{
    id: "124",
    title: "The Other Side",
    artist: "Artist One",
    album: "Album One",
    usbAnalysisPath: "/USB/PIONEER/USBANLZ/P001/TEST/ANLZ0000.DAT",
    filePath: "/USB/Contents/Artist One/Album One/file.mp3"
  }], {
    requireCurrentPlaylist: () => ({ id: "pl-1", name: "Main" }),
    resolveLocalTrackId: () => null,
    resolveLocalTrackIdAsync: async () => "wrong-local-id",
    shouldAllowResolvedFallback: () => false,
    withProgress: async () => {
      throw new Error("withProgress should not run when no trackIds");
    },
    command: async () => ({ added: 0, skipped: 0 }),
    refreshCurrentPlaylistTracks: async () => {},
    setStatus: (text) => { status = text; }
  });

  assert.equal(status, "No imported track IDs found to add");
});

test("addTracksToCurrentPlaylist materializes browse-only local tracks before adding", async () => {
  let capturedCommand = null;
  let refreshed = 0;

  await addTracksToCurrentPlaylist([{
    id: "/music/Artist Two - Track One - 01 Track One.mp3",
    title: "Track One",
    artist: "Artist Two",
    filePath: "/music/Artist Two - Track One - 01 Track One.mp3"
  }], {
    requireCurrentPlaylist: () => ({ id: "pl-1", name: "Main" }),
    resolveLocalTrackId: () => null,
    resolveLocalTrackIdAsync: async () => "track-local-123",
    shouldAllowResolvedFallback: () => true,
    withProgress: async (_label, run) => run(() => {}),
    command: async (name, payload) => {
      capturedCommand = { name, payload };
      return { added: 1, skipped: 0 };
    },
    refreshCurrentPlaylistTracks: async () => { refreshed += 1; },
    setStatus: () => {}
  });

  assert.equal(capturedCommand.name, "add_tracks_to_playlist");
  assert.deepEqual(capturedCommand.payload.trackIds, ["track-local-123"]);
  assert.equal(refreshed, 1);
});
