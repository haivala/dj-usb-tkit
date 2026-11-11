import { test, expect, openUsb } from "./coverage-fixture.mjs";

function installSortMock(page) {
  return page.addInitScript(() => {
    window.__e2eSourceFolders = ["/music"];
    window.__e2eBackend.seed({
      tracks: [
        { id: "t1", title: "Zeta", artist: "Bob", album: "A", year: 2021, filePath: "/music/zeta.mp3" },
        { id: "t2", title: "Alpha", artist: "Bob", album: "A", year: 1994, filePath: "/music/alpha.mp3" },
        { id: "t3", title: "Beta", artist: "Alice", album: "A", filePath: "/music/beta.mp3" },
        { id: "t4", title: "Gamma", artist: "Cara", album: "A", year: 2008, filePath: "/music/gamma.mp3" }
      ]
    });
    window.__e2eBackend.install();
  });
}

async function rowTitles(page) {
  return page.locator("#libraryTableBody .track-grid-row .track-title").allTextContents();
}

test("sort cycling and the Year column", async ({ page }) => {
  await installSortMock(page);
  await page.goto("/");
  // Sort cycling on Track column.
  {
    await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(4);

    const header = page.locator('#panel-library .sortable[data-sort-key="artist"]');

    await header.click();
    await expect.poll(() => rowTitles(page)).toEqual(["Beta", "Alpha", "Zeta", "Gamma"]);

    await header.click();
    await expect.poll(() => rowTitles(page)).toEqual(["Gamma", "Zeta", "Alpha", "Beta"]);

    await header.click();
    await expect.poll(() => rowTitles(page)).toEqual(["Alpha", "Beta", "Gamma", "Zeta"]);

    await header.click();
    await expect.poll(() => rowTitles(page)).toEqual(["Zeta", "Gamma", "Beta", "Alpha"]);

    await header.click();
    await expect.poll(() => rowTitles(page)).toEqual(["Zeta", "Alpha", "Beta", "Gamma"]);
  }

  await page.reload();
  // Year column shows the release year and sorts by it.
  {
    await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(4);
    await expect(page.locator("#libraryTableBody .track-grid-row .td-year"))
      .toHaveText(["2021", "1994", "", "2008"]);

    await page.locator('#panel-library .sortable[data-sort-key="year"]').click();
    await expect.poll(() => rowTitles(page)).toEqual(["Beta", "Alpha", "Gamma", "Zeta"]);
  }
});


// The library, plus a USB playlist and a USB history (sorted by the request) to see the
// one column layout in every list. `trackColumns`: a layout saved before the app starts.
function installColumnsMock(page, trackColumns) {
  return page.addInitScript((trackColumns) => {
    window.__e2eSourceFolders = ["/music"];
    const backend = window.__e2eBackend;
    const track = (id, title, bpm, genre) => ({ id, title, artist: "Bob", album: "A", genre, year: 2000, bpm, filePath: `/music/${id}.mp3` });
    backend.seed({ tracks: [track("t1", "One", 120, "Techno"), track("t2", "Two", 90, "House")], trackColumns });
    backend.install();
    const usbRows = [track("u1", "Usb One", 128)];
    const historyRows = [track("h1", "Late", 140), track("h2", "Early", 100), track("h3", "Mid", 120)];
    backend.override(async (command, request) => {
      if (command === "fetch_usb_playlists") {
        return backend.ok(backend.usbPlaylists({ items: [{ id: "usb-1", name: "Warmup", source: "mock", tracks: usbRows }] }));
      }
      if (command === "fetch_usb_playlist_tracks") return backend.ok(backend.listPage(usbRows, request));
      if (command === "fetch_usb_histories") {
        return backend.ok(backend.usbHistories({ items: [{ id: "h-1", name: "HISTORY 001", tracks: historyRows }] }));
      }
      if (command === "fetch_usb_history_tracks") return backend.ok(backend.listPage(historyRows, request));
    });
  }, trackColumns);
}

// The shown header cells of a grid, by column id (their td-* class).
function shownColumns(page, bodyId) {
  return page.locator(`[data-body-id="${bodyId}"] .track-grid-head .track-grid-row`).evaluate((row) =>
    [...row.children].filter((cell) => !cell.hidden)
      .map((cell) => [...cell.classList].find((name) => name.startsWith("td-")).slice(3)));
}

// The shown cells of a grid's first track row, by column id.
function shownRowCells(page, bodyId) {
  return page.locator(`#${bodyId} .track-grid-row[data-track-id]`).first().evaluate((row) =>
    [...row.children].filter((cell) => !cell.hidden)
      .map((cell) => [...cell.classList].find((name) => name.startsWith("td-")).slice(3)));
}

const historyTitles = (page) => page.locator("#historyTracks .track-grid-row .track-title").allTextContents();

test("the column editor hides and reorders the columns of every track list", async ({ page }) => {
  await installColumnsMock(page);
  await page.goto("/");
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(2);
  const all = ["select", "cover", "waveform", "track", "album", "year", "bpm", "key", "length", "format", "action"];
  expect(await shownColumns(page, "libraryTableBody")).toEqual(all);

  // Sort the history by BPM before the column goes.
  await openUsb(page, "usb-history");
  await expect.poll(() => historyTitles(page)).toEqual(["Late", "Early", "Mid"]);
  const historyBpm = page.locator('[data-body-id="historyTracks"] .sortable[data-sort-key="bpm"]');
  await historyBpm.click();
  await expect.poll(() => historyTitles(page)).toEqual(["Early", "Mid", "Late"]);

  await page.locator('.nav-item[data-view="library"]').click();
  await page.locator('#panel-library [data-track-columns-button]').click();
  const dialog = page.locator("#trackColumnsOverlay");
  await expect(dialog).toBeVisible();
  const item = (id) => page.locator(`#trackColumnsList [data-track-column="${id}"]`);
  await expect(page.locator("#trackColumnsList .add-to-name")).toHaveText(["Album", "Genre", "Year", "BPM", "Key", "Length", "Fmt"]);
  // Genre starts hidden.
  await expect(item("genre").locator("input")).not.toBeChecked();

  // Hide Album: gone from the library and from the USB lists too.
  await item("album").locator("input").uncheck();
  await expect.poll(() => shownColumns(page, "libraryTableBody")).toEqual(all.filter((id) => id !== "album"));
  expect(await shownRowCells(page, "libraryTableBody")).toEqual(all.filter((id) => id !== "album"));
  expect(await shownColumns(page, "historyTracks")).toEqual(all.filter((id) => id !== "album"));
  expect(await shownColumns(page, "usbPlaylistTracks")).toEqual(["drag", ...all.filter((id) => id !== "album")]);

  // BPM before Year, in the header and the rows.
  const handle = item("bpm").locator("[data-track-column-drag-handle]");
  const from = await handle.boundingBox();
  const to = await item("year").boundingBox();
  await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
  await page.mouse.down();
  await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2 - 5, { steps: 5 });
  await page.mouse.move(to.x + to.width / 2, to.y + to.height * 0.25, { steps: 10 });
  await page.mouse.up();
  const reordered = ["select", "cover", "waveform", "track", "bpm", "year", "key", "length", "format", "action"];
  await expect.poll(() => shownColumns(page, "libraryTableBody")).toEqual(reordered);
  expect(await shownRowCells(page, "libraryTableBody")).toEqual(reordered);
  expect(await page.evaluate(() => window.__calls.filter((c) => c.command === "set_track_columns").at(-1).request))
    .toEqual({ layout: { order: ["album", "genre", "bpm", "year", "key", "length", "format"], hidden: ["album", "genre"] } });

  // Show Genre: its cells hold the genre, and it sorts.
  await item("genre").locator("input").check();
  await expect.poll(() => shownColumns(page, "libraryTableBody"))
    .toEqual(["select", "cover", "waveform", "track", "genre", "bpm", "year", "key", "length", "format", "action"]);
  await expect(page.locator("#libraryTableBody .track-grid-row .td-genre")).toHaveText(["Techno", "House"]);
  await page.locator("#trackColumnsDoneBtn").click();
  await page.locator('#panel-library .sortable[data-sort-key="genre"]').click();
  await expect.poll(() => rowTitles(page)).toEqual(["Two", "One"]);
  await page.locator('#panel-library [data-track-columns-button]').click();

  // Hide BPM: the history sorted by it goes back to its own order, its arrow gone.
  await item("bpm").locator("input").uncheck();
  await page.locator("#trackColumnsDoneBtn").click();
  await expect(dialog).toBeHidden();
  await page.locator('.nav-item[data-view="usb-history"]').click();
  await expect.poll(() => historyTitles(page)).toEqual(["Late", "Early", "Mid"]);
  await expect(page.locator('[data-body-id="historyTracks"] .sort-asc, [data-body-id="historyTracks"] .sort-desc')).toHaveCount(0);
  await expect(page.locator('[data-body-id="historyTracks"] .sort-hint')).toBeHidden();
  expect(await shownColumns(page, "historyTracks"))
    .toEqual(["select", "cover", "waveform", "track", "genre", "year", "key", "length", "format", "action"]);

  // Reset to default brings every column back in its place.
  await page.locator('#panel-usb-history [data-track-columns-button]').click();
  await page.locator("#trackColumnsResetBtn").click();
  await expect.poll(() => shownColumns(page, "historyTracks")).toEqual(all);
  await expect(page.locator("#trackColumnsList .track-columns-item input:not(:checked)")).toHaveCount(1);
  await expect(item("genre").locator("input")).not.toBeChecked();
});

test("the saved column layout is there at the first paint", async ({ page }) => {
  await installColumnsMock(page, { order: ["format", "album", "year", "bpm", "key", "length"], hidden: ["year", "key"] });
  await page.addInitScript(() => {
    // Every header row the app shows, as the first rows appear.
    window.__firstPaint = null;
    new MutationObserver((_, observer) => {
      const row = document.querySelector("#libraryTableBody .track-grid-row[data-track-id]");
      if (!row || window.__firstPaint) return;
      const ids = (r) => [...r.children].filter((c) => !c.hidden).map((c) => [...c.classList].find((n) => n.startsWith("td-")).slice(3));
      window.__firstPaint = ids(row);
      observer.disconnect();
    }).observe(document, { childList: true, subtree: true });
  });
  await page.goto("/");
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(2);
  const expected = ["select", "cover", "waveform", "track", "format", "album", "bpm", "length", "action"];
  expect(await page.evaluate(() => window.__firstPaint)).toEqual(expected);
  expect(await shownColumns(page, "libraryTableBody")).toEqual(expected);
});
