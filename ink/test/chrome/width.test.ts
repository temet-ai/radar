import { describe, expect, it } from "vitest";
import { cellWidth } from "../../src/chrome/width.js";

// the reference lays out by unicode-width, Ink's own measure is string-width: the symbols with emoji presentation are two cells
// in both. The setup look step's emoji row (📦 🌿 🔀 ✅ ❌ 🐛 📗 👤 📢 🔧) came out two cells too long with ✅ and ❌ at one.
describe("cellWidth", () => {
  it("counts the emoji of the look step's preview two cells each, as string-width and unicode-width do", () => {
    for (const g of ["📦", "🌿", "🔀", "✅", "❌", "🐛", "📗", "👤", "📢", "🔧", "⚪", "🟢", "⭐", "⚡", "⌚"]) expect([g, cellWidth(g)]).toEqual([g, 2]);
    expect(cellWidth("📦 🌿 🔀 ✅ ❌ 🐛 📗 👤 📢 🔧")).toBe(29);
  });
  it("leaves the text symbols of the plain pack at one cell", () => {
    for (const g of ["▤", "⎇", "⇄", "✓", "✗", "✱", "▣", "☺", "#", "⚙", "│", "─"]) expect([g, cellWidth(g)]).toEqual([g, 1]);
  });
});
