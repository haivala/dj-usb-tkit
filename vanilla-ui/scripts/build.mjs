import { build } from "esbuild";
import { cp, mkdir, rm } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const rootDir = path.resolve(__dirname, "..");
const distDir = path.join(rootDir, "dist");

const staticFiles = [
  "index.html",
  "styles.css",
  "mock_api_client.mjs",
  "text-icon.svg",
  "playback_ui_state.js"
];

await rm(distDir, { recursive: true, force: true });
await mkdir(distDir, { recursive: true });

await build({
  entryPoints: [path.join(rootDir, "main.js")],
  bundle: true,
  format: "esm",
  platform: "browser",
  target: ["chrome110", "safari16"],
  outfile: path.join(distDir, "main.js"),
  // External sourcemap is opt-in only -- dist/ is the real Tauri production
  // frontend bundle, so it must never ship with source attached by default.
  sourcemap: process.env.COVERAGE ? true : false,
  logLevel: "info"
});

for (const relativePath of staticFiles) {
  await cp(path.join(rootDir, relativePath), path.join(distDir, relativePath), {
    recursive: false
  });
}

await cp(path.join(rootDir, "assets"), path.join(distDir, "assets"), {
  recursive: true
});
