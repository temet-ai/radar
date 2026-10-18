// Integer cell composition shared with motion/interaction.rs. No clock, retained state or view inference here.
import { cellWidth } from "../chrome/width.js";
import { perimeterIndex, perimeterLength } from "../text/frame.js";
import { copyCell, sameRect, type Cell, type Grid, type Rect } from "./grid.js";

export const FOLD_MS = 160;
export const FOCUS_MS = 300;
export const MAX_VIEWPORT_CELLS = 65_536;
export const MAX_BASELINE_ANSI = 1_048_576;
export type Border = { area: Rect; title?: Rect; keep: readonly Rect[] };
export type FoldRows = { area: Rect; expanded: readonly Cell[]; compact: readonly Cell[]; descendants: number; open: boolean };
export const contained = (a: Rect, b: Rect): boolean => a.x >= b.x && a.y >= b.y && a.x + a.w <= b.x + b.w && a.y + a.h <= b.y + b.h;
export const bounded = (r: Rect): boolean => [r.x, r.y, r.w, r.h, r.x + r.w, r.y + r.h].every(Number.isSafeInteger) && r.x >= 0 && r.y >= 0 && r.w > 0 && r.h > 0 && r.w * r.h <= MAX_VIEWPORT_CELLS;
const inside = (r: Rect, x: number, y: number) => x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h;
export const protectedCell = (b: Border, x: number, y: number) => !!b.title && inside(b.title, x, y) || b.keep.some((r) => inside(r, x, y));
export const sameBorder = (a: Border, b: Border): boolean => sameRect(a.area, b.area) && (a.title ? !!b.title && sameRect(a.title, b.title) : !b.title) && a.keep.length === b.keep.length && a.keep.every((r, i) => sameRect(r, b.keep[i]!));

/** Physical clockwise positions, including protected cells. Corners occur exactly once. */
export function perimeter(r: Rect): Array<readonly [number, number]> {
  const out: Array<readonly [number, number]> = new Array(perimeterLength(r.w, r.h));
  const put = (x: number, y: number) => { const i = perimeterIndex(r.w, r.h, x, y); if (i !== undefined) out[i] = [r.x + x, r.y + y]; };
  for (let x = 0; x < r.w; x++) { put(x, 0); if (r.h > 1) put(x, r.h - 1); }
  for (let y = 1; y < r.h - 1; y++) { put(0, y); if (r.w > 1) put(r.w - 1, y); }
  return out;
}
export function lapOrigin(border: Border): number | undefined {
  if (!bounded(border.area)) return undefined;
  const points = perimeter(border.area);
  let start = 0;
  points.forEach(([x, y], i) => { if (y === border.area.y && border.title && inside(border.title, x, y)) start = (i + 1) % points.length; });
  for (let d = 0; d < points.length; d++) {
    const i = (start + d) % points.length;
    const [x, y] = points[i]!;
    if (!protectedCell(border, x, y)) return i;
  }
  return undefined;
}
export const foldVisible = (descendants: number, ms: number, open: boolean): number => {
  const stepped = Math.floor(descendants * Math.min(FOLD_MS, Math.max(0, ms)) / FOLD_MS);
  return open ? stepped : descendants - stepped;
};
const wholeRows = (cells: readonly Cell[], width: number): boolean => cells.every((c, i) =>
  cellWidth(c.ch) <= width - i % width && (i % width !== 0 || c.ch !== ""));
export function composeFold(grid: Grid, viewport: Rect, rows: FoldRows, elapsed: number): boolean {
  const { area, expanded, compact } = rows;
  if (elapsed >= FOLD_MS || !bounded(viewport) || !contained(viewport, grid.area) || !bounded(area) || !contained(area, viewport)) return false;
  const count = area.w * area.h;
  const n = Math.min(rows.descendants, area.h);
  if (n <= 0 || expanded.length !== count || compact.length !== count || !wholeRows(expanded, area.w) || !wholeRows(compact, area.w)) return false;
  const visible = foldVisible(n, elapsed, rows.open);
  for (let y = 0; y < area.h; y++) {
    const source = y < visible ? expanded : compact;
    const row = y < visible ? y : y - visible;
    for (let x = 0; x < area.w; x++) Object.assign(grid.at(area.x + x, area.y + y), copyCell(source[row * area.w + x]!));
  }
  return true;
}
export const mixColour = (a: number, b: number, q: number): number => {
  let value = 0;
  for (const shift of [16, 8, 0]) value |= Math.floor((((a >>> shift) & 255) * (1000 - q) + ((b >>> shift) & 255) * q + 500) / 1000) << shift;
  return value;
};
export const lapWidth = (n: number) => Math.min(n, Math.max(7, Math.ceil(n / 8)));
export const lapHead = (n: number, elapsed: number) => Math.floor(n * Math.min(FOCUS_MS, Math.max(0, elapsed)) / FOCUS_MS);
export function bandColour(stops: readonly number[], age: number, width: number): number {
  const phase = width <= 1 ? 0 : Math.floor(Math.min(age, width - 1) * 6000 / (width - 1));
  if (phase === 6000) return stops[6]!;
  const index = Math.floor(phase / 1000);
  return mixColour(stops[index]!, stops[index + 1]!, phase % 1000);
}
export function borderForegrounds(grid: Grid, border: Border): number[] | undefined {
  if (!bounded(border.area) || !contained(border.area, grid.area)) return undefined;
  return perimeter(border.area).map(([x, y]) => grid.at(x, y).fg);
}
const matches = (grid: Grid, before: Border, now: Border, prior: readonly number[]) => sameBorder(before, now) && bounded(now.area) && contained(now.area, grid.area) && prior.length === perimeterLength(now.area.w, now.area.h);
export function focusLap(grid: Grid, before: Border, now: Border, quiet: readonly number[], stops: readonly number[], elapsed: number): boolean {
  if (elapsed >= FOCUS_MS || !matches(grid, before, now, quiet) || stops.length !== 7 || stops.some((n) => n < 0)) return false;
  const origin = lapOrigin(now);
  if (origin === undefined) return false;
  const points = perimeter(now.area);
  if (points.some(([x, y], i) => !protectedCell(now, x, y) && quiet[i]! < 0)) return false;
  const n = points.length, head = lapHead(n, elapsed), width = lapWidth(n);
  points.forEach(([x, y], i) => {
    if (protectedCell(now, x, y)) return;
    const distance = (i + n - origin) % n;
    if (distance > head) grid.at(x, y).fg = quiet[i]!;
    else if (head - distance < width) grid.at(x, y).fg = bandColour(stops, head - distance, width);
  });
  return true;
}
export function focusDrain(grid: Grid, before: Border, now: Border, prior: readonly number[], elapsed: number): boolean {
  if (elapsed >= FOCUS_MS || !matches(grid, before, now, prior)) return false;
  const points = perimeter(now.area);
  if (points.some(([x, y], i) => !protectedCell(now, x, y) && (prior[i]! < 0 || grid.at(x, y).fg < 0))) return false;
  const q = Math.floor(Math.max(0, elapsed) * 1000 / FOCUS_MS);
  let drawable = false;
  points.forEach(([x, y], i) => {
    if (protectedCell(now, x, y)) return;
    drawable = true;
    grid.at(x, y).fg = mixColour(prior[i]!, grid.at(x, y).fg, q);
  });
  return drawable;
}
