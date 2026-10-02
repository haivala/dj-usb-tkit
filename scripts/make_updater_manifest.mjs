#!/usr/bin/env node
// Builds the `latest.json` manifest tauri-plugin-updater reads from
// https://github.com/<repo>/releases/latest/download/latest.json.
//
// Usage: node scripts/make_updater_manifest.mjs <assets-dir> <tag> [notes-file]
//
// Run by the release workflow's publish job on the downloaded bundle artifacts.
// It also renames every asset whose name contains spaces (productName
// "DJ USB Tkit" on macOS/Windows): GitHub turns spaces in uploaded asset names
// into dots, so the manifest URLs would not match otherwise.
//
// Only installer-specific platform keys are written (`linux-x86_64-appimage`,
// `windows-x86_64-nsis`, `darwin-<arch>-app`). The updater looks those up
// before the bare `{os}-{arch}` key, so deb/rpm/MSI installs find no entry and
// can never be handed another format's payload.
//
// Without any `.sig` files (a build without the updater signing key) it writes
// no manifest and exits 0, so releases still publish. If some but not all
// updater artifacts are signed, or a signature isn't bound to this version
// (the app sets `requireSignedVersion`), it fails instead of publishing a
// manifest the updater would reject.

import { readdir, readFile, rename, writeFile } from "node:fs/promises";
import path from "node:path";

const [assetsDir, tag, notesFile] = process.argv.slice(2);
if (!assetsDir || !tag) {
  console.error("usage: make_updater_manifest.mjs <assets-dir> <tag> [notes-file]");
  process.exit(2);
}
const version = tag.replace(/^v/, "");
const repo = process.env.GITHUB_REPOSITORY || "haivala/dj-usb-tkit";

async function listFiles(dir) {
  const entries = await readdir(dir, { withFileTypes: true, recursive: true });
  return entries
    .filter((entry) => entry.isFile())
    .map((entry) => path.join(entry.parentPath ?? entry.path, entry.name));
}

// Rename "DJ USB Tkit_0.3.2_x64-setup.exe" -> "DJ_USB_Tkit_0.3.2_x64-setup.exe".
for (const file of await listFiles(assetsDir)) {
  const base = path.basename(file);
  if (base.includes(" ")) {
    await rename(file, path.join(path.dirname(file), base.replaceAll(" ", "_")));
  }
}
const files = await listFiles(assetsDir);

const sigs = files.filter((file) => file.endsWith(".sig"));
if (sigs.length === 0) {
  console.warn("No updater signatures found (built without TAURI_SIGNING_PRIVATE_KEY); skipping latest.json.");
  process.exit(0);
}

// The pubkey baked into this build has to verify these signatures; shipping
// the placeholder would give every install an updater that rejects everything.
const tauriConf = JSON.parse(
  await readFile(new URL("../desktop/src-tauri/tauri.conf.json", import.meta.url), "utf8")
);
const pubkey = tauriConf.plugins?.updater?.pubkey ?? "";
if (!pubkey || pubkey.startsWith("REPLACE_")) {
  console.error("Updater signatures exist but plugins.updater.pubkey in desktop/src-tauri/tauri.conf.json is not set.");
  process.exit(1);
}

const darwinArch = (() => {
  const dmg = files.map((file) => path.basename(file)).find((name) => name.endsWith(".dmg"));
  if (dmg?.includes("_x64")) return "x86_64";
  return "aarch64";
})();

const UPDATER_ARTIFACTS = [
  { key: "linux-x86_64-appimage", matches: (name) => name.endsWith(".AppImage") },
  { key: "windows-x86_64-nsis", matches: (name) => name.endsWith("-setup.exe") },
  { key: `darwin-${darwinArch}-app`, matches: (name) => name.endsWith(".app.tar.gz") },
];

function signedVersion(signature) {
  const text = Buffer.from(signature, "base64").toString("utf8");
  const trusted = text.split("\n").find((line) => line.startsWith("trusted comment:"));
  return trusted
    ?.slice("trusted comment:".length)
    .trim()
    .split("\t")
    .find((field) => field.startsWith("version:"))
    ?.slice("version:".length);
}

const platforms = {};
const problems = [];
for (const { key, matches } of UPDATER_ARTIFACTS) {
  const artifact = files.find((file) => matches(path.basename(file)));
  if (!artifact) {
    problems.push(`${key}: no artifact found`);
    continue;
  }
  const sigFile = `${artifact}.sig`;
  if (!files.includes(sigFile)) {
    problems.push(`${key}: ${path.basename(artifact)} has no .sig`);
    continue;
  }
  const signature = (await readFile(sigFile, "utf8")).trim();
  const signed = signedVersion(signature);
  if (signed !== version) {
    problems.push(`${key}: signature is bound to version ${signed ?? "(none)"}, expected ${version}`);
    continue;
  }
  platforms[key] = {
    signature,
    url: `https://github.com/${repo}/releases/download/${tag}/${path.basename(artifact)}`,
  };
}

if (problems.length > 0) {
  console.error("Cannot build latest.json:");
  for (const problem of problems) console.error(`  - ${problem}`);
  process.exit(1);
}

const notes = notesFile ? (await readFile(notesFile, "utf8")).trim() : "";
const manifest = { version, notes, pub_date: new Date().toISOString(), platforms };
const out = path.join(assetsDir, "latest.json");
await writeFile(out, `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`Wrote ${out} for ${version}: ${Object.keys(platforms).join(", ")}`);
