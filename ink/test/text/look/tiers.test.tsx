import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3"; // Ink's colour library reads this when it loads: keep the cells' styles
});
import React from "react";
import { afterEach, describe, expect, it } from "vitest";
import { renderNode, type Node } from "../../../src/nodes/index.js";
import { styledFrameAt } from "../../nodes/helpers.js";
import { parseAnsi } from "../../nodes/ansi.js";
import { applyLook, defaultLook, type Look } from "../../../src/text/look.js";

// The host resolves every slot at the terminal's colour tier (ipc/look.rs colour_word): `#rrggbb` at truecolor,
// `ansi256(n)` at 256, a terminal colour's name at 16, "" at mono. Ink must draw the selected row, the focus and the
// status colours with exactly those words, the way Ratatui's Theme draws the same Color::Indexed / named colours.
const base = defaultLook("off");
const tier = (slots: Record<string, string>): Look => ({ ...base, slots: { ...base.slots, ...slots } });

afterEach(() => applyLook(base));

const t = (text: string, ink?: string) => [{ text, ...(ink ? { ink } : {}) }] as never;
const list: Node = {
  node: "list",
  selected: 1,
  rows: [
    { target: "a", text: t("alpha"), right: t("x", "bad") },
    { target: "b", text: t("beta") },
    { target: "c", text: t("gamma"), right: t("y", "good") },
  ],
} as never;
const rows = (focused: boolean) => parseAnsi(styledFrameAt(renderNode(list, { width: 20, focused }), 20));

describe("the selected row and status colours at every tier", () => {
  it("256 colours: slots are ansi256(n)", () => {
    applyLook(tier({ "bg.selection": "ansi256(238)", "text.emphasis": "ansi256(231)", "status.error": "ansi256(203)", "status.success": "ansi256(78)" }));
    const r = rows(true);
    expect(r[1]!.every((c) => c.bg === "ansi256(238)")).toBe(true);
    expect([r[1]![1]!.fg, r[1]![1]!.bold]).toEqual(["ansi256(231)", true]);
    expect(r[0]![18]!.fg).toBe("ansi256(203)");
    expect(r[2]![18]!.fg).toBe("ansi256(78)");
    expect(r[0]!.some((c) => c.bg !== undefined)).toBe(false);
  });
  it("16 colours: slots are terminal colour names", () => {
    applyLook(tier({ "bg.selection": "blackBright", "text.emphasis": "whiteBright", "status.error": "red", "status.success": "greenBright" }));
    const r = rows(true);
    expect(r[1]!.every((c) => c.bg === "ansi16(100)")).toBe(true);
    expect([r[1]![1]!.fg, r[1]![1]!.bold]).toEqual(["ansi16(97)", true]);
    expect(r[0]![18]!.fg).toBe("ansi16(31)");
    expect(r[2]![18]!.fg).toBe("ansi16(92)");
  });
  it("mono: no colours, the selected row is bold and underlined where the pane has the keys", () => {
    applyLook(tier(Object.fromEntries(Object.keys(base.slots).map((k) => [k, ""]))));
    const r = rows(true);
    expect(r[1]!.some((c) => c.fg !== undefined || c.bg !== undefined)).toBe(false);
    expect([r[1]![1]!.bold, r[1]![1]!.underline]).toEqual([true, true]);
  });
});

describe("a tree's selected row", () => {
  const tree: Node = {
    node: "tree",
    columns: [],
    open: [["g"]],
    selected: ["g", "r0"],
    rows: [{ id: "g", name: t("group"), children: [{ id: "r0", name: t("repo0") }] }],
  } as never;
  it("is on bg.selection across every cell of the pane, the scrollbar's column included (tui-tree-widget's highlight)", () => {
    const rows = parseAnsi(styledFrameAt(renderNode(tree, { width: 30, focused: true }), 30));
    expect(rows[1]!.length).toBe(30);
    expect(rows[1]!.every((c) => c.bg === "#44475a")).toBe(true);
    expect(rows[0]![29]?.bg).not.toBe("#44475a");
  });
});
