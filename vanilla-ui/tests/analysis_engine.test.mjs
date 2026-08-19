import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { bindSettingsEvents, renderEssentiaInstallRow } from "../components/settings/events.mjs";

const ENGINE_HTML = `<!doctype html><body>
  <select id="analysisEngineSelect">
    <option value="stratum">Stratum (built-in)</option>
    <option value="essentia">Essentia</option>
  </select>
  <span id="analysisEngineStatus"></span>
  <div id="essentiaInstallRow" class="hidden">
    <span id="essentiaNodeStatus"></span>
    <button id="essentiaDownloadBtn">Download ~5 MB</button>
    <button id="essentiaCancelBtn" class="hidden">Cancel</button>
    <button id="essentiaRemoveBtn" class="hidden">Remove</button>
  </div>
</body>`;

function makeConstants() {
  return {
    STORAGE_KEY_HELP_SEEN: "help",
    STORAGE_KEY_EXPORT_PRUNE_STALE: "prune",
    STORAGE_KEY_EXPORT_BACKUP: "backup",
    STORAGE_KEY_BACKUP_RETENTION_COUNT: "backup_retention",
    STORAGE_KEY_ANALYSIS_BPM_RANGE: "bpm",
    STORAGE_KEY_ANALYSIS_ENGINE: "engine",
    FRONTEND_DB_KEY_HELP_SEEN: "ui_help_seen_v1",
    FRONTEND_DB_KEY_EXPORT_PRUNE_STALE: "ui_export_prune_stale_v1",
    FRONTEND_DB_KEY_EXPORT_BACKUP: "ui_export_backup_v1",
    FRONTEND_DB_KEY_BACKUP_RETENTION_COUNT: "ui_backup_retention_count_v1",
    FRONTEND_DB_KEY_ANALYSIS_BPM_RANGE: "ui_analysis_bpm_range_v1",
    FRONTEND_DB_KEY_ANALYSIS_ENGINE: "ui_analysis_engine_v1"
  };
}

function makeDom() {
  const dom = new JSDOM(ENGINE_HTML);
  const doc = dom.window.document;
  return {
    dom,
    document: doc,
    el: {
      analysisEngineSelect: doc.querySelector("#analysisEngineSelect"),
      analysisEngineStatus: doc.querySelector("#analysisEngineStatus"),
      essentiaInstallRow: doc.querySelector("#essentiaInstallRow"),
      essentiaNodeStatus: doc.querySelector("#essentiaNodeStatus"),
      essentiaDownloadBtn: doc.querySelector("#essentiaDownloadBtn"),
      essentiaCancelBtn: doc.querySelector("#essentiaCancelBtn"),
      essentiaRemoveBtn: doc.querySelector("#essentiaRemoveBtn")
    }
  };
}

test("engine selector exposes stratum and essentia", () => {
  const { el } = makeDom();
  assert.deepEqual(
    [...el.analysisEngineSelect.querySelectorAll("option")].map((option) => option.value),
    ["stratum", "essentia"]
  );
});

test("renderEssentiaInstallRow maps engine/install state to row text and actions", () => {
  const cases = [
    {
      state: { analysisEngine: "stratum", nodeAvailable: true, essentiaInstalled: false },
      hidden: true
    },
    {
      state: { analysisEngine: "essentia", nodeAvailable: true, essentiaInstalled: false, essentiaDownloading: false },
      text: "Essentia files not installed",
      downloadHidden: false,
      cancelHidden: true,
      removeHidden: true
    },
    {
      state: { analysisEngine: "essentia", nodeAvailable: false, essentiaInstalled: false },
      html: "essentia-node-link",
      downloadHidden: false,
      cancelHidden: true,
      removeHidden: true
    },
    {
      state: { analysisEngine: "essentia", nodeAvailable: true, essentiaInstalled: false, essentiaDownloading: true },
      text: "Downloading...",
      downloadHidden: true,
      cancelHidden: false,
      removeHidden: true
    },
    {
      state: { analysisEngine: "essentia", nodeAvailable: true, essentiaInstalled: true, essentiaDownloading: false },
      text: "Essentia ready",
      ready: true,
      downloadHidden: true,
      cancelHidden: true,
      removeHidden: false
    }
  ];

  for (const item of cases) {
    const { el } = makeDom();
    renderEssentiaInstallRow(item.state, el);

    assert.equal(el.essentiaInstallRow.classList.contains("hidden"), !!item.hidden);
    if (item.hidden) continue;
    if (item.text) assert.match(el.essentiaNodeStatus.textContent, new RegExp(item.text));
    if (item.html) assert.match(el.essentiaNodeStatus.innerHTML, new RegExp(item.html));
    if (item.ready) assert.equal(el.essentiaNodeStatus.classList.contains("essentia-ready"), true);
    assert.equal(el.essentiaDownloadBtn.classList.contains("hidden"), item.downloadHidden);
    assert.equal(el.essentiaCancelBtn.classList.contains("hidden"), item.cancelHidden);
    assert.equal(el.essentiaRemoveBtn.classList.contains("hidden"), item.removeHidden);
  }
});

function bindEngineSettings({ state, el, document, dom, command, persistSetting, setProgress }) {
  const statuses = [];
  bindSettingsEvents({
    state,
    el,
    document,
    window: dom.window,
    navigator: {},
    constants: makeConstants(),
    persistSetting,
    setStatus: (message) => statuses.push(message),
    command,
    getTauriEventListen: async () => null,
    setProgress,
    closeSettingsDrawer: () => {},
    switchView: async () => {},
    normalizeAnalysisBpmRange: (value) => value,
    updatePlaylistExportButtons: () => {}
  });
  return statuses;
}

test("settings events persist engine changes and invoke Essentia commands", async () => {
  const { dom, document, el } = makeDom();
  const state = {
    analysisEngine: "stratum",
    nodeAvailable: true,
    essentiaInstalled: false,
    essentiaDownloading: false
  };
  const commands = [];
  const persisted = [];
  const statuses = bindEngineSettings({
    state,
    el,
    document,
    dom,
    command: async (cmd) => { commands.push(cmd); },
    persistSetting: (storageKey, dbKey, value) => { persisted.push({ storageKey, dbKey, value }); },
    setProgress: () => {}
  });

  el.analysisEngineSelect.value = "essentia";
  el.analysisEngineSelect.dispatchEvent(new dom.window.Event("change"));
  assert.equal(state.analysisEngine, "essentia");
  assert.ok(persisted.some((item) => item.dbKey === "ui_analysis_engine_v1" && item.value === "essentia"));
  assert.ok(statuses.some((message) => message.includes("Essentia")));

  el.essentiaDownloadBtn.click();
  assert.ok(commands.includes("download_essentia"));

  state.essentiaDownloading = true;
  renderEssentiaInstallRow(state, el);
  el.essentiaCancelBtn.click();
  assert.ok(commands.includes("cancel_essentia_download"));

  state.essentiaDownloading = false;
  state.essentiaInstalled = true;
  state.analysisEngine = "essentia";
  el.analysisEngineSelect.value = "essentia";
  renderEssentiaInstallRow(state, el);
  el.essentiaRemoveBtn.click();
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.ok(commands.includes("remove_essentia"));
  assert.equal(state.essentiaInstalled, false);
  assert.equal(state.analysisEngine, "stratum");
  assert.ok(persisted.some((item) => item.dbKey === "ui_analysis_engine_v1" && item.value === "stratum"));
});
