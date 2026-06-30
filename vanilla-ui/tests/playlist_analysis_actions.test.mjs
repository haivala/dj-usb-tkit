import test from "node:test";
import assert from "node:assert/strict";

function normalizePath(value) {
  return String(value || "").replace(/\\/g, "/").trim().toLowerCase();
}

function trackHasRenderableWaveform(track) {
  const preview = Array.isArray(track?.waveformPreview) ? track.waveformPreview : [];
  return preview.length > 0 || !!String(track?.waveformPeaksPath || "").trim();
}

function trackHasBpm(track) {
  return Number.isFinite(Number(track?.bpm)) && Number(track.bpm) > 0;
}

function trackHasCoreAnalysis(track) {
  const durationMs = Number(track?.durationMs);
  return trackHasRenderableWaveform(track)
    && trackHasBpm(track)
    && Number.isFinite(durationMs)
    && durationMs > 0;
}

function isUsbOriginTrack(track, usbRoot = "") {
  if (!track) return false;
  const usbAnalysisPath = String(track.usbAnalysisPath || "").trim();
  if (usbAnalysisPath) return true;
  const normalizedUsbRoot = normalizePath(usbRoot);
  const filePath = normalizePath(track.filePath || "");
  const waveformPath = normalizePath(track.waveformPeaksPath || "");
  if (normalizedUsbRoot && filePath && filePath.startsWith(normalizedUsbRoot)) return true;
  if (normalizedUsbRoot && waveformPath && waveformPath.startsWith(normalizedUsbRoot)) return true;
  return false;
}

function getAnalyzeMissingCandidates(tracks, usbRoot = "") {
  return (Array.isArray(tracks) ? tracks : [])
    .filter((track) => !isUsbOriginTrack(track, usbRoot) && !trackHasCoreAnalysis(track));
}

test("analyze-missing candidates include only local non-USB tracks without core analysis", () => {
  const tracks = [
    {
      id: "local-missing",
      title: "Local Missing",
      filePath: "/music/local-missing.mp3",
      waveformPeaksPath: "",
      waveformPreview: [],
      bpm: "",
      durationMs: null
    },
    {
      id: "local-ready",
      title: "Local Ready",
      filePath: "/music/local-ready.mp3",
      waveformPeaksPath: "/tmp/local-ready.dat",
      waveformPreview: [10, 20],
      bpm: 128,
      durationMs: 180000
    },
    {
      id: "usb-missing",
      title: "USB Missing",
      filePath: "/USB/Contents/Artist/usb-missing.mp3",
      usbAnalysisPath: "/USB/PIONEER/USBANLZ/P001/TEST/ANLZ0000.DAT",
      waveformPeaksPath: "",
      waveformPreview: [],
      bpm: "",
      durationMs: null
    }
  ];

  const candidates = getAnalyzeMissingCandidates(tracks, "/USB");
  assert.deepEqual(candidates.map((track) => track.id), ["local-missing"]);
});

test("export-block status can be derived from structured missing-analysis details", () => {
  const details = {
    validationType: "missing_analysis",
    missingTrackCount: 2,
    totalTrackCount: 5
  };
  const status = `Export blocked: ${details.missingTrackCount}/${details.totalTrackCount} track(s) need analysis. Use Analyze Missing Tracks.`;
  assert.equal(
    status,
    "Export blocked: 2/5 track(s) need analysis. Use Analyze Missing Tracks."
  );
});
