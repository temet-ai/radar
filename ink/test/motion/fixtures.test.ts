// Ink's motion engine against T31's Ratatui fixtures (ink/test/fixtures/motion/*.json, written by
// tests/motion_cells.rs): real shell frames at 80x24, the cue as the bridge sends it, the marks the shell made, and
// the cells Ratatui's Engine+ManualClock produced at the start, middle and end of each effect. Each file is replayed
// the way the Rust side ran it: the cue, then the same frame drawn at t=0 and at every moment up to the file's own;
// every cell must equal Ratatui's (within one colour level), the frame after the end must be `after` exactly, and the
// engine must then ask for no more frames.
import { describe, expect, it } from "vitest";
import fs from "node:fs";
import { cueOf } from "../../src/motion/attach.js";
import {
  Engine,
  type InkColours,
  type Level,
  type Marks,
} from "../../src/motion/engine.js";
import { parseHex } from "../../src/motion/color.js";
import { Grid } from "../../src/motion/grid.js";
import { defaultLook } from "../../src/text/look.js";
import { charWidth } from "../../src/chrome/width.js";

const dir = new URL("../fixtures/motion/", import.meta.url);
type Run = [string, string, string, string];
type Fixture = {
  effect: string;
  level: Level;
  moment: string;
  t_ms: number;
  duration_ms: number;
  cue: any;
  motion: { level: Level; slots: Record<keyof InkColours, string> };
  marks: {
    body: number[] | null;
    popup: number[] | null;
    message: number[] | null;
    rows: Record<string, number[]>;
    loading: number[][];
  };
  before: Run[][];
  after: Run[][];
  settle: Run[][];
  settle_follows: boolean;
  rows: Run[][];
};
const load = (name: string): Fixture =>
  JSON.parse(fs.readFileSync(new URL(name, dir), "utf8"));
const files = fs
  .readdirSync(dir)
  .filter((f) => f.endsWith(".json"))
  .sort();

const colour = (s: string): number =>
  s === "reset" ? -1 : s === "white" ? 0xc0c0c0 : parseHex(s);
const rect = (r: number[]) => ({ x: r[0]!, y: r[1]!, w: r[2]!, h: r[3]! });

function gridOf(rows: Run[][]): Grid {
  const g = new Grid(80, 24);
  rows.forEach((runs, y) => {
    let x = 0;
    for (const [text, fg, bg, mods] of runs) {
      for (const ch of text) {
        const w = Math.max(1, charWidth(ch.codePointAt(0)!));
        for (let k = 0; k < w; k++)
          Object.assign(g.at(x + k, y), {
            ch: k === 0 ? ch : "",
            fg: colour(fg),
            bg: colour(bg),
            attr: [mods.includes("bold") ? "1" : "", mods.includes("dim") ? "2" : "", mods.includes("italic") ? "3" : "", mods.includes("underline") ? "4" : "", mods.includes("reversed") ? "7" : "", mods.includes("crossed") ? "9" : ""].filter(Boolean).join(";"),
          });
        x += w;
      }
    }
  });
  return g;
}
function marksOf(f: Fixture): Marks {
  const m = f.marks;
  return {
    body: m.body ? rect(m.body) : undefined,
    popup: m.popup ? rect(m.popup) : undefined,
    message: m.message ? rect(m.message) : undefined,
    rows: Object.entries(m.rows).map(([k, r]) => [k, rect(r)] as const),
    loading: m.loading.map(rect),
  };
}
const inkOf = (f: Fixture): InkColours => {
  const slots = defaultLook().slots;
  const s = f.motion.slots;
  return {
    base: parseHex(slots[s.base]),
    surface: parseHex(slots[s.surface]),
    overlay: parseHex(slots[s.overlay]),
    glow: parseHex(slots[s.glow]),
    text: parseHex(slots[s.text]),
  };
};

const TOL = 1;
function same(got: Grid, want: Grid, where: string) {
  const bad: string[] = [];
  got.cells.forEach((c, i) => {
    const w = want.cells[i]!;
    const at = `${i % 80},${Math.floor(i / 80)}`;
    if (c.attr !== w.attr) bad.push(`${at} modifiers ${c.attr} vs ${w.attr}`);
    if (c.ch !== w.ch) bad.push(`${at} symbol ${c.ch} vs ${w.ch}`);
    for (const k of ["fg", "bg"] as const) {
      const off =
        c[k] < 0 || w[k] < 0
          ? c[k] !== w[k]
          : [16, 8, 0].some(
              (sh) =>
                Math.abs(((c[k] >> sh) & 255) - ((w[k] >> sh) & 255)) > TOL,
            );
      if (off)
        bad.push(`${at} ${k} ${c[k].toString(16)} vs ${w[k].toString(16)}`);
    }
  });
  expect(bad.slice(0, 10), where).toEqual([]);
}

describe("Ink's motion engine against T31's Ratatui fixtures", () => {
  it("there are fixtures", () => expect(files.length).toBeGreaterThan(20));

  for (const file of files) {
    const f = load(file);
    it(`${file}: the cells at t=${f.t_ms} ms, the plain frame after the end, then no more frames`, () => {
      const ink = inkOf(f);
      let now = 0;
      const e = new Engine(f.motion.level, () => now);
      // A message that leaves is remembered from the frame it was drawn in.
      if (f.effect === "message_gone")
        e.draw(
          gridOf(f.before),
          { message: rect(load("message-full-start.json").marks.message!) },
          ink,
        );
      const cue = f.cue ? cueOf(f.cue) : undefined;
      if (cue) e.cue(cue, ink);
      const marks = marksOf(f);
      const times = f.effect === "off" ? [0] : [...new Set([0, f.t_ms])];
      let last = gridOf(f.after);
      for (const t of times) {
        now = t;
        last = gridOf(f.after);
        e.draw(last, marks, ink);
      }
      same(last, gridOf(f.rows), file);
      if (f.moment !== "end" || f.effect === "loading") return; // a waiting panel shimmers on
      // The effect is over: Ratatui draws one plain frame next (`settle` == `after`), then asks for nothing.
      expect(e.running(), "a plain frame is still to come").toBe(
        f.settle_follows,
      );
      now += 30;
      const plain = gridOf(f.after);
      const changed = e.draw(plain, marks, ink);
      expect(changed, "the plain frame touches no cell").toEqual([]);
      same(plain, gridOf(f.settle), `${file} settle`);
      same(plain, gridOf(f.after), `${file} after`);
      expect(e.running(), "and then nothing runs").toBe(false);
      expect(e.nextFrame()).toBeUndefined();
    });
  }
});
