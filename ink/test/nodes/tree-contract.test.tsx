import { vi } from "vitest";
vi.hoisted(() => { process.env.FORCE_COLOR = "3"; });
import React from "react";
import { afterEach, describe, expect, it } from "vitest";
import { renderNode } from "../../src/nodes/index.js";
import { foldedMarker, heldDescendantCount, heldDescendantCounts, treeLayout, treeSelectedStyle } from "../../src/nodes/tree.js";
import { hitSpecs } from "../../src/nodes/hits.js";
import type { Node, Tree, TreeRow } from "../../src/nodes/view.js";
import { applyLook, defaultLook } from "../../src/text/look.js";
import { cellWidth } from "../../src/chrome/width.js";
import { paintHover } from "../../src/app/hover.js";
import { Glow } from "../../src/motion/effects.js";
import { parseFrame } from "../../src/motion/frame.js";
import { frameAt, styledFrameAt } from "./helpers.js";
import { parseAnsi } from "./ansi.js";

const row = (id: string, children: TreeRow[] = []): TreeRow => ({ id, name: [{ text: id }], children });
const tree = (over: Partial<Tree> = {}): Node => ({ node: "tree", columns: [], rows: [row("watch", [row("cases", [row("patrol")])])], ...over });
const draw = (node: Node, width: number, focused = true) => styledFrameAt(renderNode(node, { width, focused }), Math.max(1, width));
afterEach(() => applyLook(defaultLook()));

describe("folded held-descendant counts", () => {
  it("computes a deep held tree once in postorder without recursive recounting", () => {
    let root = row("patrol");
    for (let i = 0; i < 10_000; i++) root = row(`watch-${i}`, [root]);
    const counts = heldDescendantCounts([root]);
    expect(counts.size).toBe(10_001);
    expect(counts.get(root)).toBe(10_000);
  });

  it("counts recursively regardless of the open paths, never remote totals in detail", () => {
    const root = row("watch", [row("cases", [row("patrol"), row("clues")]), row("archive")]);
    root.detail = [{ text: " 90 replies" }];
    expect(heldDescendantCount(root)).toBe(4);
    const out = frameAt(renderNode(tree({ rows: [root], open: [["watch", "cases"]] }), { width: 40, focused: true }), 40);
    expect(out).toContain("watch 90 replies ⋯ 4");
    expect(out).not.toContain("⋯ 90");
    expect(frameAt(renderNode(tree({ rows: [row("watch")] }), { width: 40, focused: true }), 40)).not.toContain("⋯");
  });

  it("reserves the full marker before clipping long Unicode names and fitting columns", () => {
    const root = row("watch", [row("patrol")]);
    root.name = [{ text: "界é界é界é界é界é界é界é" }];
    root.cells = [{ heading: "state", text: [{ text: "ready" }], target: "state:watch" }];
    for (const width of [18, 24, 80, 160]) {
      const node = tree({ rows: [root], columns: [{ heading: "state", width: 7, keep: 1 }] });
      const out = frameAt(renderNode(node, { width, focused: true }), width);
      expect(out).toContain("⋯ 1");
      expect(out).toContain("ready");
      expect(cellWidth(out.split("\n")[0]!)).toBeLessThanOrEqual(width);
      expect(out).not.toContain("�");
    }
  });

  it("keeps whole exact counts or a bare marker at tiny widths, including zero and one cell", () => {
    expect(foldedMarker(1234, 8)).toBe(" ⋯ 1234");
    for (const width of [1, 2, 3, 4, 5, 6]) expect(foldedMarker(1234, width)).toBe("⋯");
    expect(foldedMarker(1234, 0)).toBe("");
    expect(foldedMarker(0, 20)).toBe("");
    expect(treeLayout(tree() as Tree, 0).row).toBe(0);
    expect(frameAt(renderNode(tree(), { width: 2, focused: true }), 2)).toBe("⋯");
    expect(frameAt(renderNode(tree(), { width: 0, focused: true }), 1)).toBe("");
  });

  it("drops obscuring columns on a folded narrow row and removes their invisible click targets", () => {
    const root = row("watch", [row("patrol")]);
    root.cells = [{ heading: "state", text: [{ text: "ready" }], target: "state:watch" }];
    const node = tree({ rows: [root], columns: [{ heading: "state", width: 12, keep: 1 }] });
    expect(frameAt(renderNode(node, { width: 8, focused: true }), 8)).toContain("⋯ 1");
    expect(hitSpecs(node, 8).map((h) => h.target)).toEqual(["watch"]);
    expect(hitSpecs(node, 40).map((h) => h.target)).toEqual(["watch", "state:watch"]);
  });

  it("keeps the inline editor's name offset after removing the root stripe", () => {
    const root = row("watch");
    root.name = [{ text: "name: old" }];
    root.stripe = "tenant.green";
    const node = tree({ rows: [root], editing: { path: ["watch"], at: 6, text: "Vimes", caret: 5 } });
    const out = frameAt(renderNode(node, { width: 30, focused: true }), 30);
    expect(out).toContain("name: Vimes");
    expect(out).not.toContain("▌");
  });
});

describe("tree row state grounds preserve meaning", () => {
  const root: TreeRow = { id: "watch", name: [{ text: "watch" }], stripe: "tenant.green", children: [
    { id: "patrol", name: [{ text: "failed", ink: "bad", strong: true }], cells: [{ heading: "state", text: [{ text: "ready", ink: "good" }] }] },
  ] };
  const node = (selected: string[] = []): Node => tree({ rows: [root], open: [["watch"]], selected, columns: [{ heading: "state", width: 8, keep: 1 }] });
  const word = (frame: string, text: string) => parseAnsi(frame)[1]!.find((c) => c.ch === text)!;

  it("uses normal, focused selection, inactive selection and heading grounds without replacing semantic colors", () => {
    const normal = draw(node(), 30);
    const focused = draw(node(["watch", "patrol"]), 30);
    const inactive = draw(node(["watch", "patrol"]), 30, false);
    expect(word(normal, "f")).toMatchObject({ fg: "#ff6363", bold: true });
    expect(word(focused, "f")).toMatchObject({ fg: "#ff6363", bold: true, bg: "#44475a" });
    expect(word(inactive, "f")).toMatchObject({ fg: "#ff6363", bold: true, bg: "#1a162a" });
    expect(word(focused, "r").fg).toBe("#50fa7b");
    expect(word(focused, "r").bold).toBeUndefined();
    expect(parseAnsi(normal)[0]![0]).toMatchObject({ ch: "w", bg: "#1a162a" });
    expect(parseAnsi(focused)[1]!.every((c) => c.bg === "#44475a")).toBe(true);
  });

  it("hover changes only the ground and does not replace a focused selection", () => {
    const rect = { x: 0, y: 1, w: 30, h: 1 };
    const hover = paintHover(draw(node(), 30), 30, rect);
    expect(word(hover, "f")).toMatchObject({ fg: "#ff6363", bold: true, bg: "#2a263e" });
    const selected = paintHover(draw(node(["watch", "patrol"]), 30), 30, rect);
    expect(word(selected, "f")).toMatchObject({ fg: "#ff6363", bg: "#44475a" });
  });

  it("change glow pulses the existing ground, preserving glyphs, colors and modifiers", () => {
    const grid = parseFrame(draw(node(["watch", "patrol"]), 30), 30, 2);
    const before = grid.cells.map((c) => ({ ...c }));
    const glow = new Glow(0xe135ff, { fg: 0xf8f8f2, bg: 0x12101a });
    glow.process(100, grid, { x: 0, y: 1, w: 30, h: 1 });
    expect(grid.cells.map(({ ch, fg, attr }) => ({ ch, fg, attr }))).toEqual(before.map(({ ch, fg, attr }) => ({ ch, fg, attr })));
    expect(grid.at(0, 1).bg).not.toBe(before[30]!.bg);
  });

  it("uses accessible mono/plain modifiers only when colored grounds are unavailable", () => {
    expect(treeSelectedStyle(true, true)).toEqual({ bold: true, underline: true });
    expect(treeSelectedStyle(false, true)).toEqual({ underline: true });
    const look = defaultLook();
    applyLook({ ...look, tier: "mono", slots: Object.fromEntries(Object.keys(look.slots).map((s) => [s, ""])) });
    expect(treeSelectedStyle(true)).toEqual({ bold: true, underline: true });
    expect(treeSelectedStyle(false)).toEqual({ underline: true });
  });
});
