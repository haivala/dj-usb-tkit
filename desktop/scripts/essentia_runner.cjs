#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");
const readline = require("readline");

const moduleRoot = process.env.DJTKIT_ESSENTIA_NODE_MODULES
  ? path.resolve(process.env.DJTKIT_ESSENTIA_NODE_MODULES)
  : path.resolve(__dirname, "..", "node_modules");
const { EssentiaWASM, Essentia } = require(path.join(moduleRoot, "essentia.js"));

function parseArg(raw) {
  try {
    return JSON.parse(raw);
  } catch {
    return null;
  }
}

function fail(error) {
  process.stdout.write(JSON.stringify({ ok: false, error }));
  process.exit(0);
}

function failWorker(error) {
  process.stdout.write(`${JSON.stringify({ ok: false, error })}\n`);
}

function decodeMonoF32FromFile(pcmPath) {
  const raw = fs.readFileSync(pcmPath);
  if (!raw || raw.length < 4) return null;
  const samples = Math.floor(raw.length / 4);
  if (samples <= 0) {
    return null;
  }
  const mono = new Float32Array(samples);
  for (let i = 0; i < samples; i += 1) {
    mono[i] = raw.readFloatLE(i * 4);
  }
  return mono;
}

async function analyzeRequest(arg, essentia) {
  if (!arg || typeof arg !== "object") {
    return { ok: false, error: "invalid args" };
  }
  const pcmPath = String(arg.pcmPath || "");
  const sampleRate = Number(arg.sampleRate || 0);
  const bpmMinRaw = Number(arg.bpmMin);
  const bpmMaxRaw = Number(arg.bpmMax);
  const bpmMin = Number.isFinite(bpmMinRaw) && bpmMinRaw > 0 ? bpmMinRaw : 70;
  const bpmMaxCandidate = Number.isFinite(bpmMaxRaw) && bpmMaxRaw > 0 ? bpmMaxRaw : 180;
  const bpmMax = bpmMaxCandidate > bpmMin ? bpmMaxCandidate : (bpmMin + 1);
  if (!Number.isFinite(sampleRate) || sampleRate <= 0) {
    return { ok: false, error: "invalid sampleRate" };
  }
  if (!pcmPath) {
    return { ok: false, error: "missing pcm payload" };
  }
  if (!fs.existsSync(pcmPath)) {
    return { ok: false, error: "pcm file missing" };
  }

  const mono = decodeMonoF32FromFile(pcmPath);
  if (!mono) {
    return { ok: false, error: "decode failed" };
  }

  try {
    const vector = essentia.arrayToVector(Array.from(mono));
    let bpm = null;
    let key = null;
    let scale = null;
    let firstBeatMs = null;
    let bpmError = null;
    let keyError = null;

    try {
      const bpmValue = essentia.PercivalBpmEstimator(vector, 1024, 2048, 128, 128, bpmMax, bpmMin, sampleRate);
      bpm = bpmValue && typeof bpmValue.bpm === "number" ? bpmValue.bpm : null;
    } catch (err) {
      bpmError = String(err && err.message ? err.message : err);
    }

    try {
      const keyValue = essentia.KeyExtractor(
        vector,
        true,
        4096,
        4096,
        12,
        3500,
        60,
        25,
        0.2,
        "bgate",
        sampleRate,
        1e-4,
        440,
        "cosine",
        "hann"
      );
      key = keyValue && typeof keyValue.key === "string" ? keyValue.key : null;
      scale = keyValue && typeof keyValue.scale === "string" ? keyValue.scale : null;
    } catch (err) {
      keyError = String(err && err.message ? err.message : err);
    }

    try {
      const rhythm = essentia.RhythmExtractor2013(vector, bpmMax, "multifeature", bpmMin);
      const ticks = rhythm && rhythm.ticks ? rhythm.ticks : null;
      let tickArray = [];
      if (ticks && typeof essentia.vectorToArray === "function") {
        tickArray = essentia.vectorToArray(ticks);
      } else if (Array.isArray(ticks)) {
        tickArray = ticks;
      }
      if (tickArray.length > 0 && Number.isFinite(tickArray[0])) {
        firstBeatMs = Math.max(0, Math.round(tickArray[0] * 1000));
      }
    } catch {}

    const keyOut = key ? (scale === "minor" ? `${key}m` : key) : null;
    if ((bpm == null || !Number.isFinite(bpm)) && !keyOut) {
      const details = [];
      if (bpmError) details.push(`bpm: ${bpmError}`);
      if (keyError) details.push(`key: ${keyError}`);
      const suffix = details.length ? ` (${details.join("; ")})` : "";
      return { ok: false, error: `essentia produced no bpm/key${suffix}` };
    }
    return { ok: true, bpm, key: keyOut, firstBeatMs };
  } catch (err) {
    return { ok: false, error: `essentia.js failed: ${String(err && err.message ? err.message : err)}` };
  }
}

async function main() {
  const workerMode = process.argv[2] === "--worker";
  let essentia = null;
  try {
    essentia = new Essentia(EssentiaWASM, false);
    if (workerMode) {
      const rl = readline.createInterface({
        input: process.stdin,
        crlfDelay: Infinity
      });
      for await (const line of rl) {
        const arg = parseArg(line || "");
        const result = await analyzeRequest(arg, essentia);
        process.stdout.write(`${JSON.stringify(result)}\n`);
      }
      return;
    }

    const arg = parseArg(process.argv[2] || "");
    const result = await analyzeRequest(arg, essentia);
    process.stdout.write(JSON.stringify(result));
  } catch (err) {
    if (workerMode) {
      failWorker(String(err && err.message ? err.message : err));
      return;
    }
    fail(String(err && err.message ? err.message : err));
  } finally {
    if (essentia && typeof essentia.shutdown === "function") {
      try { essentia.shutdown(); } catch {}
    }
  }
}

main().catch((err) => fail(String(err && err.message ? err.message : err)));
