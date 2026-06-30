import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";
import fs from "node:fs";

const require = createRequire(import.meta.url);
const { scoreLocalTrackCandidate, selectBestLocalMatch } = require("../playback_match.js");
const __dirname = path.dirname(fileURLToPath(import.meta.url));
const fixtureRoot = path.resolve(__dirname, "../../backend/tests/fixtures/audio");
const fixtureLocalA = path.join(fixtureRoot, "embedded/track_embedded.mp3");
const fixtureLocalB = path.join(fixtureRoot, "folder/track_folder.jpg.mp3");
const fixtureLocalC = path.join(fixtureRoot, "parent/child/track_parent_folder.jpg.mp3");

test("fixture files are present", () => {
  assert.equal(fs.existsSync(fixtureLocalA), true);
  assert.equal(fs.existsSync(fixtureLocalB), true);
  assert.equal(fs.existsSync(fixtureLocalC), true);
});

test("selects local track when title+artist+album match", () => {
  const source = {
    title: "Track Alpha",
    artist: "Artist One",
    album: "Album One",
    filePath: "/mnt/media_device/contents/Artist One/Album One/01-track_alpha.mp3",
    bpm: 174.0,
  };
  const candidates = [
    {
      title: "Track Alpha",
      artist: "Artist One",
      album: "Album One",
      filePath: fixtureLocalA,
      bpm: 174.0,
    },
    {
      title: "Different Track",
      artist: "Other Artist",
      album: "Other",
      filePath: fixtureLocalB,
      bpm: 128.0,
    },
  ];

  const best = selectBestLocalMatch(source, candidates, 16);
  assert.equal(best?.filePath, candidates[0].filePath);
});

test("prefers exact filename/path similarity over weak metadata", () => {
  const usbStylePath = `/mnt/media_device/contents/Artist/Album/${path.basename(fixtureLocalB)}`;
  const source = {
    title: "",
    artist: "",
    filePath: usbStylePath,
  };
  const byPath = {
    title: "Local Title",
    artist: "Local Artist",
    filePath: fixtureLocalB,
  };
  const weakMeta = {
    title: "Unknown Title",
    artist: "Unknown Artist",
    filePath: fixtureLocalC,
  };

  const best = selectBestLocalMatch(source, [weakMeta, byPath], 16);
  assert.equal(best?.filePath, byPath.filePath);
});

test("returns null when only weak candidate exists", () => {
  const source = {
    title: "Song A",
    artist: "Artist A",
    album: "Album A",
    filePath: `/mnt/media_device/contents/A/Album A/${path.basename(fixtureLocalA)}`,
  };
  const weak = {
    title: "Song A",
    artist: "Different Artist",
    album: "Different Album",
    filePath: fixtureLocalC,
  };

  const best = selectBestLocalMatch(source, [weak], 16);
  assert.equal(best, null);
});

test("score function is deterministic for same input", () => {
  const source = {
    title: "Track Beta",
    artist: "Artist Two",
    album: "Album Two",
    filePath: `/mnt/media_device/contents/Artist Two/Album Two/${path.basename(fixtureLocalA)}`,
    bpm: 150.19,
  };
  const candidate = {
    title: "Track Beta",
    artist: "Artist Two",
    album: "Album Two",
    filePath: fixtureLocalA,
    bpm: 150.20,
  };

  const a = scoreLocalTrackCandidate(candidate, source);
  const b = scoreLocalTrackCandidate(candidate, source);
  assert.equal(a, b);
  assert.ok(a >= 16);
});
