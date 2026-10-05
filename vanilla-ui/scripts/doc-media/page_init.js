// Page init script for record.mjs: stands in for the Tauri backend with the
// payloads the real backend produced (window.__DOC_GIF_FIXTURE__) plus
// made-up playlists and a connected USB, and draws a cursor and a Shift
// keycap, which a headless screencast doesn't show. Each library named in
// `opts.externalLibraries` (`rekordbox`, `mixxx`) is "detected" as imported,
// and its made-up playlists can be imported.
//
// The USB views answer with what the real backend returned for the demo
// library exported to a USB (`fixture.usb`); `opts.brokenUsb` swaps in the
// damaged copy's diagnostics. History sessions, which only a player writes,
// are made from those USB tracks (`fixture.usbHistory`). Exporting replays
// the real export's progress messages as job events; `opts.unexported`
// playlists show as not exported until then. `opts.tauriRuntime` makes the
// page pass for the Tauri runtime (as tests/e2e/smoke.spec.mjs does), which
// the app needs to follow job events and to show its version
// (`opts.appVersion`).
(() => {
  const { tracks, detail, sourceRoots, sourceRootEnabled, playlists, usb, usbHistory } = window.__DOC_GIF_FIXTURE__;
  const opts = window.__DOC_GIF_OPTS__ || {};
  const USB_ROOT = "/run/media/dj/Chiphead";
  const ls = window.localStorage;
  ls.setItem("djusbtkit.helpSeen", "1");
  ls.setItem("djusbtkit.theme", "dark");
  ls.setItem("djusbtkit.accentHue", "270");
  ls.setItem("djusbtkit.sourceRoots", JSON.stringify(sourceRoots));
  ls.setItem("djusbtkit.sourceRootEnabled", JSON.stringify(sourceRootEnabled));
  ls.setItem("djusbtkit.cueStartOnFirstBeat", "0");
  ls.setItem("djusbtkit.cueQuantize", "1");
  // An imported Mixxx library is switched on in the Libraries row.
  if (opts.externalLibraries?.mixxx) ls.setItem("djusbtkit.mixxxDbEnabled", "1");

  const durationMs = detail.track.durationMs;
  const clock = { offsetMs: 0, startedAt: null, loaded: false };
  const positionMs = () =>
    Math.min(durationMs, clock.offsetMs + (clock.startedAt == null ? 0 : Date.now() - clock.startedAt));
  const status = () => ({
    path: detail.track.filePath,
    playing: clock.loaded && clock.startedAt != null,
    paused: clock.loaded && clock.startedAt == null,
    positionMs: positionMs(),
    durationMs,
  });
  const ok = (data) => ({ ok: true, data });
  const sumMs = (items) => items.reduce((ms, t) => ms + (Number(t.durationMs) || 0), 0);
  const exportedAt = "2026-09-26T18:00:00Z";
  const external = opts.externalLibraries || null;
  const byTitles = (titles) => titles.map((title) => tracks.find((t) => t.title === title)).filter(Boolean);
  // Imported playlists join the sidebar like any local playlist.
  const importPlaylist = (source, r) => {
    const item = external[source].find((p) => p.id === r.id && p.kind === r.kind);
    const id = `pl-import-${source}-${item.id}`;
    const items = byTitles(item.titles);
    playlists.push({ id, name: item.name, tracks: items, imported: true });
    return ok({ playlistId: id, name: item.name, added: items.length, indexed: 0, notFound: [], warnings: [] });
  };
  // USB track rows by title, from every exported playlist.
  const usbTracksByTitle = new Map(
    Object.values(usb?.playlistTracks || {}).flatMap((resp) => resp.data.items.map((t) => [t.title, t]))
  );
  const usbTracksPage = (items) => ok({
    ...Object.values(usb.playlistTracks)[0].data,
    items,
    total: items.length,
    hasMore: false,
    nextCursor: null,
    totalDurationMs: sumMs(items),
    durationKnownCount: items.length,
  });
  const historySessions = (usbHistory || []).map(({ id, name, createdAt, titles }) => {
    const items = titles.map((title) => usbTracksByTitle.get(title)).filter(Boolean);
    return { id, name, createdAt, items };
  });
  const listeners = new Map();
  const emitEvent = (name, payload) => {
    for (const cb of listeners.get(name) || []) cb({ event: name, payload });
  };
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const exportedNow = new Set();
  const isExported = (id, items, imported) =>
    exportedNow.has(id) || (!!items.length && !imported && !(opts.unexported || []).includes(id));
  // Replays the recorded export of playlist `name` as the backend's job
  // events, over about `opts.exportMs`.
  const replayExport = async (playlistId) => {
    const name = playlists.find((p) => p.id === playlistId)?.name;
    const run = usb.exports[name];
    const job = { jobId: "job-doc-export", jobType: "export", stage: "export_to_usb" };
    const emit = (event, current, total, message) => emitEvent("job:event", {
      event, ...job, current, total, percent: Math.min(100, Math.floor((current * 100) / Math.max(1, total))),
      message, timestamp: new Date().toISOString(),
    });
    emit("job.started", 0, 1, "USB: Exporting playlist");
    const stepMs = (opts.exportMs || 3000) / Math.max(1, run.progress.length);
    for (const [current, total, message] of run.progress) {
      await sleep(stepMs);
      emit("job.progress", current, Math.max(1, total), message || "USB: Exporting playlist");
    }
    exportedNow.add(playlistId);
    emit("job.completed", 1, 1, "USB: Export complete");
    return run.response;
  };

  const listExternal = (source) =>
    ok({ items: external[source].map(({ id, name, kind, titles }) => ({ id, name, kind, trackCount: titles.length })) });

  // The sidebar lists playlists newest first, i.e. reversed.
  const playlistRows = () => [...playlists].reverse().map(({ id, name, tracks: items, imported }) => ({
    id,
    name,
    source: "app",
    lastExportedAt: isExported(id, items, imported) ? exportedAt : null,
    lastExportedUsbRoot: isExported(id, items, imported) ? USB_ROOT : null,
    lastExportedTrackCount: isExported(id, items, imported) ? items.length : null,
    trackCount: items.length,
    totalDurationMs: sumMs(items),
    createdAt: exportedAt,
    updatedAt: exportedAt,
  }));

  if (opts.tauriRuntime) {
    window.isTauri = true;
    window.__TAURI_INTERNALS__ = {
      invoke: async (cmd, args) =>
        cmd === "plugin:app|version" ? opts.appVersion : window.__TAURI__.core.invoke(cmd, args),
      convertFileSrc: (path) => path,
    };
  }

  window.__TAURI__ = {
    core: {
      invoke: async (command, payload = {}) => {
        const r = payload?.request || {};
        switch (command) {
          case "clear_frontend_log":
            return "";
          case "append_frontend_log":
          case "show_window":
            return null;
          case "get_backend_log_buffer":
            return [];
          case "set_frontend_setting":
            return ok(null);
          case "detect_external_master_db":
            return ok(external?.rekordbox
              ? { found: true, path: "C:/Users/dj/AppData/Roaming/Pioneer/rekordbox/master.db", imported: true }
              : { found: false, path: null, imported: false });
          case "detect_external_mixxx_db":
            return ok(external?.mixxx
              ? { found: true, path: "/home/dj/.mixxx/mixxxdb.sqlite", imported: true }
              : { found: false, path: null, imported: false });
          case "list_rekordbox_playlists":
            return listExternal("rekordbox");
          case "list_mixxx_playlists":
            return listExternal("mixxx");
          case "import_rekordbox_playlist":
            return importPlaylist("rekordbox", r);
          case "import_mixxx_playlist":
            return importPlaylist("mixxx", r);
          case "list_playlists":
            return ok({ items: playlistRows() });
          case "get_playlist_tracks": {
            const items = playlists.find((p) => p.id === r.playlistId)?.tracks || [];
            return ok({
              playlistId: r.playlistId,
              items,
              total: items.length,
              hasMore: false,
              totalDurationMs: sumMs(items),
              durationKnownCount: items.length,
            });
          }
          case "check_for_update":
            return ok({
              updateAvailable: false, severity: "none", currentVersion: opts.appVersion, latestVersion: opts.appVersion,
              releaseUrl: "", installKind: "appimage", downloadUrl: null, canSelfUpdate: false, action: "none",
              checkFailed: false,
            });
          case "get_frontend_settings":
            return ok({ values: {}, nodeAvailable: false, essentiaInstalled: false });
          case "get_source_root_analysis":
            return ok({
              items: (r.sourceRoots || sourceRoots).map((sourceRoot) => {
                const total = tracks.filter((t) => t.filePath.startsWith(`${sourceRoot}/`)).length;
                return { sourceRoot, total, analyzed: total, fullyAnalyzed: true };
              }),
            });
          case "set_theme_background":
            return null;
          case "allow_asset_paths":
            return 0;
          case "check_source_roots":
            return ok({ missing: [] });
          case "list_usb_backups":
            return ok({ items: opts.backups || [] });
          case "list_usb_devices":
            return ok({ items: [{ id: "usb-chiphead", rootPath: USB_ROOT }] });
          case "validate_usb_root":
            return ok({
              valid: true,
              normalizedRoot: USB_ROOT,
              hasVendorRoot: true,
              hasContents: true,
              hasPdb: true,
              hasWriteAccess: true,
              warnings: [],
            });
          case "get_usb_device_name":
            return ok({ name: "Chiphead" });
          case "run_usb_diagnostics":
            if (!usb) return ok({ overallStatus: "PASS", durationMs: 842, warnings: [], checks: [], playlistUsbExportStatus: [] });
            return opts.brokenUsb ? usb.brokenDiagnostics : usb.diagnostics;
          case "repair_usb_diagnostics":
            return usb.repairPreview;
          case "fetch_usb_playlists":
            return usb ? usb.playlists : ok({ items: [], warnings: [] });
          case "fetch_usb_playlist_tracks":
            return usb.playlistTracks[r.id];
          case "fetch_usb_histories":
            if (!usb) return ok({ items: [], warnings: [] });
            return ok({
              ...usb.histories.data,
              items: historySessions.map(({ id, name, createdAt, items }) => ({
                id, name, createdAt, tracks: items, totalDurationMs: sumMs(items), durationKnownCount: items.length,
              })),
              counts: {
                ...usb.histories.data.counts,
                importedPlaylists: historySessions.length,
                importedTracks: historySessions.reduce((n, h) => n + h.items.length, 0),
              },
            });
          case "fetch_usb_history_tracks":
            return usbTracksPage(historySessions.find((h) => h.id === r.id)?.items || []);
          case "get_usb_player_menu_config":
            return usb.playerMenu;
          case "export_to_usb":
            return replayExport(r.playlistId);
          case "list_tracks":
          case "search_tracks":
            return ok({ total: tracks.length, items: tracks });
          case "browse_source_files": {
            // Only the checked folders' tracks, as the backend filters them.
            const roots = r.sourceRoots || sourceRoots;
            const items = tracks.filter((t) => roots.some((root) => t.filePath.startsWith(`${root}/`)));
            return ok({
              total: items.length,
              items,
              nextCursor: null,
              hasMore: false,
              sourceRootAnalysis: roots.map((sourceRoot) => ({ sourceRoot, fullyAnalyzed: true })),
              totalDurationMs: sumMs(items),
              durationKnownCount: items.length,
            });
          }
          case "get_tracks_by_ids_with_previews":
            return ok({ items: tracks.filter((t) => (r.trackIds || []).includes(t.id)) });
          case "resolve_track_identity":
            return ok({ trackId: r.trackId || detail.track.id, resolvedBy: "self", materialized: false });
          case "get_track_detail":
            return ok({
              ...detail,
              cues: opts.cues ?? detail.cues,
              firstBeatMs: opts.firstBeatMs ?? detail.firstBeatMs,
              track: { ...detail.track, bpm: opts.bpm ?? detail.track.bpm },
            });
          case "play_resolved_track":
            clock.offsetMs = Math.round((r.startRatio || 0) * durationMs);
            clock.startedAt = Date.now();
            clock.loaded = true;
            return ok({ started: true, positionMs: clock.offsetMs, durationMs });
          case "pause_playback_native":
            if (clock.loaded && clock.startedAt != null) {
              clock.offsetMs = positionMs();
              clock.startedAt = null;
            }
            return ok(status());
          case "resume_playback_native":
            if (clock.loaded && clock.startedAt == null) clock.startedAt = Date.now();
            return ok(status());
          case "stop_playback_native":
            clock.loaded = false;
            clock.startedAt = null;
            return ok({ stopped: true, previousPath: null });
          case "get_playback_status_native":
            return ok(status());
          case "set_playback_metronome":
            return ok({ enabled: !!r.enabled && r.bpm > 0 });
          default:
            console.warn(`doc-media: not stubbed: ${command}`);
            return { ok: false, error: { code: "UNKNOWN", message: `Not stubbed: ${command}` } };
        }
      },
    },
    event: {
      listen: async (name, cb) => {
        listeners.set(name, [...(listeners.get(name) || []), cb]);
        return () => listeners.set(name, (listeners.get(name) || []).filter((fn) => fn !== cb));
      },
    },
  };

  const installOverlay = () => {
    const style = document.createElement("style");
    style.textContent = `
#docGifCursor{position:fixed;left:-50px;top:-50px;width:22px;height:22px;pointer-events:none;z-index:2147483647;transform:translate(-3px,-2px)}
#docGifCursor svg{filter:drop-shadow(0 1px 2px rgba(0,0,0,.6))}
#docGifCursor.down::after{content:"";position:absolute;left:-7px;top:-7px;width:18px;height:18px;border-radius:50%;background:rgba(255,255,255,.35)}
#docGifKey{position:fixed;z-index:2147483647;pointer-events:none;transform:translateX(-50%);font:600 15px system-ui,sans-serif;color:#fff;background:rgba(20,20,30,.85);border:1px solid rgba(255,255,255,.35);border-bottom-width:3px;border-radius:8px;padding:6px 12px;display:none}
#docGifKey.on{display:block}`;
    document.head.appendChild(style);
    const cursor = document.createElement("div");
    cursor.id = "docGifCursor";
    cursor.innerHTML =
      '<svg width="22" height="22" viewBox="0 0 22 22"><path d="M3 2 L3 18 L7.5 14 L10.5 20.5 L13 19.3 L10 13 L16 13 Z" fill="#fff" stroke="#000" stroke-width="1.2" stroke-linejoin="round"/></svg>';
    const key = document.createElement("div");
    key.id = "docGifKey";
    document.body.append(cursor, key);
    const follow = (e) => {
      cursor.style.left = `${e.clientX}px`;
      cursor.style.top = `${e.clientY}px`;
    };
    addEventListener("mousemove", follow, true);
    addEventListener("pointermove", follow, true);
    addEventListener("mousedown", () => cursor.classList.add("down"), true);
    addEventListener("mouseup", () => cursor.classList.remove("down"), true);
    // Shows `label` centred at (x, y), or hides it.
    window.__docGifKey = (label, x, y) => {
      key.textContent = label || "";
      key.style.left = `${x}px`;
      key.style.top = `${y}px`;
      key.classList.toggle("on", !!label);
    };
  };
  if (document.readyState === "loading") addEventListener("DOMContentLoaded", installOverlay);
  else installOverlay();
})();
