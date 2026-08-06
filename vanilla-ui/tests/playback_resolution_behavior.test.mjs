import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { playTrackFromOrigin } from "../components/playback/actions.mjs";

const require = createRequire(import.meta.url);
const { getPlaybackSourceLabel } = require("../playback_source_label.js");

function pathInRoots(filePath, roots) {
  const fp = String(filePath || "").replace(/\\/g, "/").toLowerCase();
  if (!fp) return false;
  return roots.some((root) => {
    const r = String(root || "").replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase();
    return !!r && (fp === r || fp.startsWith(`${r}/`));
  });
}

test("playback policy prefers the backend-resolved library path when available", async () => {
  const calls = [];
  const state = {
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
        return { resolvedPath: "/music/Track.mp3", trackId: "t-local", matchedBy: "hash" };
      }
      if (name === "play_track_native") {
        return { path: payload.path, durationMs: 1000, positionMs: 100 };
      }
      throw new Error(`unexpected command ${name}`);
    },
    trackPathMatchesAnyRoot: pathInRoots,
    clearAllWaveformPlayheads: () => {},
    setWaveformPlayhead: () => {},
    updateTransportButtonsInDom: () => {},
    setStatus: (text) => { status = text; },
    warn: () => {}
  });

  const resolveCall = calls.find((c) => c.name === "resolve_playback_source");
  assert.equal(resolveCall.payload.trackId, "t-usb", "should pass the origin row's own id through for the fast path");
  const play = calls.find((c) => c.name === "play_track_native");
  assert.equal(play.payload.path, "/music/Track.mp3");
  assert.equal(state.playbackTrackId, "t-local");
  assert.match(status, /Playing from Library \(matched\)/);
});

test("playback policy falls back to USB path when library playback fails", async () => {
  const playedPaths = [];
  const state = {
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
        return { resolvedPath: "/music/Track.mp3", trackId: "t-local", matchedBy: "hash" };
      }
      if (name === "play_track_native") {
        playedPaths.push(payload.path);
        if (payload.path === "/music/Track.mp3") throw new Error("library missing");
        return { path: payload.path, durationMs: 1000, positionMs: 200 };
      }
      throw new Error(`unexpected command ${name}`);
    },
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
  }, "local", { rowKey: "r1" }, {
    command: async (name, payload) => {
      calls.push({ name, payload });
      if (name === "resolve_playback_source") {
        return { resolvedPath: "/music/Track.mp3", trackId: "t-local", matchedBy: "self" };
      }
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
    trackPathMatchesAnyRoot: pathInRoots,
    clearAllWaveformPlayheads: () => {},
    setWaveformPlayhead: () => {},
    updateTransportButtonsInDom: () => {},
    setStatus: () => {},
    warn: () => {}
  });

  const names = calls.map((c) => c.name);
  assert.deepEqual(names, ["resolve_playback_source", "play_track_native", "stop_playback_native", "play_track_native"]);
  assert.equal(state.playbackActive, true);
  assert.equal(state.playbackPath, "/music/Track.mp3");
});

test("playback status falls back to USB label when the resolver finds no genuine local match", async () => {
  const calls = [];
  const state = {
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
        return { resolvedPath: null, trackId: null, matchedBy: "none" };
      }
      if (name === "play_track_native") {
        return { path: payload.path, durationMs: 1000, positionMs: 100 };
      }
      throw new Error(`unexpected command ${name}`);
    },
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

test("playback does not search frontend tracks when backend resolver has no local match", async () => {
  const calls = [];
  const state = {
    usbRoot: "/usb",
    usbRootValid: true,
    playbackActive: false,
    playbackTrackId: null,
    playbackPath: null,
    playbackRowKey: null,
    activeWaveform: null
  };

  await playTrackFromOrigin(state, {
    id: "t-usb",
    title: "Track",
    filePath: "/usb/Contents/Track.mp3"
  }, "usb", { rowKey: "r1" }, {
    command: async (name, payload) => {
      calls.push({ name, payload });
      if (name === "resolve_playback_source") {
        return { resolvedPath: null, trackId: null, matchedBy: "none" };
      }
      if (name === "play_track_native") {
        return { path: payload.path, durationMs: 1000, positionMs: 100 };
      }
      throw new Error(`unexpected command ${name}`);
    },
    trackPathMatchesAnyRoot: pathInRoots,
    clearAllWaveformPlayheads: () => {},
    setWaveformPlayhead: () => {},
    updateTransportButtonsInDom: () => {},
    setStatus: () => {},
    warn: () => {}
  });

  assert.deepEqual(calls.map((c) => c.name), ["resolve_playback_source", "play_track_native"]);
  assert.equal(calls[1].payload.path, "/usb/Contents/Track.mp3");
  assert.equal(state.playbackTrackId, "t-usb");
  assert.equal(state.playbackPath, "/usb/Contents/Track.mp3");
});

test("playback policy reports unavailable when neither library nor usb path is playable", async () => {
  const state = {
    usbRoot: null,
    usbRootValid: false,
    playbackActive: false,
    playbackTrackId: null,
    playbackPath: null,
    playbackRowKey: null,
    activeWaveform: null
  };
  let status = "";
  let statusMeta = null;

  await playTrackFromOrigin(state, {
    id: "t1",
    title: "Track",
    filePath: "/unknown/Track.mp3"
  }, "usb", {}, {
    command: async (name) => {
      if (name === "resolve_playback_source") return { resolvedPath: null, trackId: null, matchedBy: "none" };
      throw new Error("play_track_native should not be called");
    },
    trackPathMatchesAnyRoot: pathInRoots,
    clearAllWaveformPlayheads: () => {},
    setWaveformPlayhead: () => {},
    updateTransportButtonsInDom: () => {},
    setStatus: (text, meta) => { status = text; statusMeta = meta; },
    warn: () => {}
  });

  assert.equal(state.playbackActive, false);
  assert.equal(status, "Cannot play: track not found in Library or selected USB.");
  assert.equal(statusMeta?.level, "warn");
  assert.equal(statusMeta?.source, "playback");
});

test("playback failure is reported with level:error so it persists to the Event Log", async () => {
  const state = {
    usbRoot: null,
    usbRootValid: false,
    playbackActive: false,
    playbackTrackId: null,
    playbackPath: null,
    playbackRowKey: null,
    activeWaveform: null
  };
  let status = "";
  let statusMeta = null;

  await playTrackFromOrigin(state, {
    id: "t-local",
    title: "Track",
    filePath: "/music/Track.mp3"
  }, "local", { rowKey: "r1" }, {
    command: async (name) => {
      if (name === "resolve_playback_source") {
        return { resolvedPath: "/music/Track.mp3", trackId: "t-local", matchedBy: "self" };
      }
      if (name === "play_track_native") throw new Error("decoder error: unrecognized format");
      throw new Error(`unexpected command ${name}`);
    },
    trackPathMatchesAnyRoot: pathInRoots,
    clearAllWaveformPlayheads: () => {},
    setWaveformPlayhead: () => {},
    updateTransportButtonsInDom: () => {},
    setStatus: (text, meta) => { status = text; statusMeta = meta; },
    warn: () => {}
  });

  assert.match(status, /Playback failed \(Library\)/);
  assert.equal(statusMeta?.level, "error");
  assert.equal(statusMeta?.source, "playback");
});

test("playTrackFromOrigin calls resolve_playback_source for local/playlist origin too, passing trackId", async (t) => {
  for (const origin of ["local", "playlist", "usb", "history"]) {
    await t.test(`origin=${origin}`, async () => {
      const calls = [];
      const state = {
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
      }, origin, { rowKey: "r1" }, {
        command: async (name, payload) => {
          calls.push({ name, payload });
          if (name === "resolve_playback_source") {
            return { resolvedPath: "/music/Track.mp3", trackId: "t-local", matchedBy: "self" };
          }
          if (name === "play_track_native") {
            return { path: payload.path, durationMs: 1000, positionMs: 0 };
          }
          throw new Error(`unexpected command ${name}`);
        },
        trackPathMatchesAnyRoot: pathInRoots,
        clearAllWaveformPlayheads: () => {},
        setWaveformPlayhead: () => {},
        updateTransportButtonsInDom: () => {},
        setStatus: () => {},
        warn: () => {}
      });

      assert.deepEqual(calls.map((c) => c.name), ["resolve_playback_source", "play_track_native"]);
      assert.equal(calls[0].payload.trackId, "t-local");
      assert.equal(state.playbackTrackId, "t-local");
      assert.equal(state.playbackPath, "/music/Track.mp3");
    });
  }
});

test("playTrackFromOrigin self-heals a playlist row whose track_id is a stale USB placeholder", async () => {
  const state = {
    usbRoot: null,
    usbRootValid: false,
    playbackActive: false,
    playbackTrackId: null,
    playbackPath: null,
    playbackRowKey: null,
    activeWaveform: null
  };
  let status = "";

  // The playlist row itself still points at a stale placeholder id/path,
  // but resolve_playback_source (Step 6c's fast path falling through to
  // fingerprint matching) reports back the genuine local track instead.
  await playTrackFromOrigin(state, {
    id: "placeholder-1",
    title: "Track",
    filePath: "/mnt/usb1/Contents/Track.mp3"
  }, "playlist", { rowKey: "r1" }, {
    command: async (name, payload) => {
      if (name === "resolve_playback_source") {
        assert.equal(payload.trackId, "placeholder-1");
        return { resolvedPath: "/music/Track.mp3", trackId: "local-1", matchedBy: "hash" };
      }
      if (name === "play_track_native") {
        return { path: payload.path, durationMs: 1000, positionMs: 0 };
      }
      throw new Error(`unexpected command ${name}`);
    },
    trackPathMatchesAnyRoot: pathInRoots,
    clearAllWaveformPlayheads: () => {},
    setWaveformPlayhead: () => {},
    updateTransportButtonsInDom: () => {},
    setStatus: (text) => { status = text; },
    warn: () => {}
  });

  assert.equal(state.playbackPath, "/music/Track.mp3", "playback should use the resolved path, not track.filePath");
  assert.equal(state.playbackTrackId, "local-1");
  // origin "playlist" isn't an external (usb/history) origin, so it reads
  // "Library" rather than "Library (matched)" -- the self-heal is proven by
  // the resolved path/id assertions above, not the label text.
  assert.match(status, /Playing from Library/);
});

test("a stale generation skips the native call and never commits state", async () => {
  const state = {
    usbRoot: null,
    usbRootValid: false,
    playbackActive: false,
    playbackTrackId: null,
    playbackPath: null,
    playbackRowKey: null,
    activeWaveform: null,
    playbackGeneration: 2
  };
  let playCalled = false;

  await playTrackFromOrigin(state, {
    id: "t-local",
    title: "Track",
    filePath: "/music/Track.mp3"
  }, "local", { rowKey: "r1" }, {
    command: async (name) => {
      if (name === "resolve_playback_source") {
        return { resolvedPath: "/music/Track.mp3", trackId: "t-local", matchedBy: "self" };
      }
      if (name === "play_track_native") {
        playCalled = true;
        return { path: "/music/Track.mp3", durationMs: 1000, positionMs: 0 };
      }
      throw new Error(`unexpected command ${name}`);
    },
    trackPathMatchesAnyRoot: pathInRoots,
    clearAllWaveformPlayheads: () => {},
    setWaveformPlayhead: () => {},
    updateTransportButtonsInDom: () => {},
    setStatus: () => {},
    warn: () => {},
    generation: 1
  });

  assert.equal(playCalled, false);
  assert.equal(state.playbackActive, false);
  assert.equal(state.playbackTrackId, null);
});
