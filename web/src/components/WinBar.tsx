interface Props {
  /** Black's chance to win, 0 to 1, or null before any search. */
  black: number | null;
  label: string;
}

/** Black's share of the win chance as a bar, Black from the left; an empty track before any score. */
export function WinBar({ black, label }: Props) {
  if (black === null) return <div className="winbar" aria-hidden="true" />;
  return (
    <div
      className="winbar"
      role="meter"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round(black * 100)}
    >
      <div className="winbar-black" style={{ flexGrow: black }} />
      <div className="winbar-white" style={{ flexGrow: 1 - black }} />
    </div>
  );
}
