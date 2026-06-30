import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { restoreStoredUiPrefs } from "../startup_bootstrap.mjs";
import { bindSettingsEvents, renderEssentiaInstallRow } from "../components/settings/events.mjs";

const ENGINE_HTML = `<!doctype html><body>
  <select id="analysisEngineSelect">
    <option value="stratum">Stratum (built-in)</option>
    <option value="essentia">Essentia</option>
  </select>
  <span id="analysisEngineStatus"></span>
  <div id="essentiaInstallRow" class="hidden">
    <span id="essentiaNodeStatus"></span>
    <div class="essentia-install-actions">
      <button id="essentiaDownloadBtn">Download ~5 MB</button>
      <button id="essentiaCancelBtn" class="hidden">Cancel</button>
      <button id="essentiaRemoveBtn" class="hidden">Remove</button>
    </div>
  </div>
</body>`;

function makeConstants() {
  return {
    STORAGE_KEY_EXPORT_PRUNE_STALE: "prune",
    STORAGE_KEY_ANALYSIS_BPM_RANGE: "bpm",
    STORAGE_KEY_ANALYSIS_ENGINE: "engine",
    STORAGE_KEY_SIDEBAR_COLLAPSED: "sidebar",
    STORAGE_KEY_HELP_SEEN: "help",
    FRONTEND_DB_KEY_HELP_SEEN: "ui_help_seen_v1",
    FRONTEND_DB_KEY_EXPORT_PRUNE_STALE: "ui_export_prune_stale_v1",
    FRONTEND_DB_KEY_ANALYSIS_BPM_RANGE: "ui_analysis_bpm_range_v1",
    FRONTEND_DB_KEY_ANALYSIS_ENGINE: "ui_analysis_engine_v1"
  };
}

test("engine selector renders both stratum and essentia options", () => {
  const dom = new JSDOM(ENGINE_HTML);
  const doc = dom.window.document;
  const select = doc.querySelector("#analysisEngineSelect");
  const options = [...select.querySelectorAll("option")];

  assert.equal(options.length, 2);
  assert.equal(options[0].value, "stratum");
  assert.equal(options[1].value, "essentia");
});

test("install row hidden when stratum engine stored", () => {
  const dom = new JSDOM(ENGINE_HTML);
  const doc = dom.window.document;
  const select = doc.querySelector("#analysisEngineSelect");
  const installRow = doc.querySelector("#essentiaInstallRow");
  const state = {
    exportPruneStale: true,
    analysisBpmRange: "",
    analysisEngine: "stratum",
    sidebarCollapsed: false
  };
  const el = {
    exportSyncModeMirror: { checked: false },
    exportSyncModeAdditive: { checked: false },
    analysisBpmRangeSelect: { value: "" },
    analysisEngineSelect: select,
    analysisEngineStatus: doc.querySelector("#analysisEngineStatus"),
    essentiaInstallRow: installRow
  };

  const storage = new Map([["engine", "stratum"]]);
  restoreStoredUiPrefs(state, el, {
    localStorageObj: { getItem: (k) => storage.get(k) ?? null, setItem: () => {} },
    constants: makeConstants(),
    normalizeAnalysisBpmRange: (v) => v,
    defaultAnalysisBpmRange: "full"
  });

  assert.equal(state.analysisEngine, "stratum");
  assert.ok(installRow.classList.contains("hidden"), "install row should be hidden for stratum");
});

test("switching engine persists setting via persistSetting", () => {
  const dom = new JSDOM(ENGINE_HTML);
  const doc = dom.window.document;
  const select = doc.querySelector("#analysisEngineSelect");
  const state = {
    analysisEngine: "stratum",
    nodeAvailable: true
  };
  const el = {
    analysisEngineSelect: select,
    analysisEngineStatus: doc.querySelector("#analysisEngineStatus"),
    essentiaInstallRow: doc.querySelector("#essentiaInstallRow"),
    essentiaNodeStatus: doc.querySelector("#essentiaNodeStatus"),
    essentiaDownloadBtn: doc.querySelector("#essentiaDownloadBtn"),
    essentiaCancelBtn: doc.querySelector("#essentiaCancelBtn"),
    essentiaRemoveBtn: doc.querySelector("#essentiaRemoveBtn")
  };

  const persisted = [];
  const statusMessages = [];

  bindSettingsEvents({
    state,
    el,
    document: doc,
    window: dom.window,
    navigator: {},
    constants: makeConstants(),
    persistSetting: (storageKey, dbKey, value) => {
      persisted.push({ storageKey, dbKey, value });
    },
    setStatus: (msg) => statusMessages.push(msg),
    command: async () => {},
    getTauriEventListen: async () => null,
    closeSettingsDrawer: () => {},
    switchView: async () => {},
    normalizeAnalysisBpmRange: (v) => v,
    updatePlaylistExportButtons: () => {}
  });

  // Simulate changing to essentia.
  select.value = "essentia";
  select.dispatchEvent(new dom.window.Event("change"));

  assert.equal(state.analysisEngine, "essentia");
  assert.equal(persisted.length, 1);
  assert.equal(persisted[0].dbKey, "ui_analysis_engine_v1");
  assert.equal(persisted[0].value, "essentia");
  assert.ok(statusMessages.some((m) => m.includes("Essentia")));
});

test("install row visible when essentia engine stored", () => {
  const dom = new JSDOM(ENGINE_HTML);
  const doc = dom.window.document;
  const select = doc.querySelector("#analysisEngineSelect");
  const installRow = doc.querySelector("#essentiaInstallRow");
  const state = {
    exportPruneStale: true,
    analysisBpmRange: "",
    analysisEngine: "stratum",
    sidebarCollapsed: false
  };
  const el = {
    exportSyncModeMirror: { checked: false },
    exportSyncModeAdditive: { checked: false },
    analysisBpmRangeSelect: { value: "" },
    analysisEngineSelect: select,
    analysisEngineStatus: doc.querySelector("#analysisEngineStatus"),
    essentiaInstallRow: installRow
  };

  const storage = new Map([["engine", "essentia"]]);
  restoreStoredUiPrefs(state, el, {
    localStorageObj: { getItem: (k) => storage.get(k) ?? null, setItem: () => {} },
    constants: makeConstants(),
    normalizeAnalysisBpmRange: (v) => v,
    defaultAnalysisBpmRange: "full"
  });

  assert.equal(state.analysisEngine, "essentia");
  assert.equal(select.value, "essentia");
  assert.ok(!installRow.classList.contains("hidden"), "install row should be visible for essentia");
});

// ── renderEssentiaInstallRow tests ──────────────────────────────────────────

function makeInstallRowDom() {
  const dom = new JSDOM(ENGINE_HTML);
  const doc = dom.window.document;
  return {
    dom,
    el: {
      essentiaInstallRow: doc.querySelector("#essentiaInstallRow"),
      essentiaNodeStatus: doc.querySelector("#essentiaNodeStatus"),
      essentiaDownloadBtn: doc.querySelector("#essentiaDownloadBtn"),
      essentiaCancelBtn: doc.querySelector("#essentiaCancelBtn"),
      essentiaRemoveBtn: doc.querySelector("#essentiaRemoveBtn")
    }
  };
}

test("install row hidden when analysisEngine is stratum", () => {
  const { el } = makeInstallRowDom();
  const state = { analysisEngine: "stratum", nodeAvailable: true, essentiaInstalled: false };
  renderEssentiaInstallRow(state, el);
  assert.ok(el.essentiaInstallRow.classList.contains("hidden"));
});

test("install row visible when analysisEngine is essentia", () => {
  const { el } = makeInstallRowDom();
  const state = { analysisEngine: "essentia", nodeAvailable: true, essentiaInstalled: false };
  renderEssentiaInstallRow(state, el);
  assert.ok(!el.essentiaInstallRow.classList.contains("hidden"));
});

test("shows download button when not installed and node available", () => {
  const { el } = makeInstallRowDom();
  const state = { analysisEngine: "essentia", nodeAvailable: true, essentiaInstalled: false, essentiaDownloading: false };
  renderEssentiaInstallRow(state, el);
  assert.ok(!el.essentiaDownloadBtn.classList.contains("hidden"), "download btn visible");
  assert.ok(el.essentiaCancelBtn.classList.contains("hidden"), "cancel btn hidden");
  assert.ok(el.essentiaRemoveBtn.classList.contains("hidden"), "remove btn hidden");
});

test("shows node link when node unavailable", () => {
  const { el } = makeInstallRowDom();
  const state = { analysisEngine: "essentia", nodeAvailable: false, essentiaInstalled: false };
  renderEssentiaInstallRow(state, el);
  assert.ok(el.essentiaNodeStatus.innerHTML.includes("essentia-node-link"), "node link present");
});

test("shows cancel button during download state", () => {
  const { el } = makeInstallRowDom();
  const state = { analysisEngine: "essentia", nodeAvailable: true, essentiaInstalled: false, essentiaDownloading: true };
  renderEssentiaInstallRow(state, el);
  assert.ok(!el.essentiaCancelBtn.classList.contains("hidden"), "cancel btn visible");
  assert.ok(el.essentiaDownloadBtn.classList.contains("hidden"), "download btn hidden while downloading");
});

test("shows ready status when essentiaInstalled and nodeAvailable both true", () => {
  const { el } = makeInstallRowDom();
  const state = { analysisEngine: "essentia", nodeAvailable: true, essentiaInstalled: true, essentiaDownloading: false };
  renderEssentiaInstallRow(state, el);
  assert.ok(el.essentiaNodeStatus.classList.contains("essentia-ready"));
  assert.ok(el.essentiaNodeStatus.textContent.includes("ready"));
  assert.ok(!el.essentiaRemoveBtn.classList.contains("hidden"), "remove btn visible when installed");
  assert.ok(el.essentiaDownloadBtn.classList.contains("hidden"), "download btn hidden when installed");
});

test("download button click invokes download_essentia command", async () => {
  const dom = new JSDOM(ENGINE_HTML);
  const doc = dom.window.document;
  const el = {
    analysisEngineSelect: doc.querySelector("#analysisEngineSelect"),
    analysisEngineStatus: doc.querySelector("#analysisEngineStatus"),
    essentiaInstallRow: doc.querySelector("#essentiaInstallRow"),
    essentiaNodeStatus: doc.querySelector("#essentiaNodeStatus"),
    essentiaDownloadBtn: doc.querySelector("#essentiaDownloadBtn"),
    essentiaCancelBtn: doc.querySelector("#essentiaCancelBtn"),
    essentiaRemoveBtn: doc.querySelector("#essentiaRemoveBtn")
  };
  const state = { analysisEngine: "essentia", nodeAvailable: true, essentiaInstalled: false, essentiaDownloading: false };
  const commands = [];
  bindSettingsEvents({
    state, el, document: doc, window: dom.window, navigator: {},
    constants: makeConstants(),
    persistSetting: () => {},
    setStatus: () => {},
    command: async (cmd) => { commands.push(cmd); },
    getTauriEventListen: async () => null,
    setProgress: () => {},
    closeSettingsDrawer: () => {},
    switchView: async () => {},
    normalizeAnalysisBpmRange: (v) => v,
    updatePlaylistExportButtons: () => {}
  });
  el.essentiaDownloadBtn.click();
  assert.ok(commands.includes("download_essentia"), "download_essentia command invoked");
});

test("cancel button click invokes cancel_essentia_download command", () => {
  const dom = new JSDOM(ENGINE_HTML);
  const doc = dom.window.document;
  const el = {
    analysisEngineSelect: doc.querySelector("#analysisEngineSelect"),
    analysisEngineStatus: doc.querySelector("#analysisEngineStatus"),
    essentiaInstallRow: doc.querySelector("#essentiaInstallRow"),
    essentiaNodeStatus: doc.querySelector("#essentiaNodeStatus"),
    essentiaDownloadBtn: doc.querySelector("#essentiaDownloadBtn"),
    essentiaCancelBtn: doc.querySelector("#essentiaCancelBtn"),
    essentiaRemoveBtn: doc.querySelector("#essentiaRemoveBtn")
  };
  const state = { analysisEngine: "essentia", nodeAvailable: true, essentiaInstalled: false, essentiaDownloading: true };
  const commands = [];
  bindSettingsEvents({
    state, el, document: doc, window: dom.window, navigator: {},
    constants: makeConstants(),
    persistSetting: () => {},
    setStatus: () => {},
    command: async (cmd) => { commands.push(cmd); },
    getTauriEventListen: async () => null,
    setProgress: () => {},
    closeSettingsDrawer: () => {},
    switchView: async () => {},
    normalizeAnalysisBpmRange: (v) => v,
    updatePlaylistExportButtons: () => {}
  });
  el.essentiaCancelBtn.click();
  assert.ok(commands.includes("cancel_essentia_download"), "cancel command invoked");
});

test("remove button click invokes remove_essentia and resets engine to stratum", async () => {
  const dom = new JSDOM(ENGINE_HTML);
  const doc = dom.window.document;
  const select = doc.querySelector("#analysisEngineSelect");
  const el = {
    analysisEngineSelect: select,
    analysisEngineStatus: doc.querySelector("#analysisEngineStatus"),
    essentiaInstallRow: doc.querySelector("#essentiaInstallRow"),
    essentiaNodeStatus: doc.querySelector("#essentiaNodeStatus"),
    essentiaDownloadBtn: doc.querySelector("#essentiaDownloadBtn"),
    essentiaCancelBtn: doc.querySelector("#essentiaCancelBtn"),
    essentiaRemoveBtn: doc.querySelector("#essentiaRemoveBtn")
  };
  const state = { analysisEngine: "essentia", nodeAvailable: true, essentiaInstalled: true, essentiaDownloading: false };
  select.value = "essentia";
  const commands = [];
  const persisted = [];
  bindSettingsEvents({
    state, el, document: doc, window: dom.window, navigator: {},
    constants: makeConstants(),
    persistSetting: (storageKey, dbKey, value) => { persisted.push({ storageKey, dbKey, value }); },
    setStatus: () => {},
    command: async (cmd) => { commands.push(cmd); },
    getTauriEventListen: async () => null,
    setProgress: () => {},
    closeSettingsDrawer: () => {},
    switchView: async () => {},
    normalizeAnalysisBpmRange: (v) => v,
    updatePlaylistExportButtons: () => {}
  });
  await el.essentiaRemoveBtn.click();
  // Allow microtask queue to flush.
  await new Promise((r) => setTimeout(r, 10));
  assert.ok(commands.includes("remove_essentia"), "remove_essentia command invoked");
  assert.equal(state.essentiaInstalled, false, "essentiaInstalled reset");
  assert.equal(state.analysisEngine, "stratum", "engine reset to stratum");
  assert.ok(persisted.some((p) => p.dbKey === "ui_analysis_engine_v1" && p.value === "stratum"));
});
