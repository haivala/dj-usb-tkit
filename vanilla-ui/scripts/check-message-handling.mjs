import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const guardedFiles = [
  "components/usb/actions.mjs",
  "components/usb/events.mjs",
  "components/library/actions.mjs",
  "components/library/events.mjs",
  "components/playlist/actions.mjs",
  "components/playlist/events.mjs",
];

const bannedPatterns = [
  { pattern: /\bsetStatus\(/g, reason: "Direct setStatus formatting is not allowed in guarded files; use emitStatus/emitMessage." },
];

const root = resolve(process.cwd());
const violations = [];

for (const relativePath of guardedFiles) {
  const filePath = resolve(root, relativePath);
  const content = readFileSync(filePath, "utf8");
  for (const { pattern, reason } of bannedPatterns) {
    pattern.lastIndex = 0;
    const match = pattern.exec(content);
    if (!match) continue;
    violations.push({ relativePath, reason });
  }
}

if (violations.length > 0) {
  console.error("Message handling guard failed:");
  for (const violation of violations) {
    console.error(`- ${violation.relativePath}: ${violation.reason}`);
  }
  process.exit(1);
}

console.log("Message handling guard passed.");
