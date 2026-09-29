import { openExternalUrl, cloneTemplate } from "../../ui_utils.mjs";
import { normalizeAnalysisBpmRange } from "../library/actions.mjs";
import {
  STORAGE_KEY_HELP_SEEN,
  FRONTEND_DB_KEY_HELP_SEEN,
  STORAGE_KEY_EXPORT_PRUNE_STALE,
  FRONTEND_DB_KEY_EXPORT_PRUNE_STALE,
  STORAGE_KEY_EXPORT_BACKUP,
  FRONTEND_DB_KEY_EXPORT_BACKUP,
  STORAGE_KEY_BACKUP_RETENTION_COUNT,
  FRONTEND_DB_KEY_BACKUP_RETENTION_COUNT,
  STORAGE_KEY_ANALYSIS_BPM_RANGE,
  FRONTEND_DB_KEY_ANALYSIS_BPM_RANGE,
  STORAGE_KEY_ANALYSIS_ENGINE,
  FRONTEND_DB_KEY_ANALYSIS_ENGINE,
  STORAGE_KEY_KEY_NOTATION,
  FRONTEND_DB_KEY_KEY_NOTATION
} from "../../settings_keys.mjs";

const NODE_JS_URL = "https://nodejs.org/";
const WEBSITE_URL = "https://chiph.art/en/projects/dj-usb-tkit?utm_source=djtkit&utm_medium=app&utm_campaign=sidebar";
const SUPPORT_URL = "https://chiph.art/en/dj-usb-tkit/support?utm_source=djtkit&utm_medium=app&utm_campaign=support";

export function renderEssentiaInstallRow(ctx) {
  const { state, el } = ctx;
  if (!el.essentiaInstallRow) return;

  const show = state.analysisEngine === "essentia";
  el.essentiaInstallRow.classList.toggle("hidden", !show);
  if (!show) return;

  const { nodeAvailable, essentiaInstalled, essentiaDownloading, essentiaDownloadError } = state;

  // Node status line
  if (el.essentiaNodeStatus) {
    if (!nodeAvailable) {
      const missing = cloneTemplate(el.essentiaNodeStatus.ownerDocument, "tplEssentiaNodeMissing");
      missing.querySelector(".essentia-node-link")
        .addEventListener("click", (e) => { e.preventDefault(); openExternalUrl(ctx.window, NODE_JS_URL); });
      el.essentiaNodeStatus.replaceChildren(...missing.childNodes);
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
    persistSetting,
    setStatus,
    setProgress,
    command,
    getTauriEventListen,
    pushEventLog,
    closeSettingsDrawer,
    switchView,
    updatePlaylistExportButtons,
    getCurrentPlaylist,
    renderCurrentPlaylistTracksFromState,
    commitActivePlaylistSort,
    isPlaylistSortActive,
    refreshPlaylistExportStatus,
    reloadTrackListsForKeyNotation
  } = ctx;
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

  el.exportSyncModeGroup?.addEventListener("change", async (event) => {
    const mode = String(event?.target?.value || "").toLowerCase();
    state.exportPruneStale = mode !== "additive";
    persistSetting(
      STORAGE_KEY_EXPORT_PRUNE_STALE,
      FRONTEND_DB_KEY_EXPORT_PRUNE_STALE,
      state.exportPruneStale ? "1" : "0"
    );

    const openPlaylist = getCurrentPlaylist?.() || null;
    // Predict whether additive mode is about to lock the open playlist purely
    // to sequence the sort commit below -- `sameNameExistsOnUsb` is a
    // backend-provided fact already on the status map. The authoritative
    // `locksReorder` comes from the backend refresh right after.
    const willLock = !!openPlaylist
      && !state.exportPruneStale
      && !!state.playlistUsbExportStatusById?.get(openPlaylist.id)?.sameNameExistsOnUsb;

    // Switching mirror -> additive would freeze an in-progress column sort where
    // it can never be committed, so commit it first, while still unlocked. The
    // mode change is a global preference and always goes through -- if the save
    // fails we just say so and carry on (the sort was only a view).
    let sortSaveFailed = false;
    if (willLock && isPlaylistSortActive?.()) {
      try {
        await commitActivePlaylistSort(openPlaylist.id);
      } catch (err) {
        console.error(err);
        sortSaveFailed = true;
      }
    }

    // The setting is now persisted; ask the backend to recompute every
    // playlist's reorder lock against it (cheap -- staged PDB/eDB, no USB scan).
    try {
      await refreshPlaylistExportStatus?.();
    } catch (err) {
      console.error(err);
    }

    if (openPlaylist) {
      try {
        await renderCurrentPlaylistTracksFromState();
      } catch (err) {
        console.error(err);
      }
    }
    updatePlaylistExportButtons();

    setStatus(
      willLock
        ? `Export sync mode: additive — "${openPlaylist.name}" already exists on USB, so its track order is locked here (${sortSaveFailed ? "couldn't save the current sort first" : "current order kept"}).`
        : state.exportPruneStale
          ? "Export sync mode: mirror (exact match)"
          : "Export sync mode: additive"
    );
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

  el.backupRetentionCountInput?.addEventListener("change", (event) => {
    const parsed = Number.parseInt(event?.target?.value, 10);
    const count = Number.isFinite(parsed) && parsed >= 1 ? Math.min(parsed, 999) : 10;
    state.backupRetentionCount = count;
    if (el.backupRetentionCountInput.value !== String(count)) {
      el.backupRetentionCountInput.value = String(count);
    }
    persistSetting(STORAGE_KEY_BACKUP_RETENTION_COUNT, FRONTEND_DB_KEY_BACKUP_RETENTION_COUNT, String(count));
    setStatus(`Backups to keep per file: ${count}`);
  });

  el.openBackupsBtn?.addEventListener("click", () => {
    closeSettingsDrawer();
    switchView("backups").catch((err) => {
      console.error(err);
      setStatus(err.message || String(err));
    });
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

  // Display only -- the backend renders every key label in this notation, so
  // save it first, then re-fetch the loaded track lists.
  el.keyNotationSelect?.addEventListener("change", async (event) => {
    const notation = event?.target?.value === "camelot" ? "camelot" : "classic";
    state.keyNotation = notation;
    try {
      await persistSetting(STORAGE_KEY_KEY_NOTATION, FRONTEND_DB_KEY_KEY_NOTATION, notation);
    } catch {
      setStatus("Could not save key notation");
      return;
    }
    try {
      await reloadTrackListsForKeyNotation?.();
    } catch (err) {
      console.error(err);
    }
    setStatus(`Key notation: ${notation === "camelot" ? "Camelot (8A)" : "Classic (Am)"}`);
  });

  el.analysisEngineSelect?.addEventListener("change", (event) => {
    const selected = String(event?.target?.value || "stratum").toLowerCase();
    const engine = selected === "essentia" ? "essentia" : "stratum";
    if (engine !== "essentia" && state.essentiaDownloading) {
      command("cancel_essentia_download").catch(() => {});
      state.essentiaDownloading = false;
      setProgress(false, 0, "Idle");
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
    renderEssentiaInstallRow(ctx);
    const engineLabel = engine === "stratum" ? "Stratum (built-in)" : "Essentia";
    setStatus(`Analysis engine: ${engineLabel}`);
    if (pushEventLog) pushEventLog({ level: "info", source: "settings", message: `Analysis engine changed to ${engineLabel}` });
  });

  el.essentiaDownloadBtn?.addEventListener("click", async () => {
    if (state.essentiaDownloading) return;
    state.essentiaDownloading = true;
    state.essentiaDownloadError = null;
    renderEssentiaInstallRow(ctx);
    setProgress(true, 0, "Downloading Essentia...");
    try {
      await command("download_essentia");
    } catch (err) {
      state.essentiaDownloading = false;
      state.essentiaDownloadError = err?.message || String(err);
      setProgress(false, 0, "Idle");
      renderEssentiaInstallRow(ctx);
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
      renderEssentiaInstallRow(ctx);
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
          setProgress(false, 0, "Idle");
          renderEssentiaInstallRow(ctx);
          setStatus("Essentia installed");
        } else if (payload.error) {
          state.essentiaDownloading = false;
          state.essentiaDownloadError = payload.error;
          setProgress(false, 0, "Idle");
          renderEssentiaInstallRow(ctx);
        } else if (typeof payload.percent === "number") {
          setProgress(true, payload.percent, "Downloading Essentia...");
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

  renderEssentiaInstallRow(ctx);
}
