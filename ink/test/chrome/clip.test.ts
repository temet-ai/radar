import { describe, expect, it } from "vitest";
import { clipSegments } from "../../src/chrome/width.js";

const text = (segs: Array<{ text: string }>) => segs.map((s) => s.text).join("");

describe("clipSegments cuts at the column, as the reference's buffer does (no ellipsis)", () => {
  it("cuts a nav label that is wider than its column", () => {
    const row = [{ text: "  " }, { text: "└─ " }, { text: "Tools and defaults" }];
    expect(text(clipSegments(row, 20))).toBe("  └─ Tools and defau");
  });

  it("keeps segments that fit, and their styles", () => {
    const row = [{ text: "ab", style: 1 }, { text: "cd", style: 2 }];
    expect(clipSegments(row, 4)).toEqual(row);
  });

  it("drops the segments past the edge", () => {
    expect(clipSegments([{ text: "abc" }, { text: "def" }], 3)).toEqual([{ text: "abc" }]);
  });

  it("leaves a half-fitting wide character out", () => {
    expect(text(clipSegments([{ text: "a世界" }], 4))).toBe("a世");
  });
});
