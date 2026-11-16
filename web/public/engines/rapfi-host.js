/**
 * Runs Rapfi (https://github.com/dhbloo/rapfi, GPL-3.0) in a worker for the
 * engine matches. The files it loads are in ./rapfi, built by the Rapfi WASM
 * workflow; the NNUE weights come from rapfi-networks (CC0).
 *
 * Messages in:
 *   { type: "start", evaluator: "nnue" | "classical", rule, threads }
 *   { type: "command", command }   one Piskvork command; BOARD with its lines
 * Messages out:
 *   { type: "progress", loaded, total }   bytes of weights downloaded
 *   { type: "ready", threads }
 *   { type: "line", line }                a line the engine printed
 *   { type: "error", message }
 */
"use strict";

const BASE = new URL("./rapfi/", self.location.href).href;

// Emscripten starts Rapfi's threads with the script of the worker that
// loaded it, which is this one: those threads load Rapfi itself instead.
if (self.name === "em-pthread") importScripts(BASE + "rapfi-multi-simd128.js");
else self.onmessage = host;
const NETWORKS_COMMIT = "7eeb19b55219a48138756293f92211a1539bab4d";
const MIRRORS = [
  `https://cdn.jsdelivr.net/gh/dhbloo/rapfi-networks@${NETWORKS_COMMIT}/mix9svq/`,
  `https://raw.githubusercontent.com/dhbloo/rapfi-networks/${NETWORKS_COMMIT}/mix9svq/`,
];
/** NNUE weight files by rule; other rules play with the classical model. */
const WEIGHTS = {
  freestyle: ["mix9svqfreestyle_bsmix.bin.lz4"],
  standard: ["mix9svqstandard_bs15.bin.lz4"],
  renju: ["mix9svqrenju_bs15_black.bin.lz4", "mix9svqrenju_bs15_white.bin.lz4"],
};
const CACHE = `rapfi-weights-${NETWORKS_COMMIT}`;

let engine = null;
const waiting = [];

function host(event) {
  const message = event.data;
  if (message.type === "start") {
    start(message).catch((error) =>
      self.postMessage({ type: "error", message: String(error && error.message ? error.message : error) }),
    );
  } else if (message.type === "command") {
    if (engine) engine.sendCommand(message.command);
    else waiting.push(message.command);
  }
}

/** Fetch a file whole, reporting progress through `onBytes`. */
async function download(url, onBytes) {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
  if (!response.body) {
    const bytes = new Uint8Array(await response.arrayBuffer());
    onBytes(bytes.length);
    return bytes;
  }
  const reader = response.body.getReader();
  const parts = [];
  let length = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    parts.push(value);
    length += value.length;
    onBytes(value.length);
  }
  const bytes = new Uint8Array(length);
  let offset = 0;
  for (const part of parts) {
    bytes.set(part, offset);
    offset += part.length;
  }
  return bytes;
}

/** A weight file from the cache, else from the first mirror that has it. */
async function weights(name, onBytes) {
  let cache = null;
  try {
    cache = await caches.open(CACHE);
    const hit = await cache.match(name);
    if (hit) {
      const bytes = new Uint8Array(await hit.arrayBuffer());
      onBytes(bytes.length);
      return bytes;
    }
  } catch {
    // No Cache Storage here (some private modes): download every time.
  }
  let lastError = null;
  for (const mirror of MIRRORS) {
    let counted = 0;
    try {
      const bytes = await download(mirror + name, (n) => {
        counted += n;
        onBytes(n);
      });
      if (cache) cache.put(name, new Response(bytes)).catch(() => {});
      return bytes;
    } catch (error) {
      onBytes(-counted);
      lastError = error;
    }
  }
  throw lastError;
}

async function start({ evaluator, rule, threads }) {
  const files = [
    ["config.toml", BASE + (evaluator === "nnue" ? "config-nnue.toml" : "config-classical.toml")],
    ["model210901.bin", BASE + "model210901.bin"],
    ["model260809.bin", BASE + "model260809.bin"],
  ];
  const own = await Promise.all(
    files.map(async ([name, url]) => {
      const response = await fetch(url);
      if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
      return [name, new Uint8Array(await response.arrayBuffer())];
    }),
  );
  const names = evaluator === "nnue" ? (WEIGHTS[rule] ?? []) : [];
  // Sizes of the weight files, roughly, for the progress bar.
  const total = names.length * 10_000_000;
  let loaded = 0;
  const big = await Promise.all(
    names.map(async (name) => [
      name,
      await weights(name, (n) => {
        loaded += n;
        self.postMessage({ type: "progress", loaded, total: Math.max(total, loaded) });
      }),
    ]),
  );

  const multi = threads > 1 && self.crossOriginIsolated && typeof SharedArrayBuffer !== "undefined";
  const script = BASE + (multi ? "rapfi-multi-simd128.js" : "rapfi-single-simd128.js");
  importScripts(script);
  engine = await self.Rapfi({
    locateFile: (path) => BASE + path,
    onReceiveStdout: (line) => self.postMessage({ type: "line", line }),
    onReceiveStderr: () => {},
    onExit: (code) => self.postMessage({ type: "error", message: `Rapfi exited with code ${code}` }),
    preRun: [
      (module) => {
        for (const [name, bytes] of own.concat(big)) module.FS.writeFile("/" + name, bytes);
      },
    ],
  });
  // Main reads config.toml at startup; load it by name as well, in case.
  engine.sendCommand("RELOADCONFIG /config.toml");
  self.postMessage({ type: "ready", threads: multi ? threads : 1 });
  while (waiting.length > 0) engine.sendCommand(waiting.shift());
}
