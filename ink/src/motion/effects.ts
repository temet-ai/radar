// The effects motion.rs builds with tachyonfx, one class each, working on a Grid in place.
// Order inside `process` is tachyonfx's: the timer takes the elapsed time first, then the cells are changed at the
// timer's new alpha. An effect is done when its timer ran out (the frame that ran it out is still drawn).
import { RESET, lerp } from "./color.js";
import { Timer, circOut, type Ease } from "./ease.js";
import { copyCell, each, type Cell, type Grid, type Rect } from "./grid.js";

export interface Effect {
  process(dt: number, grid: Grid, area: Rect): void;
  done(): boolean;
}

/** What a colour the terminal decides reads as when blended: a cell with no colour is the canvas. */
export type Reads = { fg: number; bg: number };

const f = Math.fround;
/** tachyonfx's cache reads a Reset foreground as Color::White (192,192,192), which the fade blends from. */
export const WHITE = 0xc0c0c0;

/** `fx::fade_from_fg(to, (ms, ease))`: the text starts as `to` and comes to its own colour. */
export class FadeFromFg implements Effect {
  private timer: Timer;
  constructor(
    private to: number,
    ms: number,
    ease: Ease,
    private reads: Reads,
  ) {
    this.timer = new Timer(ms, ease).mirrored();
  }
  process(dt: number, grid: Grid, area: Rect) {
    this.timer.process(dt);
    const a = this.timer.alpha();
    each(grid, area, (cell) => {
      cell.fg = lerp(cell.fg < 0 ? WHITE : cell.fg, this.to, a);
    });
  }
  done = () => this.timer.done();
}

export type Way = "leftToRight" | "upToDown";

/** `fx::sweep_in(way, gradient, 0, faded, (ms, ease))` with no randomness: a soft edge moves across the area and
 * the cells behind it come out of `faded`. Only the two ways motion.rs uses (neither flips the timer). */
export class SweepIn implements Effect {
  private timer: Timer;
  constructor(
    private way: Way,
    private gradient: number,
    private faded: number,
    ms: number,
    ease: Ease,
  ) {
    this.timer = new Timer(ms, ease);
  }
  process(dt: number, grid: Grid, area: Rect) {
    this.timer.process(dt);
    const progress = this.timer.alpha();
    const horizontal = this.way === "leftToRight";
    const start0 = horizontal ? area.x : area.y;
    const len = horizontal ? area.w : area.h;
    const start = f(
      f(start0 - this.gradient) + f(f(len + this.gradient) * progress),
    );
    const end = f(start + this.gradient);
    const perCell = f(1 / (end - start));
    const faded = this.faded;
    each(grid, area, (cell, x, y) => {
      const p = horizontal ? x : y;
      const moved = p < start ? 0 : p > end ? 1 : f(perCell * f(p - start));
      const a = f(1 - moved); // tachyonfx's window alpha for these two ways is 1 - (the right-to-left / down-to-up one)
      if (a === 0) {
        cell.fg = faded;
        cell.bg = faded;
      } else if (a !== 1) {
        const m = circOut(a);
        const fg = cell.fg < 0 ? 0xc0c0c0 : cell.fg; // A Reset foreground reads as White, a Reset background as Black
        const bg = cell.bg < 0 ? 0 : cell.bg;
        cell.fg = lerp(faded, fg, m);
        cell.bg = lerp(faded, bg, m);
      }
    });
  }
  done = () => this.timer.done();
}

/** motion::glow: the row's background pulses toward the accent and fades back. */
export class Glow implements Effect {
  private timer = new Timer(450, "sineOut");
  constructor(
    private glow: number,
    private reads: Reads,
  ) {}
  process(dt: number, grid: Grid, area: Rect) {
    this.timer.process(dt);
    const k = f((1 - this.timer.alpha()) * 0.45);
    each(grid, area, (cell) => {
      cell.bg = lerp(cell.bg < 0 ? this.reads.bg : cell.bg, this.glow, k);
    });
  }
  done = () => this.timer.done();
}

/** motion::fade_out: the cells that were there come back, their text fading into `to`. */
export class FadeOut implements Effect {
  private timer = new Timer(200, "quadIn");
  constructor(
    private was: Cell[],
    private width: number,
    private to: number,
  ) {}
  process(dt: number, grid: Grid, area: Rect) {
    this.timer.process(dt);
    const a = this.timer.alpha();
    if (a >= 1) return;
    each(grid, area, (cell, x, y) => {
      const old = this.was[(y - area.y) * this.width + (x - area.x)];
      if (!old) return;
      Object.assign(cell, copyCell(old));
      cell.fg = lerp(old.fg, this.to, a);
    });
  }
  done = () => this.timer.done();
}

export { RESET };
