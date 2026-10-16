import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3";
});
import React from "react";
import { describe, expect, it } from "vitest";
import {
  DiffView,
  blendHex,
  changedWords,
  splitRows,
} from "../../src/text/DiffView.js";
import { styledFrameAt } from "../nodes/helpers.js";
import { parseAnsi } from "../nodes/ansi.js";
import type { DiffLine } from "../../src/nodes/view.js";

// ui::diff: tints are the diff slots blended into bg.surface (0.16; 0.42 for the words that changed), the cursor line is
// lit across the row, hunks are status.info, numbers muted.
const lines: DiffLine[] = [
  { kind: "Hunk", old: null, new: null, text: "@@ -1,3 +1,3 @@" },
  { kind: "Context", old: 1, new: 1, text: "keep" },
  { kind: "Removed", old: 2, new: null, text: "let total = old_value + 1;" },
  { kind: "Added", old: null, new: 2, text: "let total = new_value + 1;" },
];
const draw = (
  over: Partial<React.ComponentProps<typeof DiffView>> = {},
  width = 60,
) =>
  parseAnsi(
    styledFrameAt(
      <DiffView
        lines={lines}
        under={[]}
        selected={1}
        top={0}
        split={false}
        empty=""
        width={width}
        focused
        height={10}
        {...over}
      />,
      width,
    ),
  );

describe("the diff's look", () => {
  it("blends a diff slot into bg.surface like widgets::blend", () => {
    expect(blendHex("#50fa7b", "#12101a", 0.16)).toBe("#1c352a"); // 0x12+(0x50-0x12)*.16, … (widgets::blend)
    expect(blendHex("ansi256(10)", "#12101a", 0.16)).toBeUndefined();
  });
  it("puts added and removed lines on their tint after the numbers and leaves context alone", () => {
    const rows = draw();
    const removed = rows[2]!;
    const added = rows[3]!;
    expect(removed[10]!.bg).toBe(blendHex("#ff6363", "#12101a", 0.16));
    expect(added[10]!.bg).toBe(blendHex("#50fa7b", "#12101a", 0.16));
    expect(
      rows[1]!.some((c) => c.bg === blendHex("#50fa7b", "#12101a", 0.16)),
    ).toBe(false);
    expect(rows[0]![1]).toMatchObject({ ch: "@", fg: "#82aaff" }); // hunk: status.info
    expect(rows[2]![1]).toMatchObject({ ch: "2", fg: "#6272a4" }); // numbers: text.muted
  });
  it("marks the cursor line with ▎ in accent.primary and lights it across the row", () => {
    const row = draw()[1]!;
    expect(row[0]).toMatchObject({ ch: "▎", fg: "#e135ff" });
    expect(row.slice(1).every((c) => c.bg === "#44475a")).toBe(true);
  });
  it("emphasises the words the host says changed in a replaced pair on the stronger tint, bold", () => {
    const { o, n } = changedWords(
      "let total = old_value + 1;",
      "let total = new_value + 1;",
    );
    expect(o).toEqual([[12, 21]]);
    expect(n).toEqual([[12, 21]]);
    const rows = draw({ words: [[], [], o, n] });
    const col = rows[2]!.map((c) => c.ch).join("").indexOf("old_value");
    expect(rows[2]![col]).toMatchObject({
      bg: blendHex("#ff6363", "#12101a", 0.42),
      bold: true,
    });
    expect(rows[2]![5]!.bold).toBeUndefined();
    // Without the host's ranges nothing is compared here, so nothing is marked.
    const plain = draw();
    expect(plain[2]![col]!.bold).toBeUndefined();
  });
  it("shares too little: no word marks", () => {
    expect(changedWords("abc def", "xyz uvw")).toEqual({ o: [], n: [] });
  });
  it("splits a removed run against the added run after it, a rule between the sides", () => {
    expect(splitRows(lines)).toEqual([
      [0, 0],
      [1, 1],
      [2, 3],
    ]);
    const row = draw({ split: true }, 80)[2]!
      .map((c) => c.ch)
      .join("");
    expect(row).toMatch(/- let total = old_value.*│.*\+ let total = new_value/);
  });
});
