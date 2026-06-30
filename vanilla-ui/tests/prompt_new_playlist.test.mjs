import test from "node:test";
import assert from "node:assert/strict";
import { createSingleSubmit } from "../components/playlist/actions.mjs";

test("promptNewPlaylist double-submit guard calls createPlaylist once", () => {
  let createCalls = 0;
  let cleanupCalls = 0;
  const input = { value: "My Playlist" };
  const createPlaylist = () => { createCalls += 1; };
  const cleanup = () => { cleanupCalls += 1; };

  const submit = createSingleSubmit(() => {
    const name = String(input.value || "").trim();
    cleanup();
    if (name) createPlaylist(name);
  });

  // Simulate Enter + blur firing back-to-back.
  submit();
  submit();

  assert.equal(createCalls, 1);
  assert.equal(cleanupCalls, 1);
});

