// Regression coverage for: sorting a tracklist by column must not change
// which track Play/scrub-play/Remove act on. Row click handlers must
// resolve track identity by the row's data-id, never by its data-index
// into an array that may no longer match rendered (sorted) order.
import { test, expect } from "./coverage-fixture.mjs";

// Seeds the library and/or one app playlist ("Test Playlist") with `tracks`:
// analyzed, 3 minutes long, at the BPM each one gives (default 120).
function installTracks(page, { library = [], playlist = [] }) {
  return page.addInitScript(({ library, playlist }) => {
    const song = ([id, title, album, bpm = 120]) => ({
      id, title, artist: "Artist", album, filePath: `/music/${id}.mp3`, waveformPeaksPath: "", waveformPreview: [],
      durationMs: 180000, bpm, key: "8A", analysisReady: true
    });
    window.__e2eSourceFolders = ["/music"];
    window.__e2eBackend.seed({
      tracks: library.map(song),
      playlists: playlist.length ? [{ id: "pl-1", name: "Test Playlist", tracks: playlist.map(song) }] : []
    });
    window.__e2eBackend.install();
  }, { library, playlist });
}

// The requests the app sent with `command`.
const requests = (page, command) =>
  page.evaluate((command) => window.__calls.filter((c) => c.command === command).map((c) => c.request), command);

const SONGS = [["t1", "Song One", "Zulu Album"], ["t2", "Song Two", "Alpha Album"], ["t3", "Song Three", "Mike Album"]];

test("Library: scrub-play after column sort plays the clicked row's track, not the pre-sort one", async ({ page }) => {
  await installTracks(page, { library: SONGS });

  await page.goto("/");
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(3);

  // Sort ascending by Album: Alpha Album (Song Two), Mike Album (Song Three), Zulu Album (Song One).
  await page.locator('#panel-library .sortable[data-sort-key="album"]').click();
  await expect(page.locator("#libraryTableBody .track-grid-row .track-title")).toHaveText([
    "Song Two",
    "Song Three",
    "Song One"
  ]);

  // Click the waveform of the row now showing "Song Two" -- it sits at
  // array index 0 in the sorted view but was originally at index 1.
  await page.locator('#libraryTableBody .track-grid-row', { hasText: "Song Two" })
    .locator(".waveform")
    .click();

  await expect.poll(async () => (await requests(page, "play_resolved_track")).length).toBeGreaterThan(0);
  const [call] = await requests(page, "play_resolved_track");
  expect(call.trackId).toBe("t2");
  expect(call.filePath).toBe("/music/t2.mp3");
});

test("Playlist: Play and Remove after column sort act on the clicked row's track", async ({ page }) => {
  await installTracks(page, { playlist: SONGS });

  await page.goto("/");
  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(3);

  // Sort ascending by Album: Alpha Album (Song Two), Mike Album (Song Three), Zulu Album (Song One).
  await page.locator('#panel-playlist .sortable[data-sort-key="album"]').click();
  await expect(page.locator("#playlistTracksBody .track-grid-row .track-title")).toHaveText([
    "Song Two",
    "Song Three",
    "Song One"
  ]);

  // Play the row now showing "Song Two" (array index 0 post-sort, index 1 pre-sort).
  await page.locator('#playlistTracksBody .track-grid-row', { hasText: "Song Two" })
    .locator('[data-action="play-library"]')
    .click();
  await expect.poll(async () => (await requests(page, "play_resolved_track")).length).toBeGreaterThan(0);
  const [playCall] = await requests(page, "play_resolved_track");
  expect(playCall.trackId).toBe("t2");
  expect(playCall.filePath).toBe("/music/t2.mp3");

  // Remove the row now showing "Song Three" (array index 1 post-sort, index 2 pre-sort).
  await page.locator('#playlistTracksBody .track-grid-row', { hasText: "Song Three" })
    .locator('[data-action="remove-playlist-track"]')
    .click();
  await expect.poll(async () => (await requests(page, "remove_tracks_from_playlist")).length).toBeGreaterThan(0);
  const [removeCall] = await requests(page, "remove_tracks_from_playlist");
  expect(removeCall.trackIds).toEqual(["t3"]);
});

test("Playlist: header click shows and clears the sorted-by hint like other tracklists", async ({ page }) => {
  await installTracks(page, { playlist: SONGS.slice(0, 2) });

  await page.goto("/");
  await page.locator("#navPlaylistList .nav-playlist-item").first().click();
  await expect(page.locator("#playlistTracksBody .track-grid-row")).toHaveCount(2);

  const albumHeader = page.locator('#panel-playlist .sortable[data-sort-key="album"]');
  const sortHint = page.locator("#panel-playlist .sort-hint");
  await expect(sortHint).toBeHidden();

  // Ascending click shows the hint, like Library/USB/History tracklists.
  await albumHeader.click();
  await expect(sortHint).toBeVisible();
  await expect(albumHeader).toHaveClass(/sort-asc/);

  // Descending click keeps it visible.
  await albumHeader.click();
  await expect(sortHint).toBeVisible();
  await expect(albumHeader).toHaveClass(/sort-desc/);

  // Third click cycles back to unsorted -- hint hides again.
  await albumHeader.click();
  await expect(sortHint).toBeHidden();
  await expect(albumHeader).not.toHaveClass(/sort-asc|sort-desc/);
});

test("Library: sorting by BPM column actually orders rows by numeric BPM value", async ({ page }) => {
  // BPM values are deliberately non-monotonic with insertion order and
  // include decimals/duplicates, so a broken comparator (e.g. lexicographic
  // string compare, or a comparator applied to the wrong array) reorders
  // rows without the BPM column actually ending up sorted.
  await installTracks(page, {
    library: [["t1", "Track A", "Album", 128], ["t2", "Track B", "Album", 90], ["t3", "Track C", "Album", 174],
      ["t4", "Track D", "Album", 9], ["t5", "Track E", "Album", 128.5]]
  });

  await page.goto("/");
  await expect(page.locator("#libraryTableBody .track-grid-row")).toHaveCount(5);

  const bpmCellTexts = () => page.locator("#libraryTableBody .track-grid-row .td-bpm").allTextContents();

  // Ascending: 9, 90, 128, 128.5, 174.
  await page.locator('#panel-library .sortable[data-sort-key="bpm"]').click();
  await expect(page.locator("#libraryTableBody .track-grid-row .track-title")).toHaveText([
    "Track D",
    "Track B",
    "Track A",
    "Track E",
    "Track C"
  ]);
  expect((await bpmCellTexts()).map(Number)).toEqual([9, 90, 128, 128.5, 174]);

  // Descending: 174, 128.5, 128, 90, 9.
  await page.locator('#panel-library .sortable[data-sort-key="bpm"]').click();
  await expect(page.locator("#libraryTableBody .track-grid-row .track-title")).toHaveText([
    "Track C",
    "Track E",
    "Track A",
    "Track B",
    "Track D"
  ]);
  expect((await bpmCellTexts()).map(Number)).toEqual([174, 128.5, 128, 90, 9]);
});
