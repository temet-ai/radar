import { bgOf, slotStyle, type Style } from "../text/look.js";
import { cellsOf, clipCells, lineCells, padCells, type Cell } from "../text/cells.js";

/**
 * The line to type into at the top of a pane (go to `:`, filter `/`), as ui::input lays it out
 * (view_tui::draw_field_bar, input::render_field): the mark in three cells (bold accent.secondary), then the field
 * on bg.overlay in bold text.emphasis, scrolled to keep the caret in view, the caret cell reversed; or, when the
 * field does not have the keys (`caret` absent), the words as set in text.emphasis. The hint (what it does) is on
 * the next row, and a blank row follows, so a pane's body starts three rows down.
 */
export interface FieldBar {
  mark: string;
  text: string;
  caret?: number | null;
  hint: any[];
}

/** The rows one top field takes in its pane (the bar, its hint, a blank). */
export const FIELD_ROWS = 3;

/** input::render_field on `width` cells: the text from the offset that keeps the caret visible, on bg.overlay. */
function inputCells(text: string, caret: number, width: number): Cell[] {
  const bg = bgOf("bg.overlay");
  const base: Style = { ...slotStyle("text.emphasis"), bold: true, backgroundColor: bg };
  const chars = [...text];
  const col = Math.min(caret, chars.length);
  const offset = Math.max(0, col + 1 - width);
  const visible = clipCells(cellsOf(chars.slice(offset).join(""), base), width);
  const row = padCells(visible, width, { backgroundColor: bg });
  const at = col - offset;
  // the caret: a block in the buffer (text.emphasis, reversed), on a cell that holds a character or a blank
  const cell = row[at];
  if (cell) row[at] = { ...cell, s: { ...cell.s, ...slotStyle("text.emphasis"), inverse: true } };
  return row;
}

/** One field bar's cells at its own width: the mark, the field or the words as set. Rows are `inner` cells wide. */
function barRow(f: FieldBar, inner: number): Cell[] {
  const mark = clipCells(cellsOf(f.mark, { ...slotStyle("accent.secondary"), bold: true }), 3);
  const lead = padCells(mark, Math.min(3, inner));
  const room = Math.max(0, inner - 4);
  if (room === 0) return padCells(lead, inner);
  const body =
    f.caret != null
      ? inputCells(f.text, f.caret, room)
      : clipCells(cellsOf(f.text, slotStyle("text.emphasis")), room);
  return padCells([...lead, ...body], inner);
}

/** A field at the top of a pane: the bar, its hint a row under it, and a blank row (at most `room` rows). */
export function fieldRows(f: FieldBar, inner: number, room: number, plain?: boolean): Cell[][] {
  if (room <= 0) return [];
  const rows: Cell[][] = [barRow(f, inner)];
  if (room > 1) {
    const hint = clipCells(lineCells(f.hint ?? [], plain), Math.max(0, inner - 4));
    rows.push(padCells([...cellsOf("   "), ...hint], inner));
  }
  if (room > 2) rows.push(padCells([], inner));
  return rows;
}

/** A one-line field kept at the bottom of a pane. */
export const bottomFieldRow = (f: FieldBar, inner: number): Cell[] => barRow(f, inner);
