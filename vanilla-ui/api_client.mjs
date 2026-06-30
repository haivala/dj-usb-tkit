// Backend API client: Tauri invoke wrapper, mock fallback, command helper.

export function createApiClient({ tauriInvoke, tauriIsTauri, tauriListen, state, normalizePath, constants }) {
  const { LIBRARY_LOAD_LIMIT_DEFAULT, LIBRARY_LOAD_LIMIT_POST_SCAN } = constants;

  function isTauriRuntime() {
    try {
      return !!tauriIsTauri();
    } catch (_) {
      return false;
    }
  }

  async function getTauriEventListen() {
    if (window.__TAURI__?.event?.listen) {
      return window.__TAURI__.event.listen;
    }
    return isTauriRuntime() ? tauriListen : null;
  }

  async function invoke(command, payload = {}) {
    if (isTauriRuntime()) {
      return tauriInvoke(command, payload);
    }
    if (window.__TAURI__?.core?.invoke) {
      return window.__TAURI__.core.invoke(command, payload);
    }

    // Mock fallback for browser-only UI testing.
    if (command === "scan_library") {
      return {
        ok: true,
        data: { jobId: "mock-scan", indexed: 3, updated: 0, removed: 0 }
      };
    }

    if (command === "scan_master_db") {
      return {
        ok: true,
        data: { jobId: "mock-scan-master-db", indexed: 0, updated: 0, removed: 0, notFound: [] }
      };
    }

    if (command === "get_system_parallelism") {
      const workers = Math.max(1, Number(window.navigator?.hardwareConcurrency) || 1);
      return {
        ok: true,
        data: { workers }
      };
    }

    if (command === "list_tracks") {
      const data = await invoke("search_tracks", {
        request: {
          query: "",
          limit: payload?.request?.limit ?? LIBRARY_LOAD_LIMIT_DEFAULT,
          cursor: payload?.request?.cursor ?? null
        }
      });
      return {
        ok: true,
        data: {
          total: data?.data?.total || 0,
          items: data?.data?.items || [],
          nextCursor: data?.data?.nextCursor || null,
          hasMore: !!data?.data?.hasMore
        }
      };
    }

    if (command === "browse_source_files") {
      const folderItems = [
        {
          id: "1",
          title: "Track A",
          artist: "Artist 1",
          album: "Album X",
          bpm: 124,
          key: "8A",
          filePath: "/music/Artist 1 - Track A.mp3",
          fileSizeBytes: 1000,
          artworkPath: null,
          waveformPeaksPath: null,
          createdAt: "2026-02-24T00:00:00Z",
          updatedAt: "2026-02-24T00:00:00Z"
        },
        {
          id: "2",
          title: "Track B",
          artist: "Artist 2",
          album: "Album Y",
          bpm: 128,
          key: "9A",
          filePath: "/music/Artist 2 - Track B.flac",
          fileSizeBytes: 2000,
          artworkPath: null,
          waveformPeaksPath: null,
          createdAt: "2026-02-24T00:00:00Z",
          updatedAt: "2026-02-24T00:00:00Z"
        },
        {
          id: "3",
          title: "Track C",
          artist: "Artist 1",
          album: "Album X",
          bpm: 121,
          key: "7A",
          filePath: "/music/Artist 1 - Track C.mp3",
          fileSizeBytes: 1500,
          artworkPath: null,
          waveformPeaksPath: null,
          createdAt: "2026-02-24T00:00:00Z",
          updatedAt: "2026-02-24T00:00:00Z"
        }
      ];
      const masterDbItems = [
        {
          id: "db-1",
          title: "Desktop Track",
          artist: "Desktop Artist",
          album: "Desktop Album",
          bpm: 126,
          key: "6A",
          filePath: "/library/Desktop Artist - Desktop Track.mp3",
          fileSizeBytes: 3000,
          artworkPath: null,
          waveformPeaksPath: null,
          masterDbSource: true,
          createdAt: "2026-02-24T00:00:00Z",
          updatedAt: "2026-02-24T00:00:00Z"
        }
      ];
      const roots = Array.isArray(payload?.sourceRoots) ? payload.sourceRoots : [];
      const includeMasterDb = payload?.includeMasterDb === true;
      const query = String(payload?.query || "").toLowerCase();
      const limit = Number(payload?.limit ?? LIBRARY_LOAD_LIMIT_DEFAULT) || LIBRARY_LOAD_LIMIT_DEFAULT;
      const cursor = String(payload?.cursor || "").trim();
      const offset = Number(cursor || 0) || 0;
      const byRoot = roots.length
        ? folderItems.filter((t) => roots.some((root) => normalizePath(t.filePath).startsWith(`${normalizePath(root).replace(/\/+$/, "")}/`) || normalizePath(t.filePath) === normalizePath(root)))
        : [];
      const scopedItems = includeMasterDb ? byRoot.concat(masterDbItems) : byRoot;
      const filtered = !query
        ? scopedItems
        : scopedItems.filter((t) => `${t.title} ${t.artist} ${t.album}`.toLowerCase().includes(query));
      const items = filtered.slice(offset, offset + limit);
      const nextOffset = offset + items.length;
      return {
        ok: true,
        data: {
          total: filtered.length,
          items,
          nextCursor: nextOffset < filtered.length ? String(nextOffset) : null,
          hasMore: nextOffset < filtered.length
        }
      };
    }

    if (command === "remove_tracks_by_source_roots") {
      return { ok: true, data: { removed: 0 } };
    }

    if (command === "allow_asset_paths") {
      const paths = Array.isArray(payload?.paths) ? payload.paths : [];
      return { ok: true, data: { allowed: paths.length } };
    }

    if (command === "search_tracks") {
      const items = [
        {
          id: "1",
          title: "Track A",
          artist: "Artist 1",
          album: "Album X",
          bpm: 124,
          key: "8A",
          filePath: "/music/Artist 1 - Track A.mp3",
          fileSizeBytes: 1000,
          artworkPath: null,
          waveformPeaksPath: null,
          createdAt: "2026-02-24T00:00:00Z",
          updatedAt: "2026-02-24T00:00:00Z"
        },
        {
          id: "2",
          title: "Track B",
          artist: "Artist 2",
          album: "Album Y",
          bpm: 128,
          key: "9A",
          filePath: "/music/Artist 2 - Track B.flac",
          fileSizeBytes: 2000,
          artworkPath: null,
          waveformPeaksPath: null,
          createdAt: "2026-02-24T00:00:00Z",
          updatedAt: "2026-02-24T00:00:00Z"
        },
        {
          id: "3",
          title: "Track C",
          artist: "Artist 1",
          album: "Album X",
          bpm: 121,
          key: "7A",
          filePath: "/music/Artist 1 - Track C.mp3",
          fileSizeBytes: 1500,
          artworkPath: null,
          waveformPeaksPath: null,
          createdAt: "2026-02-24T00:00:00Z",
          updatedAt: "2026-02-24T00:00:00Z"
        }
      ];
      const query = (payload?.request?.query || "").toLowerCase();
      const filtered = !query
        ? items
        : items.filter((t) => `${t.title} ${t.artist} ${t.album}`.toLowerCase().includes(query));
      const limit = Number(payload?.request?.limit ?? filtered.length) || filtered.length;
      const cursor = String(payload?.request?.cursor || "").trim();
      const offset = Number(cursor || 0) || 0;
      const pageItems = filtered.slice(offset, offset + limit);
      const nextOffset = offset + pageItems.length;
      return {
        ok: true,
        data: {
          total: filtered.length,
          items: pageItems,
          nextCursor: nextOffset < filtered.length ? String(nextOffset) : null,
          hasMore: nextOffset < filtered.length
        }
      };
    }

    if (command === "get_tracks_by_ids_with_previews") {
      const ids = Array.isArray(payload?.request?.trackIds)
        ? payload.request.trackIds.map((id) => String(id))
        : [];
      const data = await invoke("search_tracks", {
        request: {
          query: "",
          limit: Math.max(Number(state.libraryLoadedTotal || 0), LIBRARY_LOAD_LIMIT_POST_SCAN),
          cursor: null
        }
      });
      const items = (data?.data?.items || []).filter((t) => ids.includes(String(t.id)));
      return { ok: true, data: { items } };
    }

    if (command === "resolve_playback_source") {
      const title = String(payload?.request?.title || "").trim().toLowerCase();
      const artist = String(payload?.request?.artist || "").trim().toLowerCase();
      const found = state.tracks.find((t) =>
        String(t.title || "").trim().toLowerCase() === title &&
        String(t.artist || "").trim().toLowerCase() === artist &&
        !!t.filePath
      );
      return {
        ok: true,
        data: {
          resolvedPath: found?.filePath || null,
          matchedBy: found ? "hash" : "none",
          trackId: found?.id || null
        }
      };
    }

    if (command === "create_playlist") {
      const item = {
        id: `playlist-${Date.now()}`,
        name: payload?.request?.name || "Playlist",
        source: "local",
        lastExportedAt: null,
        lastExportedUsbRoot: null,
        lastExportedTrackCount: null,
        createdAt: new Date().toISOString(),
        updatedAt: new Date().toISOString(),
        tracks: []
      };
      state.playlists.push(item);
      return {
        ok: true,
        data: {
          playlistId: item.id,
          name: item.name
        }
      };
    }

    if (command === "delete_playlist") {
      const playlistId = payload?.request?.playlistId || "";
      const before = state.playlists.length;
      state.playlists = state.playlists.filter((p) => String(p.id) !== String(playlistId));
      return { ok: true, data: { playlistId, deleted: state.playlists.length < before } };
    }

    if (command === "list_playlists") {
      return { ok: true, data: { items: state.playlists } };
    }

    if (command === "get_playlist_tracks") {
      const p = state.playlists.find((x) => x.id === payload?.request?.playlistId);
      return { ok: true, data: { playlistId: payload?.request?.playlistId || "", items: p?.tracks || [] } };
    }

    if (command === "add_tracks_to_playlist") {
      return {
        ok: true,
        data: {
          playlistId: payload?.request?.playlistId || "",
          added: payload?.request?.trackIds?.length || 0,
          skipped: 0
        }
      };
    }

    if (command === "remove_tracks_from_playlist") {
      const playlistId = payload?.request?.playlistId || "";
      const ids = new Set(payload?.request?.trackIds || []);
      const playlist = state.playlists.find((p) => String(p.id) === String(playlistId));
      let removed = 0;
      if (playlist?.tracks?.length) {
        const before = playlist.tracks.length;
        playlist.tracks = playlist.tracks.filter((t) => !ids.has(t.id));
        removed = before - playlist.tracks.length;
      }
      return {
        ok: true,
        data: {
          playlistId,
          removed
        }
      };
    }

    if (command === "validate_usb_root") {
      const requested = String(payload?.request?.path || "");
      const valid = !!requested;
      return {
        ok: true,
        data: {
          valid,
          hasWriteAccess: valid,
          normalizedRoot: valid ? requested : null,
          hasVendorRoot: valid,
          hasContents: valid,
          hasPdb: valid,
          hasEdb: valid,
          warnings: valid ? [] : ["USB path is empty"]
        }
      };
    }

    if (command === "fetch_usb_playlists") {
      return {
        ok: true,
        data: {
          items: [
            {
              id: "usb-1",
              name: "Warmup",
              source: "mock",
              trackCount: 2,
              tracks: [
                { title: "Track A", artist: "Artist 1", album: "Album X", bpm: 124, key: "8A" },
                { title: "Track D", artist: "Artist 4", album: "Album Z", bpm: 126, key: "10A" }
              ]
            }
          ],
          stats: {
            indexedTracks: 3,
            playlistReferencedTracks: 2,
            playlistEntries: 2
          },
          warnings: []
        }
      };
    }

    if (command === "fetch_usb_histories") {
      return {
        ok: true,
        data: {
          items: [
            {
              id: "h1",
              name: "History 2026-02-20",
              createdAt: "2026-02-20 22:10",
              tracks: [
                { title: "Track A", artist: "Artist 1", album: "Album X", bpm: 124, key: "8A" }
              ]
            }
          ],
          warnings: []
        }
      };
    }

    if (command === "remove_usb_playlist") {
      return {
        ok: true,
        data: {
          playlistName: payload?.request?.playlistName || "",
          removedFromEdb: 1,
          removedFromPdb: 1,
          warnings: []
        }
      };
    }

    if (command === "run_usb_diagnostics") {
      const makeCheck = (label, status, detail) => ({ label, status, detail });
      return {
        ok: true,
        data: {
          overallStatus: "PASS",
          pdbIntegrity: {
            title: "PDB Integrity", status: "PASS",
            checks: [
              makeCheck("PDB exists", "PASS", "Found"),
              makeCheck("PDB parseable", "PASS", "3 tracks, 2 artists, 1 albums, 1 keys, 0 artworks"),
              makeCheck("Playlists", "PASS", "2 tree nodes, 3 entries"),
              makeCheck("num_rl=8191 pages", "PASS", "0 of 5 pages"),
              makeCheck("nrs wrapping", "PASS", "0 pages with row count exceeding nrs header"),
              makeCheck("Orphaned entries", "PASS", "0 entries reference 0 track IDs not in PDB")
            ]
          },
          edbAccess: {
            title: "Database Access", status: "PASS",
            checks: [
              makeCheck("eDB", "PASS", "Unlocked with default USB export key"),
              makeCheck("master.db", "PASS", "Skipped (not requested)")
            ]
          },
          contentsIntegrity: {
            title: "Contents Integrity", status: "PASS",
            checks: [
              makeCheck("Contents files", "PASS", "3 audio files on USB"),
              makeCheck("Indexed tracks", "PASS", "3 tracks in PDB"),
              makeCheck("Count match", "PASS", "Exact match")
            ]
          },
          analysisIntegrity: {
            title: "Analysis Files", status: "PASS",
            checks: [
              makeCheck("Analysis files", "PASS", "6 files in USBANLZ"),
              makeCheck("Track analysis refs", "PASS", "3/3 tracks have valid analysis paths")
            ]
          },
          playlistResolution: {
            title: "Playlist Resolution", status: "PASS",
            checks: [
              makeCheck("Overall resolution", "PASS", "3/3 entries resolve (100.0%) across 1 playlists"),
              makeCheck("PDB vs eDB key overlap (informational)", "PASS", "matched 3 track keys; PDB 100.0% (3/3), DB 100.0% (3/3)")
            ]
          },
          playlistDetails: [
            {
              name: "Warmup",
              totalEntries: 3,
              resolvedEntries: 3,
              resolutionRate: 1.0,
              status: "PASS",
              pedbEntries: 3,
              edbEntries: 3,
              matchedEntries: 3,
              pedbMatchRate: 1.0,
              edbMatchRate: 1.0
            }
          ],
          warnings: [
            { level: "info", code: "usb.diagnostics.info", message: "USB root: /media/usb", source: "usb-diagnostics" }
          ],
          durationMs: 42
        }
      };
    }

    if (command === "run_usb_parity_report") {
      const makeCheck = (label, status, detail) => ({ label, status, detail });
      const makeSummaryRow = (label, status, count) => ({ label, status, count });
      return {
        ok: true,
        data: {
          overallStatus: "FAIL",
          checks: [
            makeCheck("Overall player parity status", "FAIL", "playlists checked: 1, fail: 1"),
            makeCheck("Parity-report section (required)", "FAIL", "See parity summary rows for category counts."),
            makeCheck("Playlist identity parity", "PASS", "all compared playlists matched by identity"),
            makeCheck("Playlist membership parity", "PASS", "membership only-in-PDB=0, membership only-in-eDB=0"),
            makeCheck("Playlist ordering parity", "PASS", "order mismatches=0"),
            makeCheck("Duplicate PDB entries", "PASS", "0 duplicate PDB playlist entry/entries detected"),
            makeCheck("PDB metadata completeness", "FAIL", "1 playlist-linked PDB track(s) are missing required player metadata"),
            makeCheck("Media and analysis path parity", "FAIL", "1 playlist-linked track(s) have media/analysis path mismatches"),
            makeCheck("Artwork presence parity", "WARN", "1 playlist-linked track(s) have artwork in one DB but not the other"),
            makeCheck("PDB dictionary id resolution", "FAIL", "1 playlist-linked track(s) have unresolved required PDB dictionary ids"),
            makeCheck("eDB source completeness", "PASS", "0 playlist-linked eDB track(s) are missing metadata used by strict parity comparison")
          ],
          summaryRows: [
            makeSummaryRow("Failing playlists", "FAIL", 1),
            makeSummaryRow("Membership only-in-PDB", "PASS", 0),
            makeSummaryRow("Membership only-in-eDB", "PASS", 0),
            makeSummaryRow("Order mismatches", "PASS", 0),
            makeSummaryRow("Duplicate PDB entries", "PASS", 0),
            makeSummaryRow("PDB metadata gaps", "FAIL", 1),
            makeSummaryRow("eDB source gaps", "PASS", 0),
            makeSummaryRow("Path mismatches", "FAIL", 1),
            makeSummaryRow("Artwork presence mismatches", "WARN", 1),
            makeSummaryRow("Unresolved PDB dictionary ids", "FAIL", 1)
          ],
          playlistDetails: [
            {
              name: "Warmup",
              pedbTracks: 3,
              edbTracks: 3,
              matchedTracks: 3,
              onlyInPdb: 0,
              onlyInEdb: 0,
              orderMismatch: false,
              pdbDuplicateEntries: 0,
              pdbMissingCoreMetadata: 1,
              edbMissingCoreMetadata: 0,
              artworkMismatchTracks: 1,
              pathMismatchTracks: 1,
              dictionaryIdIssueTracks: 1,
              playlistIdMatch: true,
              sortOrderMatch: true,
              status: "FAIL"
            }
          ],
          warnings: [
            { level: "info", code: "usb.diagnostics.info", message: "USB root: /media/usb", source: "usb-diagnostics" }
          ],
          durationMs: 21
        }
      };
    }

    if (command === "repair_usb_diagnostics") {
      const apply = !!payload?.request?.apply;
      return {
        ok: true,
        data: {
          detectedIssues: [
            "1 empty USB analysis file(s) detected",
            "eDB appears to be a subset of PDB"
          ],
          proposedFixes: [
            {
              id: "fix_empty_analysis_files",
              title: "Fix Empty Analysis Files",
              description: "Regenerate missing/empty DAT/EXT/2EX bundles when source audio is resolvable.",
              supported: false,
              destructive: false,
              estimatedWrites: 3,
              estimatedDeletes: 0
            },
            {
              id: "parity_repair_exportlibrary_sync",
              title: "Parity Repair (Pro coverage aware)",
              description: "Preview and optionally sync missing eDB playlist membership/order from PDB static playlists.",
              supported: false,
              destructive: false,
              estimatedWrites: 0,
              estimatedDeletes: 0
            }
          ],
          unsupportedItems: [
            { issue: "Parity Repair", reason: "Preview is implemented; apply step is not implemented yet." }
          ],
          appliedFixes: apply ? ["Fix Empty Analysis Files: fixed 1, skipped 0"] : [],
          skippedFixes: apply ? ["Parity Repair: not supported yet", "Playlist Recovery From USB: not supported yet"] : [],
          failedFixes: [],
          estimatedFileWrites: 3,
          estimatedFileDeletes: 0,
          warnings: ["USB root: /media/usb"],
          durationMs: 25
        }
      };
    }

    if (command === "export_to_usb") {
      return {
        ok: false,
        error: {
          code: "PRECONDITION_FAILED",
          message: "export_to_usb is available only in the Tauri runtime."
        }
      };
    }

    if (command === "play_track_native") {
      const now = Date.now();
      const durationMs = state.mockPlayback.durationMs || 240000;
      const ratio = Number(payload?.request?.startRatio ?? 0);
      const explicitOffset = Number(payload?.request?.startOffsetMs ?? 0);
      const offset = Number.isFinite(explicitOffset) && explicitOffset > 0
        ? explicitOffset
        : Math.max(0, Math.min(durationMs, Math.round(durationMs * Math.max(0, Math.min(1, ratio)))));
      state.mockPlayback = {
        ...state.mockPlayback,
        path: payload?.request?.path || "",
        playing: true,
        startedAtMs: now,
        startOffsetMs: offset
      };
      return {
        ok: true,
        data: {
          path: payload?.request?.path || "",
          playing: true,
          positionMs: offset,
          durationMs
        }
      };
    }

    if (command === "stop_playback_native") {
      const previousPath = state.mockPlayback.path;
      state.mockPlayback = {
        ...state.mockPlayback,
        path: null,
        playing: false,
        startedAtMs: 0,
        startOffsetMs: 0
      };
      return {
        ok: true,
        data: { stopped: true, previousPath }
      };
    }

    if (command === "get_playback_status_native") {
      const now = Date.now();
      let positionMs = state.mockPlayback.startOffsetMs;
      if (state.mockPlayback.playing) {
        const elapsed = Math.max(0, now - state.mockPlayback.startedAtMs);
        positionMs += elapsed;
        if (positionMs >= state.mockPlayback.durationMs) {
          positionMs = state.mockPlayback.durationMs;
          state.mockPlayback.playing = false;
        }
      }
      return {
        ok: true,
        data: {
          path: state.mockPlayback.path,
          playing: state.mockPlayback.playing,
          positionMs,
          durationMs: state.mockPlayback.durationMs
        }
      };
    }

    if (command === "playback_preflight_native") {
      return {
        ok: true,
        data: {
          path: payload?.request?.path || "",
          fileExists: true,
          fileReadable: true,
          safeOutputDevices: ["pipewire"],
          ready: true,
          message: "Ready"
        }
      };
    }

    if (command === "inspect_usb_track") {
      return {
        ok: true,
        data: {
          source: "mock",
          track: {
            id: payload?.request?.trackId || "0",
            title: "Mock USB Track",
            artist: "Mock Artist",
            album: "Mock Album",
            bpm: 128,
            key: "8A",
            filePath: "",
            artworkPath: null,
            artworkDataUrl: null,
            waveformPeaksPath: null,
            usbAnalysisPath: null,
            waveformPreview: [10, 40, 80, 30, 60]
          },
          warnings: []
        }
      };
    }

    if (command === "analyze_new_tracks") {
      return {
        ok: true,
        data: {
          jobId: "job-analysis-mock",
          analyzed: 0,
          failed: 0,
          warnings: []
        }
      };
    }

    if (command === "analyze_track_piece") {
      const trackId = String(payload?.request?.trackId || "");
      const piece = String(payload?.request?.piece || "");
      const response = {
        trackId,
        piece,
        updated: true,
        bpm: null,
        key: null,
        durationMs: null,
        artworkPath: null,
        waveformPeaksPath: null,
        waveformPreview: null
      };
      if (piece === "duration") response.durationMs = 180000;
      if (piece === "artwork") response.artworkPath = `/tmp/${trackId}.jpg`;
      if (piece === "waveform") {
        response.waveformPeaksPath = `/tmp/${trackId}.DAT`;
        response.waveformPreview = [8, 20, 42, 65, 30, 55];
      }
      if (piece === "bpm_key") {
        response.bpm = 124;
        response.key = "8A";
      }
      return { ok: true, data: response };
    }

    if (command === "detect_external_master_db") {
      return {
        ok: true,
        data: {
          found: false,
          path: null
        }
      };
    }

    if (command === "initialize_usb") {
      return {
        ok: true,
        data: {
          path: payload?.usbRoot || "",
          createdDirs: ["vendor-db", "Contents"]
        }
      };
    }

    if (command === "clear_frontend_log") {
      return "";
    }

    if (command === "append_frontend_log") {
      return null;
    }

    if (command === "get_backend_log_buffer") {
      return [];
    }

    if (command === "pick_source_folders") {
      return [];
    }

    if (command === "pick_usb_folder") {
      return null;
    }

    if (command === "get_frontend_settings") {
      return {
        ok: true,
        data: { values: {} }
      };
    }

    if (command === "set_frontend_setting") {
      return {
        ok: true,
        data: { saved: true }
      };
    }

    return { ok: false, error: { code: "INTERNAL_ERROR", message: "Unknown mock command" } };
  }

  async function command(commandName, request = null) {
    const payload = request === null ? {} : { request };
    const response = await invoke(commandName, payload);

    if (!response?.ok) {
      const msg = response?.error?.message || `Command failed: ${commandName}`;
      throw new Error(msg);
    }

    return response.data;
  }

  return { invoke, command, isTauriRuntime, getTauriEventListen };
}
