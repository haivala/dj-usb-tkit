// Drop-in test/expect replacement for @playwright/test. When COVERAGE is
// unset it's a pure passthrough. When COVERAGE=1, it wraps each test's page
// in V8 JS coverage collection and appends the raw per-test coverage entry
// for dist/main.js to .coverage-e2e/raw-v8-coverage.jsonl, one JSON object
// per line. Run `node scripts/report-e2e-coverage.mjs` afterwards to merge
// those entries and print a per-source-file line-coverage table.
import { test as base, expect } from "@playwright/test";
import { appendFileSync, mkdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const COVERAGE_ENABLED = !!process.env.COVERAGE;
const coverageOutDir = path.resolve(__dirname, "../../.coverage-e2e");
const rawOutFile = path.join(coverageOutDir, "raw-v8-coverage.jsonl");

export const test = base.extend({
  page: async ({ page }, use) => {
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
