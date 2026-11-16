/** The engines a match can use, and what each one supports. */
import type { RuleId } from "../engine/protocol";
import { HostedBrain, TengenBrain, type Brain } from "./brains";

export type EngineId = "tengen" | "rapfi" | "rapfi-classical" | "slowrenju" | "pela";

export interface Engine {
  id: EngineId;
  name: string;
  author: string;
  /** Where its source is, with its license. */
  source: string;
  license: string;
  rules: RuleId[];
  /** Largest board it plays. */
  maxSize: number;
  /** Needs shared memory, so a cross-origin isolated page. */
  threaded: boolean;
}

export const ENGINES: Engine[] = [
  {
    id: "tengen",
    name: "Tengen",
    author: "SXKA",
    source: "https://github.com/SXKA/tengen",
    license: "GPL-3.0",
    rules: ["freestyle", "standard", "renju", "caro", "freestyle-caro"],
    maxSize: 20,
    threaded: false,
  },
  {
    id: "rapfi",
    name: "Rapfi",
    author: "Haobin Duan and contributors",
    source: "https://github.com/dhbloo/rapfi",
    license: "GPL-3.0",
    rules: ["freestyle", "standard", "renju"],
    maxSize: 22,
    threaded: false,
  },
  {
    id: "rapfi-classical",
    name: "Rapfi classical",
    author: "Haobin Duan and contributors",
    source: "https://github.com/dhbloo/rapfi",
    license: "GPL-3.0",
    rules: ["freestyle", "standard", "renju"],
    maxSize: 22,
    threaded: false,
  },
  {
    id: "slowrenju",
    name: "SlowRenju",
    author: "Tianyi Hao",
    source: "https://github.com/wind23/SlowRenju",
    license: "GPL-3.0",
    rules: ["freestyle", "standard", "renju"],
    maxSize: 20,
    threaded: true,
  },
  {
    id: "pela",
    name: "Pela",
    author: "Petr Laštovička",
    source: "https://github.com/plastovicka/Piskvork",
    license: "GPL",
    rules: ["freestyle", "standard", "renju", "caro", "freestyle-caro"],
    maxSize: 20,
    threaded: false,
  },
];

export const engine = (id: EngineId): Engine => ENGINES.find((e) => e.id === id) ?? ENGINES[0];

/** Why `id` cannot play this game, or null when it can. */
export function unavailable(id: EngineId, rule: RuleId, size: number): "rule" | "size" | "isolation" | null {
  const e = engine(id);
  if (!e.rules.includes(rule)) return "rule";
  if (size > e.maxSize) return "size";
  if (e.threaded && !(typeof crossOriginIsolated !== "undefined" && crossOriginIsolated)) return "isolation";
  return null;
}

/** iPhones and iPads, iPads included when they say they are Macs. */
const appleMobile = () =>
  /iPhone|iPad|iPod/.test(navigator.userAgent) ||
  (/Macintosh/.test(navigator.userAgent) && navigator.maxTouchPoints > 1);

/**
 * Search threads for engines that can use several. One on iPhones and
 * iPads, where memory is short and threaded builds have failed to start.
 */
function threads(): number {
  if (typeof crossOriginIsolated === "undefined" || !crossOriginIsolated || appleMobile()) return 1;
  return Math.max(1, Math.min(navigator.hardwareConcurrency || 1, 8));
}

/** A new brain for `id`; `onProgress` hears about large downloads. */
export function createBrain(id: EngineId, onProgress: (fraction: number) => void): Brain {
  switch (id) {
    case "tengen":
      return new TengenBrain();
    case "rapfi":
    case "rapfi-classical": {
      const evaluator = id === "rapfi" ? "nnue" : "classical";
      return new HostedBrain(
        engine(id).name,
        "/engines/rapfi-host.js",
        (rule) => ({ evaluator, rule, threads: threads() }),
        onProgress,
      );
    }
    case "slowrenju":
    case "pela":
      return new HostedBrain(engine(id).name, "/engines/classic-host.js", () => ({ engine: id }));
  }
}
