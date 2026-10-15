import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3";
});
import React from "react";
import { describe, expect, it } from "vitest";
import { TabRow } from "../../src/app/TabRow.js";
import { styledFrameAt } from "../nodes/helpers.js";
import { parseAnsi } from "../nodes/ansi.js";
import { paintHover } from "../../src/app/hover.js";

// Live survey (T22): Ratatui's page tab row sits on bg.base (#0a0812) from its leading space to the end of the row, the
// showing title (bold accent) and the inactive titles included, until the row has the keys (the showing title is then
// on the accent). Ink left the leading space, the showing title and the earlier titles without one.
describe("the page tab row", () => {
  const BASE = "#0a0812";
  const row = (on: number) =>
    parseAnsi(styledFrameAt(<TabRow labels={["Overview", "Changes", "Threads"]} on={on} focused={false} width={40} base={BASE} />, 42))[0]!;
  it.each([0, 1, 2])("puts every cell of the row on bg.base with tab %i showing", (on) => {
    expect(row(on).slice(0, 41).every((c) => c.bg === BASE)).toBe(true);
  });
  it("keeps the showing title bold in the accent", () => {
    const cells = row(1);
    const at = cells.map((c) => c.ch).join("").indexOf("[Changes]");
    expect([cells[at + 1]!.bold, cells[at + 1]!.fg]).toEqual([true, "#e135ff"]);
  });
});

describe("the page tab under the pointer", () => {
  const BASE = "#0a0812";
  it("is lit with bg.hover over its hit (the two cells before the title and the title), the others stay on bg.base", () => {
    const hits: Array<{ x: number; y: number; w: number; h: number; target: string }> = [];
    const frame = styledFrameAt(<TabRow labels={["Overview", "Changes", "Threads"]} on={0} focused={false} width={40} base={BASE} registry={{ add: (h) => void hits.push(h) }} />, 42);
    const hit = hits.find((h) => h.target === "tab:1")!;
    const cells = parseAnsi(paintHover(frame, 42, hit))[0]!;
    const text = cells.map((c) => c.ch).join("");
    const at = text.indexOf("Changes");
    for (let i = at - 2; i < at + 7; i++) expect(cells[i]!.bg, `cell ${i}`).toBe("#2a263e");
    expect(cells[text.indexOf("Threads") + 2]!.bg).toBe(BASE);
    expect(cells[text.indexOf("Overview")]!.bg).toBe(BASE);
  });
});
