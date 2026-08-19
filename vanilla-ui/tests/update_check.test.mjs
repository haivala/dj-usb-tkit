import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { STORAGE_KEY_UPDATE_DISMISSED } from "../settings_keys.mjs";
import {
  compareSemver,
  dismissCriticalUpdateBanner,
  fetchUpdateInfo,
  parseSemver,
  releaseIsCritical,
  renderCriticalUpdateBanner,
  renderUpdateNotice
} from "../update_check.mjs";

function release({ tag_name, draft = false, prerelease = false, body = "", html_url = "" }) {
  return { tag_name, draft, prerelease, body, html_url };
}

function fetchFnReturning(releases, ok = true) {
  return async () => ({ ok, json: async () => releases });
}

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

test("version helpers parse, compare, and classify release severity", () => {
  for (const [tag, expected] of [
    ["0.1.4", [0, 1, 4]],
    ["v0.1.4", [0, 1, 4]],
    ["V1.2.3", [1, 2, 3]]
  ]) {
    assert.deepEqual(parseSemver(tag), expected);
  }
  for (const tag of ["not-a-version", "", null, undefined]) {
    assert.equal(parseSemver(tag), null);
  }
  for (const [a, b, expected] of [
    [[0, 1, 3], [0, 1, 4], -1],
    [[0, 2, 0], [0, 1, 9], 1],
    [[1, 0, 0], [1, 0, 0], 0]
  ]) {
    assert.equal(compareSemver(a, b), expected);
  }
  for (const [body, expected] of [
    ["**Severity:** critical\n\n- fix", true],
    ["Severity: Critical", true],
    ["- fix a bug\n- nothing critical here", false],
    ["", false],
    [null, false]
  ]) {
    assert.equal(releaseIsCritical(body), expected);
  }
});

test("fetchUpdateInfo reports update availability and severity from stable releases", async () => {
  for (const { name, releases, expected } of [
    {
      name: "latest release is current",
      releases: [release({ tag_name: "v0.1.3" })],
      expected: { updateAvailable: false, severity: "none", latestVersion: "0.1.3" }
    },
    {
      name: "new normal release",
      releases: [
        release({ tag_name: "v0.1.4", body: "- fixes", html_url: "https://example.com/v0.1.4" })
      ],
      expected: {
        updateAvailable: true,
        severity: "normal",
        latestVersion: "0.1.4",
        releaseUrl: "https://example.com/v0.1.4"
      }
    },
    {
      name: "any newer critical release marks the update critical",
      releases: [
        release({ tag_name: "v0.1.4", body: "- minor fix" }),
        release({ tag_name: "v0.1.5", body: "**Severity:** critical\n\n- security fix" })
      ],
      expected: { updateAvailable: true, severity: "critical", latestVersion: "0.1.5" }
    },
    {
      name: "drafts and prereleases are ignored",
      releases: [
        release({ tag_name: "v0.1.9", draft: true }),
        release({ tag_name: "v0.1.8", prerelease: true })
      ],
      expected: { updateAvailable: false, severity: "none", latestVersion: "0.1.3" }
    }
  ]) {
    const info = await fetchUpdateInfo("0.1.3", { fetchFn: fetchFnReturning(releases) });
    assert.deepEqual(
      Object.fromEntries(Object.keys(expected).map((key) => [key, info[key]])),
      expected,
      name
    );
  }
});

test("fetchUpdateInfo falls back without throwing for invalid inputs and failed fetches", async () => {
  const cases = [
    { currentVersion: "Not set", fetchFn: fetchFnReturning([release({ tag_name: "v0.1.4" })]) },
    { currentVersion: "0.1.3", fetchFn: async () => { throw new Error("offline"); } },
    { currentVersion: "0.1.3", fetchFn: fetchFnReturning([], false) },
    { currentVersion: "0.1.3", fetchFn: async () => ({ ok: true, json: async () => ({ not: "array" }) }) }
  ];

  for (const { currentVersion, fetchFn } of cases) {
    const info = await fetchUpdateInfo(currentVersion, { fetchFn });
    assert.equal(info.updateAvailable, false);
  }
});

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
