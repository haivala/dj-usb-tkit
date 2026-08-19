import test from "node:test";
import assert from "node:assert/strict";
import {
  hydrateUsbTrackMetadata,
  hydrateUsbTrackMetadataBatch
} from "../components/usb/actions.mjs";

test("hydrateUsbTrackMetadata marks inspected no-artwork tracks as checked", async () => {
  const state = { usbRoot: "/tmp/usb" };
  const track = {
    id: "123",
    filePath: "/tmp/usb/Contents/track.mp3",
    title: "Track",
    artist: "Artist",
    waveformPreview: [10],
    bpm: 120,
    key: "8A",
    artworkPath: "",
    artworkUrl: ""
  };
  let inspectCalls = 0;

  const result = await hydrateUsbTrackMetadata(state, track, {
    usbTrackNeedsHydration: (candidate) => {
      assert.equal(candidate, track);
      return true;
    },
    command: async (name, payload) => {
      inspectCalls += 1;
      assert.equal(name, "inspect_usb_track");
      assert.equal(payload.trackId, "123");
      assert.equal(payload.usbRoot, "/tmp/usb");
      return {
        track: {
          id: "123",
          title: "Track",
          artist: "Artist",
          waveformPreview: [10],
          bpm: 120,
          key: "8A",
          artworkPath: "",
          artworkUrl: ""
        }
      };
    },
    normalizeTrack: (candidate) => ({ ...candidate })
  });

  assert.equal(result, track);
  assert.equal(inspectCalls, 1);
  assert.equal(track.artworkChecked, true);
  assert.equal(track.artworkPath, "");
  assert.equal(track.artworkUrl, "");
});

test("hydrateUsbTrackMetadataBatch sends one inspect_usb_tracks call and applies results by id", async () => {
  const state = { usbRoot: "/tmp/usb" };
  const trackA = { id: "1", filePath: "/a.mp3", title: "A", artist: "Artist A" };
  const trackB = { id: "2", filePath: "/b.mp3", title: "B", artist: "Artist B" };
  let commandCalls = 0;

  await hydrateUsbTrackMetadataBatch(state, [trackA, trackB], {
    usbTrackNeedsHydration: () => true,
    command: async (name, payload) => {
      commandCalls += 1;
      assert.equal(name, "inspect_usb_tracks");
      assert.equal(payload.usbRoot, "/tmp/usb");
      assert.deepEqual(payload.items.map((item) => item.trackId), ["1", "2"]);
      return {
        items: [
          { trackId: "1", source: "pdb", track: { id: "1", bpm: 120 } },
          { trackId: "2", source: "eDB", track: { id: "2", bpm: 128 } }
        ]
      };
    },
    normalizeTrack: (candidate) => ({ ...candidate })
  });

  assert.equal(commandCalls, 1);
  assert.equal(trackA.bpm, 120);
  assert.equal(trackA.artworkChecked, true);
  assert.equal(trackB.bpm, 128);
  assert.equal(trackB.artworkChecked, true);
});

test("hydrateUsbTrackMetadataBatch skips tracks that do not need hydration", async () => {
  const state = { usbRoot: "/tmp/usb" };
  const track = { id: "1", title: "A", artist: "Artist A" };
  let commandCalls = 0;

  const tracks = await hydrateUsbTrackMetadataBatch(state, [track], {
    usbTrackNeedsHydration: () => false,
    command: async () => {
      commandCalls += 1;
      return { items: [] };
    },
    normalizeTrack: (candidate) => ({ ...candidate })
  });

  assert.equal(commandCalls, 0);
  assert.equal(tracks[0], track);
  assert.equal(track.artworkChecked, undefined);
});

test("hydrateUsbTrackMetadataBatch handles unresolved and partial inspection results", async () => {
  const state = { usbRoot: "/tmp/usb" };
  const trackA = { id: "1", title: "Existing Title", artist: "Existing Artist", durationMs: 180000 };
  const trackB = { id: "999999", title: "Unresolved Title", artist: "Unresolved Artist" };

  await hydrateUsbTrackMetadataBatch(state, [trackA, trackB], {
    usbTrackNeedsHydration: () => true,
    command: async () => ({
      items: [
        { trackId: "1", source: "pdb", track: { id: "1", bpm: 120 } },
        { trackId: "999999", source: null, track: null }
      ]
    }),
    normalizeTrack: (candidate) => ({
      ...candidate,
      title: candidate.title || "Unknown Title",
      artist: candidate.artist || "Unknown Artist",
      durationMs: candidate.durationMs ?? null
    })
  });

  assert.equal(trackA.title, "Existing Title");
  assert.equal(trackA.artist, "Existing Artist");
  assert.equal(trackA.durationMs, 180000);
  assert.equal(trackA.bpm, 120);
  assert.equal(trackA.artworkChecked, true);
  assert.equal(trackB.artworkChecked, true);
  assert.equal(trackB.title, "Unresolved Title");
  assert.equal(trackB.artist, "Unresolved Artist");
});
