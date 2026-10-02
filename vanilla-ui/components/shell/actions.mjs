export function clearTrackSort(tableSortState, bodyId, grid) {
  delete tableSortState[bodyId];
  if (!grid) return;
  grid.querySelectorAll('.sortable[role="columnheader"]').forEach((h) => {
    h.classList.remove("sort-asc", "sort-desc");
    const labelEl = h.querySelector(".sort-label");
    if (labelEl) labelEl.textContent = h.dataset.sortDefault || "";
  });
  grid.querySelector(".sort-hint")?.classList.add("hidden");
}

// The track-list controllers, keyed by the grid's `data-body-id`.
export function trackListControllerFor(ctx, bodyId) {
  return [ctx.libraryTracksCtl, ctx.playlistTracksCtl, ctx.usbPlaylistTracksCtl, ctx.usbHistoryTracksCtl]
    .find((ctl) => ctl?.bodyId === bodyId) || null;
}

export function handleSortHeaderClick(ctx, event) {
  const { tableSortState } = ctx;
  const th = event?.target?.closest?.('.sortable[data-sort-key][role="columnheader"]');
  if (!th) return;
  const grid = th.closest("[data-track-grid]");
  const bodyId = grid?.dataset?.bodyId;
  if (!bodyId) return;
  if (grid.dataset.sortLocked === "true") return;

  const key = th.dataset.sortKey;
  const altKeys = th.dataset.sortAlt ? th.dataset.sortAlt.split(",") : null;
  const current = tableSortState[bodyId];

  const cycle = [{ key, dir: "asc" }, { key, dir: "desc" }];
  if (altKeys) {
    for (const ak of altKeys) {
      cycle.push({ key: ak, dir: "asc" }, { key: ak, dir: "desc" });
    }
  }

  const allKeys = [key, ...(altKeys || [])];
  const isThisHeader = current && allKeys.includes(current.key);
  let nextState = null;
  if (isThisHeader) {
    const idx = cycle.findIndex((s) => s.key === current.key && s.dir === current.dir);
    if (idx >= 0 && idx < cycle.length - 1) {
      nextState = cycle[idx + 1];
    }
  } else {
    nextState = cycle[0];
  }

  if (!nextState) {
    clearTrackSort(tableSortState, bodyId, grid);
  } else {
    tableSortState[bodyId] = nextState;
    grid.querySelectorAll('.sortable[role="columnheader"]').forEach((h) => {
      h.classList.remove("sort-asc", "sort-desc");
      const labelEl = h.querySelector(".sort-label");
      if (labelEl) labelEl.textContent = h.dataset.sortDefault || "";
    });
    const ownerTh = allKeys.includes(nextState.key)
      ? th
      : grid.querySelector(`[role="columnheader"][data-sort-key="${nextState.key}"]`);
    if (ownerTh) {
      ownerTh.classList.add(nextState.dir === "asc" ? "sort-asc" : "sort-desc");
      if (altKeys) {
        const labelEl = ownerTh.querySelector(".sort-label");
        if (labelEl) labelEl.textContent = nextState.key.charAt(0).toUpperCase() + nextState.key.slice(1);
      }
    }
    grid.querySelector(".sort-hint")?.classList.remove("hidden");
  }

  // Re-query page 1 with the new sortBy, so the sort spans the whole list.
  trackListControllerFor(ctx, bodyId)?.applyHeaderSort();
}

export function setActiveListItem(container, activeButton) {
  container.querySelectorAll("button").forEach((btn) => btn.classList.remove("active"));
  if (activeButton) activeButton.classList.add("active");
}

export function renderEmptyState(container, { icon, heading, body, actionLabel, onAction, extraActions = [] }) {
  const document = container.ownerDocument;
  const tpl = document.getElementById("emptyStateTemplate");
  if (!tpl) return;
  const clone = tpl.content.cloneNode(true);
  const iconEl = clone.querySelector(".empty-state-icon");
  const headingEl = clone.querySelector(".empty-state-heading");
  const bodyEl = clone.querySelector(".empty-state-body");
  const actionEl = clone.querySelector(".empty-state-action");
  if (iconEl) iconEl.textContent = icon || "";
  if (headingEl) headingEl.textContent = heading || "";
  if (bodyEl) bodyEl.textContent = body || "";
  if (actionLabel && onAction && actionEl) {
    actionEl.textContent = actionLabel;
    actionEl.classList.remove("hidden");
    actionEl.addEventListener("click", onAction, { once: true });
  }
  container.replaceChildren();
  container.appendChild(clone);
  const actionsEl = container.querySelector(".empty-state-actions") || container;
  for (const extra of extraActions) {
    if (!extra.label || !extra.onAction) continue;
    // A secondary copy of the template's action button.
    const btn = tpl.content.querySelector(".empty-state-action").cloneNode(true);
    btn.classList.remove("hidden");
    btn.removeAttribute("data-primary");
    btn.textContent = extra.label;
    btn.addEventListener("click", extra.onAction, { once: true });
    actionsEl.appendChild(btn);
  }
}
