import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { clearTrackSort, handleSortHeaderClick } from "../components/shell/actions.mjs";

function buildGrid(dom, { sortLocked = false } = {}) {
  const document = dom.window.document;
  document.body.innerHTML = `
    <div data-track-grid data-body-id="playlistTracksBody" ${sortLocked ? 'data-sort-locked="true"' : ""}>
      <div class="track-grid-cell th-waveform" role="columnheader"><span class="sort-hint hidden">sorted by &rarr;</span></div>
      <div class="track-grid-cell sortable" role="columnheader" data-sort-key="album">Album</div>
    </div>
  `;
  return {
    grid: document.querySelector("[data-track-grid]"),
    albumHeader: document.querySelector('.sortable[data-sort-key="album"]'),
    sortHint: document.querySelector(".sort-hint")
  };
}

test("clearTrackSort deletes the sort state and resets header classes/hint", () => {
  const dom = new JSDOM("<!doctype html><body></body>");
  const { grid, albumHeader, sortHint } = buildGrid(dom);
  albumHeader.classList.add("sort-asc");
  sortHint.classList.remove("hidden");
  const tableSortState = { playlistTracksBody: { key: "album", dir: "asc" } };

  clearTrackSort(tableSortState, "playlistTracksBody", grid);

  assert.equal(tableSortState.playlistTracksBody, undefined);
  assert.equal(albumHeader.classList.contains("sort-asc"), false);
  assert.equal(sortHint.classList.contains("hidden"), true);
});

test("clearTrackSort tolerates a missing grid (no DOM to reset)", () => {
  const tableSortState = { playlistTracksBody: { key: "album", dir: "asc" } };
  assert.doesNotThrow(() => clearTrackSort(tableSortState, "playlistTracksBody", null));
  assert.equal(tableSortState.playlistTracksBody, undefined);
});

test("handleSortHeaderClick is a no-op when the grid is sort-locked", () => {
  const dom = new JSDOM("<!doctype html><body></body>");
  const { albumHeader, sortHint } = buildGrid(dom, { sortLocked: true });
  const tableSortState = {};
  let renderCalls = 0;

  handleSortHeaderClick(tableSortState, { target: albumHeader }, {
    renderMap: { playlistTracksBody: () => { renderCalls += 1; } },
    bodyToRendererMap: {}
  });

  assert.equal(tableSortState.playlistTracksBody, undefined);
  assert.equal(albumHeader.classList.contains("sort-asc"), false);
  assert.equal(sortHint.classList.contains("hidden"), true);
  assert.equal(renderCalls, 0);
});

test("handleSortHeaderClick still sorts when the grid is not locked", () => {
  const dom = new JSDOM("<!doctype html><body></body>");
  const { albumHeader, sortHint } = buildGrid(dom, { sortLocked: false });
  const tableSortState = {};
  let renderCalls = 0;

  handleSortHeaderClick(tableSortState, { target: albumHeader }, {
    renderMap: { playlistTracksBody: () => { renderCalls += 1; } },
    bodyToRendererMap: {}
  });

  assert.deepEqual(tableSortState.playlistTracksBody, { key: "album", dir: "asc" });
  assert.equal(albumHeader.classList.contains("sort-asc"), true);
  assert.equal(sortHint.classList.contains("hidden"), false);
  assert.equal(renderCalls, 1);
});
