import { useRef, useState, type KeyboardEvent, type PointerEvent } from "react";
import { unpack, type Packed } from "../engine/protocol";
import { coordinate } from "../i18n";

/** Board cell size in SVG units; the grid starts one cell in from the edge. */
const C = 40;

export type StoneColor = "black" | "white";

export interface Ghost {
  move: Packed;
  color: StoneColor;
  /** Move number shown on the ghost stone, for previews of a line. */
  label?: number;
}

interface Props {
  size: number;
  moves: Packed[];
  five: Packed[];
  /** Color the person plays; used for the hover stone. */
  human: StoneColor;
  /** Whether clicks place stones right now. */
  interactive: boolean;
  ghosts: Ghost[];
  /** Cells the side to move may not play (renju, Black). */
  forbidden: Packed[];
  threats: { black: Int8Array; white: Int8Array } | null;
  label: string;
  onPlace: (move: Packed) => void;
  /** Told when a first tap marks a point that a second tap would place. */
  onPending?: (pending: boolean) => void;
}

function starPoints(size: number): [number, number][] {
  if (size < 9) return [];
  const edge = size >= 13 ? 3 : 2;
  const far = size - 1 - edge;
  const points: [number, number][] = [
    [edge, edge],
    [edge, far],
    [far, edge],
    [far, far],
  ];
  if (size % 2 === 1) points.push([(size - 1) / 2, (size - 1) / 2]);
  return points;
}

const at = (i: number) => C + i * C;

export function Board({
  size,
  moves,
  five,
  human,
  interactive,
  ghosts,
  forbidden,
  threats,
  label,
  onPlace,
  onPending,
}: Props) {
  const svg = useRef<SVGSVGElement>(null);
  const [hover, setHover] = useState<Packed | null>(null);
  // On touch screens the first tap only marks the point; a second tap places.
  const [pending, setPending] = useState<Packed | null>(null);
  const [cursor, setCursor] = useState<{ x: number; y: number } | null>(null);

  const occupied = new Map<Packed, number>();
  moves.forEach((m, i) => occupied.set(m, i));
  const banned = new Set(forbidden);
  const open = (move: Packed) => !occupied.has(move) && !banned.has(move);
  const last = moves.length > 0 ? moves[moves.length - 1] : null;
  const extent = (size + 1) * C;

  function cellAt(event: PointerEvent<SVGSVGElement>): Packed | null {
    const element = svg.current;
    const matrix = element?.getScreenCTM();
    if (!element || !matrix) return null;
    const point = new DOMPoint(event.clientX, event.clientY).matrixTransform(matrix.inverse());
    const x = Math.round((point.x - C) / C);
    const y = Math.round((point.y - C) / C);
    if (x < 0 || y < 0 || x >= size || y >= size) return null;
    return x | (y << 8);
  }

  function place(move: Packed) {
    if (!interactive || !open(move)) return;
    setPending(null);
    onPending?.(false);
    onPlace(move);
  }

  function onPointerUp(event: PointerEvent<SVGSVGElement>) {
    const move = cellAt(event);
    if (move === null || !interactive || !open(move)) return;
    if (event.pointerType === "touch" && pending !== move) {
      setPending(move);
      onPending?.(true);
      return;
    }
    place(move);
  }

  function onKeyDown(event: KeyboardEvent<SVGSVGElement>) {
    const current = cursor ?? { x: (size - 1) >> 1, y: (size - 1) >> 1 };
    const step: Record<string, [number, number]> = {
      ArrowLeft: [-1, 0],
      ArrowRight: [1, 0],
      ArrowUp: [0, -1],
      ArrowDown: [0, 1],
    };
    if (event.key in step) {
      event.preventDefault();
      const [dx, dy] = step[event.key];
      setCursor({
        x: Math.min(size - 1, Math.max(0, current.x + dx)),
        y: Math.min(size - 1, Math.max(0, current.y + dy)),
      });
    } else if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      place(current.x | (current.y << 8));
      setCursor(current);
    }
  }

  const lines = [];
  for (let i = 0; i < size; i++) {
    lines.push(`M${at(0)} ${at(i)}H${at(size - 1)}`, `M${at(i)} ${at(0)}V${at(size - 1)}`);
  }

  const preview = hover ?? pending;
  const showHover = interactive && preview !== null && open(preview) && ghosts.length === 0;
  // Sorting by column, then row, puts the two ends of any line first and last.
  const fiveCells = five.map(unpack).sort((a, b) => a.x - b.x || a.y - b.y);
  const fiveEnds = fiveCells.length >= 2 ? [fiveCells[0], fiveCells[fiveCells.length - 1]] : null;

  return (
    <svg
      ref={svg}
      className="board"
      viewBox={`0 0 ${extent} ${extent}`}
      role="img"
      aria-label={label}
      tabIndex={0}
      onPointerMove={(e) => e.pointerType !== "touch" && setHover(cellAt(e))}
      onPointerLeave={() => setHover(null)}
      onPointerUp={onPointerUp}
      onKeyDown={onKeyDown}
      onBlur={() => setCursor(null)}
      data-interactive={interactive}
    >
      <defs>
        <linearGradient id="kaya" x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="#e2bd79" />
          <stop offset="0.55" stopColor="#d9b06a" />
          <stop offset="1" stopColor="#c99c55" />
        </linearGradient>
        <filter id="grain" x="0" y="0" width="100%" height="100%">
          <feTurbulence type="fractalNoise" baseFrequency="0.0025 0.11" numOctaves="3" seed="11" />
          <feColorMatrix
            values="0 0 0 0 0.42
                    0 0 0 0 0.26
                    0 0 0 0 0.10
                    0 0 0 1.6 -0.62"
          />
        </filter>
        <radialGradient id="slate" cx="0.36" cy="0.32" r="0.75">
          <stop offset="0" stopColor="#6b6b73" />
          <stop offset="0.38" stopColor="#2c2c32" />
          <stop offset="1" stopColor="#101013" />
        </radialGradient>
        <radialGradient id="shell" cx="0.36" cy="0.32" r="0.8">
          <stop offset="0" stopColor="#ffffff" />
          <stop offset="0.55" stopColor="#efeadf" />
          <stop offset="1" stopColor="#c6bfb0" />
        </radialGradient>
        <filter id="shadow" x="-50%" y="-50%" width="200%" height="200%">
          <feGaussianBlur stdDeviation="2.2" />
        </filter>
      </defs>

      <rect className="board-wood" width={extent} height={extent} rx="6" fill="url(#kaya)" />
      <rect width={extent} height={extent} rx="6" filter="url(#grain)" opacity="0.55" />

      <path d={lines.join("")} className="grid" />
      <rect x={at(0)} y={at(0)} width={(size - 1) * C} height={(size - 1) * C} className="grid-edge" />
      {starPoints(size).map(([x, y]) => (
        <circle key={`${x},${y}`} cx={at(x)} cy={at(y)} r="4" className="star" />
      ))}

      {Array.from({ length: size }, (_, i) => (
        <g key={i} className="coord">
          <text x={at(i)} y={extent - C * 0.32} textAnchor="middle">
            {String.fromCharCode(97 + i)}
          </text>
          <text x={C * 0.38} y={at(i)} dominantBaseline="central" textAnchor="middle">
            {size - i}
          </text>
        </g>
      ))}

      {threats &&
        Array.from(threats.black, (b, i) => {
          const w = threats.white[i];
          if (b < 8 && w < 8) return null;
          const x = at(i % size);
          const y = at(Math.floor(i / size));
          return (
            <g key={i} className="threat">
              {b >= 8 && (
                <rect
                  x={x - 5}
                  y={y - 5}
                  width="10"
                  height="10"
                  transform={`rotate(45 ${x} ${y})`}
                  className="threat-black"
                />
              )}
              {w >= 8 && (
                <rect
                  x={x - 4.5 + (b >= 8 ? 8 : 0)}
                  y={y - 4.5 - (b >= 8 ? 8 : 0)}
                  width="9"
                  height="9"
                  transform={`rotate(45 ${x + (b >= 8 ? 8 : 0)} ${y - (b >= 8 ? 8 : 0)})`}
                  className="threat-white"
                />
              )}
            </g>
          );
        })}

      {forbidden.map((m) => {
        const { x, y } = unpack(m);
        const r = C * 0.16;
        return (
          <path
            key={`forbidden-${m}`}
            className="forbidden"
            d={`M${at(x) - r} ${at(y) - r}L${at(x) + r} ${at(y) + r}M${at(x) + r} ${at(y) - r}L${at(x) - r} ${at(y) + r}`}
          >
            <title>{coordinate(x, y, size)}</title>
          </path>
        );
      })}

      {moves.map((m, i) => {
        const { x, y } = unpack(m);
        const color = i % 2 === 0 ? "slate" : "shell";
        return (
          <g
            key={m}
            className={i === moves.length - 1 ? "stone stone-new" : "stone"}
            style={{ transformOrigin: `${at(x)}px ${at(y)}px` }}
          >
            <circle cx={at(x) + 1.6} cy={at(y) + 2.6} r={C * 0.45} className="stone-shadow" filter="url(#shadow)" />
            <circle cx={at(x)} cy={at(y)} r={C * 0.465} fill={`url(#${color})`} />
          </g>
        );
      })}

      {last !== null && <circle cx={at(unpack(last).x)} cy={at(unpack(last).y)} r={C * 0.11} className="last-move" />}

      {fiveEnds && (
        <line
          className="five-line"
          x1={at(fiveEnds[0].x)}
          y1={at(fiveEnds[0].y)}
          x2={at(fiveEnds[1].x)}
          y2={at(fiveEnds[1].y)}
          pathLength={1}
        />
      )}

      {ghosts
        .filter((g) => !occupied.has(g.move))
        .map((g) => {
          const { x, y } = unpack(g.move);
          return (
            <g key={`ghost-${g.move}`} className={`ghost ghost-${g.color}`}>
              <circle cx={at(x)} cy={at(y)} r={C * 0.465} fill={`url(#${g.color === "black" ? "slate" : "shell"})`} />
              {g.label !== undefined && (
                <text x={at(x)} y={at(y)} dominantBaseline="central" textAnchor="middle">
                  {g.label}
                </text>
              )}
            </g>
          );
        })}

      {showHover && preview !== null && (
        <circle
          cx={at(unpack(preview).x)}
          cy={at(unpack(preview).y)}
          r={C * 0.465}
          fill={`url(#${human === "black" ? "slate" : "shell"})`}
          className={pending === preview ? "hover-stone pending" : "hover-stone"}
        />
      )}

      {cursor && (
        <rect x={at(cursor.x) - C / 2} y={at(cursor.y) - C / 2} width={C} height={C} rx="4" className="cursor">
          <title>{coordinate(cursor.x, cursor.y, size)}</title>
        </rect>
      )}
    </svg>
  );
}
