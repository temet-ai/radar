import { afterEach, describe, expect, it } from "vitest";
import { applyHostLook, applyLook, currentLook, defaultLook, type Look } from "../../src/text/look.js";
import { focusRamp, frameRuns, perimeterIndex, perimeterLength, rampColor, wordmarkRuns, type FocusRamp } from "../../src/text/frame.js";

const original = currentLook();
afterEach(() => applyLook(original));
const ramp: FocusRamp = [[0, 0, 0], [255, 0, 0], [255, 255, 255]];
const look = (tier: Look["tier"] = "truecolor"): Look => ({
  ...defaultLook("off"), tier,
  slots: { ...defaultLook("off").slots, "glow.purple": "#000000", "accent.tertiary": "#ff0000", "glow.cyan": "#ffffff" },
});

describe("the static focus ramp contract", () => {
  it("uses integer phase and half-up RGB channel rounding", () => {
    expect(Array.from({ length: 6 }, (_, i) => rampColor(ramp, i, 6))).toEqual(["#000000", "#660000", "#cc0000", "#ff3333", "#ff9999", "#ffffff"]);
    expect(rampColor([[0, 0, 0], [1, 1, 1], [2, 2, 2]], 1, 5)).toBe("#010101");
    expect(rampColor(ramp, -3, 5)).toBe("#000000");
    expect(rampColor(ramp, 999, 5)).toBe("#ffffff");
    expect(rampColor(ramp, 0, 1)).toBe("#000000");
  });

  it("retains the host tier and never infers truecolor from RGB-looking slots", () => {
    for (const tier of ["truecolor", "256", "16", "mono"] as const) {
      applyHostLook({ tier, slots: { ...look().slots }, icons: {}, icon_width: 0 });
      expect(currentLook().tier).toBe(tier);
      expect(!!focusRamp()).toBe(tier === "truecolor");
    }
  });

  it("uses resolved override slots and fails closed if one is missing or malformed", () => {
    applyLook(look());
    expect(focusRamp()).toEqual(ramp);
    applyLook({ ...look(), slots: { ...look().slots, "glow.cyan": "ansi256(1)" } });
    expect(focusRamp()).toBeUndefined();
    const { "glow.purple": unused, ...slots } = look().slots;
    applyLook({ ...look(), slots });
    expect(focusRamp()).toBeUndefined();
    void unused;
  });

  it("walks every border coordinate clockwise once, with deterministic collapsed rectangles", () => {
    const points = [[0,0],[1,0],[2,0],[3,0],[3,1],[3,2],[2,2],[1,2],[0,2],[0,1]];
    expect(perimeterLength(4, 3)).toBe(10);
    expect(points.map(([x, y]) => perimeterIndex(4, 3, x!, y!))).toEqual([0,1,2,3,4,5,6,7,8,9]);
    expect(perimeterIndex(4, 3, 1, 1)).toBeUndefined();
    expect(perimeterLength(0, 4)).toBe(0);
    expect(perimeterIndex(0, 4, 0, 0)).toBeUndefined();
    expect([0,1,2].map((x) => perimeterIndex(3, 1, x, 0))).toEqual([0,1,2]);
    expect([0,1,2].map((y) => perimeterIndex(1, 3, 0, y))).toEqual([0,1,2]);
    expect(perimeterLength(1, 1)).toBe(1);
  });

  it("does not compress title gaps or change non-foreground style fields", () => {
    const base = { color: "#123456", backgroundColor: "#654321", bold: true };
    const runs = frameRuns("─╮", 12, 5, 10, 0, base, ramp);
    expect(runs.map((r) => r.style.color)).toEqual([rampColor(ramp, 10, 30), rampColor(ramp, 11, 30)]);
    expect(runs.every((r) => r.style.backgroundColor === base.backgroundColor && r.style.bold)).toBe(true);
    expect(frameRuns("─╮", 12, 5, 10, 0, base)).toEqual([{ text: "─╮", style: base }]);
  });

  it("samples wordmark graphemes at starting display cells, excluding padding", () => {
    applyLook(look());
    const base = { color: "#123456", bold: true };
    const runs = wordmarkRuns("a界e\u0301", base, " ", " ");
    expect(runs.map((r) => r.text)).toEqual([" ", "a", "界", "e\u0301", " "]);
    expect(runs.map((r) => r.style.color)).toEqual([base.color, rampColor(ramp, 0, 4), rampColor(ramp, 1, 4), rampColor(ramp, 3, 4), base.color]);
    expect(runs.every((r) => r.style.bold)).toBe(true);
  });

  it.each(["256", "16", "mono"] as const)("preserves the exact fallback run at %s", (tier) => {
    applyLook(look(tier));
    const base = { color: "#123456", bold: true };
    expect(wordmarkRuns("radar", base, " ", " ")).toEqual([{ text: " radar ", style: base }]);
  });
});
