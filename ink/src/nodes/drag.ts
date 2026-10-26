// A column being dragged, drawn: its slot in the heading line and its cells down the rows lit, and while it is moved
// its heading floats under the pointer. The marks are Ink's own (app/layout-drag.ts puts them on the column it shows
// in a drag); the host never sends them.
import type { ColumnSpec } from "./view.js";
import type { Placed } from "./columns.js";
import { cellsOf, padCells, patchCells, widthOfCells, type Cell } from "../text/cells.js";
import { bgOf, slotStyle, type Style } from "../text/look.js";

/** What a drag puts on the column it holds: `lit`, and `float`, how far the moved heading is from its slot. */
export type DragMark = { lit?: boolean; float?: number };
export const markOf = (c: ColumnSpec): DragMark => c as ColumnSpec & DragMark;

/** Cells `from` to `to` of a line; a wide cell cut at either edge becomes spaces. A short line is padded. */
export function sliceCells(cs: Cell[], from: number, to: number): Cell[] {
  const out: Cell[] = [];
  let at = 0;
  for (const c of cs) {
    const end = at + c.w;
    if (end > from && at < to) {
      if (at >= from && end <= to) out.push(c);
      else for (let k = Math.max(at, from); k < Math.min(end, to); k++) out.push({ ch: " ", w: 1, s: c.s });
    }
    at = end;
    if (at >= to) break;
  }
  return Number.isFinite(to) ? padCells(out, to - from) : out;
}

/** `over` drawn on a line at cell `at`. */
export function overlayCells(cs: Cell[], at: number, over: Cell[]): Cell[] {
  const w = widthOfCells(over);
  return [...sliceCells(cs, 0, at), ...over, ...sliceCells(cs, at + w, Math.max(at + w, widthOfCells(cs)))];
}

/** The cells `from` to `to` of a line with `s` laid over them. */
export function tintCells(cs: Cell[], from: number, to: number, s: Style): Cell[] {
  return [...sliceCells(cs, 0, from), ...patchCells(sliceCells(cs, from, to), s), ...sliceCells(cs, to, Math.max(to, widthOfCells(cs)))];
}

/** Where the heading line's edges are, each the cell between two headings that sizes a column (`sizes`): `sign` 1 when
 * dragging right widens it, -1 when dragging left does. A tree's columns hold to its right edge, as do a table's after
 * the column that takes what is left (`tree` false: a table), so there the cell before a heading sizes it. */
export function columnEdges(placed: Placed[], x0: number, tree: boolean): Array<{ x: number; sizes: Placed; sign: 1 | -1 }> {
  const flex = placed.findIndex((p) => p.column.flex);
  const out: Array<{ x: number; sizes: Placed; sign: 1 | -1 }> = [];
  let x = x0;
  placed.forEach((p, i) => {
    if (tree) {
      if (x > 0) out.push({ x: x - 1, sizes: p, sign: -1 });
    } else if (flex < 0 || i < flex) out.push({ x: x + p.width - 1, sizes: p, sign: 1 });
    else if (i + 1 < placed.length) out.push({ x: x + p.width - 1, sizes: placed[i + 1]!, sign: -1 });
    x += p.width;
  });
  return out;
}

/** The divider drawn on each edge of a heading line, so where a column is sized can be seen. */
export function withEdges(line: Cell[], placed: Placed[], x0: number, tree: boolean, plain?: boolean): Cell[] {
  let out = line;
  for (const e of columnEdges(placed, x0, tree)) out = overlayCells(out, e.x, cellsOf("│", plain ? {} : slotStyle("text.muted")));
  return out;
}

/** How a held column is lit: the selection ground, or underlined with no colours. */
export const heldStyle = (plain?: boolean): Style => (plain ? { underline: true } : { backgroundColor: bgOf("bg.selection") });

/** The held column among `placed` (laid from cell `x0`): where its slot is and how far its heading floats. */
export function heldColumn(placed: Placed[], x0: number): { x: number; w: number; float: number; at: number } | null {
  let x = x0;
  for (let i = 0; i < placed.length; i++) {
    const p = placed[i]!;
    const m = markOf(p.column);
    if (m.lit) return { x, w: p.width, float: m.float ?? 0, at: i };
    x += p.width;
  }
  return null;
}

/** A heading line with the held column's slot lit and, while it is moved, its heading drawn where the pointer has it. */
export function markHeadingLine(line: Cell[], placed: Placed[], x0: number, heading: (p: Placed) => Cell[], plain?: boolean): Cell[] {
  const held = heldColumn(placed, x0);
  if (!held) return line;
  const s = heldStyle(plain);
  if (!held.float) return tintCells(line, held.x, held.x + held.w, s);
  const slot = overlayCells(line, held.x, cellsOf(" ".repeat(held.w), plain ? {} : { ...slotStyle("text.muted"), ...s }));
  const lineW = widthOfCells(line);
  const at = Math.max(x0, Math.min(lineW - held.w, held.x + held.float));
  return overlayCells(slot, at, patchCells(heading(placed[held.at]!), { ...s, bold: true }));
}

/** A row with the held column's cells lit. */
export function markRowLine(line: Cell[], placed: Placed[], x0: number, plain?: boolean): Cell[] {
  const held = heldColumn(placed, x0);
  return held ? tintCells(line, held.x, held.x + held.w, heldStyle(plain)) : line;
}
