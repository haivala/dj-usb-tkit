import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";

import { createEventLogStore } from "../event_log.mjs";
import { pushEventLog, ensureEventLogSourceOptions, renderEventLog } from "../components/event-log/actions.mjs";
import { escapeHtml } from "../ui_utils.mjs";

function makeDom() {
  return new JSDOM(`
    <!doctype html>
    <body>
      <select id="eventLogLevelFilter">
        <option value="all">all</option>
        <option value="warn">warn</option>
      </select>
      <select id="eventLogSourceFilter">
        <option value="all">all</option>
      </select>
      <div id="eventLogSummary"></div>
      <div id="eventLogList"></div>
    </body>
  `);
}

test("pushEventLog updates state from store and rerenders active event log tab", () => {
  const state = { activeTab: "event-log", eventLogEntries: [] };
  const store = createEventLogStore({ maxEntries: 10 });
  let renders = 0;

  pushEventLog(state, store, () => { renders += 1; }, {
    source: "console",
    level: "info",
    message: "hello"
  });

  assert.equal(state.eventLogEntries.length, 1);
  assert.equal(renders, 1);
});

test("ensureEventLogSourceOptions appends new sources and preserves current value when valid", () => {
  const dom = makeDom();
  const document = dom.window.document;
  const state = {
    eventLogEntries: [
      { source: "browser" },
      { source: "console" }
    ]
  };
  const el = {
    eventLogSourceFilter: document.getElementById("eventLogSourceFilter")
  };
  el.eventLogSourceFilter.value = "all";

  ensureEventLogSourceOptions(state, el, document);

  const values = Array.from(el.eventLogSourceFilter.options).map((opt) => opt.value);
  assert.deepEqual(values, ["all", "browser", "console"]);
});

test("renderEventLog summarizes rows and renders escaped content", () => {
  const dom = makeDom();
  const document = dom.window.document;
  const state = {
    eventLogEntries: [
      {
        ts: new Date("2026-04-07T10:20:30Z").getTime(),
        level: "warn",
        source: "console",
        code: "console.event",
        message: "<danger>",
        details: "details",
        count: 2
      }
    ]
  };
  const el = {
    eventLogLevelFilter: document.getElementById("eventLogLevelFilter"),
    eventLogSourceFilter: document.getElementById("eventLogSourceFilter"),
    eventLogSummary: document.getElementById("eventLogSummary"),
    eventLogList: document.getElementById("eventLogList")
  };

  renderEventLog(state, el, document, {
    ensureEventLogSourceOptions: () => ensureEventLogSourceOptions(state, el, document),
    escapeHtml
  });

  assert.equal(el.eventLogSummary.textContent, "1 event(s) (2 occurrences)");
  assert.match(el.eventLogList.innerHTML, /&lt;danger&gt;/);
  assert.match(el.eventLogList.innerHTML, /x2/);
});
