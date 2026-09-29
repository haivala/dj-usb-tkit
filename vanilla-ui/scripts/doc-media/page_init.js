// Page init script for record.mjs: stands in for the Tauri backend with the
// payloads the real backend produced (window.__DOC_GIF_FIXTURE__) plus
// made-up playlists and a connected USB, and draws a cursor and a Shift
// keycap, which a headless screencast doesn't show.
(() => {
  const { tracks, detail, sourceRoots, playlists } = window.__DOC_GIF_FIXTURE__;
  const opts = window.__DOC_GIF_OPTS__ || {};
  const USB_ROOT = "/run/media/dj/Chiphead";
  const ls = window.localStorage;
  ls.setItem("djusbtkit.helpSeen", "1");
  ls.setItem("djusbtkit.theme", "dark");
  ls.setItem("djusbtkit.accentHue", "270");
  ls.setItem("djusbtkit.sourceRoots", JSON.stringify(sourceRoots));
  ls.setItem("djusbtkit.cueStartOnFirstBeat", "0");
  ls.setItem("djusbtkit.cueQuantize", "1");

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
  // The sidebar lists playlists newest first, i.e. reversed.
  const playlistRows = [...playlists].reverse().map(({ id, name, tracks: items }) => ({
    id,
    name,
    source: "app",
    lastExportedAt: items.length ? exportedAt : null,
    lastExportedUsbRoot: items.length ? USB_ROOT : null,
    lastExportedTrackCount: items.length || null,
    trackCount: items.length,
    totalDurationMs: sumMs(items),
    createdAt: exportedAt,
    updatedAt: exportedAt,
  }));

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
            return ok({ found: false, path: null });
          case "list_playlists":
            return ok({ items: playlistRows });
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
          case "check_source_roots":
            return ok({ missing: [] });
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
            return ok({ overallStatus: "PASS", durationMs: 842, warnings: [], checks: [], playlistUsbExportStatus: [] });
          case "fetch_usb_playlists":
          case "fetch_usb_histories":
            return ok({ items: [], warnings: [] });
          case "list_tracks":
          case "search_tracks":
            return ok({ total: tracks.length, items: tracks });
          case "browse_source_files":
            return ok({
              total: tracks.length,
              items: tracks,
              nextCursor: null,
              hasMore: false,
              sourceRootAnalysis: sourceRoots.map((sourceRoot) => ({ sourceRoot, fullyAnalyzed: true })),
              totalDurationMs: sumMs(tracks),
              durationKnownCount: tracks.length,
            });
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
            return { ok: false, error: { code: "UNKNOWN", message: `Not stubbed: ${command}` } };
        }
      },
    },
    event: { listen: async () => () => {} },
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
