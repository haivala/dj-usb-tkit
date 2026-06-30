export function applySortToTracks(tableSortState, tracks, tbodyId, deps = {}) {
  const sortTracks = deps.sortTracks || ((items) => items);
  const st = tableSortState[tbodyId];
  if (!st) return tracks;
  return sortTracks(tracks, st.key, st.dir);
}

export function handleSortHeaderClick(tableSortState, event, deps = {}) {
  const {
    renderMap = {},
    bodyToRendererMap = {},
    doc = typeof document !== "undefined" ? document : null
  } = deps;
  const th = event?.target?.closest?.('.sortable[data-sort-key][role="columnheader"]');
  if (!th) return;
  const grid = th.closest("[data-track-grid]");
  const bodyId = grid?.dataset?.bodyId;
  if (!bodyId) return;

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
  if (isThisHeader) {
    const idx = cycle.findIndex((s) => s.key === current.key && s.dir === current.dir);
    if (idx >= 0 && idx < cycle.length - 1) {
      tableSortState[bodyId] = cycle[idx + 1];
    } else {
      delete tableSortState[bodyId];
    }
  } else {
    tableSortState[bodyId] = cycle[0];
  }

  grid.querySelectorAll('.sortable[role="columnheader"]').forEach((h) => {
    h.classList.remove("sort-asc", "sort-desc");
    const labelEl = h.querySelector(".sort-label");
    if (labelEl) labelEl.textContent = h.dataset.sortDefault || "";
  });

  const st = tableSortState[bodyId];
  const sortHint = grid.querySelector(".sort-hint");
  if (st) {
    const ownerTh = allKeys.includes(st.key)
      ? th
      : grid.querySelector(`[role="columnheader"][data-sort-key="${st.key}"]`);
    if (ownerTh) {
      ownerTh.classList.add(st.dir === "asc" ? "sort-asc" : "sort-desc");
      if (altKeys) {
        const labelEl = ownerTh.querySelector(".sort-label");
        if (labelEl) labelEl.textContent = st.key.charAt(0).toUpperCase() + st.key.slice(1);
      }
    }
    if (sortHint) sortHint.classList.remove("hidden");
  } else if (sortHint) {
    sortHint.classList.add("hidden");
  }

  const rendererName = bodyToRendererMap[bodyId];
  const renderer = rendererName ? renderMap[rendererName] : renderMap[bodyId];
  if (typeof renderer === "function") {
    renderer();
    return;
  }

  if (doc && typeof renderMap[bodyId] === "function") {
    renderMap[bodyId]();
  }
}

export function setActiveListItem(container, activeButton) {
  container.querySelectorAll("button").forEach((btn) => btn.classList.remove("active"));
  if (activeButton) activeButton.classList.add("active");
}

export function renderEmptyState(document, container, { icon, heading, body, actionLabel, onAction, extraActions = [] }) {
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
  container.innerHTML = "";
  container.appendChild(clone);
  const emptyStateEl = container.querySelector(".empty-state");
  for (const extra of extraActions) {
    if (!extra.label || !extra.onAction) continue;
    const btn = document.createElement("button");
    btn.className = "empty-state-action";
    btn.textContent = extra.label;
    btn.addEventListener("click", extra.onAction, { once: true });
    (emptyStateEl || container).appendChild(btn);
  }
}
