import { describe, expect, it } from "vitest";
import { afterIntent, heldGroupPaths, screenKey, sectionKey, treeStep, treeTarget } from "../../../src/input/route.js";
import type { Chord, Intent } from "../../../src/chrome/types.js";
import type { Tree, TreeRow } from "../../../src/nodes/view.js";

const row = (id: string, children: TreeRow[] = [], heading = false): TreeRow => ({ id, name: [{ text: id }], children, heading });
const tree = (over: Partial<Tree> = {}): Tree => ({
  columns: [], rows: [row("watch", [row("cases", [row("patrol"), row("clues")]), row("archive")]), row("guild", [row("jobs")])],
  open: [["watch"], ["watch", "cases"]], selected: ["watch", "cases", "patrol"], ...over,
});
const char = (c: string): Chord => ({ key: { char: c } });
const fold = (path: string[], open: boolean): Intent => ({ intent: "fold", target: treeTarget(path), open });
const select = (path: string[]): Intent => ({ intent: "select", target: treeTarget(path) });
const snapshot = (body: Tree) => ({ focus: "middle", screen: { panes: [{ pane: "middle", body: { node: "tree", ...body } }] } });

describe("the shared tree key contract", () => {
  it("space toggles one group, plus only opens it, and equals opens all held groups", () => {
    const t = tree({ selected: ["watch"], open: [] });
    expect(treeStep(t, char(" "))).toEqual(fold(["watch"], true));
    expect(treeStep(t, char("+"))).toEqual(fold(["watch"], true));
    expect(treeStep(t, char("="))).toEqual({ intent: "open_all" });
    expect(heldGroupPaths(t)).toEqual([["watch"], ["watch", "cases"], ["guild"]]);
    expect(treeStep(tree({ selected: ["watch"] }), char("+"))).toBeUndefined();
    expect(treeStep(tree({ selected: ["watch"] }), char(" "))).toEqual(fold(["watch"], false));
  });

  it("right opens before entering; left closes before selecting a parent without closing it", () => {
    expect(treeStep(tree({ selected: ["watch"], open: [] }), { key: "right" })).toEqual(fold(["watch"], true));
    expect(treeStep(tree({ selected: ["watch"] }), { key: "right" })).toEqual(select(["watch", "cases"]));
    expect(treeStep(tree({ selected: ["watch", "cases"] }), { key: "left" })).toEqual(fold(["watch", "cases"], false));
    expect(treeStep(tree(), { key: "left" })).toEqual(select(["watch", "cases"]));
    expect(treeStep(tree(), char("-"))).toEqual(fold(["watch", "cases"], false));
  });

  it("skips unselectable headings on entry and when finding a parent", () => {
    const t = tree({ rows: [row("watch", [row("heading", [row("patrol")], true)])], open: [["watch"], ["watch", "heading"]], selected: ["watch"] });
    expect(treeStep(t, { key: "right" })).toEqual(select(["watch", "heading", "patrol"]));
    t.selected = ["watch", "heading", "patrol"];
    expect(treeStep(t, { key: "left" })).toEqual(select(["watch"]));
    expect(treeStep(t, char("-"))).toEqual(fold(["watch"], false));
    t.selected = ["watch"];
    t.open = [["watch"]];
    expect(treeStep(t, { key: "right" })).toBeUndefined();
  });

  it("leaves stale and hidden selections alone for row-local keys; movement recovers to the first selectable row", () => {
    for (const selected of [["missing"], ["watch", "cases", "patrol"]]) {
      const t = tree({ selected, open: [] });
      for (const c of [char("+"), char("-"), char(" "), { key: "left" }, { key: "right" }] as Chord[])
        expect(treeStep(t, c)).toBeUndefined();
      for (const key of ["up", "down", "home", "end"] as const)
        expect(treeStep(t, { key })).toEqual(select(["watch"]));
    }
  });

  it("handles empty trees, leaf boundaries, up ownership, and modifiers", () => {
    expect(treeStep(tree({ rows: [] }), char("="))).toEqual({ intent: "open_all" });
    expect(treeStep(tree({ rows: [] }), { key: "right" })).toBeUndefined();
    const leaf = tree({ rows: [row("watch")], selected: ["watch"] });
    expect(treeStep(leaf, { key: "up" })).toBeUndefined();
    expect(treeStep(leaf, { key: "left" })).toBeUndefined();
    expect(treeStep(leaf, { key: "right" })).toBeUndefined();
    for (const c of [char("="), char("+"), char("-"), { key: "left" }, { key: "right" }] as Chord[])
      for (const mod of [{ ctrl: true }, { alt: true }]) expect(treeStep(tree(), { ...c, ...mod })).toBeUndefined();
    for (const key of ["left", "right", "up", "down", "home", "end"] as const)
      expect(treeStep(tree(), { key, shift: true })).toBeUndefined();
  });

  it("keeps field/overlay/inline-editor capture and pane movement outside tree routing", () => {
    const s = snapshot(tree()).screen;
    for (const c of [char("h"), char("l"), char("1"), { key: "tab" }, { key: "back_tab" }] as Chord[])
      expect(screenKey(s, "middle", c)).toEqual({ intent: "key", chord: c });
    const c = char("=");
    expect(screenKey({ ...s, overlay: {} }, "middle", c)).toEqual({ intent: "key", chord: c });
    expect(screenKey({ panes: [{ ...s.panes[0]!, fields: [{ caret: 0 }] }] }, "middle", c)).toEqual({ intent: "key", chord: c });
    const body = { node: "tree", ...tree(), editing: { path: ["watch"], at: 0, text: "", caret: 0 } };
    expect(sectionKey({ body }, c)).toEqual({ intent: "key", chord: c });
    expect(screenKey({ panes: [{ pane: "middle", body }] }, "middle", c)).toEqual({ intent: "key", chord: c });
  });
});

describe("optimistic tree state between host snapshots", () => {
  it("collapsing an ancestor moves the selected descendant before the next key", () => {
    const before = snapshot(tree());
    const next = afterIntent(before, fold(["watch", "cases"], false));
    expect(next.screen.panes[0]!.body.selected).toEqual(["watch", "cases"]);
    expect(screenKey(next.screen, "middle", char("-"))).toEqual(fold(["watch"], false));
    expect(before.screen.panes[0]!.body.selected).toEqual(["watch", "cases", "patrol"]);
  });

  it("defers keys after OpenAll to actual host state regardless of optional interaction metadata", () => {
    for (const motion of [undefined, { interaction: { context: null } }, { interaction: { context: { scope: "watch" } } }]) {
      const before = { ...snapshot(tree({ open: [], selected: ["watch"] })), ...(motion ? { motion } : {}) };
      const next = afterIntent(before, { intent: "open_all" });
      expect(next.focus).toBeNull();
      expect(next.screen).toBe(before.screen);
      expect(next.screen.panes[0]!.body.open).toEqual([]);
      expect(next.screen.panes[0]!.body.selected).toEqual(["watch"]);
      expect(screenKey(next.screen, typeof next.focus === "string" ? next.focus : undefined, { key: "right" })).toEqual({ intent: "key", chord: { key: "right" } });
    }
  });

  it("does not predict an invalid, hidden, heading, leaf or duplicate target", () => {
    const before = snapshot(tree({ open: [] }));
    for (const intent of [select(["watch", "cases"]), select(["missing"]), fold(["missing"], true), fold(["watch", "cases", "patrol"], true)])
      expect(afterIntent(before, intent).screen.panes[0]!.body).toBe(before.screen.panes[0]!.body);
    const headings = snapshot(tree({ rows: [row("heading", [], true)] }));
    expect(afterIntent(headings, select(["heading"])).screen.panes[0]!.body).toBe(headings.screen.panes[0]!.body);
    const duplicates = snapshot(tree({ rows: [row("watch", [row("a")]), row("watch", [row("b")])] }));
    expect(afterIntent(duplicates, fold(["watch"], true)).screen.panes[0]!.body).toBe(duplicates.screen.panes[0]!.body);
  });

  it("rejects ambiguous ancestors even if only one duplicate is a group or owns the requested child", () => {
    for (const duplicate of [row("watch"), row("watch", [row("archive", [row("clues")])])]) {
      const before = snapshot(tree({ rows: [row("watch", [row("cases", [row("patrol")])]), duplicate, row("guild", [row("jobs")])], open: [["watch"], ["watch", "cases"]], selected: ["guild"] }));
      const body = before.screen.panes[0]!.body;
      expect(heldGroupPaths(body)).toEqual([["guild"]]);
      for (const intent of [select(["watch", "cases"]), select(["watch", "cases", "patrol"]), fold(["watch"], true), fold(["watch", "cases"], false)])
        expect(afterIntent(before, intent).screen.panes[0]!.body).toBe(body);
      const expanded = afterIntent(before, { intent: "open_all" }).screen.panes[0]!.body;
      expect(expanded.selected).toEqual(["guild"]);
      expect(expanded.open).toEqual([["watch"], ["watch", "cases"]]);
      expect(expanded).toBe(body);
    }
  });
});
