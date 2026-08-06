// Merges the per-test raw V8 coverage entries collected by
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

function mergeV8Coverage(allEntries) {
  const base = allEntries[0];
  const countByRangeKey = new Map();
  for (const entry of allEntries) {
    for (const fn of entry.functions) {
      for (const range of fn.ranges) {
        const key = `${range.startOffset}:${range.endOffset}`;
        countByRangeKey.set(key, (countByRangeKey.get(key) || 0) + range.count);
      }
    }
  }
  return {
    ...base,
    functions: base.functions.map((fn) => ({
      ...fn,
      ranges: fn.ranges.map((r) => ({
        ...r,
        count: countByRangeKey.get(`${r.startOffset}:${r.endOffset}`) ?? r.count
      }))
    }))
  };
}

const merged = mergeV8Coverage(entries);

const converter = v8toIstanbul(mainJsPath, 0, { source: merged.source });
await converter.load();
converter.applyCoverage(merged.functions);
const istanbulData = converter.toIstanbul();

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

console.log(`\nE2e coverage (from ${entries.length} test run(s), merged):\n`);
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
