import test from "node:test";
import assert from "node:assert/strict";
import { escapeHtml } from "../ui_utils.mjs";

test("escapeHtml escapes all HTML special characters", () => {
  const input = `<div class="x">'&"</div>`;
  const output = escapeHtml(input);
  assert.equal(output, "&lt;div class=&quot;x&quot;&gt;&#39;&amp;&quot;&lt;/div&gt;");
});

test("escapeHtml returns empty string for nullish values", () => {
  assert.equal(escapeHtml(null), "");
  assert.equal(escapeHtml(undefined), "");
});

test("escapeHtml neutralizes inline event handler payloads", () => {
  const payload = `<img src=x onerror='alert("xss")'>`;
  const escaped = escapeHtml(payload);
  assert.equal(
    escaped,
    "&lt;img src=x onerror=&#39;alert(&quot;xss&quot;)&#39;&gt;"
  );
  assert.equal(escaped.includes("<"), false);
  assert.equal(escaped.includes(">"), false);
});
