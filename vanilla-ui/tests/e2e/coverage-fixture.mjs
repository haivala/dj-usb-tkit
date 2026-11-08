// Drop-in test/expect replacement for @playwright/test. Every page gets the
// shared stubbed backend (backend_mock.js) installed first. When COVERAGE is
// unset nothing else changes. When COVERAGE=1, it wraps each test's page
// in V8 JS coverage collection and appends the raw per-test coverage entry
// for dist/main.js to .coverage-e2e/raw-v8-coverage.jsonl, one JSON object
// per line. Run `node scripts/report-e2e-coverage.mjs` afterwards to merge
// those entries and print a per-source-file line-coverage table.
import { test as base, expect } from "@playwright/test";
import { appendFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const COVERAGE_ENABLED = !!process.env.COVERAGE;
const coverageOutDir = path.resolve(__dirname, "../../.coverage-e2e");
const rawOutFile = path.join(coverageOutDir, "raw-v8-coverage.jsonl");
const backendMockPath = path.join(__dirname, "backend_mock.js");
// The UI's English texts (`get_ui_texts`), kept current by a backend test.
const uiTextsEn = readFileSync(path.join(__dirname, "ui_texts.en.json"), "utf8");
// The same texts in the test language (`[Ŝéééţţîîñĝš]`), for `uiTextCheck`.
const uiTextsTestLanguage = readFileSync(path.join(__dirname, "ui_texts.en-XA.json"), "utf8");
const allowlistDir = path.join(__dirname, "ui_text_allowlist");

export const test = base.extend({
  page: async ({ page }, use) => {
    // The shared stubbed backend (window.__e2eBackend) every spec builds on.
    await page.addInitScript({ content: `window.__e2eUiTexts = ${uiTextsEn};` });
    await page.addInitScript({ path: backendMockPath });
    if (!COVERAGE_ENABLED) {
      await use(page);
      return;
    }
    await page.coverage.startJSCoverage({ resetOnNavigation: false });
    await use(page);
    const coverage = await page.coverage.stopJSCoverage();
    const entry = coverage.find((e) => e.url.endsWith("/main.js"));
    if (entry) {
      mkdirSync(coverageOutDir, { recursive: true });
      appendFileSync(rawOutFile, JSON.stringify(entry) + "\n");
    }
  }
});

export { expect };

// The page's UI texts in the test language from the next load on, so `uiTextCheck` can tell
// them from English left in index.html or JS. Call before `page.goto`.
export function useTestLanguage(page) {
  return page.addInitScript({ content: `window.__e2eUiTexts = ${uiTextsTestLanguage};` });
}

// Text the app's UI shows in every language, so it's never in the catalog.
const NEVER_TRANSLATED = ["DJ USB Tkit", "FLAC", "ALAC", "AIFF", "WAV", "MP3", "AAC"];

// Collects, from the visible page, every text and label that's neither from the catalog (in
// the test language, so wrapped in brackets) nor data. Data is a string the mocked backend
// answered with (track titles, paths, names, and the response texts the backend's own tests
// check), a path's last part, or anything inside `translate="no"` (the standard attribute
// for text that's never translated, such as the logo). What's left is English the UI holds
// itself, in index.html or JS.
function collectUnwrapped({ neverTranslated }) {
  const data = new Set();
  for (const value of window.__e2eAnswerStrings || []) {
    const text = String(value).replace(/\s+/g, " ").trim();
    if (!text) continue;
    data.add(text);
    if (text.includes("/")) {
      for (const part of text.split("/")) {
        if (part.length < 3) continue;
        data.add(part);
        data.add(part.replace(/\.[A-Za-z0-9]{1,5}$/, ""));
      }
    }
  }
  const dataLongestFirst = [...data].sort((a, b) => b.length - a.length);
  const english = (text) => {
    let rest = text;
    // Catalog text; a placeholder's value sits inside the brackets.
    for (let before = ""; before !== rest; ) {
      before = rest;
      rest = rest.replace(/\[[^[\]]*\]/g, " ");
    }
    for (const value of dataLongestFirst) {
      if (rest.includes(value)) rest = rest.split(value).join(" ");
    }
    return /[A-Za-z]/.test(rest);
  };
  const hits = new Set();
  const consider = (text, label) => {
    const normalized = String(text || "").replace(/\s+/g, " ").trim();
    if (!normalized || neverTranslated.includes(normalized) || !english(normalized)) return;
    hits.add(label ? `${label}: ${normalized}` : normalized);
  };
  const shown = (element) => {
    const host = element.closest("option") ? element.closest("select") || element : element;
    return host.checkVisibility({ visibilityProperty: true, checkVisibilityCSS: true });
  };
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const parent = node.parentElement;
    if (!parent || parent.closest("script, style, template, [translate=no]") || !shown(parent)) continue;
    consider(node.nodeValue);
  }
  for (const element of document.body.querySelectorAll("[aria-label], [title], [placeholder], [data-tooltip], [alt]")) {
    if (!shown(element) || element.closest("[translate=no]")) continue;
    for (const attribute of ["aria-label", "title", "placeholder", "data-tooltip", "alt"]) {
      if (element.hasAttribute(attribute)) consider(element.getAttribute(attribute), attribute);
    }
  }
  return [...hits];
}

// The test-language check: every text and label the visible page shows comes from the
// catalog, or is data. English not moved yet is listed in `ui_text_allowlist/<name>.json`,
// which may only shrink: an entry the page no longer shows fails the check, so a piece that
// moves text deletes its entries (`UPDATE_UI_TEXT_ALLOWLIST=1` rewrites the list without
// them, and refuses to add any).
//
//   const check = uiTextCheck(page, "usb-pages");
//   await check.collect();          // on each page or state the test opens
//   await check.expectAllowed();    // once, at the end
export function uiTextCheck(page, name) {
  const found = new Set();
  const file = path.join(allowlistDir, `${name}.json`);
  return {
    async collect() {
      // The progress bar shows a finished job's text for a moment, then fades out still
      // showing "Idle" (its visibility flips only when the fade ends): wait until it's gone,
      // so every run sees the same.
      await expect(page.locator("#progressFooter")).toBeHidden();
      for (const hit of await page.evaluate(collectUnwrapped, { neverTranslated: NEVER_TRANSLATED })) {
        found.add(hit);
      }
    },
    async expectAllowed() {
      const listed = existsSync(file) ? JSON.parse(readFileSync(file, "utf8")) : null;
      const added = [...found].filter((text) => !listed?.includes(text)).sort();
      if (process.env.UPDATE_UI_TEXT_ALLOWLIST) {
        if (listed) expect(added, `English to move to the catalog, not to ${file}`).toEqual([]);
        mkdirSync(allowlistDir, { recursive: true });
        writeFileSync(file, JSON.stringify([...found].sort(), null, 2) + "\n");
        return;
      }
      const gone = (listed || []).filter((text) => !found.has(text));
      expect(added, `English on screen that isn't from the catalog (see ${file})`).toEqual([]);
      expect(gone, `No longer shown: delete from ${file} (UPDATE_UI_TEXT_ALLOWLIST=1)`).toEqual([]);
    }
  };
}
