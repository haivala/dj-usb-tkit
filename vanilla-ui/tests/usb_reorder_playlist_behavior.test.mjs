import test from "node:test";
import assert from "node:assert/strict";
import { moveArrayItem } from "../components/usb/actions.mjs";

test("moveArrayItem returns reordered copies without mutating the input", () => {
  const input = ["a", "b", "c"];

  assert.deepEqual(moveArrayItem(["a", "b", "c"], 0, 2), ["b", "c", "a"]);
  assert.deepEqual(moveArrayItem(["a", "b", "c"], 2, 0), ["c", "a", "b"]);
  assert.deepEqual(moveArrayItem(["a", "b", "c"], 0, 1), ["b", "a", "c"]);
  assert.deepEqual(moveArrayItem(["a", "b", "c"], 1, 1), ["a", "b", "c"]);
  assert.deepEqual(moveArrayItem(["only"], 0, 0), ["only"]);
  moveArrayItem(input, 0, 2);
  assert.deepEqual(input, ["a", "b", "c"]);
});
