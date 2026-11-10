// The one stubbed backend every e2e page starts with (installed by
// coverage-fixture.mjs before the spec's own init scripts). Answers are shaped
// like the backend's structs (backend/src/models.rs): the frontend reads them
// without re-checking their shape.
//
// A spec's `invoke` handles the commands it cares about and ends with
// `return window.__e2eBackend.answer(command);`. A spec's own answer only
// needs the fields it tests: every successful answer to a command in `SHAPES`
// is filled out to the struct's full shape (see `shapeAnswers`).
(() => {
  const ok = (data) => ({ ok: true, data });
  // The backend owns the UI's text: its tests check the wording. A default here is
  // only a placeholder, and a spec that checks text sends that text itself.

  // `SourceFolder` rows for `LibrarySourcesData`.
  const sourceFolders = (paths) =>
    paths.map((path) => ({ path, label: path, parent: null, enabled: true, missing: false, fullyAnalyzed: false }));

  const AUDIO_FORMATS = ["flac", "alac", "aiff", "wav", "mp3", "aac"];

  const backend = {
    ok,

    // `LibrarySourcesData` (get_library_sources). Defaults to the folders a
    // spec put in `window.__e2eSourceFolders`.
    librarySources(paths = window.__e2eSourceFolders || [], overrides = {}) {
      return {
        folders: sourceFolders(paths),
        rekordboxDbEnabled: false,
        mixxxDbEnabled: false,
        everConfigured: paths.length > 0,
        analyzeButtonLabel: paths.length > 1 ? "Analyze Libraries" : "Analyze Library",
        analyzeScanningStatus: "Scanning library files...",
        trackNotInLibraryStatus: "Track is not in the local library yet. Run Analyze Library first.",
        ...overrides
      };
    },

    // `FetchUsbPlaylistsData` (fetch_usb_playlists).
    usbPlaylists(overrides = {}) {
      const items = (overrides.items || []).map((item) => ({ selectedText: "mock selected", removeConfirmText: "mock remove confirm", ...item }));
      return {
        items: [],
        stats: { indexedTracks: 0, playlistReferencedTracks: 0, playlistEntries: 0 },
        summaryText: "mock playlists summary",
        statusText: "mock playlists status",
        doneText: "mock done",
        warnings: [],
        warningText: "",
        playlistUsbExportStatus: [],
        ...overrides,
        items
      };
    },

    // `FetchUsbHistoriesData` (fetch_usb_histories).
    usbHistories(overrides = {}) {
      const { counts, ...rest } = overrides;
      const items = (overrides.items || []).map((item) => ({ canExportTracklist: !!item.tracks?.length, ...item }));
      return {
        items: [],
        summaryText: "mock histories summary",
        statusText: "mock histories status",
        counts: {
          importedPlaylists: 0,
          importedTracks: 0,
          pdbT11Playlists: 0,
          pdbT12Entries: 0,
          pdbT17Playlists: 0,
          pdbT18Entries: 0,
          edbHistoryRows: 0,
          edbHistoryContentRows: 0,
          ...counts
        },
        warnings: [],
        warningText: "",
        ...rest,
        items
      };
    },

    // `ValidateUsbRootData` (validate_usb_root): an initialized, writable stick.
    validateUsbRoot(overrides = {}) {
      return {
        valid: true,
        normalizedRoot: "/Volumes/USB-TEST",
        canInitialize: false,
        statusText: "mock selection status",
        initHintText: "",
        warnings: [],
        ...overrides
      };
    },

    // `RunUsbDiagnosticsData` (run_usb_diagnostics).
    usbDiagnostics(overrides = {}) {
      return {
        overallStatus: "PASS",
        overview: [],
        sectionGroups: [],
        playlistDetails: [],
        warnings: [],
        durationMs: 1,
        statusText: "mock diagnostics status",
        durationText: "mock duration",
        statusLabel: "PASS", healthText: "USB health: good",
        playlistUsbExportStatus: [],
        ...overrides
      };
    },


    // `RepairUsbDiagnosticsData` (repair_usb_diagnostics, preview or apply).
    repair(overrides = {}) {
      return {
        statusText: "mock repair status",
        summary: "",
        overview: [],
        rows: [],
        nothingToFix: false,
        warnings: [],
        durationMs: 1,
        playlistUsbExportStatus: [],
        usbContentChanged: false,
        engineLibraryEnabled: false,
        ...overrides
      };
    },

    // `UsbPlayerMenuItem`: one player-menu category.
    playerMenuItem(kind, overrides = {}) {
      return {
        menuItemId: kind,
        kind,
        name: `Menu ${kind}`,
        label: overrides.name ?? `Menu ${kind}`,
        isVisible: true,
        sequenceNo: null,
        removable: true,
        canMoveUp: false,
        canMoveDown: false,
        ...overrides
      };
    },

    // `UpdateUsbPlayerMenuConfigData` (get/update/sync player-menu config).
    playerMenuConfig(overrides = {}) {
      return {
        updated: false,
        currentItems: [],
        availableItems: [],
        warnings: [],
        ...overrides
      };
    },

    // `UsbBackgroundColorData` (get/set_usb_background_color).
    usbBackgroundColor(overrides = {}) {
      const pdbColor = overrides.pdbColor ?? 0;
      const edbColor = overrides.edbColor ?? 0;
      const chips = Array.from({ length: 9 }, (_, color) => ({
        color,
        label: `c${color}`,
        selected: pdbColor === edbColor && pdbColor === color
      }));
      return { chips, noteText: "", statusText: "mock colour status", pdbColor: 0, edbColor: 0, writable: true, reason: null, updated: false, warnings: [], ...overrides };
    },

    // `UsbAudioFormatsData` (get/set_usb_audio_formats): every format allowed.
    usbAudioFormats(overrides = {}) {
      return {
        formats: AUDIO_FORMATS.map((format) => ({
          format, label: format.toUpperCase(), allowed: true, locked: false, tooltip: null
        })),
        writable: true,
        hint: "",
        ...overrides
      };
    },

    // `ExportToUsbData` (export_to_usb).
    exportToUsb(overrides = {}) {
      return {
        statusText: "mock export status",
        jobId: "job-export-1",
        playlistId: "",
        playlistName: "",
        usbRoot: "/Volumes/USB-TEST",
        exportedTracks: 0,
        skippedTracks: 0,
        exportedArtworks: 0,
        exportedAnalysisFiles: 0,
        warnings: [],
        warningText: "",
        ...overrides
      };
    },

    // What every page needs answered, and an error for anything else.
    answer(command) {
      switch (command) {
        case "clear_frontend_log": return "";
        case "append_frontend_log":
        case "show_window":
          return null;
        case "get_backend_log_buffer": return [];
        // Startup's one-time data upgrades: nothing pending.
        case "run_data_migrations": return ok({ ran: [], retryLater: [] });
        // The real English texts (ui_texts.en.json, via coverage-fixture.mjs): specs check labels.
        case "get_ui_texts": return ok(window.__e2eUiTexts);
        case "detect_external_rekordbox_db":
        case "detect_external_mixxx_db":
          return ok({ found: false, path: null, imported: false });
        case "get_library_sources": return ok(backend.librarySources());
        case "browse_source_files": return ok({ total: 0, items: [] });
        case "list_playlists":
        case "clear_usb_root":
          return ok({ playlists: [], playlistUsbExportStatus: [] });
        case "get_frontend_settings": return ok({ values: {}, nodeAvailable: true, essentiaInstalled: true });
        case "set_frontend_setting": return ok({ saved: true });
        case "list_usb_devices": return ok({ items: [] });
        case "fetch_usb_playlists": return ok(backend.usbPlaylists());
        case "fetch_usb_histories": return ok(backend.usbHistories());
        case "fetch_usb_playlist_tracks":
        case "fetch_usb_history_tracks":
          return ok({ items: [], total: 0 });
        case "get_usb_audio_formats": return ok(backend.usbAudioFormats());
        case "get_usb_player_menu_config": return ok(backend.playerMenuConfig());
        case "get_usb_background_color": return ok(backend.usbBackgroundColor());
        default:
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled: ${command}` } };
      }
    }
  };

  const withListSummary = (data) => ({ listSummaryText: "mock list summary", ...data });

  // The builders that fill out a spec's own answer to each command.
  const SHAPES = {
    get_library_sources: (data) => backend.librarySources([], data),
    fetch_usb_playlists: backend.usbPlaylists,
    fetch_usb_histories: backend.usbHistories,
    validate_usb_root: backend.validateUsbRoot,
    run_usb_diagnostics: backend.usbDiagnostics,
    repair_usb_diagnostics: (data) => backend.repair(
      data.diagnostics ? { ...data, diagnostics: backend.usbDiagnostics(data.diagnostics) } : data
    ),
    get_usb_player_menu_config: backend.playerMenuConfig,
    update_usb_player_menu_config: backend.playerMenuConfig,
    get_usb_background_color: backend.usbBackgroundColor,
    set_usb_background_color: backend.usbBackgroundColor,
    get_usb_audio_formats: backend.usbAudioFormats,
    set_usb_audio_formats: backend.usbAudioFormats,
    export_to_usb: backend.exportToUsb,
    fetch_usb_playlist_tracks: withListSummary,
    fetch_usb_history_tracks: withListSummary,
    browse_source_files: withListSummary,
    get_playlist_tracks: withListSummary,
    reorder_usb_playlists: (data) => ({ statusText: "mock reorder status", warningText: "", ...data }),
    remove_usb_playlist: (data) => ({ statusText: "mock remove status", warningText: "", ...data }),
    set_usb_device_name: (data) => ({ statusText: "mock rename status", updated: true, ...data }),
    restore_usb_backup: (data) => ({ statusText: "mock restore status", ...data }),
    delete_usb_backup: (data) => ({ statusText: "mock delete status", ...data })
  };

  // Every string the backend answered with: data and response text, which the test-language
  // check (`uiTextCheck` in coverage-fixture.mjs) doesn't count as the UI's own text.
  const answerStrings = new Set();
  window.__e2eAnswerStrings = answerStrings;
  function recordStrings(value, depth = 0) {
    if (typeof value === "string") answerStrings.add(value);
    else if (value && typeof value === "object" && depth < 12) {
      for (const item of Object.values(value)) recordStrings(item, depth + 1);
    }
  }

  // The commands that change the playlists: the backend answers each with the
  // playlists as they now are (`WithPlaylists`), i.e. what list_playlists
  // answers. A spec's answer that leaves them out gets its own list_playlists'.
  const PLAYLIST_CHANGES = new Set([
    "create_playlist", "rename_playlist", "delete_playlist", "remove_tracks_from_playlist",
    "reorder_playlist_tracks", "add_track_candidates_to_playlist", "add_track_candidates_to_playlists",
    "add_library_selection_to_playlist", "add_library_selection_to_playlists",
    "import_external_playlists", "export_to_usb", "set_usb_device_name"
  ]);

  async function withPlaylists(invoke, command, raw) {
    if (!PLAYLIST_CHANGES.has(command) || !raw?.ok || !raw.data || typeof raw.data !== "object") return raw;
    if ("playlists" in raw.data) return raw;
    const listed = await invoke("list_playlists", {});
    return listed?.ok ? { ...raw, data: { ...raw.data, ...listed.data } } : raw;
  }

  function shaped(invoke) {
    return async (command, payload) => {
      const raw = await withPlaylists(invoke, command, await invoke(command, payload));
      const shape = SHAPES[command];
      const result = !shape || !raw?.ok || !raw.data || typeof raw.data !== "object"
        ? raw
        : { ...raw, data: shape(raw.data) };
      if (command !== "get_ui_texts") recordStrings(result);
      return result;
    };
  }

  // Wraps `core.invoke` -- including one a spec swaps in later, which may
  // call the one it replaced -- so its answers come out shaped.
  function shapeAnswers(core) {
    if (!core || typeof core.invoke !== "function") return;
    let inner = core.invoke;
    Object.defineProperty(core, "invoke", {
      configurable: true,
      enumerable: true,
      get: () => shaped(inner),
      set: (fn) => { inner = fn; }
    });
  }

  let tauri;
  Object.defineProperty(window, "__TAURI__", {
    configurable: true,
    get: () => tauri,
    set: (value) => {
      tauri = value;
      shapeAnswers(value?.core);
    }
  });

  window.__e2eBackend = backend;
})();
