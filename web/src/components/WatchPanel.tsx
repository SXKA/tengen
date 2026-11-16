/** The side panel while engines play each other. */
import type { Packed } from "../engine/protocol";
import type { Text } from "../i18n";
import { ENGINES, engine, unavailable, type EngineId } from "../match/catalog";
import type { MatchState, MatchSetup } from "../match/useMatch";
import type { Side } from "../match/match";

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
}

function formatRate(nps: number): string {
  if (nps >= 1e6) return `${(nps / 1e6).toFixed(1)}M`;
  if (nps >= 1e3) return `${Math.round(nps / 1e3)}k`;
  return `${Math.round(nps)}`;
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
      note = t.watchIdleNote;
      break;
    case "loading":
      heading = download ? t.watchLoading(names[download.side]) : t.loading;
      if (download) note = t.watchDownload(names[download.side], Math.round(download.fraction * 100));
      break;
    case "paused":
      heading = thinking ? t.watchPausing : t.watchPaused;
      break;
    case "over": {
      const winner = end?.winner ?? null;
      heading = winner ? t.watchWins(names[winner]) : t.draw;
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
            note = t.endError(loser, end.detail ?? "");
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

  return (
    <>
      <section className="status" aria-live="polite">
        <h1 className={status === "running" || status === "loading" ? "status-text busy" : "status-text"}>{heading}</h1>
        <p className="status-note">{note}</p>
      </section>

      <section className="readout">
        <table className="sides">
          <tbody>
            {sides.map((side) => {
              const info = state.info[side];
              return (
                <tr key={side} className={thinking === side ? "thinking" : ""}>
                  <th scope="row">
                    <span className={`swatch swatch-${side}`} aria-hidden="true" />
                    {names[side]}
                  </th>
                  <td>
                    <span className="unit">{t.depth}</span> {info?.depth ?? "–"}
                  </td>
                  <td>
                    {info?.speed !== undefined ? formatRate(info.speed) : "–"}{" "}
                    <span className="unit">{t.perSecond}</span>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
        <div className="line" title={thinking ? state.info[thinking]?.message : undefined}>
          <span className="line-label">{t.line}</span>
          <span className="line-moves">{line.length > 0 ? line.slice(0, 12).map(name).join(" ") : "–"}</span>
        </div>
      </section>

      <section className="controls">
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

        {sides.map((side) => {
          const chosen = setup.engines[side];
          const problem = unavailable(chosen, setup.rule, setup.size);
          return (
            <fieldset key={side} className="pick">
              <legend>
                <span className={`swatch swatch-${side}`} aria-hidden="true" />
                {t.engineFor(colorName(side))}
              </legend>
              <select
                value={chosen}
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
              <p className="note">
                {problem === "rule"
                  ? t.notForRule
                  : problem === "size"
                    ? t.notForSize
                    : problem === "isolation"
                      ? t.notIsolated
                      : t.engineNotes[engine(chosen).id]}
              </p>
            </fieldset>
          );
        })}
      </section>
    </>
  );
}
