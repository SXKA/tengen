/** Promise-based access to the engine worker. */
import type { Packed, Request, Response, RuleId, SearchInfo, Snapshot } from "./protocol";

type Pending = {
  resolve: (value: Snapshot | Packed | null) => void;
  reject: (error: Error) => void;
  timer: number;
};

/** Time beyond a search's own limit before the worker counts as hung. */
const GRACE_MS = 15_000;
/** Time for loading the engine and answering a first request. */
const FIRST_MS = 45_000;

export class EngineClient {
  private worker!: Worker;
  private nextId = 1;
  private pending = new Map<number, Pending>();
  private loaded = false;
  /** Score of a win next ply, and eval units per logistic unit. */
  mate = 30000;
  scale = 200;
  /** Search threads the engine uses. */
  threads = 1;
  onInfo: (info: SearchInfo) => void = () => {};
  /** Called when a worker is ready; `recovered` after one stopped answering. */
  onReady: (recovered: boolean) => void = () => {};
  private recovering = false;

  constructor() {
    this.start();
  }

  private start() {
    this.loaded = false;
    this.worker = new Worker(new URL("./worker.ts", import.meta.url), { type: "module" });
    // A worker the browser stopped (out of memory, say) is replaced.
    this.worker.onerror = () => this.recover("the engine stopped");
    this.worker.onmessage = (event: MessageEvent<Response>) => {
      const message = event.data;
      switch (message.kind) {
        case "ready":
          this.loaded = true;
          this.mate = message.mate;
          this.scale = message.scale;
          this.threads = message.threads;
          this.onReady(this.recovering);
          this.recovering = false;
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
        case "error": {
          const p = this.pending.get(message.id);
          if (p) {
            clearTimeout(p.timer);
            p.reject(new Error(message.message));
          }
          this.pending.delete(message.id);
        }
      }
    };
  }

  private settle(id: number, value: Snapshot | Packed | null) {
    const p = this.pending.get(id);
    if (p) {
      clearTimeout(p.timer);
      p.resolve(value);
    }
    this.pending.delete(id);
  }

  /**
   * Sends `request`; a worker that does not answer in time is replaced, so
   * the page never waits for good.
   *
   * @param ms How long the answer may take, beyond loading.
   */
  private send<T extends Snapshot | Packed | null>(request: Request, ms = GRACE_MS): Promise<T> {
    const id = this.nextId++;
    return new Promise<T>((resolve, reject) => {
      const limit = this.loaded ? ms : ms + FIRST_MS;
      const timer = window.setTimeout(() => this.recover("the engine stopped responding"), limit);
      this.pending.set(id, { resolve: resolve as Pending["resolve"], reject, timer });
      this.worker.postMessage({ id, request });
    });
  }

  /** Rejects what is pending and starts a new worker, which says when it is ready. */
  private recover(reason: string) {
    this.recovering = true;
    this.worker.terminate();
    this.rejectAll(reason);
    this.start();
  }

  private rejectAll(reason: string) {
    for (const p of this.pending.values()) {
      clearTimeout(p.timer);
      p.reject(new Error(reason));
    }
    this.pending.clear();
  }

  newGame(size: number, rule: RuleId) {
    return this.send<Snapshot>({ kind: "new", size, rule });
  }

  play(move: Packed) {
    return this.send<Snapshot>({ kind: "play", move });
  }

  undo(count: number) {
    return this.send<Snapshot>({ kind: "undo", count });
  }

  /** Best move for the side to move, or null when the game is over. */
  think(ms: number, depth = 0) {
    // A depth-limited search has no time limit of its own.
    return this.send<Packed | null>({ kind: "think", ms, depth }, ms > 0 ? ms + GRACE_MS : 120_000);
  }

  /**
   * Abandon whatever the engine is doing, including a search, by replacing
   * the worker. Pending requests are rejected.
   */
  restart() {
    this.recovering = false;
    this.worker.terminate();
    this.rejectAll("engine restarted");
    this.start();
  }

  /** Stop the worker for good. */
  dispose() {
    this.worker.terminate();
    this.rejectAll("engine stopped");
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
