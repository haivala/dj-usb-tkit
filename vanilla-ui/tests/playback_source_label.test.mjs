import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { getPlaybackSourceLabel } = require("../playback_source_label.js");

test("usb origin resolved to configured library root is labeled Library (matched)", () => {
  const label = getPlaybackSourceLabel({
    origin: "usb",
    libraryResolved: true,
    hasUsbContext: true,
    resolvedPath: "/Music/House/track.mp3",
    sourceRoots: ["/Music"]
  });
  assert.equal(label, "Library (matched)");
});

test("usb origin resolved outside configured library roots is labeled USB", () => {
  const label = getPlaybackSourceLabel({
    origin: "usb",
    libraryResolved: true,
    hasUsbContext: true,
    resolvedPath: "/media/USB1/Contents/track.mp3",
    sourceRoots: ["/Music"]
  });
  assert.equal(label, "USB");
});

test("local origin resolved inside configured roots is labeled Library", () => {
  const label = getPlaybackSourceLabel({
    origin: "local",
    libraryResolved: true,
    resolvedPath: "/Music/House/track.mp3",
    sourceRoots: ["/Music"]
  });
  assert.equal(label, "Library");
});

test("usb without library match is labeled USB", () => {
  const label = getPlaybackSourceLabel({
    origin: "usb",
    libraryResolved: false,
    hasUsbContext: true,
    resolvedPath: "",
    sourceRoots: ["/Music"]
  });
  assert.equal(label, "USB");
});

test("local origin resolved outside configured roots is labeled Local file", () => {
  const label = getPlaybackSourceLabel({
    origin: "local",
    libraryResolved: true,
    resolvedPath: "/media/USB1/Contents/track.mp3",
    sourceRoots: ["/Music"]
  });
  assert.equal(label, "Local file");
});

test("usb origin without usb context is not labeled USB", () => {
  const label = getPlaybackSourceLabel({
    origin: "usb",
    libraryResolved: false,
    hasUsbContext: false,
    resolvedPath: "",
    sourceRoots: ["/Music"]
  });
  assert.equal(label, "Local file");
});
