import test from "node:test";
import assert from "node:assert/strict";
import {
  normalizePath,
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
