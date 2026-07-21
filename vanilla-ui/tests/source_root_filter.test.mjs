import test from "node:test";
import assert from "node:assert/strict";
import {
  normalizePath,
  trackPathMatchesAnyRoot,
  enabledSourceRoots,
  filterTracksBySourceRoots,
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
// filterTracksBySourceRoots
// ---------------------------------------------------------------------------

test("filterTracksBySourceRoots keeps tracks under enabled roots", () => {
  const tracks = [
    { id: "1", filePath: "/music/a/one.mp3" },
    { id: "2", filePath: "/music/b/two.mp3" },
    { id: "3", filePath: "/music/c/three.mp3" }
  ];
  const roots = ["/music/a", "/music/b", "/music/c"];
  const filtered = filterTracksBySourceRoots(tracks, roots, { "/music/b": false });
  assert.deepEqual(filtered.map((t) => t.id), ["1", "3"]);
});

test("filterTracksBySourceRoots returns empty when no roots are enabled", () => {
  const tracks = [{ id: "1", filePath: "/music/a/one.mp3" }];
  const roots = ["/music/a"];
  const filtered = filterTracksBySourceRoots(tracks, roots, { "/music/a": false });
  assert.deepEqual(filtered, []);
});

test("filterTracksBySourceRoots preserves incoming sorted order", () => {
  const sortedTracks = [
    { id: "3", filePath: "/music/a/c.mp3", artist: "A" },
    { id: "1", filePath: "/music/b/a.mp3", artist: "B" },
    { id: "2", filePath: "/music/a/b.mp3", artist: "C" }
  ];
  const roots = ["/music/a", "/music/b"];
  const filtered = filterTracksBySourceRoots(sortedTracks, roots, { "/music/b": false });
  assert.deepEqual(filtered.map((t) => t.id), ["3", "2"]);
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
