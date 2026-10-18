// Exact physical-cell parity against the production Rust interaction compositor, including modifiers/continuations.
import fs from "node:fs";
import { describe, expect, it } from "vitest";
import { Grid } from "../../src/motion/grid.js";
import { parseHex } from "../../src/motion/color.js";
import { borderForegrounds, composeFold, focusDrain, focusLap, type Border } from "../../src/motion/interaction-effects.js";

type Cell = { ch: string; fg: string; bg: string; mods: string[] };
type Mark = { area: number[]; title: number[] | null; keep: number[][] };
type Case = { name: string; kind: "fold" | "focus_lap" | "focus_drain"; width: number; height: number; before: Cell[][]; after: Cell[][]; before_mark: Mark; after_mark: Mark; area?: number[]; descendants?: number; open?: boolean; stops?: string[]; frames: Array<{ ms: number; rows: Cell[][] }> };
const cases: Case[] = JSON.parse(fs.readFileSync(new URL("./fixtures/ratatui-interaction.json", import.meta.url), "utf8")).cases;
const rect = (r: number[]) => ({ x: r[0]!, y: r[1]!, w: r[2]!, h: r[3]! });
const border = (m: Mark): Border => ({ area: rect(m.area), title: m.title ? rect(m.title) : undefined, keep: m.keep.map(rect) });
const attrs: Record<string, number> = { bold: 1, dim: 2, italic: 3, underline: 4, underlined: 4, slow_blink: 5, rapid_blink: 6, reversed: 7, inverse: 7, hidden: 8, crossed_out: 9, strike: 9 };
const grid = (rows: Cell[][], width: number, height: number): Grid => {
  const out = new Grid(width, height);
  rows.forEach((r, y) => r.forEach((c, x) => Object.assign(out.at(x, y), { ch: c.ch, fg: parseHex(c.fg), bg: parseHex(c.bg), attr: c.mods.map((m) => attrs[m.toLowerCase()]).filter((n) => n !== undefined).sort((a, b) => a - b).join(";") })));
  return out;
};
describe("exact Rust/Ink fold and focus physical cells", () => {
  for (const f of cases) for (const frame of f.frames) it(`${f.name} at ${frame.ms} ms`, () => {
    const before = grid(f.before, f.width, f.height), current = grid(f.after, f.width, f.height);
    if (f.kind === "fold") {
      const area = rect(f.area!);
      composeFold(current, current.area, { area, descendants: f.descendants!, open: f.open!, expanded: (f.open ? current : before).copyOf(area), compact: (f.open ? before : current).copyOf(area) }, frame.ms);
    } else {
      const was = border(f.before_mark), now = border(f.after_mark);
      const prior = borderForegrounds(before, was)!;
      if (f.kind === "focus_lap") focusLap(current, was, now, prior, f.stops!.map(parseHex), frame.ms);
      else focusDrain(current, was, now, prior, frame.ms);
    }
    expect(current.cells).toEqual(grid(frame.rows, f.width, f.height).cells);
  });
});
