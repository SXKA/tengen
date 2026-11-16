/** The side panel while engines play each other. */
import type { Packed } from "../engine/protocol";
import type { Text } from "../i18n";
import { ENGINES, engine, unavailable, type EngineId } from "../match/catalog";
import type { MatchState, MatchSetup } from "../match/useMatch";
import type { Side } from "../match/match";
import { WinBar } from "./WinBar";

/** Think times offered, in milliseconds. */
export const THINK_TIMES = [200, 500, 1000, 2000, 5000, 10000];

interface Props {
  t: Text;
  state: MatchState;
  setup: MatchSetup;
  onEngine: (side: Side, id: EngineId) => void;
  onTime: (side: Side, ms: number) => void;
  onStart: () => void;
  onPause: () => void;
  onResume: () => void;
  onStop: () => void;
  colorName: (side: Side) => string;
  name: (move: Packed) => string;
  /** Black's chance to win, from the engines' scores. */
  chance: number | null;
}

export function WatchPanel({
  t,
  state,
  setup,
  onEngine,
  onTime,
  onStart,
  onPause,
  onResume,
  onStop,
  colorName,
  name,
  chance,
}: Props) {
  const sides = ["black", "white"] as const;
  const { status, end, names, thinking, download } = state;
  const blocked = sides.find((side) => unavailable(setup.engines[side], setup.rule, setup.size) !== null);
  const moves = state.snapshot?.moves ?? [];

  let heading: string;
  let note = "";
  switch (status) {
    case "idle":
      heading = t.watchIdle;
      break;
    case "loading": {
      const side = download?.side ?? state.loading;
      heading = side ? t.watchLoading(names[side]) : t.loading;
      if (download) note = t.watchDownload(names[download.side], Math.round(download.fraction * 100));
      break;
    }
    case "paused":
      heading = thinking ? t.watchPausing : t.watchPaused;
      break;
    case "over": {
      const winner = end?.winner ?? null;
      const aborted = end?.reason === "error" && !end.loser;
      heading = winner ? t.watchWins(names[winner]) : aborted ? t.watchAborted : t.draw;
      if (end) {
        const loser = end.loser ? names[end.loser] : "";
        switch (end.reason) {
          case "five":
            if (winner) note = t.endFive(names[winner], colorName(winner));
            break;
          case "full":
            note = t.endFull;
            break;
          case "illegal": {
            const [x, y] = (end.detail ?? "0,0").split(",").map(Number);
            note = t.endIllegal(loser, name(x | (y << 8)));
            break;
          }
          case "timeout":
            note = t.endTimeout(loser);
            break;
          case "error":
            note = aborted ? t.endAborted(end.detail ?? "") : t.endError(loser, end.detail ?? "");
        }
      }
      break;
    }
    default:
      heading = thinking ? t.watchThinking(colorName(thinking), names[thinking]) : t.loading;
  }
  if (!note && moves.length > 0 && status !== "over") {
    const last = moves[moves.length - 1];
    note = t.lastMove(colorName(moves.length % 2 === 1 ? "black" : "white"), name(last));
  }

  const running = status === "loading" || status === "running" || status === "paused";
  const line = thinking ? (state.info[thinking]?.pv ?? []) : [];

  const problems = sides
    .map((side) => [side, unavailable(setup.engines[side], setup.rule, setup.size)] as const)
    .filter(([, problem]) => problem !== null);
  const problemText = (problem: string | null) =>
    problem === "rule"
      ? t.notForRule
      : problem === "size"
        ? t.notForSize
        : problem === "mobile"
          ? t.notMobile
          : t.notIsolated;
  const chanceText =
    chance !== null && status !== "idle" && status !== "over" ? t.blackShare(Math.round(chance * 100)) : "";

  return (
    <>
      <section className="status" aria-live="polite">
        <div className="status-head">
          <h1 className={status === "running" || status === "loading" ? "status-text busy" : "status-text"}>
            {heading}
          </h1>
          <span className="chance">{chanceText}</span>
        </div>
        <WinBar black={status === "idle" ? null : chance} label={t.winChance} />
        <p className="status-note">{note}</p>
        {line.length > 0 && (
          <p className="line" title={thinking ? state.info[thinking]?.message : undefined}>
            <span className="line-label">{t.line}</span> {line.slice(0, 8).map(name).join(" ")}
          </p>
        )}
      </section>

      <div className="actions">
        {running ? (
          <button onClick={onStop}>{t.stop}</button>
        ) : (
          <button className="primary" onClick={onStart} disabled={blocked !== undefined}>
            {t.start}
          </button>
        )}
        {status === "paused" ? (
          <button onClick={onResume}>{t.resume}</button>
        ) : (
          <button onClick={onPause} disabled={!running}>
            {t.pause}
          </button>
        )}
      </div>

      <div className="sides">
        {sides.map((side) => (
          <div key={side} className={thinking === side ? "side thinking" : "side"}>
            <span className={`swatch swatch-${side}`} aria-hidden="true" />
            <select
              value={setup.engines[side]}
              disabled={running}
              onChange={(e) => onEngine(side, e.target.value as EngineId)}
              aria-label={t.engineFor(colorName(side))}
            >
              {ENGINES.map((e) => (
                <option key={e.id} value={e.id} disabled={unavailable(e.id, setup.rule, setup.size) !== null}>
                  {e.name}
                </option>
              ))}
            </select>
            <select
              value={setup.ms[side]}
              disabled={running}
              onChange={(e) => onTime(side, Number(e.target.value))}
              aria-label={t.thinkTime}
            >
              {THINK_TIMES.map((ms) => (
                <option key={ms} value={ms}>
                  {t.seconds(ms / 1000)}
                </option>
              ))}
            </select>
          </div>
        ))}
        {problems.map(([side, problem]) => (
          <p key={side} className="note">
            {engine(setup.engines[side]).name}：{problemText(problem)}
          </p>
        ))}
      </div>
    </>
  );
}
