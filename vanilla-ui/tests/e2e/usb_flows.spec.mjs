import { test, expect } from "./coverage-fixture.mjs";

// Counts the `fetch_usb_playlists` calls in `window.__playlistFetches`, and the
// track table fetches in `window.__trackFetches`.
function countPlaylistFetches(page) {
  return page.addInitScript(() => {
    const invoke = window.__TAURI__.core.invoke;
    window.__playlistFetches = 0;
    window.__trackFetches = 0;
    window.__TAURI__.core.invoke = async (command, payload) => {
      if (command === "fetch_usb_playlists") window.__playlistFetches += 1;
      if (command === "fetch_usb_playlist_tracks" || command === "fetch_usb_history_tracks") window.__trackFetches += 1;
      return invoke(command, payload);
    };
  });
}

// Empty USB lists, answered only once the test calls `window.__releaseLists()`.
function holdEmptyLists(page) {
  return page.addInitScript(() => {
    const invoke = window.__TAURI__.core.invoke;
    const released = new Promise((resolve) => { window.__releaseLists = resolve; });
    window.__TAURI__.core.invoke = async (command, payload) => {
      const backend = window.__e2eBackend;
      if (command === "fetch_usb_playlists") {
        await released;
        return backend.ok(backend.usbPlaylists());
      }
      if (command === "fetch_usb_histories") {
        await released;
        return backend.ok(backend.usbHistories());
      }
      return invoke(command, payload);
    };
  });
}

// Subscribes the app to `job:event`: `window.__emitJobEvent` fires one.
function listenForJobEvents(page) {
  return page.addInitScript(() => {
    const listeners = [];
    window.__emitJobEvent = (payload) => {
      for (const callback of listeners.slice()) callback({ event: "job:event", payload });
    };
    window.isTauri = true;
    window.__TAURI_INTERNALS__ = { invoke: (cmd, args) => window.__TAURI__.core.invoke(cmd, args) };
    window.__TAURI__.event = {
      listen: async (eventName, callback) => {
        if (eventName === "job:event") listeners.push(callback);
        return () => {};
      }
    };
  });
}

function installTauriMock(page, mode) {
  return page.addInitScript(({ mode }) => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    const state = {
      initialized: mode === "valid" || mode === "warning-mix" || mode === "toggle-usb" || mode === "reorder" || mode === "reorder-scroll" || mode === "progress-race",
      pickCount: 0,
      usbPlaylists: mode === "valid" || mode === "warning-mix"
        ? [
            {
              id: "usb-1",
              name: "Warmup",
              source: "mock-tauri",
              tracks: [
                { title: "Track A", artist: "Artist 1", album: "Album X", bpm: 124, key: "8A" }
              ]
            }
          ]
        : mode === "reorder"
        ? [
            { id: "usb-pl-1", name: "Alpha", source: "mock-tauri", tracks: [{ title: "Track A" }] },
            { id: "usb-pl-2", name: "Bravo", source: "mock-tauri", tracks: [{ title: "Track B" }] },
            { id: "usb-pl-3", name: "Charlie", source: "mock-tauri", tracks: [{ title: "Track C" }] }
          ]
        : mode === "reorder-scroll"
        ? Array.from({ length: 40 }, (_, i) => ({
            id: `usb-pl-${i}`,
            name: `Playlist ${String(i).padStart(2, "0")}`,
            source: "mock-tauri",
            tracks: [{ title: "Track" }]
          }))
        : []
    };
    window.__reorderCalls = [];
    window.__usbAudioFormatCalls = [];
    let usbAudioFormats = [
      ["flac", "FLAC"], ["alac", "ALAC"], ["aiff", "AIFF"], ["wav", "WAV"], ["mp3", "MP3"], ["aac", "AAC"]
    ].map(([format, label]) => ({ format, label, allowed: true }));

    const diagnosticsPayload = {
      overallStatus: "WARN",
      overview: [
        { title: "Older Pioneer players", status: "PASS", detail: "Ready: 1 playlists, 3 tracks." },
        { title: "Waveforms & beat grids", status: "WARN", detail: "Some tracks' waveform or beat grid data is missing or outdated." },
        { title: "Denon / Numark players", status: null, detail: "Not set up." }
      ],
      sectionGroups: [
        {
          title: "Databases",
          sections: [
            { title: "PDB", status: "PASS", checks: [] },
            {
              title: "Engine DJ",
              status: "WARN",
              checks: [
                { label: "Kept up to date", status: "WARN", detail: "\"Write Engine DJ library\" is off" }
              ]
            }
          ]
        },
        {
          title: "Library",
          sections: [
            {
              title: "Playlists",
              status: "PASS",
              checks: [
                { label: "Overall resolution", status: "PASS", detail: "3/3 entries resolve (100.0%) across 1 playlists" }
              ]
            },
            { title: "Analysis", status: "WARN", checks: [] }
          ]
        }
      ],
      playlistDetails: [
        {
          name: "Warmup",
          totalEntries: 3,
          resolvedEntries: 3,
          resolutionRate: 1,
          status: "PASS",
          pedbEntries: 3,
          edbEntries: 3,
          matchedEntries: 3,
          pedbMatchRate: 1,
          edbMatchRate: 1
        }
      ],
      warnings: [],
      durationMs: 10,
      playlistUsbExportStatus: []
    };

    window.__TAURI__ = {
      core: {
        invoke: async (command, payload = {}) => {
          if (command === "pick_usb_folder") {
            if (mode === "toggle-usb") {
              state.pickCount += 1;
              return state.pickCount % 2 === 1 ? "/Volumes/USB-TEST" : "/Volumes/USB-INVALID";
            }
            return "/Volumes/USB-TEST";
          }
          if (command === "create_playlist" && mode === "progress-race") {
            // Still saving well past the previous job's footer-hide delay.
            await new Promise((resolve) => setTimeout(resolve, 2500));
            return { ok: true, data: { playlistId: "p-new", name: payload?.request?.name } };
          }
          if (command === "list_usb_devices") {
            return { ok: true, data: { items: [] } };
          }
          if (command === "prune_usb_device") {
            return { ok: true, data: { pruned: true } };
          }
          if (command === "run_usb_diagnostics") {
            return { ok: true, data: diagnosticsPayload };
          }
          if (command === "run_usb_parity_report") {
            return {
              ok: true,
              data: {
                overallStatus: "FAIL",
                statusLabel: "Parity FAIL",
                statusText: "Parity report complete",
                durationText: "Completed in 1ms",
                overview: [
                  { title: "Playlists", status: "PASS", detail: "Both databases have the same 1 playlists, with the same tracks in the same order." },
                  { title: "Track details", status: "FAIL", detail: "1 tracks are missing track details on older Pioneer players." },
                  { title: "Music files", status: "FAIL", detail: "1 tracks point to a different audio or waveform file depending on the player." }
                ],
                sectionGroups: [
                  {
                    title: "Comparison",
                    sections: [
                      {
                        title: "Track details",
                        status: "FAIL",
                        checks: [
                          {
                            label: "PDB metadata completeness",
                            status: "FAIL",
                            detail: "1 playlist-linked PDB track missing required player metadata",
                            link: "event-log"
                          }
                        ]
                      },
                      {
                        title: "Music files",
                        status: "FAIL",
                        checks: [
                          {
                            label: "Media and analysis path parity",
                            status: "FAIL",
                            detail: "1 playlist-linked track with media/analysis path mismatches"
                          }
                        ]
                      }
                    ]
                  }
                ],
                playlistDetails: [
                  {
                    name: "Warmup",
                    pdbTracks: 3,
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
                    sampleOnlyInPdb: [],
                    sampleOnlyInEdb: [],
                    sampleMetadataMismatches: ["Track A [analysisPath, artworkPath, artistDictId, pdbRequiredMetadata]"],
                    status: "FAIL",
                    // Backend-computed (service::diagnostics::parity_issue_labels).
                    issueLabels: ["PDB gaps 1", "path mismatch 1", "dict issues 1", "art mismatch 1"]
                  }
                ],
                warnings: [],
                durationMs: 10
              }
            };
          }
          if (command === "repair_usb_diagnostics") {
            if (payload?.request?.apply) {
              return {
                ok: true,
                data: {
                  appliedFixes: payload.request.selectedFixIds || [],
                  failedFixes: [],
                  warnings: [],
                  durationMs: 1,
                  diagnostics: diagnosticsPayload,
                  usbContentChanged: (payload.request.selectedFixIds || []).length > 0,
                  engineLibraryEnabled: (payload.request.selectedFixIds || []).includes("keep_engine_library_up_to_date")
                }
              };
            }
            return {
              ok: true,
              data: {
                summary: "Apply Fixes repairs 4 problems on this USB. The databases are backed up first.",
                overview: [
                  { title: "Older Pioneer players", status: "WARN", detail: "Repairs damage left in the player database by an interrupted export." },
                  { title: "Denon / Numark players", status: "WARN", detail: "Rebuilds the Denon / Numark library." },
                  { title: "Needs your attention", status: "FAIL", detail: "1 problem can't be fixed automatically." }
                ],
                detectedIssues: [
                  "1 table with a declared last page beyond the physical end of the file",
                  "PDB shows signs of a torn additive-growth write",
                  "1 PDB history/columns page with wrong u5/num_rl shape"
                ],
                rows: [
                  {
                    id: "repair_pdb_truncated_table_chain",
                    title: "Repair Truncated Table Chain",
                    description: "Structural prerequisite fix.",
                    metaText: "✓ supported · safe · always applied",
                    selectable: true,
                    checked: true,
                    locked: true,
                    lockedTooltip: "Always applied"
                  },
                  {
                    id: "repair_pdb_torn_growth_pages",
                    title: "Repair Torn Growth Pages (Interrupted Export)",
                    description: "Structural prerequisite fix.",
                    metaText: "✓ supported · safe · always applied",
                    selectable: true,
                    checked: true,
                    locked: true,
                    lockedTooltip: "Always applied"
                  },
                  {
                    id: "repair_pdb_wrong_history_page_shape",
                    title: "Repair PDB History/Columns Page Footer Shape",
                    description: "Ordinary structural fix, not a prerequisite.",
                    metaText: "✓ supported · safe",
                    selectable: true,
                    checked: true,
                    locked: false
                  },
                  {
                    id: "keep_engine_library_up_to_date",
                    title: "Keep Engine DJ Library Up to Date",
                    description: "Turns on \"Write Engine DJ library\" and rebuilds it now.",
                    metaText: "✓ supported · destructive",
                    selectable: true,
                    checked: true,
                    locked: false
                  },
                  {
                    id: "unsupported:malformed",
                    title: "2 malformed USBANLZ entry/entries",
                    description: "Inspect Event Log entries tagged 'analysis entry malformed'.",
                    metaText: "✗ preview-only · safe",
                    selectable: false,
                    checked: false,
                    locked: false,
                    link: "event-log"
                  }
                ],
                estimatedFileWrites: 3,
                estimatedFileDeletes: 0,
                warnings: []
              }
            };
          }
          if (command === "validate_usb_root") {
            const path = String(payload?.request?.path || "");
            if (!path) {
              return {
                ok: true,
                data: {
                  valid: false,
                  normalizedRoot: null,
                  warnings: ["USB path is empty"]
                }
              };
            }
            if (state.initialized) {
              const forceInvalid = mode === "toggle-usb" && String(path).includes("INVALID");
              if (forceInvalid) {
                return {
                  ok: true,
                  data: {
                    valid: false,
                    normalizedRoot: path,
                    canInitialize: true,
                    statusText: "USB selected but not initialized.",
                    initHintText: "USB folder is writable but missing External library structure (Missing vendor root folder)",
                    warnings: ["Missing vendor root folder"]
                  }
                };
              }
              return {
                ok: true,
                data: {
                  valid: true,
                  normalizedRoot: path,
                  warnings: []
                }
              };
            }
            const warnings = mode === "needs-init-structured"
              ? [
                  {
                    level: "warn",
                    code: "usb.validate.missing-vendor-root",
                    message: "Missing vendor root folder",
                    source: "usb-validate"
                  },
                  {
                    level: "warn",
                    code: "usb.validate.missing-contents",
                    message: "Missing Contents directory",
                    source: "usb-validate"
                  }
                ]
              : ["missing External library structure"];
            return {
              ok: true,
              data: {
                valid: false,
                normalizedRoot: path,
                canInitialize: true,
                statusText: "USB selected but not initialized.",
                initHintText: mode === "needs-init-structured"
                  ? "Missing vendor root folder | Missing Contents directory"
                  : "missing External library structure",
                warnings
              }
            };
          }
          if (command === "initialize_usb") {
            state.initialized = true;
            return {
              ok: true,
              data: {
                path: payload?.request?.usbRoot || "",
                createdDirs: ["vendor-db", "Contents"]
              }
            };
          }
          if (command === "fetch_usb_playlists") {
            const warnings = mode === "warning-mix"
              ? [
                  {
                    level: "info",
                    code: "usb.playlists.info",
                    message: "USB root in use: /Volumes/USB-TEST",
                    source: "usb-import"
                  },
                  {
                    level: "warn",
                    code: "usb.playlists.partial",
                    message: "Some analysis files are missing",
                    source: "usb-import"
                  },
                  {
                    level: "error",
                    code: "usb.playlists.timeout",
                    message: "Timed out reading artwork index",
                    source: "usb-import"
                  }
                ]
              : [];
            return {
              ok: true,
              data: {
                items: state.usbPlaylists.map((p) => ({
                  ...p,
                  trackCount: p.tracks.length,
                  selectedText: `USB playlist selected: ${p.name}`,
                  removeConfirmText: `mock remove ${p.name}?`
                })),
                summaryText: `${state.usbPlaylists.length} playlists`,
                statusText: `USB playlists loaded: ${state.usbPlaylists.length}${mode === "warning-mix" ? " | (2 warnings)" : ""}`,
                doneText: "Done",
                stats: {
                  indexedTracks: 0,
                  playlistReferencedTracks: 0,
                  playlistEntries: state.usbPlaylists.reduce((sum, p) => sum + p.tracks.length, 0)
                },
                playlistUsbExportStatus: [],
                warnings,
                // The backend counts the warn/error entries (the info one isn't).
                warningCount: mode === "warning-mix" ? 2 : 0
              }
            };
          }
          // As the backend sends them: the last ticked box locked, the hint.
          const formatsData = () => {
            const ticked = usbAudioFormats.filter((item) => item.allowed).length;
            return {
              formats: usbAudioFormats.map((item) => {
                const last = item.allowed && ticked === 1;
                return { ...item, locked: last, tooltip: last ? "At least one format stays ticked" : null };
              }),
              writable: true,
              hint: "The formats export puts on this USB · drag to set the order of preference",
            };
          };
          if (command === "fetch_usb_playlist_tracks") {
            const id = String(payload?.request?.id);
            const tracks = state.usbPlaylists.find((p) => String(p.id) === id)?.tracks || [];
            return { ok: true, data: { items: tracks, total: tracks.length, listSummaryText: `${tracks.length} listed` } };
          }
          if (command === "get_usb_audio_formats") {
            return { ok: true, data: formatsData() };
          }
          if (command === "set_usb_audio_formats") {
            const sent = payload?.request?.formats || [];
            window.__usbAudioFormatCalls.push(sent);
            const labels = new Map(usbAudioFormats.map((item) => [item.format, item.label]));
            usbAudioFormats = sent.map(({ format, allowed }) => ({ format, allowed, label: labels.get(format) }));
            return { ok: true, data: formatsData() };
          }
          if (command === "reorder_usb_playlists") {
            const orderedIds = Array.isArray(payload?.request?.orderedPlaylistIds)
              ? payload.request.orderedPlaylistIds
              : [];
            window.__reorderCalls.push(orderedIds);
            const byId = new Map(state.usbPlaylists.map((p) => [String(p.id), p]));
            state.usbPlaylists = orderedIds
              .map((id) => byId.get(String(id)))
              .filter((p) => p !== undefined);
            return {
              ok: true,
              data: {
                reordered: state.usbPlaylists.length,
                statusText: "Playlist order saved | (1 warning)",
                warnings: [{ level: "error", code: "usb.reorder.pdb-write-back-failed", message: "PDB write-back to USB failed", source: "usb-import" }],
                warningCount: 1
              }
            };
          }
          if (command === "remove_usb_playlist") {
            const playlistId = String(payload?.request?.playlistId || "");
            state.usbPlaylists = state.usbPlaylists.filter((p) => String(p.id) !== playlistId);
            return {
              ok: true,
              data: {
                playlistName: payload?.request?.playlistName || "",
                statusText: `Removed USB playlist: ${payload?.request?.playlistName || ""}`,
                removedFromEdb: 1,
                removedFromPdb: 1,
                warnings: mode === "reorder"
                  ? [{ level: "warn", code: "usb.remove.kept-file", message: "Removal kept a shared file", source: "usb-import" }]
                  : [],
                warningCount: mode === "reorder" ? 1 : 0
              }
            };
          }
          return window.__e2eBackend.answer(command);
        }
      }
    };
  }, { mode });
}

test("USB initialize flow: invalid-but-writable root can be initialized and unlocked", async ({ page }) => {
  await installTauriMock(page, "needs-init");
  await page.goto("/");

  // Navigate to USB panel via sidebar
  await page.locator('.nav-item[data-view="usb"]').click();
  // USB connection bar is hidden until a folder is picked;
  // click the empty-state action button which delegates to #selectUsbFolderBtn
  await page.locator("#usbEmptyState .empty-state-action").click();

  await expect(page.locator("#usbInitRow")).not.toHaveClass(/hidden/);
  await expect(page.locator("#initializeUsbBtn")).toBeEnabled();
  await expect(page.locator("#usbInitHint")).toContainText("missing External library structure");
  // The heading's diagnostics buttons only show for a usable USB.
  await expect(page.locator("#usbDiagToolbar")).toBeHidden();

  await page.locator("#initializeUsbBtn").click();

  await expect(page.locator("#usbInitRow")).toHaveClass(/hidden/);
  await expect(page.locator("#usbSelectedControls")).not.toHaveClass(/hidden/);
  await expect(page.locator("#usbDiagToolbar")).toBeVisible();
  await expect(page.locator("#reDiagnoseBtn")).toBeVisible();
});

test("a repair preview with nothing to fix hides the list and turns Preview Fixes off", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.addInitScript(() => {
    const invoke = window.__TAURI__.core.invoke;
    window.__TAURI__.core.invoke = async (command, payload) => {
      if (command === "repair_usb_diagnostics" && !payload?.request?.apply) {
        return { ok: true, data: { summary: "Nothing to repair.", overview: [], rows: [], nothingToFix: true } };
      }
      return invoke(command, payload);
    };
  });
  await page.goto("/");
  await connectUsb(page);
  await page.locator("#usbHealthCard").evaluate((node) => { node.open = true; });
  await page.locator("#previewRepairsBtn").click();
  await expect(page.locator("#diagRepairSummary")).toHaveText("Nothing to repair.");
  await expect(page.locator("#diagRepairTechnical")).toBeHidden();
  await expect(page.locator("#previewRepairsBtn")).toBeDisabled();
  await expect(page.locator("#applyRepairsBtn")).toBeDisabled();
});

test("USB initialize hint and status show what the backend sends", async ({ page }) => {
  await installTauriMock(page, "needs-init-structured");
  await page.goto("/");

  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();

  await expect(page.locator("#usbInitRow")).not.toHaveClass(/hidden/);
  await expect(page.locator("#usbInitHint")).toContainText("Missing vendor root folder");
  await expect(page.locator("#usbInitHint")).toContainText("Missing Contents directory");
  await expect(page.locator("#statusText")).toContainText("USB selected but not initialized.");
});

test("USB playlist removal confirm path handles cancel and confirm", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.goto("/");

  // Navigate to USB, select folder, then go to playlists
  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();

  // Open a diagnostics report first -- a successful removal must clear it,
  // since it no longer reflects the (now stale) playlist set it reported on.
  const usbHealthCard = page.locator("#usbHealthCard");
  await usbHealthCard.evaluate((node) => { node.open = true; });
  await page.locator("#reDiagnoseBtn").click();
  await expect(page.locator("#diagSections")).not.toBeEmpty();

  await page.locator('.nav-item[data-view="usb-playlists"]').click();
  await expect(page.locator('[data-usb-playlist="usb-1"]')).toBeVisible();

  // The remove × only shows on the hovered row.
  await expect(page.locator('[data-usb-remove-playlist="usb-1"]')).toBeHidden();
  await page.locator('[data-usb-playlist="usb-1"]').hover();
  await page.locator('[data-usb-remove-playlist="usb-1"]').click();
  await expect(page.locator("#confirmOverlay")).toBeVisible();
  await expect(page.locator("#confirmMessage")).toHaveText("mock remove Warmup?");
  await page.locator("#confirmCancelBtn").click();
  await expect(page.locator("#confirmOverlay")).toBeHidden();
  await expect(page.locator('[data-usb-playlist="usb-1"]')).toBeVisible();

  await page.locator('[data-usb-playlist="usb-1"]').hover();
  await page.locator('[data-usb-remove-playlist="usb-1"]').click();
  await page.locator("#confirmOkBtn").click();

  await expect(page.locator("#statusText")).toContainText("Removed USB playlist: Warmup");
  await expect(page.locator("#usbPlaylists")).toBeEmpty();
  await expect(page.locator("#usbPlaylists")).toHaveAttribute("data-loaded");
  await expect(page.locator("#diagSections")).toBeEmpty();
  await expect(page.locator("#diagOverallStatus")).toBeEmpty();
});

test("USB playlists tab shows the empty text only after a read finds nothing", async ({ page }) => {
  await installTauriMock(page, "valid");
  await holdEmptyLists(page);
  await countPlaylistFetches(page);
  await page.goto("/");

  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();
  await page.locator('.nav-item[data-view="usb-playlists"]').click();

  await expect(page.locator("#panel-usb-playlists")).toHaveClass(/active/);
  await expect(page.locator("#usbPlaylists")).toBeEmpty();
  await expect(page.locator("#usbPlaylists")).not.toHaveAttribute("data-loaded");

  await page.evaluate(() => window.__releaseLists());
  await expect(page.locator("#usbPlaylists")).toHaveAttribute("data-loaded");
  await expect(page.locator("#usbPlaylists")).toBeEmpty();
  expect(await page.locator("#usbPlaylists").evaluate((node) => getComputedStyle(node, "::before").content)).toBe('"No playlists on this USB."');
  await expect(page.locator("#panel-usb-playlists .split > .right")).toBeHidden();
  expect(await page.evaluate(() => window.__trackFetches)).toBe(0);
});

test("USB history tab shows the empty text only after a read finds nothing", async ({ page }) => {
  await installTauriMock(page, "valid");
  await holdEmptyLists(page);
  await countPlaylistFetches(page);
  await page.goto("/");

  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();
  await page.locator('.nav-item[data-view="usb-history"]').click();

  await expect(page.locator("#panel-usb-history")).toHaveClass(/active/);
  await expect(page.locator("#historyList")).toBeEmpty();
  await expect(page.locator("#historyList")).not.toHaveAttribute("data-loaded");

  await page.evaluate(() => window.__releaseLists());
  await expect(page.locator("#historyList")).toHaveAttribute("data-loaded");
  await expect(page.locator("#historyList")).toBeEmpty();
  expect(await page.locator("#historyList").evaluate((node) => getComputedStyle(node, "::before").content)).toBe('"No history on this USB."');
  await expect(page.locator("#panel-usb-history .split > .right")).toBeHidden();
  await expect(page.locator("#exportHistoryTracklistBtn")).toBeHidden();
  expect(await page.evaluate(() => window.__trackFetches)).toBe(0);
});

test("USB playlists status shows the backend's warning count, linked to the Event Log", async ({ page }) => {
  await installTauriMock(page, "warning-mix");
  await page.goto("/");

  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();
  await page.locator('.nav-item[data-view="usb-playlists"]').click();

  // The first playlist opens by itself, and that doesn't replace the status.
  await expect(page.locator('[data-usb-playlist="usb-1"]')).toHaveClass(/active/);
  await expect(page.locator("#usbPlaylistHeading .track-list-heading-name")).toHaveText("Warmup");
  await expect(page.locator("#usbPlaylistTracks .track-grid-row")).toHaveCount(1);
  await expect(page.locator("#statusText")).toContainText("USB playlists loaded: 1 | (2 warnings)");
  await expect(page.locator("#statusText .status-warning-link")).toHaveText("(2 warnings)");
});

test("USB lists read after leaving their pages open the first item on return, and keep it on the next return", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.addInitScript(() => {
    const invoke = window.__TAURI__.core.invoke;
    const released = new Promise((resolve) => { window.__releaseReads = resolve; });
    window.__TAURI__.core.invoke = async (command, payload) => {
      const backend = window.__e2eBackend;
      if (command === "fetch_usb_playlists") await released;
      if (command === "fetch_usb_histories") {
        await released;
        return backend.ok(backend.usbHistories({ items: [{ id: "h-1", name: "HISTORY 001", tracks: [{ title: "Track A" }] }] }));
      }
      if (command === "fetch_usb_history_tracks") return backend.ok({ items: [{ title: "Track A" }], total: 1 });
      return invoke(command, payload);
    };
  });
  await countPlaylistFetches(page);
  await page.goto("/");

  const nav = (view) => page.locator(`.nav-item[data-view="${view}"]`);
  await nav("usb").click();
  await page.locator("#usbEmptyState .empty-state-action").click();
  await nav("usb-playlists").click();
  await nav("usb-history").click();
  await nav("usb").click();

  await page.evaluate(() => window.__releaseReads());
  const lists = [
    { view: "usb-playlists", row: "#usbPlaylists button", heading: "#usbPlaylistHeading", name: "Warmup", body: "#usbPlaylistTracks" },
    { view: "usb-history", row: "#historyList button", heading: "#historyHeading", name: "HISTORY 001", body: "#historyTracks" },
  ];
  for (const { row } of lists) {
    await expect(page.locator(row)).toHaveCount(1);
    await expect(page.locator(row)).not.toHaveClass(/active/);
  }
  expect(await page.evaluate(() => window.__trackFetches)).toBe(0);

  for (const [index, { view, row, heading, name, body }] of lists.entries()) {
    for (let open = 0; open < 2; open += 1) {
      await nav(view).click();
      await expect(page.locator(row)).toHaveClass(/active/);
      await expect(page.locator(`${heading} .track-list-heading-name`)).toHaveText(name);
      await expect(page.locator(`${body} .track-grid-row`)).toHaveCount(1);
      await nav("usb").click();
    }
    expect(await page.evaluate(() => window.__trackFetches)).toBe(index + 1);
  }
  expect(await page.evaluate(() => window.__playlistFetches)).toBe(1);
});

test("removing a USB playlist keeps the open one; removing the open one opens nothing, even clicking its page again, until the page is opened again", async ({ page }) => {
  await installTauriMock(page, "reorder");
  await countPlaylistFetches(page);
  await page.addInitScript(() => {
    const invoke = window.__TAURI__.core.invoke;
    window.__TAURI__.core.invoke = async (command, payload) => {
      if (command === "fetch_usb_playlist_tracks") await window.__tracksGate;
      return invoke(command, payload);
    };
  });
  await page.goto("/");

  const nav = page.locator('.nav-item[data-view="usb-playlists"]');
  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();
  await nav.click();
  const first = page.locator('[data-usb-playlist="usb-pl-1"]');
  await expect(first).toHaveClass(/active/);
  await expect(page.locator("#usbPlaylistTracks .track-grid-row")).toHaveCount(1);

  const rows = page.locator("#usbPlaylists button");
  // The heading keeps its count while the open playlist's table loads again.
  await page.evaluate(() => {
    window.__tracksGate = new Promise((resolve) => { window.__releaseTracks = resolve; });
  });
  await page.locator('[data-usb-playlist="usb-pl-2"]').hover();
  await page.locator('[data-usb-remove-playlist="usb-pl-2"]').click();
  await page.locator("#confirmOkBtn").click();
  await expect(page.locator("#statusText")).toContainText("Removed USB playlist: Bravo");
  await expect(rows).toHaveCount(2);
  await expect(page.locator("#usbPlaylistHeading .track-list-heading-count")).toHaveText("1 listed");
  await page.evaluate(() => { window.__tracksGate = null; window.__releaseTracks(); });
  await expect(first).toHaveClass(/active/);
  await expect(page.locator("#usbPlaylistHeading .track-list-heading-name")).toHaveText("Alpha");
  await expect(page.locator("#usbPlaylistTracks .track-grid-row")).toContainText("Track A");

  await first.hover();
  await page.locator('[data-usb-remove-playlist="usb-pl-1"]').click();
  await page.locator("#confirmOkBtn").click();
  await expect(page.locator("#statusText")).toContainText("Removed USB playlist: Alpha");
  await expect(rows).toHaveCount(1);
  await expect(page.locator("#usbPlaylists button.active")).toHaveCount(0);

  await nav.click();
  await page.waitForTimeout(100);
  await expect(page.locator("#usbPlaylists button.active")).toHaveCount(0);
  expect(await page.evaluate(() => window.__trackFetches)).toBe(2);

  await page.locator('.nav-item[data-view="usb"]').click();
  await nav.click();
  await expect(page.locator('[data-usb-playlist="usb-pl-3"]')).toHaveClass(/active/);
  await expect(page.locator("#usbPlaylistTracks .track-grid-row")).toHaveCount(1);
  expect(await page.evaluate(() => window.__trackFetches)).toBe(3);

  // A removal's warnings reach the Event Log its status line links to.
  await page.locator("#eventLogBtn").click();
  await expect(page.locator("#eventLogList")).toContainText("Removal kept a shared file");
});

test("a failed USB list read retries when the page is opened again", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.addInitScript(() => {
    const invoke = window.__TAURI__.core.invoke;
    let fetches = 0;
    window.__TAURI__.core.invoke = async (command, payload) => {
      if (command === "fetch_usb_playlists") {
        fetches += 1;
        if (fetches === 1) return { ok: false, error: { code: "IO", message: "read failed" } };
      }
      return invoke(command, payload);
    };
  });
  await page.goto("/");
  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();

  await page.locator('.nav-item[data-view="usb-playlists"]').click();
  await expect(page.locator("#statusText")).toContainText("read failed");
  await expect(page.locator("#usbPlaylists")).toBeEmpty();

  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator('.nav-item[data-view="usb-playlists"]').click();
  await expect(page.locator('[data-usb-playlist="usb-1"]')).toBeVisible();
});

test("console.log messages appear in Event Log", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.goto("/");

  const marker = `console-mirror-${Date.now()}`;
  await page.evaluate((text) => {
    console.log(text);
  }, marker);

  await page.locator("#eventLogBtn").click();

  await expect(page.locator("#eventLogList")).toContainText(marker);
});

test("CSP security events appear in Event Log", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.goto("/");

  await page.evaluate(() => {
    const evt = new Event("securitypolicyviolation");
    Object.defineProperty(evt, "violatedDirective", { value: "style-src", configurable: true });
    Object.defineProperty(evt, "blockedURI", { value: "inline", configurable: true });
    window.dispatchEvent(evt);
  });

  await page.locator("#eventLogBtn").click();
  await expect(page.locator("#eventLogList")).toContainText("CSP violation: style-src");
});

test("startup includes console bridge message in Event Log", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.goto("/");

  await page.locator("#eventLogBtn").click();
  await expect(page.locator("#eventLogList")).toContainText("Frontend console bridge initialized");
});

test("Event Log source filter dropdown never grows duplicate options for a repeated source", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.goto("/");

  // Startup alone already logs several "startup"-sourced events; add more
  // "console"-sourced ones on top so the same source shows up repeatedly
  // across renders -- ensureEventLogSourceOptions must still add each
  // distinct source to the filter dropdown only once.
  await page.evaluate(() => {
    console.log("dup-source-1");
    console.log("dup-source-2");
    console.log("dup-source-3");
  });

  await page.locator("#eventLogBtn").click();

  await expect(page.locator("#eventLogList")).toContainText("dup-source-3");
  await expect(page.locator('#eventLogSourceFilter option[value="console"]')).toHaveCount(1);
  await expect(page.locator('#eventLogSourceFilter option[value="startup"]')).toHaveCount(1);
});

test("USB sub-nav reveal/hide and fallback to USB panel", async ({ page }) => {
  await installTauriMock(page, "toggle-usb");
  await page.goto("/");

  const usbPlaylistsNav = page.locator('.nav-sub-item[data-view="usb-playlists"]');
  await expect(usbPlaylistsNav).not.toHaveClass(/revealed/);
  await expect(usbPlaylistsNav).toBeHidden(); // Keyboard can't reach it either.

  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();
  await expect(usbPlaylistsNav).toHaveClass(/revealed/);

  await usbPlaylistsNav.click();
  await expect(page.locator("#panel-usb-playlists")).toHaveClass(/active/);

  // Backups opens from the USB page toolbar; the sidebar keeps USB highlighted.
  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbBackupsBtn").click();
  await expect(page.locator("#panel-backups")).toHaveClass(/active/);
  await expect(page.locator('.nav-item[data-view="usb"]')).toHaveClass(/active/);

  // Second folder pick simulates disconnect/invalid USB in toggle mode.
  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#selectUsbFolderBtn").click();
  await expect(usbPlaylistsNav).not.toHaveClass(/revealed/);
  await expect(page.locator("#panel-usb")).toHaveClass(/active/);
});

test("Event Log header button counts errors logged while closed", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.goto("/");

  const badge = page.locator("#eventLogBadge");
  await page.locator("#eventLogBtn").click();
  await expect(badge).toBeHidden();
  await page.locator('.nav-item[data-view="library"]').click();

  await page.evaluate(() => console.warn("badge-test-warning"));
  await expect(badge).toBeHidden();

  await page.evaluate(() => {
    console.error("badge-test-error-1");
    console.error("badge-test-error-2");
  });
  await expect(badge).toHaveText("2");
  await expect(page.locator("#eventLogBtn")).toHaveAttribute("aria-label", "Event Log, 2 new errors");

  await page.locator("#eventLogBtn").click();
  await expect(badge).toBeHidden();
  await expect(page.locator("#eventLogBtn")).toHaveClass(/active/);

  // Errors logged while the log is open are seen, not counted.
  await page.evaluate(() => console.error("badge-test-error-3"));
  await page.locator('.nav-item[data-view="library"]').click();
  await expect(badge).toBeHidden();
});

test("Event Log flood remains capped", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.goto("/");

  await page.evaluate(() => {
    for (let i = 0; i < 1200; i += 1) console.log(`flood-${i}`);
  });

  await page.locator("#eventLogBtn").click();
  await expect(page.locator("#eventLogSummary")).toContainText("500 events");
});

test("an open Event Log updates live: new entries on top, repeats coalesce, rows are kept", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.goto("/");
  await page.locator("#eventLogBtn").click();

  const rows = page.locator("#eventLogList .event-log-row");
  await page.evaluate(() => console.log("live-first"));
  await expect(rows.first()).toContainText("live-first");
  // Tag a row node: an incremental update must leave it in place.
  await rows.first().evaluate((row) => { row.__kept = true; });

  await page.evaluate(() => console.log("live-second"));
  await expect(rows.first()).toContainText("live-second");
  expect(await rows.nth(1).evaluate((row) => row.__kept === true && row.textContent.includes("live-first"))).toBe(true);

  // A repeat moves the existing entry back to the top with its count.
  await page.evaluate(() => console.log("live-first"));
  await expect(rows.first()).toContainText("live-first");
  await expect(rows.first().locator(".event-log-count")).toHaveText("x2");
  await expect(page.locator("#eventLogList .event-log-row", { hasText: "live-first" })).toHaveCount(1);

  // Entries hidden by the source filter don't show up.
  await page.locator("#eventLogSourceFilter").selectOption("startup");
  await page.evaluate(() => console.log("live-filtered-out"));
  await expect(page.locator("#eventLogList")).not.toContainText("live-filtered-out");
  await page.locator("#eventLogSourceFilter").selectOption("all");

  // A flood while open stays capped at the store's 500 entries.
  await page.evaluate(() => {
    for (let i = 0; i < 1200; i += 1) console.log(`live-flood-${i}`);
  });
  await expect(page.locator("#eventLogSummary")).toContainText("500 events");
  await expect(rows).toHaveCount(500);
  await expect(rows.first()).toContainText("live-flood-1199");
});

test("Rediagnose USB and Parity Report open Health & Diagnostics whatever the result", async ({ page }) => {
  await installTauriMock(page, "valid");
  // A passing diagnostics run, which the automatic run on USB select leaves
  // collapsed, and a failing parity report run.
  await page.addInitScript(() => {
    const invoke = window.__TAURI__.core.invoke;
    window.__TAURI__.core.invoke = async (command, payload) => {
      const result = await invoke(command, payload);
      if (command === "run_usb_diagnostics") result.data = { ...result.data, overallStatus: "PASS" };
      if (command === "run_usb_parity_report" && window.__failParity) {
        return { ok: false, error: { code: "PARITY_FAILED", message: "parity failed" } };
      }
      return result;
    };
  });
  await page.goto("/");

  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();
  const usbHealthCard = page.locator("#usbHealthCard");
  await expect(page.locator("#diagOverallStatus")).toHaveText("PASS");
  await expect(usbHealthCard).not.toHaveAttribute("open");

  await page.locator("#reDiagnoseBtn").click();
  await expect(usbHealthCard).toHaveAttribute("open");
  await expect(usbHealthCard).not.toHaveClass(/is-loading/);

  await usbHealthCard.evaluate((node) => { node.open = false; });
  await page.locator("#runUsbParityBtn").click();
  await expect(usbHealthCard).toHaveAttribute("open");
  await expect(page.locator("#diagSections .diag-intro h3")).toHaveText("Parity Report");

  await usbHealthCard.evaluate((node) => { node.open = false; });
  await page.evaluate(() => { window.__failParity = true; });
  await page.locator("#runUsbParityBtn").click();
  await expect(usbHealthCard).toHaveAttribute("open");
});

test("Diagnostics and parity render without warning panel", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.goto("/");

  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();
  const usbHealthCard = page.locator("#usbHealthCard");
  await usbHealthCard.evaluate((node) => {
    node.open = true;
  });
  await page.locator("#reDiagnoseBtn").click();
  await expect(page.locator("#diagSections .diag-overview-title")).toHaveText([
    "Older Pioneer players",
    "Waveforms & beat grids",
    "Denon / Numark players"
  ]);
  await expect(page.locator("#diagSections .diag-overview-item.diag-check-na .diag-indicator")).toHaveText("–");
  const technical = page.locator("#diagSections .diag-technical");
  await expect(technical).not.toHaveAttribute("open");
  await technical.locator("summary").click();
  await expect(page.locator("#diagSections .diag-group-title")).toHaveText(["Databases", "Library"]);
  await expect(page.locator("#diagSections .diag-group").first().locator(".diag-section-title")).toHaveText(["PDB", "Engine DJ"]);
  await expect(page.locator("#diagSections")).toContainText("Overall resolution");
  await expect(page.locator("#diagSections")).toContainText("Kept up to date");
  await expect(page.locator("#diagSections")).not.toContainText("Playlists compared");
  await expect(page.locator("#diagRawWarnings")).toHaveCount(0);
  await expect(page.locator("#eventLogDot")).toBeHidden();
  // Errors the mock logged take the badge's place; see them so the dot can show.
  await page.locator("#eventLogBtn").click();
  await page.locator('.nav-item[data-view="usb"]').click();

  await page.locator("#runUsbParityBtn").click();
  // Parity report: heading, plain-language rows, then the collapsed technical details
  await expect(page.locator("#diagSections .diag-intro h3")).toHaveText("Parity Report");
  // The parity result gets its own badge; the health badge stays as diagnostics left it.
  await expect(page.locator("#diagParityStatus")).toHaveText("Parity FAIL");
  await expect(page.locator("#diagOverallStatus")).toHaveText("WARN");
  await expect(page.locator("#diagSections .diag-overview-title")).toHaveText([
    "Playlists",
    "Track details",
    "Music files"
  ]);
  await expect(page.locator("#diagSections .diag-overview-item.diag-check-fail")).toHaveCount(2);
  const parityTechnical = page.locator("#diagSections .diag-technical");
  await expect(parityTechnical).not.toHaveAttribute("open");
  await parityTechnical.locator("summary").click();
  await expect(page.locator("#diagSections .diag-group-title")).toHaveText(["Comparison", "Playlists compared"]);
  await expect(parityTechnical).toContainText("PDB metadata completeness");
  // Diagnostics content should not leak into parity
  await expect(page.locator("#diagSections")).not.toContainText("Databases");
  await expect(page.locator("#diagSections")).not.toContainText("Older Pioneer players");
  // Per-playlist comparison table
  const playlistRows = parityTechnical.locator("tbody");
  await expect(playlistRows).toContainText("Warmup");
  await expect(playlistRows).toContainText("path mismatch 1");
  await expect(playlistRows).toContainText("dict issues 1");
  await expect(playlistRows).toContainText("PDB gaps 1");
  // A check that links to the Event Log puts a dot on the header button until the log is opened.
  const eventLogDot = page.locator("#eventLogDot");
  await expect(eventLogDot).toBeVisible();
  await parityTechnical.locator(".diag-log-link").click();
  await expect(page.locator("#panel-event-log")).toHaveClass(/active/);
  await expect(eventLogDot).toBeHidden();
  await page.locator('.nav-item[data-view="usb"]').click();
  // Back to diagnostics: the parity badge goes away with the parity report.
  await page.locator("#reDiagnoseBtn").click();
  await expect(page.locator("#diagSections .diag-intro")).toHaveCount(0);
  await expect(page.locator("#diagParityStatus")).toBeEmpty();
  await expect(page.locator("#diagRawWarnings")).toHaveCount(0);
});

test("Repair preview shows the backend's rows and locks structural-prerequisite fix checkboxes; the Engine fix turns the setting on", async ({ page }) => {
  await installTauriMock(page, "valid");
  await countPlaylistFetches(page);
  await page.goto("/");

  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();

  // Load playlists first -- a successful repair apply must clear them, since
  // an applied fix may have rewritten the PDB playlist tree they came from.
  await page.locator('.nav-item[data-view="usb-playlists"]').click();
  await expect(page.locator('[data-usb-playlist="usb-1"]')).toBeVisible();
  await page.locator('.nav-item[data-view="usb"]').click();

  const usbHealthCard = page.locator("#usbHealthCard");
  await usbHealthCard.evaluate((node) => {
    node.open = true;
  });
  await page.locator("#reDiagnoseBtn").click();
  await page.locator("#previewRepairsBtn").click();

  // Plain-language headline and rows first; the fix list is collapsed under
  // Technical details.
  await expect(page.locator("#diagRepairSummary")).toHaveText(/repairs 4 problems/);
  await expect(page.locator("#diagRepairOverview .diag-overview-title")).toHaveText([
    "Older Pioneer players",
    "Denon / Numark players",
    "Needs your attention"
  ]);
  await expect(page.locator("#diagRepairOverview .diag-overview-item.diag-check-fail")).toHaveCount(1);
  const repairTechnical = page.locator("#diagRepairTechnical");
  await expect(repairTechnical).not.toHaveAttribute("open");
  await expect(page.locator("#diagRepairFixes")).toBeHidden();
  await repairTechnical.locator("summary").click();

  const truncatedChainCheckbox = page
    .locator("#diagRepairFixes li", { hasText: "Repair Truncated Table Chain" })
    .locator(".diag-repair-fix-check");
  const tornGrowthCheckbox = page
    .locator("#diagRepairFixes li", { hasText: "Repair Torn Growth Pages" })
    .locator(".diag-repair-fix-check");
  const controlCheckbox = page
    .locator("#diagRepairFixes li", { hasText: "Repair PDB History/Columns Page Footer Shape" })
    .locator(".diag-repair-fix-check");

  await expect(truncatedChainCheckbox).toBeChecked();
  await expect(truncatedChainCheckbox).toBeDisabled();
  await expect(tornGrowthCheckbox).toBeChecked();
  await expect(tornGrowthCheckbox).toBeDisabled();

  // Control: an ordinary (non-prerequisite) fix stays interactive.
  await expect(controlCheckbox).toBeChecked();
  await expect(controlCheckbox).toBeEnabled();
  await controlCheckbox.uncheck();
  await expect(controlCheckbox).not.toBeChecked();

  // Items whose details the Event Log lists link to it; the others don't. A row
  // that can't be chosen has no checkbox.
  const malformedItem = page.locator("#diagRepairFixes li", { hasText: "malformed USBANLZ" });
  await expect(malformedItem.locator(".diag-repair-fix-check")).toHaveCount(0);
  await expect(malformedItem.locator(".diag-repair-fix-meta")).toHaveText("✗ preview-only · safe");
  await expect(malformedItem.locator(".diag-log-link")).toBeVisible();
  await expect(page.locator("#diagRepairFixes li", { hasText: "Repair PDB History/Columns Page Footer Shape" }).locator(".diag-log-link")).toHaveCount(0);

  await expect(page.locator("#exportEngineLibraryCheckbox")).not.toBeChecked();
  const engineFix = page.locator("#diagRepairFixes li", { hasText: "Keep Engine DJ Library Up to Date" });
  await expect(engineFix.locator(".diag-repair-fix-check")).toBeChecked();
  await expect(engineFix.locator(".diag-repair-fix-meta")).toContainText("destructive");

  // The repair response carries fresh diagnostics; assert on the settled state
  // rather than the transient status text.
  await page.locator("#applyRepairsBtn").click();
  await expect(page.locator("#exportEngineLibraryCheckbox")).toBeChecked();
  expect(await page.evaluate(() => window.__playlistFetches)).toBe(1);

  // The repair cleared the list, so opening the page reads it again.
  await page.locator('.nav-item[data-view="usb-playlists"]').click();
  await expect(page.locator('[data-usb-playlist="usb-1"]')).toBeVisible();
  expect(await page.evaluate(() => window.__playlistFetches)).toBe(2);
});

test("a repair whose diagnostics re-run failed still applies the export status it returns", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.addInitScript(() => {
    const invoke = window.__TAURI__.core.invoke;
    window.__statusRefreshes = 0;
    window.__TAURI__.core.invoke = async (command, payload) => {
      if (command === "list_playlists") {
        return { ok: true, data: { items: [{ id: "pl-1", name: "Local Set", source: "local" }] } };
      }
      if (command === "repair_usb_diagnostics" && payload?.request?.apply) {
        return {
          ok: true,
          data: {
            appliedFixes: payload.request.selectedFixIds,
            usbContentChanged: true,
            statusText: "Repair apply complete: 1 applied",
            playlistUsbExportStatus: [{ playlistId: "pl-1", playlistName: "Local Set", usbSelected: true, sameNameExistsOnUsb: true, markLabel: "On this USB" }]
          }
        };
      }
      if (command === "refresh_playlist_export_status") window.__statusRefreshes += 1;
      return invoke(command, payload);
    };
  });
  await page.goto("/");
  await connectUsb(page);
  const mark = page.locator('#navPlaylistList .nav-playlist-item[data-playlist-id="pl-1"] .nav-playlist-status');
  await expect(mark).not.toHaveClass(/exported/);
  // Selecting the USB refreshed the status. The repair must not need a second command.
  await expect.poll(() => page.evaluate(() => window.__statusRefreshes)).toBeGreaterThan(0);
  const refreshesBeforeRepair = await page.evaluate(() => window.__statusRefreshes);

  await page.locator("#usbHealthCard").evaluate((node) => { node.open = true; });
  await page.locator("#previewRepairsBtn").click();
  await page.locator("#applyRepairsBtn").click();
  await expect(page.locator("#statusText")).toContainText("Repair apply complete");
  await expect(page.locator("#statusText")).not.toContainText("Diagnostics refreshed");
  // The same USB is still selected, so its diagnostics panel stays.
  await expect(page.locator("#usbDiagnosticsCard")).toBeVisible();

  await expect(mark).toHaveClass(/exported/);
  await expect(mark).toHaveAttribute("aria-label", "On this USB");
  expect(await page.evaluate(() => window.__statusRefreshes)).toBe(refreshesBeforeRepair);
});

async function dragViaHandle(page, fromHandleSelector, toRowSelector, dropNearTop) {
  const source = page.locator(fromHandleSelector);
  const target = page.locator(toRowSelector);
  const sourceBox = await source.boundingBox();
  const targetBox = await target.boundingBox();
  await page.mouse.move(sourceBox.x + sourceBox.width / 2, sourceBox.y + sourceBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(sourceBox.x + sourceBox.width / 2, sourceBox.y + sourceBox.height / 2 + 10, { steps: 5 });
  const targetY = dropNearTop ? targetBox.y + targetBox.height * 0.25 : targetBox.y + targetBox.height * 0.75;
  await page.mouse.move(targetBox.x + targetBox.width / 2, targetY, { steps: 10 });
  await page.mouse.up();
}

test("USB playlist drag-and-drop reorder persists the new order", async ({ page }) => {
  await installTauriMock(page, "reorder");
  await countPlaylistFetches(page);
  await page.goto("/");

  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();

  // Open a diagnostics report first -- a successful reorder must clear it,
  // since it no longer reflects the (now stale) playlist order it reported on.
  const usbHealthCard = page.locator("#usbHealthCard");
  await usbHealthCard.evaluate((node) => { node.open = true; });
  await page.locator("#reDiagnoseBtn").click();
  await expect(page.locator("#diagSections")).not.toBeEmpty();

  await page.locator('.nav-item[data-view="usb-playlists"]').click();

  const rows = page.locator("#usbPlaylists [data-usb-playlist]");
  await expect(rows).toHaveCount(3);
  await expect(rows.nth(0)).toContainText("Alpha");
  await expect(rows.nth(1)).toContainText("Bravo");
  await expect(rows.nth(2)).toContainText("Charlie");
  await expect(rows.nth(0)).toHaveClass(/active/);
  await expect(page.locator("#usbPlaylistTracks .track-grid-row")).toHaveCount(1);

  // Drag "Alpha" (row 0) to below "Charlie" (row 2), moving it to the end.
  await dragViaHandle(
    page,
    '#usbPlaylists li:nth-child(1) [data-usb-drag-handle]',
    '#usbPlaylists li:nth-child(3)',
    false
  );

  await expect(page.locator("#statusText")).toContainText("Playlist order saved");
  // A write that didn't reach the stick isn't shown as a clean save.
  await expect(page.locator("#statusText .status-warning-link")).toHaveText("(1 warning)");
  await expect(rows.nth(0)).toContainText("Bravo");
  await expect(rows.nth(1)).toContainText("Charlie");
  await expect(rows.nth(2)).toContainText("Alpha");
  await expect(page.locator("#diagSections")).toBeEmpty();
  await expect(page.locator("#diagOverallStatus")).toBeEmpty();

  const reorderCalls = await page.evaluate(() => window.__reorderCalls);
  expect(reorderCalls.length).toBeGreaterThan(0);
  const lastCall = reorderCalls[reorderCalls.length - 1];
  expect(lastCall).toEqual(["usb-pl-2", "usb-pl-3", "usb-pl-1"]);

  // Nothing is read again: the moved playlist stays open.
  await expect(rows.nth(2)).toHaveClass(/active/);
  await expect(page.locator("#usbPlaylistHeading .track-list-heading-name")).toHaveText("Alpha");
  await expect(page.locator("#usbPlaylistTracks .track-grid-row")).toContainText("Track A");
  expect(await page.evaluate(() => window.__playlistFetches)).toBe(1);
  expect(await page.evaluate(() => window.__trackFetches)).toBe(1);

  // A click opens the row clicked, in its new place.
  await rows.nth(0).click();
  await expect(page.locator("#statusText")).toContainText("USB playlist selected: Bravo");
  await expect(page.locator("#usbPlaylistHeading .track-list-heading-name")).toHaveText("Bravo");
  await expect(page.locator("#usbPlaylistTracks .track-grid-row")).toContainText("Track B");
});

test("a failed USB playlist reorder restores the order and keeps the playlist open", async ({ page }) => {
  await installTauriMock(page, "reorder");
  await page.addInitScript(() => {
    const invoke = window.__TAURI__.core.invoke;
    window.__TAURI__.core.invoke = async (command, payload) => {
      if (command === "reorder_usb_playlists") return { ok: false, error: { code: "IO", message: "write failed" } };
      return invoke(command, payload);
    };
  });
  await countPlaylistFetches(page);
  await page.goto("/");

  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();
  await page.locator('.nav-item[data-view="usb-playlists"]').click();

  const rows = page.locator("#usbPlaylists [data-usb-playlist]");
  await expect(rows.nth(0)).toHaveClass(/active/);
  await expect(page.locator("#usbPlaylistTracks .track-grid-row")).toHaveCount(1);

  await dragViaHandle(
    page,
    '#usbPlaylists li:nth-child(1) [data-usb-drag-handle]',
    '#usbPlaylists li:nth-child(3)',
    false
  );

  await expect(page.locator("#statusText")).toContainText("Failed to save playlist order: write failed");
  await expect(rows.nth(0)).toContainText("Alpha");
  await expect(rows.nth(1)).toContainText("Bravo");
  await expect(rows.nth(2)).toContainText("Charlie");
  await expect(rows.nth(0)).toHaveClass(/active/);
  await expect(page.locator("#usbPlaylistHeading .track-list-heading-name")).toHaveText("Alpha");
  await expect(page.locator("#usbPlaylistTracks .track-grid-row")).toContainText("Track A");
  expect(await page.evaluate(() => window.__playlistFetches)).toBe(2);
});

test("USB playlist drag that does not change position skips persisting order", async ({ page }) => {
  await installTauriMock(page, "reorder");
  await page.goto("/");

  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();
  await page.locator('.nav-item[data-view="usb-playlists"]').click();

  const rows = page.locator("#usbPlaylists [data-usb-playlist]");
  await expect(rows).toHaveCount(3);

  // Drag row 0's handle a few pixels and drop back within the same row.
  const handle = page.locator('#usbPlaylists li:nth-child(1) [data-usb-drag-handle]');
  const box = await handle.boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2 + 2, { steps: 3 });
  await page.mouse.up();

  await expect(rows.nth(0)).toContainText("Alpha");
  const reorderCalls = await page.evaluate(() => window.__reorderCalls);
  expect(reorderCalls).toEqual([]);
});

test("USB playlist drag auto-scrolls the list when held near its bottom edge", async ({ page }) => {
  await installTauriMock(page, "reorder-scroll");
  await page.goto("/");

  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();
  await page.locator('.nav-item[data-view="usb-playlists"]').click();

  const list = page.locator("#usbPlaylists");
  await expect(page.locator("#usbPlaylists [data-usb-playlist]")).toHaveCount(40);

  const initialScrollTop = await list.evaluate((node) => node.scrollTop);
  expect(initialScrollTop).toBe(0);

  // The progress footer briefly overlaps the bottom of the viewport after a
  // refresh -- wait for it to clear so it doesn't swallow the drag events
  // meant for the list underneath it.
  await expect(page.locator("#progressFooter")).not.toHaveClass(/active/, { timeout: 3000 });

  const handle = page.locator('#usbPlaylists li:nth-child(1) [data-usb-drag-handle]');
  const handleBox = await handle.boundingBox();
  const listBox = await list.boundingBox();

  await page.mouse.move(handleBox.x + handleBox.width / 2, handleBox.y + handleBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(handleBox.x + handleBox.width / 2, handleBox.y + handleBox.height / 2 + 10, { steps: 5 });

  // Move down toward the bottom edge of the scrollable list, then dwell
  // there -- native dragover keeps firing periodically while the pointer
  // stays over a valid drop target, which is what drives auto-scroll (see
  // dnd_autoscroll.mjs).
  const endX = listBox.x + listBox.width / 2;
  const endY = listBox.y + listBox.height - 4;
  await page.mouse.move(endX, endY, { steps: 15 });
  for (let i = 0; i < 15; i += 1) {
    await page.mouse.move(endX, endY - (i % 2), { steps: 1 });
    await page.waitForTimeout(80);
  }

  await page.waitForFunction(
    () => document.querySelector("#usbPlaylists").scrollTop > 0,
    null,
    { timeout: 3000 }
  );
  const scrolledTop = await list.evaluate((node) => node.scrollTop);
  await page.mouse.up();

  expect(scrolledTop).toBeGreaterThan(0);
});

test("USB toggle race ends in deterministic final state, with nothing left of the previous USB", async ({ page }) => {
  await installTauriMock(page, "toggle-usb");
  await page.addInitScript(() => {
    const invoke = window.__TAURI__.core.invoke;
    window.__TAURI__.core.invoke = async (command, payload) => {
      if (command === "list_playlists") {
        return { ok: true, data: { items: [{ id: "pl-1", name: "Local Set", source: "local" }] } };
      }
      const playlistUsbExportStatus = [{ playlistId: "pl-1", playlistName: "Local Set", usbSelected: true, sameNameExistsOnUsb: true, markLabel: "On this USB" }];
      if (command === "fetch_usb_playlists") {
        const backend = window.__e2eBackend;
        return backend.ok(backend.usbPlaylists({ items: [{ id: "usb-1", name: "Warmup", tracks: [] }], playlistUsbExportStatus }));
      }
      const result = await invoke(command, payload);
      if (command === "run_usb_diagnostics") {
        result.data = { ...result.data, playlistUsbExportStatus };
      }
      return result;
    };
  });
  await page.goto("/");

  const usbPlaylistsNav = page.locator('.nav-sub-item[data-view="usb-playlists"]');
  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click(); // valid (1)
  await expect(usbPlaylistsNav).toHaveClass(/revealed/);

  await page.locator("#selectUsbFolderBtn").click(); // invalid (2)
  await page.locator("#selectUsbFolderBtn").click(); // valid (3)
  await usbPlaylistsNav.click();
  await expect(page.locator("#panel-usb-playlists")).toHaveClass(/active/);
  await expect(page.locator('[data-usb-playlist="usb-1"]')).toBeVisible();
  await expect(page.locator("#usbCountsText")).not.toBeEmpty();
  await page.locator('.nav-sub-item[data-view="usb-history"]').click();
  await expect(page.locator("#historyList")).toHaveAttribute("data-loaded");
  const mark = page.locator('#navPlaylistList .nav-playlist-item[data-playlist-id="pl-1"] .nav-playlist-status');
  await expect(mark).toHaveClass(/exported/);
  await page.locator('.nav-item[data-view="usb"]').click();
  await expect(page.locator("#usbDiagnosticsCard")).toBeVisible();
  await page.locator("#selectUsbFolderBtn").click(); // invalid (4)

  await expect(usbPlaylistsNav).not.toHaveClass(/revealed/);
  await expect(page.locator("#panel-usb")).toHaveClass(/active/);
  // The lists are cleared and unread, so they show nothing rather than "no playlists".
  await expect(page.locator("#usbPlaylists")).toBeEmpty();
  await expect(page.locator("#usbPlaylists")).not.toHaveAttribute("data-loaded");
  await expect(page.locator("#usbCountsText")).toBeEmpty();
  await expect(page.locator("#historyList")).toBeEmpty();
  await expect(page.locator("#historyList")).not.toHaveAttribute("data-loaded");
  await expect(page.locator("#historyCountsText")).toBeEmpty();
  await expect(mark).not.toHaveClass(/exported/);
  await expect(page.locator("#usbDiagnosticsCard")).toBeHidden();
});

test("a USB list read overtaken by a USB change drops its late response", async ({ page }) => {
  await installTauriMock(page, "toggle-usb");
  await page.addInitScript(() => {
    const invoke = window.__TAURI__.core.invoke;
    const backend = window.__e2eBackend;
    const playlist = (name) => ({ id: name, name, source: "mock-tauri", tracks: [] });
    const released = new Promise((resolve) => { window.__releaseFirstRead = resolve; });
    let fetches = 0;
    window.__TAURI__.core.invoke = async (command, payload) => {
      if (command !== "fetch_usb_playlists") return invoke(command, payload);
      fetches += 1;
      if (fetches > 1) return backend.ok(backend.usbPlaylists({ items: [playlist("Fresh")] }));
      await released;
      return backend.ok(backend.usbPlaylists({ items: [playlist("Stale")] }));
    };
  });
  await page.goto("/");

  const usbPlaylistsNav = page.locator('.nav-sub-item[data-view="usb-playlists"]');
  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click(); // valid
  await usbPlaylistsNav.click(); // the first read starts and stays open

  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#selectUsbFolderBtn").click(); // invalid
  await expect(usbPlaylistsNav).not.toHaveClass(/revealed/);
  await page.locator("#selectUsbFolderBtn").click(); // valid
  await expect(usbPlaylistsNav).toHaveClass(/revealed/);
  await usbPlaylistsNav.click();
  await expect(page.locator('[data-usb-playlist="Fresh"]')).toBeVisible();

  await page.evaluate(async () => {
    window.__releaseFirstRead();
    await new Promise((resolve) => setTimeout(resolve, 50));
  });
  await expect(page.locator('[data-usb-playlist="Fresh"]')).toBeVisible();
  await expect(page.locator('[data-usb-playlist="Stale"]')).toHaveCount(0);
});

test("a job started right after another finished keeps its progress footer", async ({ page }) => {
  await installTauriMock(page, "progress-race");
  await page.goto("/");

  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();
  await page.locator('.nav-item[data-view="usb-playlists"]').click();
  // The read finished; its footer is due to hide 1.2s from now.
  await expect(page.locator("#progressText")).toContainText("Done");

  await page.locator("#addPlaylistBtn").click();
  await page.locator(".nav-new-input").fill("Race");
  await page.locator(".nav-new-input").press("Enter");
  await expect(page.locator("#progressText")).toContainText("Saving playlist");

  // Past the read's hide delay, the still-running save keeps the footer.
  await page.waitForTimeout(1500);
  await expect(page.locator("#progressFooter")).toHaveClass(/active/);
  await expect(page.locator("#progressText")).toContainText("Saving playlist");
});

test("USB audio formats: ticks and drag order are saved, and the last ticked format can't be unticked", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.goto("/");

  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();

  const chips = page.locator("#usbAudioFormatsList .usb-cfg-audio-format-chip");
  const names = page.locator("#usbAudioFormatsList .usb-cfg-audio-format-name");
  await expect(page.locator("#usbAudioFormatsRow")).toBeVisible();
  await expect(names).toHaveText(["FLAC", "ALAC", "AIFF", "WAV", "MP3", "AAC"]);
  const boxes = page.locator("#usbAudioFormatsList .usb-cfg-audio-format-check");
  for (let i = 0; i < 6; i += 1) await expect(boxes.nth(i)).toBeChecked();

  const lastCall = () => page.evaluate(() => window.__usbAudioFormatCalls.at(-1));

  await boxes.nth(0).uncheck();
  await expect.poll(lastCall).toEqual([
    { format: "flac", allowed: false },
    { format: "alac", allowed: true },
    { format: "aiff", allowed: true },
    { format: "wav", allowed: true },
    { format: "mp3", allowed: true },
    { format: "aac", allowed: true },
  ]);

  // Untick all but AAC: its box is then the last ticked one and locks.
  for (const index of [1, 2, 3, 4]) {
    await boxes.nth(index).uncheck();
    await expect(boxes.nth(index)).not.toBeChecked();
  }
  await expect(boxes.nth(5)).toBeChecked();
  await expect(boxes.nth(5)).toBeDisabled();
  await expect(chips.nth(5).locator(".usb-cfg-audio-format-label")).toHaveAttribute("data-tooltip", "At least one format stays ticked");
  await boxes.nth(4).check();
  await expect(boxes.nth(5)).toBeEnabled();

  // Drag MP3 (chip 4) to the left half of FLAC (chip 0): it goes first.
  const handle = chips.nth(4).locator("[data-usb-audio-format-drag-handle]");
  const sourceBox = await handle.boundingBox();
  const targetBox = await chips.nth(0).boundingBox();
  await page.mouse.move(sourceBox.x + sourceBox.width / 2, sourceBox.y + sourceBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(sourceBox.x + sourceBox.width / 2 - 10, sourceBox.y + sourceBox.height / 2, { steps: 5 });
  await page.mouse.move(targetBox.x + targetBox.width * 0.25, targetBox.y + targetBox.height / 2, { steps: 10 });
  await page.mouse.up();

  await expect(names).toHaveText(["MP3", "FLAC", "ALAC", "AIFF", "WAV", "AAC"]);
  await expect.poll(async () => (await lastCall()).map((item) => item.format))
    .toEqual(["mp3", "flac", "alac", "aiff", "wav", "aac"]);
});

async function connectUsb(page) {
  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();
  await expect(page.locator("#diagOverallStatus")).toHaveText("WARN");
}

test("a running USB job locks the USB selection and export until it ends; other jobs don't", async ({ page }) => {
  await installTauriMock(page, "valid");
  await listenForJobEvents(page);
  await page.addInitScript(() => {
    const invoke = window.__TAURI__.core.invoke;
    window.__picks = 0;
    window.__validates = 0;
    window.__TAURI__.core.invoke = async (command, payload) => {
      if (command === "list_playlists") {
        return { ok: true, data: { items: [{ id: "pl-1", name: "Local Set", source: "local" }] } };
      }
      // Another drive shows up as recent once the first is connected.
      if (command === "list_usb_devices" && window.__validates > 0) {
        return { ok: true, data: { items: [{ id: "dev-1", rootPath: "/Volumes/OTHER", label: "Other", mounted: false }] } };
      }
      if (command === "pick_usb_folder") window.__picks += 1;
      if (command === "validate_usb_root") window.__validates += 1;
      return invoke(command, payload);
    };
  });
  await page.goto("/");
  await connectUsb(page);
  await page.locator('#navPlaylistList .nav-playlist-item[data-playlist-id="pl-1"]').click();
  await page.locator('.nav-item[data-view="usb"]').click();

  const select = page.locator("#selectUsbFolderBtn");
  const recent = page.locator('#usbRecentList button[data-usb-recent-path="/Volumes/OTHER"]');
  const exportBtn = page.locator("#exportPlaylistBtn");
  const counts = () => page.evaluate(() => ({ picks: window.__picks, validates: window.__validates }));
  const emit = (jobType, event) => page.evaluate(
    ([jobType, event]) => window.__emitJobEvent({ jobId: `job-${jobType}`, jobType, stage: "test", event, percent: 0, message: "USB: working" }),
    [jobType, event]
  );
  await expect(recent).toBeEnabled();
  await expect(exportBtn).toBeEnabled();

  for (const [jobType, endEvent] of [
    ["usb_read", "job.completed"],
    ["usb_write", "job.failed"],
    ["diagnostics", "job.completed"],
    ["export", "job.completed"]
  ]) {
    await emit(jobType, "job.started");
    await expect(select).toBeDisabled();
    await expect(select).toHaveAttribute("title", /wait/i);
    await expect(recent).toBeDisabled();
    await expect(exportBtn).toBeDisabled();

    // A click already on its way when the job starts changes nothing either.
    const before = await counts();
    for (const selector of ["#selectUsbFolderBtn", '#usbRecentList button[data-usb-recent-path="/Volumes/OTHER"]']) {
      await page.evaluate((selector) => {
        document.querySelector("#statusText").textContent = "";
        const button = document.querySelector(selector);
        button.disabled = false;
        button.click();
      }, selector);
      await expect(page.locator("#statusText")).toContainText(/wait/i);
    }
    expect(await counts()).toEqual(before);
    await page.evaluate(() => { document.querySelector("#selectUsbFolderBtn").disabled = true; });

    await emit(jobType, endEvent);
    await expect(select).toBeEnabled();
    await expect(recent).toBeEnabled();
    await expect(exportBtn).toBeEnabled();
  }

  await emit("analysis", "job.started");
  await page.waitForTimeout(100);
  await expect(select).toBeEnabled();
  await expect(exportBtn).toBeEnabled();
});

test("the drive name badge does nothing with no USB selected", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.goto("/");
  await expect(page.locator("#usbNameBadgeLabel")).toHaveText("Not connected");
  // A real mouse still lands on it: Playwright only refuses because of aria-disabled.
  await page.locator("#usbNameBadgeLabel").click({ force: true });
  await page.waitForTimeout(100);
  await expect(page.locator("#driveNameOverlay")).toBeHidden();
});

test("a stored USB isn't shown as selected, or read, until it's validated", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.addInitScript(() => {
    window.localStorage.setItem("djusbtkit.usbRoot", "/Volumes/USB-TEST");
    const invoke = window.__TAURI__.core.invoke;
    window.__validates = 0;
    window.__TAURI__.core.invoke = async (command, payload) => {
      if (command === "validate_usb_root") window.__validates += 1;
      return invoke(command, payload);
    };
  });
  await page.goto("/");
  await page.locator('.nav-item[data-view="usb"]').click();

  await expect(page.locator("#usbRootPathText")).toHaveText("No USB selected");
  await expect(page.locator("#usbSelectedControls")).toBeHidden();
  await expect(page.locator("#usbInitRow")).toBeHidden();
  await expect(page.locator('.nav-sub-item[data-view="usb-playlists"]')).not.toHaveClass(/revealed/);
  expect(await page.evaluate(() => window.__validates)).toBe(0);
});

test("selecting a USB lets its folder and the library's into the asset scope", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.addInitScript(() => {
    const invoke = window.__TAURI__.core.invoke;
    window.__assetPaths = [];
    window.__TAURI__.core.invoke = async (command, payload) => {
      if (command === "get_library_sources") return window.__e2eBackend.ok(window.__e2eBackend.librarySources(["/music"]));
      if (command === "allow_asset_paths") window.__assetPaths.push(payload.paths);
      return invoke(command, payload);
    };
  });
  await page.goto("/");
  await connectUsb(page);
  expect((await page.evaluate(() => window.__assetPaths)).at(-1)).toEqual(["/music", "/Volumes/USB-TEST"]);
});

test("restoring a backup: cancel and failure change nothing; success clears the report and lists and applies the export status", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.addInitScript(() => {
    const invoke = window.__TAURI__.core.invoke;
    window.__playlistReads = 0;
    window.__restores = 0;
    window.__deletes = [];
    window.__TAURI__.core.invoke = async (command, payload) => {
      if (command === "list_playlists") {
        return { ok: true, data: { items: [{ id: "pl-1", name: "Local Set", source: "local" }] } };
      }
      if (command === "list_usb_backups") {
        return { ok: true, data: { summaryText: "the summary", emptyText: "none", items: [{ timestamp: "2020-01-01_00-00-00", timeText: "the time", reasonText: "the reason", detailText: "the detail", restoreConfirmText: "Restore it?", deleteConfirmText: "Delete it?", files: ["export.pdb"], reason: "export", sizeBytes: 1024, location: "usb" }] } };
      }
      if (command === "fetch_usb_playlists") window.__playlistReads += 1;
      if (command === "delete_usb_backup") {
        window.__deletes.push(payload.request.timestamp);
        return { ok: true, data: { deleted: true, statusText: "Deleted PDB backup" } };
      }
      if (command === "restore_usb_backup") {
        window.__restores += 1;
        if (window.__failRestore) return { ok: false, error: { code: "IO", message: "disk gone" } };
        return {
          ok: true,
          data: {
            restored: true,
            statusText: "Restored eDB and PDB from backup",
            playlistUsbExportStatus: [{ playlistId: "pl-1", playlistName: "Local Set", usbSelected: true, sameNameExistsOnUsb: true, markLabel: "On this USB" }]
          }
        };
      }
      return invoke(command, payload);
    };
  });
  await page.goto("/");
  await connectUsb(page);

  const usbRow = page.locator('[data-usb-playlist="usb-1"]');
  const report = page.locator("#diagOverallStatus");
  const mark = page.locator('#navPlaylistList .nav-playlist-item[data-playlist-id="pl-1"] .nav-playlist-status');
  await page.locator('.nav-item[data-view="usb-playlists"]').click();
  await expect(usbRow).toBeVisible();
  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbBackupsBtn").click();
  const restoreBtn = page.locator(".backups-restore-btn");

  await restoreBtn.click();
  await page.locator("#confirmCancelBtn").click();
  await expect(page.locator("#confirmOverlay")).toBeHidden();
  expect(await page.evaluate(() => window.__restores)).toBe(0);

  // The row and summary show what the backend sends.
  await expect(page.locator("#backupsSummary")).toHaveText("the summary");
  await expect(page.locator(".event-log-time").first()).toHaveText("the time");
  await expect(page.locator(".event-log-source").first()).toHaveText("the reason");
  await expect(page.locator(".event-log-message").first()).toHaveText("the detail");

  // Deleting a backup leaves the report alone.
  await page.locator(".backups-delete-btn").click();
  await expect(page.locator("#confirmMessage")).toHaveText("Delete it?");
  await page.locator("#confirmOkBtn").click();
  await expect(page.locator("#statusText")).toContainText("Deleted PDB backup");
  expect(await page.evaluate(() => window.__deletes)).toEqual(["2020-01-01_00-00-00"]);
  await expect(report).toHaveText("WARN");

  await page.evaluate(() => { window.__failRestore = true; });
  await restoreBtn.click();
  await expect(page.locator("#confirmMessage")).toHaveText("Restore it?");
  await page.locator("#confirmOkBtn").click();
  await expect(page.locator("#statusText")).toContainText("Restore failed: disk gone");
  await expect(usbRow).toHaveCount(1);
  await expect(report).toHaveText("WARN");
  await expect(mark).not.toHaveClass(/exported/);

  await page.evaluate(() => { window.__failRestore = false; });
  await restoreBtn.click();
  await page.locator("#confirmOkBtn").click();
  await expect(page.locator("#statusText")).toContainText("Restored eDB and PDB from backup");
  await expect(report).toHaveText("");
  await expect(usbRow).toHaveCount(0);
  await expect(mark).toHaveClass(/exported/);
  await expect(mark).toHaveAttribute("aria-label", "On this USB");

  // Opening the page reads the restored list.
  await page.locator('.nav-item[data-view="usb-playlists"]').click();
  await expect(usbRow).toBeVisible();
  expect(await page.evaluate(() => window.__playlistReads)).toBe(2);
});

test("applying repairs that change nothing keeps the loaded USB playlists and shows the refreshed report", async ({ page }) => {
  await installTauriMock(page, "valid");
  await countPlaylistFetches(page);
  await page.addInitScript(() => {
    const invoke = window.__TAURI__.core.invoke;
    window.__TAURI__.core.invoke = async (command, payload) => {
      if (command === "repair_usb_diagnostics" && payload?.request?.apply) {
        return {
          ok: true,
          data: { failedFixes: payload.request.selectedFixIds, usbContentChanged: false, statusText: "Repair apply complete: 0 applied", diagnostics: { overallStatus: "FAIL" } }
        };
      }
      return invoke(command, payload);
    };
  });
  await page.goto("/");
  await connectUsb(page);
  await page.locator('.nav-item[data-view="usb-playlists"]').click();
  await expect(page.locator("#usbPlaylistTracks .track-grid-row")).toHaveCount(1);
  await page.locator('.nav-item[data-view="usb"]').click();

  await page.locator("#usbHealthCard").evaluate((node) => { node.open = true; });
  await page.locator("#previewRepairsBtn").click();
  await page.locator("#applyRepairsBtn").click();
  await expect(page.locator("#statusText")).toContainText("Repair apply complete: 0 applied");
  await expect(page.locator("#diagOverallStatus")).toHaveText("FAIL");

  // Coming back keeps the open playlist and its table, with no new fetch.
  await page.locator('.nav-item[data-view="usb-playlists"]').click();
  await expect(page.locator('[data-usb-playlist="usb-1"]')).toHaveClass(/active/);
  await expect(page.locator("#usbPlaylistHeading .track-list-heading-name")).toHaveText("Warmup");
  await expect(page.locator("#usbPlaylistTracks .track-grid-row")).toHaveCount(1);
  expect(await page.evaluate(() => window.__playlistFetches)).toBe(1);
  expect(await page.evaluate(() => window.__trackFetches)).toBe(1);
});

test("a player-menu edit clears the diagnostics report only when it changed the stick", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.addInitScript(() => {
    const backend = window.__e2eBackend;
    const invoke = window.__TAURI__.core.invoke;
    window.__menuUpdated = false;
    const config = (updated, statusText) => ({
      ok: true,
      data: {
        updated,
        statusText,
        currentItems: [backend.playerMenuItem(131, { name: "ARTIST" })],
        availableItems: [backend.playerMenuItem(132, { name: "PLAYLIST" })],
      }
    });
    window.__TAURI__.core.invoke = async (command, payload) => {
      if (command === "get_usb_player_menu_config") return config(false, "Player menu loaded");
      if (command === "update_usb_player_menu_config") {
        return config(window.__menuUpdated, window.__menuUpdated ? "Player menu updated" : "Player menu unchanged");
      }
      return invoke(command, payload);
    };
  });
  await page.goto("/");
  await connectUsb(page);
  const report = page.locator("#diagOverallStatus");
  const status = page.locator("#statusText");
  const menuNav = page.locator('.nav-item[data-view="usb-player-menu"]');
  const addPlaylistCategory = async () => {
    await page.locator("#usbPlayerMenuAvailable .player-menu-item[data-menu-kind='132']").click();
    await page.locator("#usbPlayerMenuAddBtn").click();
  };

  await menuNav.click();
  await addPlaylistCategory();
  await expect(status).toContainText("Player menu unchanged");
  await expect(report).toHaveText("WARN");

  await page.evaluate(() => { window.__menuUpdated = true; });
  await addPlaylistCategory();
  await expect(status).toContainText("Player menu updated");
  await expect(report).toHaveText("");
});

test("the player menu editor selects in one list at a time, and its buttons follow the selected item", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.addInitScript(() => {
    const backend = window.__e2eBackend;
    const invoke = window.__TAURI__.core.invoke;
    window.__TAURI__.core.invoke = async (command, payload) => {
      if (command === "get_usb_player_menu_config") {
        return backend.ok({
          currentItems: [
            backend.playerMenuItem(132, { name: "PLAYLIST", removable: false, canMoveDown: true }),
            backend.playerMenuItem(139, { name: "KEY", canMoveUp: true })
          ],
          availableItems: [
            backend.playerMenuItem(133, { name: "BPM", removable: false }),
            backend.playerMenuItem(134, { name: "RATING" })
          ]
        });
      }
      return invoke(command, payload);
    };
  });
  await page.goto("/");
  await connectUsb(page);
  await page.locator('.nav-item[data-view="usb-player-menu"]').click();
  const item = (list, kind) => page.locator(`#${list} .player-menu-item[data-menu-kind='${kind}']`);
  const add = page.locator("#usbPlayerMenuAddBtn");
  const remove = page.locator("#usbPlayerMenuRemoveBtn");

  await item("usbPlayerMenuAvailable", 133).click();
  await expect(add).toBeEnabled();
  await expect(remove).toBeDisabled();

  await item("usbPlayerMenuCurrent", 139).click();
  await expect(item("usbPlayerMenuCurrent", 139)).toHaveClass(/is-selected/);
  await expect(item("usbPlayerMenuAvailable", 133)).not.toHaveClass(/is-selected/);
  await expect(add).toBeDisabled();
  await expect(remove).toBeEnabled();
  await expect(page.locator("#usbPlayerMenuUpBtn")).toBeEnabled();
  await expect(page.locator("#usbPlayerMenuDownBtn")).toBeDisabled();

  // The backend says this one can't be removed: it looks locked before it's
  // selected, and can still be selected and moved.
  await expect(item("usbPlayerMenuCurrent", 132)).toHaveClass(/is-locked/);
  await expect(item("usbPlayerMenuCurrent", 139)).not.toHaveClass(/is-locked/);
  await expect(item("usbPlayerMenuAvailable", 133)).not.toHaveClass(/is-locked/);
  await item("usbPlayerMenuCurrent", 132).click();
  await expect(item("usbPlayerMenuCurrent", 132)).toHaveClass(/is-selected/);
  await expect(remove).toBeDisabled();
  await expect(page.locator("#usbPlayerMenuDownBtn")).toBeEnabled();
});

test("background colour shows nothing selected when the databases differ, and a stick that can't take one says why", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.addInitScript(() => {
    const invoke = window.__TAURI__.core.invoke;
    window.__color = { pdbColor: 6, edbColor: 6, writable: true };
    window.__TAURI__.core.invoke = async (command, payload) => {
      if (command === "get_usb_background_color") {
        if (window.__colorFails) return { ok: false, error: { code: "IO", message: "colour read failed" } };
        return { ok: true, data: window.__color };
      }
      return invoke(command, payload);
    };
  });
  await page.goto("/");
  await connectUsb(page);
  const openMenu = async (color) => {
    await page.locator('.nav-item[data-view="usb"]').click();
    await page.evaluate((color) => { window.__color = color; }, color);
    await page.locator('.nav-item[data-view="usb-player-menu"]').click();
  };
  const chips = page.locator("#usbBackgroundColorChips .bg-color-chip");
  const selected = page.locator("#usbBackgroundColorChips .bg-color-chip.is-selected");
  const note = page.locator("#usbBackgroundColorNote");

  await openMenu({ pdbColor: 6, edbColor: 6, writable: true });
  await expect(chips).toHaveCount(9);
  await expect(chips.nth(5).locator(".bg-color-chip-dot")).toHaveCSS("background-color", "rgb(69, 184, 79)");
  await expect(selected).toHaveAttribute("data-color", "6");
  await expect(chips.first()).toBeEnabled();
  await expect(note).toBeHidden();

  await openMenu({ pdbColor: 6, edbColor: 0, writable: true, noteText: "the note about two colours" });
  await expect(selected).toHaveCount(0);
  await expect(note).toHaveText("the note about two colours");

  await openMenu({ pdbColor: null, edbColor: 0, writable: false, noteText: "no t19 page" });
  await expect(chips.first()).toBeDisabled();
  await expect(note).toHaveText("no t19 page");

  // A failed read says so.
  await page.locator('.nav-item[data-view="usb"]').click();
  await page.evaluate(() => { window.__colorFails = true; });
  await page.locator('.nav-item[data-view="usb-player-menu"]').click();
  await expect(chips).toHaveCount(0);
  await expect(page.locator("#statusText")).toContainText("Failed to read the background colour: colour read failed");
});

test("choosing a background colour sets it on the USB and redraws from the answer", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.addInitScript(() => {
    const invoke = window.__TAURI__.core.invoke;
    window.__colorCalls = [];
    window.__colorUpdated = false;
    window.__TAURI__.core.invoke = async (command, payload) => {
      if (command === "get_usb_background_color") return { ok: true, data: { pdbColor: 0, edbColor: 0 } };
      if (command === "set_usb_background_color") {
        const { color } = payload.request;
        window.__colorCalls.push(payload.request);
        const now = window.__colorUpdated ? color : 0;
        const statusText = window.__colorUpdated ? "Background colour set to Green" : "Background colour unchanged";
        return { ok: true, data: { pdbColor: now, edbColor: now, updated: window.__colorUpdated, statusText } };
      }
      return invoke(command, payload);
    };
  });
  await page.goto("/");
  await connectUsb(page);
  await page.locator('.nav-item[data-view="usb-player-menu"]').click();
  const chip = (color) => page.locator(`#usbBackgroundColorChips .bg-color-chip[data-color='${color}']`);
  await expect(chip(0)).toHaveClass(/is-selected/);

  await chip(5).click();
  await expect(page.locator("#statusText")).toContainText("Background colour unchanged");
  await expect(chip(0)).toHaveClass(/is-selected/);
  await expect(page.locator("#diagOverallStatus")).toHaveText("WARN");

  await page.evaluate(() => { window.__colorUpdated = true; });
  await chip(5).click();
  await expect(page.locator("#statusText")).toContainText("Background colour set to Green");
  await expect(chip(5)).toHaveClass(/is-selected/);
  await expect(chip(0)).not.toHaveClass(/is-selected/);
  await expect(page.locator("#diagOverallStatus")).toHaveText("");
  expect(await page.evaluate(() => window.__colorCalls)).toEqual([
    { usbRoot: "/Volumes/USB-TEST", color: 5 },
    { usbRoot: "/Volumes/USB-TEST", color: 5 }
  ]);
});
