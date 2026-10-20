import React from "react";
import { describe, expect, it } from "vitest";
import { renderNode, type Node } from "../../src/nodes/index.js";
import { firstRow } from "../../src/nodes/window.js";
import { frameAt } from "./helpers.js";

// T22: the whole list was laid out on every snapshot (66 ms for 200 rows). A node taller than its room draws only the
// rows that show (plus MARGIN), keeps its first row where it was unless the cursor leaves it (ui::rows::list), and
// registers hits only for what it draws.
const t = (text: string) => [{ text }] as never;
type Hit = { x: number; y: number; w: number; h: number; target: string };
const draw = (node: Node, height: number, width = 60) => {
  const hits: Hit[] = [];
  const out = frameAt(
    renderNode(node, {
      width,
      height,
      focused: true,
      registry: { add: (h: Hit) => hits.push(h) },
    }),
    width,
    height,
  );
  return { lines: out.split("\n"), hits };
};
const list = (selected: number, n = 200): Node =>
  ({
    node: "list",
    selected,
    rows: Array.from({ length: n }, (_, i) => ({
      target: `row:${i}`,
      text: t(`row number ${i}`),
    })),
  }) as never;

describe("a long list draws what shows", () => {
  it("lays out at most the room's rows for 200 rows at 24 high (rows rendered ≤ visible + margin)", () => {
    const { lines, hits } = draw(list(0), 24);
    expect(lines.length).toBeLessThanOrEqual(24);
    expect(hits.length).toBe(24);
    expect(lines[0]).toContain("row number 0");
    expect(lines[23]).toContain("row number 23");
  });
  it("scrolls only to keep the cursor in view and puts the hits on the lines that show the rows", () => {
    const { lines, hits } = draw(list(150), 24);
    expect(lines.some((l) => l.includes("row number 150"))).toBe(true);
    expect(lines[23]).toContain("row number 150"); // the cursor entered at the bottom edge
    expect(hits.length).toBe(24);
    for (const h of hits)
      expect(lines[h.y]).toContain(h.target.replace("row:", "row number "));
  });
  it("the reference's scroll rule: the first row stays while the cursor is inside, moves by the least to follow it", () => {
    expect(firstRow(200, 5, 24, 0)).toBe(0);
    expect(firstRow(200, 30, 24, 0)).toBe(7);
    expect(firstRow(200, 10, 24, 7)).toBe(7);
    expect(firstRow(200, 3, 24, 7)).toBe(3);
    expect(firstRow(200, 199, 24, 0)).toBe(176);
    expect(firstRow(10, 9, 24, 0)).toBe(0);
  });
  it("a tree of 200 rows and a table draw one screenful too", () => {
    const tree = {
      node: "tree",
      columns: [],
      name_width: 20,
      selected: ["r120"],
      open: [],
      rows: Array.from({ length: 200 }, (_, i) => ({
        id: `r${i}`,
        name: t(`repo ${i}`),
      })),
    } as never;
    const a = draw(tree, 24);
    expect(a.lines.length).toBeLessThanOrEqual(24);
    expect(a.lines.some((l) => l.includes("repo 120"))).toBe(true);
    const table = {
      node: "table",
      columns: [{ heading: "name", width: 20, keep: 1 }],
      selected: 0,
      rows: Array.from({ length: 200 }, (_, i) => ({
        target: `t${i}`,
        cells: [{ heading: "name", text: t(`cell ${i}`) }],
      })),
    } as never;
    const b = draw(table, 24);
    expect(b.lines.length).toBeLessThanOrEqual(24);
    expect(b.hits.length).toBe(23); // the heading row takes one
  });
});
