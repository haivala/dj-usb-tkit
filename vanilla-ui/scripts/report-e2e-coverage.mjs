// Merges the per-page-load raw V8 coverage entries collected by
// tests/e2e/coverage-fixture.mjs (one JSON object per line, written when
// COVERAGE=1) into a single coverage set, maps it back to original source
// files via dist/main.js.map through v8-to-istanbul, and prints a per-file
// line-coverage table shaped like `node --test --experimental-test-coverage`'s
// output, for direct before/after comparison against unit coverage.
import { readFileSync, existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import v8toIstanbul from "v8-to-istanbul";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const rootDir = path.resolve(__dirname, "..");
const rawFile = path.join(rootDir, ".coverage-e2e", "raw-v8-coverage.jsonl");
const mainJsPath = path.join(rootDir, "dist", "main.js");

if (!existsSync(rawFile)) {
  console.error(`No coverage data at ${rawFile} -- run with COVERAGE=1 first.`);
  process.exit(1);
}

const lines = readFileSync(rawFile, "utf8").split("\n").filter((l) => l.trim());
const entries = lines.map((l) => JSON.parse(l));

// Each entry (one page load) is mapped on its own and the hit counts added up
// per statement, function and branch: merging the raw V8 ranges first would
// keep only the first entry's ranges, and V8 reports ranges only for the code
// a load ran, so code only later loads ran would count as never run.
async function toIstanbul(entry) {
  const converter = v8toIstanbul(mainJsPath, 0, { source: entry.source });
  await converter.load();
  converter.applyCoverage(entry.functions);
  return converter.toIstanbul();
}

const istanbulData = {};
for (const entry of entries) {
  for (const [file, fc] of Object.entries(await toIstanbul(entry))) {
    const merged = istanbulData[file];
    if (!merged) {
      istanbulData[file] = fc;
      continue;
    }
    for (const id of Object.keys(fc.s)) merged.s[id] = (merged.s[id] || 0) + fc.s[id];
    for (const id of Object.keys(fc.f)) merged.f[id] = (merged.f[id] || 0) + fc.f[id];
    for (const id of Object.keys(fc.b)) {
      merged.b[id] = (merged.b[id] || fc.b[id].map(() => 0)).map((n, i) => n + (fc.b[id][i] || 0));
    }
  }
}

const rows = [];
for (const [file, fc] of Object.entries(istanbulData)) {
  if (file.includes("node_modules")) continue;
  const relFile = path.relative(rootDir, file);
  const stmtIds = Object.keys(fc.statementMap);
  const coveredStmts = stmtIds.filter((id) => fc.s[id] > 0).length;
  const fnIds = Object.keys(fc.fnMap);
  const coveredFns = fnIds.filter((id) => fc.f[id] > 0).length;
  const branchIds = Object.keys(fc.branchMap);
  let totalBranches = 0;
  let coveredBranches = 0;
  for (const id of branchIds) {
    const hits = fc.b[id] || [];
    totalBranches += hits.length;
    coveredBranches += hits.filter((h) => h > 0).length;
  }
  rows.push({
    file: relFile,
    linePct: stmtIds.length ? (coveredStmts / stmtIds.length) * 100 : 100,
    branchPct: totalBranches ? (coveredBranches / totalBranches) * 100 : 100,
    funcPct: fnIds.length ? (coveredFns / fnIds.length) * 100 : 100,
    uncoveredLines: stmtIds
      .filter((id) => fc.s[id] === 0)
      .map((id) => fc.statementMap[id].start.line)
      .filter((v, i, arr) => arr.indexOf(v) === i)
      .sort((a, b) => a - b)
  });
}

rows.sort((a, b) => a.file.localeCompare(b.file));

console.log(`\nE2e coverage (from ${entries.length} page load${entries.length === 1 ? "" : "s"}, merged):\n`);
console.log("file".padEnd(50), "line %".padStart(8), "branch %".padStart(10), "func %".padStart(8));
for (const r of rows) {
  console.log(
    r.file.padEnd(50),
    r.linePct.toFixed(2).padStart(8),
    r.branchPct.toFixed(2).padStart(10),
    r.funcPct.toFixed(2).padStart(8)
  );
}

console.log(`\n${rows.length} source files touched by e2e coverage.`);
console.log("Per-file uncovered line numbers (for cross-checking specific deleted-test coverage):");
for (const r of rows) {
  if (r.uncoveredLines.length) {
    console.log(`  ${r.file}: ${r.uncoveredLines.slice(0, 40).join(",")}${r.uncoveredLines.length > 40 ? "..." : ""}`);
  }
}
