import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3";
});
import { afterEach, describe, expect, it } from "vitest";
import { PACK } from "../../../src/text/generated/pack.js";
import { lineCells, widthOfCells } from "../../../src/text/cells.js";
import { applyHostLook, applyLook, defaultLook, slotStyle } from "../../../src/text/look.js";

// Icons come from the look the host sends (ipc/look.rs): each icon's glyph in the pack in use and the style it is drawn
// in, and the cells a glyph takes. Ink draws exactly those and picks no glyph or colour of its own. This holds the
// icon packs without Ratatui's recorded cells (test/cells), which go once Ink is the only renderer.
afterEach(() => applyLook(defaultLook()));

const lead = [{ icon: "settings" }, { text: "Settings" }] as never;
const withWords = [{ icon: "settings", text: "settings" }] as never;
const shown = (t: never) => lineCells(t).map((c) => c.ch).join("");

describe("icon packs", () => {
  for (const [pack, glyph, cells] of [
    ["unicode", "⚙", 1],
    ["nerd", PACK.settings!.nerd, 1],
    ["emoji", "🔧", 2],
  ] as const) {
    it(`${pack}: the pack's glyph, then its gap, in the icon's own style`, () => {
      applyLook(defaultLook(pack));
      const row = lineCells(lead);
      expect(shown(lead)).toBe(`${glyph} Settings`);
      expect(row[0]!.s.color).toBe(slotStyle("text.secondary").color);
      expect(widthOfCells(row.slice(0, 1))).toBe(cells);
    });
  }

  it("off: a lead icon is nothing, and an icon with words is its words", () => {
    applyLook(defaultLook("off"));
    expect(shown(lead)).toBe("Settings");
    expect(shown(withWords)).toBe("settings");
  });

  it("the host's own glyph and style win (a user's [icons] override, a palette's colour)", () => {
    const base = defaultLook("unicode");
    applyHostLook({
      tier: "truecolor",
      slots: { ...base.slots, "accent.primary": "#e135ff" },
      icons: { ...base.icons, settings: { glyph: "S", style: "accent.primary bold" } },
      icon_width: 1,
    });
    const row = lineCells(lead);
    expect(shown(lead)).toBe("S Settings");
    expect([row[0]!.s.color, row[0]!.s.bold]).toEqual(["#e135ff", true]);
  });
});
