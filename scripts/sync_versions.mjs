import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const rootDir = path.resolve(__dirname, "..");

const rootCargoToml = path.join(rootDir, "Cargo.toml");
const targets = [
  path.join(rootDir, "desktop", "package.json"),
  path.join(rootDir, "desktop", "package-lock.json"),
  path.join(rootDir, "vanilla-ui", "package.json"),
  path.join(rootDir, "vanilla-ui", "package-lock.json")
];

function extractWorkspaceVersion(toml) {
  const workspacePackageIndex = toml.indexOf("[workspace.package]");
  if (workspacePackageIndex === -1) {
    throw new Error("missing [workspace.package] section in Cargo.toml");
  }

  const workspacePackage = toml.slice(workspacePackageIndex);
  const versionMatch = workspacePackage.match(/^\s*version\s*=\s*"([^"]+)"/m);
  if (!versionMatch) {
    throw new Error("missing workspace.package.version in Cargo.toml");
  }
  return versionMatch[1];
}

async function syncJsonVersion(filePath, version) {
  const raw = await readFile(filePath, "utf8");
  const parsed = JSON.parse(raw);
  parsed.version = version;
  if (parsed.packages && parsed.packages[""]) {
    parsed.packages[""].version = version;
  }
  await writeFile(filePath, `${JSON.stringify(parsed, null, 2)}\n`);
}

const workspaceVersion = extractWorkspaceVersion(await readFile(rootCargoToml, "utf8"));
await Promise.all(targets.map((target) => syncJsonVersion(target, workspaceVersion)));

process.stdout.write(`${workspaceVersion}\n`);
