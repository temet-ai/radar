// The screen-assemble moment (crates/radar-core/src/motion/assemble.rs): on a screen switch at full, the new screen
// builds from its parts in the layout's order. Panels are found in the frame just drawn (their corners), left to
// right; each panel's frame draws down the screen row by row, and each row inside reads in from the left a moment
// after its frame row. Anything outside a panel is there at once. Same integer cell math as the Rust engine.
import { cellWidth } from "../chrome/width.js";
import { Timer } from "./ease.js";
import type { Effect } from "./effects.js";
import type { Grid, Rect } from "./grid.js";

/** From the key to the finished screen. */
export const ASSEMBLE_MS = 450;
const FRAME_START = 40, PANEL_GAP = 30, LAST_PANEL_START = 130;
const FRAME_ROW = 8, FRAME_SPAN = 200;
const ROW_DELAY = 40, ROW_MS = 60;

const TOP_LEFT = new Set(["╭", "┌", "┏"]), TOP_RIGHT = new Set(["╮", "┐", "┓"]);
const BOTTOM_LEFT = new Set(["╰", "└", "┗"]), BOTTOM_RIGHT = new Set(["╯", "┘", "┛"]);
const SIDE = new Set(["│", "┃", "├", "┣"]);
const inside = (p: Rect, x: number, y: number) => x >= p.x && x < p.x + p.w && y >= p.y && y < p.y + p.h;

/** The panels drawn inside `area`, by their corners, in the order they assemble (left to right, then down). */
export function panels(grid: Grid, area: Rect): Rect[] {
  const at = (x: number, y: number) => grid.at(x, y).ch;
  const found: Rect[] = [];
  for (let y = area.y; y < area.y + area.h; y++)
    for (let x = area.x; x < area.x + area.w; x++) {
      if (!TOP_LEFT.has(at(x, y)) || found.some((p) => inside(p, x, y))) continue;
      let right = -1;
      for (let r = x + 1; r < area.x + area.w; r++) if (TOP_RIGHT.has(at(r, y))) { right = r; break; }
      if (right < 0) continue;
      let bottom = -1;
      for (let b = y + 1; b < area.y + area.h; b++) {
        const s = at(x, b);
        if (BOTTOM_LEFT.has(s)) { bottom = b; break; }
        if (!SIDE.has(s)) break;
      }
      if (bottom >= 0 && BOTTOM_RIGHT.has(at(right, bottom))) found.push({ x, y, w: right - x + 1, h: bottom - y + 1 });
    }
  return found.sort((a, b) => a.x - b.x || a.y - b.y);
}

function shows(found: readonly Rect[], x: number, y: number, elapsed: number): boolean {
  const i = found.findIndex((p) => inside(p, x, y));
  if (i < 0) return true;
  const p = found[i]!;
  const start = FRAME_START + Math.min(i * PANEL_GAP, LAST_PANEL_START - FRAME_START);
  const step = Math.min(FRAME_ROW, Math.floor(FRAME_SPAN / Math.max(1, p.h - 1)));
  const framed = start + step * (y - p.y);
  if (y === p.y || y === p.y + p.h - 1 || x === p.x || x === p.x + p.w - 1) return elapsed >= framed;
  const from = framed + ROW_DELAY;
  if (elapsed < from) return false;
  const read = Math.ceil((p.w - 2) * Math.min(elapsed - from, ROW_MS) / ROW_MS);
  return x - p.x - 1 < read;
}

/** Hide what has not arrived yet: its cell keeps its ground and loses its glyph. A wide glyph the reading edge would
 * cut stays hidden until it is whole. */
export function applyAssemble(grid: Grid, area: Rect, elapsed: number) {
  if (elapsed >= ASSEMBLE_MS || area.w <= 0 || area.h <= 0) return;
  const found = panels(grid, area);
  for (let y = area.y; y < area.y + area.h; y++) {
    const visible = Array.from({ length: area.w }, (_, i) => shows(found, area.x + i, y, elapsed));
    visible.forEach((show, i) => {
      const cell = grid.at(area.x + i, y);
      const cut = cellWidth(cell.ch) > 1 && visible[i + 1] === false;
      if (!show || cut) { cell.ch = " "; cell.attr = ""; }
    });
  }
}

/** The moment as an engine effect, on its own linear clock. */
export class Assemble implements Effect {
  private timer = new Timer(ASSEMBLE_MS, "linear");
  process(dt: number, grid: Grid, area: Rect) {
    this.timer.process(dt);
    applyAssemble(grid, area, this.timer.total - this.timer.remaining);
  }
  done = () => this.timer.done();
}
