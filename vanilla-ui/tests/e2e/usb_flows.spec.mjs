import { test, expect } from "@playwright/test";

function installTauriMock(page, mode) {
  return page.addInitScript(({ mode }) => {
    window.localStorage.setItem("djusbtkit.helpSeen", "1");
    const state = {
      initialized: mode === "valid" || mode === "warning-mix" || mode === "toggle-usb" || mode === "reorder",
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
        : []
    };
    window.__reorderCalls = [];

    const diagnosticsPayload = {
      overallStatus: "WARN",
      pdbIntegrity: { title: "PDB Integrity", status: "PASS", checks: [] },
      edbAccess: { title: "Database Access", status: "PASS", checks: [] },
      contentsIntegrity: { title: "Contents Integrity", status: "PASS", checks: [] },
      analysisIntegrity: { title: "Analysis Files", status: "WARN", checks: [] },
      playlistResolution: {
        title: "Playlist Resolution",
        status: "PASS",
        checks: [
          { label: "Overall resolution", status: "PASS", detail: "3/3 entries resolve (100.0%) across 1 playlists" }
        ]
      },
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
      durationMs: 10
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
          if (command === "pick_usb_folder") {
            if (mode === "toggle-usb") {
              state.pickCount += 1;
              return state.pickCount % 2 === 1 ? "/Volumes/USB-TEST" : "/Volumes/USB-INVALID";
            }
            return "/Volumes/USB-TEST";
          }
          if (command === "list_playlists") {
            return { ok: true, data: { items: [] } };
          }
          if (command === "search_tracks") {
            return { ok: true, data: { total: 0, items: [] } };
          }
          if (command === "list_tracks") {
            return { ok: true, data: { total: 0, items: [] } };
          }
          if (command === "fetch_usb_histories") {
            return { ok: true, data: { items: [], warnings: [] } };
          }
          if (command === "run_usb_diagnostics") {
            return { ok: true, data: diagnosticsPayload };
          }
          if (command === "run_usb_parity_report") {
            return {
              ok: true,
              data: {
                overallStatus: "FAIL",
                checks: [
                  {
                    label: "Overall player parity status",
                    status: "FAIL",
                    detail: "playlists checked: 1, fail: 1"
                  },
                  {
                    label: "PDB metadata completeness",
                    status: "FAIL",
                    detail: "1 playlist-linked PDB track(s) are missing required player metadata"
                  },
                  {
                    label: "Media and analysis path parity",
                    status: "FAIL",
                    detail: "1 playlist-linked track(s) have media/analysis path mismatches"
                  },
                  {
                    label: "Artwork presence parity",
                    status: "WARN",
                    detail: "1 playlist-linked track(s) have artwork in one DB but not the other"
                  },
                  {
                    label: "PDB dictionary id resolution",
                    status: "FAIL",
                    detail: "1 playlist-linked track(s) have unresolved required PDB dictionary ids"
                  }
                ],
                summaryRows: [
                  { label: "Failing playlists", status: "FAIL", count: 1 },
                  { label: "Membership only-in-PDB", status: "PASS", count: 0 },
                  { label: "Membership only-in-eDB", status: "PASS", count: 0 },
                  { label: "Order mismatches", status: "PASS", count: 0 },
                  { label: "Duplicate PDB entries", status: "PASS", count: 0 },
                  { label: "PDB metadata gaps", status: "FAIL", count: 1 },
                  { label: "eDB source gaps", status: "PASS", count: 0 },
                  { label: "Path mismatches", status: "FAIL", count: 1 },
                  { label: "Artwork presence mismatches", status: "WARN", count: 1 },
                  { label: "Unresolved PDB dictionary ids", status: "FAIL", count: 1 }
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
                    sampleOnlyInPdb: [],
                    sampleOnlyInEdb: [],
                    sampleMetadataMismatches: ["Track A [analysisPath, artworkPath, artistDictId, pdbRequiredMetadata]"],
                    status: "FAIL"
                  }
                ],
                warnings: [],
                durationMs: 10
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
                  hasWriteAccess: false,
                  normalizedRoot: null,
                  hasVendorRoot: false,
                  hasContents: false,
                  hasPdb: false,
                  hasEdb: false,
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
                    hasWriteAccess: true,
                    normalizedRoot: path,
                    hasVendorRoot: false,
                    hasContents: false,
                    hasPdb: false,
                    hasEdb: false,
                    warnings: ["Missing vendor root folder"]
                  }
                };
              }
              return {
                ok: true,
                data: {
                  valid: true,
                  hasWriteAccess: true,
                  normalizedRoot: path,
                  hasVendorRoot: true,
                  hasContents: true,
                  hasPdb: true,
                  hasEdb: true,
                  warnings: []
                }
              };
            }
            return {
              ok: true,
              data: {
                valid: false,
                hasWriteAccess: true,
                normalizedRoot: path,
                hasVendorRoot: false,
                hasContents: false,
                hasPdb: false,
                hasEdb: false,
                warnings: ["missing External library structure"]
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
                  trackCount: p.tracks.length
                })),
                stats: {
                  indexedTracks: 0,
                  playlistReferencedTracks: 0,
                  playlistEntries: state.usbPlaylists.reduce((sum, p) => sum + p.tracks.length, 0)
                },
                warnings
              }
            };
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
            return { ok: true, data: { reordered: state.usbPlaylists.length, warnings: [] } };
          }
          if (command === "remove_usb_playlist") {
            const playlistId = String(payload?.request?.playlistId || "");
            state.usbPlaylists = state.usbPlaylists.filter((p) => String(p.id) !== playlistId);
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
          return { ok: false, error: { code: "UNKNOWN", message: `Unhandled command: ${command}` } };
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

  await page.locator("#initializeUsbBtn").click();

  await expect(page.locator("#usbInitRow")).toHaveClass(/hidden/);
  await expect(page.locator("#usbSelectedControls")).not.toHaveClass(/hidden/);
});

test("USB playlist removal confirm path handles cancel and confirm", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.goto("/");

  // Navigate to USB, select folder, then go to playlists
  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();
  await page.locator('.nav-item[data-view="usb-playlists"]').click();
  await page.locator("#refreshUsbBtn").click();
  await expect(page.locator('[data-usb-playlist="usb-1"]')).toBeVisible();

  await page.locator('[data-usb-remove-playlist="usb-1"]').click();
  await expect(page.locator("#confirmOverlay")).toBeVisible();
  await page.locator("#confirmCancelBtn").click();
  await expect(page.locator("#confirmOverlay")).toBeHidden();
  await expect(page.locator('[data-usb-playlist="usb-1"]')).toBeVisible();

  await page.locator('[data-usb-remove-playlist="usb-1"]').click();
  await page.locator("#confirmOkBtn").click();

  await expect(page.locator("#statusText")).toContainText("Removed USB playlist: Warmup");
  await expect(page.locator("#usbPlaylists")).toContainText("No playlists imported yet");
});

test("USB playlists tab shows empty state before import", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.goto("/");

  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();
  await page.locator('.nav-item[data-view="usb-playlists"]').click();

  await expect(page.locator("#usbPlaylists")).toContainText("No playlists imported yet");
  await expect(page.locator("#usbPlaylists")).toContainText("Import Playlists");
});

test("USB history tab shows empty state before import", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.goto("/");

  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();
  await page.locator('.nav-item[data-view="usb-history"]').click();

  await expect(page.locator("#historyList")).toContainText("No history imported yet");
  await expect(page.locator("#historyList")).toContainText("Import History");
});

test("USB playlists status counts only warn/error warnings", async ({ page }) => {
  await installTauriMock(page, "warning-mix");
  await page.goto("/");

  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();
  await page.locator('.nav-item[data-view="usb-playlists"]').click();
  await page.locator("#refreshUsbBtn").click();

  await expect(page.locator("#statusText")).toContainText("USB playlists loaded: 1 | (2 warning(s))");
  await expect(page.locator("#statusText .status-warning-link")).toHaveText("(2 warning(s))");
});

test("console.log messages appear in Event Log", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.goto("/");

  const marker = `console-mirror-${Date.now()}`;
  await page.evaluate((text) => {
    console.log(text);
  }, marker);

  await page.locator("#settingsBtn").click();
  await page.locator("#openEventLogBtn").click();

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

  await page.locator("#settingsBtn").click();
  await page.locator("#openEventLogBtn").click();
  await expect(page.locator("#eventLogList")).toContainText("CSP violation: style-src");
});

test("startup includes console bridge message in Event Log", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.goto("/");

  await page.locator("#settingsBtn").click();
  await page.locator("#openEventLogBtn").click();
  await expect(page.locator("#eventLogList")).toContainText("Frontend console bridge initialized");
});

test("USB sub-nav reveal/hide and fallback to USB panel", async ({ page }) => {
  await installTauriMock(page, "toggle-usb");
  await page.goto("/");

  const usbPlaylistsNav = page.locator('.nav-sub-item[data-view="usb-playlists"]');
  await expect(usbPlaylistsNav).not.toHaveClass(/revealed/);

  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();
  await expect(usbPlaylistsNav).toHaveClass(/revealed/);

  await usbPlaylistsNav.click();
  await expect(page.locator("#panel-usb-playlists")).toHaveClass(/active/);

  // Second folder pick simulates disconnect/invalid USB in toggle mode.
  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#selectUsbFolderBtn").click();
  await expect(usbPlaylistsNav).not.toHaveClass(/revealed/);
  await expect(page.locator("#panel-usb")).toHaveClass(/active/);
});

test("Event Log flood remains capped", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.goto("/");

  await page.evaluate(() => {
    for (let i = 0; i < 1200; i += 1) console.log(`flood-${i}`);
  });

  await page.locator("#settingsBtn").click();
  await page.locator("#openEventLogBtn").click();
  await expect(page.locator("#eventLogSummary")).toContainText("1000 event(s)");
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
  await expect(page.locator("#diagSections")).toContainText("PDB Integrity");
  await expect(page.locator("#diagSections")).toContainText("Playlist Resolution");
  await expect(page.locator("#diagSections")).toContainText("Overall resolution");
  await expect(page.locator("#diagPlaylistDetails")).toContainText("Playlist Resolution Details");
  await expect(page.locator("#diagPlaylistDetails")).not.toContainText("Strict Parity Playlist Details");
  await expect(page.locator("#diagPlaylistTableBody")).toContainText("Warmup");
  await expect(page.locator("#diagRawWarnings")).toHaveCount(0);

  await page.locator("#runUsbParityBtn").click();
  // Strict parity section header
  await expect(page.locator("#diagSections")).toContainText("USB Strict Parity Report");
  await expect(page.locator("#diagSections")).toContainText("Overall player parity status");
  await expect(page.locator("#diagSections")).toContainText("PDB metadata completeness");
  // Parity summary table renders structured rows, not a dense sentence
  await expect(page.locator("#diagSections")).toContainText("Parity Summary");
  await expect(page.locator("#diagSections")).toContainText("Failing playlists");
  await expect(page.locator("#diagSections")).toContainText("PDB metadata gaps");
  await expect(page.locator("#diagSections")).toContainText("Path mismatches");
  await expect(page.locator("#diagSections")).toContainText("Artwork presence mismatches");
  await expect(page.locator("#diagSections")).toContainText("Unresolved PDB dictionary ids");
  // Legacy dense summary sentence is absent
  await expect(page.locator("#diagSections")).not.toContainText("failing playlists=1, membership only-in-PDB");
  await expect(page.locator("#diagSections")).not.toContainText("Track key overlap");
  await expect(page.locator("#diagSections")).not.toContainText("Pro playlist coverage mode");
  // Diagnostics section should not leak into parity
  await expect(page.locator("#diagSections")).not.toContainText("Playlist Resolution");
  await expect(page.locator("#diagSections")).not.toContainText("PDB Integrity");
  // Strict parity playlist details table
  await expect(page.locator("#diagPlaylistDetails")).toContainText("Strict Parity Playlist Details");
  await expect(page.locator("#diagPlaylistTableBody")).toContainText("Warmup");
  await expect(page.locator("#diagPlaylistTableBody")).toContainText("path mismatch 1");
  await expect(page.locator("#diagPlaylistTableBody")).toContainText("dict issues 1");
  await expect(page.locator("#diagPlaylistTableBody")).toContainText("PDB gaps 1");
  await expect(page.locator("#diagRawWarnings")).toHaveCount(0);
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
  await page.goto("/");

  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();
  await page.locator('.nav-item[data-view="usb-playlists"]').click();
  await page.locator("#refreshUsbBtn").click();

  const rows = page.locator("#usbPlaylists li[data-usb-playlist-li]");
  await expect(rows).toHaveCount(3);
  await expect(rows.nth(0)).toContainText("Alpha");
  await expect(rows.nth(1)).toContainText("Bravo");
  await expect(rows.nth(2)).toContainText("Charlie");

  // Drag "Alpha" (row 0) to below "Charlie" (row 2), moving it to the end.
  await dragViaHandle(
    page,
    '#usbPlaylists li[data-usb-playlist-li="0"] [data-usb-drag-handle]',
    '#usbPlaylists li[data-usb-playlist-li="2"]',
    false
  );

  // reorderUsbPlaylists emits "Playlist order saved" and then immediately
  // refreshes from the backend, which overwrites status with the fetch
  // result — so assert on the settled state (row order + recorded call)
  // rather than the transient status text.
  await expect(page.locator("#statusText")).toContainText("USB playlists loaded: 3");
  await expect(rows.nth(0)).toContainText("Bravo");
  await expect(rows.nth(1)).toContainText("Charlie");
  await expect(rows.nth(2)).toContainText("Alpha");

  const reorderCalls = await page.evaluate(() => window.__reorderCalls);
  expect(reorderCalls.length).toBeGreaterThan(0);
  const lastCall = reorderCalls[reorderCalls.length - 1];
  expect(lastCall).toEqual(["usb-pl-2", "usb-pl-3", "usb-pl-1"]);
});

test("USB playlist drag that does not change position skips persisting order", async ({ page }) => {
  await installTauriMock(page, "reorder");
  await page.goto("/");

  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click();
  await page.locator('.nav-item[data-view="usb-playlists"]').click();
  await page.locator("#refreshUsbBtn").click();

  const rows = page.locator("#usbPlaylists li[data-usb-playlist-li]");
  await expect(rows).toHaveCount(3);

  // Drag row 0's handle a few pixels and drop back within the same row.
  const handle = page.locator('#usbPlaylists li[data-usb-playlist-li="0"] [data-usb-drag-handle]');
  const box = await handle.boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2 + 2, { steps: 3 });
  await page.mouse.up();

  await expect(rows.nth(0)).toContainText("Alpha");
  const reorderCalls = await page.evaluate(() => window.__reorderCalls);
  expect(reorderCalls).toEqual([]);
});

test("USB toggle race ends in deterministic final state", async ({ page }) => {
  await installTauriMock(page, "toggle-usb");
  await page.goto("/");

  const usbPlaylistsNav = page.locator('.nav-sub-item[data-view="usb-playlists"]');
  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbEmptyState .empty-state-action").click(); // valid (1)
  await expect(usbPlaylistsNav).toHaveClass(/revealed/);

  await page.locator("#selectUsbFolderBtn").click(); // invalid (2)
  await page.locator("#selectUsbFolderBtn").click(); // valid (3)
  await usbPlaylistsNav.click();
  await expect(page.locator("#panel-usb-playlists")).toHaveClass(/active/);
  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#selectUsbFolderBtn").click(); // invalid (4)

  await expect(usbPlaylistsNav).not.toHaveClass(/revealed/);
  await expect(page.locator("#panel-usb")).toHaveClass(/active/);
});
