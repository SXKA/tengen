/// <reference lib="webworker" />
/**
 * Hosts the WebAssembly engine. The module is single-threaded and blocks
 * while it searches, so requests are simply answered in order.
 */
import wasmUrl from "./tengen.wasm?url";
import type { Packed, Request, Response, Snapshot } from "./protocol";

interface Exports {
  memory: WebAssembly.Memory;
  tg_new(size: number): number;
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
let size = 15;
let result: Snapshot["result"] = 0;

const ready = (async () => {
  const bytes = await (await fetch(wasmUrl)).arrayBuffer();
  const { instance } = await WebAssembly.instantiate(bytes, {
    env: {
      tg_now: () => performance.now(),
      tg_info: (
        depth: number,
        seldepth: number,
        value: number,
        nodes: number,
        elapsedMs: number,
        ptr: number,
        len: number,
      ) => {
        const pv = Array.from(new Uint16Array(engine.memory.buffer, ptr, len));
        post({ kind: "info", info: { depth, seldepth, value, nodes, elapsedMs, pv } });
      },
    },
  });
  engine = instance.exports as unknown as Exports;
  engine.tg_new(size);
  post({ kind: "ready", mate: engine.tg_mate(), scale: engine.tg_scale() });
})();

function snapshot(): Snapshot {
  const moves: Packed[] = [];
  for (let i = 0; i < engine.tg_ply(); i++) moves.push(engine.tg_move(i));
  const count = engine.tg_five_line();
  const five = Array.from(new Uint16Array(engine.memory.buffer, engine.tg_line_ptr(), count));
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
      if (engine.tg_new(request.size) !== 0) {
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
