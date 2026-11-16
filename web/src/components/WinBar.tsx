interface Props {
  /** Black's chance to win, 0 to 1, or null before any search. */
  black: number | null;
  label: string;
}

/** Black's share of the win chance as a bar: Black fills from the bottom. */
export function WinBar({ black, label }: Props) {
  const share = black ?? 0.5;
  return (
    <div
      className="winbar"
      role="meter"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round(share * 100)}
    >
      <div className="winbar-black" style={{ flexGrow: share }} />
      <div className="winbar-white" style={{ flexGrow: 1 - share }} />
    </div>
  );
}
