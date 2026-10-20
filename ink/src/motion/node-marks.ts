// Where a node drew what motion animates, as the host's motion marks it: a tree row that has a `glow` key is a row
// a sync changed (`motion::row`, one cell high, the node's width), and a node that waits for its first data is a
// loading panel (`motion::loading`, the whole node: a State node, or a tree with a loading note).
import type { Node } from "../nodes/view.js";
import { visibleRows } from "../nodes/tree.js";
import type { Window } from "../nodes/window.js";
import type { Rect } from "./grid.js";
import type { SettingsNav } from "../protocol/generated/SettingsNav.js";
import type { TreeMark } from "./interaction.js";
import { bounded } from "./interaction-effects.js";

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


/** Actual visible tree rows only. The last column belongs to the tree's scrollbar even when it is quiet. */
export function treeMark(node: Node, pane: string, x: number, y: number, width: number, height: number, win: Window | null = null): import("./interaction.js").TreeMark | undefined {
  if (node.node !== "tree") return undefined;
  const top = (!node.note && ((node.header ?? []).length > 0 || !!node.header_target) ? 1 : 0) + (node.note ? 1 : 0);
  const room = Math.max(0, height - top);
  if (width <= 1 || room <= 0 || width * room > 65_536) return undefined;
  const shown = visibleRows(node).slice(win?.first ?? 0, (win?.first ?? 0) + Math.min(win?.count ?? room, room));
  const rows = shown.map(({ path }, i) => ({ path: [...path], y: y + top + i }));
  return {
    pane, slot: "body", area: { x, y: y + top, w: width - 1, h: room }, rows,
    open: (node.open ?? []).filter((p) => rows.some((r) => r.path.length === p.length && r.path.every((s, i) => p[i] === s))).map((p) => [...p]),
  };
}


/** Settings uses target-qualified paths; only rows in the measured viewport may retain open-group metadata. */
export function settingsTreeMark(nav: Pick<SettingsNav, "groups">, visible: readonly { target: string }[], area: Rect): TreeMark | undefined {
  if (!bounded(area)) return undefined;
  const shown = visible.slice(0, area.h);
  const wanted = new Set(shown.map((row) => row.target));
  const paths = new Map<string, string[]>();
  const open: string[][] = [];
  for (const group of nav.groups) {
    if (wanted.has(group.target)) {
      paths.set(group.target, [group.target]);
      if (group.open) open.push([group.target]);
    }
    for (const section of group.sections) if (wanted.has(section.target)) paths.set(section.target, [group.target, section.target]);
    if (paths.size === wanted.size) break;
  }
  if (paths.size !== wanted.size) return undefined;
  return { pane: "nav", slot: "settings_nav", area, open,
    rows: shown.map((row, i) => ({ path: paths.get(row.target)!, y: area.y + i })) };
}
