import test from "node:test";
import assert from "node:assert/strict";

import { formatParityIssues } from "../components/usb/actions.mjs";

test("formatParityIssues returns empty array at full parity", () => {
  assert.deepEqual(
    formatParityIssues({
      pdbTracks: 10,
      edbTracks: 10,
      onlyInPdb: 0,
      onlyInEdb: 0,
      orderMismatch: false,
      playlistIdMatch: true,
      sortOrderMatch: true
    }),
    []
  );
});

test("formatParityIssues shows +PDB only when both DBs have tracks", () => {
  assert.deepEqual(
    formatParityIssues({
      pdbTracks: 10,
      edbTracks: 6,
      onlyInPdb: 4,
      onlyInEdb: 0,
      playlistIdMatch: true,
      sortOrderMatch: true
    }),
    ["+PDB 4"]
  );
});

test("formatParityIssues suppresses +PDB when eDB is 0", () => {
  assert.deepEqual(
    formatParityIssues({
      pdbTracks: 50,
      edbTracks: 0,
      onlyInPdb: 50,
      onlyInEdb: 0,
      playlistIdMatch: false,
      sortOrderMatch: false
    }),
    ["id mismatch", "sort mismatch"]
  );
});

test("formatParityIssues shows structural issues", () => {
  assert.deepEqual(
    formatParityIssues({
      pdbTracks: 10,
      edbTracks: 10,
      onlyInPdb: 0,
      onlyInEdb: 0,
      orderMismatch: true,
      playlistIdMatch: true,
      sortOrderMatch: true
    }),
    ["order mismatch"]
  );
});

test("formatParityIssues shows multiple issues", () => {
  const issues = formatParityIssues({
    pdbTracks: 10,
    edbTracks: 8,
    onlyInPdb: 2,
    onlyInEdb: 0,
    playlistIdMatch: false,
    sortOrderMatch: false,
    pdbMissingCoreMetadata: 1,
    dictionaryIdIssueTracks: 2
  });
  assert.deepEqual(issues, [
    "+PDB 2",
    "id mismatch",
    "sort mismatch",
    "PDB gaps 1",
    "dict issues 2"
  ]);
});
