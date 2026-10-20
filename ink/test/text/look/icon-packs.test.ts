import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3";
});
import { afterEach, describe, expect, it } from "vitest";
import { PACK } from "../../../src/text/generated/pack.js";
import { lineCells, widthOfCells } from "../../../src/text/cells.js";
import { SLOTS } from "../../../src/text/generated/pack.js";
import { applyHostLook, applyLook, defaultLook, slotStyle, specStyle } from "../../../src/text/look.js";

// Icons come from the look the host sends (ipc/look.rs): each icon's glyph in the pack in use and the style it is drawn
// in, and the cells a glyph takes. Ink draws exactly those and picks no glyph or colour of its own. This holds the
// icon packs (nerd, unicode, emoji, off) and the colour slots on Ink's own output; no recorded picture is needed.
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

describe("every icon of the pack, in every pack", () => {
  const icons = Object.entries(PACK);
  it("the pack has icons to check", () => expect(icons.length).toBeGreaterThan(20));
  for (const pack of ["unicode", "nerd", "emoji"] as const) {
    it(`${pack}: each lead icon draws the pack's glyph, then one gap, in the icon's own style`, () => {
      applyLook(defaultLook(pack));
      const bad: string[] = [];
      for (const [name, icon] of icons) {
        const text = [{ icon: name }, { text: "w" }] as never;
        const row = lineCells(text);
        const glyph = icon[pack];
        if (!glyph) continue;
        if (shown(text) !== `${glyph} w`) bad.push(`${name}: ${JSON.stringify(shown(text))} for ${JSON.stringify(`${glyph} w`)}`);
        const want = specStyle(icon.style);
        if (row[0]!.s.color !== want.color || !!row[0]!.s.bold !== !!want.bold) bad.push(`${name}: style ${JSON.stringify(row[0]!.s)} for ${JSON.stringify(want)}`);
      }
      expect(bad).toEqual([]);
    });
  }
  it("off: no icon draws a glyph", () => {
    applyLook(defaultLook("off"));
    expect(icons.filter(([name]) => shown([{ icon: name }, { text: "w" }] as never) !== "w").map(([name]) => name)).toEqual([]);
  });
  it("a glyph takes the cells the look says (1 for unicode and nerd, 2 for emoji)", () => {
    for (const [pack, cells] of [["unicode", 1], ["nerd", 1], ["emoji", 2]] as const) {
      applyLook(defaultLook(pack));
      expect(defaultLook(pack).iconWidth).toBe(cells);
    }
  });
});

describe("colour slots", () => {
  const inkSlots = {
    primary: "text.primary", secondary: "text.secondary", muted: "text.muted", emphasis: "text.emphasis",
    accent: "accent.primary", accent_muted: "accent.muted", accent_secondary: "accent.secondary",
    accent_tertiary: "accent.tertiary", good: "status.success", warn: "status.warning", bad: "status.error",
    info: "status.info", number: "data.number", added: "diff.added", removed: "diff.removed",
  } as const;

  it("every theme slot of the default look is the colour the host would send, as #rrggbb", () => {
    applyLook(defaultLook());
    expect(Object.keys(SLOTS).length).toBeGreaterThan(30);
    expect(Object.entries(SLOTS).filter(([slot, hex]) => slotStyle(slot).color !== hex || !/^#[0-9a-f]{6}$/.test(hex))).toEqual([]);
  });

  it("each Ink name draws in its slot's colour, and only the colour the look gives it", () => {
    const base = defaultLook();
    // every slot gets a colour of its own, so a name drawn in another slot's colour cannot pass
    const distinct = Object.fromEntries(Object.keys(base.slots).map((slot, i) => [slot, `#${(0x101010 + i * 0x010203).toString(16).padStart(6, "0")}`]));
    applyLook({ ...base, slots: distinct });
    const bad: string[] = [];
    for (const [ink, slot] of Object.entries(inkSlots)) {
      const cell = lineCells([{ text: "x", ink }] as never)[0]!;
      if (cell.s.color !== distinct[slot]) bad.push(`${ink}: ${cell.s.color} for ${slot} ${distinct[slot]}`);
    }
    expect(bad).toEqual([]);
    expect(new Set(Object.values(inkSlots).map((slot) => distinct[slot])).size).toBe(Object.keys(inkSlots).length);
  });

  it("the mono tier gives no colour: a slot is the terminal's own", () => {
    const base = defaultLook();
    applyLook({ ...base, tier: "mono", slots: Object.fromEntries(Object.keys(base.slots).map((slot) => [slot, ""])) });
    expect(Object.keys(base.slots).filter((slot) => slotStyle(slot).color !== undefined)).toEqual([]);
    expect(lineCells([{ text: "x", ink: "bad" }] as never)[0]!.s.color).toBeUndefined();
  });

  it("an unknown slot is text.primary", () => {
    applyLook(defaultLook());
    expect(slotStyle("no.such.slot").color).toBe(SLOTS["text.primary"]);
  });
});
