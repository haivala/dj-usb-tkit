import test from "node:test";
import assert from "node:assert/strict";
import { bindShellEvents } from "../components/shell/events.mjs";
import { bindSettingsEvents } from "../components/settings/events.mjs";
import { bindEventLogEvents } from "../components/event-log/events.mjs";
import { bindPlaylistEvents } from "../components/playlist/events.mjs";
import { bindUsbEvents } from "../components/usb/events.mjs";

test("component event modules are importable and callable", () => {
  assert.equal(typeof bindShellEvents, "function");
  assert.equal(typeof bindSettingsEvents, "function");
  assert.equal(typeof bindEventLogEvents, "function");
  assert.equal(typeof bindPlaylistEvents, "function");
  assert.equal(typeof bindUsbEvents, "function");
});
