import { describe, expect, it } from "vitest";
import { layoutNav, layoutSources } from "../../src/chrome/layout.js";
import { selectedStyle } from "../../src/nodes/tree.js";
import { bgOf, slotStyle } from "../../src/text/look.js";
import { cellWidth } from "../../src/chrome/width.js";
import { booksChrome } from "./fixture.js";
import type { SettingsNav } from "../../src/chrome/types.js";

describe.each([80, 160])("sources row at %i columns (shell.rs source_tabs)", (w) => {
  it("is on bg.overlay across the row; the active source on bg.selection in bold emphasis; the others secondary", () => {
    const s = layoutSources(booksChrome(), w, 1)!;
    expect(s.cells.every((c) => c.style?.backgroundColor === bgOf("bg.overlay") || c.style?.backgroundColor === bgOf("bg.selection"))).toBe(true);
    expect(cellWidth(s.cells.map((c) => c.text).join(""))).toBe(w);
    const on = s.cells.find((c) => c.text === "  GitLab  ")!;
    expect(on.style).toMatchObject({ backgroundColor: bgOf("bg.selection"), bold: true, color: slotStyle("text.emphasis").color });
    const off = s.cells.find((c) => c.text === "  GitHub  ")!;
    expect(off.style).toMatchObject({ backgroundColor: bgOf("bg.overlay"), color: slotStyle("text.secondary").color });
    expect(s.hits.map((h) => [h.x, h.w])).toEqual([[0, 10], [10, 10]]);
  });

  it("when the sources have the keys the active one is bold bg.base on accent.primary", () => {
    const s = layoutSources(booksChrome({ keys_at: "sources" }), w, 1)!;
    const on = s.cells.find((c) => c.text === "  GitLab  ")!;
    expect(on.style).toMatchObject({ bold: true, backgroundColor: slotStyle("accent.primary").color, color: slotStyle("bg.base").color });
  });
});

describe("settings navigation rows (shell.rs settings: a tree, no chevrons)", () => {
  const nav: SettingsNav = {
    groups: [
      { id: "a", name: "General", open: true, target: "chrome:group:a", sections: [{ id: "s1", name: "views", icon: "settings", target: "chrome:section:s1" }, { id: "s2", name: "keys", target: "chrome:section:s2" }] },
      { id: "b", name: "Git", open: false, target: "chrome:group:b", sections: [] },
    ],
    selected: "chrome:section:s1",
    section_title: "s",
    detail_title: "d",
  } as unknown as SettingsNav;
  const rows = layoutNav(nav, 1, 2, 20, 10).rows;

  it("groups are bold emphasis, a folded one has a muted ⋯, sections have muted tree lines and primary names", () => {
    expect(rows[0]!.cells[0]).toMatchObject({ text: "General", style: { bold: true, color: slotStyle("text.emphasis").color } });
    const folded = rows[3]!;
    expect(folded.cells.map((c) => c.text).join("")).toBe("Git ⋯");
    expect(folded.cells[1]!.style?.color).toBe(slotStyle("text.muted").color);
    const keys = rows[2]!.cells;
    expect(keys.find((c) => c.text === "└─ ")!.style?.color).toBe(slotStyle("text.muted").color);
    expect(keys.find((c) => c.text === "keys")!.style?.color).toBe(slotStyle("text.primary").color);
  });

  it("a section's icon is drawn before its name, in the pack's style", () => {
    const cells = rows[1]!.cells;
    const at = cells.findIndex((c) => c.text === "views");
    expect(cells[at - 1]!.text.endsWith(" ")).toBe(true);
    expect(cells[at - 1]!.text.trim()).not.toBe("");
    expect(rows[2]!.cells.every((c) => c.text.trim() === "" || /[─└├]|keys/.test(c.text))).toBe(true);
  });

  it("the selected row is marked for the selection shade (selectedStyle), not by a glyph", () => {
    expect(rows.filter((r) => r.selected).map((r) => r.target)).toEqual(["chrome:section:s1"]);
    expect(selectedStyle(true).backgroundColor).toBe(bgOf("bg.selection"));
    for (const r of rows) expect(r.text).not.toMatch(/[▶▸▾›]|\[[+-]\]/);
  });
});
