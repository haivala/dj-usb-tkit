import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { getPlaybackSourceLabel } = require("../playback_source_label.js");

test("usb origin resolved to a genuine local track is labeled Library (matched)", () => {
  const label = getPlaybackSourceLabel({
    origin: "usb",
    libraryResolved: true,
    hasUsbContext: true
  });
  assert.equal(label, "Library (matched)");
});

test("history origin resolved to a genuine local track is labeled Library (matched)", () => {
  const label = getPlaybackSourceLabel({
    origin: "history",
    libraryResolved: true,
    hasUsbContext: true
  });
  assert.equal(label, "Library (matched)");
});

test("local origin resolved to a genuine local track is labeled Library", () => {
  const label = getPlaybackSourceLabel({
    origin: "local",
    libraryResolved: true,
    hasUsbContext: false
  });
  assert.equal(label, "Library");
});

test("usb origin without a library match falls back to USB when usb context is present", () => {
  const label = getPlaybackSourceLabel({
    origin: "usb",
    libraryResolved: false,
    hasUsbContext: true
  });
  assert.equal(label, "USB");
});

test("local origin without a library match is labeled Local file", () => {
  const label = getPlaybackSourceLabel({
    origin: "local",
    libraryResolved: false,
    hasUsbContext: false
  });
  assert.equal(label, "Local file");
});

test("usb origin without usb context and no library match is labeled Local file", () => {
  const label = getPlaybackSourceLabel({
    origin: "usb",
    libraryResolved: false,
    hasUsbContext: false
  });
  assert.equal(label, "Local file");
});
