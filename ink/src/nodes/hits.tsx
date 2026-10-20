import React, { useId, useLayoutEffect, useRef, useContext } from "react";
import { Box, type DOMElement } from "ink";
import type { Node, NodeContext } from "./view.js";
import { visibleRows, treeLayout, treeRowHasColumns } from "./tree.js";
import { textWords } from "../text/TextLine.js";
import { cellWidth } from "../chrome/width.js";
import { useScrollReport } from "./index.js";
import { entriesFirst, entryLines } from "./lists.js";
import { WindowContext, firstRow, type Window } from "./window.js";
import { motion } from "../motion/motion.js";
import { nodeMarks, treeMark } from "../motion/node-marks.js";
import { MotionSceneContext, MotionPaneContext } from "../motion/scene.js";
import { screenRect } from "../workarea/hit.js";

/** A rectangle a press acts on, in cells relative to the node's top-left corner. */
export interface HitSpec {
  dx: number;
  dy: number;
  w: number;
  h: number;
  target: string;
}

/** view_tui::padded: a label's target takes a cell either side of it, within the line, and is at least MIN_TARGET (3)
 * cells wide. `x`/`w` are the label's cells in a line `within` cells wide; the target's `dx` and `w` come back. */
export function padded(x: number, w: number, within: number): { dx: number; w: number } {
  let from = Math.max(0, x - 1);
  let right = Math.min(within, x + w + 1);
  while (right - from < 3 && (from > 0 || right < within)) {
    if (right < within) right += 1;
    if (right - from < 3 && from > 0) from -= 1;
  }
  return { dx: from, w: right - from };
}

/** ui::view's Target::PATH_SEP: a tree row's target is its path's ids joined by it (TreeRow::target_of). */
export const PATH_SEP = "\u001f";

/** The rectangles Ratatui's view_tui registers (`cx.hit`) for a node, with the target ids it uses, laid out the way
 * these components draw: a list's, table's and entries' rows, a tree's rows (`node:<id>` under `pointer_action`),
 * its header target and its contributed cells' targets, a log's lines. Later entries win where they overlap. */
export function hitSpecs(
  node: Node,
  width: number,
  win: Window | null = null,
  plain = false,
): HitSpec[] {
  const out: HitSpec[] = [];
  const f0 = win?.first ?? 0;
  const inWin = (i: number) =>
    !win || (i >= win.first && i < win.first + win.count);
  const row = (dy: number, target: string, h = 1) =>
    out.push({ dx: 0, dy, w: width, h, target });
  switch (node.node) {
    case "list":
      node.rows.forEach((r, i) => {
        if (!r.heading && inWin(i)) row(i - f0, r.target);
      });
      break;
    case "table":
      // the heading row is the first line
      node.rows.forEach((r, i) => inWin(i) && row(1 + i - f0, r.target));
      break;
    case "entries": {
      // lines, not rows: a head and its body are one target; a window cuts them to the lines that show
      const { spans } = entryLines(node.rows, width, plain);
      spans.forEach(([start, stop], i) => {
        const top = Math.max(start, f0);
        const end = Math.min(stop, win ? win.first + win.count : Infinity);
        if (end > top) row(top - f0, node.rows[i]!.target, end - top);
      });
      break;
    }
    case "log":
      node.lines.forEach((l, i) => row(i, `log:${l.row}`));
      break;
    case "tree": {
      const hasHeader =
        !node.note &&
        ((node.header ?? []).length > 0 || Boolean(node.header_target));
      const top = (hasHeader ? 1 : 0) + (node.note ? 1 : 0);
      const { fitted, namesWidth, row: rowWidth } = treeLayout(node, width);
      const shown = visibleRows(node);
      shown.forEach(({ path, row: r }, i) => {
        if (!r.heading && inWin(i))
          row(
            top + i - f0,
            node.pointer_action
              ? `node:${path[path.length - 1]}`
              : path.join(PATH_SEP),
          );
      });
      if (hasHeader && node.header_target) {
        // the header's words (after its leading space), as far as they show, choose its target: a label, so a cell
        // either side of it (view_tui draw_tree, Cx::hit_label)
        const w = Math.min(
          1 + cellWidth(textWords(node.header ?? [])),
          Math.max(0, namesWidth - 1),
        );
        out.push({ ...padded(0, w, width), dy: 0, h: 1, target: node.header_target });
      }
      shown.forEach(({ row: r, path }, i) => {
        if (!inWin(i)) return;
        const open = (node.open ?? []).some((p) => p.length === path.length && p.every((id, at) => id === path[at]));
        if (!treeRowHasColumns(r, path.length - 1, open, fitted, rowWidth)) return;
        for (const cell of r.cells ?? []) {
          if (!cell.target) continue;
          let x = namesWidth;
          for (const f of fitted) {
            if (f.column.heading === cell.heading) {
              out.push({
                dx: x,
                dy: top + i - f0,
                w: Math.max(1, f.width - 1),
                h: 1,
                target: cell.target,
              });
              break;
            }
            x += f.width;
          }
        }
      });
      break;
    }
    default:
      break;
  }
  return out;
}

/** How many rows a node can show in `height` and which row the cursor is on, for the nodes that scroll their rows
 * (lists, tables, trees); null for the rest. */
function scrollOf(
  node: Node,
  width: number,
  height: number | undefined,
  plain = false,
): { total: number; cursor: number; room: number; spans?: [number, number][] } | null {
  if (height === undefined) return null;
  switch (node.node) {
    case "list":
      return {
        total: node.rows.length,
        cursor: node.selected ?? 0,
        room: height,
      };
    case "table":
      return {
        total: node.rows.length,
        cursor: node.selected ?? 0,
        room: height - 1,
      };
    case "tree": {
      const hasHeader =
        !node.note &&
        ((node.header ?? []).length > 0 || Boolean(node.header_target));
      const shown = visibleRows(node);
      const sel = node.selected ?? [];
      const at = shown.findIndex(
        (s) =>
          s.path.length === sel.length && s.path.every((x, i) => x === sel[i]),
      );
      void width;
      return {
        total: shown.length,
        cursor: Math.max(0, at),
        room: height - (hasHeader ? 1 : 0) - (node.note ? 1 : 0),
      };
    }
    case "entries": {
      const { lines, spans } = entryLines(node.rows, width, plain);
      return {
        total: lines.length,
        cursor: node.selected ?? -1,
        room: height,
        spans,
      };
    }
    default:
      return null;
  }
}

/** Wraps a node and, after each layout, registers the node's hit rectangles at its absolute place (the sum of its
 * ancestors' computed offsets, which Ink has settled by the time layout effects run). It also keeps the node's scroll
 * (the first row shown, moved only to keep the cursor in view) and lays out only the rows that show. */
export function HitRegion({
  node,
  ctx,
  children,
}: {
  node: Node;
  ctx: NodeContext;
  children: React.ReactNode;
}) {
  const ref = useRef<DOMElement>(null);
  const first = useRef(0);
  const sc = scrollOf(node, ctx.width, ctx.height, ctx.plainText);
  let win: Window | null = null;
  // A body whose heading takes all its room (a table or a tree with a header, one row high) shows no row: the window
  // is empty, so no row is laid out and no row is a target under the pane's border or the pane below.
  const noRoom = !!sc && sc.total > 0 && sc.room <= 0;
  if (sc && sc.room > 0 && sc.total > sc.room) {
    first.current = sc.spans
      ? entriesFirst(sc.spans, sc.total, sc.room, sc.cursor < 0 ? null : sc.cursor, first.current)
      : firstRow(sc.total, sc.cursor, sc.room, first.current);
    win = { first: first.current, count: sc.room };
  } else {
    first.current = 0;
    if (noRoom) win = { first: 0, count: 0 };
  }
  // ui::table / ui::rows: scroll::scrollbar(n - h, first) on the panel's border (trees draw theirs in their last column)
  useScrollReport(
    win && !noRoom && node.node !== "tree" && sc
      ? { max: sc.total - sc.room, at: win.first }
      : null,
  );
  // Where this node drew rows a sync changed and a panel waiting for data, for motion (view_tui's motion::row and
  // motion::loading). Nothing is measured while motion is off or the node has neither.
  const markId = useId();
  const scene = useContext(MotionSceneContext);
  const pane = useContext(MotionPaneContext);
  useLayoutEffect(() => {
    // The wrapper survives body-kind changes. Every committed layout replaces its marks,
    // including nonloading bodies and a measurement that is no longer usable.
    motion.markNode(markId, [], [], scene);
    if (scene) motion.markTree(markId, scene);
    if ((node.node !== "tree" && node.node !== "state") || motion.engine.level === "off" || !ref.current) return;
    const at = screenRect(ref.current);
    if (!at || !Object.values(at).every(Number.isSafeInteger) || at.w <= 0 || at.h <= 0) return;
    // A waiting panel is marked over the body's whole room, as view_tui marks its inner area, not only its words.
    const m = nodeMarks(node, at.x, at.y, at.w, Math.max(at.h, ctx.height ?? 0), win);
    motion.markNode(markId, m.rows, m.loading, scene);
    if (scene && pane && !pane.startsWith("work:") && motion.engine.level === "full") motion.markTree(markId, scene, treeMark(node, pane, at.x, at.y, ctx.width, ctx.height ?? at.h, win));
  });
  useLayoutEffect(() => () => motion.markNode(markId, [], []), [markId]);
  useLayoutEffect(() => () => { if (scene) motion.markTree(markId, scene); }, [markId, scene]);
  useLayoutEffect(() => {
    const registry = ctx.registry;
    if (!registry || !ref.current) return;
    let x = 0;
    let y = 0;
    try {
      for (let n: DOMElement | undefined = ref.current; n; n = n.parentNode) {
        x += n.yogaNode?.getComputedLeft() ?? 0;
        y += n.yogaNode?.getComputedTop() ?? 0;
      }
    } catch {
      return; // a node torn down mid-commit has no layout to register
    }
    for (const s of hitSpecs(node, ctx.width, win, ctx.plainText))
      registry.add({
        x: x + s.dx,
        y: y + s.dy,
        w: s.w,
        h: s.h,
        target: s.target,
      });
  });
  return (
    <Box ref={ref} width={ctx.width} flexDirection="column" overflow="hidden">
      <WindowContext.Provider value={win}>{children}</WindowContext.Provider>
    </Box>
  );
}
