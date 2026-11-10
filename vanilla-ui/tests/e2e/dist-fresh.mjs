// Playwright globalSetup: the e2e tests run against the built bundle
// (dist/, served by static-server.mjs), not the sources. A run on a bundle
// older than the sources tests old code, so it stops here instead.
import { existsSync, readdirSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const rootDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const bundle = path.join(rootDir, "dist", "main.js");

// What scripts/build.mjs puts in dist/: the app's modules and its static files.
const SOURCE_DIRS = ["components", "assets"];
const SOURCE_FILES = ["index.html", "styles.css", "text-icon.svg"];
const isAppModule = (name) => /\.(mjs|js)$/.test(name) && name !== "playwright.config.mjs";

function newestIn(dir) {
  let newest = null;
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    const found = entry.isDirectory() ? newestIn(full) : { file: full, mtimeMs: statSync(full).mtimeMs };
    if (found && (!newest || found.mtimeMs > newest.mtimeMs)) newest = found;
  }
  return newest;
}

export default function checkDistFresh() {
  const rebuild = process.env.COVERAGE ? "npm run test:e2e:coverage" : "npm run build (or npm run test:e2e)";
  if (!existsSync(bundle)) {
    throw new Error(`No dist/main.js to test: run ${rebuild} first.`);
  }
  // A coverage run maps onto the sources through the coverage build's source map.
  if (process.env.COVERAGE && !existsSync(`${bundle}.map`)) {
    throw new Error(`dist/ is a production build, without the source map coverage needs: run ${rebuild}.`);
  }
  // And a normal run tests the production build, not the one a coverage run left.
  if (!process.env.COVERAGE && existsSync(`${bundle}.map`)) {
    throw new Error(`dist/ is the coverage build a coverage run left: run ${rebuild}.`);
  }
  const candidates = [
    ...readdirSync(rootDir).filter(isAppModule),
    ...SOURCE_FILES,
  ].map((name) => {
    const file = path.join(rootDir, name);
    return { file, mtimeMs: statSync(file).mtimeMs };
  });
  for (const dir of SOURCE_DIRS) {
    const found = newestIn(path.join(rootDir, dir));
    if (found) candidates.push(found);
  }
  const builtAt = statSync(bundle).mtimeMs;
  const stale = candidates.filter((c) => c.mtimeMs > builtAt).map((c) => path.relative(rootDir, c.file));
  if (stale.length) {
    throw new Error(
      `dist/ is older than ${stale.join(", ")}: the e2e tests would run the old bundle. Run ${rebuild}.`
    );
  }
}
