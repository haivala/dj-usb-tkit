import test from "node:test";
import assert from "node:assert/strict";
import { bindSettingsEvents, renderEssentiaInstallRow } from "../components/settings/events.mjs";
import { makeTestCtx } from "./test_helpers.mjs";

// The real app context on index.html; `el` is its real settings controls.
function makeDom() {
  const ctx = makeTestCtx();
  return { ctx, dom: ctx.dom, el: ctx.el };
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
    renderEssentiaInstallRow({ state: item.state, el });

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

function bindEngineSettings({ ctx, state, command, persistSetting, setProgress }) {
  const statuses = [];
  bindSettingsEvents(Object.assign(ctx, {
    state,
    persistSetting,
    emitStatus: (message) => statuses.push(message),
    command,
    setProgress,
    closeSettingsDrawer: () => {},
    switchView: async () => {},
    updatePlaylistExportButtons: () => {}
  }));
  return statuses;
}

test("settings events persist engine changes and invoke Essentia commands", async () => {
  const { ctx, dom, el } = makeDom();
  const state = {
    analysisEngine: "stratum",
    nodeAvailable: true,
    essentiaInstalled: false,
    essentiaDownloading: false
  };
  const commands = [];
  const persisted = [];
  const statuses = bindEngineSettings({
    ctx,
    state,
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
  renderEssentiaInstallRow({ state, el });
  el.essentiaCancelBtn.click();
  assert.ok(commands.includes("cancel_essentia_download"));

  state.essentiaDownloading = false;
  state.essentiaInstalled = true;
  state.analysisEngine = "essentia";
  el.analysisEngineSelect.value = "essentia";
  renderEssentiaInstallRow({ state, el });
  el.essentiaRemoveBtn.click();
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.ok(commands.includes("remove_essentia"));
  assert.equal(state.essentiaInstalled, false);
  assert.equal(state.analysisEngine, "stratum");
  assert.ok(persisted.some((item) => item.dbKey === "ui_analysis_engine_v1" && item.value === "stratum"));
});
