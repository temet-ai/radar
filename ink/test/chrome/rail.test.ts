import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { layoutRail, RAIL_HEIGHT } from "../../src/chrome/rail.js";
import { applyLook, bgOf, currentLook, defaultLook, slotStyle } from "../../src/text/look.js";
import type { RailStop } from "../../src/protocol/generated/RailStop.js";

/**
 * The step rail. The text is Ink's own golden (test/chrome/rail/rail_<w>.txt, frozen from the last recording of the kit's
 * rail: six steps in every state and one with a long name, drawn at the full width of 2 rows); the styles are read off rail.ts.
 */
const here = path.dirname(fileURLToPath(import.meta.url));
const goldens = path.resolve(here, "rail");
const reference = (w: number): string[] =>
  fs.readFileSync(path.join(goldens, `rail_${w}.txt`), "utf8").split("\n").filter((l, i) => i < 2 || l !== "").map((l) => l.trimEnd());

const stop = (label: string, state: RailStop["state"], result: string | null, i: number): RailStop => ({
  id: label,
  label,
  icon: "board",
  state,
  result,
  target: `chrome:setup:${i}`,
});
const STOPS: RailStop[] = [
  stop("look & terminal", "done", "neon · nerd icons", 0),
  stop("local code", "skipped", "skipped", 1),
  stop("Shelves", "current", "step 3 of 5", 2),
  stop("the Unseen University archive", "upcoming", null, 3),
  stop("Ledger", "upcoming", null, 4),
  stop("tour?", "upcoming", "asked at the end", 5),
];

let before = currentLook();
beforeEach(() => {
  before = currentLook();
  applyLook(defaultLook("off"));
});
afterEach(() => applyLook(before));

const text = (w: number) => layoutRail(STOPS, w).rows.map((r) => r.map((c) => c.text).join("").trimEnd());

describe("the step rail: the reference's text", () => {
  for (const w of [80, 160]) {
    it(`six stops across ${w} cells`, () => {
      expect(text(w)).toEqual(reference(w));
    });
  }

  it("takes two rows, each exactly as wide as asked, and a hit for every stop that has room for its words", () => {
    for (const w of [80, 160]) {
      const laid = layoutRail(STOPS, w);
      expect(laid.rows).toHaveLength(RAIL_HEIGHT);
      for (const row of laid.rows) expect([...row.map((c) => c.text).join("")].length).toBe(w);
      expect(laid.hits.map((h) => h.target)).toEqual(STOPS.map((s) => s.target));
      expect(laid.hits.every((h) => h.x + h.w <= w && h.h === RAIL_HEIGHT && h.y === 0)).toBe(true);
    }
  });

  it("cuts a label to its stop's room with an ellipsis and leaves a stop with no room out of the hits", () => {
    const laid = layoutRail(STOPS, 12);
    expect(laid.rows[0]!.map((c) => c.text).join("")).toHaveLength(12);
    expect(laid.hits.length).toBeLessThan(STOPS.length);
  });

  it("is empty for no stops", () => {
    expect(layoutRail([], 80).hits).toEqual([]);
  });
});

describe("the step rail: its styles", () => {
  const cellAt = (w: number, row: number, x: number) => {
    let at = 0;
    for (const c of layoutRail(STOPS, w).rows[row]!) {
      const n = [...c.text].length;
      if (x < at + n) return { ch: [...c.text][x - at]!, style: c.style ?? {} };
      at += n;
    }
    throw new Error("off the row");
  };

  it("the current stop is a square chip: bold ground-coloured words on the accent", () => {
    const row = text(160)[0]!;
    const x = [...row].indexOf("S");
    const s = cellAt(160, 0, x).style;
    expect(s.backgroundColor).toBe(slotStyle("accent.primary").color);
    expect(s.color).toBe(slotStyle("bg.base").color);
    expect(s.bold).toBe(true);
    // the chip's padding is on the accent too
    expect(cellAt(160, 0, x - 1).style.backgroundColor).toBe(slotStyle("accent.primary").color);
  });

  it("the track is lit up to the current stop and muted after it, on the canvas", () => {
    const row = [...text(160)[0]!];
    const lit = row.indexOf("─");
    expect(cellAt(160, 0, lit).style.color).toBe(slotStyle("status.success").color);
    const after = row.lastIndexOf("─");
    expect(cellAt(160, 0, after).style.color).toBe(slotStyle("text.muted").color);
    expect(cellAt(160, 0, after).style.backgroundColor).toBe(bgOf("bg.base"));
  });

  it("a done stop's mark is the success colour and its words primary; a skipped one's are muted", () => {
    const row = [...text(160)[0]!];
    const check = cellAt(160, 0, row.indexOf("✓"));
    expect(check.style.color).toBe(slotStyle("status.success").color);
    expect(cellAt(160, 0, row.indexOf("l")).style.color).toBe(slotStyle("text.primary").color);
    expect(cellAt(160, 0, row.indexOf("»")).style.color).toBe(slotStyle("text.muted").color);
  });

  it("a note is secondary under the current stop and muted under the others", () => {
    const row = [...text(160)[1]!];
    expect(cellAt(160, 1, row.join("").indexOf("step 3")).style.color).toBe(slotStyle("text.secondary").color);
    expect(cellAt(160, 1, row.indexOf("n")).style.color).toBe(slotStyle("text.muted").color);
  });
});
