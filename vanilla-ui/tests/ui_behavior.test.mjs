import { test } from "node:test";
import assert from "node:assert/strict";
import {
  detectExternalMasterDb
} from "../components/usb/actions.mjs";

function makeClassList() {
  const classes = new Set();
  return {
    add(name) { classes.add(name); },
    remove(name) { classes.delete(name); },
    toggle(name, force) {
      if (typeof force === "boolean") {
        if (force) classes.add(name);
        else classes.delete(name);
        return;
      }
      if (classes.has(name)) classes.delete(name);
      else classes.add(name);
    },
    contains(name) { return classes.has(name); }
  };
}

test("detectExternalMasterDb populates state and calls renderSourceChips when DB is found", async () => {
  const toggleClassList = makeClassList();
  const state = { externalMasterDbPath: null, masterDbEnabled: false };
  const el = {
    externalMasterDbToggle: { classList: toggleClassList },
  };

  const dbPath = "/Users/me/Library/vendor/master.db";
  let chipsRendered = false;
  await detectExternalMasterDb(state, el, {
    command: async () => ({ found: true, path: dbPath }),
    warn: () => {},
    renderSourceChips: () => { chipsRendered = true; }
  });

  assert.equal(state.externalMasterDbPath, dbPath);
  assert.ok(chipsRendered, "renderSourceChips should be called");
  assert.ok(toggleClassList.contains("hidden"),
    "legacy toggle row should always be hidden (chip in source-chips replaces it)");
});

test("detectExternalMasterDb resets state and calls renderSourceChips when DB not found", async () => {
  const toggleClassList = makeClassList();
  const state = { externalMasterDbPath: "/old/path", masterDbEnabled: true };
  const el = {
    externalMasterDbToggle: { classList: toggleClassList },
  };

  let chipsRendered = false;
  await detectExternalMasterDb(state, el, {
    command: async () => ({ found: false, path: null }),
    warn: () => {},
    renderSourceChips: () => { chipsRendered = true; }
  });

  assert.equal(state.externalMasterDbPath, null);
  assert.ok(!state.masterDbEnabled, "masterDbEnabled should be cleared when DB not found");
  assert.ok(chipsRendered, "renderSourceChips should be called");
  assert.ok(toggleClassList.contains("hidden"),
    "legacy toggle row should always be hidden");
});

test("detectExternalMasterDb handles command failure gracefully", async () => {
  const warnings = [];
  const state = { externalMasterDbPath: "/old" };
  const el = {
    externalMasterDbToggle: { classList: makeClassList() },
    externalMasterDbCheckbox: { checked: true },
    externalMasterDbPath: { textContent: "x", title: "x" }
  };

  await detectExternalMasterDb(state, el, {
    command: async () => { throw new Error("disk error"); },
    warn: (...args) => { warnings.push(args); }
  });

  assert.equal(state.externalMasterDbPath, null);
  assert.ok(warnings.length > 0, "should have logged a warning");
});
