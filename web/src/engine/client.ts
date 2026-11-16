/** Promise-based access to the engine worker. */
import type { Packed, Request, Response, SearchInfo, Snapshot } from "./protocol";

type Pending = { resolve: (value: Snapshot | Packed | null) => void; reject: (error: Error) => void };

export class EngineClient {
  private worker!: Worker;
  private nextId = 1;
  private pending = new Map<number, Pending>();
  /** Score of a win next ply, and eval units per logistic unit. */
  mate = 30000;
  scale = 200;
  /** Search threads the engine uses. */
  threads = 1;
  onInfo: (info: SearchInfo) => void = () => {};
  onReady: () => void = () => {};

  constructor() {
    this.start();
  }

  private start() {
    this.worker = new Worker(new URL("./worker.ts", import.meta.url), { type: "module" });
    this.worker.onmessage = (event: MessageEvent<Response>) => {
      const message = event.data;
      switch (message.kind) {
        case "ready":
          this.mate = message.mate;
          this.scale = message.scale;
          this.threads = message.threads;
          this.onReady();
          return;
        case "info":
          this.onInfo(message.info);
          return;
        case "snapshot":
          this.settle(message.id, message.snapshot);
          return;
        case "move":
          this.settle(message.id, message.move);
          return;
        case "error":
          this.pending.get(message.id)?.reject(new Error(message.message));
          this.pending.delete(message.id);
      }
    };
  }

  private settle(id: number, value: Snapshot | Packed | null) {
    this.pending.get(id)?.resolve(value);
    this.pending.delete(id);
  }

  private send<T extends Snapshot | Packed | null>(request: Request): Promise<T> {
    const id = this.nextId++;
    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, { resolve: resolve as Pending["resolve"], reject });
      this.worker.postMessage({ id, request });
    });
  }

  newGame(size: number) {
    return this.send<Snapshot>({ kind: "new", size });
  }

  play(move: Packed) {
    return this.send<Snapshot>({ kind: "play", move });
  }

  undo(count: number) {
    return this.send<Snapshot>({ kind: "undo", count });
  }

  /** Best move for the side to move, or null when the game is over. */
  think(ms: number, depth = 0) {
    return this.send<Packed | null>({ kind: "think", ms, depth });
  }

  /**
   * Abandon whatever the engine is doing, including a search, by replacing
   * the worker. Pending requests are rejected.
   */
  restart() {
    this.worker.terminate();
    for (const p of this.pending.values()) p.reject(new Error("engine restarted"));
    this.pending.clear();
    this.start();
  }

  /** Stop the worker for good. */
  dispose() {
    this.worker.terminate();
    for (const p of this.pending.values()) p.reject(new Error("engine stopped"));
    this.pending.clear();
  }

  /** Black's chance to win for a score from `color`'s point of view. */
  blackWinChance(value: number, color: "black" | "white") {
    const own = Math.abs(value) >= this.mate - 1000 ? (value > 0 ? 1 : 0) : 1 / (1 + Math.exp(-value / this.scale));
    return color === "black" ? own : 1 - own;
  }

  /** Moves to a forced win (positive) or loss (negative), if the score is a mate. */
  mateIn(value: number): number | null {
    if (value >= this.mate - 1000) return Math.floor((this.mate - value + 1) / 2);
    if (value <= -(this.mate - 1000)) return -Math.floor((this.mate + value) / 2);
    return null;
  }
}
