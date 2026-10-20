import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3"; // Ink's colour library reads this when it loads: keep the cells' styles
});
import chalk from "chalk";
import { afterEach, describe, expect, it } from "vitest";
import { paintScrim } from "../../src/app/scrim.js";
import { applyLook, currentLook, defaultLook } from "../../src/text/look.js";
import { parseAnsi } from "../nodes/ansi.js";

// ui::scrim (crates/radar-core/src/ui/scrim.rs): behind a menu every cell dims, and the menu's box throws a one-cell shadow
// right of it and below it that keeps every glyph. The numbers are the reference's (bg.base #0a0812, text.primary #f8f8f2).
afterEach(() => applyLook(defaultLook()));

const SURFACE = "#12101a";
const MUTED = "#6272a4";
// 8 wide, 5 rows of tree lines on bg.surface.
const lines = Array.from({ length: 5 }, () => chalk.bgHex(SURFACE).hex(MUTED)("││││││││")).join("\n");

describe("the scrim and shadow over a finished frame", () => {
  it("in full colour every cell outside the layer mixes toward bg.base, bold off, and the layer is as drawn", () => {
    const frame = chalk.bold("└─ api") + "\n  " + chalk.hex("#f8f8f2")("menu");
    const cells = parseAnsi(paintScrim(frame, 10, 2, { x: 2, y: 1, w: 5, h: 1 }));
    expect(cells[0]![0]).toMatchObject({ ch: "└", fg: "#757477", bg: "#0a0812" });
    expect(cells[0]![0]!.bold).toBeFalsy();
    expect(cells[1]![3]).toMatchObject({ ch: "e", fg: "#f8f8f2" });
    expect(cells[1]![3]!.bg).toBeUndefined();
  });

  it("the shadow is the column right of the layer and the row below it, the corner once, and keeps its lines", () => {
    const out = parseAnsi(paintScrim(lines, 8, 5, { x: 1, y: 1, w: 3, h: 2 }));
    const shaded: Array<[number, number]> = [];
    for (let y = 0; y < 5; y++)
      for (let x = 0; x < 8; x++) {
        expect(out[y]![x]!.ch).toBe("│");
        const inLayer = x >= 1 && x < 4 && y >= 1 && y < 3;
        if (inLayer) expect(out[y]![x]!.bg).toBe(SURFACE);
        else if (out[y]![x]!.bg !== "#0f0d17") shaded.push([x, y]); // not the scrim's ground
      }
    expect(shaded).toEqual([[4, 2], [2, 3], [3, 3], [4, 3]]);
    expect(new Set(shaded.map(([x, y]) => out[y]![x]!.bg))).toEqual(new Set(["#060509"]));
  });

  it("the shadow's numbers: ground 60% toward black, text 45% toward bg.base, over the scrim", () => {
    // A one-cell layer at the corner: the shadow falls on (1, 1) only, which the scrim has had.
    const frame = chalk.bgHex(SURFACE).hex(MUTED)("││") + "\n" + chalk.bgHex(SURFACE).hex(MUTED)("││");
    const out = parseAnsi(paintScrim(frame, 2, 2, { x: 0, y: 0, w: 1, h: 1 }));
    // scrim: muted → #323854 (55% to base), surface → #0f0d17 (43% to base); then the shadow over it.
    expect(out[0]![1]).toMatchObject({ fg: "#323854", bg: "#0f0d17" });
    expect(out[1]![1]).toMatchObject({ ch: "│", fg: "#202236", bg: "#060509" });
  });

  it("clips the shadow of a layer flush with the bottom and right edges without growing the frame", () => {
    const original = parseAnsi(lines);
    const out = parseAnsi(paintScrim(lines, 8, 5, { x: 5, y: 3, w: 3, h: 2 }));
    expect(out).toHaveLength(5);
    for (let y = 0; y < 5; y++) {
      expect(out[y]).toHaveLength(8);
      for (let x = 0; x < 8; x++) {
        expect(out[y]![x]!.ch).toBe(original[y]![x]!.ch);
        if (x >= 5 && y >= 3) expect(out[y]![x]).toEqual(original[y]![x]);
        else expect(out[y]![x]).toEqual({ ch: "│", fg: "#323854", bg: "#0f0d17" });
      }
    }
  });

  it("with fewer colours the scrim is dim and the shadow's ground black; with none, dim only", () => {
    for (const [tier, base] of [["256", "ansi256(233)"], ["mono", ""]] as const) {
      applyLook({ ...currentLook(), slots: Object.fromEntries(Object.keys(defaultLook().slots).map((k) => [k, tier === "mono" ? "" : base])) });
      const frame = "repo\n" + chalk.bold("ab") + "\n    ";
      const out = parseAnsi(paintScrim(frame, 4, 3, { x: 0, y: 1, w: 2, h: 1 }));
      expect(out[0]![1]).toMatchObject({ ch: "e", dim: true });
      expect(out[1]![0]!.dim).toBeFalsy();
      expect(out[2]![2]!.dim).toBe(true);
      expect(out[2]![2]!.bg).toBe(tier === "mono" ? undefined : "ansi16(40)"); // SGR 40, black
    }
  });
});
