/**
 * Runs a game between two brains. The page's own Tengen worker referees: it
 * holds the position, rejects illegal moves (renju forbidden points
 * included) and finds fives.
 */
import type { EngineClient } from "../engine/client";
import type { Packed, RuleId, Snapshot } from "../engine/protocol";
import { TimeoutError, type Brain, type BrainInfo } from "./brains";

export type Side = "black" | "white";

export interface MatchSettings {
  size: number;
  rule: RuleId;
  /** Milliseconds per move for each side. */
  ms: Record<Side, number>;
  /** Moves played before the brains take over. */
  opening: Packed[];
}

export type EndReason = "five" | "full" | "illegal" | "timeout" | "error";

export interface MatchEnd {
  winner: Side | null;
  reason: EndReason;
  /** The side whose brain failed, for the reasons other than five and full. */
  loser?: Side;
  detail?: string;
}

const other = (side: Side): Side => (side === "black" ? "white" : "black");

export class Match {
  onSnapshot: (snapshot: Snapshot) => void = () => {};
  /** The side thinking now, or null between moves and while loading. */
  onThinking: (side: Side | null) => void = () => {};
  onInfo: (side: Side, info: BrainInfo) => void = () => {};
  /** The side whose engine is loading. */
  onLoading: (side: Side) => void = () => {};
  onEnd: (end: MatchEnd) => void = () => {};
  private stopped = false;
  private finished = false;
  private paused = false;
  private resume: (() => void) | null = null;

  constructor(
    private referee: EngineClient,
    private brains: Record<Side, Brain>,
    private settings: MatchSettings,
  ) {
    for (const side of ["black", "white"] as const) brains[side].onInfo = (info) => this.onInfo(side, info);
  }

  get isPaused() {
    return this.paused;
  }

  /** Over by its own rules, its brains idle and ready for another game. */
  get isFinished() {
    return this.finished;
  }

  async run() {
    const { size, rule } = this.settings;
    let snapshot: Snapshot;
    try {
      snapshot = await this.referee.newGame(size, rule);
      for (const move of this.settings.opening) snapshot = await this.referee.play(move);
    } catch (error) {
      if (this.stopped) return;
      return this.end({ winner: null, reason: "error", detail: message(error) });
    }
    if (this.stopped) return;
    this.onSnapshot(snapshot);

    for (const side of ["black", "white"] as const) {
      this.onLoading(side);
      try {
        await this.brains[side].start(size, rule);
      } catch (error) {
        return this.end({ winner: other(side), reason: "error", loser: side, detail: message(error) });
      }
      if (this.stopped) return;
    }

    for (;;) {
      if (snapshot.result === 1 || snapshot.result === 2) {
        return this.end({ winner: snapshot.result === 1 ? "black" : "white", reason: "five" });
      }
      if (snapshot.result === 3) return this.end({ winner: null, reason: "full" });
      if (this.paused) {
        await new Promise<void>((resolve) => (this.resume = resolve));
        if (this.stopped) return;
      }
      const side: Side = snapshot.moves.length % 2 === 0 ? "black" : "white";
      this.onThinking(side);
      let move: Packed;
      try {
        move = await this.brains[side].think(snapshot.moves, this.settings.ms[side]);
      } catch (error) {
        if (this.stopped) return;
        const reason = error instanceof TimeoutError ? "timeout" : "error";
        return this.end({ winner: other(side), reason, loser: side, detail: message(error) });
      }
      if (this.stopped) return;
      try {
        snapshot = await this.referee.play(move);
      } catch (error) {
        if (this.stopped) return;
        if (!(error instanceof Error && error.message === "illegal move")) {
          return this.end({ winner: null, reason: "error", detail: message(error) });
        }
        const { x, y } = { x: move & 0xff, y: move >> 8 };
        return this.end({ winner: other(side), reason: "illegal", loser: side, detail: `${x},${y}` });
      }
      if (this.stopped) return;
      this.onThinking(null);
      this.onSnapshot(snapshot);
    }
  }

  /** Stops after the move being thought over. */
  pause() {
    this.paused = true;
  }

  unpause() {
    this.paused = false;
    this.resume?.();
    this.resume = null;
  }

  /** Abandons the game at once; a brain may be thinking, so all are stopped. */
  stop() {
    this.stopped = true;
    this.resume?.();
    this.resume = null;
    this.dispose();
  }

  private end(end: MatchEnd) {
    this.onThinking(null);
    // Brains that failed are no use again; the others can play another game.
    if (end.reason === "five" || end.reason === "full") this.finished = true;
    else this.dispose();
    this.onEnd(end);
  }

  private dispose() {
    this.brains.black.dispose();
    if (this.brains.white !== this.brains.black) this.brains.white.dispose();
  }
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * A short random opening near the center, Black's and White's stones
 * alternating, so that repeated games between the same brains differ.
 */
export function randomOpening(size: number, stones: number, random = Math.random): Packed[] {
  const center = (size - 1) >> 1;
  const moves: Packed[] = [];
  while (moves.length < stones) {
    const x = center - 2 + Math.floor(random() * 5);
    const y = center - 2 + Math.floor(random() * 5);
    const move = x | (y << 8);
    if (!moves.includes(move)) moves.push(move);
  }
  return moves;
}
