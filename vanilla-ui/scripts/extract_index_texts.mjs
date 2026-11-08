// One-time script (USB_TEXT_CATALOG_PLAN.md, piece 4; deleted in piece 14): moves index.html's
// English into the catalog. For each text node, and each aria-label, data-tooltip,
// placeholder, title and alt, it proposes a key by "Keys and comments", rewrites the HTML to
// `data-text*` (a text beside child elements gets its own `<span data-text>`), and appends the
// entries to backend/locales/en.ftl. Run it again and it finds nothing.
//
//   node scripts/extract_index_texts.mjs [--dry-run]
//
// It prints what a person decides: texts used in more than one area, and the elements it
// named from their text.
import { parse } from "parse5";
import { readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const uiDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const htmlPath = path.join(uiDir, "index.html");
const ftlPath = path.resolve(uiDir, "../backend/locales/en.ftl");
const dryRun = process.argv.includes("--dry-run");

const ATTRIBUTES = [
  ["aria-label", "aria-label"],
  ["data-tooltip", "tooltip"],
  ["placeholder", "placeholder"],
  ["title", "title"],
  ["alt", "alt"]
];
// Never translated (see the plan's "Never translated").
const KEEP = new Set(["DJ USB Tkit", "FLAC", "ALAC", "AIFF", "WAV", "MP3", "AAC", "rekordbox", "Mixxx", "Essentia"]);

// The area of everything inside an element with this id.
const AREA_BY_ID = {
  "panel-library": "library",
  "panel-usb": "usb",
  "panel-usb-playlists": "usb-playlists",
  "panel-usb-history": "usb-history",
  "panel-usb-player-menu": "usb-player-menu",
  "panel-playlist": "playlist",
  "panel-event-log": "event-log",
  "panel-backups": "backups",
  trackDetailView: "track-detail",
  settingsDrawer: "settings",
  updateBanner: "header",
  navSidebar: "sidebar",
  progressFooter: "job",
  driveNameOverlay: "drive-name",
  confirmOverlay: "common",
  groupEditOverlay: "group-edit",
  addToOverlay: "add-to",
  convertOverlay: "convert",
  tracklistExportOverlay: "tracklist-export",
  playlistImportOverlay: "playlist-import",
  helpOverlay: "help"
};
// The top bar has no id.
const AREA_BY_TAG = { header: "header" };

const html = readFileSync(htmlPath, "utf8");
const doc = parse(html, { sourceCodeLocationInfo: true });

// --- Which pages use each template: the JS files that name its id. ---
function jsFiles(dir) {
  const out = [];
  for (const name of readdirSync(dir)) {
    if (["node_modules", "dist", "tests", "scripts", "test-results", "assets"].includes(name) || name.startsWith(".")) continue;
    const full = path.join(dir, name);
    if (statSync(full).isDirectory()) out.push(...jsFiles(full));
    else if (/\.m?js$/.test(name) && name !== "playwright.config.mjs") out.push(full);
  }
  return out;
}
const sources = jsFiles(uiDir).map((file) => [path.relative(uiDir, file), readFileSync(file, "utf8")]);
// A component folder is its page's area; shared/, shell/ and root modules aren't one page's.
function fileArea(file) {
  const match = file.match(/^components\/([^/]+)\//);
  if (!match || ["shared", "shell"].includes(match[1])) return null;
  return match[1];
}
function templateArea(id) {
  const users = sources.filter(([, text]) => text.includes(`"${id}"`)).map(([file]) => fileArea(file));
  const areas = new Set(users);
  if (areas.size === 1 && !areas.has(null)) return [...areas][0];
  return kebab(id.replace(/^tpl/, "").replace(/Template$/, ""));
}

// --- Naming. ---
function kebab(text) {
  return text
    .replace(/([a-z0-9])([A-Z])/g, "$1-$2")
    .replace(/[^A-Za-z0-9]+/g, "-")
    .replace(/^-|-$/g, "")
    .toLowerCase();
}
function words(text, count) {
  const ascii = text.normalize("NFKD").replace(/[^\x00-\x7f]/g, "");
  return (ascii.toLowerCase().match(/[a-z0-9]+/g) || []).slice(0, count).join("-");
}
function thingFromId(id, area) {
  let thing = kebab(id).replace(/-(btn|button|label|text|el)$/, "");
  for (const prefix of [area, area.split("-")[0]]) {
    if (thing.startsWith(`${prefix}-`)) {
      thing = thing.slice(prefix.length + 1);
      break;
    }
  }
  return thing === kebab(id) && thing === area ? "" : thing;
}

const attr = (node, name) => node.attrs?.find((a) => a.name === name)?.value;
const collapse = (text) => text.replace(/\s+/g, " ").trim();
const hasLetters = (text) => /[A-Za-z]/.test(text);

// --- Walk. ---
const found = []; // { area, base, named, kind, text, edit }
const edits = []; // { start, end, replacement }

// Markup repeated on several pages, like a template used on more than one: its own area.
function sharedArea(element) {
  if (attr(element, "data-selection-bar") != null) return "selection-bar";
  if (attr(element, "data-track-grid") != null) return "track-table";
  const classes = (attr(element, "class") || "").split(/\s+/);
  if (classes.includes("track-total-time")) return "track-table";
  if (classes.includes("track-detail-rating")) return "rating";
  return null;
}

// Keys the rules name badly, renamed by hand (piece 4's review). Two keys renamed to one
// merge, which only works for the same text.
const RENAME = {
  "ui-backups-each-backup-holds": "ui-backups-intro",
  "ui-event-log-event-log": "ui-event-log-title",
  "ui-settings-settings": "ui-settings-title",
  "ui-usb-history-usb-history": "ui-usb-history-title",
  "ui-usb-playlists-usb-playlists": "ui-usb-playlists-title",
  "ui-usb-connection-bar-usb-browser": "ui-usb-title",
  "ui-library-content-media-library": "ui-library-title",
  "ui-playlist-panel-title": "ui-playlist-title",
  "ui-usb-player-menu-player-menu-editor": "ui-usb-player-menu-title",
  "ui-help-quick-start": "ui-help-title",
  "ui-help-quick-start-help-aria-label": "ui-help-aria-label",
  "ui-selection-bar-select-tracks-to": "ui-selection-bar-hint-library",
  "ui-selection-bar-select-tracks-to-select-tracks-to": "ui-selection-bar-hint-usb",
  "ui-selection-bar-select-tracks-to-select-tracks-to-2": "ui-selection-bar-hint-playlist",
  "ui-selection-bar-add-to": "ui-selection-bar-add-to-target",
  "ui-selection-bar-add-to-add-to": "ui-selection-bar-add-to",
  "ui-selection-bar-0-selected": "ui-selection-bar-count-none",
  "ui-selection-bar-pick-one-or-tooltip": "ui-selection-bar-add-to-tooltip",
  "ui-selection-bar-reset-waveform-bpm-tooltip": "ui-selection-bar-reanalyze-tooltip",
  "ui-selection-bar-make-aiff-wav-tooltip": "ui-selection-bar-convert-tooltip",
  "ui-selection-bar-edit-genre-release-tooltip": "ui-selection-bar-edit-tooltip",
  "ui-selection-bar-edit": "ui-selection-bar-edit-short",
  "ui-selection-bar-remove-the-selected-tooltip": "ui-selection-bar-remove-tooltip",
  "ui-track-table-total-time-0": "ui-track-table-total-time",
  "ui-track-table-select-all-tracks-aria-label": "ui-track-table-select-all-aria-label",
  "ui-track-table-sorted-by": "ui-track-table-sort-hint",
  "ui-track-detail-play-from-this-aria-label": "ui-track-detail-play-from-cue-aria-label",
  "ui-track-detail-play-from-this-play-from-this-aria-label": "ui-track-detail-play-from-memory-cue-aria-label",
  "ui-track-detail-how-strongly-the-tooltip": "ui-track-detail-grid-level-tooltip",
  "ui-track-detail-mix": "ui-track-detail-metronome-mix",
  "ui-track-detail-metronome-mix-left-tooltip": "ui-track-detail-metronome-mix-tooltip",
  "ui-track-detail-hint-waveform": "ui-track-detail-hint-waveform",
  "ui-track-detail-hint-click-to-move": "ui-track-detail-hint-waveform-text",
  "ui-track-detail-hint-drag-to-move": "ui-track-detail-hint-cue-markers-drag",
  "ui-track-detail-hint-on-snaps-to": "ui-track-detail-hint-cue-markers-quantize",
  "ui-track-detail-hint-shift": "ui-track-detail-hint-shift-key",
  "ui-track-detail-hint-places-freely-click": "ui-track-detail-hint-cue-markers-free",
  "ui-track-detail-hint-adds-a-memory": "ui-track-detail-hint-cue-markers-memory",
  "ui-track-detail-memory-cue": "ui-track-detail-memory-cue-badge",
  "ui-track-detail-add-a-memory-aria-label": "ui-track-detail-add-memory-cue-aria-label",
  "ui-track-detail-add-a-memory-tooltip": "ui-track-detail-add-memory-cue-tooltip",
  "ui-track-row-cue-points-beat-aria-label": "ui-track-row-edit-cues-aria-label",
  "ui-track-row-empty-no-tracks-available": "ui-track-row-empty",
  "ui-cover-img-cover-alt": "ui-cover-img-alt",
  "ui-sidebar-donate-donate": "ui-sidebar-donate",
  "ui-sidebar-website-website": "ui-sidebar-website",
  "ui-sidebar-add-playlist": "ui-sidebar-new-playlist",
  "ui-sidebar-add-playlist-new": "ui-sidebar-add-playlist",
  "ui-sidebar-import-playlist-import": "ui-sidebar-import-playlist",
  "ui-sidebar-nav-sidebar-resizer-tooltip": "ui-sidebar-resizer-tooltip",
  "ui-sidebar-app-sections-aria-label": "ui-sidebar-nav-aria-label",
  "ui-common-sidebar-expand": "ui-sidebar-expand",
  "ui-header-badge": "ui-header-badge-none",
  "ui-header-playlist-badge-playlist": "ui-header-playlist-badge",
  "ui-header-usb-name-badge-usb": "ui-header-usb-name-badge",
  "ui-header-usb-header-health-dot": "ui-header-usb-health-dot",
  "ui-header-update-banner-actions-download": "ui-header-update-banner-download",
  "ui-header-update-banner-actions-update-restart": "ui-header-update-banner-restart",
  "ui-settings-update-actions-download": "ui-settings-update-download",
  "ui-settings-update-actions-update-restart": "ui-settings-update-restart",
  "ui-settings-export-sync-mode-group-sync-mode": "ui-settings-sync-mode",
  "ui-settings-export-sync-mode-group-additive-playlist-update": "ui-settings-sync-mode-additive",
  "ui-settings-export-sync-mode-group-adds-missing-tracks": "ui-settings-sync-mode-additive-hint",
  "ui-settings-export-sync-mode-group-mirror-playlist-exact": "ui-settings-sync-mode-mirror",
  "ui-settings-export-sync-mode-group-same-name-playlists": "ui-settings-sync-mode-mirror-hint",
  "ui-settings-analysis-engine-select-stratum-built-in": "ui-settings-analysis-engine-stratum",
  "ui-settings-bpm-key-engine": "ui-settings-analysis-engine",
  "ui-settings-key-notation-select-camelot-8a": "ui-settings-key-notation-camelot",
  "ui-settings-key-notation-select-classic-am": "ui-settings-key-notation-classic",
  "ui-settings-theme-selector-auto": "ui-settings-theme-auto",
  "ui-settings-theme-selector-dark": "ui-settings-theme-dark",
  "ui-settings-theme-selector-light": "ui-settings-theme-light",
  "ui-settings-node-js-not": "ui-settings-node-js-missing",
  "ui-settings-back-up-pdb": "ui-settings-backup-before-export",
  "ui-settings-before-each-export": "ui-settings-backup-before-export-hint",
  "ui-settings-the-newest-backup": "ui-settings-backups-to-keep-hint",
  "ui-settings-write-engine-dj": "ui-settings-engine-library",
  "ui-settings-whenever-this-app": "ui-settings-engine-library-hint",
  "ui-settings-where-convert-saves": "ui-settings-transcode-folder-hint",
  "ui-library-chips-row-libraries": "ui-library-libraries",
  "ui-library-source-body-folders": "ui-library-folders",
  "ui-library-source-filter-header-sources": "ui-library-sources",
  "ui-library-external-rekordbox-db-toggle-external-library": "ui-library-external-rekordbox-db",
  "ui-usb-health-card-health-diagnostics": "ui-usb-health-card",
  "ui-usb-recent-header-recent-usbs": "ui-usb-recent",
  "ui-usb-audio-formats-header-audio-formats": "ui-usb-audio-formats",
  "ui-usb-diag-repair-technical-technical-details": "ui-usb-repair-technical-details",
  "ui-usb-checks-that-older": "ui-usb-parity-intro",
  "ui-usb-run-usb-parity": "ui-usb-parity-report",
  "ui-usb-run-usb-parity-tooltip": "ui-usb-parity-report-tooltip",
  "ui-usb-event-log": "ui-usb-diag-log-link",
  "ui-usb-forget-this-usb-tooltip": "ui-usb-forget-tooltip",
  "ui-usb-history-export-history-tracklist": "ui-usb-history-export-tracklist",
  "ui-usb-history-history-track-search-placeholder": "ui-usb-history-search-placeholder",
  "ui-usb-playlists-track-search-placeholder": "ui-usb-playlists-search-placeholder",
  "ui-usb-player-menu-current-player-menu": "ui-usb-player-menu-current",
  "ui-usb-player-menu-available-hidden": "ui-usb-player-menu-available",
  "ui-usb-player-menu-background-color-chips-aria-label": "ui-usb-player-menu-background-colour-aria-label",
  "ui-drive-name-give-this-usb": "ui-drive-name-intro",
  "ui-drive-name-the-drive-s": "ui-drive-name-rename-note",
  "ui-drive-name-skip-not-now": "ui-drive-name-skip",
  "ui-drive-name-skip-cancel": "ui-common-cancel",
  "ui-drive-name-title-name-this-drive": "ui-drive-name-title-name",
  "ui-drive-name-title-rename-this-drive": "ui-drive-name-title-rename",
  "ui-convert-makes-another-file": "ui-convert-intro",
  "ui-convert-format-group-format": "ui-convert-format",
  "ui-group-edit-only-the-fields": "ui-group-edit-intro",
  "ui-help-add-folders": "ui-help-step-folders",
  "ui-help-click-add-folder": "ui-help-step-folders-text",
  "ui-help-analyze": "ui-help-step-analyze",
  "ui-help-click-analyze-library": "ui-help-step-analyze-text",
  "ui-help-connect-usb": "ui-help-step-usb",
  "ui-help-go-to-usb": "ui-help-step-usb-text",
  "ui-help-create-playlist": "ui-help-step-playlist",
  "ui-help-click-new-in": "ui-help-step-playlist-text",
  "ui-help-export": "ui-help-step-export",
  "ui-help-open-your-playlist": "ui-help-step-export-text",
  "ui-help-donate-section-support-dj-usb": "ui-help-donate-heading",
  "ui-help-donate-section-support-options-are": "ui-help-donate-text",
  "ui-help-donate-section-open-support-page": "ui-help-donate-open",
  "ui-job-operation-progress-aria-label": "ui-job-progress-aria-label",
  "ui-job-progress-cancel-transcode-aria-label": "ui-job-cancel-transcode-aria-label",
  "ui-job-progress-cancel-transcode-title": "ui-job-cancel-transcode-title",
  "ui-job-progress-cancel-analysis": "ui-job-cancel-analysis",
  "ui-job-progress-pause": "ui-job-pause-analysis",
  "ui-job-progress-dismiss-aria-label": "ui-job-dismiss-aria-label",
  "ui-job-progress-idle": "ui-job-idle",
  "ui-playlist-analyze-playlist-missing": "ui-playlist-analyze-missing",
  "ui-playlist-export-playlist": "ui-playlist-export",
  "ui-playlist-playlist-name-placeholder": "ui-playlist-name-placeholder",
  "ui-playlist-rename-playlist-or-tooltip": "ui-playlist-rename-tooltip",
  "ui-playlist-rename-playlist-aria-label": "ui-playlist-rename-aria-label",
  "ui-playlist-search-input-placeholder": "ui-playlist-search-placeholder",
  "ui-rating-click-a-star-tooltip": "ui-rating-tooltip",
  "ui-rating-rating-aria-label": "ui-rating-aria-label",
  "ui-tracklist-export-placement-after-track-artist": "ui-tracklist-export-placement-after",
  "ui-tracklist-export-placement-before-track-0": "ui-tracklist-export-placement-before",
  "ui-tracklist-export-placement-row-time-placement": "ui-tracklist-export-placement",
  "ui-tracklist-export-start-row-start-from-track": "ui-tracklist-export-start-from",
  "ui-tracklist-export-times-are-estimated": "ui-tracklist-export-estimated-note",
  "ui-playlist-import-each-is-imported": "ui-playlist-import-intro",
  "ui-playlist-import-replaces-the-bpm": "ui-playlist-import-force-note"
};

// Elements whose text JS always replaces before it shows: the text goes, with no key.
const DROP_IDS = new Set(["settingsVersionText", "confirmTitle", "confirmOkBtn"]);

// Generic dialog buttons, the same everywhere: `common`.
const COMMON_BUTTONS = new Set(["Cancel", "Save", "OK", "Close"]);

function contextOf(node, ancestors) {
  const chain = [...ancestors, node];
  let area = null;
  let root = -1; // The element that gives the area; ids above it don't name things.
  chain.forEach((a, index) => {
    const id = attr(a, "id");
    let found = null;
    if (a.tagName === "template") found = templateArea(id);
    else if (sharedArea(a)) found = sharedArea(a);
    else if (id && AREA_BY_ID[id]) found = AREA_BY_ID[id];
    else if (AREA_BY_TAG[a.tagName] && area == null) found = AREA_BY_TAG[a.tagName];
    if (found) {
      area = found;
      root = index;
    }
  });
  area = area || "common";
  // The nearest id below the area's own element.
  let idThing = "";
  for (let index = chain.length - 1; index > root; index -= 1) {
    const id = attr(chain[index], "id");
    if (id) {
      idThing = thingFromId(id, area);
      break;
    }
  }
  return { area, idThing, ownId: attr(node, "id") };
}

function propose(context, text, kind, elementHasOwnId) {
  const named = !elementHasOwnId;
  const parts = [context.idThing];
  if (named) parts.push(words(text, 3));
  const thing = parts.filter(Boolean).join("-") || words(text, 3);
  return { base: thing, named };
}

// Where a new attribute goes in an element's start tag: before its `>` or `/>`.
function attributeInsertAt(node) {
  const tag = node.sourceCodeLocation.startTag;
  return html.slice(tag.startOffset, tag.endOffset).endsWith("/>") ? tag.endOffset - 2 : tag.endOffset - 1;
}

function walk(node, ancestors) {
  if (!node.tagName && node.nodeName !== "#document" && node.nodeName !== "#document-fragment") return;
  if (["script", "style", "title", "head"].includes(node.tagName)) return;
  if (attr(node, "translate") === "no") return;
  const isElement = !!node.tagName;
  if (isElement && node.sourceCodeLocation) {
    const context = contextOf(node, ancestors);
    for (const [name, suffix] of ATTRIBUTES) {
      const value = attr(node, name);
      if (value == null || !hasLetters(value) || KEEP.has(collapse(value))) continue;
      if (attr(node, `data-text-${suffix}`)) continue;
      const proposal = propose(context, value, suffix, !!context.ownId && context.idThing !== "");
      const loc = node.sourceCodeLocation.attrs[name];
      const item = { ...proposal, area: context.area, kind: suffix, text: collapse(value) };
      found.push(item);
      edits.push({ start: loc.startOffset, end: loc.endOffset, item, attribute: `data-text-${suffix}` });
    }
    const children = node.tagName === "template" ? node.content.childNodes : node.childNodes;
    const textNodes = children.filter((c) => c.nodeName === "#text" && hasLetters(c.value) && !KEEP.has(collapse(c.value)));
    const singleKey = node.tagName === "kbd" && collapse(node.childNodes.map((c) => c.value || "").join("")).length === 1;
    if (textNodes.length && !attr(node, "data-text") && !singleKey && node.tagName !== "template") {
      const mixed = children.some((c) => c.tagName);
      if (mixed) {
        // A text beside child elements: its own span, the whitespace around it kept outside.
        for (const textNode of textNodes) {
          const text = collapse(textNode.value);
          const item = { ...propose(context, text, "content", false), area: context.area, kind: "content", text };
          found.push(item);
          const loc = textNode.sourceCodeLocation;
          const lead = textNode.value.match(/^\s*/)[0];
          const trail = textNode.value.match(/\s*$/)[0];
          edits.push({ start: loc.startOffset, end: loc.endOffset, item, wrap: { lead, trail } });
        }
      } else if (DROP_IDS.has(context.ownId)) {
        const loc = node.sourceCodeLocation;
        edits.push({ start: loc.startTag.endOffset, end: loc.endTag.startOffset, clear: true, drop: true });
      } else {
        // Only text: the element gets `data-text`, and its content goes.
        const text = collapse(children.map((c) => c.value || "").join(" "));
        const ownName = !!context.ownId && context.idThing !== "";
        const common = node.tagName === "button" && COMMON_BUTTONS.has(text);
        const item = common
          ? { base: text.toLowerCase(), named: false, area: "common", kind: "content", text }
          : { ...propose(context, text, "content", ownName), area: context.area, kind: "content", text };
        found.push(item);
        const loc = node.sourceCodeLocation;
        edits.push({ start: loc.startTag.endOffset, end: loc.endTag.startOffset, item, clear: true });
        edits.push({ start: attributeInsertAt(node), end: attributeInsertAt(node), item, onElement: true });
      }
    }
  }
  const children = node.tagName === "template" ? node.content.childNodes : node.childNodes || [];
  for (const child of children) walk(child, isElement ? [...ancestors, node] : ancestors);
}
walk(doc, []);

// --- Keys: the same text in the same area shares one. ---
const groups = new Map(); // `${area}\n${text}` -> { key, items }
for (const item of found) {
  const id = `${item.area}\n${item.text}`;
  if (!groups.has(id)) groups.set(id, { items: [] });
  groups.get(id).items.push(item);
}
const used = new Map(); // key -> group
for (const group of groups.values()) {
  const first = group.items[0];
  const kinds = new Set(group.items.map((i) => i.kind));
  const suffix = kinds.size === 1 && first.kind !== "content" ? `-${first.kind}` : "";
  let base = `ui-${first.area}-${first.base || words(first.text, 3)}`;
  let key = `${base}${suffix}`;
  if (used.has(key)) {
    base = `${base}-${words(first.text, 3)}`;
    key = `${base}${suffix}`;
  }
  for (let n = 2; used.has(key); n += 1) key = `${base}-${n}${suffix}`;
  used.set(key, group);
  group.key = key;
}
// The hand renames; keys renamed to the same name merge.
const keys = new Map(); // final key -> group
for (const group of groups.values()) {
  const key = RENAME[group.key] ?? group.key;
  const other = keys.get(key);
  if (other && other.items[0].text !== group.items[0].text) {
    throw new Error(`${key}: renamed onto a different text`);
  }
  if (other) other.items.push(...group.items);
  else keys.set(key, group);
  group.key = key;
  for (const item of group.items) item.key = key;
}
for (const from of Object.keys(RENAME)) {
  if (!used.has(from)) console.log(`RENAME names a key the rules didn't make: ${from}`);
}

// --- Rewrite the HTML. ---
const resolved = [];
for (const edit of edits) {
  const key = edit.item?.key;
  if (edit.attribute) resolved.push({ start: edit.start, end: edit.end, replacement: `${edit.attribute}="${key}"` });
  else if (edit.wrap) resolved.push({ start: edit.start, end: edit.end, replacement: `${edit.wrap.lead}<span data-text="${key}"></span>${edit.wrap.trail}` });
  else if (edit.clear) resolved.push({ start: edit.start, end: edit.end, replacement: "" });
  else if (edit.onElement) resolved.push({ start: edit.start, end: edit.start, replacement: ` data-text="${key}"` });
}
resolved.sort((a, b) => b.start - a.start || b.end - a.end);
let out = html;
for (const { start, end, replacement } of resolved) out = out.slice(0, start) + replacement + out.slice(end);

// --- The catalog. ---
function ftlValue(text) {
  return text.replace(/[{}]/g, (brace) => `{"${brace}"}`);
}
const byArea = new Map();
const areas = [...new Set(found.map((item) => item.area))].sort((a, b) => b.length - a.length);
for (const [key] of keys) {
  const area = areas.find((a) => key.startsWith(`ui-${a}-`)) ?? "common";
  if (!byArea.has(area)) byArea.set(area, []);
  byArea.get(area).push(`${key} = ${ftlValue(keys.get(key).items[0].text)}`);
}
let ftl = "";
for (const [area, lines] of [...byArea].sort(([a], [b]) => a.localeCompare(b))) {
  ftl += `\n## ui-${area} (index.html)\n\n${lines.sort().join("\n")}\n`;
}

// --- For a person to decide. ---
const byText = new Map();
for (const group of keys.values()) {
  const text = group.items[0].text;
  if (!byText.has(text)) byText.set(text, []);
  byText.get(text).push(group.key);
}
console.log(`${found.length} texts, ${keys.size} keys.`);
console.log("\nIn more than one area:");
for (const [text, keys] of byText) if (keys.length > 1) console.log(`  ${JSON.stringify(text)}: ${keys.join(", ")}`);
console.log("\nNamed from their text:");
for (const group of groups.values()) if (group.items.some((i) => i.named)) console.log(`  ${group.key} = ${group.items[0].text}`);

if (dryRun) {
  console.log(ftl);
} else {
  writeFileSync(htmlPath, out);
  writeFileSync(ftlPath, readFileSync(ftlPath, "utf8").replace(/\n*$/, "\n") + ftl);
}
