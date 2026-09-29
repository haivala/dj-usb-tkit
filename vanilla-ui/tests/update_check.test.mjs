import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { renderUpdateNotice } from "../update_check.mjs";

// The update check itself (fetch + version compare + severity) is backend-owned
// now -- see backend/src/service/update_check.rs and its unit tests. This file
// only covers rendering the `state.updateCheck` verdict.

function makeEls() {
  const dom = new JSDOM(`<!doctype html><body>
    <span id="note" class="hidden"><a href="#" class="update-note-link"></a></span>
  </body>`);
  const document = dom.window.document;
  return {
    settingsUpdateNote: document.querySelector("#note")
  };
}

test("renderUpdateNotice toggles the note and opens the release link", () => {
  const el = makeEls();
  renderUpdateNotice({ state: { updateCheck: { updateAvailable: false } }, el });
  assert.equal(el.settingsUpdateNote.classList.contains("hidden"), true);
  assert.equal(el.settingsUpdateNote.textContent, "");

  let opened = null;
  renderUpdateNotice({
    state: {
      updateCheck: {
        updateAvailable: true,
        severity: "normal",
        latestVersion: "0.1.4",
        releaseUrl: "https://example.com/release"
      }
    },
    el,
    window: { open: (url) => { opened = url; } }
  });

  assert.equal(el.settingsUpdateNote.classList.contains("hidden"), false);
  assert.match(el.settingsUpdateNote.textContent, /0\.1\.4/);
  el.settingsUpdateNote.querySelector(".update-note-link").dispatchEvent(
    new el.settingsUpdateNote.ownerDocument.defaultView.Event("click", { bubbles: true, cancelable: true })
  );
  assert.equal(opened, "https://example.com/release");
});
