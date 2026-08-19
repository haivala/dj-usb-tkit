import test from "node:test";
import assert from "node:assert/strict";
import {
  appendUrlRevision,
  buildCoverSrcCandidates,
  convertFileSrcLocal
} from "../components/library/actions.mjs";

test("convertFileSrcLocal converts local paths and preserves protocol URLs", () => {
  assert.equal(convertFileSrcLocal("/music/track.mp3"), "asset://localhost/music/track.mp3");
  assert.equal(
    convertFileSrcLocal("/my music/Artist #1.mp3"),
    "asset://localhost/my%20music/Artist%20%231.mp3"
  );
  assert.equal(
    convertFileSrcLocal("/mnt\\data\\music\\track.flac"),
    "asset://localhost/mnt/data/music/track.flac"
  );

  for (const empty of ["", null, undefined, "   "]) {
    assert.equal(convertFileSrcLocal(empty), null);
  }

  for (const url of [
    "asset://localhost/covers/a.jpg",
    "tauri://localhost/covers/a.jpg",
    "file:///tmp/a.jpg"
  ]) {
    assert.equal(convertFileSrcLocal(url), url);
  }

  const converted = convertFileSrcLocal("/music/track.mp3");
  assert.ok(!converted.startsWith("http://asset.localhost"), `got: ${converted}`);
  assert.ok(!converted.startsWith("https://asset.localhost"), `got: ${converted}`);
});

test("buildCoverSrcCandidates orders cover sources and removes unusable entries", () => {
  const candidates = buildCoverSrcCandidates({
    artworkDataUrl: "data:image/jpeg;base64,abc123",
    artworkPath: "/covers/art.jpg",
    artworkUrl: "asset://localhost/covers/art.jpg"
  }, {
    toPlayableUrl: (path) => `file://${path}`
  });

  assert.deepEqual(candidates, [
    "data:image/jpeg;base64,abc123",
    "file:///covers/art.jpg",
    "file://covers/art.jpg",
    "asset://localhost/covers/art.jpg"
  ]);
  assert.equal(candidates.length, new Set(candidates).size);
  assert.ok(candidates.every((url) => !url.startsWith("http://asset.localhost")));
  assert.ok(candidates.every((url) => !url.startsWith("https://asset.localhost")));

  assert.deepEqual(buildCoverSrcCandidates({
    artworkPath: "tauri://localhost/covers/protocol.jpg",
    artworkUrl: "asset://localhost/covers/fallback.jpg"
  }), [
    "tauri://localhost/covers/protocol.jpg",
    "asset://localhost/covers/fallback.jpg"
  ]);
  assert.deepEqual(buildCoverSrcCandidates({}), []);
  assert.deepEqual(buildCoverSrcCandidates(null), []);
});

test("appendUrlRevision appends revisions without rewriting data URLs", () => {
  assert.equal(
    appendUrlRevision("asset://localhost/covers/art.jpg", "2026-03-29T22:00:00Z"),
    "asset://localhost/covers/art.jpg?rev=2026-03-29T22%3A00%3A00Z"
  );

  const dataUrl = "data:image/jpeg;base64,abc123";
  assert.equal(appendUrlRevision(dataUrl, "123"), dataUrl);
});
