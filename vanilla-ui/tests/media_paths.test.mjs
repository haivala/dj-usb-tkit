import test from "node:test";
import assert from "node:assert/strict";
import { toPlayableUrl } from "../components/playback/actions.mjs";

test("toPlayableUrl keeps protocol urls unchanged", () => {
  assert.equal(toPlayableUrl({}, "https://example.com/a.mp3"), "https://example.com/a.mp3");
  assert.equal(toPlayableUrl({}, "file:///tmp/a.mp3"), "file:///tmp/a.mp3");
  assert.equal(toPlayableUrl({}, "data:audio/wav;base64,AAA"), "data:audio/wav;base64,AAA");
});

test("toPlayableUrl loads a local file through Tauri's asset protocol", () => {
  const out = toPlayableUrl({
    isTauriRuntime: () => true,
    tauriConvertFileSrc: (v) => `asset://${v}`
  }, "/tmp/a.mp3");
  assert.equal(out, "asset:///tmp/a.mp3");
});

test("toPlayableUrl has no url for an empty path", () => {
  assert.equal(toPlayableUrl({}, ""), null);
  assert.equal(toPlayableUrl({}, null), null);
});
