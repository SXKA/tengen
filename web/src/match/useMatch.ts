/** Engine matches for the page: starts, pauses and stops them, and keeps their state. */
import { useCallback, useEffect, useRef, useState, type RefObject } from "react";
import type { EngineClient } from "../engine/client";
import type { RuleId, Snapshot } from "../engine/protocol";
import type { BrainInfo } from "./brains";
import { createBrain, engine, type EngineId } from "./catalog";
import { Match, randomOpening, type MatchEnd, type Side } from "./match";

export interface MatchSetup {
  size: number;
  rule: RuleId;
  engines: Record<Side, EngineId>;
  ms: Record<Side, number>;
  randomOpening: boolean;
}

export interface MatchState {
  status: "idle" | "loading" | "running" | "paused" | "over";
  snapshot: Snapshot | null;
  thinking: Side | null;
  names: Record<Side, string>;
  info: Record<Side, BrainInfo | null>;
  /** The side whose engine is downloading, with progress from 0 to 1. */
  download: { side: Side; fraction: number } | null;
  end: MatchEnd | null;
}

const idle = (engines: Record<Side, EngineId>): MatchState => ({
  status: "idle",
  snapshot: null,
  thinking: null,
  names: { black: engine(engines.black).name, white: engine(engines.white).name },
  info: { black: null, white: null },
  download: null,
  end: null,
});

/**
 * @param referee The worker that holds the position: the page's own Tengen.
 * @param engines Engines chosen now, named while no match runs.
 */
export function useMatch(referee: RefObject<EngineClient | null>, engines: Record<Side, EngineId>) {
  const [state, setState] = useState<MatchState>(() => idle(engines));
  const match = useRef<Match | null>(null);

  const stop = useCallback(() => {
    match.current?.stop();
    match.current = null;
  }, []);

  // Stop for good when the page goes.
  useEffect(() => stop, [stop]);

  // Show the chosen names before a match starts.
  useEffect(() => {
    setState((s) => (s.status === "idle" ? idle(engines) : s));
  }, [engines.black, engines.white]);

  const start = useCallback(
    (setup: MatchSetup) => {
      const client = referee.current;
      if (!client) return;
      stop();
      const progress = (side: Side, fraction: number) =>
        setState((s) => (s.status === "loading" ? { ...s, download: { side, fraction } } : s));
      const brains = {
        black: createBrain(setup.engines.black, (fraction) => progress("black", fraction)),
        white: createBrain(setup.engines.white, (fraction) => progress("white", fraction)),
      };
      const current = new Match(client, brains, {
        size: setup.size,
        rule: setup.rule,
        ms: setup.ms,
        opening: setup.randomOpening ? randomOpening(setup.size, 3) : [],
      });
      match.current = current;
      const live = (update: (s: MatchState) => MatchState) => {
        if (match.current === current) setState(update);
      };
      current.onSnapshot = (snapshot) => live((s) => ({ ...s, snapshot }));
      current.onThinking = (thinking) =>
        live((s) => ({
          ...s,
          thinking,
          download: null,
          status: thinking === null ? s.status : current.isPaused ? "paused" : "running",
        }));
      current.onInfo = (side, info) =>
        live((s) => ({
          ...s,
          // A message alone keeps the last search report's numbers.
          info: {
            ...s.info,
            [side]: info.depth === undefined && s.info[side] ? { ...s.info[side], message: info.message } : info,
          },
        }));
      current.onEnd = (end) => live((s) => ({ ...s, status: "over", thinking: null, download: null, end }));
      setState({
        ...idle(setup.engines),
        status: "loading",
        names: { black: brains.black.name, white: brains.white.name },
      });
      void current.run();
    },
    [referee, stop],
  );

  const pause = useCallback(() => {
    match.current?.pause();
    setState((s) => (s.status === "running" || s.status === "loading" ? { ...s, status: "paused" } : s));
  }, []);

  const resume = useCallback(() => {
    match.current?.unpause();
    setState((s) => (s.status === "paused" ? { ...s, status: "running" } : s));
  }, []);

  /** Abandons the match and clears the board. */
  const reset = useCallback(() => {
    stop();
    setState(idle(engines));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [stop, engines.black, engines.white]);

  return { state, start, pause, resume, reset };
}
