import test from "node:test";
import assert from "node:assert/strict";
import {
  normalizeTrack,
  normalizeUsbPlaylist
} from "../components/library/actions.mjs";

// The backend sends tracks fully typed (models.rs Track / UsbTrack);
// normalizeTrack only adds the frontend-owned bits.
const CTX = { toPlayableUrl: (path) => (path ? `asset://localhost${path}` : null) };

test("normalizeTrack passes backend fields through and adds the cover's asset URL", () => {
  const track = {
    id: "1",
    title: "Song",
    artist: "Artist",
    bpm: 128,
    waveformPreview: [0, 40, 100],
    formatCompat: { severity: "ok" },
    artworkPath: "/covers/a.jpg",
    updatedAt: "2026-01-01T00:00:00Z"
  };
  const normalized = normalizeTrack(CTX, track);

  assert.equal(normalized.bpm, 128);
  assert.deepEqual(normalized.waveformPreview, [0, 40, 100]);
  assert.deepEqual(normalized.formatCompat, { severity: "ok" });
  assert.equal(normalized.artworkUrl, "asset://localhost/covers/a.jpg?rev=2026-01-01T00%3A00%3A00Z");
  assert.equal(normalized.artworkChecked, false);
  assert.equal(normalized.needsHydration, false);
});

test("normalizeTrack fills the display defaults for an untitled row", () => {
  const normalized = normalizeTrack(CTX, { id: "2", title: "", artist: "" });
  assert.equal(normalized.title, "Unknown Title");
  assert.equal(normalized.artist, "Unknown Artist");
  assert.equal(normalized.artworkUrl, "");
});

test("normalizeUsbPlaylist normalizes each track", () => {
  const playlist = normalizeUsbPlaylist(CTX, {
    id: "u1",
    name: "USB Set",
    source: "pdb",
    trackCount: 2,
    tracks: [{ id: "t1", title: "A", artist: "B" }, { id: "t2", title: "", artist: "D", needsHydration: true }]
  });

  assert.equal(playlist.trackCount, 2);
  assert.equal(playlist.tracks[1].title, "Unknown Title");
  assert.equal(playlist.tracks[1].needsHydration, true);
});
