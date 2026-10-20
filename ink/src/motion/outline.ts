// Loading outlines (crates/radar-core/src/motion/outline.rs): while a panel waits for its first rows, its blank rows
// show the shape the rows will have (an icon slot, a name, a state, an age), and a light wave runs down them.
import { lerp } from "./color.js";
import { Timer } from "./ease.js";
import type { Effect } from "./effects.js";
import { each, type Grid, type Rect } from "./grid.js";

const SHAPES: ReadonlyArray<readonly [number, number, number]> = [[6, 0, 0], [12, 8, 2], [16, 10, 3], [11, 8, 2], [9, 8, 2]];
/** One pass of the wave down the panel's rows: each row lights and dims in turn. */
export const WAVE_MS = 1400;
const WAVE_RISE_MS = 170, WAVE_FALL_MS = 250;
/** How far an outline sits from the panel's ground toward its text, at rest and at the wave's peak. */
export const OUTLINE_DIM = 0.12, OUTLINE_LIGHT = 0.45;
const f = Math.fround;

/** The glyph at each cell of outline row `row` in a panel `width` cells wide; undefined leaves the cell blank. */
export function outlineRow(row: number, width: number): Array<string | undefined> {
  const out: Array<string | undefined> = new Array(Math.max(0, width)).fill(undefined);
  if (width < 8) return out;
  const [name, state, age] = SHAPES[row % SHAPES.length]!, last = width - 1;
  const bar = (from: number, len: number) => { for (let x = from; x < Math.min(from + len, last); x++) out[x] = "━"; };
  out[1] = "●";
  bar(3, name);
  if (state > 0) {
    const at = Math.max(3 + name + 2, Math.floor(width * 5 / 8));
    bar(at, state);
    bar(at + state + 2, age);
  }
  return out;
}

/** How lit row `row` of `rows` is, `elapsed` ms into a pass of the wave: 0 at rest, 1 at the peak. */
export function wave(elapsed: number, row: number, rows: number): number {
  const lag = Math.floor(row * (WAVE_MS - WAVE_RISE_MS - WAVE_FALL_MS) / Math.max(1, rows)), p = ((elapsed % WAVE_MS) + WAVE_MS - lag) % WAVE_MS;
  if (p < WAVE_RISE_MS) return f(p / WAVE_RISE_MS);
  if (p < WAVE_RISE_MS + WAVE_FALL_MS) return f(1 - f((p - WAVE_RISE_MS) / WAVE_FALL_MS));
  return 0;
}

/** A waiting panel's blank rows as outlines of the rows to come, with the wave running down them, for as long as it waits. */
export class Outline implements Effect {
  private timer = new Timer(WAVE_MS, "linear");
  private dim: number;
  private light: number;
  /** A colour the terminal decides reads as black here, as tachyonfx reads it on the Rust side. */
  constructor(surface: number, text: number) {
    this.dim = lerp(surface, text, OUTLINE_DIM);
    this.light = lerp(surface, text, OUTLINE_LIGHT);
  }
  process(dt: number, grid: Grid, area: Rect) {
    const over = this.timer.process(dt);
    const elapsed = this.timer.total - this.timer.remaining;
    for (let r = 0; r < area.h; r++) {
      const y = area.y + r;
      let blank = true;
      each(grid, { x: area.x, y, w: area.w, h: 1 }, (cell) => { if (cell.ch !== " ") blank = false; });
      if (!blank) continue;
      const colour = lerp(this.dim, this.light, wave(elapsed, r, area.h));
      outlineRow(r, area.w).forEach((glyph, i) => {
        if (glyph === undefined) return;
        const cell = grid.at(area.x + i, y);
        cell.ch = glyph;
        cell.fg = colour;
      });
    }
    if (over !== undefined) this.timer.reset(); // repeats for as long as the panel waits
  }
  done = () => false;
}
