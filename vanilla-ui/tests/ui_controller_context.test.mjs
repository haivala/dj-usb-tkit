import test from "node:test";
import assert from "node:assert/strict";
import { createBindEventsContext } from "../ui_controller.mjs";

test("createBindEventsContext composes state, el, and deps", () => {
  const state = { a: 1 };
  const el = { b: 2 };
  const ctx = createBindEventsContext(state, el, { c: 3 });
  assert.equal(ctx.state, state);
  assert.equal(ctx.el, el);
  assert.equal(ctx.c, 3);
});
