/**
 * Engines that can play a side in an engine match, each in its own worker:
 * Tengen through its own interface, the others through the Piskvork
 * protocol.
 */
import { EngineClient } from "../engine/client";
import { RULE_NUMBER, type Packed, type RuleId } from "../engine/protocol";

/** What a brain reports while it thinks. */
export interface BrainInfo {
  /** Search depth, when the brain says. */
  depth?: number;
  /** Black's chance to win, 0 to 1, when the brain gives a score. */
  blackChance?: number;
  /** Moves to a forced win (positive) or loss for the side to move. */
  mate?: number | null;
  /** Positions per second. */
  speed?: number;
  /** Expected continuation, starting with the brain's move. */
  pv?: Packed[];
  /** A message the brain printed, shown as is. */
  message?: string;
}

export interface Brain {
  readonly name: string;
  /** Prepares a new game; may download and compile the engine first. */
  start(size: number, rule: RuleId): Promise<void>;
  /** The move for the side to move after `moves`, thought over for about `ms`. */
  think(moves: Packed[], ms: number): Promise<Packed>;
  onInfo: (info: BrainInfo) => void;
  /** Search threads in use, once started. */
  threads: number;
  /** Stops the brain for good, abandoning any search. */
  dispose(): void;
}

/** Tengen in its own worker, so two Tengens do not share a hash table. */
export class TengenBrain implements Brain {
  readonly name = "Tengen";
  threads = 1;
  onInfo: (info: BrainInfo) => void = () => {};
  private client = new EngineClient();
  private ready = new Promise<void>((resolve) => (this.client.onReady = resolve));
  private size = 15;
  private rule: RuleId = "freestyle";
  /** Moves the worker's board holds. */
  private synced: Packed[] = [];
  private color: "black" | "white" = "black";

  constructor() {
    this.client.onInfo = (info) => {
      this.onInfo({
        depth: info.depth,
        blackChance: this.client.blackWinChance(info.value, this.color),
        mate: this.client.mateIn(info.value),
        speed: (info.nodes / Math.max(info.elapsedMs, 1)) * 1000,
        pv: info.pv,
      });
    };
  }

  async start(size: number, rule: RuleId) {
    await this.ready;
    this.threads = this.client.threads;
    this.size = size;
    this.rule = rule;
    await this.client.newGame(size, rule);
    this.synced = [];
  }

  async think(moves: Packed[], ms: number): Promise<Packed> {
    const follows = this.synced.length <= moves.length && this.synced.every((m, i) => moves[i] === m);
    if (!follows) {
      await this.client.newGame(this.size, this.rule);
      this.synced = [];
    }
    for (const move of moves.slice(this.synced.length)) {
      await this.client.play(move);
      this.synced.push(move);
    }
    this.color = moves.length % 2 === 0 ? "black" : "white";
    const move = await this.client.think(ms);
    if (move === null) throw new Error("Tengen found no move");
    return move;
  }

  dispose() {
    this.client.dispose();
  }
}

/** A line-based connection to a Piskvork brain. */
export interface Transport {
  /** Sends one command; BOARD goes with its lines and DONE. */
  send(command: string): void;
  onLine: (line: string) => void;
  /** Called once if the connection fails or the brain quits. */
  onClose: (reason: string) => void;
  close(): void;
}

/** Extra time a brain may take beyond its turn before it loses on time. */
const GRACE_MS = 5000;
/** Time a brain may take to load and answer START. */
const START_MS = 120_000;

/**
 * Any brain that speaks the Piskvork protocol. Every move sends the whole
 * position with BOARD, so the brain needs no memory of the game.
 */
export class PiskvorkBrain implements Brain {
  threads = 1;
  onInfo: (info: BrainInfo) => void = () => {};
  private waiter: {
    resolve: (line: string) => void;
    reject: (e: Error) => void;
    accept: (line: string) => boolean;
  } | null = null;
  private closed: string | null = null;
  private turnMs = -1;
  /** The side to move in the position being thought over. */
  private color: "black" | "white" = "black";

  /** Clears an earlier failure, for a new connection. */
  protected reopen() {
    this.closed = null;
  }

  constructor(
    readonly name: string,
    private transport: Transport,
  ) {
    transport.onLine = (line) => this.receive(line.trim());
    transport.onClose = (reason) => {
      this.closed = reason;
      this.waiter?.reject(new Error(reason));
      this.waiter = null;
    };
  }

  private receive(line: string) {
    if (line === "") return;
    const space = line.indexOf(" ");
    const head = (space < 0 ? line : line.slice(0, space)).toUpperCase();
    const rest = space < 0 ? "" : line.slice(space + 1);
    if (head === "MESSAGE" || head === "DEBUG") {
      if (rest.trim() !== "") this.onInfo(parseMessage(rest.trim(), this.color));
      return;
    }
    if (head === "ERROR" || head === "UNKNOWN") {
      this.waiter?.reject(new Error(`${this.name}: ${line}`));
      this.waiter = null;
      return;
    }
    if (this.waiter?.accept(line)) {
      this.waiter.resolve(line);
      this.waiter = null;
    }
  }

  /** Sends `command` and waits for a reply that `accept` takes. */
  private ask(command: string, accept: (line: string) => boolean, ms: number): Promise<string> {
    if (this.closed !== null) return Promise.reject(new Error(this.closed));
    return new Promise<string>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.waiter = null;
        reject(new TimeoutError(`${this.name} did not answer in time`));
      }, ms);
      this.waiter = {
        accept,
        resolve: (line) => {
          clearTimeout(timer);
          resolve(line);
        },
        reject: (error) => {
          clearTimeout(timer);
          reject(error);
        },
      };
      this.transport.send(command);
    });
  }

  async start(size: number, rule: RuleId) {
    this.turnMs = -1;
    await this.ask(`START ${size}`, (line) => line.toUpperCase() === "OK", START_MS);
    this.transport.send(`INFO rule ${RULE_NUMBER[rule]}`);
    this.transport.send("INFO game_type 1");
    // Effectively no game clock: every move has its own limit.
    this.transport.send("INFO timeout_match 100000000");
    this.transport.send("INFO time_left 100000000");
    if (this.threads > 1) this.transport.send(`INFO thread_num ${this.threads}`);
  }

  async think(moves: Packed[], ms: number): Promise<Packed> {
    if (ms !== this.turnMs) {
      this.transport.send(`INFO timeout_turn ${ms}`);
      this.turnMs = ms;
    }
    this.color = moves.length % 2 === 0 ? "black" : "white";
    let command = "BEGIN";
    if (moves.length > 0) {
      // 1 marks the stones of the side to move, 2 the opponent's.
      const own = moves.length % 2;
      const lines = moves.map((m, i) => `${m & 0xff},${m >> 8},${i % 2 === own ? 1 : 2}`);
      command = ["BOARD", ...lines, "DONE"].join("\n");
    }
    const line = await this.ask(command, (l) => /^\d+\s*,\s*\d+$/.test(l), ms + GRACE_MS);
    const [x, y] = line.split(",").map((n) => parseInt(n, 10));
    return x | (y << 8);
  }

  dispose() {
    this.closed ??= "stopped";
    this.waiter?.reject(new Error("stopped"));
    this.waiter = null;
    this.transport.close();
  }
}

export class TimeoutError extends Error {}

/** Eval units per logistic unit in Rapfi's scores. */
const RAPFI_SCALE = 200;

/**
 * What a MESSAGE line says. Rapfi's search reports, such as
 * `Speed 308K | Depth 5-7 | Eval -95 | Node 616 | Time 2ms | H10 J8`, give
 * depth, score, speed and line; SlowRenju's, such as
 * `Nodes=96556, Time=502ms, speed=192342, depth=7-13, Evaluation=-8`, give
 * depth and speed (its scores are on a scale of its own). Anything else is
 * kept as text.
 */
export function parseMessage(text: string, color: "black" | "white"): BrainInfo {
  const info: BrainInfo = { message: text };
  const fields = text.split(text.includes("|") ? "|" : ",").map((f) => f.trim());
  for (const field of fields) {
    const [key, value = ""] = field.includes("=") ? field.split("=", 2) : field.split(/\s+/, 2);
    switch (key.toLowerCase()) {
      case "depth": {
        const depth = parseInt(value, 10);
        if (!Number.isNaN(depth)) info.depth = depth;
        break;
      }
      case "eval": {
        const mate = /^([+-])M(\d+)$/.exec(value);
        let own: number | null = null;
        if (mate) {
          const moves = Math.floor((parseInt(mate[2], 10) + 1) / 2);
          info.mate = mate[1] === "+" ? moves : -moves;
          own = mate[1] === "+" ? 1 : 0;
        } else if (/^[+-]?\d+$/.test(value)) {
          own = 1 / (1 + Math.exp(-parseInt(value, 10) / RAPFI_SCALE));
          info.mate = null;
        }
        if (own !== null) info.blackChance = color === "black" ? own : 1 - own;
        break;
      }
      case "speed": {
        const speed = /^([\d.]+)([KMG]?)$/i.exec(value);
        if (speed) {
          const unit = { "": 1, K: 1e3, M: 1e6, G: 1e9 }[speed[2].toUpperCase() as "" | "K" | "M" | "G"];
          info.speed = parseFloat(speed[1]) * unit;
        }
        break;
      }
    }
  }
  // The line is the last field when it is made of coordinates like H10.
  const last = fields[fields.length - 1] ?? "";
  if (fields.length > 1 && /^([A-Za-z]\d{1,2})(\s+[A-Za-z]\d{1,2})*$/.test(last)) {
    info.pv = last
      .split(/\s+/)
      .map((c) => (c.toUpperCase().charCodeAt(0) - 65) | ((parseInt(c.slice(1), 10) - 1) << 8));
  }
  return info;
}

/**
 * A brain in a host worker under /engines: the host loads the WebAssembly
 * build named in the start message and passes commands and lines.
 */
export class WorkerTransport implements Transport {
  onLine: (line: string) => void = () => {};
  onClose: (reason: string) => void = () => {};
  /** Download progress of large files, 0 to 1. */
  onProgress: (fraction: number) => void = () => {};
  /** Resolves with the threads the brain uses, once it is loaded. */
  readonly ready: Promise<number>;
  private worker: Worker;

  constructor(host: string, start: Record<string, unknown>) {
    this.worker = new Worker(host);
    this.ready = new Promise<number>((resolve, reject) => {
      this.worker.onmessage = (event: MessageEvent) => {
        const message = event.data;
        switch (message.type) {
          case "progress":
            this.onProgress(message.total > 0 ? message.loaded / message.total : 0);
            break;
          case "ready":
            resolve(message.threads);
            break;
          case "line":
            this.onLine(message.line);
            break;
          case "error":
            reject(new Error(message.message));
            this.onClose(message.message);
        }
      };
      this.worker.onerror = (event) => {
        const reason = event.message || "the engine failed to load";
        reject(new Error(reason));
        this.onClose(reason);
      };
    });
    this.worker.postMessage({ type: "start", ...start });
  }

  send(command: string) {
    this.worker.postMessage({ type: "command", command });
  }

  close() {
    this.worker.terminate();
  }
}

/** Passes a brain's lines through, whichever connection carries them now. */
class Relay implements Transport {
  onLine: (line: string) => void = () => {};
  onClose: (reason: string) => void = () => {};
  private inner: Transport | null = null;

  attach(inner: Transport) {
    this.inner?.close();
    this.inner = inner;
    inner.onLine = (line) => this.onLine(line);
    inner.onClose = (reason) => this.onClose(reason);
  }

  send(command: string) {
    this.inner?.send(command);
  }

  close() {
    this.inner?.close();
    this.inner = null;
  }
}

/** A brain in a host worker, loaded again for every game. */
export class HostedBrain extends PiskvorkBrain {
  private relay: Relay;

  /**
   * @param name Shown in the interface.
   * @param host The host worker's script.
   * @param startMessage What the host needs to load the brain for `rule`.
   * @param onProgress Download progress, 0 to 1, while it loads.
   */
  constructor(
    name: string,
    private host: string,
    private startMessage: (rule: RuleId) => Record<string, unknown>,
    private onProgress: (fraction: number) => void = () => {},
  ) {
    const relay = new Relay();
    super(name, relay);
    this.relay = relay;
  }

  async start(size: number, rule: RuleId) {
    const transport = new WorkerTransport(this.host, this.startMessage(rule));
    transport.onProgress = this.onProgress;
    this.relay.attach(transport);
    this.reopen();
    this.threads = await transport.ready;
    await super.start(size, rule);
  }
}
