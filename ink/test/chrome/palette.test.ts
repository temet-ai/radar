import { describe, expect, it } from "vitest";
import type { Choice, MenuView } from "../../src/chrome/types.js";
import { hitAt, layoutMenu, type Cell } from "../../src/chrome/layout.js";
import { paletteLabel, paletteLines, paletteWidth, wrapPalette } from "../../src/chrome/palette.js";
import { bgOf, slotStyle } from "../../src/text/look.js";

const text = (cells: Cell[]) => cells.map((cell) => cell.text).join("");
const choice = (label: string, keys: string, index: number): Choice => ({
  label, keys, hint: "", value: `command:${index}`, target: `chrome:menu:${index}`,
});
const menu = (): MenuView => ({
  id: "palette", title: "commands", question: false, selected: 0, typed: "re", caret: 2,
  items: [choice("Refresh", "ctrl-r", 0), choice("Review records", "r", 1), choice("Restore session", "", 2)],
  palette: { matched: 3, total: 12, highlights: [[0, 1], [0, 1], [0, 1]] },
});

describe("command palette layout", () => {
  it("highlights host scalar offsets without splitting combining or emoji graphemes", () => {
    const base = slotStyle("text.primary");
    const matched = { ...slotStyle("accent.primary"), bold: true };
    const lines = paletteLabel("é界e\u0301\n🚀👩‍💻Z", [1, 3, 5, 7, 9, 999], base, matched);
    expect(lines).toHaveLength(2);
    expect(lines.flat().filter((c) => c.style === matched).map((c) => c.text).join("")).toBe("界e\u0301🚀👩‍💻Z");
    const wrapped = wrapPalette(lines, 2);
    expect(wrapped.every((line) => paletteWidth(text(line)) <= 2)).toBe(true);
    expect(wrapped.map(text).join("")).toBe("é界e\u0301🚀👩‍💻Z");
  });

  it.each([4, 8, 16, 80, 160])("keeps every Unicode grapheme and right-aligned shortcut at %i cells", (width) => {
    const item = choice("界ée\u0301\n🚀👩‍💻Z", "ctrl-r", 0);
    item.hint = "watch\n界e\u0301";
    const lines = paletteLines(item, true, [...item.label].map((_, i) => i), width, 6);
    expect(lines.every((line) => paletteWidth(text(line)) === width)).toBe(true);
    const highlighted = lines.flat().filter((c) => c.style?.color === slotStyle("accent.primary").color);
    expect(highlighted.map((c) => c.text).join("")).toBe("界ée\u0301🚀👩‍💻Z");
    expect(highlighted.every((c) => c.style?.bold && c.style?.backgroundColor === bgOf("bg.selection"))).toBe(true);
    const muted = lines.flat().filter((c) => c.style?.color === slotStyle("text.muted").color);
    expect(muted.map((c) => c.text).join("")).toBe("watch界e\u0301");
    expect(muted.every((c) => c.style?.backgroundColor === bgOf("bg.selection"))).toBe(true);
    const hintOnly = paletteLines(item, true, [], width, 6, [0, 6, 8]);
    const hintMatches = hintOnly.flat().filter((c) => c.style?.color === slotStyle("accent.primary").color);
    expect(hintMatches.map((c) => c.text).join("")).toBe("w界e\u0301");
    expect(hintMatches.every((c) => c.style?.bold && c.style?.backgroundColor === bgOf("bg.selection"))).toBe(true);
    const keys = lines.flat().filter((c) => c.style?.color === slotStyle("accent.secondary").color);
    expect(keys.map((c) => c.text.trim()).join("")).toBe("ctrl-r");
    const keyLine = width >= 12 ? lines[0]! : lines.at(-1)!;
    expect(text(keyLine).endsWith(width >= 8 ? "ctrl-r " : "-r ")).toBe(true);
  });

  it.each([80, 160])("shows query, match count, themed highlights, and aligned shortcuts at %i columns", (width) => {
    const view = menu();
    view.items[0]!.hint = "watch";
    view.palette!.hint_highlights = [[0, 1], [], []];
    const laid = layoutMenu(view, width, 1, 22);
    expect(text(laid.lines[0]!)).toContain("commands · 3 of 12");
    expect(text(laid.lines[1]!)).toContain("re");
    const rows = laid.hits.map((hit) => laid.lines[hit.y - laid.y]!);
    expect(rows.map(text).map((line) => line.slice(-8))).toEqual(["ctrl-r │", "     r │", "       │"]);
    expect(laid.hits.map((h) => h.target)).toEqual(view.items.map((i) => i.target));
    const highlighted = rows[0]!.filter((c) => c.style?.color === slotStyle("accent.primary").color && c.text !== "│");
    expect(highlighted.map((c) => c.text).join("")).toBe("Rewa");
  });

  it("keeps multiline continuations and pointer targets in order through scrolling", () => {
    const view = menu();
    view.title = "p";
    view.items[0] = choice("Watch 界界\nReview 👩‍💻 reports", "ctrl-r", 0);
    view.palette!.highlights[0] = [6, 7, 9, 16];
    const full = layoutMenu(view, 16, 1, 100);
    const targets = full.hits.map((h) => h.target);
    expect(targets.length).toBeGreaterThan(view.items.length);
    expect([...new Set(targets)]).toEqual(view.items.map((item) => item.target));
    expect(targets).toEqual([...targets].sort());
    expect(full.lines.every((line) => paletteWidth(text(line)) === full.w)).toBe(true);
    const first = full.hits[1]!;
    const scroll = first.y - full.y - 2; // top border and query are fixed
    const shown = layoutMenu({ ...view, scroll }, 16, 1, 7);
    expect(text(shown.lines[0]!)).toContain("3 of 12");
    expect(shown.hits[0]?.target).toBe(first.target);
    expect(shown.hits.map((h) => h.target)).toEqual(targets.slice(1, 1 + shown.hits.length));
    for (const hit of shown.hits) {
      expect(hitAt(shown.hits, hit.x, hit.y)?.target).toBe(hit.target);
      expect(hitAt(shown.hits, hit.x + hit.w - 1, hit.y)?.target).toBe(hit.target);
    }
  });

  it("shows zero matches without manufacturing selectable rows", () => {
    const view = menu();
    view.items = [];
    view.palette = { matched: 0, total: 12, highlights: [] };
    const laid = layoutMenu(view, 80, 1, 22);
    expect(text(laid.lines[0]!)).toContain("0 of 12");
    expect(laid.hits).toEqual([]);
  });
});
