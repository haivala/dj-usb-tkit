import { test, expect } from "@playwright/test";

function installScanAnalysisMock(page, opts = {}) {
  const trackCount = Number(opts?.trackCount || 40);
  const pieceDelayMs = Number(opts?.pieceDelayMs || 60);
  const workers = Number(opts?.workers || 6);
  const seedExistingWaveform = !!opts?.seedExistingWaveform;
  const seedDuration = !!opts?.seedDuration;
  const seedArtwork = !!opts?.seedArtwork;
  const variedArtists = !!opts?.variedArtists;
  const analysisBpmRange = String(opts?.analysisBpmRange || "70-180");
  return page.addInitScript(({
    trackCount,
    pieceDelayMs,
    workers,
    seedExistingWaveform,
    seedDuration,
    seedArtwork,
    variedArtists,
    analysisBpmRange
  }) => {
    window.localStorage.setItem("djusbtkit.sourceRoots", JSON.stringify(["/music"]));
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    window.localStorage.setItem("djusbtkit.analysisBpmRange", analysisBpmRange);

    const listeners = new Map();
    let pieceCalls = 0;
    const pieceCallsByPiece = { duration: 0, artwork: 0, waveform: 0, bpm_key: 0 };
    let bpmRangeSeen = null;
    const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

    const listen = async (eventName, callback) => {
      const key = String(eventName || "");
      const arr = listeners.get(key) || [];
      arr.push(callback);
      listeners.set(key, arr);
      return () => {
        const current = listeners.get(key) || [];
        listeners.set(key, current.filter((fn) => fn !== callback));
      };
    };

    const tracks = Array.from({ length: Math.max(1, trackCount) }, (_, index) => {
      const n = String(index + 1).padStart(2, "0");
      return {
        id: `track-${n}`,
        title: `Track ${n}`,
        artist: variedArtists ? (index % 2 === 0 ? "B Artist" : "A Artist") : "Batch Artist",
        album: "Batch Album",
        bpm: null,
        key: null,
        filePath: `/music/Batch Artist - Track ${n}.wav`,
        fileSizeBytes: 1000 + index,
        durationMs: null,
        artworkPath: null,
        waveformPeaksPath: null,
        waveformPreview: [],
        createdAt: "2026-03-01T00:00:00Z",
        updatedAt: "2026-03-01T00:00:00Z"
      };
    });
    if (seedExistingWaveform) {
      for (const track of tracks) {
        track.waveformPeaksPath = `/tmp/${track.id}.DAT`;
      }
    }
    if (seedDuration) {
      for (const track of tracks) {
        track.durationMs = 180000;
      }
    }
    if (seedArtwork) {
      for (const track of tracks) {
        track.artworkPath = `/tmp/${track.id}.jpg`;
      }
    }

    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          if (command === "clear_frontend_log") return "";
          if (command === "append_frontend_log") return null;
          if (command === "show_window") return null;
          if (command === "detect_external_master_db") {
            return { ok: true, data: { found: false, path: null } };
          }
          if (command === "list_playlists") {
            return { ok: true, data: { items: [] } };
          }
          if (command === "scan_library") {
            return {
              ok: true,
              data: { jobId: "job-scan-mock", indexed: tracks.length, updated: 0, removed: 0 }
            };
          }
          if (command === "get_system_parallelism") {
            return { ok: true, data: { workers } };
          }
          if (command === "search_tracks" || command === "browse_source_files") {
            const query = String(payload?.request?.query ?? payload?.query ?? "").toLowerCase();
            const filtered = tracks.filter((t) => {
              if (!query) return true;
              return `${t.title} ${t.artist} ${t.album}`.toLowerCase().includes(query);
            });
            return { ok: true, data: { total: filtered.length, items: filtered } };
          }
          if (command === "list_tracks") {
            return { ok: true, data: { total: tracks.length, items: tracks } };
          }
          if (command === "get_tracks_by_ids_with_previews") {
            const ids = Array.isArray(payload?.request?.trackIds)
              ? payload.request.trackIds.map((v) => String(v))
              : [];
            const items = tracks.filter((t) => ids.includes(String(t.id)));
            return { ok: true, data: { items } };
          }
          if (command === "analyze_new_tracks") {
            return {
              ok: true,
              data: { jobId: "job-analysis-mock", analyzed: 0, failed: 0, warnings: [] }
            };
          }
          if (command === "analyze_track_piece") {
            pieceCalls += 1;
            const id = String(payload?.request?.trackId || "");
            const piece = String(payload?.request?.piece || "");
            if (piece === "bpm_key") {
              bpmRangeSeen = {
                min: Number(payload?.request?.bpmMin || 0),
                max: Number(payload?.request?.bpmMax || 0)
              };
            }
            if (Object.hasOwn(pieceCallsByPiece, piece)) {
              pieceCallsByPiece[piece] += 1;
            }
            const track = tracks.find((t) => t.id === id);
            if (!track) {
              return {
                ok: false,
                error: { code: "NOT_FOUND", message: `Track not found: ${id}` }
              };
            }

            await sleep(Math.max(0, pieceDelayMs));

            if (piece === "duration") {
              track.durationMs = 180000;
              return {
                ok: true,
                data: {
                  trackId: id,
                  piece,
                  updated: true,
                  bpm: null,
                  key: null,
                  durationMs: track.durationMs,
                  artworkPath: null,
                  waveformPeaksPath: null,
                  waveformPreview: null
                }
              };
            }
            if (piece === "artwork") {
              track.artworkPath = `/tmp/${id}.jpg`;
              return {
                ok: true,
                data: {
                  trackId: id,
                  piece,
                  updated: true,
                  bpm: null,
                  key: null,
                  durationMs: null,
                  artworkPath: track.artworkPath,
                  waveformPeaksPath: null,
                  waveformPreview: null
                }
              };
            }
            if (piece === "waveform") {
              track.waveformPeaksPath = `/tmp/${id}.DAT`;
              track.waveformPreview = [8, 20, 42, 65, 30, 55];
              return {
                ok: true,
                data: {
                  trackId: id,
                  piece,
                  updated: true,
                  bpm: null,
                  key: null,
                  durationMs: null,
                  artworkPath: null,
                  waveformPeaksPath: track.waveformPeaksPath,
                  waveformPreview: track.waveformPreview
                }
              };
            }
            if (piece === "bpm_key") {
              track.bpm = 120 + (Number(id.replace(/\D+/g, "")) % 4);
              track.key = `${(Number(id.replace(/\D+/g, "")) % 12) + 1}A`;
              track.updatedAt = "2026-03-04T00:00:00Z";
              return {
                ok: true,
                data: {
                  trackId: id,
                  piece,
                  updated: true,
                  bpm: track.bpm,
                  key: track.key,
                  durationMs: null,
                  artworkPath: null,
                  waveformPeaksPath: null,
                  waveformPreview: null
                }
              };
            }

            return {
              ok: false,
              error: { code: "VALIDATION", message: `Unsupported piece: ${piece}` }
            };
          }
          if (command === "fetch_usb_playlists" || command === "fetch_usb_histories") {
            return { ok: true, data: { items: [], warnings: [] } };
          }
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled command: ${command}` } };
        }
      },
      event: { listen }
    };

    window.__scanTestStats = {
      get pieceCalls() {
        return pieceCalls;
      },
      get pieceCallsByPiece() {
        return pieceCallsByPiece;
      },
      get bpmRangeSeen() {
        return bpmRangeSeen;
      }
    };
  }, { trackCount, pieceDelayMs, workers, seedExistingWaveform, seedDuration, seedArtwork, variedArtists, analysisBpmRange });
}

function installPagedMaterializeAnalyzeMock(page, opts = {}) {
  const trackCount = Number(opts?.trackCount || 260);
  const pageSize = Number(opts?.pageSize || 200);
  const pieceDelayMs = Number(opts?.pieceDelayMs || 80);
  const materializedIds = !!opts?.materializedIds;
  return page.addInitScript(({ trackCount, pageSize, pieceDelayMs, materializedIds }) => {
    window.localStorage.setItem("djusbtkit.sourceRoots", JSON.stringify(["/music"]));
    window.localStorage.setItem("djusbtkit.helpSeen", "1");

    const listeners = new Map();
    const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
    let listQueryCount = 0;
    let searchQueryCount = 0;
    let analyzeCalls = 0;
    let materializeCalls = 0;

    const tracks = Array.from({ length: Math.max(1, trackCount) }, (_, index) => {
      const n = String(index + 1).padStart(4, "0");
      const filePath = `/music/Auto Artist - Track ${n}.wav`;
      return {
        id: materializedIds ? `ml-${n}` : filePath,
        localTrackId: materializedIds ? `ml-${n}` : null,
        title: `Track ${n}`,
        artist: "Auto Artist",
        album: "Auto Album",
        bpm: null,
        key: null,
        filePath,
        fileSizeBytes: 10_000 + index,
        durationMs: null,
        artworkPath: null,
        waveformPeaksPath: null,
        waveformPreview: [],
        createdAt: "2026-03-01T00:00:00Z",
        updatedAt: "2026-03-01T00:00:00Z"
      };
    });

    const toTrackDto = (row) => {
      const id = row.localTrackId || row.id;
      return {
        id,
        title: row.title,
        artist: row.artist,
        album: row.album,
        bpm: row.bpm,
        key: row.key,
        filePath: row.filePath,
        fileSizeBytes: row.fileSizeBytes,
        durationMs: row.durationMs,
        artworkPath: row.artworkPath,
        waveformPeaksPath: row.waveformPeaksPath,
        waveformPreview: row.waveformPreview,
        createdAt: row.createdAt,
        updatedAt: row.updatedAt
      };
    };

    const listPage = (cursorValue, query = "") => {
      const q = String(query || "").toLowerCase().trim();
      const filtered = q
        ? tracks.filter((t) => `${t.title} ${t.artist} ${t.album}`.toLowerCase().includes(q))
        : tracks.slice();
      const offset = Number(String(cursorValue || "0")) || 0;
      const items = filtered.slice(offset, offset + pageSize).map(toTrackDto);
      const nextOffset = offset + items.length;
      const hasMore = nextOffset < filtered.length;
      return {
        total: filtered.length,
        items,
        next_cursor: hasMore ? String(nextOffset) : null,
        has_more: hasMore
      };
    };

    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          if (command === "clear_frontend_log") return "";
          if (command === "append_frontend_log") return null;
          if (command === "show_window") return null;
          if (command === "detect_external_master_db") {
            return { ok: true, data: { found: false, path: null } };
          }
          if (command === "list_playlists") {
            return { ok: true, data: { items: [] } };
          }
          if (command === "get_system_parallelism") {
            return { ok: true, data: { workers: 6 } };
          }
          if (command === "list_tracks") {
            listQueryCount += 1;
            const cursor = payload?.request?.cursor ?? null;
            return { ok: true, data: listPage(cursor, "") };
          }
          if (command === "search_tracks") {
            searchQueryCount += 1;
            const query = payload?.request?.query ?? payload?.query ?? "";
            const cursor = payload?.request?.cursor ?? payload?.cursor ?? null;
            return { ok: true, data: listPage(cursor, query) };
          }
          if (command === "browse_source_files") {
            searchQueryCount += 1;
            const query = payload?.request?.query ?? "";
            const cursor = payload?.request?.cursor ?? null;
            return { ok: true, data: listPage(cursor, query) };
          }
          if (command === "materialize_source_track") {
            materializeCalls += 1;
            const filePath = String(payload?.request?.filePath || "");
            const row = tracks.find((t) => t.filePath === filePath);
            if (!row) {
              return { ok: false, error: { code: "NOT_FOUND", message: "path not found" } };
            }
            if (!row.localTrackId) {
              const suffix = String(row.filePath).replace(/[^0-9]+/g, "") || "1";
              row.localTrackId = `ml-${suffix}`;
            }
            return { ok: true, data: { trackId: row.localTrackId } };
          }
          if (command === "analyze_track_piece") {
            analyzeCalls += 1;
            const id = String(payload?.request?.trackId || "");
            const piece = String(payload?.request?.piece || "");
            const row = tracks.find((t) => (t.localTrackId || t.id) === id || t.id === id || t.localTrackId === id);
            if (!row) {
              return { ok: false, error: { code: "NOT_FOUND", message: `track not found: ${id}` } };
            }
            await sleep(Math.max(0, pieceDelayMs));
            if (piece === "duration") {
              row.durationMs = 182000;
              return { ok: true, data: { trackId: id, piece, updated: true, durationMs: row.durationMs } };
            }
            if (piece === "artwork") {
              row.artworkPath = `/tmp/${id}.jpg`;
              return { ok: true, data: { trackId: id, piece, updated: true, artworkPath: row.artworkPath } };
            }
            if (piece === "waveform") {
              row.waveformPeaksPath = `/tmp/${id}.DAT`;
              row.waveformPreview = [10, 25, 45, 70, 50, 30];
              return {
                ok: true,
                data: { trackId: id, piece, updated: true, waveformPeaksPath: row.waveformPeaksPath, waveformPreview: row.waveformPreview }
              };
            }
            if (piece === "bpm_key") {
              row.bpm = 123;
              row.key = "8A";
              return { ok: true, data: { trackId: id, piece, updated: true, bpm: row.bpm, key: row.key } };
            }
            return { ok: false, error: { code: "VALIDATION", message: `Unsupported piece: ${piece}` } };
          }
          if (command === "get_tracks_by_ids_with_previews") {
            const ids = Array.isArray(payload?.request?.trackIds)
              ? payload.request.trackIds.map((v) => String(v))
              : [];
            const items = tracks
              .filter((t) => ids.includes(String(t.localTrackId || t.id)) || ids.includes(String(t.id)))
              .map(toTrackDto);
            return { ok: true, data: { items } };
          }
          if (command === "scan_library") {
            return {
              ok: true,
              data: { jobId: "job-scan-mock", indexed: tracks.length, updated: 0, removed: 0 }
            };
          }
          if (command === "analyze_new_tracks") {
            return {
              ok: true,
              data: { jobId: "job-analysis-mock", analyzed: 0, failed: 0, warnings: [] }
            };
          }
          if (command === "fetch_usb_playlists" || command === "fetch_usb_histories") {
            return { ok: true, data: { items: [], warnings: [] } };
          }
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled command: ${command}` } };
        }
      },
      event: { listen: async (eventName, callback) => {
        const key = String(eventName || "");
        const arr = listeners.get(key) || [];
        arr.push(callback);
        listeners.set(key, arr);
        return () => {
          const current = listeners.get(key) || [];
          listeners.set(key, current.filter((fn) => fn !== callback));
        };
      } }
    };

    window.__pagedAnalyzeStats = {
      get listQueryCount() {
        return listQueryCount;
      },
      get searchQueryCount() {
        return searchQueryCount;
      },
      get analyzeCalls() {
        return analyzeCalls;
      },
      get materializeCalls() {
        return materializeCalls;
      }
    };
  }, { trackCount, pageSize, pieceDelayMs, materializedIds });
}

test("scan applies per-piece row updates before track-ready status", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 1, pieceDelayMs: 250 });
  await page.goto("/");

  await page.locator("#scanLibraryBtn").click();
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(1);

  await page.waitForFunction(() => {
    const row = document.querySelector("#libraryTableBody .track-grid-row");
    return !!row && (row.textContent || "").includes("3:00");
  });
  await expect(page.locator("#libraryTableBody .track-grid-row").first()).toHaveClass(/is-analyzing/);
  await expect.poll(async () => {
    const text = await page.locator("#statusText").textContent();
    return (text || "").includes("Scan analysis: 0/1 track(s) ready")
      || (text || "").includes("Scan done: 1 tracks / 1 albums | analyzed 1, failed 0");
  }).toBeTruthy();
  await expect(page.locator("#libraryTotalDuration")).toContainText("1 without length");

  await page.waitForFunction(() => {
    const waveform = document.querySelector("#libraryTableBody .track-grid-row .waveform");
    return !!waveform?.classList.contains("waveform-canvas");
  });
  await expect.poll(async () => {
    const text = await page.locator("#statusText").textContent();
    return (text || "").includes("Scan analysis: 0/1 track(s) ready")
      || (text || "").includes("Scan done: 1 tracks / 1 albums | analyzed 1, failed 0");
  }).toBeTruthy();

  await expect(page.locator("#statusText")).toContainText("analyzed 1, failed 0");
  await expect(page.locator("#libraryTableBody .bpm-pill")).toHaveCount(1);
  await expect(page.locator("#libraryTableBody .key-pill")).toHaveCount(1);
  await expect.poll(async () => page.evaluate(() => window.__scanTestStats?.bpmRangeSeen)).toEqual({ min: 70, max: 180 });
  await expect(page.locator("#libraryTotalDuration")).toHaveText("Total time: 3:00");
  await expect(page.locator("#libraryTableBody .track-grid-row").first()).not.toHaveClass(/is-analyzing/);
});

test("scan progressively changes action buttons to Reanalyze", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 40, pieceDelayMs: 20 });
  await page.goto("/");

  await page.locator("#scanLibraryBtn").click();
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(40);

  await page.waitForFunction(() => {
    const status = document.querySelector("#statusText")?.textContent || "";
    const buttons = Array.from(document.querySelectorAll('#libraryTableBody button[data-action="analyze-track"]'));
    const ready = buttons.filter((el) => String(el.textContent || "").trim() === "Reanalyze").length;
    return status.includes("Scan analysis:") && ready > 0 && ready < buttons.length;
  });

  const midScanReady = await page
    .locator('#libraryTableBody button[data-action="analyze-track"]', { hasText: "Reanalyze" })
    .count();
  expect(midScanReady).toBeGreaterThan(0);
  expect(midScanReady).toBeLessThan(40);

  await expect(
    page.locator('#libraryTableBody button[data-action="analyze-track"]', { hasText: "Reanalyze" })
  ).toHaveCount(40);
  await expect(
    page.locator('#libraryTableBody button[data-action="analyze-track"]', { hasText: /^Analyze$/ })
  ).toHaveCount(0);
  await expect.poll(async () => page.evaluate(() => window.__scanTestStats?.pieceCalls || 0)).toBeGreaterThan(0);
});

test("scan forwards selected BPM range to analyze_track_piece", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 1, pieceDelayMs: 20, analysisBpmRange: "88-175" });
  await page.goto("/");
  await page.locator("#scanLibraryBtn").click();
  await expect(page.locator("#statusText")).toContainText("analyzed 1, failed 0");
  await expect.poll(async () => page.evaluate(() => window.__scanTestStats?.bpmRangeSeen)).toEqual({ min: 88, max: 175 });
});

test("analysis populates BPM and key cells for all tracks", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 40, pieceDelayMs: 10 });
  await page.goto("/");

  await page.locator("#scanLibraryBtn").click();
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(40);
  await expect(
    page.locator('#libraryTableBody button[data-action="analyze-track"]', { hasText: "Reanalyze" })
  ).toHaveCount(40);

  await expect(page.locator("#libraryTableBody .bpm-pill")).toHaveCount(40);
  await expect(page.locator("#libraryTableBody .key-pill")).toHaveCount(40);
});

test("library total duration advances only when each track is fully ready", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 2, pieceDelayMs: 120, workers: 3 });
  await page.goto("/");

  await page.locator("#scanLibraryBtn").click();
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(2);

  await page.waitForFunction(() => {
    const row = document.querySelector("#libraryTableBody .track-grid-row");
    const status = document.querySelector("#statusText")?.textContent || "";
    const total = document.querySelector("#libraryTotalDuration")?.textContent || "";
    return !!row
      && (row.textContent || "").includes("3:00")
      && status.includes("0/2 track(s) ready")
      && total.includes("2 without length");
  });

  await page.waitForFunction(() => {
    const total = document.querySelector("#libraryTotalDuration")?.textContent || "";
    return total.includes("Total time: 3:00 (1 without length)");
  });

  await expect(page.locator("#libraryTotalDuration")).toHaveText("Total time: 6:00");
});

test("scan skips waveform piece when waveform already exists", async ({ page }) => {
  await installScanAnalysisMock(page, {
    trackCount: 1,
    pieceDelayMs: 20,
    seedExistingWaveform: true,
    seedDuration: true,
    seedArtwork: true
  });
  await page.goto("/");

  await page.locator("#scanLibraryBtn").click();
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(1);
  await expect(page.locator("#statusText")).toContainText("analyzed 1, failed 0");

  await expect.poll(async () => {
    return page.evaluate(() => window.__scanTestStats?.pieceCallsByPiece?.waveform || 0);
  }).toBe(0);
  await expect.poll(async () => {
    return page.evaluate(() => window.__scanTestStats?.pieceCallsByPiece?.bpm_key || 0);
  }).toBeGreaterThan(0);
});

test("analysis patch updates BPM cell in-place without replacing row node", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 1, pieceDelayMs: 120 });
  await page.goto("/");

  await page.locator("#scanLibraryBtn").click();
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(1);

  const initial = await page.evaluate(() => {
    const row = document.querySelector("#libraryTableBody .track-grid-row");
    const bpmCell = row?.querySelector(".td-bpm");
    return {
      rowId: row?.dataset?.trackId || "",
      hasBpmPill: !!bpmCell?.querySelector(".bpm-pill"),
      bpmText: String(bpmCell?.textContent || "").trim()
    };
  });
  expect(initial.hasBpmPill).toBeFalsy();
  expect(initial.bpmText).toBe("-");

  await page.waitForFunction(() => {
    const row = document.querySelector("#libraryTableBody .track-grid-row");
    const bpmCell = row?.querySelector(".td-bpm");
    return !!bpmCell?.querySelector(".bpm-pill");
  });

  const after = await page.evaluate(() => {
    const row = document.querySelector("#libraryTableBody .track-grid-row");
    const bpmCell = row?.querySelector(".td-bpm");
    return {
      rowId: row?.dataset?.trackId || "",
      hasBpmPill: !!bpmCell?.querySelector(".bpm-pill")
    };
  });
  expect(after.hasBpmPill).toBeTruthy();
  expect(after.rowId).toBe(initial.rowId);
});

test("sorted library order stays stable during live analysis patching", async ({ page }) => {
  await installScanAnalysisMock(page, { trackCount: 8, pieceDelayMs: 60, variedArtists: true });
  await page.goto("/");
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(8);

  const sortHeader = page.locator('#panel-library .sortable[data-sort-key="artist"]');
  await sortHeader.click();
  const before = await page.locator("#libraryTableBody .track-grid-row .track-title").allTextContents();

  await page.locator("#scanLibraryBtn").click();
  await expect(page.locator("#statusText")).toContainText("analyzed 8, failed 0");
  const after = await page.locator("#libraryTableBody .track-grid-row .track-title").allTextContents();

  expect(after).toEqual(before);
});

test("analyze on auto-loaded non-materialized track resolves local id", async ({ page }) => {
  await installPagedMaterializeAnalyzeMock(page, { trackCount: 260, pageSize: 200, pieceDelayMs: 120 });
  await page.goto("/");

  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(200);

  await page.evaluate(() => {
    const wrap = document.querySelector("#libraryTableWrap");
    if (!wrap) return;
    wrap.scrollTop = wrap.scrollHeight;
    wrap.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(260);

  const targetRow = page.locator("#libraryTableBody .track-grid-row").nth(230);
  await targetRow.locator('[data-action="analyze-track"]').click();

  await expect(page.locator("#statusText")).not.toContainText("Track is not in local library yet");
  await expect.poll(async () => {
    return page.evaluate(() => Number(window.__pagedAnalyzeStats?.materializeCalls || 0));
  }).toBeGreaterThan(0);
  await expect.poll(async () => {
    return page.evaluate(() => Number(window.__pagedAnalyzeStats?.analyzeCalls || 0));
  }).toBeGreaterThan(0);
});

test("analyze on auto-loaded track updates row in place without full reload", async ({ page }) => {
  await installPagedMaterializeAnalyzeMock(page, {
    trackCount: 260,
    pageSize: 200,
    pieceDelayMs: 120,
    materializedIds: true
  });
  await page.goto("/");

  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(200);

  await page.evaluate(() => {
    const wrap = document.querySelector("#libraryTableWrap");
    if (!wrap) return;
    wrap.scrollTop = wrap.scrollHeight;
    wrap.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(260);

  const beforeStats = await page.evaluate(() => ({
    list: Number(window.__pagedAnalyzeStats?.listQueryCount || 0),
    search: Number(window.__pagedAnalyzeStats?.searchQueryCount || 0)
  }));

  const targetRow = page.locator("#libraryTableBody .track-grid-row").nth(230);
  const before = await targetRow.evaluate((row) => ({
    trackId: String(row.getAttribute("data-track-id") || ""),
    hasBpmPill: !!row.querySelector(".bpm-pill")
  }));
  expect(before.trackId.startsWith("ml-")).toBeTruthy();
  expect(before.hasBpmPill).toBeFalsy();

  await targetRow.locator('[data-action="analyze-track"]').click();

  await expect(page.locator("#statusText")).toContainText("Analyze missing done: analyzed 1, failed 0");
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(260);

  const afterTrackId = await page.locator("#libraryTableBody .track-grid-row").nth(230)
    .evaluate((row) => String(row.getAttribute("data-track-id") || ""));
  expect(afterTrackId).toBe(before.trackId);

  const afterStats = await page.evaluate(() => ({
    list: Number(window.__pagedAnalyzeStats?.listQueryCount || 0),
    search: Number(window.__pagedAnalyzeStats?.searchQueryCount || 0),
    analyzeCalls: Number(window.__pagedAnalyzeStats?.analyzeCalls || 0)
  }));
  expect(afterStats.analyzeCalls).toBeGreaterThan(0);
  expect(afterStats.list).toBe(beforeStats.list);
  expect(afterStats.search).toBeGreaterThanOrEqual(beforeStats.search);
});
