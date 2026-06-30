export function renderPlaylistTabsAndPanels(state, el, deps) {
  const {
    document,
    escapeHtml,
    formatPlaylistExportStatus,
    renderPlaylistTabContent,
    getPlaylistTabDomId
  } = deps;

  el.tabsContainer.querySelectorAll(".tab.playlist-tab").forEach((btn) => btn.remove());
  el.tabsContainer.querySelectorAll(".tab-new-input-wrap").forEach((wrap) => wrap.remove());
  el.playlistPanels.innerHTML = "";

  state.playlists.forEach((playlist) => {
    const canExportToUsb = !!state.usbRoot && !!state.usbRootValid;
    const exportDisabledAttr = canExportToUsb ? "" : " disabled";
    const exportTitle = canExportToUsb
      ? "Export current playlist to selected USB"
      : "Select a valid USB folder first";
    const playlistTabId = getPlaylistTabDomId(playlist.id);
    const playlistPanelId = `panel-${playlist.id}`;
    const tab = document.createElement("button");
    tab.className = "tab playlist-tab";
    tab.id = playlistTabId;
    tab.dataset.tab = playlist.id;
    tab.setAttribute("role", "tab");
    tab.setAttribute("aria-selected", "false");
    tab.setAttribute("aria-controls", playlistPanelId);
    tab.innerHTML = renderPlaylistTabContent(playlist);
    el.tabsContainer.insertBefore(tab, el.addPlaylistTab);

    const panel = document.createElement("section");
    panel.className = "panel";
    panel.id = playlistPanelId;
    panel.setAttribute("role", "tabpanel");
    panel.setAttribute("aria-hidden", "true");
    panel.setAttribute("aria-labelledby", playlistTabId);
    panel.innerHTML = `
      <section class="card">
        <h2>${escapeHtml(playlist.name)}</h2>
        <div class="row search-row">
          <input data-playlist-search="${playlist.id}" value="${escapeHtml(state.playlistTrackSearch || "")}" placeholder="Search playlist tracks" />
        </div>
        <div class="table-wrap">
          <table>
            <thead>
              <tr>
                <th>Cover</th>
                <th>Waveform</th>
                <th>No.</th>
                <th>Title</th>
                <th>Artist</th>
                <th>Album</th>
                <th>Added</th>
                <th>Format</th>
                <th>Length</th>
                <th>BPM</th>
                <th>Key</th>
                <th>Action</th>
              </tr>
            </thead>
            <tbody data-playlist-tracks="${playlist.id}"></tbody>
          </table>
        </div>
        <p class="muted track-total-time" data-playlist-total-duration="${playlist.id}">Total time: 0:00</p>
        <div class="row playlist-actions">
          <p class="muted playlist-export-status">${escapeHtml(formatPlaylistExportStatus(playlist))}</p>
          <button data-export-playlist="${playlist.id}" title="${escapeHtml(exportTitle)}"${exportDisabledAttr}>Export to USB</button>
        </div>
      </section>
    `;
    el.playlistPanels.appendChild(panel);
  });
}

function resolveEmitStatus(deps = {}) {
  if (typeof deps.emitStatus === "function") return deps.emitStatus;
  if (typeof deps.setStatus === "function") return deps.setStatus;
  return () => {};
}

export function renderPlaylistList(state, el, deps) {
  const { document, renderPlaylistSidebarItemContent } = deps;
  el.navPlaylistList.querySelectorAll(".nav-playlist-item").forEach((item) => item.closest("li")?.remove());
  el.navPlaylistList.querySelectorAll(".nav-new-input-wrap").forEach((wrap) => wrap.remove());

  [...state.playlists].reverse().forEach((playlist) => {
    const li = document.createElement("li");
    const btn = document.createElement("button");
    btn.className = "nav-playlist-item";
    btn.dataset.playlistId = playlist.id;
    btn.innerHTML = renderPlaylistSidebarItemContent(playlist);
    if (state.activeTab === playlist.id) btn.classList.add("active");
    if (state.currentPlaylistId === playlist.id) {
      btn.classList.add("playlist-active-mode");
      btn.title = "Active playlist";
    }
    li.appendChild(btn);
    el.navPlaylistList.appendChild(li);
  });
}

export function promptNewPlaylist(el, deps) {
  const {
    document,
    requestAnimationFrame,
    createPlaylist,
    setStatus
  } = deps;
  const emitStatus = resolveEmitStatus(deps);
  const existing = el.navPlaylistList.querySelector(".nav-new-input-wrap");
  if (existing) {
    existing.querySelector(".nav-new-input")?.focus();
    return;
  }

  el.addPlaylistBtn.classList.add("hidden");

  const wrap = document.createElement("li");
  wrap.className = "nav-new-input-wrap";

  const input = document.createElement("input");
  input.className = "nav-new-input";
  input.type = "text";
  input.placeholder = "Playlist name...";

  const cancel = document.createElement("button");
  cancel.className = "nav-new-cancel";
  cancel.textContent = "\u00d7";
  cancel.type = "button";

  wrap.appendChild(input);
  wrap.appendChild(cancel);
  const addItem = el.navPlaylistList.querySelector(".nav-playlist-add-item");
  if (addItem?.nextSibling) {
    el.navPlaylistList.insertBefore(wrap, addItem.nextSibling);
  } else {
    el.navPlaylistList.appendChild(wrap);
  }

  const cleanup = () => {
    wrap.remove();
    el.addPlaylistBtn.classList.remove("hidden");
  };

  const submit = createSingleSubmit(() => {
    const name = input.value.trim();
    cleanup();
    if (name) {
      createPlaylist(name).catch((err) => {
        console.error(err);
        emitStatus(err.message || String(err));
      });
    }
  });

  input.addEventListener("keydown", (event) => {
    if (event.key === "Enter") { event.preventDefault(); submit(); }
    if (event.key === "Escape") { event.preventDefault(); cleanup(); }
  });
  input.addEventListener("blur", () => {
    setTimeout(() => {
      if (document.activeElement !== cancel) submit();
    }, 80);
  });
  cancel.addEventListener("click", (event) => {
    event.preventDefault();
    cleanup();
  });

  requestAnimationFrame(() => input.focus());
}

export function startPlaylistRename(playlistId, state, el, deps) {
  const {
    document,
    requestAnimationFrame,
    command,
    setStatus,
    renderPlaylistSidebarItemContent,
    getCurrentPlaylist,
    formatPlaylistExportStatus
  } = deps;
  const emitStatus = resolveEmitStatus(deps);
  const playlist = state.playlists.find((item) => item.id === playlistId);
  if (!playlist) return;

  const item = el.navPlaylistList.querySelector(`.nav-playlist-item[data-playlist-id="${playlistId}"]`);
  if (!item) return;

  const originalName = playlist.name;
  const input = document.createElement("input");
  input.className = "nav-rename-input";
  input.type = "text";
  input.value = originalName;

  item.textContent = "";
  item.appendChild(input);

  let finished = false;
  const finish = async (save) => {
    if (finished) return;
    finished = true;
    const newName = input.value.trim();
    if (save && newName && newName !== originalName) {
      try {
        const data = await command("rename_playlist", { playlistId, name: newName });
        playlist.name = data.name;
        playlist.lastExportedAt = null;
        playlist.lastExportedUsbRoot = null;
        playlist.lastExportedTrackCount = null;
      } catch (err) {
        console.error("rename failed:", err);
        emitStatus(`Rename failed: ${err.message || err}`);
      }
    }
    item.innerHTML = renderPlaylistSidebarItemContent(playlist);
    if (state.activeTab === playlistId) {
      el.playlistPanelTitle.textContent = playlist.name;
      el.playlistExportStatus.textContent = formatPlaylistExportStatus(playlist);
    }
    const badge = getCurrentPlaylist();
    if (badge?.id === playlistId) {
      el.badgeLabel.textContent = playlist.name;
    }
  };

  input.addEventListener("keydown", (event) => {
    if (event.key === "Enter") { event.preventDefault(); finish(true); }
    if (event.key === "Escape") { event.preventDefault(); finish(false); }
  });
  input.addEventListener("blur", () => finish(true));

  requestAnimationFrame(() => {
    input.focus();
    input.select();
  });
}

export function formatPlaylistExportStatus(playlist, deps) {
  const { formatTimestampLocal } = deps;
  const when = String(playlist?.lastExportedAt || "").trim();
  if (!when) return "Not exported yet.";
  const formattedWhen = formatTimestampLocal(when);
  const root = String(playlist?.lastExportedUsbRoot || "").trim();
  const count = Number(playlist?.lastExportedTrackCount);
  const countText = Number.isFinite(count) && count >= 0 ? `${count} track(s)` : "unknown track count";
  const rootText = root ? ` to ${root}` : "";
  return `Last exported ${formattedWhen}${rootText} (${countText}).`;
}

export async function loadPlaylists(state, deps) {
  const { command, renderPlaylistTabsAndPanels, updatePlaylistExportButtons } = deps;
  const data = await command("list_playlists");
  state.playlists = (data.items || []).map((playlist) => ({ ...playlist, tracks: [] }));
  renderPlaylistTabsAndPanels();
  updatePlaylistExportButtons();
}

export async function refreshCurrentPlaylistTracks(state, el, deps) {
  const {
    getCurrentPlaylist,
    command,
    normalizeTrack,
    filterTracksByQuery,
    renderEmptyState,
    applySortToTracks,
    renderTrackTable,
    updateTrackListDurationSummary,
    updatePlaylistPanelTitle,
    updatePlaylistExportButtons,
    renderPlaylistList
  } = deps;
  const playlist = getCurrentPlaylist();
  if (!playlist) return;

  const data = await command("get_playlist_tracks", { playlistId: playlist.id });
  playlist.tracks = (data.items || []).map((track) => normalizeTrack(track, "plt"));
  state.currentPlaylistTracksView = filterTracksByQuery(playlist.tracks, state.playlistTrackSearch);
  if (el.playlistSearchInput && el.playlistSearchInput.value !== state.playlistTrackSearch) {
    el.playlistSearchInput.value = state.playlistTrackSearch;
  }

  const playlistEmpty = !playlist.tracks.length;
  if (el.playlistEmptyState) {
    el.playlistEmptyState.innerHTML = "";
    if (playlistEmpty) {
      renderEmptyState(el.playlistEmptyState, {
        icon: "\u266B",
        heading: "Browse Library or USB to add tracks"
      });
    }
  }
  if (el.playlistTableWrap) {
    el.playlistTableWrap.classList.toggle("hidden", playlistEmpty);
  }
  el.playlistTotalDuration?.classList.toggle("hidden", playlistEmpty);
  el.exportPlaylistBtn?.closest(".playlist-actions")?.classList.toggle("hidden", playlistEmpty);

  const sortedTracks = applySortToTracks(state.currentPlaylistTracksView, "playlistTracksBody");
  renderTrackTable(el.playlistTracksBody, sortedTracks, {
    withCheckbox: false,
    origin: "local",
    secondaryActionLabel: "Play",
    secondaryActionType: "play-library",
    enableAnalyzeActions: false,
    actionLabel: "\u00d7",
    actionType: "remove-playlist-track",
    compactAddButton: true
  });
  updateTrackListDurationSummary(el.playlistTotalDuration, state.currentPlaylistTracksView);
  updatePlaylistPanelTitle(playlist);
  updatePlaylistExportButtons();
  renderPlaylistList();
}

export function updatePlaylistExportButtons(state, el, deps) {
  const {
    getCurrentPlaylist,
    computeExportButtonState,
    isUsbOriginTrack,
    trackHasCoreAnalysis
  } = deps;
  const current = getCurrentPlaylist();
  const buttonState = computeExportButtonState({
    usbRoot: state.usbRoot,
    usbRootValid: state.usbRootValid,
    exportPruneStale: state.exportPruneStale,
    currentPlaylistName: current?.name,
    knownUsbPlaylistNames: state.usbKnownPlaylistNames
  });

  el.exportPlaylistBtn.disabled = false;
  el.exportPlaylistBtn.textContent = buttonState.text;
  el.exportPlaylistBtn.title = buttonState.title;

  const analyzeCandidates = Array.isArray(current?.tracks)
    ? current.tracks.filter((track) => !isUsbOriginTrack(track) && !trackHasCoreAnalysis(track))
    : [];
  const showAnalyzeMissing = analyzeCandidates.length > 0;
  if (el.analyzePlaylistMissingBtn) {
    el.analyzePlaylistMissingBtn.disabled = !showAnalyzeMissing;
    el.analyzePlaylistMissingBtn.hidden = !showAnalyzeMissing;
    el.analyzePlaylistMissingBtn.textContent = showAnalyzeMissing
      ? `Analyze Missing Tracks (${analyzeCandidates.length})`
      : "Analyze Missing Tracks";
    el.analyzePlaylistMissingBtn.title = showAnalyzeMissing
      ? "Analyze missing waveform, BPM, and duration for local non-USB tracks in this playlist"
      : "No local non-USB tracks in this playlist need analysis";
  }
  if (el.exportPlaylistBtn) {
    el.exportPlaylistBtn.hidden = showAnalyzeMissing;
  }
}

export async function createPlaylist(name, deps) {
  const {
    setStatus,
    withProgress,
    command,
    loadPlaylists,
    state,
    updateModeText,
    switchTab
  } = deps;
  const emitStatus = resolveEmitStatus(deps);
  if (!name) {
    emitStatus("Playlist name is required");
    return;
  }

  const created = await withProgress("Creating playlist", async (progress) => {
    progress(35, "Saving playlist...");
    const playlist = await command("create_playlist", { name });
    progress(70, "Refreshing playlists...");
    await loadPlaylists();
    const loadedPlaylists = Array.isArray(state.playlists) ? state.playlists : [];
    const createdPlaylistId = String(playlist?.playlistId || "").trim();
    const selectedPlaylist = loadedPlaylists.find((item) => String(item?.id || "") === createdPlaylistId)
      || (createdPlaylistId
        ? null
        : loadedPlaylists.find((item) => String(item?.name || "") === String(playlist?.name || "")))
      || loadedPlaylists[loadedPlaylists.length - 1]
      || null;
    const selectedPlaylistId = selectedPlaylist?.id || createdPlaylistId;
    state.currentPlaylistId = selectedPlaylistId;
    updateModeText();
    if (selectedPlaylistId) {
      await switchTab(selectedPlaylistId);
    }
    return playlist;
  });

  emitStatus(`Playlist created: ${created.name}`);
}

export async function deletePlaylist(playlistId, deps) {
  const {
    state,
    openConfirmDialog,
    command,
    loadPlaylists,
    updateModeText,
    switchTab,
    setStatus
  } = deps;
  const emitStatus = resolveEmitStatus(deps);

  if (!playlistId || state.deletingPlaylistId === playlistId) return;
  const playlist = state.playlists.find((p) => p.id === playlistId);
  if (!playlist) return;

  const exportedHint = playlist.lastExportedAt
    ? "\n\nThis playlist was exported before. It is easy to recreate by importing playlists from USB."
    : "";
  const confirmed = await openConfirmDialog({
    title: "Delete App Playlist",
    message: `Delete "${playlist.name}"?\n\nThis removes the app playlist and its app playlist-track links.${exportedHint}`,
    confirmLabel: "Delete"
  });
  if (!confirmed) return;

  state.deletingPlaylistId = playlistId;
  try {
    const data = await command("delete_playlist", { playlistId });
    if (!data?.deleted) {
      emitStatus(`Delete failed: ${playlist.name}`);
      return;
    }
    await loadPlaylists();
    if (state.currentPlaylistId === playlistId) {
      state.currentPlaylistId = state.playlists.at(-1)?.id || null;
    }
    updateModeText();
    await switchTab(state.currentPlaylistId || "library");
    emitStatus(`Playlist deleted: ${playlist.name}`);
  } finally {
    state.deletingPlaylistId = null;
  }
}

export async function addTracksToCurrentPlaylist(tracks, deps) {
  const {
    requireCurrentPlaylist,
    resolveLocalTrackId,
    resolveLocalTrackIdAsync,
    shouldAllowResolvedFallback,
    pushEventLog,
    setStatus,
    withProgress,
    command,
    refreshCurrentPlaylistTracks
  } = deps;
  const emitStatus = resolveEmitStatus(deps);

  const playlist = requireCurrentPlaylist();
  if (!playlist) return;

  const trackIds = [];
  for (const track of tracks) {
    let id = resolveLocalTrackId(track);
    if (!id
      && typeof resolveLocalTrackIdAsync === "function"
      && (typeof shouldAllowResolvedFallback !== "function" || shouldAllowResolvedFallback(track))) {
      id = await resolveLocalTrackIdAsync(track);
    }
    if (typeof pushEventLog === "function") {
      pushEventLog({
        level: "info",
        source: "playlist-add",
        code: "playlist_add.resolve",
        message: `Resolved add-track candidate: ${track?.title || "Unknown Title"}`,
        details: `origin=${track?.usbAnalysisPath ? "usb" : "local"} | resolvedTrackId=${id || "none"} | localTrackId=${track?.localTrackId || "none"} | filePath=${track?.filePath || ""}`
      });
    }
    if (id) trackIds.push(id);
  }

  if (!trackIds.length) {
    emitStatus("No imported track IDs found to add");
    return;
  }

  const result = await withProgress("Adding tracks", async (progress) => {
    progress(40, "Writing playlist entries...");
    if (typeof pushEventLog === "function") {
      pushEventLog({
        level: "info",
        source: "playlist-add",
        code: "playlist_add.request",
        message: `Adding ${trackIds.length} track(s) to ${playlist.name}`,
        details: `trackIds=${trackIds.join(",")}`
      });
    }
    const add = await command("add_tracks_to_playlist", {
      playlistId: playlist.id,
      trackIds,
      dedupe: "skip"
    });
    progress(80, "Refreshing playlist...");
    await refreshCurrentPlaylistTracks();
    if (typeof pushEventLog === "function") {
      pushEventLog({
        level: "info",
        source: "playlist-add",
        code: "playlist_add.result",
        message: `Added ${add.added} track(s) to ${playlist.name}`,
        details: `requested=${trackIds.length} | added=${add.added} | skipped=${add.skipped}`
      });
    }
    return add;
  });
  emitStatus(`Added ${result.added} tracks (skipped ${result.skipped}) to ${playlist.name}`);
}

export function createSingleSubmit(handler) {
  let submitted = false;
  return () => {
    if (submitted) return false;
    submitted = true;
    handler();
    return true;
  };
}

export function renderPlaylistSidebarItemContent(playlist, deps) {
  const { escapeHtml } = deps;
  const statusIcon = playlist.lastExportedAt ? "\u2713" : "";
  const statusClass = playlist.lastExportedAt ? " exported" : "";
  const statusTitle = playlist.lastExportedAt ? "Exported to USB" : "";
  return `
    <span class="nav-playlist-name">${escapeHtml(playlist.name)}</span>
    <span class="nav-playlist-status${statusClass}"${statusTitle ? ` title="${statusTitle}"` : ""}>${statusIcon}</span>
    <span class="nav-playlist-delete" data-delete-playlist="${playlist.id}" title="Delete playlist" aria-label="Delete playlist" role="button" tabindex="0">&times;</span>
  `;
}

export function updatePlaylistPanelTitle(el, playlist, deps) {
  const { formatDurationMs } = deps;
  if (!el?.playlistPanelTitle || !playlist) return;
  const tracks = Array.isArray(playlist.tracks) ? playlist.tracks : [];
  const count = tracks.length;
  const durations = tracks
    .map((t) => Number(t?.durationMs))
    .filter((v) => Number.isFinite(v) && v > 0);
  const totalMs = durations.reduce((s, v) => s + v, 0);
  const parts = [playlist.name];
  if (count > 0) {
    parts.push(`(${count} track${count !== 1 ? "s" : ""}, Total time: ${formatDurationMs(totalMs)})`);
  }
  el.playlistPanelTitle.textContent = parts.join(" ");
}

export function populatePlaylistPanel(el, state, playlist, deps) {
  const { updatePlaylistPanelTitle, formatPlaylistExportStatus, updatePlaylistExportButtons } = deps;
  if (!playlist) return;
  updatePlaylistPanelTitle(playlist);
  el.playlistExportStatus.textContent = formatPlaylistExportStatus(playlist);
  updatePlaylistExportButtons();
  el.playlistSearchInput.value = state.playlistTrackSearch || "";
}
