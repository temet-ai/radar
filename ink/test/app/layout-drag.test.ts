import { describe, expect, it } from "vitest";
import { LayoutDrag, layoutKey, previewScreen, previewSnapshot, type Preview } from "../../src/app/layout-drag.js";
import { markHeadingLine, markRowLine, overlayCells, sliceCells, tintCells } from "../../src/nodes/drag.js";
import { cellsOf, type Cell } from "../../src/text/cells.js";
import type { Hit } from "../../src/chrome/types.js";
import type { MouseEvent } from "../../src/input/mouse.js";

const ev = (kind: MouseEvent["kind"], x: number, y = 3): MouseEvent => ({ kind, button: kind === "motion" ? "none" : "left", x, y, ctrl: false, alt: false, shift: false });
const text = (cs: Cell[]) => cs.map((c) => c.ch).join("");

/** A pane "middle" whose heading line, at row 3, has STATE (x 10, 8 wide), APPROVALS (x 18, 11) and UPDATED (x 29, 8). */
function rig() {
  const sent: any[] = [];
  const shown: Array<Preview | null> = [];
  let now = 0;
  const hits: Hit[] = [
    { x: 10, y: 3, w: 7, h: 1, target: "layout:head:state" },
    { x: 17, y: 3, w: 1, h: 1, target: "layout:edge:state:1" },
    { x: 18, y: 3, w: 10, h: 1, target: "layout:head:approvals" },
    { x: 29, y: 3, w: 7, h: 1, target: "layout:head:updated" },
    { x: 28, y: 3, w: 1, h: 1, target: "layout:edge:updated:-1" },
    { x: 0, y: 2, w: 2, h: 20, target: "layout:split:nav:0:200" },
  ];
  const drag = new LayoutDrag({
    send: (i) => sent.push(i),
    show: (p) => shown.push(p),
    screen: () => ({ scope: "gitlab", panes: [] }),
    paneAt: () => "middle",
    hits: () => hits,
    now: () => now,
  });
  const at = (target: string) => hits.find((h) => h.target === target)!;
  return { sent, shown, drag, at, later: (ms: number) => (now += ms) };
}

describe("dragging a column, a heading or a pane border", () => {
  it("the cell after a heading sizes its column: drawn as it moves, sent once on release", () => {
    const { sent, shown, drag, at } = rig();
    expect(drag.mouse(ev("press", 17), at("layout:edge:state:1"))).toBe(true);
    expect(drag.shape()).toBe("col-resize");
    drag.mouse(ev("motion", 20), undefined);
    drag.mouse(ev("motion", 22), undefined);
    expect(shown.at(-1)).toEqual({ kind: "width", key: "gitlab/middle", heading: "state", width: 13 });
    expect(sent).toEqual([]);
    drag.mouse(ev("release", 22), undefined);
    expect(sent).toEqual([{ intent: "layout", layout: "column_width", key: "gitlab/middle", heading: "state", width: 13 }]);
    expect(drag.holding).toBe(false);
  });

  it("a column held to the right widens as its left edge is dragged left, never under the least width", () => {
    const { sent, drag, at } = rig();
    drag.mouse(ev("press", 28), at("layout:edge:updated:-1"));
    drag.mouse(ev("motion", 24), undefined);
    drag.mouse(ev("release", 24), undefined);
    expect(sent.at(-1)).toMatchObject({ heading: "updated", width: 12 });
    drag.mouse(ev("press", 28), at("layout:edge:updated:-1"));
    drag.mouse(ev("motion", 60), undefined);
    drag.mouse(ev("release", 60), undefined);
    expect(sent.at(-1)).toMatchObject({ heading: "updated", width: 3 });
  });

  it("a double click on an edge puts the column's width back", () => {
    const { sent, drag, at, later } = rig();
    drag.mouse(ev("press", 17), at("layout:edge:state:1"));
    drag.mouse(ev("release", 17), undefined);
    later(200);
    drag.mouse(ev("press", 17), at("layout:edge:state:1"));
    expect(sent).toEqual([{ intent: "layout", layout: "column_width", key: "gitlab/middle", heading: "state", width: null }]);
    expect(drag.holding).toBe(false);
  });

  it("a double click fits a column to its widest value, no wider than the room; again on a fitted one, back to the part's", () => {
    const { sent, drag, later } = rig();
    const edge = (most: number, fit: number): Hit => ({ x: 17, y: 3, w: 1, h: 1, target: `layout:edge:state:1:${most}:${fit}` });
    const twice = (hit: Hit) => {
      drag.mouse(ev("press", 17), hit);
      drag.mouse(ev("release", 17), undefined);
      later(100);
      drag.mouse(ev("press", 17), hit);
      later(1000);
    };
    twice(edge(30, 14));
    expect(sent.at(-1)).toEqual({ intent: "layout", layout: "column_width", key: "gitlab/middle", heading: "state", width: 14 });
    twice(edge(11, 40));
    expect(sent.at(-1)).toMatchObject({ width: 11 });
    // STATE is 8 wide in the rig: a value that fits in 8 means it fits already, so the part's width comes back
    twice(edge(30, 8));
    expect(sent.at(-1)).toMatchObject({ width: null });
  });

  it("a heading moved goes where its floating left edge is nearest; the others make room as it passes", () => {
    const { sent, shown, drag, at } = rig();
    drag.mouse(ev("press", 12), at("layout:head:state"));
    expect(drag.shape()).toBe("grabbing");
    drag.mouse(ev("motion", 15), undefined);
    expect(shown.at(-1)).toMatchObject({ order: ["state", "approvals", "updated"], float: 3 });
    drag.mouse(ev("motion", 24), undefined);
    // its left edge at 22 is nearest the boundary after approvals (10 + 11 = 21)
    expect(shown.at(-1)).toMatchObject({ kind: "move", heading: "state", order: ["approvals", "state", "updated"], float: 1 });
    drag.mouse(ev("release", 24), undefined);
    expect(sent).toEqual([{ intent: "layout", layout: "column_order", key: "gitlab/middle", order: ["approvals", "state", "updated"] }]);
  });

  it("a heading pressed and let go where it was sends nothing and clears what was drawn", () => {
    const { sent, shown, drag, at } = rig();
    drag.mouse(ev("press", 12), at("layout:head:state"));
    drag.mouse(ev("motion", 13), undefined);
    drag.mouse(ev("release", 12), undefined);
    expect(sent).toEqual([]);
    expect(shown.at(-1)).toBeNull();
  });

  it("a pane border follows the pointer as a share of the width; a double click puts it back", () => {
    const { sent, shown, drag, at, later } = rig();
    drag.mouse(ev("press", 0, 5), at("layout:split:nav:0:200"));
    drag.mouse(ev("motion", 59, 5), undefined);
    expect(shown.at(-1)).toEqual({ kind: "split", edge: "nav", share: 300 });
    drag.mouse(ev("release", 59, 5), undefined);
    expect(sent).toEqual([{ intent: "layout", layout: "split", edge: "nav", share: 300 }]);
    drag.mouse(ev("press", 0, 5), at("layout:split:nav:0:200"));
    drag.mouse(ev("release", 0, 5), undefined);
    later(100);
    drag.mouse(ev("press", 0, 5), at("layout:split:nav:0:200"));
    expect(sent.at(-1)).toEqual({ intent: "layout", layout: "split", edge: "nav", share: null });
  });

  it("anything but a left press on its targets is left to the rest of the input", () => {
    const { drag, at } = rig();
    expect(drag.mouse(ev("press", 5), { x: 0, y: 3, w: 9, h: 1, target: "row:1" })).toBe(false);
    expect(drag.mouse({ ...ev("press", 12), button: "right" }, at("layout:head:state"))).toBe(false);
    expect(drag.mouse(ev("motion", 12), at("layout:head:state"))).toBe(false);
  });
});

describe("what a drag draws", () => {
  const screen = {
    scope: "gitlab",
    panes: [
      { pane: "middle", body: { node: "tree", columns: [{ heading: "state", width: 8 }, { heading: "ticket", width: 8, extra: true }, { heading: "updated", width: 8 }] } },
      { pane: "detail", body: { node: "table", columns: [{ heading: "state", width: 8 }] } },
    ],
  };

  it("keys name the screen and pane, or the page's tab", () => {
    expect(layoutKey(screen, "middle")).toBe("gitlab/middle");
    expect(layoutKey({ scope: "gitlab", page: { tabs: [{ label: "Merge requests" }], tab: 0 } }, "middle")).toBe("gitlab/page/merge requests/middle");
    expect(layoutKey({ scope: "gitlab", page: { tabs: [], tab: 0 } }, "detail")).toBe("gitlab/page/detail");
  });

  it("a width or an order is laid over only the pane it was dragged in, the held column marked", () => {
    const w = previewScreen(screen, { kind: "width", key: "gitlab/middle", heading: "state", width: 20 });
    expect(w.panes[0].body.columns[0]).toMatchObject({ width: 20, lit: true });
    expect(w.panes[1].body.columns[0].width).toBe(8);
    const m = previewScreen(screen, { kind: "move", key: "gitlab/middle", heading: "updated", order: ["updated", "state"], float: 2 });
    expect(m.panes[0].body.columns.map((c: any) => c.heading)).toEqual(["updated", "state", "ticket"]);
    expect(m.panes[0].body.ordered_columns).toBe(true);
    expect(m.panes[0].body.columns[0]).toMatchObject({ lit: true, float: 2 });
    expect(previewScreen(screen, { kind: "split", edge: "detail", share: 400 }).layout).toEqual({ detail: 400 });
  });

  it("the dock is drawn at the height its edge was dragged to", () => {
    const snap = { screen: { ...screen, dock: { rows: 5, full: true } }, dock: null };
    expect(previewSnapshot(snap, null, null)).toBe(snap);
    expect(previewSnapshot(snap, null, 12).screen.dock).toEqual({ rows: 12, full: false });
  });

  it("cells are cut, laid over and tinted by display cell", () => {
    const line = cellsOf("ab界cd");
    expect(text(sliceCells(line, 1, 3))).toBe("b ");
    expect(text(overlayCells(line, 1, cellsOf("XY")))).toBe("aXY cd");
    const tinted = tintCells(line, 0, 1, { underline: true });
    expect(tinted[0]!.s.underline).toBe(true);
    expect(tinted[1]!.s.underline).toBeUndefined();
  });

  it("the held column's slot is lit down the rows, and its heading floats where the pointer has it", () => {
    const placed = [
      { column: { heading: "a", width: 4, keep: 0 } as any, width: 4 },
      { column: { heading: "b", width: 4, keep: 0, lit: true, float: -2 } as any, width: 4 },
    ];
    const head = markHeadingLine(cellsOf("  A   B   "), placed, 2, (p) => cellsOf(p.column.heading.toUpperCase().padEnd(p.width)), true);
    // the slot (6 to 10) is blank and lit, the heading drawn two cells left of it, over A's air
    expect(text(head)).toBe("  A B     ");
    expect(head[4]!.s.underline).toBe(true);
    // it never floats off the line: at the right edge it stays inside
    const far = markHeadingLine(cellsOf("  A   B   "), [placed[0]!, { ...placed[1]!, column: { ...placed[1]!.column, float: 9 } }], 2, (p) => cellsOf(p.column.heading.toUpperCase().padEnd(p.width)), true);
    expect(text(far)).toBe("  A   B   ");
    const row = markRowLine(cellsOf("  1   2   "), placed, 2, true);
    expect(row.slice(6, 10).every((c) => c.s.underline)).toBe(true);
    expect(row[2]!.s.underline).toBeUndefined();
  });
});
