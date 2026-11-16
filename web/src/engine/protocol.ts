/** Messages between the UI and the engine worker. */

/** A move packed as `x | y << 8`. */
export type Packed = number;

/** The game rules the engine plays. */
export type RuleId = "freestyle" | "standard" | "renju" | "caro" | "freestyle-caro";

/** Every rule, in the order the interface offers them. */
export const RULES: RuleId[] = ["freestyle", "standard", "renju", "caro", "freestyle-caro"];

/** Piskvork protocol number of each rule, as the engine takes it. */
export const RULE_NUMBER: Record<RuleId, number> = {
  freestyle: 0,
  standard: 1,
  renju: 4,
  "freestyle-caro": 8,
  caro: 9,
};

export function unpack(move: Packed): { x: number; y: number } {
  return { x: move & 0xff, y: move >> 8 };
}

/** Game state after a request, as the engine sees it. */
export interface Snapshot {
  size: number;
  /** Moves played, oldest first. */
  moves: Packed[];
  /** 0 ongoing, 1 Black won, 2 White won, 3 board full. */
  result: 0 | 1 | 2 | 3;
  /** Cells of the winning five, when there is one. */
  five: Packed[];
  /**
   * What a stone would make on each cell (index `y * size + x`) for Black
   * and White, from 0 (nothing) to 12 (five); -1 when occupied, and -2
   * where Black may not play under renju rules.
   */
  threats: { black: Int8Array; white: Int8Array };
}

/** Search progress after each completed iteration. */
export interface SearchInfo {
  depth: number;
  seldepth: number;
  /** Score for the side to move; see `mate` and `scale`. */
  value: number;
  nodes: number;
  elapsedMs: number;
  pv: Packed[];
}

export type Request =
  | { kind: "new"; size: number; rule: RuleId }
  | { kind: "play"; move: Packed }
  | { kind: "undo"; count: number }
  | { kind: "think"; ms: number; depth: number };

export type Response =
  | { kind: "ready"; mate: number; scale: number; threads: number }
  /** The engine could not start in this browser. */
  | { kind: "fatal"; message: string }
  | { kind: "info"; info: SearchInfo }
  | { kind: "snapshot"; id: number; snapshot: Snapshot }
  | { kind: "move"; id: number; move: Packed | null }
  | { kind: "error"; id: number; message: string };
