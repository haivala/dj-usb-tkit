import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { STORAGE_KEY_UPDATE_DISMISSED } from "../settings_keys.mjs";
import {
  dismissCriticalUpdateBanner,
  renderCriticalUpdateBanner,
  renderUpdateNotice
} from "../update_check.mjs";

// The update check itself (fetch + version compare + severity) is backend-owned
// now -- see backend/src/service/update_check.rs and its unit tests. This file
// only covers rendering the `state.updateCheck` verdict.

function makeEls() {
  const dom = new JSDOM(`<!doctype html><body>
    <span id="note" class="hidden"></span>
    <div id="banner" class="hidden"><span id="text"></span></div>
  </body>`);
  const document = dom.window.document;
  return {
    settingsUpdateNote: document.querySelector("#note"),
    criticalUpdateBanner: document.querySelector("#banner"),
    criticalUpdateText: document.querySelector("#text")
  };
}

function fakeStorage(initial = {}) {
  const store = { ...initial };
  return {
    getItem: (key) => (key in store ? store[key] : null),
    setItem: (key, value) => {
      store[key] = value;
    }
  };
}

test("renderUpdateNotice toggles the note and opens the release link", () => {
  const el = makeEls();
  renderUpdateNotice({ updateCheck: { updateAvailable: false } }, el);
  assert.equal(el.settingsUpdateNote.classList.contains("hidden"), true);
  assert.equal(el.settingsUpdateNote.textContent, "");

  let opened = null;
  renderUpdateNotice({
    updateCheck: {
      updateAvailable: true,
      severity: "normal",
      latestVersion: "0.1.4",
      releaseUrl: "https://example.com/release"
    }
  }, el, { openUrl: (url) => { opened = url; } });

  assert.equal(el.settingsUpdateNote.classList.contains("hidden"), false);
  assert.match(el.settingsUpdateNote.textContent, /0\.1\.4/);
  el.settingsUpdateNote.querySelector(".update-note-link").dispatchEvent(
    new el.settingsUpdateNote.ownerDocument.defaultView.Event("click", { bubbles: true, cancelable: true })
  );
  assert.equal(opened, "https://example.com/release");
});

test("renderCriticalUpdateBanner handles visibility, dismissal, links, and persistence", () => {
  const state = {
    updateCheck: {
      updateAvailable: true,
      severity: "critical",
      latestVersion: "0.1.5",
      releaseUrl: "https://example.com/v0.1.5"
    }
  };

  const el = makeEls();
  renderCriticalUpdateBanner({ updateCheck: null }, el);
  assert.equal(el.criticalUpdateBanner.classList.contains("hidden"), true);
  renderCriticalUpdateBanner({ updateCheck: { severity: "normal" } }, el);
  assert.equal(el.criticalUpdateBanner.classList.contains("hidden"), true);

  let opened = null;
  renderCriticalUpdateBanner(state, el, {
    localStorageObj: fakeStorage(),
    openUrl: (url) => { opened = url; }
  });
  assert.equal(el.criticalUpdateBanner.classList.contains("hidden"), false);
  assert.match(el.criticalUpdateText.textContent, /0\.1\.5/);
  el.criticalUpdateText.querySelector(".critical-update-link").dispatchEvent(
    new el.criticalUpdateText.ownerDocument.defaultView.Event("click", { bubbles: true, cancelable: true })
  );
  assert.equal(opened, "https://example.com/v0.1.5");

  renderCriticalUpdateBanner(state, el, {
    localStorageObj: fakeStorage({ [STORAGE_KEY_UPDATE_DISMISSED]: "0.1.5" })
  });
  assert.equal(el.criticalUpdateBanner.classList.contains("hidden"), true);
  renderCriticalUpdateBanner(state, el, {
    localStorageObj: fakeStorage({ [STORAGE_KEY_UPDATE_DISMISSED]: "0.1.4" })
  });
  assert.equal(el.criticalUpdateBanner.classList.contains("hidden"), false);

  const storage = fakeStorage();
  dismissCriticalUpdateBanner({ updateCheck: { latestVersion: "0.1.5" } }, el, { localStorageObj: storage });
  assert.equal(el.criticalUpdateBanner.classList.contains("hidden"), true);
  assert.equal(storage.getItem(STORAGE_KEY_UPDATE_DISMISSED), "0.1.5");
});
