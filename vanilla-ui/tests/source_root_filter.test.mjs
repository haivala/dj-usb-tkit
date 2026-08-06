import test from "node:test";
import assert from "node:assert/strict";
import {
  normalizePath,
  trackPathMatchesAnyRoot,
  enabledSourceRoots,
  scanLibraryButtonLabel
} from "../components/library/actions.mjs";

// ---------------------------------------------------------------------------
// normalizePath
// ---------------------------------------------------------------------------

test("normalizePath lowercases and trims", () => {
  assert.equal(normalizePath("  /Music/DJ  "), "/music/dj");
});

test("normalizePath converts backslashes to forward slashes", () => {
  assert.equal(normalizePath("C:\\Users\\DJ\\music"), "c:/users/dj/music");
});

test("normalizePath handles null/undefined/empty", () => {
  assert.equal(normalizePath(null), "");
  assert.equal(normalizePath(undefined), "");
  assert.equal(normalizePath(""), "");
  assert.equal(normalizePath("   "), "");
});

// ---------------------------------------------------------------------------
// trackPathMatchesAnyRoot
// ---------------------------------------------------------------------------

test("trackPathMatchesAnyRoot returns true for path under root", () => {
  assert.equal(
    trackPathMatchesAnyRoot("/music/artist/track.mp3", ["/music"]),
    true
  );
});

test("trackPathMatchesAnyRoot returns true for path exactly matching root", () => {
  assert.equal(
    trackPathMatchesAnyRoot("/music", ["/music"]),
    true
  );
});

test("trackPathMatchesAnyRoot returns false for path outside root", () => {
  assert.equal(
    trackPathMatchesAnyRoot("/other/track.mp3", ["/music"]),
    false
  );
});

test("trackPathMatchesAnyRoot returns false for partial directory name match", () => {
  // /music-extra should NOT match root /music
  assert.equal(
    trackPathMatchesAnyRoot("/music-extra/track.mp3", ["/music"]),
    false
  );
});

test("trackPathMatchesAnyRoot matches any of multiple roots", () => {
  const roots = ["/music/a", "/music/b"];
  assert.equal(trackPathMatchesAnyRoot("/music/a/track.mp3", roots), true);
  assert.equal(trackPathMatchesAnyRoot("/music/b/track.mp3", roots), true);
  assert.equal(trackPathMatchesAnyRoot("/music/c/track.mp3", roots), false);
});

test("trackPathMatchesAnyRoot returns false for empty roots array", () => {
  assert.equal(
    trackPathMatchesAnyRoot("/music/track.mp3", []),
    false
  );
});

test("trackPathMatchesAnyRoot returns false for empty file path", () => {
  assert.equal(trackPathMatchesAnyRoot("", ["/music"]), false);
  assert.equal(trackPathMatchesAnyRoot(null, ["/music"]), false);
});

test("trackPathMatchesAnyRoot handles trailing slashes on root", () => {
  assert.equal(
    trackPathMatchesAnyRoot("/music/track.mp3", ["/music/"]),
    true
  );
  assert.equal(
    trackPathMatchesAnyRoot("/music/track.mp3", ["/music///"]),
    true
  );
});

test("trackPathMatchesAnyRoot is case-insensitive", () => {
  assert.equal(
    trackPathMatchesAnyRoot("/Music/Artist/Track.mp3", ["/music"]),
    true
  );
});

test("trackPathMatchesAnyRoot normalizes backslashes in file path", () => {
  assert.equal(
    trackPathMatchesAnyRoot("C:\\Users\\DJ\\music\\track.mp3", ["C:\\Users\\DJ\\music"]),
    true
  );
});

// ---------------------------------------------------------------------------
// enabledSourceRoots
// ---------------------------------------------------------------------------

test("enabledSourceRoots defaults all roots to enabled", () => {
  const roots = ["/a", "/b"];
  assert.deepEqual(enabledSourceRoots(roots, {}), roots);
});

test("enabledSourceRoots excludes roots explicitly set to false", () => {
  const roots = ["/a", "/b", "/c"];
  assert.deepEqual(enabledSourceRoots(roots, { "/b": false }), ["/a", "/c"]);
});

test("enabledSourceRoots excludes missing roots without changing enabled map", () => {
  const roots = ["/a", "/b", "/c"];
  assert.deepEqual(enabledSourceRoots(roots, { "/b": true }, new Set(["/b"])), ["/a", "/c"]);
});

// ---------------------------------------------------------------------------
// scanLibraryButtonLabel
// ---------------------------------------------------------------------------

test("scanLibraryButtonLabel is singular for zero or one root", () => {
  assert.equal(scanLibraryButtonLabel([]), "Scan Library");
  assert.equal(scanLibraryButtonLabel(["/music"]), "Scan Library");
});

test("scanLibraryButtonLabel is plural for multiple roots", () => {
  assert.equal(scanLibraryButtonLabel(["/music/a", "/music/b"]), "Scan Libraries");
});
