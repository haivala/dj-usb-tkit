import { openExternalUrl, cloneTemplate } from "../../ui_utils.mjs";
import { normalizeAnalysisBpmRange } from "../library/actions.mjs";
import { refreshUpdateCheck } from "../../update_check.mjs";
import { t, uiLanguage, uiLanguages } from "../../ui_texts.mjs";
import {
  STORAGE_KEY_HELP_SEEN,
  FRONTEND_DB_KEY_HELP_SEEN,
  STORAGE_KEY_EXPORT_PRUNE_STALE,
  FRONTEND_DB_KEY_EXPORT_PRUNE_STALE,
  STORAGE_KEY_EXPORT_BACKUP,
  FRONTEND_DB_KEY_EXPORT_BACKUP,
  STORAGE_KEY_EXPORT_ENGINE_LIBRARY,
  FRONTEND_DB_KEY_EXPORT_ENGINE_LIBRARY,
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

  const show = state.analysisEngine === "essentia";
  el.essentiaInstallRow.classList.toggle("hidden", !show);
  if (!show) return;

  const { nodeAvailable, essentiaInstalled, essentiaDownloading, essentiaDownloadError } = state;

  // Node status line
  if (!nodeAvailable) {
    const missing = cloneTemplate(el.essentiaNodeStatus.ownerDocument, "tplEssentiaNodeMissing");
    missing.querySelector(".essentia-node-link")
      .addEventListener("click", (e) => { e.preventDefault(); openExternalUrl(ctx.window, NODE_JS_URL); });
    el.essentiaNodeStatus.replaceChildren(...missing.childNodes);
  } else if (essentiaInstalled) {
    el.essentiaNodeStatus.textContent = t("ui-settings-essentia-ready");
    el.essentiaNodeStatus.classList.add("essentia-ready");
    el.essentiaNodeStatus.classList.remove("essentia-warn");
  } else if (essentiaDownloading) {
    el.essentiaNodeStatus.textContent = t("ui-settings-essentia-downloading");
    el.essentiaNodeStatus.classList.remove("essentia-ready", "essentia-warn");
  } else if (essentiaDownloadError) {
    el.essentiaNodeStatus.textContent = t("ui-settings-essentia-download-failed", { error: essentiaDownloadError });
    el.essentiaNodeStatus.classList.add("essentia-warn");
    el.essentiaNodeStatus.classList.remove("essentia-ready");
  } else {
    el.essentiaNodeStatus.textContent = t("ui-settings-essentia-not-installed");
    el.essentiaNodeStatus.classList.remove("essentia-ready", "essentia-warn");
  }

  // Button visibility
  el.essentiaDownloadBtn.classList.toggle("hidden", essentiaDownloading || essentiaInstalled);
  el.essentiaCancelBtn.classList.toggle("hidden", !essentiaDownloading);
  el.essentiaRemoveBtn.classList.toggle("hidden", !essentiaInstalled);
}

export function bindSettingsEvents(ctx) {
  const {
    state,
    el,
    document,
    window,
    persistSetting,
    emitStatus,
    setProgress,
    command,
    getTauriEventListen,
    pushEventLog,
    closeSettingsDrawer,
    updatePlaylistExportButtons,
    getCurrentPlaylist,
    renderCurrentPlaylistTracksFromState,
    commitActivePlaylistSort,
    isPlaylistSortActive,
    loadPlaylists,
    reloadTrackListsForKeyNotation
  } = ctx;
  el.settingsBtn.addEventListener("click", () => {
    el.settingsDrawer.classList.remove("hidden");
    el.settingsBackdrop.classList.remove("hidden");
    loadTranscodeFolder(ctx).catch(catchTranscodeFolderError);
  });
  const catchTranscodeFolderError = (err) => ctx.emitStatus(err?.message || String(err));
  el.transcodeFolderChangeBtn.addEventListener("click", () => {
    (async () => {
      const picked = await ctx.invoke("pick_transcode_folder");
      if (!picked) return;
      renderTranscodeFolder(el, await ctx.command("set_transcode_folder", { path: String(picked) }));
    })().catch(catchTranscodeFolderError);
  });
  el.transcodeFolderResetBtn.addEventListener("click", () => {
    ctx.command("set_transcode_folder", { path: null })
      .then((folder) => renderTranscodeFolder(el, folder))
      .catch(catchTranscodeFolderError);
  });
  el.settingsCloseBtn.addEventListener("click", closeSettingsDrawer);
  el.settingsUpdateCheckBtn.addEventListener("click", () => {
    refreshUpdateCheck(ctx, { manual: true });
  });
  el.settingsBackdrop.addEventListener("click", closeSettingsDrawer);

  el.helpBtn.addEventListener("click", () => {
    el.helpOverlay.classList.remove("hidden");
  });
  el.helpCloseBtn.addEventListener("click", () => {
    el.helpOverlay.classList.add("hidden");
    persistSetting(STORAGE_KEY_HELP_SEEN, FRONTEND_DB_KEY_HELP_SEEN, "1");
  });
  el.helpOverlay.addEventListener("click", (event) => {
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

  el.exportSyncModeGroup.addEventListener("change", async (event) => {
    const mode = String(event?.target?.value || "").toLowerCase();
    state.exportPruneStale = mode !== "additive";
    persistSetting(
      STORAGE_KEY_EXPORT_PRUNE_STALE,
      FRONTEND_DB_KEY_EXPORT_PRUNE_STALE,
      state.exportPruneStale ? "1" : "0"
    );

    const openPlaylist = getCurrentPlaylist();
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
    if (willLock && isPlaylistSortActive()) {
      try {
        await commitActivePlaylistSort(openPlaylist.id);
      } catch (err) {
        console.error(err);
        sortSaveFailed = true;
      }
    }

    // The setting is now persisted; the playlists' reorder locks follow it.
    try {
      await loadPlaylists();
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

    emitStatus(
      willLock
        ? t(sortSaveFailed ? "ui-settings-sync-mode-additive-locked-sort-failed" : "ui-settings-sync-mode-additive-locked", { playlist: openPlaylist.name })
        : t(state.exportPruneStale ? "ui-settings-sync-mode-mirror-saved" : "ui-settings-sync-mode-additive-saved")
    );
  });

  el.exportBackupCheckbox.addEventListener("change", (event) => {
    state.exportBackup = !!event?.target?.checked;
    persistSetting(
      STORAGE_KEY_EXPORT_BACKUP,
      FRONTEND_DB_KEY_EXPORT_BACKUP,
      state.exportBackup ? "1" : "0"
    );
    emitStatus(t(state.exportBackup ? "ui-settings-export-backup-enabled" : "ui-settings-export-backup-disabled"));
  });

  el.exportEngineLibraryCheckbox.addEventListener("change", (event) => {
    state.exportEngineLibrary = !!event?.target?.checked;
    persistSetting(
      STORAGE_KEY_EXPORT_ENGINE_LIBRARY,
      FRONTEND_DB_KEY_EXPORT_ENGINE_LIBRARY,
      state.exportEngineLibrary ? "1" : "0"
    );
    emitStatus(
      t(state.exportEngineLibrary ? "ui-settings-engine-library-enabled" : "ui-settings-engine-library-disabled")
    );
  });

  el.backupRetentionCountInput.addEventListener("change", (event) => {
    const parsed = Number.parseInt(event?.target?.value, 10);
    const count = Number.isFinite(parsed) && parsed >= 1 ? Math.min(parsed, 999) : 10;
    state.backupRetentionCount = count;
    if (el.backupRetentionCountInput.value !== String(count)) {
      el.backupRetentionCountInput.value = String(count);
    }
    persistSetting(STORAGE_KEY_BACKUP_RETENTION_COUNT, FRONTEND_DB_KEY_BACKUP_RETENTION_COUNT, String(count));
    emitStatus(t("ui-settings-backup-retention-saved", { value: count }));
  });

  el.analysisBpmRangeSelect.addEventListener("change", (event) => {
    const selected = normalizeAnalysisBpmRange(event?.target?.value);
    state.analysisBpmRange = selected;
    if (el.analysisBpmRangeSelect.value !== selected) {
      el.analysisBpmRangeSelect.value = selected;
    }
    persistSetting(STORAGE_KEY_ANALYSIS_BPM_RANGE, FRONTEND_DB_KEY_ANALYSIS_BPM_RANGE, selected);
    emitStatus(t("ui-settings-bpm-range-saved", { range: selected }));
  });

  // The language every text is in. A change reloads the window, so everything is read
  // again in it; the backend refuses it while any job runs, which a reload would lose.
  el.languageSelect.replaceChildren(...uiLanguages().map(({ code, name }) => {
    const option = cloneTemplate(ctx.document, "tplLanguageOption");
    option.value = code;
    option.textContent = name;
    return option;
  }));
  el.languageSelect.value = uiLanguage();
  el.languageSelect.addEventListener("change", async (event) => {
    const select = event.currentTarget;
    const current = uiLanguage();
    if (select.value === current) return;
    try {
      await ctx.command("set_language", { language: select.value });
    } catch (err) {
      select.value = current;
      emitStatus(err?.message || String(err));
      return;
    }
    // The player is the backend's: it would play on with no control on screen.
    await ctx.stopPlaybackIfActive().catch(() => {});
    ctx.window.location.reload();
  });

  // Display only -- the backend renders every key label in this notation, so
  // save it first, then re-fetch the loaded track lists.
  el.keyNotationSelect.addEventListener("change", async (event) => {
    const notation = event?.target?.value === "camelot" ? "camelot" : "classic";
    state.keyNotation = notation;
    try {
      await persistSetting(STORAGE_KEY_KEY_NOTATION, FRONTEND_DB_KEY_KEY_NOTATION, notation);
    } catch {
      emitStatus(t("ui-settings-key-notation-failed"));
      return;
    }
    try {
      await reloadTrackListsForKeyNotation();
    } catch (err) {
      console.error(err);
    }
    emitStatus(t("ui-settings-key-notation-saved", { notation: t(notation === "camelot" ? "ui-settings-key-notation-camelot" : "ui-settings-key-notation-classic") }));
  });

  el.analysisEngineSelect.addEventListener("change", (event) => {
    const selected = String(event?.target?.value || "stratum").toLowerCase();
    const engine = selected === "essentia" ? "essentia" : "stratum";
    if (engine !== "essentia" && state.essentiaDownloading) {
      command("cancel_essentia_download").catch(() => {});
      state.essentiaDownloading = false;
      setProgress(false, 0, t("ui-job-idle"));
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
    const engineLabel = engine === "stratum" ? t("ui-settings-analysis-engine-stratum") : "Essentia";
    emitStatus(t("ui-settings-analysis-engine-saved", { engine: engineLabel }));
    if (pushEventLog) {
      pushEventLog({ level: "info", source: "settings", message: t("ui-settings-analysis-engine-changed", { engine: engineLabel }) });
    }
  });

  el.essentiaDownloadBtn.addEventListener("click", async () => {
    if (state.essentiaDownloading) return;
    state.essentiaDownloading = true;
    state.essentiaDownloadError = null;
    renderEssentiaInstallRow(ctx);
    setProgress(true, 0, t("ui-settings-essentia-downloading-progress"));
    try {
      await command("download_essentia");
    } catch (err) {
      state.essentiaDownloading = false;
      state.essentiaDownloadError = err?.message || String(err);
      setProgress(false, 0, t("ui-job-idle"));
      renderEssentiaInstallRow(ctx);
    }
  });

  el.essentiaCancelBtn.addEventListener("click", () => {
    command("cancel_essentia_download").catch(() => {});
  });

  el.essentiaRemoveBtn.addEventListener("click", async () => {
    try {
      await command("remove_essentia");
      state.essentiaInstalled = false;
      state.analysisEngine = "stratum";
      el.analysisEngineSelect.value = "stratum";
      persistSetting(STORAGE_KEY_ANALYSIS_ENGINE, FRONTEND_DB_KEY_ANALYSIS_ENGINE, "stratum");
      renderEssentiaInstallRow(ctx);
      emitStatus(t("ui-settings-essentia-removed"));
    } catch (err) {
      emitStatus(err?.message || String(err));
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
          setProgress(false, 0, t("ui-job-idle"));
          renderEssentiaInstallRow(ctx);
          emitStatus(t("ui-settings-essentia-installed"));
        } else if (payload.error) {
          state.essentiaDownloading = false;
          state.essentiaDownloadError = payload.error;
          setProgress(false, 0, t("ui-job-idle"));
          renderEssentiaInstallRow(ctx);
        } else if (typeof payload.percent === "number") {
          setProgress(true, payload.percent, t("ui-settings-essentia-downloading-progress"));
        }
      }).catch(() => {});
    }).catch(() => {});
  }

  renderEssentiaInstallRow(ctx);
}

// Settings → Transcodes: the backend's folder (the chosen one, or its
// default inside the app data).
async function loadTranscodeFolder(ctx) {
  renderTranscodeFolder(ctx.el, await ctx.command("get_transcode_folder"));
}

function renderTranscodeFolder(el, folder) {
  el.transcodeFolderText.textContent = String(folder?.path || "");
  el.transcodeFolderDefaultTag.hidden = !folder?.isDefault;
  el.transcodeFolderResetBtn.hidden = !!folder?.isDefault;
}
