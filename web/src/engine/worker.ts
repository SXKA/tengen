/// <reference lib="webworker" />
/**
 * Hosts the WebAssembly engine. The engine blocks while it searches, so
 * requests are simply answered in order.
 *
 * When the page is cross-origin isolated, the multi-threaded build runs here
 * and helper workers join its searches through shared memory; otherwise the
 * single-threaded build runs alone.
 */
import singleUrl from "./tengen.wasm?url";
import threadedUrl from "./tengen-mt.wasm?url";
import { RULE_NUMBER, type Packed, type Request, type Response, type Snapshot } from "./protocol";

interface Exports {
  /** Exported by the single-threaded build only; the threaded one imports it. */
  memory: WebAssembly.Memory;
  __tls_size: WebAssembly.Global;
  __tls_align: WebAssembly.Global;
  tg_set_threads(threads: number): void;
  tg_alloc(size: number, align: number): number;
  tg_new(size: number, rule: number): number;
  tg_play(x: number, y: number): number;
  tg_undo(): number;
  tg_think(ms: number, depth: number): number;
  tg_ply(): number;
  tg_move(index: number): number;
  tg_threat(x: number, y: number, color: number): number;
  tg_five_line(): number;
  tg_line_ptr(): number;
  tg_mate(): number;
  tg_scale(): number;
}

const scope = self as unknown as DedicatedWorkerGlobalScope;
const post = (message: Response) => scope.postMessage(message);

let engine: Exports;
let memory: WebAssembly.Memory;
let size = 15;
let result: Snapshot["result"] = 0;

/** Search threads to use: one per core, within reason. */
function threadCount(): number {
  if (!scope.crossOriginIsolated || typeof SharedArrayBuffer === "undefined") return 1;
  return Math.max(1, Math.min(navigator.hardwareConcurrency || 1, 8));
}

const HELPER_STACK = 2 << 20;

async function compile(url: string): Promise<WebAssembly.Module> {
  try {
    return await WebAssembly.compileStreaming(fetch(url));
  } catch {
    // Servers that do not send application/wasm defeat compileStreaming.
    return WebAssembly.compile(await (await fetch(url)).arrayBuffer());
  }
}

const ready = (async () => {
  const threads = threadCount();
  const module = await compile(threads > 1 ? threadedUrl : singleUrl);
  // The threaded module imports its memory: shared, so helpers can join.
  const shared = threads > 1 ? new WebAssembly.Memory({ initial: 1024, maximum: 16384, shared: true }) : null;
  const instance = await WebAssembly.instantiate(module, {
    env: {
      ...(shared ? { memory: shared } : {}),
      tg_now: () => performance.timeOrigin + performance.now(),
      tg_info: (
        depth: number,
        seldepth: number,
        value: number,
        nodes: number,
        elapsedMs: number,
        ptr: number,
        len: number,
      ) => {
        const pv = Array.from(new Uint16Array(memory.buffer, ptr, len));
        post({ kind: "info", info: { depth, seldepth, value, nodes, elapsedMs, pv } });
      },
    },
  });
  engine = instance.exports as unknown as Exports;
  memory = shared ?? engine.memory;

  for (let id = 1; id < threads; id++) {
    const stack = engine.tg_alloc(HELPER_STACK, 16);
    const tls = engine.tg_alloc(engine.__tls_size.value as number, engine.__tls_align.value as number);
    const helper = new Worker(new URL("./helper.ts", import.meta.url), { type: "module" });
    helper.postMessage({ module, memory, stackTop: stack + HELPER_STACK, tls, id });
  }
  engine.tg_new(size, 0);
  engine.tg_set_threads(threads);
  post({ kind: "ready", mate: engine.tg_mate(), scale: engine.tg_scale(), threads });
})();

function snapshot(): Snapshot {
  const moves: Packed[] = [];
  for (let i = 0; i < engine.tg_ply(); i++) moves.push(engine.tg_move(i));
  const count = engine.tg_five_line();
  const five = Array.from(new Uint16Array(memory.buffer, engine.tg_line_ptr(), count));
  const black = new Int8Array(size * size);
  const white = new Int8Array(size * size);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      black[y * size + x] = engine.tg_threat(x, y, 1);
      white[y * size + x] = engine.tg_threat(x, y, 2);
    }
  }
  return { size, moves, result, five, threats: { black, white } };
}

scope.onmessage = async (event: MessageEvent<{ id: number; request: Request }>) => {
  await ready;
  const { id, request } = event.data;
  switch (request.kind) {
    case "new":
      if (engine.tg_new(request.size, RULE_NUMBER[request.rule]) !== 0) {
        post({ kind: "error", id, message: `unsupported board size ${request.size}` });
        return;
      }
      size = request.size;
      result = 0;
      break;
    case "play": {
      const status = engine.tg_play(request.move & 0xff, request.move >> 8);
      if (status < 0) {
        post({ kind: "error", id, message: "illegal move" });
        return;
      }
      result = status as Snapshot["result"];
      break;
    }
    case "undo":
      for (let i = 0; i < request.count; i++) engine.tg_undo();
      result = 0;
      break;
    case "think": {
      const move = engine.tg_think(request.ms, request.depth);
      post({ kind: "move", id, move: move < 0 ? null : move });
      return;
    }
  }
  post({ kind: "snapshot", id, snapshot: snapshot() });
};
