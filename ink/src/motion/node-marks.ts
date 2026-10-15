// Where a node drew what motion animates, as view_tui marks it for Ratatui: a tree row that has a `glow` key is a row
// a sync changed (`motion::row`, one cell high, the node's width), and a node that waits for its first data is a
// loading panel (`motion::loading`, the whole node: a State node, or a tree with a loading note).
import type { Node } from "../nodes/view.js";
import { visibleRows } from "../nodes/tree.js";
import type { Window } from "../nodes/window.js";
import type { Rect } from "./grid.js";

export type NodeMarks = {
  rows: Array<readonly [string, Rect]>;
  loading: Rect[];
};

/** The marks of `node` drawn at (x, y) in screen cells, `width` x `height` big, showing `win` of its rows. */
export function nodeMarks(
  node: Node,
  x: number,
  y: number,
  width: number,
  height: number,
  win: Window | null = null,
): NodeMarks {
  const out: NodeMarks = { rows: [], loading: [] };
  const whole: Rect = { x, y, w: width, h: height };
  if (node.node === "state") {
    if (node.state === "loading") out.loading.push(whole);
    return out;
  }
  if (node.node !== "tree") return out;
  if (node.note?.state === "loading") out.loading.push(whole);
  const hasHeader =
    !node.note &&
    ((node.header ?? []).length > 0 || Boolean(node.header_target));
  const top = (hasHeader ? 1 : 0) + (node.note ? 1 : 0);
  const first = win?.first ?? 0;
  visibleRows(node).forEach(({ row }, i) => {
    if (!row.glow) return;
    if (win && (i < win.first || i >= win.first + win.count)) return;
    out.rows.push([row.glow, { x, y: y + top + i - first, w: width, h: 1 }]);
  });
  return out;
}
