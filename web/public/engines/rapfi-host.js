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
 *   { type: "error", message, retry }     retry: try again with one thread
 */
"use strict";

const BASE = new URL("./rapfi/", self.location.href).href;

// Emscripten starts Rapfi's threads with the script of the worker that
// loaded it, which is this one: those threads load Rapfi itself instead.
if (self.name === "em-pthread") importScripts(BASE + "rapfi-multi-simd128.js");
else self.onmessage = host;
const NETWORKS_COMMIT = "7eeb19b55219a48138756293f92211a1539bab4d";
const MIRRORS = [
  // This site, when its build could fetch them; then two CDNs.
  BASE + "weights/",
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
/** A download that receives nothing for this long goes to the next mirror. */
const STALL_MS = 15000;
/** How long the engine has to load, and then to print its first line. */
const LIFE_MS = 20000;

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

/**
 * Fetch a file whole, reporting progress through `onBytes`. Gives up when
 * nothing arrives for STALL_MS: some networks hold connections they block
 * open instead of refusing them.
 */
async function download(url, onBytes) {
  const abort = new AbortController();
  let timer = 0;
  const watch = () => {
    clearTimeout(timer);
    timer = setTimeout(() => abort.abort(), STALL_MS);
  };
  watch();
  try {
    const response = await fetch(url, { signal: abort.signal });
    if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
    const reader = response.body.getReader();
    const parts = [];
    let length = 0;
    for (;;) {
      watch();
      const { done, value } = await reader.read();
      if (done) break;
      parts.push(value);
      length += value.length;
      onBytes(value.length);
    }
    return join(parts, length);
  } catch (error) {
    throw abort.signal.aborted ? new Error(`${url}: no data for ${STALL_MS / 1000} s`) : error;
  } finally {
    clearTimeout(timer);
  }
}

function join(parts, length) {
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
    // Cache Storage can hang in some browsers and modes: give it a moment.
    cache = await within(caches.open(CACHE), 3000);
    const hit = await within(cache.match(name), 3000);
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
  throw new Error(`could not download Rapfi's network (${lastError && lastError.message ? lastError.message : lastError})`);
}

async function start({ evaluator, rule, threads }) {
  const files = [
    ["config.toml", BASE + (evaluator === "nnue" ? "config-nnue.toml" : "config-classical.toml")],
    ["model210901.bin", BASE + "model210901.bin"],
    ["model260809.bin", BASE + "model260809.bin"],
  ];
  const own = await Promise.all(files.map(async ([name, url]) => [name, await download(url, () => {})]));
  const names = evaluator === "nnue" ? (WEIGHTS[rule] ?? []) : [];
  // Sizes of the weight files, roughly, for the progress bar.
  const total = names.length * 10_000_000;
  let loaded = 0;
  if (names.length > 0) self.postMessage({ type: "progress", loaded: 0, total });
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
  let alive = null;
  const life = new Promise((resolve) => (alive = resolve));
  const loading = self.Rapfi({
    locateFile: (path) => BASE + path,
    onReceiveStdout: (line) => {
      alive();
      self.postMessage({ type: "line", line });
    },
    onReceiveStderr: () => {},
    onExit: (code) => self.postMessage({ type: "error", message: `Rapfi exited with code ${code}` }),
    preRun: [
      (module) => {
        for (const [name, bytes] of own.concat(big)) module.FS.writeFile("/" + name, bytes);
      },
    ],
  });
  // The threaded build waits for its threads, which some browsers never
  // start; then the page tries one thread.
  try {
    engine = await within(loading, LIFE_MS);
  } catch {
    self.postMessage({ type: "error", message: "Rapfi did not start", retry: multi });
    return;
  }
  // Main reads config.toml at startup; load it by name as well, in case.
  engine.sendCommand("RELOADCONFIG /config.toml");
  // Rapfi says it loaded the config. When the threaded build stays silent,
  // its threads did not start in this browser: the page tries one thread.
  const silent = await Promise.race([life.then(() => false), sleep(LIFE_MS).then(() => true)]);
  if (silent) {
    self.postMessage({ type: "error", message: "Rapfi did not start", retry: multi });
    return;
  }
  self.postMessage({ type: "ready", threads: multi ? threads : 1 });
  while (waiting.length > 0) engine.sendCommand(waiting.shift());
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/** `promise`, or a rejection after `ms`. */
function within(promise, ms) {
  return Promise.race([promise, sleep(ms).then(() => Promise.reject(new Error("timed out")))]);
}
