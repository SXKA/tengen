/**
 * Runs a classic Piskvork brain compiled to WebAssembly (./classic, built by
 * the Classic engines WASM workflow) in a worker for the engine matches:
 * Pela by Petr Lastovicka and SlowRenju by Tianyi Hao, both GPL; see
 * ./classic/NOTICE.md. Each command is answered before the next one is
 * handled.
 *
 * Messages in:
 *   { type: "start", engine: "pela" | "slowrenju" }
 *   { type: "command", command }   one Piskvork command; BOARD with its lines
 * Messages out:
 *   { type: "ready", threads: 1 }
 *   { type: "line", line }         a line the brain printed
 *   { type: "error", message }
 */
"use strict";

const BASE = new URL("./classic/", self.location.href).href;

// Emscripten starts SlowRenju's timer thread with the script of the worker
// that loaded it, which is this one: that thread loads SlowRenju instead.
if (self.name === "em-pthread") importScripts(BASE + "slowrenju.js");
else self.onmessage = host;

let brain = null;
const waiting = [];

function run(command) {
  try {
    brain.ccall("piskvork_command", null, ["string"], [command]);
  } catch (error) {
    self.postMessage({ type: "error", message: `the brain stopped: ${error && error.message ? error.message : error}` });
  }
}

function host(event) {
  const message = event.data;
  if (message.type === "start") {
    start(message.engine).catch((error) =>
      self.postMessage({ type: "error", message: String(error && error.message ? error.message : error) }),
    );
  } else if (message.type === "command") {
    if (brain) run(message.command);
    else waiting.push(message.command);
  }
}

async function start(engine) {
  if (!/^[a-z]+$/.test(engine)) throw new Error(`unknown engine ${engine}`);
  const script = BASE + engine + ".js";
  importScripts(script);
  brain = await self.Brain({
    locateFile: (path) => BASE + path,
    print: (line) => self.postMessage({ type: "line", line }),
    printErr: () => {},
  });
  self.postMessage({ type: "ready", threads: 1 });
  while (waiting.length > 0) run(waiting.shift());
}
