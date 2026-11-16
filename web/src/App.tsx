import { useCallback, useEffect, useRef, useState } from "react";
import { Board, type Ghost, type StoneColor } from "./components/Board";
import { WatchPanel, THINK_TIMES } from "./components/WatchPanel";
import { WinBar } from "./components/WinBar";
import { EngineClient } from "./engine/client";
import { RULES, unpack, type Packed, type RuleId, type SearchInfo, type Snapshot } from "./engine/protocol";
import { coordinate, defaultLang, text, type Lang } from "./i18n";
import { ENGINES, type EngineId } from "./match/catalog";
import type { Side } from "./match/match";
import { useMatch } from "./match/useMatch";

/** Think time and depth cap per strength level. */
const LEVELS = [
  { ms: 400, depth: 3 },
  { ms: 700, depth: 0 },
  { ms: 2500, depth: 0 },
  { ms: 7000, depth: 0 },
];
const SIZES = [15, 20];
const HINT_MS = 1200;

type Mode = "play" | "watch";

interface Settings {
  mode: Mode;
  human: StoneColor;
  level: number;
  size: number;
  rule: RuleId;
  threats: boolean;
  /** Move numbers on the stones. */
  numbers: boolean;
  lang: Lang;
  /** Engine matches: who plays each side, for how long a move. */
  engines: Record<Side, EngineId>;
  ms: Record<Side, number>;
  randomOpening: boolean;
}

const STORAGE_KEY = "tengen.settings";

function loadSettings(): Settings {
  const fallback: Settings = {
    mode: "play",
    human: "black",
    level: 1,
    size: 15,
    rule: "freestyle",
    threats: false,
    numbers: false,
    lang: defaultLang(),
    engines: { black: "tengen", white: "rapfi-classical" },
    ms: { black: 1000, white: 1000 },
    randomOpening: false,
  };
  try {
    const saved = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "{}") as Partial<Settings>;
    const settings = { ...fallback, ...saved };
    if (!RULES.includes(settings.rule)) settings.rule = fallback.rule;
    if (settings.mode !== "watch") settings.mode = "play";
    const known = (id: unknown) => ENGINES.some((e) => e.id === id);
    settings.engines = {
      black: known(settings.engines?.black) ? settings.engines.black : fallback.engines.black,
      white: known(settings.engines?.white) ? settings.engines.white : fallback.engines.white,
    };
    const time = (ms: unknown, side: Side) => (THINK_TIMES.includes(ms as number) ? (ms as number) : fallback.ms[side]);
    settings.ms = { black: time(settings.ms?.black, "black"), white: time(settings.ms?.white, "white") };
    return settings;
  } catch {
    return fallback;
  }
}

function saveSettings(settings: Settings) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
  } catch {
    // Storage can be unavailable (private mode); settings then last for the visit.
  }
}

type Phase = "loading" | "human" | "engine" | "hint" | "over";

const sideToMove = (moves: Packed[]): StoneColor => (moves.length % 2 === 0 ? "black" : "white");

function formatRate(nps: number): string {
  if (nps >= 1e6) return `${(nps / 1e6).toFixed(1)}M`;
  if (nps >= 1e3) return `${Math.round(nps / 1e3)}k`;
  return `${Math.round(nps)}`;
}

export function App() {
  const [settings, setSettings] = useState(loadSettings);
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [phase, setPhase] = useState<Phase>("loading");
  // The latest search report, who was searching, and how many moves had
  // been played when it started.
  const [info, setInfo] = useState<{ info: SearchInfo; color: StoneColor; ply: number } | null>(null);
  const [hint, setHint] = useState<Packed | null>(null);
  const [previewLine, setPreviewLine] = useState(false);
  const [tapPending, setTapPending] = useState(false);

  const t = text[settings.lang];
  const engine = useRef<EngineClient | null>(null);
  const settingsRef = useRef(settings);
  settingsRef.current = settings;
  // Bumped by every new game so late answers from an older game are ignored.
  const generation = useRef(0);
  const searching = useRef<{ color: StoneColor; ply: number }>({ color: "black", ply: 0 });
  const watch = useMatch(engine, settings.engines);
  const watching = settings.mode === "watch";

  const update = (patch: Partial<Settings>) => {
    setSettings((s) => {
      const next = { ...s, ...patch };
      saveSettings(next);
      return next;
    });
  };

  const startGame = useCallback((size: number, rule: RuleId) => {
    const client = engine.current;
    if (!client) return;
    generation.current++;
    setInfo(null);
    setHint(null);
    setPhase("loading");
    client.newGame(size, rule).then(setSnapshot, () => {});
  }, []);

  useEffect(() => {
    const client = new EngineClient();
    engine.current = client;
    client.onReady = () => {
      // While engines play, the worker referees their games instead.
      if (settingsRef.current.mode === "play") startGame(settingsRef.current.size, settingsRef.current.rule);
    };
    client.onInfo = (i) => setInfo({ info: i, ...searching.current });
    return () => client.dispose();
  }, [startGame]);

  // Decide whose turn it is after every change of position.
  useEffect(() => {
    const client = engine.current;
    if (!snapshot || !client || settingsRef.current.mode !== "play") return;
    if (snapshot.result !== 0) {
      setPhase("over");
      return;
    }
    const toMove = sideToMove(snapshot.moves);
    if (toMove === settings.human) {
      setPhase("human");
      return;
    }
    const game = generation.current;
    const level = LEVELS[settings.level];
    searching.current = { color: toMove, ply: snapshot.moves.length };
    setPhase("engine");
    client
      .think(level.ms, level.depth)
      .then((move) => (game === generation.current && move !== null ? client.play(move) : null))
      .then((next) => {
        if (next && game === generation.current) setSnapshot(next);
      })
      .catch(() => {});
    // Only a new position should start a search, not a settings change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [snapshot]);

  const place = (move: Packed) => {
    const client = engine.current;
    if (!client || phase !== "human") return;
    setHint(null);
    client.play(move).then(setSnapshot, () => {});
  };

  const newGame = (patch: Partial<Settings> = {}) => {
    const next = { ...settings, ...patch };
    update(patch);
    const client = engine.current;
    if (!client) return;
    if (phase === "engine" || phase === "hint") {
      // A running search cannot be interrupted, so replace the worker; it
      // starts the new game once it is ready.
      generation.current++;
      setPhase("loading");
      settingsRef.current = next;
      client.restart();
    } else {
      startGame(next.size, next.rule);
    }
  };

  /** Switch between playing and watching; each starts afresh. */
  const switchMode = (mode: Mode) => {
    if (mode === settings.mode) return;
    const client = engine.current;
    settingsRef.current = { ...settings, mode };
    update({ mode });
    generation.current++;
    setInfo(null);
    setHint(null);
    if (mode === "watch") {
      // Abandon Tengen's search for the person's game, if one runs.
      if (client && (phase === "engine" || phase === "hint")) client.restart();
    } else {
      watch.reset();
      startGame(settings.size, settings.rule);
    }
  };

  /** A new rule or board size for engine matches. */
  const watchSetting = (patch: Partial<Settings>) => {
    update(patch);
    watch.reset();
  };

  const shown = watching ? watch.state.snapshot : snapshot;
  const over = watching ? watch.state.status === "over" : phase === "over";
  const moves = shown?.moves ?? [];
  const size = shown?.size ?? settings.size;
  const toMove = sideToMove(moves);
  // Black's forbidden points, marked while Black is to move.
  const forbidden: Packed[] = [];
  if (shown && toMove === "black" && !over) {
    shown.threats.black.forEach((v, i) => {
      if (v === -2) forbidden.push((i % size) | (Math.floor(i / size) << 8));
    });
  }
  // Undo back to the person's previous turn.
  const undoCount = toMove === settings.human ? 2 : 1;
  const minimum = settings.human === "white" ? 1 : 0;
  const canUndo = (phase === "human" || phase === "over") && moves.length - undoCount >= minimum;

  const undo = () => {
    const client = engine.current;
    if (!client || !canUndo) return;
    generation.current++;
    setHint(null);
    client.undo(undoCount).then(setSnapshot, () => {});
  };

  const askHint = () => {
    const client = engine.current;
    if (!client || phase !== "human") return;
    const game = generation.current;
    searching.current = { color: settings.human, ply: moves.length };
    setPhase("hint");
    client
      .think(HINT_MS)
      .then((move) => {
        if (game !== generation.current) return;
        setHint(move);
        setPhase("human");
      })
      .catch(() => {});
  };

  const colorName = (c: StoneColor) => (c === "black" ? t.black : t.white);
  const engineColor: StoneColor = settings.human === "black" ? "white" : "black";

  let status: string;
  switch (phase) {
    case "loading":
      status = t.loading;
      break;
    case "engine":
      status = t.thinking;
      break;
    case "hint":
      status = t.hinting;
      break;
    case "over": {
      const result = snapshot?.result ?? 0;
      if (result === 3) status = t.draw;
      else status = (result === 1 ? "black" : "white") === settings.human ? t.youWin : t.engineWins;
      break;
    }
    default:
      status = t.yourMove;
  }

  const client = engine.current;
  const result = over ? (shown?.result ?? 0) : 0;
  const finished: StoneColor | null = result === 1 ? "black" : result === 2 ? "white" : null;
  let blackChance = info && client ? client.blackWinChance(info.info.value, info.color) : null;
  if (watching) {
    const { info: reports, thinking } = watch.state;
    const order: Side[] = thinking === "white" ? ["white", "black"] : ["black", "white"];
    blackChance = order.map((side) => reports[side]?.blackChance).find((c) => c !== undefined) ?? null;
    if (watch.state.status === "idle") blackChance = null;
    if (watch.state.end && watch.state.end.winner) blackChance = watch.state.end.winner === "black" ? 1 : 0;
  }
  if (finished) blackChance = finished === "black" ? 1 : 0;
  const mate = info && client && phase !== "over" ? client.mateIn(info.info.value) : null;
  const winner = mate === null ? null : mate > 0 ? info!.color : info!.color === "black" ? "white" : "black";

  // Drop the start of the expected line once it has been played.
  let pv = info?.info.pv ?? [];
  if (info) {
    const played = moves.slice(info.ply);
    const follows = played.length <= pv.length && played.every((m, i) => pv[i] === m);
    pv = follows ? pv.slice(played.length) : [];
  }
  const ghosts: Ghost[] = [];
  const thinking = watch.state.thinking;
  const next = thinking ? watch.state.info[thinking]?.pv?.[0] : undefined;
  if (watching) {
    if (thinking && next !== undefined && next >= 0) ghosts.push({ move: next, color: thinking });
  } else if (previewLine && phase !== "over") {
    pv.forEach((move, i) => {
      const color = (moves.length + i) % 2 === 0 ? "black" : "white";
      ghosts.push({ move, color, label: i + 1 });
    });
  } else if (phase === "engine" && pv.length > 0) {
    ghosts.push({ move: pv[0], color: engineColor });
  } else if (hint !== null && phase === "human") {
    ghosts.push({ move: hint, color: settings.human });
  }

  const name = (m: Packed) => {
    const { x, y } = unpack(m);
    return coordinate(x, y, size);
  };
  const lastMove =
    moves.length > 0 ? t.lastMove(colorName(sideToMove(moves.slice(0, -1))), name(moves[moves.length - 1])) : "";

  return (
    <div className="app" lang={settings.lang === "zh" ? "zh-Hant-TW" : "en"}>
      <header className="masthead">
        <a className="brand" href="https://github.com/SXKA/tengen" aria-label="Tengen on GitHub">
          <svg className="seal" viewBox="0 0 32 32" aria-hidden="true">
            <rect x="1.5" y="1.5" width="29" height="29" rx="3" />
            <path d="M5 16H27M16 5V27M5 9.5H27M5 22.5H27M9.5 5V27M22.5 5V27" />
            <circle cx="16" cy="16" r="3.4" />
          </svg>
          <span className="wordmark">Tengen</span>
        </a>
        <p className="tagline">{t.tagline}</p>
        <button className="link-button" onClick={() => update({ lang: settings.lang === "en" ? "zh" : "en" })}>
          {t.language}
        </button>
      </header>

      <main className="table">
        <div className="board-area">
          <WinBar black={blackChance} label={t.winChance} />
          <div className="board-frame">
            <Board
              size={size}
              moves={moves}
              five={shown?.five ?? []}
              human={settings.human}
              interactive={!watching && phase === "human"}
              ghosts={ghosts}
              forbidden={forbidden}
              threats={settings.threats && !over ? (shown?.threats ?? null) : null}
              numbers={settings.numbers}
              label={t.boardLabel(size)}
              onPlace={place}
              onPending={setTapPending}
            />
          </div>
        </div>

        <aside className="panel">
          <div className="modes" role="group" aria-label={t.modes}>
            {(["play", "watch"] as const).map((m) => (
              <button key={m} aria-pressed={settings.mode === m} onClick={() => switchMode(m)}>
                {m === "play" ? t.modePlay : t.modeWatch}
              </button>
            ))}
          </div>

          {watching ? (
            <WatchPanel
              t={t}
              state={watch.state}
              setup={{
                size: settings.size,
                rule: settings.rule,
                engines: settings.engines,
                ms: settings.ms,
                randomOpening: settings.randomOpening,
              }}
              onEngine={(side, id) => watchSetting({ engines: { ...settings.engines, [side]: id } })}
              onTime={(side, ms) => update({ ms: { ...settings.ms, [side]: ms } })}
              onStart={() =>
                watch.start({
                  size: settings.size,
                  rule: settings.rule,
                  engines: settings.engines,
                  ms: settings.ms,
                  randomOpening: settings.randomOpening,
                })
              }
              onPause={watch.pause}
              onResume={watch.resume}
              onStop={watch.reset}
              colorName={colorName}
              name={name}
            />
          ) : (
            <>
              <section className="status" aria-live="polite">
                <h1 className={phase === "engine" || phase === "hint" ? "status-text busy" : "status-text"}>
                  {status}
                </h1>
                <p className="status-note">
                  {phase === "human" && tapPending
                    ? t.tapAgain
                    : phase === "human" && moves.length === 0
                      ? t.firstMove
                      : phase === "human" && forbidden.length > 0
                        ? t.forbiddenNote(forbidden.length)
                        : hint !== null && phase === "human"
                          ? t.hintIs(name(hint))
                          : lastMove}
                </p>
              </section>

              <section className="readout" aria-label={t.winChance}>
                <p className="chance">
                  {finished
                    ? t.fiveBy(colorName(finished))
                    : result === 3
                      ? t.draw
                      : winner !== null && mate !== null
                        ? t.mateFor(colorName(winner), Math.abs(mate))
                        : blackChance !== null
                          ? t.blackShare(Math.round(blackChance * 100))
                          : " "}
                </p>
                <dl className="stats">
                  <div>
                    <dt>{t.depth}</dt>
                    <dd>{info ? `${info.info.depth}/${info.info.seldepth}` : "–"}</dd>
                  </div>
                  <div>
                    <dt>{t.threads}</dt>
                    <dd>{engine.current?.threads ?? 1}</dd>
                  </div>
                  <div>
                    <dt>{t.speed}</dt>
                    <dd>
                      {info ? formatRate((info.info.nodes / Math.max(info.info.elapsedMs, 1)) * 1000) : "–"}{" "}
                      <span className="unit">{t.perSecond}</span>
                    </dd>
                  </div>
                </dl>
                <div
                  className="line"
                  onPointerEnter={() => setPreviewLine(true)}
                  onPointerLeave={() => setPreviewLine(false)}
                  onFocus={() => setPreviewLine(true)}
                  onBlur={() => setPreviewLine(false)}
                  tabIndex={pv.length > 0 ? 0 : -1}
                  title={t.lineNote}
                >
                  <span className="line-label">{t.line}</span>
                  <span className="line-moves">{pv.length > 0 ? pv.slice(0, 12).map(name).join(" ") : "–"}</span>
                </div>
              </section>

              <section className="controls">
                <div className="actions">
                  <button className="primary" onClick={() => newGame()}>
                    {t.newGame}
                  </button>
                  <button onClick={undo} disabled={!canUndo}>
                    {t.undo}
                  </button>
                  <button onClick={askHint} disabled={phase !== "human"}>
                    {t.hint}
                  </button>
                </div>

                <fieldset className="choice">
                  <legend>{t.playAs}</legend>
                  {(["black", "white"] as const).map((c) => (
                    <label key={c} className={settings.human === c ? "selected" : ""}>
                      <input
                        type="radio"
                        name="human"
                        checked={settings.human === c}
                        onChange={() => newGame({ human: c })}
                      />
                      <span className={`swatch swatch-${c}`} aria-hidden="true" />
                      {colorName(c)}
                    </label>
                  ))}
                  <p className="note">{t.blackMovesFirst}</p>
                </fieldset>

                <fieldset className="choice">
                  <legend>{t.strength}</legend>
                  {t.levels.map((label, i) => (
                    <label key={label} className={settings.level === i ? "selected" : ""}>
                      <input
                        type="radio"
                        name="level"
                        checked={settings.level === i}
                        onChange={() => update({ level: i })}
                      />
                      {label}
                    </label>
                  ))}
                </fieldset>
              </section>
            </>
          )}

          <section className="controls">
            <fieldset className="choice rules">
              <legend>{t.rule}</legend>
              {RULES.map((r) => (
                <label key={r} className={settings.rule === r ? "selected" : ""}>
                  <input
                    type="radio"
                    name="rule"
                    checked={settings.rule === r}
                    onChange={() => (watching ? watchSetting({ rule: r }) : newGame({ rule: r }))}
                  />
                  {t.rules[r].name}
                </label>
              ))}
              <p className="note">{t.rules[settings.rule].note}</p>
            </fieldset>

            <fieldset className="choice">
              <legend>{t.board}</legend>
              {SIZES.map((s) => (
                <label key={s} className={settings.size === s ? "selected" : ""}>
                  <input
                    type="radio"
                    name="size"
                    checked={settings.size === s}
                    onChange={() => (watching ? watchSetting({ size: s }) : newGame({ size: s }))}
                  />
                  {s} × {s}
                </label>
              ))}
            </fieldset>

            <label className="toggle">
              <input
                type="checkbox"
                checked={settings.threats}
                onChange={(e) => update({ threats: e.target.checked })}
              />
              <span>
                {t.showThreats}
                <small>{t.showThreatsNote}</small>
              </span>
            </label>

            <label className="toggle">
              <input
                type="checkbox"
                checked={settings.numbers}
                onChange={(e) => update({ numbers: e.target.checked })}
              />
              <span>
                {t.showNumbers}
                <small>{t.showNumbersNote}</small>
              </span>
            </label>

            {watching && (
              <label className="toggle">
                <input
                  type="checkbox"
                  checked={settings.randomOpening}
                  onChange={(e) => update({ randomOpening: e.target.checked })}
                />
                <span>
                  {t.randomOpening}
                  <small>{t.randomOpeningNote}</small>
                </span>
              </label>
            )}
          </section>

          <section className="record">
            <h2>{t.moves}</h2>
            {moves.length === 0 ? (
              <p className="note">{t.noMoves}</p>
            ) : (
              <ol className="kifu">
                {moves.map((m, i) => (
                  <li key={i} className={i % 2 === 0 ? "by-black" : "by-white"}>
                    {name(m)}
                  </li>
                ))}
              </ol>
            )}
          </section>
        </aside>
      </main>

      <footer className="colophon">
        <p>
          {t.footer}
          {watching && <> {t.enginesCredit}</>}
        </p>
        <p>
          {watching && <a href="https://github.com/SXKA/tengen/tree/master/web/public/engines">{t.engineSources}</a>}
          <a href="https://github.com/SXKA/tengen">{t.source}</a>
          <a href="https://github.com/SXKA/tengen/releases/latest">{t.download}</a>
        </p>
      </footer>
    </div>
  );
}
