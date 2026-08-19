import test from "node:test";
import assert from "node:assert/strict";
import {
  normalizePath,
  trackPathMatchesAnyRoot,
  enabledSourceRoots,
  scanLibraryButtonLabel
} from "../components/library/actions.mjs";

test("normalizePath normalizes case, slashes, and empty values", () => {
  for (const [input, expected] of [
    ["  /Music/DJ  ", "/music/dj"],
    ["C:\\Users\\DJ\\music", "c:/users/dj/music"],
    [null, ""],
    [undefined, ""],
    ["", ""],
    ["   ", ""]
  ]) {
    assert.equal(normalizePath(input), expected);
  }
});

test("trackPathMatchesAnyRoot matches normalized complete root boundaries", () => {
  for (const [filePath, roots, expected] of [
    ["/music/artist/track.mp3", ["/music"], true],
    ["/music", ["/music"], true],
    ["/other/track.mp3", ["/music"], false],
    ["/music-extra/track.mp3", ["/music"], false],
    ["/music/a/track.mp3", ["/music/a", "/music/b"], true],
    ["/music/b/track.mp3", ["/music/a", "/music/b"], true],
    ["/music/c/track.mp3", ["/music/a", "/music/b"], false],
    ["/music/track.mp3", [], false],
    ["", ["/music"], false],
    [null, ["/music"], false],
    ["/music/track.mp3", ["/music/"], true],
    ["/music/track.mp3", ["/music///"], true],
    ["/Music/Artist/Track.mp3", ["/music"], true],
    ["C:\\Users\\DJ\\music\\track.mp3", ["C:\\Users\\DJ\\music"], true]
  ]) {
    assert.equal(trackPathMatchesAnyRoot(filePath, roots), expected);
  }
});

test("enabledSourceRoots applies defaults, disabled roots, and missing roots", () => {
  const roots = ["/a", "/b", "/c"];
  assert.deepEqual(enabledSourceRoots(["/a", "/b"], {}), ["/a", "/b"]);
  assert.deepEqual(enabledSourceRoots(roots, { "/b": false }), ["/a", "/c"]);
  assert.deepEqual(enabledSourceRoots(roots, { "/b": true }, new Set(["/b"])), ["/a", "/c"]);
});

test("scanLibraryButtonLabel pluralizes by root count", () => {
  assert.equal(scanLibraryButtonLabel([]), "Scan Library");
  assert.equal(scanLibraryButtonLabel(["/music"]), "Scan Library");
  assert.equal(scanLibraryButtonLabel(["/music/a", "/music/b"]), "Scan Libraries");
});
