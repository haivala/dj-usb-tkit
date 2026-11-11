import { test, expect, openUsb, uiTextCheck, useTestLanguage } from "./coverage-fixture.mjs";

// Counts the `fetch_usb_playlists` calls in `window.__playlistFetches`, and the
// track table fetches in `window.__trackFetches`.
function countPlaylistFetches(page) {
  return page.addInitScript(() => {
    window.__playlistFetches = 0;
    window.__trackFetches = 0;
    const backend = window.__e2eBackend;
    backend.override(async (command, request, payload, next) => {
      if (command === "fetch_usb_playlists") window.__playlistFetches += 1;
      if (command === "fetch_usb_playlist_tracks" || command === "fetch_usb_history_tracks") window.__trackFetches += 1;
    });
  });
}

// Empty USB lists, answered only once the test calls `window.__releaseLists()`.
function holdEmptyLists(page) {
  return page.addInitScript(() => {
    const released = new Promise((resolve) => { window.__releaseLists = resolve; });
    const backend = window.__e2eBackend;
    backend.override(async (command, request, payload, next) => {
      if (command === "fetch_usb_playlists") {
        await released;
        return backend.ok(backend.usbPlaylists());
      }
      if (command === "fetch_usb_histories") {
        await released;
        return backend.ok(backend.usbHistories());
      }
    });
  });
}

// Subscribes the app to the backend's events: `window.__emitJobEvent` fires a
// `job:event`, `window.__emitBackendLog` a `backend:log` (what the backend's
// `logging::log` sends for each warning it makes).
function listenForBackendEvents(page) {
  return page.addInitScript(() => window.__e2eBackend.useTauriRuntime());
}

function installTauriMock(page, mode) {
  return page.addInitScript(({ mode }) => {
    const backend = window.__e2eBackend;
    const state = {
      initialized: ["valid", "warning-mix", "toggle-usb", "reorder", "reorder-scroll", "progress-race"].includes(mode),
      pickCount: 0,
      usbPlaylists: mode === "valid" || mode === "warning-mix"
        ? [{ id: "usb-1", name: "Warmup", source: "mock-tauri", tracks: [{ title: "Track A", artist: "Artist 1", album: "Album X", bpm: 124, key: "8A" }] }]
        : mode === "reorder"
        ? ["Alpha", "Bravo", "Charlie"].map((name, i) => ({
            id: `usb-pl-${i + 1}`, name, source: "mock-tauri", tracks: [{ id: `t-${"abc"[i]}`, title: `Track ${"ABC"[i]}` }]
          }))
        : mode === "reorder-scroll"
        ? Array.from({ length: 40 }, (_, i) => ({
            id: `usb-pl-${i}`, name: `Playlist ${String(i).padStart(2, "0")}`, source: "mock-tauri", tracks: [{ title: "Track" }]
          }))
        : []
    };
    // Bravo holds three tracks to reorder: B, K, L. By artist they go L, B, K;
    // by title B, K, L (the order on the stick).
    if (mode === "reorder") {
      state.usbPlaylists[1].tracks = [
        { id: "t-b", title: "Track B Mike", artist: "Artist Lima" },
        { id: "t-k", title: "Track K Kilo", artist: "Artist Mike" },
        { id: "t-l", title: "Track L Lima", artist: "Artist Kilo" },
      ];
    }
    // Edit order: the pending order, as the backend holds it (null: no edit).
    let orderEdit = null;
    const orderEditState = () => {
      if (!orderEdit) return { active: false, changed: false, editedPlaylistIds: [], editingText: "" };
      const editedPlaylistIds = Object.keys(orderEdit.tracks).sort();
      const listChanged = !!orderEdit.playlists;
      const changed = listChanged || editedPlaylistIds.length > 0;
      return {
        active: true,
        changed,
        editedPlaylistIds,
        editingText: changed
          ? `Editing order: ${listChanged ? "playlist order" : ""}${listChanged && editedPlaylistIds.length ? " and " : ""}${editedPlaylistIds.length ? `${editedPlaylistIds.length} track order` : ""} changed`
          : "Editing order: nothing changed yet",
      };
    };
    const pendingPlaylists = () => {
      const byId = new Map(state.usbPlaylists.map((p) => [String(p.id), p]));
      const ordered = orderEdit?.playlists ? orderEdit.playlists.map((id) => byId.get(id)) : state.usbPlaylists;
      return ordered.map((p) => {
        const trackIds = orderEdit?.tracks[p.id];
        if (!trackIds) return p;
        const tracksById = new Map(p.tracks.map((track) => [track.id, track]));
        return { ...p, tracks: trackIds.map((id) => tracksById.get(id)) };
      });
    };
    const moveBefore = (ids, id, beforeId) => {
      const rest = ids.filter((item) => item !== id);
      rest.splice(beforeId == null ? rest.length : rest.indexOf(beforeId), 0, id);
      return rest;
    };
    const sortTracks = (tracks, by, dir) => tracks.slice().sort((a, b) =>
      String(a[by] || "").localeCompare(String(b[by] || "")) * (dir === "desc" ? -1 : 1));
    const setTrackOrder = (playlistId, ids) => {
      const base = state.usbPlaylists.find((p) => p.id === playlistId).tracks.map((track) => track.id);
      if (ids.join() === base.join()) delete orderEdit.tracks[playlistId];
      else orderEdit.tracks[playlistId] = ids;
    };
    const orderEditAnswer = (data = {}) => backend.ok({ statusText: "", warnings: [], warningText: "", ...data, orderEdit: orderEditState() });
    window.__usbAudioFormatCalls = [];
    let usbAudioFormats = [
      ["flac", "FLAC"], ["alac", "ALAC"], ["aiff", "AIFF"], ["wav", "WAV"], ["mp3", "MP3"], ["aac", "AAC"]
    ].map(([format, label]) => ({ format, label, allowed: true }));
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

    const diagnosticsPayload = {
      overallStatus: "WARN",
      statusLabel: "WARN", healthText: "USB health: warnings",
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

    backend.install(async (command, request, payload) => {
      if (command === "pick_usb_folder" && mode === "toggle-usb") {
        state.pickCount += 1;
        return state.pickCount % 2 === 1 ? "/Volumes/USB-TEST" : "/Volumes/USB-INVALID";
      }
      if (command === "create_playlist" && mode === "create-fails") {
        return { ok: false, error: { code: "INTERNAL", message: "disk full (mock)" } };
      }
      if (command === "create_playlist" && mode === "progress-race") {
        // Still saving well past the previous job's footer-hide delay.
        await new Promise((resolve) => setTimeout(resolve, 2500));
        return { ok: true, data: { playlistId: "p-new", name: payload?.request?.name } };
      }
      if (command === "run_usb_diagnostics") {
        return { ok: true, data: diagnosticsPayload };
      }
      if (command === "run_usb_parity_report") {
        return backend.ok({
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
        });
      }
      if (command === "repair_usb_diagnostics") {
        if (payload?.request?.apply) {
          return backend.ok({
            appliedFixes: payload.request.selectedFixIds || [],
            failedFixes: [],
            warnings: [],
            durationMs: 1,
            diagnostics: diagnosticsPayload,
            usbContentChanged: (payload.request.selectedFixIds || []).length > 0,
            engineLibraryEnabled: (payload.request.selectedFixIds || []).includes("keep_engine_library_up_to_date")
          });
        }
        return backend.ok({
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
        });
      }
      if (command === "select_usb") {
        const path = String(payload?.request?.path || "");
        if (!path) {
          return backend.ok({
            valid: false,
            normalizedRoot: null,
            warnings: ["USB path is empty"]
          });
        }
        if (state.initialized) {
          const forceInvalid = mode === "toggle-usb" && String(path).includes("INVALID");
          if (forceInvalid) {
            return backend.ok({
              valid: false,
              normalizedRoot: path,
              canInitialize: true,
              statusText: "USB selected but not initialized.",
              initHintText: "USB folder is writable but missing External library structure (Missing vendor root folder)",
              warnings: ["Missing vendor root folder"]
            });
          }
          return backend.ok({
            valid: true,
            normalizedRoot: path,
            audioFormats: formatsData(),
            warnings: []
          });
        }
        return backend.ok({
          valid: false,
          normalizedRoot: path,
          canInitialize: true,
          statusText: "USB selected but not initialized.",
          initHintText: "Missing vendor root folder | Missing Contents directory",
          warnings: [
            { level: "warn", code: "usb.validate.missing-vendor-root", message: "Missing vendor root folder", source: "usb-validate" },
            { level: "warn", code: "usb.validate.missing-contents", message: "Missing Contents directory", source: "usb-validate" }
          ]
        });
      }
      if (command === "initialize_usb") {
        state.initialized = true;
        return backend.ok({
          path: payload?.request?.usbRoot || "",
          createdDirs: ["vendor-db", "Contents"]
        });
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
        return backend.ok({
          items: pendingPlaylists().map((p) => ({
            ...p,
            trackCount: p.tracks.length,
            orderEdited: !!orderEdit?.tracks[p.id],
            selectedText: `USB playlist selected: ${p.name}`,
            removeConfirmText: `mock remove ${p.name}?`
          })),
          orderEdit: orderEditState(),
          orderEditBlockedText: null,
          summaryText: `${state.usbPlaylists.length} playlists`,
          statusText: `USB playlists loaded: ${state.usbPlaylists.length}`,
          doneText: "Done",
          stats: {
            indexedTracks: 0,
            playlistReferencedTracks: 0,
            playlistEntries: state.usbPlaylists.reduce((sum, p) => sum + p.tracks.length, 0)
          },
          playlistUsbExportStatus: [],
          warnings,
          // The backend counts the warn/error entries (the info one isn't).
          warningText: mode === "warning-mix" ? "(2 warnings)" : ""
        });
      }
      if (command === "fetch_usb_playlist_tracks") {
        const { id, query, sortBy, sortDir } = payload?.request || {};
        let tracks = pendingPlaylists().find((p) => String(p.id) === String(id))?.tracks || [];
        if (query) tracks = tracks.filter((track) => track.title.toLowerCase().includes(query.toLowerCase()));
        if (sortBy) tracks = sortTracks(tracks, sortBy, sortDir);
        return { ok: true, data: { items: tracks, total: tracks.length, listSummaryText: `${tracks.length} listed` } };
      }
      if (command === "set_usb_audio_formats") {
        const sent = payload?.request?.formats || [];
        window.__usbAudioFormatCalls.push(sent);
        const labels = new Map(usbAudioFormats.map((item) => [item.format, item.label]));
        usbAudioFormats = sent.map(({ format, allowed }) => ({ format, allowed, label: labels.get(format) }));
        return { ok: true, data: formatsData() };
      }
      if (command === "begin_usb_order_edit") {
        orderEdit = { playlists: null, tracks: {} };
        return orderEditAnswer({ statusText: "Editing the order: nothing is written until Save order." });
      }
      if (command === "move_usb_playlist") {
        const { playlistId, beforePlaylistId } = payload.request;
        const ids = moveBefore(pendingPlaylists().map((p) => p.id), playlistId, beforePlaylistId);
        orderEdit.playlists = ids.join() === state.usbPlaylists.map((p) => p.id).join() ? null : ids;
        return orderEditAnswer();
      }
      if (command === "move_usb_playlist_track") {
        const { playlistId, moveTrackId, beforeTrackId } = payload.request;
        const ids = pendingPlaylists().find((p) => p.id === playlistId).tracks.map((track) => track.id);
        setTrackOrder(playlistId, moveBefore(ids, moveTrackId, beforeTrackId));
        return orderEditAnswer();
      }
      if (command === "sort_usb_playlist_tracks") {
        const { playlistId, sortBy, sortDir } = payload.request;
        const tracks = pendingPlaylists().find((p) => p.id === playlistId).tracks;
        setTrackOrder(playlistId, sortTracks(tracks, sortBy, sortDir).map((track) => track.id));
        return orderEditAnswer();
      }
      if (command === "cancel_usb_order_edit") {
        orderEdit = null;
        return orderEditAnswer({ statusText: "Order changes discarded" });
      }
      if (command === "commit_usb_order_edit") {
        state.usbPlaylists = pendingPlaylists();
        orderEdit = null;
        return orderEditAnswer({
          statusText: "Order saved to the USB",
          warnings: [{ level: "error", code: "usb.order-edit.engine-failed", message: "Engine DJ library not rebuilt", source: "usb-import" }],
          warningText: "(1 warning)"
        });
      }
      if (command === "remove_usb_playlist") {
        const playlistId = String(payload?.request?.playlistId || "");
        state.usbPlaylists = state.usbPlaylists.filter((p) => String(p.id) !== playlistId);
        const warnings = mode === "reorder"
          ? [{ level: "warn", code: "usb.remove.kept-file", message: "Removal kept a shared file", source: "usb-import" }]
          : [];
        // The backend logged them as it made them.
        for (const warning of warnings) window.__emitBackendLog?.(warning);
        return backend.ok({
          playlistName: payload?.request?.playlistName || "",
          statusText: `Removed USB playlist: ${payload?.request?.playlistName || ""}`,
          removedFromEdb: 1,
          removedFromPdb: 1,
          warnings,
          warningText: warnings.length ? `(${warnings.length} warning${warnings.length === 1 ? "" : "s"})` : ""
        });
      }
    });
  }, { mode });
}

// USB initialize flow: an invalid-but-writable root shows the backend's hint and status, and can be
// initialized and unlocked.
test("USB initialize", async ({ page }) => {
  await installTauriMock(page, "needs-init");
  await page.goto("/");

  await openUsb(page);

  await expect(page.locator("#usbInitRow")).not.toHaveClass(/hidden/);
  await expect(page.locator("#initializeUsbBtn")).toBeEnabled();
  await expect(page.locator("#usbInitHint")).toHaveText("Missing vendor root folder | Missing Contents directory");
  await expect(page.locator("#statusText")).toContainText("USB selected but not initialized.");
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
    const backend = window.__e2eBackend;
    backend.override(async (command, request, payload, next) => {
      if (command === "repair_usb_diagnostics" && !payload?.request?.apply) {
        return { ok: true, data: { summary: "Nothing to repair.", overview: [], rows: [], nothingToFix: true } };
      }
    });
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

// USB playlist removal confirm path handles cancel and confirm; the Event Log header button counts
// errors logged while closed; the log shows console warnings and CSP events, not console.log or
// startup's developer lines, with one filter option per source; an open Event Log updates live: new
// entries on top, repeats coalesce, rows are kept; Diagnostics and parity render without warning
// panel; USB audio formats: ticks and drag order are saved, and the last ticked format can't be
// unticked.
test("USB playlist removal and the Event Log", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.goto("/");
  {
    // Navigate to USB, select folder, then go to playlists
    await openUsb(page);

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
  }

  await page.reload();
  {
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

    const marker = `console-mirror-${Date.now()}`;
    await page.evaluate((text) => {
      // Repeated "console"-sourced warnings: the source filter still lists it once.
      console.warn(text);
      console.warn(`${text}-again`);
      console.log(`${text}-log`);
      const evt = new Event("securitypolicyviolation");
      Object.defineProperty(evt, "violatedDirective", { value: "style-src", configurable: true });
      Object.defineProperty(evt, "blockedURI", { value: "inline", configurable: true });
      window.dispatchEvent(evt);
    }, marker);

    await page.locator("#eventLogBtn").click();
    const list = page.locator("#eventLogList");
    await expect(list).toContainText(`${marker}-again`);
    await expect(list).toContainText("CSP violation: style-src");
    await expect(list).not.toContainText(`${marker}-log`);
    await expect(list).not.toContainText("Frontend console bridge initialized");
    await expect(list).not.toContainText("Initial view ready");
    await expect(page.locator('#eventLogSourceFilter option[value="console"]')).toHaveCount(1);
  }

  await page.reload();
  {
    await page.locator("#eventLogBtn").click();

    const rows = page.locator("#eventLogList .event-log-row");
    await page.evaluate(() => console.warn("live-first"));
    await expect(rows.first()).toContainText("live-first");
    // Tag a row node: an incremental update must leave it in place.
    await rows.first().evaluate((row) => { row.__kept = true; });

    await page.evaluate(() => console.warn("live-second"));
    await expect(rows.first()).toContainText("live-second");
    expect(await rows.nth(1).evaluate((row) => row.__kept === true && row.textContent.includes("live-first"))).toBe(true);

    // A repeat moves the existing entry back to the top with its count.
    await page.evaluate(() => console.warn("live-first"));
    await expect(rows.first()).toContainText("live-first");
    await expect(rows.first().locator(".event-log-count")).toHaveText("x2");
    await expect(page.locator("#eventLogList .event-log-row", { hasText: "live-first" })).toHaveCount(1);

    // Entries hidden by the level filter don't show up.
    await page.locator("#eventLogLevelFilter").selectOption("error");
    await page.evaluate(() => console.warn("live-filtered-out"));
    await expect(page.locator("#eventLogList")).not.toContainText("live-filtered-out");
    await page.locator("#eventLogLevelFilter").selectOption("all");

    // A flood while open stays capped at the store's 500 entries.
    await page.evaluate(() => {
      for (let i = 0; i < 1200; i += 1) console.warn(`live-flood-${i}`);
    });
    await expect(page.locator("#eventLogSummary")).toContainText("500 events");
    await expect(rows).toHaveCount(500);
    await expect(rows.first()).toContainText("live-flood-1199");
  }

  await page.reload();
  {
    await openUsb(page);
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
    // The health dots show the result, with the backend's text.
    await expect(page.locator("#usbHealthDot")).toHaveClass(/health-warn/);
    await expect(page.locator("#usbHealthDot")).toHaveAttribute("data-tooltip", "USB health: warnings");
    await expect(page.locator("#usbHeaderHealthDot")).toHaveClass(/health-warn/);
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
  }

  await page.reload();
  {
    await openUsb(page);

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
  }
});

test("USB playlists and history tabs show their empty text only after a read finds nothing", async ({ page }) => {
  await installTauriMock(page, "valid");
  await holdEmptyLists(page);
  await countPlaylistFetches(page);
  await page.goto("/");

  await openUsb(page);
  const tabs = [
    { view: "usb-playlists", list: "#usbPlaylists", empty: "No playlists on this USB." },
    { view: "usb-history", list: "#historyList", empty: "No history on this USB." }
  ];
  for (const { view, list } of tabs) {
    await page.locator(`.nav-item[data-view="${view}"]`).click();
    await expect(page.locator(`#panel-${view}`)).toHaveClass(/active/);
    await expect(page.locator(list)).toBeEmpty();
    await expect(page.locator(list)).not.toHaveAttribute("data-loaded");
  }

  await page.evaluate(() => window.__releaseLists());
  for (const { view, list, empty } of tabs) {
    await page.locator(`.nav-item[data-view="${view}"]`).click();
    await expect(page.locator(list)).toHaveAttribute("data-loaded");
    await expect(page.locator(list)).toBeEmpty();
    expect(await page.locator(list).evaluate((node) => getComputedStyle(node, "::before").content)).toBe(`"${empty}"`);
    await expect(page.locator(`#panel-${view} .split > .right`)).toBeHidden();
  }
  await expect(page.locator("#exportHistoryTracklistBtn")).toBeHidden();
  expect(await page.evaluate(() => window.__trackFetches)).toBe(0);
});

test("USB playlists status shows the backend's warning count, linked to the Event Log", async ({ page }) => {
  await installTauriMock(page, "warning-mix");
  await page.goto("/");

  await openUsb(page, "usb-playlists");

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
    const released = new Promise((resolve) => { window.__releaseReads = resolve; });
    const backend = window.__e2eBackend;
    backend.override(async (command, request, payload, next) => {
      if (command === "fetch_usb_playlists") await released;
      if (command === "fetch_usb_histories") {
        await released;
        return backend.ok(backend.usbHistories({ items: [{ id: "h-1", name: "HISTORY 001", tracks: [{ title: "Track A" }] }] }));
      }
      if (command === "fetch_usb_history_tracks") return backend.ok({ items: [{ title: "Track A" }], total: 1 });
    });
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

// Removing a USB playlist keeps the open one; removing the open one opens nothing, even clicking
// its page again, until the page is opened again.
test("removing the open USB playlist", async ({ page }) => {
  await installTauriMock(page, "reorder");
  await listenForBackendEvents(page);
  await countPlaylistFetches(page);
  await page.addInitScript(() => {
    const backend = window.__e2eBackend;
    backend.override(async (command, request, payload, next) => {
      if (command === "fetch_usb_playlist_tracks") await window.__tracksGate;
    });
  });
  await page.goto("/");

  const nav = page.locator('.nav-item[data-view="usb-playlists"]');
  await openUsb(page);
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
    let fetches = 0;
    const backend = window.__e2eBackend;
    backend.override(async (command, request, payload, next) => {
      if (command === "fetch_usb_playlists") {
        fetches += 1;
        if (fetches === 1) return { ok: false, error: { code: "IO", message: "read failed" } };
      }
    });
  });
  await page.goto("/");
  await openUsb(page);

  await page.locator('.nav-item[data-view="usb-playlists"]').click();
  await expect(page.locator("#statusText")).toContainText("read failed");
  await expect(page.locator("#usbPlaylists")).toBeEmpty();

  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator('.nav-item[data-view="usb-playlists"]').click();
  await expect(page.locator('[data-usb-playlist="usb-1"]')).toBeVisible();
});

test("USB sub-nav reveal/hide and fallback to USB panel", async ({ page }) => {
  await installTauriMock(page, "toggle-usb");
  await page.goto("/");

  const usbPlaylistsNav = page.locator('.nav-sub-item[data-view="usb-playlists"]');
  await expect(usbPlaylistsNav).not.toHaveClass(/revealed/);
  await expect(usbPlaylistsNav).toBeHidden(); // Keyboard can't reach it either.

  await openUsb(page);
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

test("Rediagnose USB and Parity Report open Health & Diagnostics whatever the result", async ({ page }) => {
  await installTauriMock(page, "valid");
  // A passing diagnostics run, which the automatic run on USB select leaves
  // collapsed, and a failing parity report run.
  await page.addInitScript(() => {
    const backend = window.__e2eBackend;
    backend.override(async (command, request, payload, next) => {
      const result = await next();
      if (command === "run_usb_diagnostics") result.data = { ...result.data, overallStatus: "PASS", statusLabel: "PASS", healthText: "USB health: good" };
      if (command === "run_usb_parity_report" && window.__failParity) {
        return { ok: false, error: { code: "PARITY_FAILED", message: "parity failed" } };
      }
      return result;
    });
  });
  await page.goto("/");

  await openUsb(page);
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

// Repair preview shows the backend's rows and locks structural-prerequisite fix checkboxes; the
// Engine fix turns the setting on.
test("repair preview", async ({ page }) => {
  await installTauriMock(page, "valid");
  await countPlaylistFetches(page);
  await page.goto("/");

  await openUsb(page);

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
    window.__playlistLoads = 0;
    const backend = window.__e2eBackend;
    backend.seed({ playlists: [{ id: "pl-1", name: "Local Set" }] });
    backend.override(async (command, request, payload, next) => {
      if (command === "list_playlists") window.__playlistLoads += 1;
      if (command === "repair_usb_diagnostics" && payload?.request?.apply) {
        return backend.ok({
          appliedFixes: payload.request.selectedFixIds,
          usbContentChanged: true,
          statusText: "Repair apply complete: 1 applied",
          playlists: [{ id: "pl-1", name: "Local Set", source: "local" }],
          playlistUsbExportStatus: [{ playlistId: "pl-1", playlistName: "Local Set", usbSelected: true, sameNameExistsOnUsb: true, markLabel: "On this USB" }]
        });
      }
    });
  });
  await page.goto("/");
  await connectUsb(page);
  const mark = page.locator('#navPlaylistList .nav-playlist-item[data-playlist-id="pl-1"] .nav-playlist-status');
  await expect(mark).not.toHaveClass(/exported/);
  await page.locator("#usbHealthCard").evaluate((node) => { node.open = true; });
  await page.locator("#previewRepairsBtn").click();
  await expect(page.locator("#applyRepairsBtn")).toBeEnabled();
  // The repair must not need a second command to refresh the export status.
  const loadsBeforeRepair = await page.evaluate(() => window.__playlistLoads);
  await page.locator("#applyRepairsBtn").click();
  await expect(page.locator("#statusText")).toContainText("Repair apply complete");
  await expect(page.locator("#statusText")).not.toContainText("Diagnostics refreshed");
  // The same USB is still selected, so its diagnostics panel stays.
  await expect(page.locator("#usbDiagnosticsCard")).toBeVisible();

  await expect(mark).toHaveClass(/exported/);
  await expect(mark).toHaveAttribute("aria-label", "On this USB");
  expect(await page.evaluate(() => window.__playlistLoads)).toBe(loadsBeforeRepair);
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

// The `request`s of every `command` sent so far.
function sentRequests(page, command) {
  return page.evaluate((command) => window.__calls.filter((c) => c.command === command).map((c) => c.request), command);
}

const usbTrackIds = (page) => page.locator("#usbPlaylistTracks .track-grid-row[data-track-id]")
  .evaluateAll((rows) => rows.map((row) => row.dataset.trackId));
const usbTrackHandle = (trackId) => `#usbPlaylistTracks .track-grid-row[data-track-id="${trackId}"] [data-playlist-track-drag-handle]`;
const usbTrackRow = (trackId) => `#usbPlaylistTracks .track-grid-row[data-track-id="${trackId}"]`;

// Edit order: playlists and tracks are put in a new order (drag or sort) and nothing is sent to
// be written until Save order; Cancel drops it. Going to any other page with unsaved changes,
// the other USB pages too, asks to save, discard or stay.
test("USB playlist reorder", async ({ page }) => {
  await installTauriMock(page, "reorder");
  await countPlaylistFetches(page);
  await page.goto("/");

  await openUsb(page);

  // Open a diagnostics report first -- a successful save must clear it,
  // since it no longer reflects the playlist order it reported on.
  const usbHealthCard = page.locator("#usbHealthCard");
  await usbHealthCard.evaluate((node) => { node.open = true; });
  await page.locator("#reDiagnoseBtn").click();
  await expect(page.locator("#diagSections")).not.toBeEmpty();

  await page.locator('.nav-item[data-view="usb-playlists"]').click();

  const rows = page.locator("#usbPlaylists [data-usb-playlist]");
  const editBtn = page.locator("#usbEditOrderBtn");
  const saveBtn = page.locator("#usbSaveOrderBtn");
  const cancelBtn = page.locator("#usbCancelOrderBtn");
  const editText = page.locator("#usbOrderEditText");
  const trackHeader = page.locator('[data-body-id="usbPlaylistTracks"] [role="columnheader"][data-sort-key="artist"]');
  const dialog = page.locator("#confirmOverlay");
  await expect(rows).toHaveCount(3);
  await expect(rows.nth(0)).toHaveClass(/active/);
  await expect(page.locator("#usbPlaylistTracks .track-grid-row")).toHaveCount(1);

  // Browsing: no drag handles, and a sort only changes the view.
  await expect(page.locator("#usbPlaylists [data-usb-drag-handle]").first()).toBeHidden();
  await expect(page.locator("#usbPlaylistTracks [data-playlist-track-drag-handle]")).toHaveCount(0);
  await expect(saveBtn).toBeHidden();
  await rows.nth(1).click();
  await expect(page.locator("#usbPlaylistTracks .track-grid-row")).toHaveCount(3);
  await trackHeader.click();
  await expect.poll(() => usbTrackIds(page)).toEqual(["t-l", "t-b", "t-k"]);
  expect(await sentRequests(page, "sort_usb_playlist_tracks")).toHaveLength(0);
  // Back to the order on the stick.
  for (let i = 0; i < 4; i += 1) await trackHeader.click();
  await expect.poll(() => usbTrackIds(page)).toEqual(["t-b", "t-k", "t-l"]);
  await rows.nth(0).click();

  // The columns can't change while editing (a header sort is the pending order then).
  const columnsBtn = page.locator('#panel-usb-playlists [data-track-columns-button]');
  await expect(columnsBtn).toBeEnabled();
  await editBtn.click();
  await expect(editText).toHaveText("Editing order: nothing changed yet");
  await expect(saveBtn).toBeDisabled();
  await expect(columnsBtn).toBeDisabled();
  await expect(columnsBtn).toHaveAttribute("data-tooltip", "Save or cancel the order edit first to change the columns");
  await expect(cancelBtn).toBeVisible();
  await expect(page.locator("#usbPlaylists [data-usb-drag-handle]").first()).toBeVisible();
  await expect(page.locator("#usbPlaylistTracks [data-playlist-track-drag-handle]")).toHaveCount(1);

  // Drag "Alpha" (row 0) to below "Charlie" (row 2), moving it to the end.
  await dragViaHandle(page, "#usbPlaylists li:nth-child(1) [data-usb-drag-handle]", "#usbPlaylists li:nth-child(3)", false);
  await expect(rows.nth(0)).toContainText("Bravo");
  await expect(rows.nth(1)).toContainText("Charlie");
  await expect(rows.nth(2)).toContainText("Alpha");
  expect(await sentRequests(page, "move_usb_playlist")).toEqual([
    { usbRoot: "/Volumes/USB-TEST", playlistId: "usb-pl-1", beforePlaylistId: null }
  ]);
  await expect(editText).toHaveText("Editing order: playlist order changed");
  await expect(saveBtn).toBeEnabled();
  // The moved playlist stays open.
  await expect(rows.nth(2)).toHaveClass(/active/);
  await expect(page.locator("#usbPlaylistHeading .track-list-heading-name")).toHaveText("Alpha");

  // A track drag in Bravo: B to the end.
  await rows.nth(0).click();
  await expect.poll(() => usbTrackIds(page)).toEqual(["t-b", "t-k", "t-l"]);
  await dragViaHandle(page, usbTrackHandle("t-b"), usbTrackRow("t-l"), false);
  await expect.poll(() => usbTrackIds(page)).toEqual(["t-k", "t-l", "t-b"]);
  expect(await sentRequests(page, "move_usb_playlist_track")).toEqual([
    { usbRoot: "/Volumes/USB-TEST", playlistId: "usb-pl-2", moveTrackId: "t-b", beforeTrackId: null }
  ]);
  await expect(rows.nth(0).locator(".playlist-order-edited")).toBeVisible();
  await expect(rows.nth(1).locator(".playlist-order-edited")).toBeHidden();
  await expect(editText).toHaveText("Editing order: playlist order and 1 track order changed");

  // A sort click makes the sort the pending order: through the header's cycle (artist up and
  // down, then title up and down), the pages are fetched in their natural (pending) order.
  const fetchesBefore = (await sentRequests(page, "fetch_usb_playlist_tracks")).length;
  for (const [sortBy, sortDir, order] of [
    ["artist", "asc", ["t-l", "t-b", "t-k"]],
    ["artist", "desc", ["t-k", "t-b", "t-l"]],
    ["title", "asc", ["t-b", "t-k", "t-l"]],
    ["title", "desc", ["t-l", "t-k", "t-b"]],
  ]) {
    await trackHeader.click();
    await expect.poll(() => usbTrackIds(page)).toEqual(order);
    expect((await sentRequests(page, "sort_usb_playlist_tracks")).at(-1)).toEqual({ usbRoot: "/Volumes/USB-TEST", playlistId: "usb-pl-2", sortBy, sortDir });
  }
  await expect(trackHeader).toHaveClass(/sort-desc/);
  // The cycle's unsorted state keeps the pending order.
  await trackHeader.click();
  await expect(trackHeader).not.toHaveClass(/sort-(asc|desc)/);
  await page.waitForTimeout(100);
  expect(await usbTrackIds(page)).toEqual(["t-l", "t-k", "t-b"]);
  expect(await sentRequests(page, "sort_usb_playlist_tracks")).toHaveLength(4);
  const fetches = (await sentRequests(page, "fetch_usb_playlist_tracks")).slice(fetchesBefore);
  expect(fetches.length).toBeGreaterThan(0);
  expect(fetches.every((request) => request.sortBy === null)).toBe(true);

  // A drag after a sort click moves the row without sorting again, and forgets the header choice.
  await trackHeader.click();
  await expect.poll(() => usbTrackIds(page)).toEqual(["t-l", "t-b", "t-k"]);
  await dragViaHandle(page, usbTrackHandle("t-k"), usbTrackRow("t-l"), true);
  await expect.poll(() => usbTrackIds(page)).toEqual(["t-k", "t-l", "t-b"]);
  expect(await sentRequests(page, "sort_usb_playlist_tracks")).toHaveLength(5);
  expect((await sentRequests(page, "move_usb_playlist_track")).at(-1)).toEqual(
    { usbRoot: "/Volumes/USB-TEST", playlistId: "usb-pl-2", moveTrackId: "t-k", beforeTrackId: "t-l" }
  );
  await expect(trackHeader).not.toHaveClass(/sort-(asc|desc)/);

  // Searching: the handles stay, disabled.
  await page.locator("#usbTrackSearch").fill("Lima");
  await expect(page.locator("#usbPlaylistTracks .track-grid-row")).toHaveCount(1);
  const disabledHandle = page.locator("#usbPlaylistTracks .td-drag .drag-handle");
  await expect(disabledHandle).toHaveClass(/disabled/);
  await expect(disabledHandle).toHaveAttribute("data-tooltip", "Clear the search to drag tracks");
  await page.locator("#usbTrackSearch").fill("");
  await expect(page.locator("#usbPlaylistTracks [data-playlist-track-drag-handle]")).toHaveCount(3);

  // Leaving with unsaved changes, for any page: Escape, a backdrop click and Stay keep the edit
  // and the page.
  const libraryNav = page.locator('.nav-item[data-view="library"]');
  const dismissals = [
    () => page.keyboard.press("Escape"),
    () => dialog.click({ position: { x: 5, y: 5 } }),
    () => page.locator("#confirmCancelBtn").click(),
  ];
  for (const [i, view] of ["library", "usb", "usb-history", "usb-player-menu"].entries()) {
    await page.locator(`.nav-item[data-view="${view}"]`).click();
    await expect(dialog).toBeVisible();
    await expect(page.locator("#confirmTitle")).toHaveText("Unsaved order");
    await expect(page.locator("#confirmOkBtn")).toHaveText("Save order");
    await expect(page.locator("#confirmAltBtn")).toHaveText("Discard changes");
    await expect(page.locator("#confirmCancelBtn")).toHaveText("Stay");
    await dismissals[i % dismissals.length]();
    await expect(dialog).toBeHidden();
    await expect(page.locator("#panel-usb-playlists")).toHaveClass(/active/);
  }
  expect(await sentRequests(page, "commit_usb_order_edit")).toHaveLength(0);
  expect(await sentRequests(page, "cancel_usb_order_edit")).toHaveLength(0);
  await expect(editText).toContainText("changed");

  // Discard: the edit is cancelled, then the view switches.
  await libraryNav.click();
  await page.locator("#confirmAltBtn").click();
  await expect(page.locator("#panel-library")).toHaveClass(/active/);
  expect(await sentRequests(page, "cancel_usb_order_edit")).toHaveLength(1);
  expect(await sentRequests(page, "commit_usb_order_edit")).toHaveLength(0);
  await page.locator('.nav-item[data-view="usb-playlists"]').click();
  await expect(rows.nth(0)).toContainText("Alpha");
  await expect(editBtn).toBeVisible();
  await expect(page.locator("#usbPlaylists [data-usb-drag-handle]").first()).toBeHidden();

  // Save from the dialog: the edit is saved, then the view switches.
  await editBtn.click();
  await dragViaHandle(page, "#usbPlaylists li:nth-child(1) [data-usb-drag-handle]", "#usbPlaylists li:nth-child(3)", false);
  await expect(rows.nth(2)).toContainText("Alpha");
  await libraryNav.click();
  await page.locator("#confirmOkBtn").click();
  await expect(page.locator("#panel-library")).toHaveClass(/active/);
  expect(await sentRequests(page, "commit_usb_order_edit")).toEqual([{ usbRoot: "/Volumes/USB-TEST" }]);
  await page.locator('.nav-item[data-view="usb-playlists"]').click();
  await expect(rows.nth(2)).toContainText("Alpha");
  await expect(editBtn).toBeVisible();

  // With nothing changed, leaving just ends the edit.
  await editBtn.click();
  await libraryNav.click();
  await expect(page.locator("#panel-library")).toHaveClass(/active/);
  await expect(dialog).toBeHidden();
  expect(await sentRequests(page, "cancel_usb_order_edit")).toHaveLength(2);
  await page.locator('.nav-item[data-view="usb-playlists"]').click();

  // Save order: the status line with its warnings, the stale diagnostics gone.
  await editBtn.click();
  await dragViaHandle(page, "#usbPlaylists li:nth-child(3) [data-usb-drag-handle]", "#usbPlaylists li:nth-child(1)", true);
  await expect(rows.nth(0)).toContainText("Alpha");
  await saveBtn.click();
  await expect(page.locator("#statusText")).toContainText("Order saved to the USB");
  await expect(page.locator("#statusText .status-warning-link")).toHaveText("(1 warning)");
  await expect(page.locator("#diagSections")).toBeEmpty();
  await expect(page.locator("#diagOverallStatus")).toBeEmpty();
  await expect(editBtn).toBeVisible();
  await expect(columnsBtn).toBeEnabled();
  await expect(rows.nth(0)).toContainText("Alpha");

  // Cancel: the order on the stick comes back.
  await editBtn.click();
  await dragViaHandle(page, "#usbPlaylists li:nth-child(1) [data-usb-drag-handle]", "#usbPlaylists li:nth-child(3)", false);
  await expect(rows.nth(2)).toContainText("Alpha");
  await cancelBtn.click();
  await expect(page.locator("#statusText")).toContainText("Order changes discarded");
  await expect(rows.nth(0)).toContainText("Alpha");
  await expect(editBtn).toBeVisible();
  await expect(columnsBtn).toBeEnabled();
  await expect(columnsBtn).toHaveAttribute("data-tooltip", "Show, hide and reorder columns");
  expect(await sentRequests(page, "cancel_usb_order_edit")).toHaveLength(3);

  // A view sort from before the edit becomes the pending order when a drag starts, then the
  // row moves among what was on screen.
  await rows.nth(1).click();
  await expect.poll(() => usbTrackIds(page)).toEqual(["t-b", "t-k", "t-l"]);
  await trackHeader.click();
  await expect.poll(() => usbTrackIds(page)).toEqual(["t-l", "t-b", "t-k"]);
  const sortsBefore = (await sentRequests(page, "sort_usb_playlist_tracks")).length;
  await editBtn.click();
  await dragViaHandle(page, usbTrackHandle("t-k"), usbTrackRow("t-l"), true);
  await expect.poll(() => usbTrackIds(page)).toEqual(["t-k", "t-l", "t-b"]);
  expect((await sentRequests(page, "sort_usb_playlist_tracks")).slice(sortsBefore)).toEqual([
    { usbRoot: "/Volumes/USB-TEST", playlistId: "usb-pl-2", sortBy: "artist", sortDir: "asc" }
  ]);
  expect((await sentRequests(page, "move_usb_playlist_track")).at(-1)).toEqual(
    { usbRoot: "/Volumes/USB-TEST", playlistId: "usb-pl-2", moveTrackId: "t-k", beforeTrackId: "t-l" }
  );
  await expect(trackHeader).not.toHaveClass(/sort-(asc|desc)/);
  await cancelBtn.click();
  await expect.poll(() => usbTrackIds(page)).toEqual(["t-b", "t-k", "t-l"]);
});

// A failed save re-reads the list, shows the error and keeps the edit on, also when it was the
// leave dialog's Save (the view stays). The save's failure shows even when the re-read fails.
test("failed USB playlist reorder", async ({ page }) => {
  await installTauriMock(page, "reorder");
  await page.addInitScript(() => {
    const backend = window.__e2eBackend;
    backend.override(async (command) => {
      if (command === "commit_usb_order_edit") return backend.fail("Failed to save the order: write failed", "IO");
      // `__stickGone`: the read after the failed save fails too.
      if (command === "fetch_usb_playlists" && window.__stickGone) return backend.fail("stick gone", "IO");
    });
  });
  await countPlaylistFetches(page);
  await page.goto("/");

  await openUsb(page, "usb-playlists");

  const rows = page.locator("#usbPlaylists [data-usb-playlist]");
  const saveBtn = page.locator("#usbSaveOrderBtn");
  await expect(rows.nth(0)).toHaveClass(/active/);
  await page.locator("#usbEditOrderBtn").click();
  await dragViaHandle(page, "#usbPlaylists li:nth-child(1) [data-usb-drag-handle]", "#usbPlaylists li:nth-child(3)", false);
  await expect(rows.nth(2)).toContainText("Alpha");
  const fetchesBefore = await page.evaluate(() => window.__playlistFetches);

  await saveBtn.click();
  await expect(page.locator("#statusText")).toContainText("Failed to save the order: write failed");
  await expect.poll(() => page.evaluate(() => window.__playlistFetches)).toBe(fetchesBefore + 1);
  // Still editing, the pending order still shown.
  await expect(saveBtn).toBeEnabled();
  await expect(page.locator("#usbOrderEditText")).toHaveText("Editing order: playlist order changed");
  await expect(rows.nth(2)).toContainText("Alpha");
  await expect(rows.nth(2)).toHaveClass(/active/);
  await expect(page.locator("#usbPlaylistHeading .track-list-heading-name")).toHaveText("Alpha");

  // Save from the leave dialog fails: no view switch.
  await page.locator('.nav-item[data-view="library"]').click();
  await page.locator("#confirmOkBtn").click();
  await expect(page.locator("#statusText")).toContainText("Failed to save the order: write failed");
  await expect(page.locator("#panel-usb-playlists")).toHaveClass(/active/);
  await expect(saveBtn).toBeEnabled();

  // The stick went away: the save's failure, not the read's, and the read's
  // failure on the progress bar.
  await page.evaluate(() => { window.__stickGone = true; });
  await saveBtn.click();
  await expect(page.locator("#progressText")).toHaveText("USB load failed");
  await expect(page.locator("#progressFooter")).toHaveClass(/error/);
  await expect(page.locator("#statusText")).toContainText("Failed to save the order: write failed");
  // The failure stays until its × closes it.
  await page.locator("#progressDismiss").click();
  await expect(page.locator("#progressFooter")).not.toHaveClass(/active/);
});

test("USB playlist drag auto-scrolls the list when held near its bottom edge", async ({ page }) => {
  await installTauriMock(page, "reorder-scroll");
  await page.goto("/");

  await openUsb(page, "usb-playlists");

  const list = page.locator("#usbPlaylists");
  await expect(page.locator("#usbPlaylists [data-usb-playlist]")).toHaveCount(40);

  const initialScrollTop = await list.evaluate((node) => node.scrollTop);
  expect(initialScrollTop).toBe(0);

  // The progress footer briefly overlaps the bottom of the viewport after a
  // refresh -- wait for it to clear so it doesn't swallow the drag events
  // meant for the list underneath it.
  await expect(page.locator("#progressFooter")).not.toHaveClass(/active/, { timeout: 3000 });

  await page.locator("#usbEditOrderBtn").click();
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
    const backend = window.__e2eBackend;
    backend.seed({ playlists: [{ id: "pl-1", name: "Local Set" }] });
    backend.override(async (command, request, payload, next) => {
      const playlistUsbExportStatus = [{ playlistId: "pl-1", playlistName: "Local Set", usbSelected: true, sameNameExistsOnUsb: true, markLabel: "On this USB" }];
      if (command === "fetch_usb_playlists") {
        const backend = window.__e2eBackend;
        return backend.ok(backend.usbPlaylists({ items: [{ id: "usb-1", name: "Warmup", tracks: [] }], playlistUsbExportStatus }));
      }
      const result = await next();
      if (command === "run_usb_diagnostics") {
        result.data = { ...result.data, playlistUsbExportStatus };
      }
      return result;
    });
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
    const backend = window.__e2eBackend;
    const playlist = (name) => ({ id: name, name, source: "mock-tauri", tracks: [] });
    const released = new Promise((resolve) => { window.__releaseFirstRead = resolve; });
    let fetches = 0;
    backend.override(async (command, request, payload, next) => {
      if (command !== "fetch_usb_playlists") return next();
      fetches += 1;
      if (fetches > 1) return backend.ok(backend.usbPlaylists({ items: [playlist("Fresh")] }));
      await released;
      return backend.ok(backend.usbPlaylists({ items: [playlist("Stale")] }));
    });
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

  await openUsb(page, "usb-playlists");
  // The read finished; its footer is due to hide 1.2s from now.
  await expect(page.locator("#progressText")).toContainText("Done");

  // Every text the footer shows, in order: the done text is up for only 350 ms.
  await page.evaluate(() => {
    const node = document.getElementById("progressText");
    window.__progressTexts = [];
    new MutationObserver(() => window.__progressTexts.push(node.textContent))
      .observe(node, { childList: true, characterData: true, subtree: true });
  });
  await page.locator("#addPlaylistBtn").click();
  await page.locator(".nav-new-input").fill("Race");
  await page.locator(".nav-new-input").press("Enter");
  await expect(page.locator("#progressText")).toContainText("Saving playlist");

  // Past the read's hide delay, the still-running save keeps the footer.
  await page.waitForTimeout(1500);
  await expect(page.locator("#progressFooter")).toHaveClass(/active/);
  await expect(page.locator("#progressText")).toContainText("Saving playlist");

  // The job's own texts: running first, done once the save is in.
  await expect.poll(() => page.evaluate(() => window.__progressTexts)).toContain("Creating playlist done");
  const texts = await page.evaluate(() => window.__progressTexts);
  expect(texts.indexOf("Creating playlist...")).toBeGreaterThanOrEqual(0);
  expect(texts.indexOf("Creating playlist...")).toBeLessThan(texts.indexOf("Creating playlist done"));
});

test("a failed playlist create leaves its failed text on the progress footer", async ({ page }) => {
  await installTauriMock(page, "create-fails");
  await page.goto("/");

  await page.locator("#addPlaylistBtn").click();
  await page.locator(".nav-new-input").fill("Doomed");
  await page.locator(".nav-new-input").press("Enter");

  await expect(page.locator("#progressText")).toHaveText("Creating playlist failed");
  await expect(page.locator("#progressFooter")).toHaveClass(/error/);
  await expect(page.locator("#progressFooter")).toHaveClass(/dismissable/);
});

async function connectUsb(page) {
  await openUsb(page);
  await expect(page.locator("#diagOverallStatus")).toHaveText("WARN");
}

// A running USB job locks the USB selection and export until it ends; other jobs don't.
test("USB job locks", async ({ page }) => {
  await installTauriMock(page, "valid");
  await listenForBackendEvents(page);
  await page.addInitScript(() => {
    window.__picks = 0;
    window.__validates = 0;
    const backend = window.__e2eBackend;
    backend.seed({ playlists: [{ id: "pl-1", name: "Local Set" }] });
    backend.override(async (command, request, payload, next) => {
      if (command === "pick_usb_folder") window.__picks += 1;
      if (command === "select_usb") {
        window.__validates += 1;
        // Another drive shows up as recent once the first is connected.
        const selected = await next();
        return { ...selected, data: { ...selected.data, devices: [{ id: "dev-1", rootPath: "/Volumes/OTHER", label: "Other", mounted: false }] } };
      }
    });
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

  // The footer follows the running job only, its percent kept within 0–100;
  // a failed job's reason is on the status line (the backend logs it itself).
  const job = (jobId, event, percent, message) => page.evaluate(
    (e) => window.__emitJobEvent(e),
    { jobId, jobType: "export", stage: "copy", event, percent, message }
  );
  const fill = page.locator("#progressFill");
  await emit("analysis", "job.completed");
  await job("export-9", "job.started", 5, "Copying files");
  await expect(fill).toHaveAttribute("style", /width: 5%/);
  await job("other-job", "job.progress", 80, "Wrong job");
  await page.waitForTimeout(100);
  await expect(fill).toHaveAttribute("style", /width: 5%/);
  await expect(page.locator("#progressText")).not.toContainText("Wrong job");
  await job("export-9", "job.progress", 150, "Copying files");
  await expect(fill).toHaveAttribute("style", /width: 100%/);
  await job("export-9", "job.failed", 100, "Export failed: disk gone");
  await expect(page.locator("#statusText")).toContainText("Export failed: disk gone");
  await page.locator("#eventLogBtn").click();
  await expect(page.locator("#eventLogList")).not.toContainText("Export failed: disk gone");
});

// A stored USB isn't shown as selected, or read, until it's validated, and the drive name badge
// does nothing meanwhile.
test("stored USB before validation", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.addInitScript(() => {
    window.localStorage.setItem("djusbtkit.usbRoot", "/Volumes/USB-TEST");
    window.__validates = 0;
    const backend = window.__e2eBackend;
    backend.override(async (command, request, payload, next) => {
      if (command === "select_usb") window.__validates += 1;
    });
  });
  await page.goto("/");
  await page.locator('.nav-item[data-view="usb"]').click();

  await expect(page.locator("#usbRootPathText")).toHaveText("No USB selected");
  await expect(page.locator("#usbSelectedControls")).toBeHidden();
  await expect(page.locator("#usbInitRow")).toBeHidden();
  await expect(page.locator('.nav-sub-item[data-view="usb-playlists"]')).not.toHaveClass(/revealed/);
  expect(await page.evaluate(() => window.__validates)).toBe(0);

  await expect(page.locator("#usbNameBadgeLabel")).toHaveText("Not connected");
  // A real mouse still lands on it: Playwright only refuses because of aria-disabled.
  await page.locator("#usbNameBadgeLabel").click({ force: true });
  await page.waitForTimeout(100);
  await expect(page.locator("#driveNameOverlay")).toBeHidden();
});

// Restoring a backup: cancel and failure change nothing; success clears the report and lists and
// applies the playlists and backups it answers with.
test("restoring a backup", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.addInitScript(() => {
    window.__playlistReads = 0;
    window.__restores = 0;
    window.__deletes = [];
    window.__backupLists = 0;
    const backup = { timestamp: "2020-01-01_00-00-00", timeText: "the time", reasonText: "the reason", detailText: "the detail", restoreConfirmText: "Restore it?", deleteConfirmText: "Delete it?", files: ["export.pdb"], reason: "export", sizeBytes: 1024, location: "usb" };
    const backups = (summaryText) => ({ summaryText, emptyText: "none", items: [backup] });
    const backend = window.__e2eBackend;
    backend.seed({ playlists: [{ id: "pl-1", name: "Local Set" }] });
    backend.override(async (command, request, payload, next) => {
      if (command === "list_usb_backups") {
        window.__backupLists += 1;
        return { ok: true, data: backups("the summary") };
      }
      if (command === "fetch_usb_playlists") window.__playlistReads += 1;
      if (command === "delete_usb_backup") {
        window.__deletes.push(payload.request.timestamp);
        return { ok: true, data: { deleted: true, statusText: "Deleted PDB backup", backups: backups("after the delete") } };
      }
      if (command === "restore_usb_backup") {
        window.__restores += 1;
        if (window.__failRestore) return { ok: false, error: { code: "IO", message: "Restore failed: disk gone" } };
        return backend.ok({
          restored: true,
          statusText: "Restored eDB and PDB from backup",
          backups: backups("after the restore"),
          playlistUsbExportStatus: [{ playlistId: "pl-1", playlistName: "Local Set", usbSelected: true, sameNameExistsOnUsb: true, markLabel: "On this USB" }]
        });
      }
    });
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
  // The list came with the delete's answer: no second read.
  await expect(page.locator("#backupsSummary")).toHaveText("after the delete");
  expect(await page.evaluate(() => window.__backupLists)).toBe(1);

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
  await expect(page.locator("#backupsSummary")).toHaveText("after the restore");
  // Opening the list, then the failed restore's re-read; the successful one
  // brought its list along.
  expect(await page.evaluate(() => window.__backupLists)).toBe(2);

  // Opening the page reads the restored list.
  await page.locator('.nav-item[data-view="usb-playlists"]').click();
  await expect(usbRow).toBeVisible();
  expect(await page.evaluate(() => window.__playlistReads)).toBe(2);
});

test("applying repairs that change nothing keeps the loaded USB playlists and shows the refreshed report", async ({ page }) => {
  await installTauriMock(page, "valid");
  await countPlaylistFetches(page);
  await page.addInitScript(() => {
    const backend = window.__e2eBackend;
    backend.override(async (command, request, payload, next) => {
      if (command === "repair_usb_diagnostics" && payload?.request?.apply) {
        return {
          ok: true,
          data: { failedFixes: payload.request.selectedFixIds, usbContentChanged: false, statusText: "Repair apply complete: 0 applied", diagnostics: { overallStatus: "FAIL", statusLabel: "FAIL", healthText: "USB health: issues found" } }
        };
      }
    });
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

// The player menu editor selects in one list at a time, its buttons follow the selected item, and
// an edit clears the diagnostics report only when it changed the stick.
test("player menu editor", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.addInitScript(() => {
    window.__menuUpdated = false;
    const backend = window.__e2eBackend;
    const config = (extra) => backend.ok({
      currentItems: [
        backend.playerMenuItem(132, { name: "PLAYLIST", removable: false, canMoveDown: true }),
        backend.playerMenuItem(139, { name: "KEY", canMoveUp: true })
      ],
      availableItems: [
        backend.playerMenuItem(133, { name: "BPM", removable: false }),
        backend.playerMenuItem(134, { name: "RATING" })
      ],
      ...extra
    });
    backend.override(async (command) => {
      if (command === "get_usb_player_menu_config") return config({ updated: false, statusText: "Player menu loaded" });
      if (command === "update_usb_player_menu_config") {
        const updated = window.__menuUpdated;
        return config({ updated, statusText: updated ? "Player menu updated" : "Player menu unchanged" });
      }
    });
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

  const report = page.locator("#diagOverallStatus");
  const status = page.locator("#statusText");
  const addRating = async () => {
    await item("usbPlayerMenuAvailable", 134).click();
    await add.click();
  };
  await addRating();
  await expect(status).toContainText("Player menu unchanged");
  await expect(report).toHaveText("WARN");

  await page.evaluate(() => { window.__menuUpdated = true; });
  await addRating();
  await expect(status).toContainText("Player menu updated");
  await expect(report).toHaveText("");
});

// Background colour shows nothing selected when the databases differ, a stick that can't take one
// says why, and choosing one sets it and redraws from the answer.
test("background colour", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.addInitScript(() => {
    window.__color = { pdbColor: 6, edbColor: 6, writable: true };
    window.__colorCalls = [];
    window.__colorUpdated = false;
    const backend = window.__e2eBackend;
    backend.override(async (command, request, payload, next) => {
      if (command === "get_usb_background_color") {
        if (window.__colorFails) return backend.fail("Failed to read the background colour: colour read failed", "IO");
        return backend.ok(window.__color);
      }
      if (command === "set_usb_background_color") {
        const { color } = payload.request;
        window.__colorCalls.push(payload.request);
        const now = window.__colorUpdated ? color : 0;
        const statusText = window.__colorUpdated ? "Background colour set to Green" : "Background colour unchanged";
        return backend.ok({ pdbColor: now, edbColor: now, updated: window.__colorUpdated, statusText });
      }
    });
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

  // Choosing one: an unchanged answer keeps the old chip and the report.
  await page.evaluate(() => { window.__colorFails = false; });
  await openMenu({ pdbColor: 0, edbColor: 0, writable: true });
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

test("when the UI's texts can't be loaded, the page shows one line and the window still opens", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.addInitScript(() => {
    window.__showWindowCalls = 0;
    const backend = window.__e2eBackend;
    backend.override(async (command, request, payload, next) => {
      if (command === "get_ui_texts") return { ok: false, error: { code: "INTERNAL_ERROR", message: "catalog missing" } };
      if (command === "show_window") window.__showWindowCalls += 1;
    });
  });
  await page.goto("/");

  await expect(page.locator("body > .ui-texts-failed")).toHaveText(/couldn't load its texts/);
  await expect(page.locator("body > *")).toHaveCount(1);
  await expect.poll(() => page.evaluate(() => window.__showWindowCalls)).toBe(1);
});

test("in the test language, the USB pages, Backups, Event Log and Settings show only catalog text and data", async ({ page }) => {
  await installTauriMock(page, "valid");
  await page.addInitScript(() => {
    const backend = window.__e2eBackend;
    const historyTracks = [{ title: "Session Opener", artist: "Made-up Artist", durationMs: 200000 }];
    backend.override(async (command, request, payload, next) => {
      if (command === "browse_source_files") return backend.ok({ total: 0, items: [] });
      if (command === "get_transcode_folder") return backend.ok({ path: "/data/transcodes", isDefault: true });
      if (command === "fetch_usb_histories") {
        return backend.ok(backend.usbHistories({
          items: [{ id: "hist-1", name: "HISTORY 001", createdAt: "2026-08-01", tracks: historyTracks }],
          summaryText: "the histories summary"
        }));
      }
      if (command === "fetch_usb_history_tracks") {
        return backend.ok({ items: historyTracks, total: 1, listSummaryText: "the history list summary" });
      }
      if (command === "get_usb_player_menu_config") {
        return backend.ok(backend.playerMenuConfig({
          currentItems: [backend.playerMenuItem(132, { name: "PLAYLIST", removable: false, canMoveDown: true })],
          availableItems: [backend.playerMenuItem(133, { name: "BPM" })]
        }));
      }
      if (command === "list_usb_backups") {
        return backend.ok({ summaryText: "the summary", emptyText: "none", items: [{ timestamp: "2020-01-01_00-00-00", timeText: "the time", reasonText: "the reason", detailText: "the detail", restoreConfirmText: "Restore it?", deleteConfirmText: "Delete it?", files: ["export.pdb"], reason: "export", sizeBytes: 1024, location: "usb" }] });
      }
    });
  });
  await useTestLanguage(page);
  await page.goto("/");
  const check = uiTextCheck(page);
  await check.collect();
  // The USB page before a USB is chosen.
  await page.locator('.nav-item[data-view="usb"]').click();
  await expect(page.locator("#usbEmptyState")).toBeVisible();
  await check.collect();

  await connectUsb(page);
  await page.locator("#usbHealthCard").evaluate((node) => { node.open = true; });
  await check.collect();
  await page.locator("#previewRepairsBtn").click();
  await expect(page.locator("#diagRepairSummary")).toHaveText(/repairs 4 problems/);
  await page.locator("#diagRepairTechnical summary").click();
  await expect(page.locator("#diagRepairFixes")).toBeVisible();
  await check.collect();

  await page.locator('.nav-item[data-view="usb-playlists"]').click();
  await page.locator('[data-usb-playlist="usb-1"]').click();
  await expect(page.locator("#panel-usb-playlists")).toContainText("Track A");
  await check.collect();

  await page.locator('.nav-item[data-view="usb-history"]').click();
  await expect(page.locator("#panel-usb-history")).toContainText("Session Opener");
  await check.collect();

  await page.locator('.nav-item[data-view="usb-player-menu"]').click();
  await expect(page.locator("#panel-usb-player-menu")).toContainText("PLAYLIST");
  await check.collect();

  await page.locator('.nav-item[data-view="usb"]').click();
  await page.locator("#usbBackupsBtn").click();
  await expect(page.locator("#backupsSummary")).toHaveText("the summary");
  await check.collect();

  await page.locator("#eventLogBtn").click();
  await expect(page.locator("#panel-event-log")).toHaveClass(/active/);
  await check.collect();

  await page.locator("#settingsBtn").click();
  await expect(page.locator("#settingsDrawer")).toBeVisible();
  await check.collect();

  await check.expectNoEnglish();
});

// Backend log entries: level, source and spacing are normalized, so repeats coalesce; different
// messages with the same details don't; Clear empties the log.
test("backend log entries", async ({ page }) => {
  await installTauriMock(page, "valid");
  await listenForBackendEvents(page);
  await page.goto("/");
  await page.locator("#eventLogBtn").click();
  const rows = page.locator("#eventLogList .event-log-row");
  await page.evaluate(() => {
    const log = window.__emitBackendLog;
    log({ level: "WARNING", source: "USB-Diagnostics", code: "USB.DIAG.EMPTY", message: "  analysis file appears empty  " });
    log({ level: "warn", source: "usb-diagnostics", code: "usb.diag.empty", message: "analysis   file appears empty" });
    log({ level: "warn", source: "usb-diagnostics", code: "usb.diagnostics.unindexed-audio", message: "unindexed audio file: /Contents/A/one.mp3", details: "context: preview" });
    log({ level: "warn", source: "usb-diagnostics", code: "usb.diagnostics.unindexed-audio", message: "unindexed audio file: /Contents/B/two.mp3", details: "context: preview" });
  });
  const empty = rows.filter({ hasText: "analysis file appears empty" });
  await expect(empty).toHaveCount(1);
  await expect(empty.locator(".event-log-count")).toHaveText("x2");
  // The code lower-cased, the message trimmed.
  await expect(empty.locator(".event-log-message")).toHaveText("[usb.diag.empty] analysis file appears empty x2");
  await expect(rows.filter({ hasText: "unindexed audio file" })).toHaveCount(2);
  // "WARNING" and "USB-Diagnostics" are the warn level and the usb-diagnostics source.
  await page.locator("#eventLogLevelFilter").selectOption("warn");
  await page.locator("#eventLogSourceFilter").selectOption("usb-diagnostics");
  await expect(rows).toHaveCount(3);
  await page.locator("#eventLogLevelFilter").selectOption("all");
  await page.locator("#eventLogSourceFilter").selectOption("all");

  await page.locator("#eventLogClearBtn").click();
  await expect(page.locator("#eventLogSummary")).toHaveText("0 events");
  await expect(page.locator("#eventLogList")).toHaveText("No events");
});

// Logging: console.log/info go to the log file only, warnings (any arguments) and uncaught errors
// and rejections to the Event Log; a backend entry isn't written to the file again.
test("frontend logging", async ({ page }) => {
  await installTauriMock(page, "valid");
  await listenForBackendEvents(page);
  await page.goto("/");
  const fileLines = () => page.evaluate(() => window.__calls
    .filter((c) => c.command === "append_frontend_log")
    .map((c) => `${c.payload?.level ?? c.request?.level} ${c.payload?.message ?? c.request?.message}`));

  await page.evaluate(() => {
    console.log("file-only-log");
    console.info("file-only-info");
    console.warn("count:", 42, { key: "val" });
    setTimeout(() => { throw new Error("thrown-uncaught"); });
    Promise.reject(new Error("async failure"));
    // A Tauri command's error arrives as a plain string.
    Promise.reject("command refused");
    window.__emitBackendLog({ level: "warn", source: "backend", code: "backend.thing", message: "from the backend" });
  });
  await page.locator("#eventLogBtn").click();
  const list = page.locator("#eventLogList");
  await expect(list).toContainText('count: 42 {"key":"val"}');
  await expect(list).toContainText("thrown-uncaught");
  await expect(list).toContainText("async failure");
  await expect(list).toContainText("command refused");
  await expect(list).toContainText("from the backend");
  await expect(list).not.toContainText("file-only-log");
  await expect(list).not.toContainText("file-only-info");
  // Uncaught ones are errors.
  await page.locator("#eventLogLevelFilter").selectOption("error");
  await expect(list).toContainText("async failure");
  await expect(list).toContainText("thrown-uncaught");
  await expect(list).not.toContainText("count: 42");

  const lines = await fileLines();
  expect(lines).toContainEqual("debug [console] [console.event] file-only-log");
  expect(lines).toContainEqual("debug [console] [console.event] file-only-info");
  expect(lines).toContainEqual('warn [console] [console.event] count: 42 {"key":"val"}');
  expect(lines.some((line) => line.includes("from the backend"))).toBe(false);
});
