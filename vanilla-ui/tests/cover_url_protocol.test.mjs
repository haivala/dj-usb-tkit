import test from "node:test";
import assert from "node:assert/strict";
import { appendUrlRevision } from "../components/library/actions.mjs";

test("appendUrlRevision appends revisions without rewriting data URLs", () => {
  assert.equal(
    appendUrlRevision("asset://localhost/covers/art.jpg", "2026-03-29T22:00:00Z"),
    "asset://localhost/covers/art.jpg?rev=2026-03-29T22%3A00%3A00Z"
  );

  const dataUrl = "data:image/jpeg;base64,abc123";
  assert.equal(appendUrlRevision(dataUrl, "123"), dataUrl);
});
