// The cells an effect works on: one per terminal cell, with a colour pair and the rest of the style kept as it was.
import { RESET } from "./color.js";

export type Rect = { x: number; y: number; w: number; h: number };

export type Cell = {
  /** The glyph; "" is the second half of a wide one. */
  ch: string;
  fg: number;
  bg: number;
  /** Every other SGR parameter of the cell (bold, underline, a colour that is not truecolor), unchanged by effects. */
  attr: string;
};

export const blank = (): Cell => ({ ch: " ", fg: RESET, bg: RESET, attr: "" });
export const copyCell = (c: Cell): Cell => ({
  ch: c.ch,
  fg: c.fg,
  bg: c.bg,
  attr: c.attr,
});

export class Grid {
  readonly cells: Cell[];
  constructor(
    readonly width: number,
    readonly height: number,
  ) {
    // a plain loop: Array.from({ length }, fn) was most of what reading a frame cost (motion/frame.ts parseFrame)
    const cells: Cell[] = [];
    for (let i = width * height; i > 0; i--) cells.push(blank());
    this.cells = cells;
  }
  at(x: number, y: number): Cell {
    return this.cells[y * this.width + x]!;
  }
  get area(): Rect {
    return { x: 0, y: 0, w: this.width, h: this.height };
  }
  /** A copy of the cells of `r`, row by row. */
  copyOf(r: Rect): Cell[] {
    const out: Cell[] = [];
    for (let y = r.y; y < r.y + r.h; y++)
      for (let x = r.x; x < r.x + r.w; x++) out.push(copyCell(this.at(x, y)));
    return out;
  }
}

export function intersection(a: Rect, b: Rect): Rect {
  const x = Math.max(a.x, b.x);
  const y = Math.max(a.y, b.y);
  const r = Math.min(a.x + a.w, b.x + b.w);
  const bot = Math.min(a.y + a.h, b.y + b.h);
  return r > x && bot > y
    ? { x, y, w: r - x, h: bot - y }
    : { x, y, w: 0, h: 0 };
}

export const isEmpty = (r: Rect): boolean => r.w <= 0 || r.h <= 0;
export const sameRect = (a: Rect, b: Rect): boolean =>
  a.x === b.x && a.y === b.y && a.w === b.w && a.h === b.h;

/** Visit the cells of `r` (already inside the grid). */
export function each(
  grid: Grid,
  r: Rect,
  fn: (cell: Cell, x: number, y: number) => void,
) {
  for (let y = r.y; y < r.y + r.h; y++)
    for (let x = r.x; x < r.x + r.w; x++) fn(grid.at(x, y), x, y);
}
