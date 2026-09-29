import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { STORAGE_KEY_UPDATE_DISMISSED } from "../settings_keys.mjs";
import {
  dismissUpdateBanner,
  renderUpdateBanner,
  renderUpdateNotice
} from "../update_check.mjs";

// The update check itself (fetch + version compare + severity) is backend-owned
// now -- see backend/src/service/update_check.rs and its unit tests. This file
// only covers rendering the `state.updateCheck` verdict.

function makeEls() {
  const dom = new JSDOM(`<!doctype html><body>
    <span id="note" class="hidden"><a href="#" class="update-note-link"></a></span>
    <div id="banner" class="hidden"><span id="text"><span class="update-banner-message"></span> — <a href="#" class="update-banner-link">view release</a></span></div>
  </body>`);
  const document = dom.window.document;
  return {
    settingsUpdateNote: document.querySelector("#note"),
    updateBanner: document.querySelector("#banner"),
    updateBannerText: document.querySelector("#text")
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

test("renderUpdateBanner handles visibility, dismissal, links, and persistence", () => {
  const state = {
    updateCheck: {
      updateAvailable: true,
      severity: "critical",
      latestVersion: "0.1.5",
      releaseUrl: "https://example.com/v0.1.5"
    }
  };

  const el = makeEls();
  renderUpdateBanner({ state: { updateCheck: null }, el });
  assert.equal(el.updateBanner.classList.contains("hidden"), true);
  renderUpdateBanner({ state: { updateCheck: { severity: "normal" } }, el });
  assert.equal(el.updateBanner.classList.contains("hidden"), true);

  let opened = null;
  renderUpdateBanner({
    state,
    el,
    localStorage: fakeStorage(),
    window: { open: (url) => { opened = url; } }
  });
  assert.equal(el.updateBanner.classList.contains("hidden"), false);
  assert.match(el.updateBannerText.textContent, /0\.1\.5/);
  el.updateBannerText.querySelector(".update-banner-link").dispatchEvent(
    new el.updateBannerText.ownerDocument.defaultView.Event("click", { bubbles: true, cancelable: true })
  );
  assert.equal(opened, "https://example.com/v0.1.5");

  renderUpdateBanner({ state, el, localStorage: fakeStorage({ [STORAGE_KEY_UPDATE_DISMISSED]: "0.1.5" }) });
  assert.equal(el.updateBanner.classList.contains("hidden"), true);
  renderUpdateBanner({ state, el, localStorage: fakeStorage({ [STORAGE_KEY_UPDATE_DISMISSED]: "0.1.4" }) });
  assert.equal(el.updateBanner.classList.contains("hidden"), false);

  const storage = fakeStorage();
  dismissUpdateBanner({ state: { updateCheck: { latestVersion: "0.1.5" } }, el, localStorage: storage });
  assert.equal(el.updateBanner.classList.contains("hidden"), true);
  assert.equal(storage.getItem(STORAGE_KEY_UPDATE_DISMISSED), "0.1.5");
});
