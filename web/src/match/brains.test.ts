import { describe, expect, it } from "vitest";
import { PiskvorkBrain, TimeoutError, parseMessage, type Transport } from "./brains";
import { randomOpening } from "./match";

/** A brain that answers like a Piskvork engine would, and records what it got. */
function fakeBrain(answer: (command: string) => string[]) {
  const sent: string[] = [];
  const transport: Transport = {
    send(command) {
      sent.push(command);
      // Answer later, as a real engine would.
      setTimeout(() => answer(command).forEach((line) => transport.onLine(line)), 0);
    },
    onLine: () => {},
    onClose: () => {},
    close() {},
  };
  return { brain: new PiskvorkBrain("fake", transport), sent, transport };
}

describe("Piskvork brains", () => {
  it("start a game and send the position with BOARD", async () => {
    const { brain, sent } = fakeBrain((c) =>
      c.startsWith("START") ? ["OK"] : c.startsWith("BOARD") ? ["MESSAGE hi", "8,7"] : [],
    );
    await brain.start(15, "renju");
    expect(sent[0]).toBe("START 15");
    expect(sent).toContain("INFO rule 4");
    const move = await brain.think([7 | (7 << 8), 8 | (8 << 8)], 300);
    expect(move).toBe(8 | (7 << 8));
    expect(sent).toContain("INFO timeout_turn 300");
    // Black to move: Black's stone is the brain's own (1), White's the opponent's (2).
    expect(sent[sent.length - 1]).toBe("BOARD\n7,7,1\n8,8,2\nDONE");
  });

  it("open the game with BEGIN", async () => {
    const { brain, sent } = fakeBrain((c) => (c === "START 15" ? ["OK"] : c === "BEGIN" ? ["7,7"] : []));
    await brain.start(15, "freestyle");
    expect(await brain.think([], 100)).toBe(7 | (7 << 8));
    expect(sent[sent.length - 1]).toBe("BEGIN");
  });

  it("stop waiting when disposed", async () => {
    const { brain } = fakeBrain((c) => (c.startsWith("START") ? ["OK"] : []));
    await brain.start(15, "freestyle");
    const pending = brain.think([], 1);
    brain.dispose();
    await expect(pending).rejects.toThrow();
    expect(new TimeoutError("x")).toBeInstanceOf(Error);
  });

  it("report errors", async () => {
    const { brain } = fakeBrain((c) => (c.startsWith("START") ? ["ERROR unsupported size"] : []));
    await expect(brain.start(20, "freestyle")).rejects.toThrow(/unsupported size/);
  });
});

describe("messages", () => {
  it("read Rapfi's search reports", () => {
    const info = parseMessage("Speed 308K | Depth 5-7 | Eval -95 | Node 616 | Time 2ms | H10 J8", "white");
    expect(info.depth).toBe(5);
    expect(info.speed).toBe(308_000);
    expect(info.blackChance).toBeGreaterThan(0.5);
    expect(info.pv).toEqual([7 | (9 << 8), 9 | (7 << 8)]);
  });

  it("read mate scores in plies", () => {
    expect(parseMessage("Depth 9-12 | Eval +M5 | Time 3ms | H8", "black").mate).toBe(3);
    expect(parseMessage("Depth 9-12 | Eval -M4 | Time 3ms | H8", "black").blackChance).toBe(0);
  });

  it("read SlowRenju's reports, without its scores", () => {
    const info = parseMessage("Nodes=96556, Time=502ms, speed=192342, depth=7-13, Evaluation=-8", "black");
    expect(info.depth).toBe(7);
    expect(info.speed).toBe(192342);
    expect(info.blackChance).toBeUndefined();
  });

  it("keep other text as it is", () => {
    const info = parseMessage("Load config from /config.toml", "black");
    expect(info).toEqual({ message: "Load config from /config.toml" });
  });
});

describe("random openings", () => {
  it("place distinct stones near the center", () => {
    const moves = randomOpening(15, 3);
    expect(new Set(moves).size).toBe(3);
    for (const m of moves) {
      expect(Math.abs((m & 0xff) - 7)).toBeLessThanOrEqual(2);
      expect(Math.abs((m >> 8) - 7)).toBeLessThanOrEqual(2);
    }
  });
});
