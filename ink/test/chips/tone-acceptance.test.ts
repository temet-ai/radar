import { afterEach, describe, expect, it } from "vitest";
import { chipGround, chipRuns, chipText } from "../../src/text/chips.js";
import { applyLook, currentLook, defaultLook } from "../../src/text/look.js";
import { lineCells } from "../../src/text/cells.js";
import type { Seg } from "../../src/nodes/view.js";

// Explicit widget tone examples, not application-state mapping tests. GitLab's
// merged state text uses muted, while its configurable merged look is tertiary.
const named = [
  ["passed", "good", "status.success", "#50fa7b", "#233a37", false],
  ["needs review", "warn", "status.warning", "#f1fa8c", "#3c3a3a", true],
  ["failed", "bad", "status.error", "#ff6363", "#3f2233", true],
  ["running", "info", "status.info", "#82aaff", "#2b2e4c", false],
  ["merged", "muted", "text.muted", "#6272a4", "#26253e", false],
] as const;

const original = currentLook();
afterEach(() => applyLook(original));

describe("chip tone acceptance (#30)", () => {
  it.each(named)("%s/%s has square padding and an independently specified RGB tint", (text, ink, _slot, fg, bg, bold) => {
    applyLook(defaultLook("off"));
    const seg: Seg = { text, ink, chip: true };
    const runs = chipRuns(seg);
    expect(runs).toHaveLength(1);
    expect(chipText(seg)).toBe(` ${text} `);
    const cells = lineCells([seg]);
    expect(cells).toHaveLength(text.length + 2);
    for (const cell of cells) {
      expect(cell.s.color).toBe(fg);
      expect(cell.s.backgroundColor).toBe(bg);
      expect(Boolean(cell.s.bold)).toBe(bold);
      expect(cell.w).toBe(1);
    }
  });

  it.each([
    ["plain", "#f8f8f2", "#3e3a4a", false],
    ["accent", "#e135ff", "#3a1b4c", true],
  ] as const)("also preserves the %s tone", (ink, color, backgroundColor, bold) => {
    applyLook(defaultLook("off"));
    const run = chipRuns({ text: "label", ink, chip: true })[0]!;
    expect(run.style.color).toBe(color);
    expect(run.style.backgroundColor).toBe(backgroundColor);
    expect(Boolean(run.style.bold)).toBe(bold);
  });

  it.each([
    ["256", "ansi256(235)", ["ansi256(84)", "ansi256(228)", "ansi256(203)", "ansi256(111)", "ansi256(61)"]],
    ["16", "blackBright", ["greenBright", "yellowBright", "redBright", "blueBright", "blackBright"]],
    ["mono", "", ["", "", "", "", ""]],
  ] as const)("%s keeps its flat host overlay, words and modifiers", (tier, overlay, foregrounds) => {
    const look = defaultLook("off");
    // HostLook's real wire spellings (ipc/look.rs), never RGB stand-ins for
    // limited-colour capabilities. Only the slots these chips read are needed.
    applyLook({ ...look, tier, slots: { "bg.overlay": overlay, ...Object.fromEntries(named.map((n, i) => [n[2], foregrounds[i]!])) } });
    for (const [i, [text, ink, , , , bold]] of named.entries()) {
      const runs = chipRuns({ text, ink, chip: true });
      expect(runs[0]!.text).toBe(` ${text} `);
      expect(runs[0]!.style.color).toBe(foregrounds[i] || undefined);
      expect(runs[0]!.style.backgroundColor).toBe(overlay || undefined);
      expect(Boolean(runs[0]!.style.bold)).toBe(bold);
    }
  });

  it("uses supported slot overrides rather than the built-in palette", () => {
    const look = defaultLook("off");
    applyLook({ ...look, slots: { ...look.slots, "bg.overlay": "#102030", "status.success": "#F010E0" } });
    const [run] = chipRuns({ text: "passed", ink: "good", chip: true });
    expect(run!.style.color).toBe("#F010E0");
    // 84% (16,32,48) + 16% (240,16,224), rounded: (52,29,76).
    expect(run!.style.backgroundColor).toBe("#341d4c");
  });

  it("falls back if either colour is missing or not full RGB", () => {
    applyLook(defaultLook("off"));
    for (const fg of [undefined, "", "ansi256(84)", "greenBright", "#fff", "rgb(80,250,123)"])
      expect(chipGround(fg), `foreground ${fg}`).toBe("#1a162a");
    for (const overlay of ["", "ansi256(235)", "blackBright", "#123"]) {
      const look = defaultLook("off");
      applyLook({ ...look, slots: { ...look.slots, "bg.overlay": overlay } });
      expect(chipGround("#50fa7b"), `overlay ${overlay}`).toBe(overlay || undefined);
    }
  });

  it("keeps the icon's own foreground on the words' ground, including both padding cells", () => {
    applyLook(defaultLook("unicode"));
    const seg: Seg = { text: "failed", ink: "bad", icon: "thread", chip: true };
    const runs = chipRuns(seg);
    expect(runs.map((r) => r.text)).toEqual([" ", "❝ ", "failed "]);
    expect(runs.map((r) => r.style.color)).toEqual(["#ff6363", "#80ffea", "#ff6363"]);
    expect(runs.map((r) => r.style.backgroundColor)).toEqual(["#3f2233", "#3f2233", "#3f2233"]);
    expect(lineCells([seg]).map((c) => c.ch).join("")).toBe(" ❝ failed ");
    applyLook(defaultLook("off"));
    expect(chipText(seg)).toBe(" failed ");
    expect(chipRuns(seg)).toHaveLength(1);
  });
});
