import test from "node:test";
import assert from "node:assert/strict";
import { hydrateUsbTrackMetadata } from "../components/usb/actions.mjs";

// USB playlist/history track pages are hydrated server-side by
// fetch_usb_playlist_tracks / fetch_usb_history_tracks, which also set
// `needsHydration`. `hydrateUsbTrackMetadata` (singular, inspect_usb_track)
// survives only as the deeper re-hydrate for a clicked row that still isn't
// complete.

test("hydrateUsbTrackMetadata inspects a row flagged needsHydration and then clears the flag", async () => {
  const state = { usbRoot: "/tmp/usb" };
  const track = {
    id: "123",
    filePath: "/tmp/usb/Contents/track.mp3",
    title: "Track",
    artist: "Artist",
    needsHydration: true,
    waveformPreview: [10],
    bpm: 120,
    key: "8A",
    artworkPath: "",
    artworkUrl: ""
  };
  let inspectCalls = 0;

  const result = await hydrateUsbTrackMetadata(state, track, {
    command: async (name, payload) => {
      inspectCalls += 1;
      assert.equal(name, "inspect_usb_track");
      assert.equal(payload.trackId, "123");
      assert.equal(payload.usbRoot, "/tmp/usb");
      return { track: { id: "123", title: "Track", artist: "Artist" } };
    },
    normalizeTrack: (candidate) => ({ ...candidate })
  });

  assert.equal(result, track);
  assert.equal(inspectCalls, 1);
  assert.equal(track.artworkChecked, true);
  assert.equal(track.needsHydration, false);
});

test("hydrateUsbTrackMetadata skips a row the backend did not flag", async () => {
  const state = { usbRoot: "/tmp/usb" };
  const track = { id: "1", title: "A", artist: "Artist A", needsHydration: false };
  let commandCalls = 0;
  const result = await hydrateUsbTrackMetadata(state, track, {
    command: async () => { commandCalls += 1; return {}; },
    normalizeTrack: (t) => ({ ...t })
  });
  assert.equal(commandCalls, 0);
  assert.equal(result, track);
  assert.equal(track.artworkChecked, undefined);
});

test("hydrateUsbTrackMetadata ignores a non-numeric id (eDB-only placeholder)", async () => {
  const state = { usbRoot: "/tmp/usb" };
  const track = { id: "abc", title: "A", artist: "B", needsHydration: true };
  let commandCalls = 0;
  await hydrateUsbTrackMetadata(state, track, {
    command: async () => { commandCalls += 1; return {}; },
    normalizeTrack: (t) => ({ ...t })
  });
  assert.equal(commandCalls, 0);
});
