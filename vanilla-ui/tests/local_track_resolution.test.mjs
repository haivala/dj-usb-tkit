import test from "node:test";
import assert from "node:assert/strict";
import {
  resolveLocalTrackId,
  shouldAllowResolvedFallback,
  resolveLocalTrackIdAsync,
  resolveLocalTrack,
  resolveLocalTrackForPlayback,
  getTrackPlaybackPath,
  isTrackCurrentlyPlaying
} from "../components/playback/actions.mjs";

function normalizePath(value) {
  return String(value || "").replace(/\\/g, "/").trim().toLowerCase();
}

test("resolveLocalTrackId prefers materialized id match", () => {
  const state = {
    tracks: [
      { id: "local-1", filePath: "/music/a.mp3", title: "A", artist: "AA" }
    ]
  };
  const id = resolveLocalTrackId({ id: "local-1", filePath: "/usb/a.mp3" }, state, { normalizePath });
  assert.equal(id, "local-1");
});

test("shouldAllowResolvedFallback blocks usb-origin candidates", () => {
  const state = { usbRoot: "/usb" };
  assert.equal(
    shouldAllowResolvedFallback({ filePath: "/usb/Contents/a.mp3" }, state, { normalizePath }),
    false
  );
  assert.equal(
    shouldAllowResolvedFallback({ filePath: "/music/a.mp3" }, state, { normalizePath }),
    true
  );
});

test("resolveLocalTrackIdAsync materializes local track and promotes identity", async () => {
  const state = { tracks: [] };
  const calls = [];
  let promoted = null;
  const track = { id: "usb-1", filePath: "/music/a.mp3", title: "A", artist: "AA" };

  const id = await resolveLocalTrackIdAsync(track, state, {
    command: async (name, payload) => {
      calls.push({ name, payload });
      if (name === "materialize_source_track") return { trackId: "local-99" };
      throw new Error(`unexpected command ${name}`);
    },
    normalizePath,
    promoteTrackIdentity: (from, to) => {
      promoted = { from, to };
    },
    resolveLocalTrackId: () => null,
    shouldAllowResolvedFallback: () => true
  });

  assert.equal(id, "local-99");
  assert.equal(track.localTrackId, "local-99");
  assert.deepEqual(promoted, { from: "usb-1", to: "local-99" });
  assert.equal(calls[0].name, "materialize_source_track");
});

test("resolveLocalTrack finds exact file path candidate", () => {
  const state = {
    tracks: [
      { id: "x1", filePath: "/music/a.mp3", title: "Wrong", artist: "Wrong" },
      { id: "x2", filePath: "/music/b.mp3", title: "B", artist: "BB" }
    ]
  };
  const resolved = resolveLocalTrack({ filePath: "/music/b.mp3", title: "Other", artist: "Other" }, state);
  assert.equal(resolved?.id, "x2");
});

test("resolveLocalTrackForPlayback uses search fallback and returns best match", async () => {
  const state = { tracks: [] };
  const result = await resolveLocalTrackForPlayback({ title: "Song A", artist: "Artist A" }, state, {
    command: async (name) => {
      if (name !== "search_tracks") throw new Error("unexpected command");
      return {
        items: [
          { id: "weak", title: "Song A", artist: "Else", filePath: "/music/else.mp3" },
          { id: "best", title: "Song A", artist: "Artist A", filePath: "/music/song-a.mp3" }
        ]
      };
    },
    normalizeTrack: (t) => t,
    resolveLocalTrack: () => null
  });
  assert.equal(result?.id, "best");
});

test("isTrackCurrentlyPlaying compares normalized playback path", () => {
  const state = {
    playbackActive: true,
    playbackTrackId: null,
    playbackPath: "C:/Music/A.MP3"
  };
  const active = isTrackCurrentlyPlaying({ filePath: "c:\\music\\a.mp3" }, state, {
    normalizePath,
    getTrackPlaybackPath: (track) => getTrackPlaybackPath(track, { resolveLocalTrack: () => null })
  });
  assert.equal(active, true);
});
