import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import {
  parseSemver,
  compareSemver,
  releaseIsCritical,
  fetchUpdateInfo,
  renderUpdateNotice,
  renderCriticalUpdateBanner,
  dismissCriticalUpdateBanner
} from "../update_check.mjs";

function release({ tag_name, draft = false, prerelease = false, body = "", html_url = "" }) {
  return { tag_name, draft, prerelease, body, html_url };
}

function fetchFnReturning(releases, ok = true) {
  return async () => ({ ok, json: async () => releases });
}

test("parseSemver parses plain and v-prefixed tags", () => {
  assert.deepEqual(parseSemver("0.1.4"), [0, 1, 4]);
  assert.deepEqual(parseSemver("v0.1.4"), [0, 1, 4]);
  assert.deepEqual(parseSemver("V1.2.3"), [1, 2, 3]);
});

test("parseSemver returns null for malformed tags", () => {
  assert.equal(parseSemver("not-a-version"), null);
  assert.equal(parseSemver(""), null);
  assert.equal(parseSemver(null), null);
  assert.equal(parseSemver(undefined), null);
});

test("compareSemver orders versions", () => {
  assert.equal(compareSemver([0, 1, 3], [0, 1, 4]), -1);
  assert.equal(compareSemver([0, 2, 0], [0, 1, 9]), 1);
  assert.equal(compareSemver([1, 0, 0], [1, 0, 0]), 0);
});

test("releaseIsCritical matches the markdown severity marker", () => {
  assert.equal(releaseIsCritical("**Severity:** critical\n\n- fix"), true);
  assert.equal(releaseIsCritical("Severity: Critical"), true);
  assert.equal(releaseIsCritical("- fix a bug\n- nothing critical here"), false);
  assert.equal(releaseIsCritical(""), false);
  assert.equal(releaseIsCritical(null), false);
});

test("fetchUpdateInfo reports no update when latest release is not newer", async () => {
  const info = await fetchUpdateInfo("0.1.3", {
    fetchFn: fetchFnReturning([release({ tag_name: "v0.1.3" })])
  });
  assert.equal(info.updateAvailable, false);
  assert.equal(info.severity, "none");
  assert.equal(info.latestVersion, "0.1.3");
});

test("fetchUpdateInfo reports a normal update for a newer non-critical release", async () => {
  const info = await fetchUpdateInfo("0.1.3", {
    fetchFn: fetchFnReturning([
      release({ tag_name: "v0.1.4", body: "- some fixes", html_url: "https://example.com/v0.1.4" })
    ])
  });
  assert.equal(info.updateAvailable, true);
  assert.equal(info.severity, "normal");
  assert.equal(info.latestVersion, "0.1.4");
  assert.equal(info.releaseUrl, "https://example.com/v0.1.4");
});

test("fetchUpdateInfo reports critical when any newer release is marked critical", async () => {
  const info = await fetchUpdateInfo("0.1.3", {
    fetchFn: fetchFnReturning([
      release({ tag_name: "v0.1.4", body: "- minor fix" }),
      release({ tag_name: "v0.1.5", body: "**Severity:** critical\n\n- security fix" })
    ])
  });
  assert.equal(info.updateAvailable, true);
  assert.equal(info.severity, "critical");
  assert.equal(info.latestVersion, "0.1.5");
});

test("fetchUpdateInfo excludes draft and prerelease entries", async () => {
  const info = await fetchUpdateInfo("0.1.3", {
    fetchFn: fetchFnReturning([
      release({ tag_name: "v0.1.9", draft: true }),
      release({ tag_name: "v0.1.8", prerelease: true })
    ])
  });
  assert.equal(info.updateAvailable, false);
});

test("fetchUpdateInfo swallows network and parse errors", async () => {
  const rejecting = await fetchUpdateInfo("0.1.3", {
    fetchFn: async () => { throw new Error("offline"); }
  });
  assert.equal(rejecting.updateAvailable, false);

  const notOk = await fetchUpdateInfo("0.1.3", { fetchFn: fetchFnReturning([], false) });
  assert.equal(notOk.updateAvailable, false);

  const badShape = await fetchUpdateInfo("0.1.3", {
    fetchFn: async () => ({ ok: true, json: async () => ({ not: "an array" }) })
  });
  assert.equal(badShape.updateAvailable, false);
});

test("fetchUpdateInfo returns no update without throwing for an unparsable current version", async () => {
  const info = await fetchUpdateInfo("Not set", {
    fetchFn: fetchFnReturning([release({ tag_name: "v0.1.4" })])
  });
  assert.equal(info.updateAvailable, false);
});

function makeSettingsEl() {
  const dom = new JSDOM(`<!doctype html><body><span id="note" class="hidden"></span></body>`);
  return { settingsUpdateNote: dom.window.document.querySelector("#note") };
}

test("renderUpdateNotice hides the note when no update is available", () => {
  const el = makeSettingsEl();
  const state = { updateCheck: { updateAvailable: false } };
  renderUpdateNotice(state, el);
  assert.equal(el.settingsUpdateNote.classList.contains("hidden"), true);
});

test("renderUpdateNotice shows a link for normal and critical updates alike", () => {
  const el = makeSettingsEl();
  const state = {
    updateCheck: {
      updateAvailable: true,
      severity: "normal",
      latestVersion: "0.1.4",
      releaseUrl: "https://example.com/release"
    }
  };
  let opened = null;
  renderUpdateNotice(state, el, { openUrl: (url) => { opened = url; } });
  assert.equal(el.settingsUpdateNote.classList.contains("hidden"), false);
  assert.match(el.settingsUpdateNote.textContent, /0\.1\.4/);

  el.settingsUpdateNote.querySelector(".update-note-link").dispatchEvent(
    new el.settingsUpdateNote.ownerDocument.defaultView.Event("click", { bubbles: true, cancelable: true })
  );
  assert.equal(opened, "https://example.com/release");
});

function makeBannerEl() {
  const dom = new JSDOM(
    `<!doctype html><body><div id="banner" class="hidden"><span id="text"></span></div></body>`
  );
  return {
    criticalUpdateBanner: dom.window.document.querySelector("#banner"),
    criticalUpdateText: dom.window.document.querySelector("#text")
  };
}

function fakeStorage(initial = {}) {
  const store = { ...initial };
  return {
    getItem: (k) => (k in store ? store[k] : null),
    setItem: (k, v) => { store[k] = v; }
  };
}

test("renderCriticalUpdateBanner stays hidden for non-critical or missing update info", () => {
  const el = makeBannerEl();
  renderCriticalUpdateBanner({ updateCheck: null }, el);
  assert.equal(el.criticalUpdateBanner.classList.contains("hidden"), true);

  renderCriticalUpdateBanner(
    { updateCheck: { updateAvailable: true, severity: "normal", latestVersion: "0.1.4" } },
    el
  );
  assert.equal(el.criticalUpdateBanner.classList.contains("hidden"), true);
});

test("renderCriticalUpdateBanner shows for a critical update and respects a matching dismissal", () => {
  const el = makeBannerEl();
  const state = {
    updateCheck: {
      updateAvailable: true,
      severity: "critical",
      latestVersion: "0.1.5",
      releaseUrl: "https://example.com/v0.1.5"
    }
  };

  renderCriticalUpdateBanner(state, el, { localStorageObj: fakeStorage() });
  assert.equal(el.criticalUpdateBanner.classList.contains("hidden"), false);
  assert.match(el.criticalUpdateText.textContent, /0\.1\.5/);

  renderCriticalUpdateBanner(state, el, { localStorageObj: fakeStorage({ "djusbtkit.updateDismissedVersion": "0.1.5" }) });
  assert.equal(el.criticalUpdateBanner.classList.contains("hidden"), true);

  renderCriticalUpdateBanner(state, el, { localStorageObj: fakeStorage({ "djusbtkit.updateDismissedVersion": "0.1.4" }) });
  assert.equal(el.criticalUpdateBanner.classList.contains("hidden"), false);
});

test("dismissCriticalUpdateBanner hides the banner and persists the dismissed version", () => {
  const el = makeBannerEl();
  el.criticalUpdateBanner.classList.remove("hidden");
  const storage = fakeStorage();
  const state = { updateCheck: { latestVersion: "0.1.5" } };

  dismissCriticalUpdateBanner(state, el, { localStorageObj: storage });

  assert.equal(el.criticalUpdateBanner.classList.contains("hidden"), true);
  assert.equal(storage.getItem("djusbtkit.updateDismissedVersion"), "0.1.5");
});
