import test from "node:test";
import assert from "node:assert/strict";

import { readLibraryPagination } from "../components/library/actions.mjs";

test("readLibraryPagination supports camelCase fields", () => {
  const out = readLibraryPagination({ nextCursor: "abc", hasMore: true });
  assert.equal(out.nextCursor, "abc");
  assert.equal(out.hasMore, true);
});

test("readLibraryPagination supports snake_case fields", () => {
  const out = readLibraryPagination({ next_cursor: "def", has_more: true });
  assert.equal(out.nextCursor, "def");
  assert.equal(out.hasMore, true);
});

test("readLibraryPagination defaults hasMore from nextCursor", () => {
  assert.deepEqual(readLibraryPagination({ next_cursor: "x" }), { nextCursor: "x", hasMore: true });
  assert.deepEqual(readLibraryPagination({ nextCursor: null }), { nextCursor: null, hasMore: false });
});
