import test from "node:test";
import assert from "node:assert/strict";
import { convertFileSrcLocal, buildCoverSrcCandidates, appendUrlRevision } from "../components/library/actions.mjs";

// ---------------------------------------------------------------------------
// convertFileSrcLocal
// ---------------------------------------------------------------------------

test("convertFileSrcLocal returns asset://localhost URL", () => {
  const result = convertFileSrcLocal("/music/track.mp3");
  assert.equal(result, "asset://localhost/music/track.mp3");
});

test("convertFileSrcLocal does NOT return http://asset.localhost", () => {
  const result = convertFileSrcLocal("/music/track.mp3");
  assert.ok(!result.startsWith("http://asset.localhost"), `got: ${result}`);
  assert.ok(!result.startsWith("https://asset.localhost"), `got: ${result}`);
  assert.ok(result.startsWith("asset://localhost"), `got: ${result}`);
});

test("convertFileSrcLocal encodes spaces and special chars per segment", () => {
  const result = convertFileSrcLocal("/my music/Artist One — 寂.mp3");
  assert.ok(result.includes("%20"), "spaces should be encoded");
  assert.ok(result.startsWith("asset://localhost/"));
  assert.ok(!result.includes(" "), "no literal spaces");
});

test("convertFileSrcLocal returns null for empty input", () => {
  assert.equal(convertFileSrcLocal(""), null);
  assert.equal(convertFileSrcLocal(null), null);
  assert.equal(convertFileSrcLocal(undefined), null);
  assert.equal(convertFileSrcLocal("   "), null);
});

test("convertFileSrcLocal normalizes backslashes to forward slashes", () => {
  const result = convertFileSrcLocal("/home/dj/music/track.flac");
  assert.ok(!result.includes("\\"), "no backslashes in output");
  assert.ok(result.startsWith("asset://localhost/"));
  // Also verify backslash input is normalized
  const winPath = convertFileSrcLocal("/mnt\\data\\music\\track.flac");
  assert.ok(!winPath.includes("\\"), "backslashes normalized");
  assert.ok(winPath.startsWith("asset://localhost/"));
});

test("convertFileSrcLocal preserves already-protocol URLs", () => {
  assert.equal(
    convertFileSrcLocal("asset://localhost/covers/a.jpg"),
    "asset://localhost/covers/a.jpg"
  );
  assert.equal(
    convertFileSrcLocal("tauri://localhost/covers/a.jpg"),
    "tauri://localhost/covers/a.jpg"
  );
  assert.equal(
    convertFileSrcLocal("file:///tmp/a.jpg"),
    "file:///tmp/a.jpg"
  );
});

// ---------------------------------------------------------------------------
// buildCoverSrcCandidates
// ---------------------------------------------------------------------------

test("buildCoverSrcCandidates never produces http://asset.localhost URLs", () => {
  const track = { artworkPath: "/covers/art.jpg", artworkUrl: "", artworkDataUrl: "" };
  const candidates = buildCoverSrcCandidates(track);
  for (const url of candidates) {
    assert.ok(!url.startsWith("http://asset.localhost"), `bad URL: ${url}`);
    assert.ok(!url.startsWith("https://asset.localhost"), `bad URL: ${url}`);
  }
});

test("buildCoverSrcCandidates includes artworkDataUrl first when present", () => {
  const track = {
    artworkDataUrl: "data:image/jpeg;base64,abc123",
    artworkPath: "/covers/art.jpg"
  };
  const candidates = buildCoverSrcCandidates(track);
  assert.equal(candidates[0], "data:image/jpeg;base64,abc123");
  assert.ok(candidates.length >= 2);
});

test("buildCoverSrcCandidates uses asset://localhost for artworkPath", () => {
  const track = { artworkPath: "/covers/art.jpg" };
  const candidates = buildCoverSrcCandidates(track);
  const assetUrl = candidates.find((u) => u.startsWith("asset://localhost"));
  assert.ok(assetUrl, `expected asset://localhost URL in: ${JSON.stringify(candidates)}`);
});

test("buildCoverSrcCandidates deduplicates entries", () => {
  const track = {
    artworkPath: "/covers/art.jpg",
    artworkUrl: "asset://localhost/covers/art.jpg" // same as convertFileSrcLocal output
  };
  const candidates = buildCoverSrcCandidates(track);
  const unique = new Set(candidates);
  assert.equal(candidates.length, unique.size, `duplicates found: ${JSON.stringify(candidates)}`);
});

test("buildCoverSrcCandidates returns empty array for track with no artwork", () => {
  const candidates = buildCoverSrcCandidates({});
  assert.deepEqual(candidates, []);
});

test("buildCoverSrcCandidates returns empty array for null track", () => {
  const candidates = buildCoverSrcCandidates(null);
  assert.deepEqual(candidates, []);
});

test("buildCoverSrcCandidates uses toPlayableUrl dep when provided", () => {
  const track = { artworkPath: "/covers/art.jpg" };
  const candidates = buildCoverSrcCandidates(track, {
    toPlayableUrl: (p) => `file://${p}`
  });
  assert.ok(candidates.includes("file:///covers/art.jpg"));
});

test("buildCoverSrcCandidates keeps protocol artworkPath first before artworkUrl", () => {
  const track = {
    artworkPath: "tauri://localhost/covers/protocol.jpg",
    artworkUrl: "asset://localhost/covers/fallback.jpg"
  };
  const candidates = buildCoverSrcCandidates(track);
  assert.equal(candidates[0], "tauri://localhost/covers/protocol.jpg");
  assert.equal(candidates[1], "asset://localhost/covers/fallback.jpg");
});

test("appendUrlRevision adds a rev query for asset URLs", () => {
  assert.equal(
    appendUrlRevision("asset://localhost/covers/art.jpg", "2026-03-29T22:00:00Z"),
    "asset://localhost/covers/art.jpg?rev=2026-03-29T22%3A00%3A00Z"
  );
});

test("appendUrlRevision leaves data URLs unchanged", () => {
  const dataUrl = "data:image/jpeg;base64,abc123";
  assert.equal(appendUrlRevision(dataUrl, "123"), dataUrl);
});
