import test from "node:test";
import assert from "node:assert/strict";
import { toPlayableUrl } from "../components/playback/actions.mjs";

test("toPlayableUrl keeps protocol urls unchanged", () => {
  assert.equal(toPlayableUrl("https://example.com/a.mp3"), "https://example.com/a.mp3");
  assert.equal(toPlayableUrl("file:///tmp/a.mp3"), "file:///tmp/a.mp3");
  assert.equal(toPlayableUrl("data:audio/wav;base64,AAA"), "data:audio/wav;base64,AAA");
});

test("toPlayableUrl converts absolute unix paths", () => {
  assert.equal(toPlayableUrl("/tmp/audio/a b.mp3"), "file:///tmp/audio/a%20b.mp3");
});

test("toPlayableUrl converts windows paths", () => {
  assert.equal(toPlayableUrl("C:\\Music\\Track.mp3"), "file:///C:/Music/Track.mp3");
});

test("toPlayableUrl uses tauriConvertFileSrc when available", () => {
  const out = toPlayableUrl("/tmp/a.mp3", {
    isTauriRuntime: () => true,
    tauriConvertFileSrc: (v) => `asset://${v}`,
    windowObj: {}
  });
  assert.equal(out, "asset:///tmp/a.mp3");
});

test("toPlayableUrl falls back to window tauri convertFileSrc", () => {
  const out = toPlayableUrl("/tmp/a.mp3", {
    isTauriRuntime: () => false,
    windowObj: {
      __TAURI__: {
        core: {
          convertFileSrc: (v) => `asset-win://${v}`
        }
      }
    }
  });
  assert.equal(out, "asset-win:///tmp/a.mp3");
});
