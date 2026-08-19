import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { playTrackFromOrigin } from "../components/playback/actions.mjs";

const require = createRequire(import.meta.url);
const { getPlaybackSourceLabel } = require("../playback_source_label.js");
const localTrack = { id: "t-local", title: "Track", filePath: "/music/Track.mp3" };
const usbTrack = { id: "t-usb", title: "Track", filePath: "/usb/Contents/Track.mp3" };

function pathInRoots(filePath, roots) {
  const fp = String(filePath || "").replace(/\\/g, "/").toLowerCase();
  if (!fp) return false;
  return roots.some((root) => {
    const r = String(root || "").replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase();
    return !!r && (fp === r || fp.startsWith(`${r}/`));
  });
}

function playbackState(overrides = {}) {
  return {
    usbRoot: null,
    usbRootValid: false,
    playbackActive: false,
    playbackTrackId: null,
    playbackPath: null,
    playbackRowKey: null,
    activeWaveform: null,
    ...overrides
  };
}

function playbackDeps({ command, setStatus = () => {}, warn = () => {}, generation } = {}) {
  return {
    command,
    trackPathMatchesAnyRoot: pathInRoots,
    clearAllWaveformPlayheads: () => {},
    setWaveformPlayhead: () => {},
    updateTransportButtonsInDom: () => {},
    setStatus,
    warn,
    getPlaybackSourceLabel,
    ...(generation === undefined ? {} : { generation })
  };
}

function playbackResponse(payload) {
  return { path: payload.path, durationMs: 1000, positionMs: 100 };
}

function unexpectedCommand(name) {
  throw new Error(`unexpected command ${name}`);
}

test("playback policy prefers the backend-resolved library path when available", async () => {
  const calls = [];
  const state = playbackState({ usbRoot: "/usb", usbRootValid: true });
  let status = "";

  await playTrackFromOrigin(state, usbTrack, "usb", { rowKey: "r1" }, playbackDeps({
    setStatus: (text) => { status = text; },
    command: async (name, payload) => {
      calls.push({ name, payload });
      if (name === "resolve_playback_source") {
        return { resolvedPath: "/music/Track.mp3", trackId: "t-local", matchedBy: "hash" };
      }
      if (name === "play_track_native") return playbackResponse(payload);
      return unexpectedCommand(name);
    }
  }));

  assert.equal(calls.find((c) => c.name === "resolve_playback_source").payload.trackId, "t-usb");
  assert.equal(calls.find((c) => c.name === "play_track_native").payload.path, "/music/Track.mp3");
  assert.equal(state.playbackTrackId, "t-local");
  assert.match(status, /Playing from Library \(matched\)/);
});

test("playback policy falls back to USB path when library playback fails", async () => {
  const playedPaths = [];
  const state = playbackState({ usbRoot: "/usb", usbRootValid: true });
  let status = "";

  await playTrackFromOrigin(state, usbTrack, "usb", { rowKey: "r1" }, playbackDeps({
    setStatus: (text) => { status = text; },
    command: async (name, payload) => {
      if (name === "resolve_playback_source") {
        return { resolvedPath: "/music/Track.mp3", trackId: "t-local", matchedBy: "hash" };
      }
      if (name === "play_track_native") {
        playedPaths.push(payload.path);
        if (payload.path === "/music/Track.mp3") throw new Error("library missing");
        return playbackResponse(payload);
      }
      return unexpectedCommand(name);
    }
  }));

  assert.deepEqual(playedPaths, ["/music/Track.mp3", "/usb/Contents/Track.mp3"]);
  assert.equal(state.playbackTrackId, "t-usb");
  assert.match(status, /Playing from USB \(library unavailable\)/);
});

test("playback retries once after recoverable native busy error", async () => {
  const calls = [];
  const state = playbackState();

  await playTrackFromOrigin(state, localTrack, "local", { rowKey: "r1" }, playbackDeps({
    command: async (name, payload) => {
      calls.push({ name, payload });
      if (name === "resolve_playback_source") {
        return { resolvedPath: "/music/Track.mp3", trackId: "t-local", matchedBy: "self" };
      }
      if (name === "play_track_native") {
        if (calls.filter((c) => c.name === "play_track_native").length === 1) {
          throw new Error("Output device is busy");
        }
        return playbackResponse(payload);
      }
      if (name === "stop_playback_native") return { stopped: true };
      return unexpectedCommand(name);
    }
  }));

  assert.deepEqual(calls.map((c) => c.name), [
    "resolve_playback_source",
    "play_track_native",
    "stop_playback_native",
    "play_track_native"
  ]);
  assert.equal(state.playbackActive, true);
  assert.equal(state.playbackPath, "/music/Track.mp3");
});

test("playback uses USB path when the resolver finds no genuine local match", async () => {
  const calls = [];
  const state = playbackState({ usbRoot: "/usb", usbRootValid: true });
  let status = "";

  await playTrackFromOrigin(state, usbTrack, "usb", { rowKey: "r1" }, playbackDeps({
    setStatus: (text) => { status = text; },
    command: async (name, payload) => {
      calls.push({ name, payload });
      if (name === "resolve_playback_source") {
        return { resolvedPath: null, trackId: null, matchedBy: "none" };
      }
      if (name === "play_track_native") return playbackResponse(payload);
      return unexpectedCommand(name);
    }
  }));

  assert.deepEqual(calls.map((c) => c.name), ["resolve_playback_source", "play_track_native"]);
  assert.equal(calls[1].payload.path, "/usb/Contents/Track.mp3");
  assert.equal(state.playbackTrackId, "t-usb");
  assert.equal(state.playbackPath, "/usb/Contents/Track.mp3");
  assert.match(status, /Playing from USB/);
});

test("playback policy reports unavailable when neither library nor usb path is playable", async () => {
  const state = playbackState();
  let status = "";
  let statusMeta = null;

  await playTrackFromOrigin(state, {
    id: "t1",
    title: "Track",
    filePath: "/unknown/Track.mp3"
  }, "usb", {}, playbackDeps({
    setStatus: (text, meta) => { status = text; statusMeta = meta; },
    command: async (name) => {
      if (name === "resolve_playback_source") {
        return { resolvedPath: null, trackId: null, matchedBy: "none" };
      }
      throw new Error("play_track_native should not be called");
    }
  }));

  assert.equal(state.playbackActive, false);
  assert.equal(status, "Cannot play: track not found in Library or selected USB.");
  assert.equal(statusMeta?.level, "warn");
  assert.equal(statusMeta?.source, "playback");
});

test("playback failure is reported with level:error so it persists to the Event Log", async () => {
  const state = playbackState();
  let status = "";
  let statusMeta = null;

  await playTrackFromOrigin(state, localTrack, "local", { rowKey: "r1" }, playbackDeps({
    setStatus: (text, meta) => { status = text; statusMeta = meta; },
    command: async (name) => {
      if (name === "resolve_playback_source") {
        return { resolvedPath: "/music/Track.mp3", trackId: "t-local", matchedBy: "self" };
      }
      if (name === "play_track_native") throw new Error("decoder error: unrecognized format");
      return unexpectedCommand(name);
    }
  }));

  assert.match(status, /Playback failed \(Library\)/);
  assert.equal(statusMeta?.level, "error");
  assert.equal(statusMeta?.source, "playback");
});

test("playTrackFromOrigin resolves local, playlist, usb, and history origins through the backend", async (t) => {
  for (const origin of ["local", "playlist", "usb", "history"]) {
    await t.test(`origin=${origin}`, async () => {
      const calls = [];
      const state = playbackState();

      await playTrackFromOrigin(state, localTrack, origin, { rowKey: "r1" }, playbackDeps({
        command: async (name, payload) => {
          calls.push({ name, payload });
          if (name === "resolve_playback_source") {
            return { resolvedPath: "/music/Track.mp3", trackId: "t-local", matchedBy: "self" };
          }
          if (name === "play_track_native") return { path: payload.path, durationMs: 1000, positionMs: 0 };
          return unexpectedCommand(name);
        }
      }));

      assert.deepEqual(calls.map((c) => c.name), ["resolve_playback_source", "play_track_native"]);
      assert.equal(calls[0].payload.trackId, "t-local");
      assert.equal(state.playbackTrackId, "t-local");
      assert.equal(state.playbackPath, "/music/Track.mp3");
    });
  }
});

test("playTrackFromOrigin self-heals a playlist row whose track_id is a stale USB placeholder", async () => {
  const state = playbackState();
  let status = "";

  await playTrackFromOrigin(state, {
    id: "placeholder-1",
    title: "Track",
    filePath: "/mnt/usb1/Contents/Track.mp3"
  }, "playlist", { rowKey: "r1" }, playbackDeps({
    setStatus: (text) => { status = text; },
    command: async (name, payload) => {
      if (name === "resolve_playback_source") {
        assert.equal(payload.trackId, "placeholder-1");
        return { resolvedPath: "/music/Track.mp3", trackId: "local-1", matchedBy: "hash" };
      }
      if (name === "play_track_native") return { path: payload.path, durationMs: 1000, positionMs: 0 };
      return unexpectedCommand(name);
    }
  }));

  assert.equal(state.playbackPath, "/music/Track.mp3", "playback should use the resolved path, not track.filePath");
  assert.equal(state.playbackTrackId, "local-1");
  assert.match(status, /Playing from Library/);
});

test("a stale generation skips the native call and never commits state", async () => {
  const state = playbackState({ playbackGeneration: 2 });
  let playCalled = false;

  await playTrackFromOrigin(state, localTrack, "local", { rowKey: "r1" }, playbackDeps({
    generation: 1,
    command: async (name) => {
      if (name === "resolve_playback_source") {
        return { resolvedPath: "/music/Track.mp3", trackId: "t-local", matchedBy: "self" };
      }
      if (name === "play_track_native") {
        playCalled = true;
        return { path: "/music/Track.mp3", durationMs: 1000, positionMs: 0 };
      }
      return unexpectedCommand(name);
    }
  }));

  assert.equal(playCalled, false);
  assert.equal(state.playbackActive, false);
  assert.equal(state.playbackTrackId, null);
});
