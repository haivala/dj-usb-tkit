import test from "node:test";
import assert from "node:assert/strict";
import { playTrackFromOrigin } from "../components/playback/actions.mjs";

function pathInRoots(filePath, roots) {
  const fp = String(filePath || "").replace(/\\/g, "/").toLowerCase();
  if (!fp) return false;
  return roots.some((root) => {
    const r = String(root || "").replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase();
    return !!r && (fp === r || fp.startsWith(`${r}/`));
  });
}

test("playback policy prefers library path when available", async () => {
  const calls = [];
  const state = {
    sourceRoots: ["/music"],
    usbRoot: "/usb",
    usbRootValid: true,
    playbackActive: false,
    playbackTrackId: null,
    playbackPath: null,
    playbackRowKey: null,
    activeWaveform: null
  };
  let status = "";

  await playTrackFromOrigin(state, {
    id: "t-usb",
    title: "Track",
    filePath: "/usb/Contents/Track.mp3"
  }, "usb", { rowKey: "r1" }, {
    command: async (name, payload) => {
      calls.push({ name, payload });
      if (name === "resolve_playback_source") {
        return { resolvedPath: "/music/Track.mp3", trackId: "t-local" };
      }
      if (name === "play_track_native") {
        return { path: payload.path, durationMs: 1000, positionMs: 100 };
      }
      throw new Error(`unexpected command ${name}`);
    },
    resolveLocalTrackForPlayback: async () => null,
    trackPathMatchesAnyRoot: pathInRoots,
    clearAllWaveformPlayheads: () => {},
    setWaveformPlayhead: () => {},
    updateTransportButtonsInDom: () => {},
    setStatus: (text) => { status = text; },
    warn: () => {}
  });

  const play = calls.find((c) => c.name === "play_track_native");
  assert.equal(play.payload.path, "/music/Track.mp3");
  assert.equal(state.playbackTrackId, "t-local");
  assert.match(status, /Playing from Library/);
});

test("playback policy falls back to USB path when library playback fails", async () => {
  const playedPaths = [];
  const state = {
    sourceRoots: ["/music"],
    usbRoot: "/usb",
    usbRootValid: true,
    playbackActive: false,
    playbackTrackId: null,
    playbackPath: null,
    playbackRowKey: null,
    activeWaveform: null
  };
  let status = "";

  await playTrackFromOrigin(state, {
    id: "t-usb",
    title: "Track",
    filePath: "/usb/Contents/Track.mp3"
  }, "usb", { rowKey: "r1" }, {
    command: async (name, payload) => {
      if (name === "resolve_playback_source") {
        return { resolvedPath: "/music/Track.mp3", trackId: "t-local" };
      }
      if (name === "play_track_native") {
        playedPaths.push(payload.path);
        if (payload.path === "/music/Track.mp3") throw new Error("library missing");
        return { path: payload.path, durationMs: 1000, positionMs: 200 };
      }
      throw new Error(`unexpected command ${name}`);
    },
    resolveLocalTrackForPlayback: async () => null,
    trackPathMatchesAnyRoot: pathInRoots,
    clearAllWaveformPlayheads: () => {},
    setWaveformPlayhead: () => {},
    updateTransportButtonsInDom: () => {},
    setStatus: (text) => { status = text; },
    warn: () => {}
  });

  assert.deepEqual(playedPaths, ["/music/Track.mp3", "/usb/Contents/Track.mp3"]);
  assert.equal(state.playbackTrackId, "t-usb");
  assert.match(status, /Playing from USB \(library unavailable\)/);
});

test("playback retries once after recoverable native busy error", async () => {
  const calls = [];
  const state = {
    sourceRoots: ["/music"],
    usbRoot: null,
    usbRootValid: false,
    playbackActive: false,
    playbackTrackId: null,
    playbackPath: null,
    playbackRowKey: null,
    activeWaveform: null
  };

  await playTrackFromOrigin(state, {
    id: "t-local",
    title: "Track",
    filePath: "/music/Track.mp3"
  }, "library", { rowKey: "r1" }, {
    command: async (name, payload) => {
      calls.push({ name, payload });
      if (name === "play_track_native") {
        const playAttempts = calls.filter((c) => c.name === "play_track_native").length;
        if (playAttempts === 1) throw new Error("Output device is busy");
        return { path: payload.path, durationMs: 1000, positionMs: 100 };
      }
      if (name === "stop_playback_native") {
        return { stopped: true };
      }
      throw new Error(`unexpected command ${name}`);
    },
    resolveLocalTrackForPlayback: async () => null,
    trackPathMatchesAnyRoot: pathInRoots,
    clearAllWaveformPlayheads: () => {},
    setWaveformPlayhead: () => {},
    updateTransportButtonsInDom: () => {},
    setStatus: () => {},
    warn: () => {}
  });

  const names = calls.map((c) => c.name);
  assert.deepEqual(names, ["play_track_native", "stop_playback_native", "play_track_native"]);
  assert.equal(state.playbackActive, true);
  assert.equal(state.playbackPath, "/music/Track.mp3");
});

test("playback status reports USB when resolver returns non-library path with track id", async () => {
  const calls = [];
  const state = {
    sourceRoots: ["/music"],
    usbRoot: "/usb",
    usbRootValid: true,
    playbackActive: false,
    playbackTrackId: null,
    playbackPath: null,
    playbackRowKey: null,
    activeWaveform: null
  };
  let status = "";

  await playTrackFromOrigin(state, {
    id: "t-usb",
    title: "Track",
    filePath: "/usb/Contents/Track.mp3"
  }, "usb", { rowKey: "r1" }, {
    command: async (name, payload) => {
      calls.push({ name, payload });
      if (name === "resolve_playback_source") {
        return { resolvedPath: "/archive/not-in-library/Track.mp3", trackId: "t-local" };
      }
      if (name === "play_track_native") {
        return { path: payload.path, durationMs: 1000, positionMs: 100 };
      }
      throw new Error(`unexpected command ${name}`);
    },
    resolveLocalTrackForPlayback: async () => null,
    trackPathMatchesAnyRoot: pathInRoots,
    clearAllWaveformPlayheads: () => {},
    setWaveformPlayhead: () => {},
    updateTransportButtonsInDom: () => {},
    setStatus: (text) => { status = text; },
    warn: () => {}
  });

  const play = calls.find((c) => c.name === "play_track_native");
  assert.equal(play.payload.path, "/usb/Contents/Track.mp3");
  assert.equal(state.playbackTrackId, "t-usb");
  assert.match(status, /Playing from USB/);
});

test("playback policy reports unavailable when neither library nor usb path is playable", async () => {
  const state = {
    sourceRoots: ["/music"],
    usbRoot: null,
    usbRootValid: false,
    playbackActive: false,
    playbackTrackId: null,
    playbackPath: null,
    playbackRowKey: null,
    activeWaveform: null
  };
  let status = "";

  await playTrackFromOrigin(state, {
    id: "t1",
    title: "Track",
    filePath: "/unknown/Track.mp3"
  }, "usb", {}, {
    command: async (name) => {
      if (name === "resolve_playback_source") return { resolvedPath: null, trackId: null };
      throw new Error("play_track_native should not be called");
    },
    resolveLocalTrackForPlayback: async () => null,
    trackPathMatchesAnyRoot: pathInRoots,
    clearAllWaveformPlayheads: () => {},
    setWaveformPlayhead: () => {},
    updateTransportButtonsInDom: () => {},
    setStatus: (text) => { status = text; },
    warn: () => {}
  });

  assert.equal(state.playbackActive, false);
  assert.equal(status, "Cannot play: track not found in Library or selected USB.");
});
