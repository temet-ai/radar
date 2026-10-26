import React from "react";
import { Box, Text as InkText } from "ink";
import type { NodeContext, Text, Tree, TreeRow } from "./view.js";
import { InputField } from "../editor/InputField.js";
import { fitTreeColumns, type Placed } from "./columns.js";
import { markHeadingLine, markRowLine } from "./drag.js";
import { shrinkRuns } from "./cut.js";
import { useWindow, windowOf } from "./window.js";
import { thumbOf } from "./index.js";
import {
  CellBlock,
  CellLine,
  type Cell,
  cellsOf,
  clipCells,
  ellipsize,
  lineCells,
  padCells,
  patch,
  patchCells,
  useRowCache,
  widthOfCells,
} from "../text/cells.js";
import {
  bgOf,
  iconOf,
  monoTier,
  slotStyle,
  specStyle,
  type Style,
} from "../text/look.js";
import { segRunStyle, segRunWords } from "../text/TextLine.js";
import { cellWidth, fit } from "../chrome/width.js";

interface Shown {
  path: string[];
  row: TreeRow;
  /** Last among its siblings (draws `└─`, else `├─`). */
  last: boolean;
  /** For each ancestor level below the top: whether a line runs down past it (`│`). */
  continues: boolean[];
  /** Position among its siblings (the bands alternate by it). */
  index: number;
}

/** The rows a frontend shows: the roots and the children of the open groups (Tree::visible). */
export function visibleRows(tree: Tree): Shown[] {
  const open = tree.open ?? [];
  const isOpen = (p: string[]) =>
    open.some((o) => o.length === p.length && o.every((s, i) => s === p[i]));
  const out: Shown[] = [];
  const walk = (
    rows: TreeRow[],
    prefix: string[],
    continues: boolean[],
  ) => {
    rows.forEach((row, i) => {
      const path = [...prefix, row.id];
      const last = i === rows.length - 1;
      out.push({ path, row, last, continues, index: i });
      // A level's line runs on past its children while that row has siblings after it (the top level has no line).
      if ((row.children?.length ?? 0) > 0 && isOpen(path))
        walk(
          row.children!,
          path,
          prefix.length === 0 ? continues : [...continues, !last],
        );
    });
  };
  walk(tree.rows, [], []);
  return out;
}

/** One postorder pass over held rows, including hidden descendants; no repeated subtree walk per cached row. */
export function heldDescendantCounts(rows: TreeRow[]): Map<TreeRow, number> {
  const counts = new Map<TreeRow, number>();
  const stack: Array<[TreeRow, boolean]> = rows.map((row) => [row, false]);
  while (stack.length) {
    const [row, visited] = stack.pop()!;
    if (visited) counts.set(row, (row.children ?? []).reduce((n, child) => n + 1 + counts.get(child)!, 0));
    else {
      stack.push([row, true]);
      for (const child of row.children ?? []) stack.push([child, false]);
    }
  }
  return counts;
}

/** Descendants actually held in this view, regardless of which ancestors are open. */
export function heldDescendantCount(row: TreeRow): number {
  return heldDescendantCounts([row]).get(row)!;
}

/** Reserve an honest folded signal: never clip an exact count into a different number. */
export function foldedMarker(count: number, room: number): string {
  if (count === 0 || room < 1) return "";
  const full = ` ⋯ ${count}`;
  return cellWidth(full) <= room ? full : "⋯";
}

/** ui::tree::joint: nothing at the top; under it one space less than the depth, then `├─ ` or `└─ `. */
export function joint(depth: number, last: boolean): string {
  return depth === 0 ? "" : `${" ".repeat(depth - 1)}${last ? "└─ " : "├─ "}`;
}

/** The target a press on the row sends (view_tui draw_tree): `node:<id>` under pointer_action, else the path. */
export const rowTarget = (tree: Tree, s: { path: string[] }) =>
  tree.pointer_action
    ? `node:${s.path[s.path.length - 1]}`
    : s.path.join("\u001f");
const same = (a: string[], b: string[]) =>
  a.length === b.length && a.every((s, i) => s === b[i]);
const total = (cs: { width: number }[]) => cs.reduce((n, c) => n + c.width, 0);

/** The tree's cells and where they go in `width` cells (the pane's inner width): the last cell holds the scrollbar,
 * so the rows use `width - 1`; the columns take their declared widths (ui::Columns::fit) and the names the rest. */
export function treeLayout(tree: Tree, width: number) {
  const row = Math.max(0, width - 1);
  const fitted = fitTreeColumns(
    tree.columns,
    Boolean(tree.ordered_columns),
    row,
    tree.name_width ?? 24,
  );
  const colsWidth = total(fitted);
  return { row, fitted, colsWidth, namesWidth: Math.max(0, row - colsWidth) };
}

const lookOf = (tree: Tree) => ({
  lines: specStyle(tree.look?.lines ?? "text.muted"),
  band: bgOf(tree.look?.band ?? "bg.overlay"),
  heading: bgOf(tree.look?.heading ?? "bg.overlay"),
  bandLevel: tree.look?.band_level ?? 1,
});

/** widgets::row_style(focused, selected): bold emphasis on the selection shade in the pane that has the keys; the
 * secondary text on the same shade in one that has not. The mono tier (no colours) underlines it. */
export function selectedStyle(focused: boolean, plain?: boolean): Style {
  // design::select::selected_row and widgets::row_style: with no colours the selected row is underlined
  if (plain || monoTier())
    return focused ? { bold: true, underline: true } : { underline: true };
  // Only the pane with the keys carries the selection ground; another pane's current row sits on the overlay shade.
  return { ...slotStyle("text.emphasis"), bold: true, backgroundColor: bgOf(focused ? "bg.selection" : "bg.overlay") };
}

/** Tree states change the ground, and bold the row in the pane with the keys, keeping each icon/name/cell's
 * semantic foreground. */
export function treeSelectedStyle(focused: boolean, plain?: boolean): Style {
  if (plain || monoTier())
    return focused ? { bold: true, underline: true } : { underline: true };
  return focused ? { bold: true, backgroundColor: bgOf("bg.selection") } : { backgroundColor: bgOf("bg.overlay") };
}

/** Fold signals have priority over columns only when a narrow row leaves no cell for its name. */
export function treeRowHasColumns(row: TreeRow, depth: number, open: boolean, fitted: Placed[], width: number): boolean {
  const room = Math.max(0, width - depth * 2 - cellWidth(joint(depth, true)));
  const folded = !open && (row.children?.length ?? 0) > 0;
  return (row.cells?.length ?? 0) > 0 && !(folded && room <= total(fitted) + 1);
}

/** A name's runs and the room to cut them in, as ui::tree item() makes the row's line. */
function nameRuns(row: TreeRow, plain: boolean | undefined) {
  return row.name.map((seg) => {
    const words = segRunWords(seg, plain);
    return cellsOf(words, plain ? {} : segRunStyle(seg));
  });
}

/** One tree row as a cell buffer holds it after BandedTree::render: the tree widget's indent and line, the name
 * cut to the room the columns leave, the cells under their headings, then the highlight or band over the row, the
 * lines running down past parents. */
function rowCells(
  tree: Tree,
  shown: Shown,
  fitted: Placed[],
  width: number,
  open: boolean,
  selected: boolean,
  focused: boolean,
  heldCount: number,
  plain?: boolean,
): Cell[] {
  const { row, path, last, continues, index } = shown;
  const depth = path.length - 1;
  const look = lookOf(tree);
  const lead = cellsOf(joint(depth, last), look.lines);
  const indent = cellsOf(" ".repeat(depth * 2));
  const leadW = widthOfCells(lead);
  const room = Math.max(0, width - depth * 2 - leadW);
  const count = open ? 0 : heldCount;
  const hasCells = treeRowHasColumns(row, depth, open, fitted, width);
  const colsW = hasCells ? total(fitted) : 0;
  const namesW = Math.max(0, room - colsW);
  const capacity = count > 0 && namesW === 1 ? 1 : Math.max(0, namesW - 1);
  const marker = foldedMarker(count, capacity);
  const markerW = cellWidth(marker);
  const nameRoom = Math.max(0, capacity - markerW);
  let runs = nameRuns(row, plain);
  if (tree.cut_names) {
    runs = shrinkRuns(runs, nameRoom);
  }
  if (row.detail) {
    const d = lineCells(row.detail, plain);
    const used = runs.reduce((n, r) => n + widthOfCells(r), 0);
    if (d.length && used + widthOfCells(d) <= nameRoom) runs = [...runs, d];
  }
  // A chip gives way after the extra columns hold their place: once one is dropped for lack of width the chip stays
  // gone at every narrower width, though the dropped column frees room for the name.
  const extrasKept = tree.columns.every((c) => !c.extra || fitted.some((f) => f.column === c));
  if (extrasKept && row.chips && row.chips.text.length) {
    const gives = row.chips.gives;
    const chips = lineCells(row.chips.text, plain);
    if (gives < runs.length) {
      const fixed =
        runs.reduce((n, r) => n + widthOfCells(r), 0) -
        widthOfCells(runs[gives]!);
      const left = Math.max(0, nameRoom - (fixed + widthOfCells(chips)));
      if (left >= 12) {
        const text = runs[gives]!.map((c) => c.ch)
          .join("")
          .trimEnd();
        const style = runs[gives]![0]?.s ?? {};
        runs = runs.map((r, i) =>
          i === gives
            ? cellsOf(fit(text, Math.min(left, cellWidth(text))), style)
            : r,
        );
        runs = [...runs, chips];
      }
    }
  }
  // line_of: the name is cut at one cell under its room, then padded to it; the cells follow under their headings.
  const name: Cell[] = [];
  let used = 0;
  for (const r of runs) {
    const cut = clipCells(r, Math.max(0, nameRoom - used));
    used += widthOfCells(cut);
    name.push(...cut);
  }
  name.push(...cellsOf(marker, plain ? {} : specStyle("text.muted")));
  const line: Cell[] = [...padCells(name, namesW)];
  if (hasCells) {
    for (const { column, width: w } of fitted) {
      const cell = row.cells!.find((c) => c.heading === column.heading);
      // ui::Columns::cell_lines: cut to the column less its air cell, then padded to the whole column in the style of the
      // cell's last run (an underlined or shaded last word runs on to the column's edge)
      const text = cell ? lineCells(cell.text, plain) : [];
      line.push(
        ...padCells(clipCells(text, w - 1), w, text[text.length - 1]?.s ?? {}),
      );
    }
  }
  let cells = padCells(clipCells([...indent, ...lead, ...line], width), width);
  if (plain) {
    if (selected) cells = patchCells(cells, treeSelectedStyle(focused, true));
    return cells;
  }
  if (selected) cells = patchCells(cells, treeSelectedStyle(focused));
  else {
    const bg =
      depth === 0
        ? look.heading
        : depth >= look.bandLevel && bandOf(tree, shown, look.bandLevel)
          ? look.band
          : undefined;
    if (bg) cells = patchCells(cells, { backgroundColor: bg });
  }
  void index;
  continues.forEach((goesOn, k) => {
    const level = k + 1;
    const x = 3 * level - 1;
    if (goesOn && level < depth && cells[x])
      cells[x] = { ch: "│", w: 1, s: patch(cells[x]!.s, look.lines) };
  });
  return cells;
}

/** Whether the row sits on a band: the blocks at the band level alternate by position, and everything under a
 * banded block is banded with it. */
function bandOf(tree: Tree, shown: Shown, bandLevel: number): boolean {
  // walk down from the top-level row along the path, remembering the band of the ancestor at the band level
  let rows = tree.rows;
  let band = false;
  for (let d = 0; d < shown.path.length; d++) {
    const i = rows.findIndex((r) => r.id === shown.path[d]);
    if (i < 0) break;
    if (d === bandLevel) band = i % 2 === 1;
    rows = rows[i]!.children ?? [];
  }
  return shown.path.length - 1 >= bandLevel && band;
}

/** Column::heading_spans in `width` cells: the icon (when icons are on), the heading in capitals, bold and muted. */
function headingCells(column: Placed["column"], width: number): Cell[] {
  const style: Style = { ...slotStyle("text.muted"), bold: true };
  const text = column.heading.toUpperCase();
  const icon = column.icon ? iconOf(column.icon) : undefined;
  if (!icon?.glyph) return padCells(cellsOf(text, style), width, style);
  const lead = cellsOf(`${icon.glyph} `, specStyle(icon.style));
  return padCells(
    clipCells([...lead, ...cellsOf(text, style)], Math.max(0, width - 1)),
    width,
    style,
  );
}

/** The highlight style covers the whole area, the cell the scrollbar uses too (tui-tree-widget's highlight_style); a
 * band stops short of it. */
function withScrollCell(
  cells: Cell[],
  selected: boolean,
  focused: boolean,
  plain?: boolean,
  bar?: Cell,
): Cell[] {
  // the highlight is patched over the bar's glyph (tui-tree-widget draws the bar first, then the rows' styles)
  const ch = bar?.ch ?? " ";
  const base = bar?.s ?? {};
  if (!selected) return bar ? [...cells, bar] : cells;
  return [
    ...cells,
    { ch, w: 1, s: patch(base, treeSelectedStyle(focused, plain)) },
  ];
}

/** tui-tree-widget's experimental_scrollbar: the tree's last column over its rows (under the heading), a track ║ in
 * bg.overlay and a thumb █ in accent.muted; sized as ScrollbarState::new(rows - room).position(first), whose content
 * length is one under the scroll range. Nothing while everything fits. */
function barCell(
  k: number,
  room: number,
  total: number,
  first: number,
  plain?: boolean,
): Cell | undefined {
  const max = total - room;
  if (max <= 0) return undefined;
  const { start, len } = thumbOf(room, Math.max(0, max - 1), first);
  const on = k >= start && k < start + len;
  return {
    ch: on ? "█" : "║",
    w: 1,
    s: plain ? {} : slotStyle(on ? "accent.muted" : "bg.overlay"),
  };
}

export function TreeNode({ tree, ctx }: { tree: Tree; ctx: NodeContext }) {
  const all = visibleRows(tree);
  const counts = heldDescendantCounts(tree.rows);
  const window = useWindow();
  const cached = useRowCache();
  const win = windowOf(all, window);
  const { row: width, fitted, colsWidth } = treeLayout(tree, ctx.width);
  const open = tree.open ?? [];
  const selected = tree.selected ?? [];
  const hasHeader =
    !tree.note &&
    ((tree.header && tree.header.length > 0) || Boolean(tree.header_target));
  // What every row of this tree is drawn from besides its own data: a row's key adds that data and its place.
  const treeKey = JSON.stringify([
    width,
    fitted.map((f) => [f.column, f.width]),
    tree.look ?? null,
    tree.cut_names ?? null,
    ctx.focused,
    ctx.plainText ?? false,
  ]);
  const look = lookOf(tree);
  const headerCells = (): Cell[] => {
    const room = Math.max(0, width - colsWidth);
    const left = clipCells(
      [...cellsOf(" "), ...lineCells(tree.header ?? [], ctx.plainText)],
      Math.max(0, room - 1),
    );
    const cols = fitted.flatMap(({ column, width: w }) =>
      headingCells(column, w),
    );
    return markHeadingLine([...padCells(left, room), ...cols], fitted, room, (p) => headingCells(p.column, p.width), ctx.plainText);
  };
  return (
    <Box flexDirection="column">
      {hasHeader ? (
        <CellLine
          cells={cached(
            `h${treeKey}${JSON.stringify(tree.header ?? [])}`,
            headerCells,
          )}
          plain={ctx.plainText}
        />
      ) : null}
      {tree.note ? (
        <InkText
          dimColor={tree.note.state !== "failed"}
          color={
            tree.note.state === "failed" && !ctx.plainText
              ? slotStyle("status.error").color
              : undefined
          }
        >
          {" "}
          {tree.note.state === "loading"
            ? "… "
            : tree.note.state === "failed"
              ? "✗ "
              : ""}
          {tree.note.text}
        </InkText>
      ) : null}
      {(() => {
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
        win.shown.forEach((s, k) => {
          const isOpen = open.some((o) => same(o, s.path));
          const isSel = same(selected, s.path);
          const editing =
            tree.editing && same(tree.editing.path, s.path)
              ? tree.editing
              : null;
          if (editing) {
            flush();
            const depth = s.path.length - 1;
            const prefixW = Math.min(width, depth * 2 + cellWidth(joint(depth, s.last)) + editing.at);
            const prefix = padCells(clipCells(rowCells(tree, s, fitted, width, isOpen, isSel, ctx.focused, counts.get(s.row)!, ctx.plainText), prefixW), prefixW);
            out.push(
              <Box key={s.path.join("/")}>
                <CellLine cells={prefix} plain={ctx.plainText} />
                <InputField
                  text={editing.text}
                  caret={editing.caret}
                  width={Math.max(0, ctx.width - 1 - widthOfCells(prefix))}
                />
              </Box>,
            );
            return;
          }
          const bar = window
            ? barCell(k, window.count, all.length, window.first, ctx.plainText)
            : undefined;
          const { children, ...own } = s.row;
          const key = [
            treeKey,
            s.path.join("\u001f"),
            s.last ? 1 : 0,
            s.continues.map(Number).join(""),
            isOpen ? 1 : 0,
            isSel ? 1 : 0,
            bandOf(tree, s, look.bandLevel) ? 1 : 0,
            bar ? `${bar.ch}${bar.s.color ?? ""}` : "",
            counts.get(s.row)!,
            JSON.stringify(own),
          ].join("|");
          block.push(
            cached(key, () =>
              withScrollCell(
                markRowLine(
                  rowCells(
                    tree,
                    s,
                    fitted,
                    width,
                    isOpen,
                    isSel,
                    ctx.focused,
                    counts.get(s.row)!,
                    ctx.plainText,
                  ),
                  fitted,
                  width - colsWidth,
                  ctx.plainText,
                ),
                isSel,
                ctx.focused,
                ctx.plainText,
                bar,
              ),
            ),
          );
        });
        flush();
        return out;
      })()}
    </Box>
  );
}

export { ellipsize };
