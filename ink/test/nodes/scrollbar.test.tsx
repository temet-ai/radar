import React from "react";
import { describe, expect, it } from "vitest";
import { renderNode, thumbOf, type Node } from "../../src/nodes/index.js";
import { Framed } from "../../src/app/Panes.js";
import { frameAt } from "./helpers.js";

// ui::scroll::scrollbar: ║ track and █ thumb down a panel's right border between the corners when a list, table or
// log runs past its pane (view_tui draws it with `scrollbar(frame, n - h, first)`); a tree draws the same two glyphs in
// its own last column (tui-tree-widget's experimental_scrollbar), under its heading.
const t = (text: string) => [{ text }] as never;
const W = 40;
const H = 12; // pane height: 10 inner rows

const inFrame = (node: Node) =>
  frameAt(
    <Framed title="pane" width={W} height={H} focused>
      {renderNode(node, { width: W - 2, height: H - 2, focused: true })}
    </Framed>,
    W,
    H,
  ).split("\n");

const edge = (lines: string[]) => lines.slice(1, H - 1).map((l) => l.padEnd(W)[W - 1]);
const inner = (lines: string[]) => lines.slice(1, H - 1).map((l) => l.padEnd(W)[W - 2]);

const rows = (n: number) =>
  Array.from({ length: n }, (_, i) => ({ target: `row:${i}`, text: t(`row number ${i}`) }));
const list = (n: number, selected = 0): Node => ({ node: "list", selected, rows: rows(n) }) as never;
const table = (n: number, selected = 0): Node =>
  ({
    node: "table",
    columns: [{ heading: "name", width: 20, keep: 1 }],
    selected,
    rows: Array.from({ length: n }, (_, i) => ({ target: `t${i}`, cells: [{ heading: "name", text: t(`cell ${i}`) }] })),
  }) as never;
const tree = (n: number, selected: string): Node =>
  ({
    node: "tree",
    columns: [],
    name_width: 20,
    selected: [selected],
    open: [],
    rows: Array.from({ length: n }, (_, i) => ({ id: `r${i}`, name: t(`repo ${i}`) })),
  }) as never;

const glyphs = (col: string[]) => col.join("");

describe("a list, table or log taller than its pane draws Ratatui's scrollbar on the panel's border", () => {
  it("list: thumb at the top on the border from the first row, sized as the Scrollbar sizes it", () => {
    const col = edge(inFrame(list(40)));
    const { start, len } = thumbOf(10, 30, 0);
    expect(col.map((c, i) => (i >= start && i < start + len ? "█" : "║")).join("")).toBe(glyphs(col));
  });
  it("list: the thumb follows the first row shown (the cursor at the end scrolls to the last screenful)", () => {
    const col = edge(inFrame(list(40, 39)));
    const { start, len } = thumbOf(10, 30, 30);
    expect(col.map((c, i) => (i >= start && i < start + len ? "█" : "║")).join("")).toBe(glyphs(col));
    expect(col[9]).toBe("█");
  });
  it("table: the heading row is not a data row: max is rows - (room - 1)", () => {
    const col = edge(inFrame(table(40, 39)));
    const { start, len } = thumbOf(10, 40 - 9, 40 - 9);
    expect(col.map((c, i) => (i >= start && i < start + len ? "█" : "║")).join("")).toBe(glyphs(col));
  });
  it("log: max is total - shown, at is top", () => {
    const lines = Array.from({ length: 10 }, (_, i) => ({ row: i + 5, text: t(`log ${i + 5}`), selected: i === 0 }));
    const col = edge(inFrame({ node: "log", lines, top: 5, total: 40, empty: "none" } as never));
    const { start, len } = thumbOf(10, 30, 5);
    expect(col.map((c, i) => (i >= start && i < start + len ? "█" : "║")).join("")).toBe(glyphs(col));
  });
  it("nothing when everything fits", () => {
    for (const n of [list(10), table(9), list(3)]) expect(edge(inFrame(n)).join("")).not.toMatch(/[║█]/);
  });
  it("the hits still line up: a row's target is on the line that shows the row", () => {
    const hits: { y: number; target: string }[] = [];
    frameAt(
      <Framed title="pane" width={W} height={H} focused>
        {renderNode(list(40, 39), { width: W - 2, height: H - 2, focused: true, registry: { add: (h: { y: number; target: string }) => hits.push(h) } })}
      </Framed>,
      W,
      H,
    );
    expect(hits.find((h) => h.target === "row:39")!.y).toBe(hits.find((h) => h.target === "row:30")!.y + 9);
    expect(hits.length).toBe(10);
  });
});

describe("a tree taller than its pane draws the bar in its last column, under tui-tree-widget's sizing", () => {
  it("the bar sits in the cell the rows leave (width - 1) and the border stays", () => {
    const lines = inFrame(tree(40, "r0"));
    const col = inner(lines);
    const max = 40 - 10;
    const { start, len } = thumbOf(10, max - 1, 0);
    expect(col.map((c, i) => (i >= start && i < start + len ? "█" : "║")).join("")).toBe(glyphs(col));
    expect(edge(lines).every((c) => c === "│")).toBe(true);
  });
  it("it follows the first row", () => {
    const col = inner(inFrame(tree(40, "r39")));
    expect(col[9]).toBe("█");
  });
  it("a tree that fits has no bar", () => {
    expect(inner(inFrame(tree(8, "r0"))).join("")).not.toMatch(/[║█]/);
  });
});

describe("entries (ui::entries): scrolled by line to keep the cursor entry in view, bar scrollbar(n - h, first)", () => {
  // 20 entries of a head and a body line: 40 lines in 10
  const entries = (selected: number): Node =>
    ({
      node: "entries",
      selected,
      empty: "none",
      rows: Array.from({ length: 20 }, (_, i) => ({ target: `e:${i}`, head: t(`head ${i}`), body: [t(`body ${i}`)] })),
    }) as never;

  it("the cursor past the height stays visible (its head too) and is not drawn off the pane", () => {
    const lines = inFrame(entries(15));
    expect(lines.some((l) => l.includes("head 15"))).toBe(true);
    expect(lines.some((l) => l.includes("body 15"))).toBe(true);
    expect(lines.some((l) => l.includes("head 0 "))).toBe(false);
  });
  it("the bar: max = lines - room, at = first line shown", () => {
    const col = edge(inFrame(entries(15)));
    // cursor 15 spans lines 30..32; first = 32 - 10 = 22
    const { start, len } = thumbOf(10, 30, 22);
    expect(col.map((c, i) => (i >= start && i < start + len ? "█" : "║")).join("")).toBe(glyphs(col));
  });
  it("hits sit on the lines that show their entry", () => {
    const hits: { y: number; h: number; target: string }[] = [];
    frameAt(
      <Framed title="pane" width={W} height={H} focused>
        {renderNode(entries(15), { width: W - 2, height: H - 2, focused: true, registry: { add: (h: never) => hits.push(h) } })}
      </Framed>,
      W,
      H,
    );
    expect(hits.find((h) => h.target === "e:15")!.h).toBe(2);
    expect(hits.every((h) => h.y >= 1 && h.y + h.h <= H - 1)).toBe(true);
  });
  it("entries that fit draw no bar", () => {
    expect(edge(inFrame({ node: "entries", selected: 0, empty: "", rows: [{ target: "a", head: t("a"), body: [] }] } as never)).join("")).not.toMatch(/[║█]/);
  });
});
