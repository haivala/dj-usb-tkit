import { test, expect } from "./coverage-fixture.mjs";

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

