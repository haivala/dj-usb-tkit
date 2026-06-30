const NODE_JS_URL = "https://nodejs.org/";
const WEBSITE_URL = "https://chiph.art?utm_source=djtkit&utm_medium=app&utm_campaign=sidebar";
const SUPPORT_URL = "https://chiph.art/en/dj-usb-tkit/support?utm_source=djtkit&utm_medium=app&utm_campaign=support";

function openExternalUrl(window, url) {
  if (window.__TAURI__?.opener?.openUrl) {
    window.__TAURI__.opener.openUrl(url);
  } else if (window.__TAURI_INTERNALS__?.invoke) {
    window.__TAURI_INTERNALS__.invoke("plugin:opener|open_url", { url });
  } else {
    window.open(url, "_blank");
  }
}

export function renderEssentiaInstallRow(state, el, deps = {}) {
  const { openUrl = () => {} } = deps;
  if (!el.essentiaInstallRow) return;

  const show = state.analysisEngine === "essentia";
  el.essentiaInstallRow.classList.toggle("hidden", !show);
  if (!show) return;

  const { nodeAvailable, essentiaInstalled, essentiaDownloading, essentiaDownloadError } = state;

  // Node status line
  if (el.essentiaNodeStatus) {
    if (!nodeAvailable) {
      el.essentiaNodeStatus.innerHTML =
        `Node.js not found — <a href="#" class="essentia-node-link">Get Node.js</a>`;
      el.essentiaNodeStatus.querySelector(".essentia-node-link")
        ?.addEventListener("click", (e) => { e.preventDefault(); openUrl(NODE_JS_URL); });
    } else if (essentiaInstalled) {
      el.essentiaNodeStatus.textContent = "✓ Essentia ready";
      el.essentiaNodeStatus.classList.add("essentia-ready");
      el.essentiaNodeStatus.classList.remove("essentia-warn");
    } else if (essentiaDownloading) {
      el.essentiaNodeStatus.textContent = "Downloading...";
      el.essentiaNodeStatus.classList.remove("essentia-ready", "essentia-warn");
    } else if (essentiaDownloadError) {
      el.essentiaNodeStatus.textContent = `Download failed: ${essentiaDownloadError}`;
      el.essentiaNodeStatus.classList.add("essentia-warn");
      el.essentiaNodeStatus.classList.remove("essentia-ready");
    } else {
      el.essentiaNodeStatus.textContent = "Essentia files not installed";
      el.essentiaNodeStatus.classList.remove("essentia-ready", "essentia-warn");
    }
  }

  // Button visibility
  if (el.essentiaDownloadBtn) {
    el.essentiaDownloadBtn.classList.toggle("hidden", essentiaDownloading || essentiaInstalled);
  }
  if (el.essentiaCancelBtn) {
    el.essentiaCancelBtn.classList.toggle("hidden", !essentiaDownloading);
  }
  if (el.essentiaRemoveBtn) {
    el.essentiaRemoveBtn.classList.toggle("hidden", !essentiaInstalled);
  }
}

export function bindSettingsEvents(ctx) {
  const {
    state,
    el,
    document,
    window,
    constants,
    persistSetting,
    setStatus,
    setProgress,
    command,
    getTauriEventListen,
    pushEventLog,
    closeSettingsDrawer,
    switchView,
    normalizeAnalysisBpmRange,
    updatePlaylistExportButtons
  } = ctx;
  const {
    STORAGE_KEY_HELP_SEEN,
    FRONTEND_DB_KEY_HELP_SEEN,
    STORAGE_KEY_EXPORT_PRUNE_STALE,
    FRONTEND_DB_KEY_EXPORT_PRUNE_STALE,
    STORAGE_KEY_EXPORT_BACKUP,
    FRONTEND_DB_KEY_EXPORT_BACKUP,
    STORAGE_KEY_ANALYSIS_BPM_RANGE,
    FRONTEND_DB_KEY_ANALYSIS_BPM_RANGE,
    STORAGE_KEY_ANALYSIS_ENGINE,
    FRONTEND_DB_KEY_ANALYSIS_ENGINE
  } = constants;

  el.settingsBtn?.addEventListener("click", () => {
    el.settingsDrawer.classList.remove("hidden");
    el.settingsBackdrop.classList.remove("hidden");
  });
  el.settingsCloseBtn?.addEventListener("click", closeSettingsDrawer);
  el.settingsBackdrop?.addEventListener("click", closeSettingsDrawer);

  el.helpBtn?.addEventListener("click", () => {
    el.helpOverlay.classList.remove("hidden");
  });
  el.helpCloseBtn?.addEventListener("click", () => {
    el.helpOverlay.classList.add("hidden");
    persistSetting(STORAGE_KEY_HELP_SEEN, FRONTEND_DB_KEY_HELP_SEEN, "1");
  });
  el.helpOverlay?.addEventListener("click", (event) => {
    if (event.target === el.helpOverlay) {
      el.helpOverlay.classList.add("hidden");
      persistSetting(STORAGE_KEY_HELP_SEEN, FRONTEND_DB_KEY_HELP_SEEN, "1");
    }
  });

  document.getElementById("websiteBtn")?.addEventListener("click", () => {
    openExternalUrl(window, WEBSITE_URL);
  });

  document.getElementById("donateBtn")?.addEventListener("click", () => {
    openExternalUrl(window, SUPPORT_URL);
  });

  document.querySelectorAll(".help-donate-link").forEach((link) => {
    link.addEventListener("click", (e) => {
      e.preventDefault();
      openExternalUrl(window, e.currentTarget.dataset.externalUrl);
    });
  });

  el.exportSyncModeGroup?.addEventListener("change", (event) => {
    const mode = String(event?.target?.value || "").toLowerCase();
    state.exportPruneStale = mode !== "additive";
    persistSetting(
      STORAGE_KEY_EXPORT_PRUNE_STALE,
      FRONTEND_DB_KEY_EXPORT_PRUNE_STALE,
      state.exportPruneStale ? "1" : "0"
    );
    setStatus(
      state.exportPruneStale
        ? "Export sync mode: mirror (exact match)"
        : "Export sync mode: additive"
    );
    updatePlaylistExportButtons();
  });

  el.exportBackupCheckbox?.addEventListener("change", (event) => {
    state.exportBackup = !!event?.target?.checked;
    persistSetting(
      STORAGE_KEY_EXPORT_BACKUP,
      FRONTEND_DB_KEY_EXPORT_BACKUP,
      state.exportBackup ? "1" : "0"
    );
    setStatus(state.exportBackup ? "Export backup: enabled" : "Export backup: disabled");
  });

  el.analysisBpmRangeSelect?.addEventListener("change", (event) => {
    const selected = normalizeAnalysisBpmRange(event?.target?.value);
    state.analysisBpmRange = selected;
    if (el.analysisBpmRangeSelect.value !== selected) {
      el.analysisBpmRangeSelect.value = selected;
    }
    persistSetting(STORAGE_KEY_ANALYSIS_BPM_RANGE, FRONTEND_DB_KEY_ANALYSIS_BPM_RANGE, selected);
    setStatus(`Analysis BPM range: ${selected}`);
  });

  el.analysisEngineSelect?.addEventListener("change", (event) => {
    const selected = String(event?.target?.value || "stratum").toLowerCase();
    const engine = selected === "essentia" ? "essentia" : "stratum";
    if (engine !== "essentia" && state.essentiaDownloading) {
      command("cancel_essentia_download").catch(() => {});
      state.essentiaDownloading = false;
      if (setProgress) setProgress(false, 0, "Idle");
    }
    state.analysisEngine = engine;
    if (el.analysisEngineSelect.value !== engine) {
      el.analysisEngineSelect.value = engine;
    }
    const persistPromise = Promise.resolve(
      persistSetting(STORAGE_KEY_ANALYSIS_ENGINE, FRONTEND_DB_KEY_ANALYSIS_ENGINE, engine)
    ).catch(() => {});
    state.analysisEnginePersistPromise = persistPromise;
    persistPromise.finally(() => {
      if (state.analysisEnginePersistPromise === persistPromise) {
        state.analysisEnginePersistPromise = null;
      }
    });
    renderEssentiaInstallRow(state, el, { openUrl: (url) => openExternalUrl(window, url) });
    const engineLabel = engine === "stratum" ? "Stratum (built-in)" : "Essentia";
    setStatus(`Analysis engine: ${engineLabel}`);
    if (pushEventLog) pushEventLog({ level: "info", source: "settings", message: `Analysis engine changed to ${engineLabel}` });
  });

  el.essentiaDownloadBtn?.addEventListener("click", async () => {
    if (state.essentiaDownloading) return;
    state.essentiaDownloading = true;
    state.essentiaDownloadError = null;
    renderEssentiaInstallRow(state, el, { openUrl: (url) => openExternalUrl(window, url) });
    if (setProgress) setProgress(true, 0, "Downloading Essentia...");
    try {
      await command("download_essentia");
    } catch (err) {
      state.essentiaDownloading = false;
      state.essentiaDownloadError = err?.message || String(err);
      if (setProgress) setProgress(false, 0, "Idle");
      renderEssentiaInstallRow(state, el, { openUrl: (url) => openExternalUrl(window, url) });
    }
  });

  el.essentiaCancelBtn?.addEventListener("click", () => {
    command("cancel_essentia_download").catch(() => {});
  });

  el.essentiaRemoveBtn?.addEventListener("click", async () => {
    try {
      await command("remove_essentia");
      state.essentiaInstalled = false;
      state.analysisEngine = "stratum";
      if (el.analysisEngineSelect) el.analysisEngineSelect.value = "stratum";
      persistSetting(STORAGE_KEY_ANALYSIS_ENGINE, FRONTEND_DB_KEY_ANALYSIS_ENGINE, "stratum");
      renderEssentiaInstallRow(state, el, { openUrl: (url) => openExternalUrl(window, url) });
      setStatus("Essentia removed");
    } catch (err) {
      setStatus(`Remove failed: ${err?.message || String(err)}`);
    }
  });

  // Listen for download progress events from backend
  if (getTauriEventListen) {
    getTauriEventListen().then((listen) => {
      if (!listen) return;
      listen("essentia_download_progress", (event) => {
        const payload = event?.payload;
        if (!payload) return;
        if (payload.done) {
          state.essentiaInstalled = true;
          state.essentiaDownloading = false;
          state.essentiaDownloadError = null;
          if (setProgress) setProgress(false, 0, "Idle");
          renderEssentiaInstallRow(state, el, { openUrl: (url) => openExternalUrl(window, url) });
          setStatus("Essentia installed");
        } else if (payload.error) {
          state.essentiaDownloading = false;
          state.essentiaDownloadError = payload.error;
          if (setProgress) setProgress(false, 0, "Idle");
          renderEssentiaInstallRow(state, el, { openUrl: (url) => openExternalUrl(window, url) });
        } else if (typeof payload.percent === "number") {
          if (setProgress) setProgress(true, payload.percent, "Downloading Essentia...");
        }
      }).catch(() => {});
    }).catch(() => {});
  }

  el.openEventLogBtn?.addEventListener("click", () => {
    closeSettingsDrawer();
    switchView("event-log").catch((err) => {
      console.error(err);
      setStatus(err.message || String(err));
    });
  });

  renderEssentiaInstallRow(state, el, { openUrl: (url) => openExternalUrl(window, url) });
}
