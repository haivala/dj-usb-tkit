// The one stubbed backend every e2e page starts with (installed by
// coverage-fixture.mjs before the spec's own init scripts). Answers are shaped
// like the backend's structs (backend/src/models.rs): the frontend reads them
// without re-checking their shape.
//
// A spec seeds the library and playlists (`seed`) and installs its own
// handler for the commands it cares about (`install`); every other command
// gets the default answer (`answer`), which reads and changes what was seeded
// as the backend does. A spec's own answer only
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

  // Every page starts past the first-visit help.
  window.localStorage.setItem("djusbtkit.helpSeen", "1");

  // The backend's events: `emit` plays one at the app's `listen` callbacks.
  const listeners = new Map();
  const listen = async (eventName, callback) => {
    listeners.set(eventName, [...(listeners.get(eventName) || []), callback]);
    return () => listeners.set(eventName, (listeners.get(eventName) || []).filter((fn) => fn !== callback));
  };
  const emit = (eventName, payload) => {
    for (const callback of (listeners.get(eventName) || []).slice()) callback({ event: eventName, payload });
  };
  window.__emit = emit;
  window.__emitJobEvent = (payload) => emit("job:event", payload);
  // What the backend's `logging::log` sends for each warning it makes.
  window.__emitBackendLog = (payload) => emit("backend:log", payload);
  window.__hasJobListener = () => (listeners.get("job:event") || []).length > 0;

  // The footer's "Total time" line, as the backend words it.
  const totalTimeText = (totalMs, unknown) => {
    const seconds = Math.floor(totalMs / 1000);
    const time = `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
    return `Total time: ${time}${unknown ? ` (${unknown} without length)` : ""}`;
  };
  const hasLength = (track) => track.durationMs > 0;

  // The backend's TrackSort: numbers by value, "artist" ties broken by title.
  const sortRows = (rows, sortBy, sortDir) => {
    if (!sortBy) return rows.slice();
    const m = sortDir === "desc" ? -1 : 1;
    const value = (t) => (sortBy === "artist" ? `${t.artist}\0${t.title}` : t[sortBy]);
    return rows.slice().sort((a, b) => {
      const av = value(a);
      const bv = value(b);
      if (typeof av === "number" || typeof bv === "number") return m * ((Number(av) || 0) - (Number(bv) || 0));
      const as = String(av ?? "");
      const bs = String(bv ?? "");
      return as < bs ? -m : as > bs ? m : 0;
    });
  };

  // The rows matching a search (title, artist, album), as the backend's queries do.
  const searchRows = (rows, query) => {
    const q = String(query || "").trim().toLowerCase();
    return q ? rows.filter((t) => `${t.title} ${t.artist} ${t.album || ""}`.toLowerCase().includes(q)) : rows.slice();
  };

  // One page of a track list (browse_source_files, get_playlist_tracks,
  // fetch_usb_*_tracks): searched, sorted and paged by the request, with the
  // footer's totals. `countable` decides which tracks count toward the total
  // time; `map` shapes each row on the page.
  function listPage(all, request = {}, { pageSize, countable = hasLength, map = (t) => t } = {}) {
    const rows = sortRows(searchRows(all || [], request.query), request.sortBy, request.sortDir);
    const offset = Number(request.offset || 0) || 0;
    const limit = pageSize || Number(request.limit || 0) || rows.length;
    const items = rows.slice(offset, offset + limit).map(map);
    const next = offset + items.length;
    const counted = rows.filter(countable);
    const totalDurationMs = counted.reduce((sum, t) => sum + t.durationMs, 0);
    return {
      total: rows.length,
      items,
      nextCursor: next < rows.length ? String(next) : null,
      hasMore: next < rows.length,
      totalDurationMs,
      durationUnknownCount: rows.length - counted.length,
      durationText: totalTimeText(totalDurationMs, rows.length - counted.length)
    };
  }

  // A playlist's track order after `reorder_playlist_tracks`: a whole order,
  // a sort, or one track moved before another.
  function applyReorder(list, request) {
    const byId = new Map(list.map((t) => [t.id, t]));
    let ids = list.map((t) => t.id);
    if (request.orderedTrackIds?.length) ids = request.orderedTrackIds;
    else if (request.sortBy) ids = sortRows(list, request.sortBy, request.sortDir).map((t) => t.id);
    else if (request.moveTrackId) {
      ids = ids.filter((id) => id !== request.moveTrackId);
      const at = request.beforeTrackId ? ids.indexOf(request.beforeTrackId) : -1;
      ids.splice(at < 0 ? ids.length : at, 0, request.moveTrackId);
    }
    return ids.map((id) => byId.get(id)).filter(Boolean);
  }

  // The library and the app's playlists, as `seed` sets them; the default
  // answers below read and change them as the backend does.
  const store = { tracks: [], playlists: [], playlistTracks: new Map(), created: 0 };
  const playlistRow = (playlist) => ({
    source: "local", lastExportedAt: null, lastExportedUsbRoot: null, lastExportedTrackCount: null, ...playlist
  });
  const playlistName = (id) => store.playlists.find((p) => p.id === id)?.name;
  const tracksText = (n) => `${n} ${n === 1 ? "track" : "tracks"}`;
  // Adds library tracks (by id) to a playlist, skipping ones already in it.
  const addToPlaylist = (playlistId, ids) => {
    const current = store.playlistTracks.get(playlistId) || [];
    const fresh = ids.filter((id, i) => ids.indexOf(id) === i && !current.some((t) => t.id === id))
      .map((id) => store.tracks.find((t) => t.id === id) || { id });
    store.playlistTracks.set(playlistId, [...current, ...fresh]);
    return { playlistId, added: fresh.length, skipped: ids.length - fresh.length };
  };
  const addedText = (added, skipped, names) => `Added ${tracksText(added)} (skipped ${skipped}) to ${names}`;
  const candidateIds = (tracks) => (tracks || [])
    .map((t) => String(t?.localTrackId || t?.trackId || "").trim()).filter(Boolean);

  // The default answers that read the store (undefined: not one of them).
  function storeAnswer(command, r = {}) {
    switch (command) {
      case "browse_source_files": return ok(listPage(store.tracks, r));
      case "list_matching_track_ids": {
        const trackIds = searchRows(store.tracks, r.query).map((t) => t.id);
        return ok({ trackIds, total: trackIds.length });
      }
      case "list_playlists":
        return ok({ playlists: store.playlists.map(playlistRow), playlistUsbExportStatus: [] });
      case "get_playlist_tracks": {
        const all = store.playlistTracks.get(r.playlistId) || [];
        const unanalyzed = searchRows(all, r.query).filter((t) => !t.analysisReady).length;
        return ok({
          playlistId: r.playlistId,
          ...listPage(all, r),
          unanalyzedCount: unanalyzed,
          analyzeMissingLabel: `Analyze Missing Tracks (${unanalyzed})`,
          // The heading's totals cover the whole playlist, ignoring the search.
          listTrackCount: all.length,
          listSummaryText: tracksText(all.length),
          listSizeBytes: all.reduce((sum, t) => sum + (t.fileSizeBytes > 0 ? t.fileSizeBytes : 0), 0)
        });
      }
      case "create_playlist": {
        const name = String(r.name || "Untitled");
        store.created += 1;
        const playlistId = `pl-${store.created}`;
        // New playlists go to the top of the sidebar order.
        store.playlists.unshift({ id: playlistId, name });
        store.playlistTracks.set(playlistId, []);
        return ok({ playlistId, name, statusText: `Playlist created: ${name}` });
      }
      case "rename_playlist": {
        const row = store.playlists.find((p) => p.id === r.playlistId);
        const name = String(r.name || "").trim();
        if (!row || !name) return { ok: false, error: { code: "VALIDATION", message: "rename failed" } };
        row.name = name;
        return ok({ playlistId: r.playlistId, name });
      }
      case "delete_playlist": {
        const name = playlistName(r.playlistId);
        store.playlists = store.playlists.filter((p) => p.id !== r.playlistId);
        return ok({ playlistId: r.playlistId, deleted: name !== undefined, statusText: `Playlist deleted: ${name}` });
      }
      case "reorder_playlists": {
        const ids = r.orderedPlaylistIds || [];
        store.playlists = ids.map((id) => store.playlists.find((p) => p.id === id)).filter(Boolean);
        return ok({ reordered: ids.length });
      }
      case "reorder_playlist_tracks": {
        const reordered = applyReorder(store.playlistTracks.get(r.playlistId) || [], r);
        store.playlistTracks.set(r.playlistId, reordered);
        return ok({ playlistId: r.playlistId, reordered: reordered.length });
      }
      case "remove_tracks_from_playlist": {
        const gone = new Set((r.trackIds || []).map(String));
        const current = store.playlistTracks.get(r.playlistId) || [];
        const kept = current.filter((t) => !gone.has(String(t.id)));
        store.playlistTracks.set(r.playlistId, kept);
        const removed = current.length - kept.length;
        return ok({ playlistId: r.playlistId, removed, statusText: `Removed ${tracksText(removed)} from ${playlistName(r.playlistId)}` });
      }
      case "get_playlist_membership": {
        const ids = r.trackIds || [];
        const items = store.playlists.map((p) => {
          const held = (store.playlistTracks.get(p.id) || []).map((t) => t.id);
          const contained = ids.filter((id) => id && held.includes(id)).length;
          const state = contained === 0 ? "none" : contained >= ids.length ? "all" : "some";
          const noteText = { none: "", all: "Already in", some: `${contained} of ${ids.length} in` }[state];
          return { playlistId: p.id, name: p.name, contained, state, noteText };
        });
        return ok({ items });
      }
      case "add_library_selection_to_playlist": {
        // `allMatching`: every track the library search matches.
        const ids = r.allMatching ? searchRows(store.tracks, r.query).map((t) => t.id) : (r.trackIds || []).map(String);
        const result = addToPlaylist(r.playlistId, ids);
        return ok({ ...result, statusText: addedText(result.added, result.skipped, playlistName(r.playlistId)) });
      }
      case "add_track_candidates_to_playlist": {
        const ids = candidateIds(r.tracks);
        const result = addToPlaylist(r.playlistId, ids);
        return ok({
          ...result,
          requested: (r.tracks || []).length, resolved: ids.length, unresolved: (r.tracks || []).length - ids.length,
          statusText: addedText(result.added, result.skipped, playlistName(r.playlistId)),
          resolutions: (r.tracks || []).map((t) => {
            const trackId = candidateIds([t])[0] || null;
            return { previousId: t?.trackId || null, trackId, resolvedBy: trackId ? "self" : "none", materialized: false };
          })
        });
      }
      case "add_library_selection_to_playlists":
      case "add_track_candidates_to_playlists": {
        const library = command === "add_library_selection_to_playlists";
        const ids = library ? (r.trackIds || []).map(String) : candidateIds(r.tracks);
        const requested = library ? ids.length : (r.tracks || []).length;
        const items = (r.playlistIds || []).map((id) => addToPlaylist(id, ids));
        const sum = (key) => items.reduce((total, item) => total + item[key], 0);
        // The playlists' names in request order.
        const names = (r.playlistIds || []).map(playlistName).join(", ");
        return ok({
          requested, resolved: ids.length, unresolved: requested - ids.length,
          added: sum("added"), skipped: sum("skipped"), items, resolutions: [],
          statusText: ids.length ? addedText(sum("added"), sum("skipped"), names) : ""
        });
      }
      default: return undefined;
    }
  }

  const backend = {
    ok,
    // An error answer, as the backend's `AppError` arrives.
    fail: (message, code = "INTERNAL") => ({ ok: false, error: { code, message } }),
    emit,
    listPage,
    applyReorder,
    totalTimeText,
    store,

    // Sets the library's tracks and the app's playlists the default answers
    // use. A playlist's `tracks` are its rows, in order.
    seed({ tracks, playlists } = {}) {
      if (tracks) store.tracks = tracks;
      if (playlists) {
        store.playlists = playlists.map(({ tracks: rows, ...playlist }) => playlist);
        store.playlistTracks = new Map(playlists.map((p) => [p.id, p.tracks || []]));
      }
    },

    // The real Tauri runtime: the app then subscribes to the backend's events,
    // and the bundled @tauri-apps/api invokes through `__TAURI_INTERNALS__`.
    useTauriRuntime() {
      window.isTauri = true;
      window.__TAURI_INTERNALS__ = {
        invoke: (cmd, args) => window.__TAURI__.core.invoke(cmd, args),
        transformCallback: (callback) => callback,
        convertFileSrc: (filePath) => filePath
      };
    },

    // Installs the stubbed backend. `handle(command, request, payload)`
    // answers the commands a spec cares about; whatever it leaves undefined
    // gets the default answer. Every call lands in `window.__calls`.
    install(handle, { tauriRuntime = false } = {}) {
      window.__calls = [];
      if (tauriRuntime) backend.useTauriRuntime();
      window.__TAURI__ = {
        core: {
          invoke: async (command, payload = {}) => {
            const request = payload?.request ?? payload ?? {};
            window.__calls.push({ command, payload, request: payload?.request ?? null });
            const answer = handle ? await handle(command, request, payload) : undefined;
            return answer === undefined ? backend.answer(command, payload) : answer;
          },
          convertFileSrc: (path) => path
        },
        event: { listen }
      };
    },

    // Answers some commands differently from the backend installed before:
    // `handle(command, request, payload, next)` returns an answer, or
    // undefined to pass the command on (`next()` asks the one before).
    override(handle) {
      const inner = window.__TAURI__.core.invoke;
      window.__TAURI__.core.invoke = async (command, payload = {}) => {
        const next = () => inner(command, payload);
        const answer = await handle(command, payload?.request ?? payload ?? {}, payload, next);
        return answer === undefined ? next() : answer;
      };
    },

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
        filtering: false,
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

    // `SelectUsbData` (select_usb): an initialized, writable stick with every
    // audio format allowed, its name unread (no naming prompt) and no known
    // drives. Without a valid USB, no audio formats.
    selectUsb(overrides = {}) {
      const valid = overrides.valid ?? true;
      return {
        valid,
        normalizedRoot: "/Volumes/USB-TEST",
        canInitialize: false,
        statusText: "mock selection status",
        initHintText: "",
        warnings: [],
        audioFormats: valid ? backend.usbAudioFormats() : null,
        deviceName: null,
        devices: [],
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

    // `UsbAudioFormatsData` (select_usb's audioFormats, set_usb_audio_formats): every format allowed.
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
    answer(command, payload = {}) {
      const request = payload?.request ?? {};
      const stored = storeAnswer(command, request);
      if (stored !== undefined) return stored;
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
        case "get_frontend_settings": return ok({ values: {}, nodeAvailable: true, essentiaInstalled: true });
        case "set_frontend_setting": return ok({ saved: true });
        case "list_usb_devices": return ok({ items: [] });
        case "fetch_usb_playlists": return ok(backend.usbPlaylists());
        case "fetch_usb_histories": return ok(backend.usbHistories());
        case "fetch_usb_playlist_tracks":
        case "fetch_usb_history_tracks":
          return ok({ items: [], total: 0 });
        case "get_usb_player_menu_config": return ok(backend.playerMenuConfig());
        case "get_usb_background_color": return ok(backend.usbBackgroundColor());
        case "pick_usb_folder": return "/Volumes/USB-TEST";
        // The picked folder is an initialized stick; no folder, no USB.
        case "select_usb": {
          const path = String(request.path || "");
          return ok(path ? backend.selectUsb({ normalizedRoot: path }) : backend.selectUsb({ valid: false, normalizedRoot: null }));
        }
        case "run_usb_diagnostics": return ok(backend.usbDiagnostics());
        case "resolve_playback_source": return ok({ resolvedPath: null, matchedBy: "none", trackId: null });
        case "play_resolved_track": return ok({ playing: true, paused: false, positionMs: 0, durationMs: 0 });
        case "stop_playback_native":
        case "get_playback_status_native":
          return ok({});
        default:
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled: ${command}` } };
      }
    }
  };

  const withListSummary = (data) => ({ listSummaryText: "mock list summary", ...data });

  // A small PWV5 detail waveform (2 bytes per entry), base64.
  const pwv5 = new Uint8Array(4000);
  for (let i = 0; i < pwv5.length; i += 2) {
    const v = (2 << 13) | (3 << 10) | (5 << 7) | ((8 + (i % 20)) << 2);
    pwv5[i] = (v >> 8) & 0xff;
    pwv5[i + 1] = v & 0xff;
  }
  const detailWaveform = btoa(String.fromCharCode.apply(null, pwv5));
  const MAJORS = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
  // `TrackDetailData` (get_track_detail, get_usb_track_detail). `keyLabel`
  // renders a key in the notation the spec's settings pick.
  backend.trackDetail = (data, keyLabel = (k) => k) => ({
    firstBeatMs: null,
    cues: [],
    detailWaveform,
    keyOptions: [MAJORS, MAJORS.map((k) => `${k}m`)].map((keys, i) => ({
      label: i ? "Minor" : "Major", keys: keys.map((k) => ({ value: k, label: keyLabel(k) }))
    })),
    maxHotCues: 8,
    ...data
  });

  // The builders that fill out a spec's own answer to each command.
  const noBackups = { summaryText: "", emptyText: "", items: [] };
  const SHAPES = {
    get_library_sources: (data) => backend.librarySources([], data),
    fetch_usb_playlists: backend.usbPlaylists,
    fetch_usb_histories: backend.usbHistories,
    select_usb: backend.selectUsb,
    run_usb_diagnostics: backend.usbDiagnostics,
    repair_usb_diagnostics: (data) => backend.repair(
      data.diagnostics ? { ...data, diagnostics: backend.usbDiagnostics(data.diagnostics) } : data
    ),
    get_usb_player_menu_config: backend.playerMenuConfig,
    update_usb_player_menu_config: backend.playerMenuConfig,
    get_usb_background_color: backend.usbBackgroundColor,
    set_usb_background_color: backend.usbBackgroundColor,
    set_usb_audio_formats: backend.usbAudioFormats,
    export_to_usb: backend.exportToUsb,
    fetch_usb_playlist_tracks: withListSummary,
    fetch_usb_history_tracks: withListSummary,
    browse_source_files: withListSummary,
    get_track_detail: (data) => backend.trackDetail(data),
    get_usb_track_detail: (data) => backend.trackDetail(data),
    get_playlist_tracks: withListSummary,
    reorder_usb_playlists: (data) => ({ statusText: "mock reorder status", warningText: "", ...data }),
    remove_usb_playlist: (data) => ({ statusText: "mock remove status", warningText: "", ...data }),
    set_usb_device_name: (data) => ({ statusText: "mock rename status", updated: true, ...data }),
    restore_usb_backup: (data) => ({ statusText: "mock restore status", backups: noBackups, ...data }),
    delete_usb_backup: (data) => ({ statusText: "mock delete status", backups: noBackups, ...data }),
    prune_usb_device: (data) => ({ pruned: true, devices: [], ...data })
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
    "import_external_playlists", "export_to_usb", "set_usb_device_name", "select_usb",
    "restore_usb_backup", "set_export_sync_mode",
    "fetch_usb_playlists", "run_usb_diagnostics", "repair_usb_diagnostics"
  ]);

  // Answers given their playlists here, not by a spec, carry this mark (kept
  // through a spec's spread): a spec's invoke that passes the command on to the
  // one it replaced got them from that one's list_playlists, so they're taken
  // again from the spec's own.
  const FILLED = "__mockFilledPlaylists";

  async function withPlaylists(invoke, command, raw) {
    if (!PLAYLIST_CHANGES.has(command) || !raw?.ok || !raw.data || typeof raw.data !== "object") return raw;
    if ("playlists" in raw.data && !raw.data[FILLED]) return raw;
    const listed = await invoke("list_playlists", {});
    if (!listed?.ok) return raw;
    // A spec's own export status (its answer without playlists) stays: it is
    // what that spec tests. A re-fill takes the outer spec's list as a whole.
    const data = raw.data[FILLED]
      ? { ...raw.data, ...listed.data }
      : { ...listed.data, ...raw.data };
    return { ...raw, data: { ...data, [FILLED]: true } };
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
