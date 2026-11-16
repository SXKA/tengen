import { describe, expect, it } from "vitest";
import { unpack } from "./engine/protocol";
import { coordinate, text } from "./i18n";

describe("coordinates", () => {
  it("names columns by letter and rows from the bottom", () => {
    expect(coordinate(7, 7, 15)).toBe("h8");
    expect(coordinate(0, 0, 15)).toBe("a15");
    expect(coordinate(19, 19, 20)).toBe("t1");
  });

  it("unpacks engine moves", () => {
    expect(unpack(7 | (12 << 8))).toEqual({ x: 7, y: 12 });
  });
});

describe("translations", () => {
  it("cover the same keys in both languages", () => {
    expect(Object.keys(text.zh).sort()).toEqual(Object.keys(text.en).sort());
    expect(text.zh.levels).toHaveLength(text.en.levels.length);
  });
});
