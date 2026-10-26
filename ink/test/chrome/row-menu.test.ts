import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { GEAR, hitAt, layoutMenu, type Cell } from "../../src/chrome/layout.js";
import { cellWidth } from "../../src/chrome/width.js";
import { applyLook, bgOf, currentLook, defaultLook, slotStyle } from "../../src/text/look.js";
import type { Choice, MenuView } from "../../src/chrome/types.js";

const text = (cells: Cell[]) => cells.map((c) => c.text).join("");
let before = currentLook();
beforeEach(() => { before = currentLook(); applyLook(defaultLook("unicode")); });
afterEach(() => applyLook(before));

const row = (label: string, key: string, hint: string, i: number, extra: Partial<Choice> = {}): Choice => ({
  value: label, label, hint, keys: key, key, mnemonic_at: null, mnemonic_shift: false, target: `chrome:menu:${i}`, ...extra,
});
const rowMenu = (over: Partial<MenuView> = {}): MenuView => ({
  id: "actions",
  title: "watch-rota vimes/night-shift",
  question: false,
  selected: 0,
  items: [
    row("open in the browser", "o", "", 0),
    row("pull", "p", "not behind main", 1),
    row("ask an agent", "A", "running 4m", 2),
    row("hide it", "H", "radar is read only", 3, { muted: true }),
  ],
  ...over,
});

describe("a list menu's hint", () => {
  it.each([80, 160])("ends one cell before the right border, muted, at %i columns", (width) => {
    const laid = layoutMenu(rowMenu(), width, 1, 30);
    const lines = laid.lines.map(text);
    for (const [at, hint] of [[2, "not behind main"], [3, "running 4m"], [4, "radar is read only"]] as const) {
      const line = lines[at]!;
      expect(cellWidth(line)).toBe(laid.w);
      expect(line.endsWith(`${hint} │`)).toBe(true);
      const cell = laid.lines[at]!.find((c) => c.text === hint)!;
      expect(cell.style?.color).toBe(slotStyle("text.muted").color);
    }
    // The label stays after the key column.
    expect(lines[3]).toMatch(/^│ A {2}ask an agent/);
  });

  it("lines up every label when only some rows have an icon", () => {
    const items = [row("open its page", "⏎", "", 0), row("check out the branch", "c", "", 1, { icon: "branch" })];
    const lines = layoutMenu(rowMenu({ items }), 80, 1, 30).lines.map(text);
    expect(lines[1]!.indexOf("open")).toBe(lines[2]!.indexOf("check"));
  });

  it("keeps the selection shade across the gap before the hint", () => {
    const laid = layoutMenu(rowMenu({ selected: 1 }), 80, 1, 30);
    const cells = laid.lines[2]!.slice(1, -1);
    const bg = bgOf("bg.selection");
    expect(bg).toBeTruthy();
    expect(cells.every((c) => c.style?.backgroundColor === bg)).toBe(true);
    // An unselected row does not carry it.
    expect(laid.lines[3]!.slice(1, -1).some((c) => c.style?.backgroundColor === bg)).toBe(false);
  });

  it("is wide enough for key, label, three cells and the hint", () => {
    const laid = layoutMenu(rowMenu({ title: "x", items: [row("pull", "p", "not behind main", 0)] }), 80, 1, 30);
    // ` p ` + ` ` + `pull` + 3 + hint + 1, inside the borders
    expect(laid.w).toBe(2 + 3 + 1 + 4 + 3 + cellWidth("not behind main") + 1);
    expect(text(laid.lines[1]!)).toBe("│ p  pull   not behind main │");
  });

  it("shows no 'shift ' words and no underline when mnemonic_at is null", () => {
    const laid = layoutMenu(rowMenu(), 80, 1, 30);
    expect(laid.lines.slice(1).map(text).join("\n")).not.toContain("shift ");
    expect(laid.lines.flat().some((c) => c.style?.underline)).toBe(false);
    expect(laid.lines.map(text).join("\n")).toContain("│ H  hide it");
  });

  it("is a target on its whole row", () => {
    const laid = layoutMenu(rowMenu(), 80, 1, 30);
    expect(laid.hits.filter((h) => h.target.startsWith("chrome:menu:")).map((h) => h.target)).toEqual(["chrome:menu:0", "chrome:menu:1", "chrome:menu:2", "chrome:menu:3"]);
  });
});

describe("a row menu's gear", () => {
  const top = (m: MenuView, width = 80) => {
    const laid = layoutMenu(m, width, 1, 30);
    return { laid, line: laid.lines[0]! };
  };

  it("sits before the corner of the title edge and is the chrome:gear target", () => {
    const { laid, line } = top(rowMenu({ gear: true }));
    const flat = text(line);
    expect(cellWidth(flat)).toBe(laid.w);
    expect(flat).toMatch(/^╭ watch-rota vimes\/night-shift ─+ ⚙ , ─╮$/);
    const hit = laid.hits.find((h) => h.target === GEAR)!;
    expect(hit).toMatchObject({ y: laid.y, h: 1, w: cellWidth(" ⚙ , ") });
    // The hit covers exactly the ` ⚙ , ` cells, counted from the left edge.
    const left = cellWidth(flat.slice(0, flat.indexOf(" ⚙ , ")));
    expect(hit.x).toBe(laid.x + left);
    expect(hitAt(laid.hits, hit.x, laid.y)?.target).toBe(GEAR);
    expect(hitAt(laid.hits, hit.x + hit.w - 1, laid.y)?.target).toBe(GEAR);
    expect(hitAt(laid.hits, hit.x + hit.w, laid.y)?.target).not.toBe(GEAR);
    expect(hitAt(laid.hits, hit.x - 1, laid.y)?.target).not.toBe(GEAR);
  });

  it("draws the comma in bold accent.secondary", () => {
    const comma = top(rowMenu({ gear: true })).line.find((c) => c.text === ",")!;
    expect(comma.style).toMatchObject({ ...slotStyle("accent.secondary"), bold: true });
  });

  it("is on the selection shade while customising, on the overlay shade otherwise", () => {
    const gearCells = (m: MenuView) => {
      const line = top(m).line;
      const at = line.findIndex((c) => c.text === ",");
      expect(at).toBeGreaterThan(2);
      return line.slice(at - 2, at + 2);
    };
    expect(gearCells(rowMenu({ gear: true, customising: true })).every((c) => c.style?.backgroundColor === bgOf("bg.selection"))).toBe(true);
    expect(gearCells(rowMenu({ gear: true })).some((c) => c.style?.backgroundColor === bgOf("bg.selection"))).toBe(false);
  });

  it("is not drawn, with no target, when the menu has no gear (a pane menu)", () => {
    const { laid, line } = top(rowMenu());
    expect(text(line)).not.toContain("⚙");
    expect(laid.hits.some((h) => h.target === GEAR)).toBe(false);
  });

  it("widens the menu to hold the title and the gear, and gives way to the title when the terminal is too narrow for both", () => {
    const title = "watch-rota vimes/night-shift on the night watch rota";
    const m = rowMenu({ gear: true, title, items: [row("pull", "p", "", 0)] });
    const bare = layoutMenu({ ...m, gear: false }, 80, 1, 30);
    const wide = top(m).laid;
    expect(wide.w).toBeGreaterThan(bare.w);
    expect(wide.hits.some((h) => h.target === GEAR)).toBe(true);
    // 60 columns hold the title (54 cells and the corners) but not the gear after it.
    const { laid, line } = top(m, 60);
    expect(text(line)).toContain(title);
    expect(laid.w).toBe(60);
    expect(text(line)).not.toContain("⚙");
    expect(laid.hits.some((h) => h.target === GEAR)).toBe(false);
  });

  it("draws at both sizes with the same shape", () => {
    for (const width of [80, 160]) {
      const { laid, line } = top(rowMenu({ gear: true }), width);
      expect(cellWidth(text(line))).toBe(laid.w);
      expect(laid.hits.filter((h) => h.target === GEAR)).toHaveLength(1);
    }
  });
});
