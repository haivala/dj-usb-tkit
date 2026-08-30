// Browser/dev mock backend. Production API wiring lives in api_client.mjs.

import { playlistLocksReorderOnExport } from "./components/shared/export_reorder_lock.mjs";

// Mirrors the backend's PlaylistUsbExportStatus join (see
// backend/src/service/export.rs's compute_playlist_usb_export_status). The
// reorder-lock rule itself lives in one place (playlistLocksReorderOnExport in
// components/shared/export_reorder_lock.mjs) that both this mock and the live UI
// import, so e2e tests exercise the same logic the real backend computes.
function computeMockPlaylistUsbExportStatus(localPlaylists, usbPlaylistNames, pruneStale) {
  const normalize = (value) => String(value || "").trim().toLowerCase();
  const knownUsbNames = new Set((usbPlaylistNames || []).map(normalize));
  return (localPlaylists || []).map((playlist) => {
    const sameNameExistsOnUsb = knownUsbNames.has(normalize(playlist.name));
    return {
      playlistId: playlist.id,
      playlistName: playlist.name,
      sameNameExistsOnUsb,
      locksReorder: playlistLocksReorderOnExport(pruneStale, sameNameExistsOnUsb)
    };
  });
}

// Mirrors the backend's has_core_analysis_fields (backend/src/service/mod.rs):
// the mock stands in for the backend, which stamps analysisReady on every
// Track row it returns.
const mockAnalysisReady = (t) => !!t?.waveformPeaksPath
  && Number(t?.bpm || 0) > 0
  && Number(t?.durationMs || 0) > 0;
// Mirrors the backend's format_ext_from_path fallback (backend/src/utils.rs):
// every track-returning command guarantees formatExt on the wire.
const mockFormatExt = (t) => {
  if (t?.formatExt) return String(t.formatExt).toLowerCase();
  const m = String(t?.filePath || "").match(/\.([a-zA-Z0-9]+)$/);
  return m ? m[1].toLowerCase() : null;
};
const withDerivedFields = (items) => (items || []).map((t) => ({
  ...t,
  analysisReady: mockAnalysisReady(t),
  formatExt: mockFormatExt(t)
}));
const withAnalysisReady = withDerivedFields;

export function createMockInvoke({ state, normalizePath, constants }) {
  const { LIBRARY_LOAD_LIMIT_DEFAULT, LIBRARY_LOAD_LIMIT_POST_SCAN } = constants;

  async function invoke(command, payload = {}) {
    if (command === "scan_library") {
      return {
        ok: true,
        data: { jobId: "mock-scan", indexed: 3, updated: 0, removed: 0, notFound: [], warnings: [] }
      };
    }

    if (command === "check_source_roots") {
      const roots = Array.isArray(payload?.request?.sourceRoots)
        ? payload.request.sourceRoots
        : (Array.isArray(payload?.sourceRoots) ? payload.sourceRoots : []);
      return {
        ok: true,
        data: {
          items: roots.map((root) => ({ sourceRoot: root, exists: true, isDir: true })),
          missing: []
        }
      };
    }

    if (command === "relocate_source_root") {
      return {
        ok: true,
        data: {
          oldRoot: payload?.request?.oldRoot || "",
          newRoot: payload?.request?.newRoot || "",
          matched: 0,
          updated: 0,
          unchanged: 0,
          missingAtNewRoot: 0,
          conflicts: 0
        }
      };
    }

    if (command === "scan_master_db") {
      return {
        ok: true,
        data: { jobId: "mock-scan-master-db", indexed: 0, updated: 0, removed: 0, notFound: [] }
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
      const sourceRootAnalysis = roots.map((root) => {
        const rootKey = normalizePath(root).replace(/\/+$/, "");
        const rootItems = folderItems.filter((t) => {
          const fileKey = normalizePath(t.filePath);
          return fileKey === rootKey || fileKey.startsWith(`${rootKey}/`);
        });
        const analyzed = rootItems.filter((t) => t.waveformPeaksPath && Number(t.bpm || 0) > 0 && Number(t.durationMs || 0) > 0).length;
        return {
          sourceRoot: root,
          total: rootItems.length,
          analyzed,
          fullyAnalyzed: rootItems.length > 0 && analyzed === rootItems.length
        };
      });
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
          items: withAnalysisReady(items),
          nextCursor: nextOffset < filtered.length ? String(nextOffset) : null,
          hasMore: nextOffset < filtered.length,
          sourceRootAnalysis
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
          items: withAnalysisReady(pageItems),
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

    if (command === "materialize_source_track") {
      const req = payload?.request || {};
      const filePath = String(req.filePath || "").trim();
      if (!filePath) {
        return { ok: false, error: { code: "VALIDATION_ERROR", message: "filePath must not be empty" } };
      }
      const existing = state.tracks.find((track) => normalizePath(track.filePath || "") === normalizePath(filePath));
      if (existing?.id) {
        existing.title = req.title || existing.title || "";
        existing.artist = req.artist || existing.artist || "";
        existing.album = req.album ?? existing.album ?? null;
        existing.fileSizeBytes = req.fileSizeBytes ?? existing.fileSizeBytes ?? null;
        return { ok: true, data: { trackId: existing.id } };
      }
      const track = {
        id: `local-${Date.now()}`,
        title: req.title || "",
        artist: req.artist || "",
        album: req.album || null,
        trackNumber: req.trackNumber || null,
        bpm: null,
        key: req.key || null,
        filePath,
        fileSizeBytes: req.fileSizeBytes || null,
        formatExt: mockFormatExt({ formatExt: req.formatExt, filePath }),
        sampleRateHz: req.sampleRateHz || null,
        bitDepth: req.bitDepth || null,
        bitrateKbps: req.bitrateKbps || null,
        artworkPath: null,
        waveformPeaksPath: null,
        createdAt: new Date().toISOString(),
        updatedAt: new Date().toISOString()
      };
      state.tracks.push(track);
      return { ok: true, data: { trackId: track.id } };
    }

    if (command === "resolve_track_identity") {
      const req = payload?.request || {};
      const trackId = String(req.trackId || "").trim();
      const byId = state.tracks.find((track) => String(track.id || "") === trackId);
      if (byId?.id && normalizePath(byId.id) !== normalizePath(byId.filePath || "")) {
        return { ok: true, data: { trackId: byId.id, resolvedBy: "self", materialized: false } };
      }

      const filePath = String(req.filePath || "").trim();
      const usbRoot = String(req.usbRoot || "").trim();
      const normalizedPath = normalizePath(filePath);
      const normalizedRoot = normalizePath(usbRoot).replace(/\/+$/, "");
      const pathIsSelectedUsb = !!normalizedPath && !!normalizedRoot
        && (normalizedPath === normalizedRoot || normalizedPath.startsWith(`${normalizedRoot}/`));
      const usbMarked = !!String(req.usbAnalysisPath || "").trim() || pathIsSelectedUsb;

      if (filePath && !usbMarked) {
        const materialized = await invoke("materialize_source_track", { request: req });
        if (materialized?.ok && materialized.data?.trackId) {
          return {
            ok: true,
            data: {
              trackId: materialized.data.trackId,
              resolvedBy: "materialized",
              materialized: true
            }
          };
        }
      }

      const resolved = await invoke("resolve_playback_source", { request: req });
      if (!resolved?.ok) return resolved;
      return {
        ok: true,
        data: {
          trackId: resolved.data?.trackId || null,
          resolvedBy: resolved.data?.matchedBy || "none",
          materialized: false
        }
      };
    }

    if (command === "play_resolved_track") {
      const req = payload?.request || {};
      const resolved = await invoke("resolve_playback_source", { request: req });
      const isLibraryResolved = ["self", "hash", "metadata"].includes(resolved?.data?.matchedBy);
      const libraryPath = isLibraryResolved ? String(resolved?.data?.resolvedPath || "").trim() : "";
      const hasUsbContext = !!req.usbRoot && req.usbRootValid !== false;
      const trackPath = String(req.filePath || "").trim();
      const root = String(req.usbRoot || "").trim();
      const normalizedTrackPath = normalizePath(trackPath);
      const normalizedRoot = normalizePath(root).replace(/\/+$/, "");
      const usbPath = hasUsbContext && normalizedRoot
        && (normalizedTrackPath === normalizedRoot || normalizedTrackPath.startsWith(`${normalizedRoot}/`))
        ? trackPath
        : "";
      const playPath = libraryPath || usbPath;

      if (!playPath) {
        return {
          ok: false,
          error: {
            code: "NOT_FOUND",
            message: "track not found in Library or selected USB"
          }
        };
      }

      const played = await invoke("play_track_native", {
        request: {
          path: playPath,
          startOffsetMs: req.startOffsetMs,
          startRatio: req.startRatio
        }
      });
      if (!played?.ok) return played;

      const source = libraryPath ? "library" : "usb";
      const externalOrigin = ["usb", "history"].includes(String(req.origin || "").toLowerCase());
      const sourceLabel = source === "library"
        ? (externalOrigin ? "Library (matched)" : "Library")
        : (externalOrigin && hasUsbContext ? "USB" : "Local file");
      return {
        ok: true,
        data: {
          ...played.data,
          trackId: libraryPath ? (resolved?.data?.trackId || req.trackId || null) : (req.trackId || null),
          matchedBy: resolved?.data?.matchedBy || "none",
          source,
          sourceLabel,
          libraryResolved: !!libraryPath,
          hasUsbContext
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
      const req = payload?.request || {};
      const p = state.playlists.find((x) => x.id === req.playlistId);
      let rows = withDerivedFields(p?.tracks || []);
      const q = String(req.query || "").trim().toLowerCase();
      if (q) {
        rows = rows.filter((t) => `${t.title || ""} ${t.artist || ""} ${t.album || ""}`.toLowerCase().includes(q));
      }
      if (req.sortBy) {
        const dir = req.sortDir === "desc" ? -1 : 1;
        const val = (t) => req.sortBy === "bpm" || req.sortBy === "durationMs"
          ? Number(t[req.sortBy === "durationMs" ? "durationMs" : "bpm"] || 0)
          : String(t[req.sortBy === "format" ? "formatExt" : req.sortBy] || "").toLowerCase();
        rows = [...rows].sort((a, b) => (val(a) < val(b) ? -dir : val(a) > val(b) ? dir : 0));
      }
      const total = rows.length;
      const totalDurationMs = rows.reduce((sum, t) => sum + (Number(t.durationMs) > 0 ? Number(t.durationMs) : 0), 0);
      const durationKnownCount = rows.filter((t) => Number(t.durationMs) > 0).length;
      const unanalyzedCount = rows.filter((t) => !t.analysisReady).length;
      const offset = Number(req.cursor || 0);
      const limit = Number(req.limit || 0) || total;
      const page = rows.slice(offset, offset + limit);
      const nextOffset = offset + limit;
      return {
        ok: true,
        data: {
          playlistId: req.playlistId || "",
          items: page,
          total,
          hasMore: nextOffset < total,
          nextCursor: nextOffset < total ? String(nextOffset) : null,
          totalDurationMs,
          durationKnownCount,
          unanalyzedCount
        }
      };
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

    if (command === "add_track_candidates_to_playlist") {
      const req = payload?.request || {};
      const resolutions = (req.tracks || []).map((track) => {
        const localTrackId = String(track?.localTrackId || "").trim();
        const trackId = localTrackId || String(track?.trackId || track?.id || "").trim() || null;
        return {
          previousId: track?.trackId || track?.id || null,
          trackId,
          resolvedBy: localTrackId ? "localTrackId" : (trackId ? "self" : "none"),
          materialized: false
        };
      });
      const trackIds = resolutions.map((item) => item.trackId).filter(Boolean);
      return {
        ok: true,
        data: {
          playlistId: req.playlistId || "",
          requested: req.tracks?.length || 0,
          resolved: trackIds.length,
          unresolved: (req.tracks?.length || 0) - trackIds.length,
          added: trackIds.length,
          skipped: 0,
          resolutions
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

    if (command === "reorder_playlist_tracks") {
      const req = payload?.request || {};
      const playlistId = req.playlistId || "";
      const playlist = state.playlists.find((p) => String(p.id) === String(playlistId));
      const cur = playlist?.tracks || [];
      let ids;
      if (Array.isArray(req.orderedTrackIds) && req.orderedTrackIds.length) {
        ids = req.orderedTrackIds.map(String);
      } else if (req.sortBy) {
        const dir = req.sortDir === "desc" ? -1 : 1;
        const val = (t) => req.sortBy === "bpm" || req.sortBy === "durationMs"
          ? Number(t[req.sortBy] || 0)
          : String(t[req.sortBy === "format" ? "formatExt" : req.sortBy] || "").toLowerCase();
        ids = [...cur].sort((a, b) => (val(a) < val(b) ? -dir : val(a) > val(b) ? dir : 0)).map((t) => String(t.id));
      } else if (req.moveTrackId) {
        ids = cur.map((t) => String(t.id)).filter((id) => id !== String(req.moveTrackId));
        const at = req.beforeTrackId ? ids.indexOf(String(req.beforeTrackId)) : -1;
        ids.splice(at < 0 ? ids.length : at, 0, String(req.moveTrackId));
      } else {
        ids = cur.map((t) => String(t.id));
      }
      if (playlist?.tracks?.length) {
        const byId = new Map(cur.map((t) => [String(t.id), t]));
        playlist.tracks = ids.map((id) => byId.get(id)).filter(Boolean);
      }
      return { ok: true, data: { playlistId, reordered: ids.length } };
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

    if (command === "list_usb_devices") {
      return { ok: true, data: { items: state?.__mockUsbDevices || [] } };
    }

    if (command === "prune_usb_device") {
      const id = String(payload?.request?.id || payload?.id || "");
      if (Array.isArray(state?.__mockUsbDevices)) {
        state.__mockUsbDevices = state.__mockUsbDevices.filter((d) => d.id !== id);
      }
      return { ok: true, data: { pruned: !!id } };
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
                { title: "Track A", artist: "Artist 1", album: "Album X", bpm: 124, key: "8A", filePath: "/Contents/Artist 1/Track A.mp3", formatExt: "mp3" },
                { title: "Track D", artist: "Artist 4", album: "Album Z", bpm: 126, key: "10A", filePath: "/Contents/Artist 4/Track D.aiff", formatExt: "aiff" }
              ]
            }
          ],
          stats: {
            indexedTracks: 3,
            playlistReferencedTracks: 2,
            playlistEntries: 2
          },
          warnings: [],
          playlistUsbExportStatus: computeMockPlaylistUsbExportStatus(
            state.playlists,
            ["Warmup"],
            state.exportPruneStale
          )
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
                { title: "Track A", artist: "Artist 1", album: "Album X", bpm: 124, key: "8A", filePath: "/Contents/Artist 1/Track A.mp3", formatExt: "mp3" }
              ]
            }
          ],
          warnings: []
        }
      };
    }

    if (command === "fetch_usb_playlist_tracks" || command === "fetch_usb_history_tracks") {
      const req = payload?.request || {};
      const listCmd = command === "fetch_usb_playlist_tracks" ? "fetch_usb_playlists" : "fetch_usb_histories";
      const list = await invoke(listCmd, {});
      const src = (list?.data?.items || []).find((x) => x.id === req.id);
      if (!src) return { ok: false, error: { code: "NOT_FOUND", message: `not found: ${req.id}` } };
      let rows = withDerivedFields(src.tracks || []).map((t, i) => ({
        ...t,
        id: t.id || String(i),
        // "hydrated" page fields the list command omits
        waveformPreview: t.waveformPreview || [10, 40, 80, 30],
        artworkDataUrl: t.artworkDataUrl || null
      }));
      const q = String(req.query || "").trim().toLowerCase();
      if (q) rows = rows.filter((t) => `${t.title || ""} ${t.artist || ""} ${t.album || ""}`.toLowerCase().includes(q));
      if (req.sortBy) {
        const dir = req.sortDir === "desc" ? -1 : 1;
        const val = (t) => req.sortBy === "bpm" || req.sortBy === "durationMs"
          ? Number(t[req.sortBy] || 0)
          : String(t[req.sortBy === "format" ? "formatExt" : req.sortBy] || "").toLowerCase();
        rows = [...rows].sort((a, b) => (val(a) < val(b) ? -dir : val(a) > val(b) ? dir : 0));
      }
      const total = rows.length;
      const totalDurationMs = rows.reduce((s, t) => s + (Number(t.durationMs) > 0 ? Number(t.durationMs) : 0), 0);
      const durationKnownCount = rows.filter((t) => Number(t.durationMs) > 0).length;
      const offset = Number(req.cursor || 0);
      const limit = Number(req.limit || 0) || total;
      const page = rows.slice(offset, offset + limit);
      const nextOffset = offset + page.length;
      return {
        ok: true,
        data: {
          items: page,
          total,
          hasMore: nextOffset < total,
          nextCursor: nextOffset < total ? String(nextOffset) : null,
          totalDurationMs,
          durationKnownCount,
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
          durationMs: 42,
          playlistUsbExportStatus: computeMockPlaylistUsbExportStatus(
            state.playlists,
            ["Warmup"],
            state.exportPruneStale
          )
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
            filePath: "/Contents/Mock Artist/Mock USB Track.mp3",
            formatExt: "mp3",
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
          warnings: [],
          items: []
        }
      };
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

    if (command === "set_analysis_paused") {
      return {
        ok: true,
        data: { paused: !!payload?.request?.paused }
      };
    }

    if (command === "cancel_analysis") {
      return { ok: true, data: null };
    }

    return { ok: false, error: { code: "INTERNAL_ERROR", message: "Unknown mock command" } };
  }

  return invoke;
}
