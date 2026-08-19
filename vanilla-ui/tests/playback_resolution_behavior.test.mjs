import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { playTrackFromOrigin } from "../components/playback/actions.mjs";

const require = createRequire(import.meta.url);
const { getPlaybackSourceLabel } = require("../playback_source_label.js");

const localTrack = {
  id: "t-local",
  title: "Track",
  artist: "Artist",
  album: "Album",
  bpm: 124,
  fileSizeBytes: 1000,
  filePath: "/music/Track.mp3",
};
const usbTrack = {
  id: "t-usb",
  title: "Track",
  artist: "Artist",
  filePath: "/usb/Contents/Track.mp3",
};

function playbackState(overrides = {}) {
  return {
    usbRoot: null,
    usbRootValid: false,
    playbackActive: false,
    playbackTrackId: null,
    playbackPath: null,
    playbackRowKey: null,
    activeWaveform: null,
    ...overrides,
  };
}

function playbackDeps({ command, setStatus = () => {}, generation } = {}) {
  return {
    command,
    clearAllWaveformPlayheads: () => {},
    setWaveformPlayhead: () => {},
    updateTransportButtonsInDom: () => {},
    setStatus,
    warn: () => {},
    getPlaybackSourceLabel,
    ...(generation === undefined ? {} : { generation }),
  };
}

function backendPlayback(overrides = {}) {
  return {
    path: "/music/Track.mp3",
    playing: true,
    durationMs: 1000,
    positionMs: 100,
    trackId: "t-local",
    matchedBy: "hash",
    source: "library",
    sourceLabel: "Library",
    libraryResolved: true,
    hasUsbContext: false,
    ...overrides,
  };
}

test("playTrackFromOrigin delegates playback resolution to one backend command", async () => {
  const calls = [];
  const state = playbackState({ usbRoot: "/usb", usbRootValid: true });
  let status = "";

  await playTrackFromOrigin(state, usbTrack, "usb", { rowKey: "r1", startRatio: 0.25 }, playbackDeps({
    setStatus: (text) => { status = text; },
    command: async (name, payload) => {
      calls.push({ name, payload });
      assert.equal(name, "play_resolved_track");
      return backendPlayback({
        sourceLabel: "Library (matched)",
        hasUsbContext: true,
      });
    },
  }));

  assert.equal(calls.length, 1);
  assert.deepEqual(calls[0].payload, {
    title: "Track",
    artist: "Artist",
    album: null,
    bpm: null,
    filePath: "/usb/Contents/Track.mp3",
    fileSizeBytes: null,
    trackId: "t-usb",
    origin: "usb",
    usbRoot: "/usb",
    usbRootValid: true,
    startOffsetMs: null,
    startRatio: 0.25,
  });
  assert.equal(state.playbackActive, true);
  assert.equal(state.playbackTrackId, "t-local");
  assert.equal(state.playbackPath, "/music/Track.mp3");
  assert.equal(state.playbackRowKey, "r1");
  assert.deepEqual(state.playbackLabelContext, {
    origin: "usb",
    libraryResolved: true,
    hasUsbContext: true,
    title: "Artist - Track",
  });
  assert.match(status, /Playing from Library \(matched\): Artist - Track/);
});

test("playTrackFromOrigin commits a backend USB fallback result without local fallback logic", async () => {
  const state = playbackState({ usbRoot: "/usb", usbRootValid: true });
  let status = "";

  await playTrackFromOrigin(state, usbTrack, "usb", { rowKey: "r1" }, playbackDeps({
    setStatus: (text) => { status = text; },
    command: async (name) => {
      assert.equal(name, "play_resolved_track");
      return backendPlayback({
        path: "/usb/Contents/Track.mp3",
        trackId: "t-usb",
        matchedBy: "none",
        source: "usb",
        sourceLabel: "USB (library unavailable)",
        libraryResolved: false,
        hasUsbContext: true,
      });
    },
  }));

  assert.equal(state.playbackTrackId, "t-usb");
  assert.equal(state.playbackPath, "/usb/Contents/Track.mp3");
  assert.equal(state.playbackLabelContext.libraryResolved, false);
  assert.match(status, /Playing from USB \(library unavailable\): Artist - Track/);
});

test("playTrackFromOrigin reports backend not-found as a warning", async () => {
  const state = playbackState();
  let status = "";
  let statusMeta = null;

  await playTrackFromOrigin(state, { id: "missing", title: "Track" }, "usb", {}, playbackDeps({
    setStatus: (text, meta) => { status = text; statusMeta = meta; },
    command: async (name) => {
      assert.equal(name, "play_resolved_track");
      throw new Error("track not found in Library or selected USB");
    },
  }));

  assert.equal(state.playbackActive, false);
  assert.equal(status, "Cannot play: track not found in Library or selected USB.");
  assert.equal(statusMeta?.level, "warn");
  assert.equal(statusMeta?.source, "playback");
});

test("playTrackFromOrigin reports backend playback failures as event-log errors", async () => {
  const state = playbackState();
  let status = "";
  let statusMeta = null;

  await playTrackFromOrigin(state, localTrack, "local", { rowKey: "r1" }, playbackDeps({
    setStatus: (text, meta) => { status = text; statusMeta = meta; },
    command: async (name) => {
      assert.equal(name, "play_resolved_track");
      throw new Error("decoder error: unrecognized format");
    },
  }));

  assert.match(status, /Playback failed \(Library\): decoder error/);
  assert.equal(statusMeta?.level, "error");
  assert.equal(statusMeta?.source, "playback");
});

test("all playback origins route through play_resolved_track", async (t) => {
  for (const origin of ["local", "playlist", "usb", "history"]) {
    await t.test(`origin=${origin}`, async () => {
      const calls = [];
      const state = playbackState();

      await playTrackFromOrigin(state, localTrack, origin, { rowKey: "r1" }, playbackDeps({
        command: async (name, payload) => {
          calls.push({ name, payload });
          return backendPlayback();
        },
      }));

      assert.deepEqual(calls.map((c) => c.name), ["play_resolved_track"]);
      assert.equal(calls[0].payload.trackId, "t-local");
      assert.equal(calls[0].payload.origin, origin);
      assert.equal(state.playbackTrackId, "t-local");
      assert.equal(state.playbackPath, "/music/Track.mp3");
    });
  }
});

test("a stale generation skips the backend play command and never commits state", async () => {
  const state = playbackState({ playbackGeneration: 2 });
  let playCalled = false;

  await playTrackFromOrigin(state, localTrack, "local", { rowKey: "r1" }, playbackDeps({
    generation: 1,
    command: async () => {
      playCalled = true;
      return backendPlayback();
    },
  }));

  assert.equal(playCalled, false);
  assert.equal(state.playbackActive, false);
  assert.equal(state.playbackTrackId, null);
});
