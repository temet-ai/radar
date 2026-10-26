import React from "react";
import { Box, Text as InkText } from "ink";
import type {
  EntryView,
  ListRow,
  LogPaintLine,
  NodeContext,
  TableRow,
  Text,
} from "./view.js";
import type { ColumnSpec } from "./view.js";
import { InputField } from "../editor/InputField.js";
import { fitTableColumns, type Placed } from "./columns.js";
import { markHeadingLine, markRowLine, withEdges } from "./drag.js";
import {
  CellBlock,
  CellLine,
  type Cell,
  cellsOf,
  clipCells,
  lineCells,
  padCells,
  paragraphCells,
  patch,
  patchCells,
  useRowCache,
  widthOfCells,
} from "../text/cells.js";
import { iconOf, slotStyle, specStyle, type Style } from "../text/look.js";
import { selectedStyle } from "./tree.js";
import { useWindow, windowOf } from "./window.js";
import { useScrollReport } from "./index.js";
import { StateView } from "../text/StateView.js";

/** List::style(text.primary): a cell with no colour of its own is primary text. */
const primary = (): Style => slotStyle("text.primary");
const under = (cs: Cell[], base: Style): Cell[] =>
  cs.map((c) => ({ ...c, s: patch(base, c.s) }));

/** A line of the list's rows: the highlight across the row for the cursor (widgets::row_style). */
function rowLine(
  cells: Cell[],
  width: number,
  selected: boolean,
  focused: boolean,
  plain?: boolean,
): Cell[] {
  const full = padCells(clipCells(cells, width), width);
  const base = plain ? full : under(full, primary());
  return selected ? patchCells(base, selectedStyle(focused, plain)) : base;
}

/** ui::rows::settle: the row the cursor rests on: `cursor` when a row can be selected there, else the next one that can,
 * else the nearest before it; a list with none to rest on keeps its cursor where it is (so a heading is lit). */
export function settledCursor(rows: readonly { heading?: boolean }[], cursor: number): number {
  if (rows.length === 0) return 0;
  const at = Math.min(Math.max(cursor, 0), rows.length - 1);
  for (let i = at; i < rows.length; i++) if (!rows[i]!.heading) return i;
  for (let i = at; i >= 0; i--) if (!rows[i]!.heading) return i;
  return at;
}

/** ui::rows::list: one cell of air, the left part cut to leave room for the word at the right edge, that word one cell
 * from the edge; the detail only when it fits whole. */
export function ListNode({
  rows,
  selected,
  editing,
  ctx,
}: {
  rows: ListRow[];
  selected?: number | null;
  editing?: { row: number; at: number; text: string; caret: number } | null;
  ctx: NodeContext;
}) {
  const width = ctx.width;
  const cursor = settledCursor(rows, selected ?? 0);
  const win = windowOf(rows, useWindow());
  const cached = useRowCache();
  const listKey = `${width}|${ctx.focused ? 1 : 0}|${ctx.plainText ? 1 : 0}`;
  const out: React.ReactNode[] = [];
  let block: Cell[][] = [];
  const flush = () => {
    if (block.length)
      out.push(
        <CellBlock
          key={`b${out.length}`}
          lines={block}
          plain={ctx.plainText}
        />,
      );
    block = [];
  };
  win.shown.forEach((r, k) => {
    const i = k + win.first;
    const isSel = i === cursor;
    const lineOf = (isSel: boolean): Cell[] => {
      const right = r.right ? flatCells(r.right, ctx.plainText) : [];
      const rw = widthOfCells(right);
      const room = Math.max(0, width - (rw > 0 ? rw + 3 : 2));
      let left = lineCells(r.text, ctx.plainText);
      if (r.detail) {
        const d = flatCells(r.detail, ctx.plainText);
        if (
          d.length &&
          widthOfCells(left) + widthOfCells(d) + (rw > 0 ? 2 : 0) <= room
        )
          left = [...left, ...d];
      }
      left = clipCells(left, room);
      // A row about something with a link opens it (ui::link::line): its words carry the address.
      if (r.link) left = left.map((c) => ({ ...c, s: { ...c.s, link: r.link! } }));
      let line = [...cellsOf(" "), ...left];
      if (rw > 0)
        line = [...padCells(line, Math.max(0, width - rw - 1)), ...right];
      return rowLine(line, width, isSel, ctx.focused, ctx.plainText);
    };
    if (editing && editing.row === i) {
      // view_tui render_section: the row is drawn as it is, then the field over it from `at` cells in, to one cell short
      // of the row's end (that last cell stays the row's own).
      flush();
      const row = lineOf(isSel);
      const at = Math.min(Math.max(0, editing.at), width);
      const field = Math.max(0, width - at - 1);
      out.push(
        <Box key={r.target + i}>
          {at > 0 ? <CellLine cells={padCells(clipCells(row, at), at)} plain={ctx.plainText} /> : null}
          {field > 0 ? <InputField text={editing.text} caret={editing.caret} width={field} /> : null}
          {width - at - field > 0 && row.length > 0 ? <CellLine cells={row.slice(-1)} plain={ctx.plainText} /> : null}
        </Box>,
      );
      return;
    }
    block.push(cached(`${listKey}|${isSel ? 1 : 0}|${JSON.stringify(r)}`, () => lineOf(isSel)));
  });
  flush();
  return <Box flexDirection="column">{out}</Box>;
}

/** view_tui::flat / word: the words as one run in the first non-icon run's style (icons as their glyph and a gap). */
export function flatCells(text: Text, plain?: boolean): Cell[] {
  const cells = lineCells(text, plain);
  if (plain) return cells;
  const first =
    lineCells(
      text.filter((s) => !s.icon),
      false,
    )[0]?.s ?? {};
  void iconOf;
  return cells.map((c) => ({ ...c, s: first }));
}

function cellFor(
  cells: { heading: string; text: Text }[],
  column: ColumnSpec,
): Text {
  return cells.find((c) => c.heading === column.heading)?.text ?? [];
}

/** ui::table: padded by a cell each side; one bold muted heading row in capitals; the cursor row highlighted across the
 * whole width. A cell too long for its column ends in `…`. */
export function TableNode({
  columns,
  rows,
  selected,
  empty,
  ctx,
}: {
  columns: ColumnSpec[];
  rows: TableRow[];
  selected?: number | null;
  empty?: string | null;
  ctx: NodeContext;
}) {
  const win = windowOf(rows, useWindow());
  if (rows.length === 0)
    return (
      <StateView
        state="empty"
        text={empty ?? "Nothing here."}
        plain={ctx.plainText}
        width={ctx.width}
      />
    );
  const width = ctx.width;
  const fitted = fitTableColumns(columns, Math.max(1, width - 2));
  const flexDeclared = Math.max(
    0,
    width - 2 - columns.reduce((n, c) => (c.flex ? n : n + c.width), 0),
  );
  const heading: Style = { ...slotStyle("text.muted"), bold: true };
  const head: Cell[] = markHeadingLine(
    withEdges([...cellsOf(" "), ...fitted.flatMap(({ column, width: w }) => headingFor(column, w, heading))], fitted, 1, "table", ctx.plainText),
    fitted,
    1,
    (p) => headingFor(p.column, p.width, heading),
    ctx.plainText,
  );
  // ui::table::cut: nothing at all in no room, not even the `…`
  const cut = (cs: Cell[], w: number) =>
    widthOfCells(cs) <= w
      ? cs
      : w === 0
        ? []
        : [
            ...clipCells(cs, Math.max(0, w - 1)),
            { ch: "…", w: 1, s: cs[cs.length - 1]?.s ?? {} },
          ];
  return (
    <Box flexDirection="column">
      <CellLine
        cells={padCells(
          clipCells(head, width),
          width,
          ctx.plainText ? {} : heading,
        )}
        plain={ctx.plainText}
      />
      <CellBlock
        plain={ctx.plainText}
        lines={win.shown.map((r, k) => {
          const i = k + win.first;
          const line: Cell[] = [...cellsOf(" ")];
          for (const p of fitted) {
            const w = p.width - 1; // the cell of air after the value is not the cell's
            // ui::table cuts a value to the width the column declares (the flex one: what the panel leaves it) with a
            // `…`, then the table clips it again, hard, to the narrower column its layout gave
            const declared = p.column.flex ? flexDeclared : p.column.width - 1;
            line.push(
              ...padCells(
                clipCells(
                  cut(
                    lineCells(cellFor(r.cells, p.column), ctx.plainText),
                    declared,
                  ),
                  w,
                ),
                p.width,
              ),
            );
          }
          return rowLine(
            markRowLine(line, fitted, 1, ctx.plainText),
            width,
            i === (selected ?? 0),
            ctx.focused,
            ctx.plainText,
          );
        })}
      />
    </Box>
  );
}

function headingFor(column: ColumnSpec, width: number, style: Style): Cell[] {
  const text = column.heading.toUpperCase();
  const icon = column.icon ? iconOf(column.icon) : undefined;
  if (!icon?.glyph)
    return padCells(clipCells(cellsOf(text, style), width), width, style);
  const lead = cellsOf(`${icon.glyph} `, specStyle(icon.style));
  return padCells(
    clipCells([...lead, ...cellsOf(text, style)], Math.max(0, width - 1)),
    width,
    style,
  );
}
export type { Placed };

/** ui::entries: every line of every entry, with the entry it belongs to and whether it is the head. */
export function entryLines(rows: EntryView[], width?: number, plain = false) {
  const lines: { text: Text; cells?: Cell[]; entry: number | null; head: boolean }[] = [];
  const spans: [number, number][] = [];
  const append = (text: Text, entry: number | null, head: boolean) => {
    if (plain && width !== undefined) {
      // Accessibility suffixes and links expand the authored visual line. Wrap
      // that presentation once, and share its rows with scrolling and targets.
      for (const cells of paragraphCells(lineCells(text, true), Math.max(1, width - 2)))
        lines.push({ text, cells, entry, head });
    } else lines.push({ text, entry, head });
  };
  rows.forEach((r, i) => {
    for (const l of r.before ?? [])
      append(l, null, false);
    const start = lines.length;
    append(r.head, i, true);
    for (const l of r.body ?? [])
      append(l, i, false);
    spans.push([start, lines.length]);
  });
  return { lines, spans };
}

/** Entries::render's scroll: the first line shown stays unless the cursor entry's body (as far as it fits, never its
 * head out of view) needs it moved, and never past the last full screen. */
export function entriesFirst(
  spans: [number, number][],
  n: number,
  h: number,
  cursor: number | null,
  first: number,
): number {
  let f = first;
  if (cursor !== null && spans.length > 0) {
    const [start, end] = spans[Math.min(cursor, spans.length - 1)]!;
    if (end > f + h) f = end - h;
    if (start < f) f = start;
  }
  return Math.max(0, Math.min(f, n - h));
}

/** ui::entries: lines before a row (never chosen), the head behind one cell of air, the body under it; the cursor's
 * head is highlighted. A pane shorter than the lines shows them from the scroll's first line (HitRegion keeps it). */
export function EntriesNode({
  rows,
  selected,
  empty,
  ctx,
}: {
  rows: EntryView[];
  selected?: number | null;
  empty: string;
  ctx: NodeContext;
}) {
  const win = useWindow();
  if (rows.length === 0)
    return (
      <StateView
        state="empty"
        text={empty}
        plain={ctx.plainText}
        width={ctx.width}
      />
    );
  const width = ctx.width;
  const line = (t: Text, isSel: boolean, wrapped?: Cell[]) => {
    const cells = [
      ...cellsOf(" "),
      ...(wrapped ?? clipCells(lineCells(t, ctx.plainText), Math.max(0, width - 2))),
    ];
    return isSel
      ? rowLine(cells, width, true, ctx.focused, ctx.plainText)
      : padCells(cells, width);
  };
  const { lines } = entryLines(rows, width, ctx.plainText);
  const shown = win ? lines.slice(win.first, win.first + win.count) : lines;
  return (
    <Box flexDirection="column">
      <CellBlock
        plain={ctx.plainText}
        lines={shown.map((l) =>
          line(l.text, l.head && l.entry === (selected ?? -1), l.cells),
        )}
      />
    </Box>
  );
}

/** ui::log in the pane: each line at the left edge, the selected one highlighted across the row. */
export function LogNode({
  lines,
  top,
  total,
  empty,
  ctx,
}: {
  lines: LogPaintLine[];
  top: number;
  total: number;
  empty: string;
  ctx: NodeContext;
}) {
  const hidden = Math.max(0, total - new Set(lines.map((l) => l.row)).size);
  // view_tui Node::Log: scroll::scrollbar(total - shown, top.min(total - shown)) on the panel's border
  useScrollReport({ max: hidden, at: Math.min(top, hidden) });
  if (lines.length === 0)
    return (
      <StateView
        state="empty"
        text={empty}
        plain={ctx.plainText}
        width={ctx.width}
      />
    );
  const rows = ctx.height ? lines.slice(0, ctx.height) : lines;

  return (
    <Box flexDirection="column">
      <CellBlock
        plain={ctx.plainText}
        lines={rows.map((l) =>
          l.selected
            ? rowLine(
                lineCells(l.text, ctx.plainText),
                ctx.width,
                true,
                ctx.focused,
                ctx.plainText,
              )
            : padCells(
                clipCells(lineCells(l.text, ctx.plainText), ctx.width),
                ctx.width,
              ),
        )}
      />
      {hidden > 0 ? (
        <InkText
          dimColor
        >{`  ${top + 1}–${top + lines.length} of ${total}`}</InkText>
      ) : null}
    </Box>
  );
}
