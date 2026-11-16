/** Interface text in English and Traditional Chinese. */

export type Lang = "en" | "zh";

const en = {
  tagline: "Gomoku and renju against an engine that runs in your browser.",
  yourMove: "Your move",
  tapAgain: "Tap again to place the stone",
  thinking: "Tengen is thinking",
  hinting: "Looking for a move for you",
  youWin: "You win",
  engineWins: "Tengen wins",
  draw: "Draw: the board is full",
  loading: "Loading the engine",
  firstMove: "Place your first stone. The center point is the usual start.",
  playAs: "Play as",
  black: "Black",
  white: "White",
  blackMovesFirst: "Black moves first.",
  strength: "Strength",
  levels: ["Easy", "Medium", "Hard", "Max"],
  board: "Board",
  rule: "Rules",
  rules: {
    freestyle: { name: "Freestyle", note: "Five or more in a row wins." },
    standard: { name: "Standard", note: "Exactly five wins; six or more does not." },
    renju: {
      name: "Renju",
      note: "Black may not make an overline, two fours or two open threes at once (marked ×). White wins with five or more.",
    },
    caro: {
      name: "Caro",
      note: "Exactly five wins, unless stones block both ends. The board edge does not block.",
    },
    "freestyle-caro": {
      name: "Free caro",
      note: "Six or more wins; five wins unless stones block both ends.",
    },
  },
  forbiddenNote: (n: number) =>
    `${n === 1 ? "The point" : `${n} points`} marked × ${n === 1 ? "is" : "are"} forbidden for Black.`,
  showThreats: "Mark fours",
  showThreatsNote: "Points where a stone makes four or five.",
  newGame: "New game",
  undo: "Undo",
  hint: "Hint",
  hintIs: (c: string) => `Suggested move: ${c}`,
  winChance: "Win chance",
  blackShare: (p: number) => `Black ${p}%`,
  mateFor: (color: string, n: number) => `${color} wins in ${n} ${n === 1 ? "move" : "moves"}`,
  fiveBy: (color: string) => `${color} made five in a row`,
  depth: "Depth",
  speed: "Speed",
  threads: "Threads",
  perSecond: "positions/s",
  line: "Expected line",
  lineNote: "Hover to show it on the board.",
  moves: "Moves",
  noMoves: "No moves yet.",
  footer: "Bitboard pattern tables, threat-space search and PVS, written in Rust and compiled to WebAssembly.",
  source: "Source code",
  download: "Gomocup brain",
  language: "中文",
  boardLabel: (size: number) => `${size} by ${size} gomoku board`,
  lastMove: (who: string, c: string) => `${who} played ${c}`,
};

const zh: typeof en = {
  tagline: "在瀏覽器裡和引擎下五子棋與連珠。",
  yourMove: "輪到你",
  tapAgain: "再點一次落子",
  thinking: "Tengen 思考中",
  hinting: "正在幫你找一手",
  youWin: "你贏了",
  engineWins: "Tengen 獲勝",
  draw: "和局：棋盤已滿",
  loading: "引擎載入中",
  firstMove: "下第一顆子，通常從天元開始。",
  playAs: "執子",
  black: "黑",
  white: "白",
  blackMovesFirst: "黑棋先行。",
  strength: "強度",
  levels: ["簡單", "普通", "困難", "全力"],
  board: "棋盤",
  rule: "規則",
  rules: {
    freestyle: { name: "自由", note: "連成五子或更多即勝。" },
    standard: { name: "標準", note: "剛好五子才勝，長連不算。" },
    renju: { name: "連珠", note: "黑棋不能長連、四四或三三（標 ×）；白棋五子以上即勝。" },
    caro: { name: "Caro", note: "剛好五子才勝，兩端都被棋子擋住則不算；棋盤邊緣不算擋。" },
    "freestyle-caro": { name: "自由 Caro", note: "六子以上即勝；五子兩端都被棋子擋住則不算。" },
  },
  forbiddenNote: (n: number) => `標 × 的 ${n} 個點是黑棋禁手。`,
  showThreats: "標出衝四點",
  showThreatsNote: "落子即成四或成五的位置。",
  newGame: "新對局",
  undo: "悔棋",
  hint: "提示",
  hintIs: (c: string) => `建議下在 ${c}`,
  winChance: "勝率",
  blackShare: (p: number) => `黑 ${p}%`,
  mateFor: (color: string, n: number) => `${color}棋 ${n} 手內必勝`,
  fiveBy: (color: string) => `${color}棋連成五子`,
  depth: "深度",
  speed: "速度",
  threads: "執行緒",
  perSecond: "局面/秒",
  line: "預想手順",
  lineNote: "游標移上來可在棋盤上顯示。",
  moves: "棋譜",
  noMoves: "還沒有落子。",
  footer: "位元棋盤棋型表、威脅空間搜尋與 PVS，以 Rust 撰寫並編譯成 WebAssembly。",
  source: "原始碼",
  download: "Gomocup 引擎",
  language: "English",
  boardLabel: (size: number) => `${size} 路五子棋盤`,
  lastMove: (who: string, c: string) => `${who}棋下在 ${c}`,
};

export const text = { en, zh };
export type Text = typeof en;

export function defaultLang(): Lang {
  return typeof navigator !== "undefined" && navigator.language.toLowerCase().startsWith("zh") ? "zh" : "en";
}

/** Board coordinate in the usual notation: column letter, row counted from the bottom. */
export function coordinate(x: number, y: number, size: number): string {
  return `${String.fromCharCode(97 + x)}${size - y}`;
}
