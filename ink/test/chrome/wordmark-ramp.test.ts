import { afterEach, describe, expect, it } from "vitest";
import { layoutSetupTopBar, layoutTopBar, type Cell } from "../../src/chrome/layout.js";
import { cellWidth } from "../../src/chrome/width.js";
import { applyLook, currentLook, defaultLook, type Look } from "../../src/text/look.js";
import { booksChrome } from "./fixture.js";

const original = currentLook();
afterEach(() => applyLook(original));
const look = (tier: Look["tier"]): Look => {
  const base = defaultLook("off");
  return { ...base, tier, slots: { ...base.slots, "glow.purple": "#000000", "accent.tertiary": "#ff0000", "glow.cyan": "#ffffff" } };
};
const cells = (runs: Cell[]) => runs.flatMap((r) => [...r.text].flatMap((ch) => Array.from({ length: cellWidth(ch) }, (_, i) => ({ text: i ? "" : ch, ...r.style }))));
const withoutForeground = (runs: Cell[]) => cells(runs).map(({ color: _, ...c }) => c);

describe.each(["normal", "setup"] as const)("the %s top bar wordmark", (kind) => {
  const draw = (width: number) => {
    const chrome = booksChrome({ width });
    return kind === "normal" ? layoutTopBar(chrome, width) : layoutSetupTopBar(chrome, { title: "look", step: "step 1 of 5" }, width);
  };

  it("colors only the five brand cells, with the original bold and padding", () => {
    applyLook(look("truecolor"));
    const row = cells(draw(80).cells);
    expect(row.slice(1, 6).map((c) => c.text).join("")).toBe("radar");
    expect(row.slice(1, 6).map((c) => c.color)).toEqual(["#000000", "#800000", "#ff0000", "#ff8080", "#ffffff"]);
    expect(row.slice(1, 6).every((c) => c.bold && c.backgroundColor === "#1a162a")).toBe(true);
    expect(row[0]!.color).toBe("#e135ff");
  });

  it.each([8, 20, 80, 160])("preserves display geometry and every hit at width %i", (width) => {
    applyLook(look("256")); // Same resolved slots but the old solid brand path.
    const flat = draw(width);
    applyLook(look("truecolor"));
    const ramp = draw(width);
    expect(withoutForeground(ramp.cells)).toEqual(withoutForeground(flat.cells));
    expect(ramp.hits).toEqual(flat.hits);
  });

  it.each(["256", "16", "mono"] as const)("preserves the original padded brand run at %s", (tier) => {
    const base = look(tier);
    applyLook(base);
    const brand = draw(80).cells[0]!;
    expect(brand.text).toBe(kind === "normal" ? " radar " : " radar");
    expect(brand.style).toMatchObject({ color: "#e135ff", bold: true });
    for (const width of [8, 20, 80, 160]) {
      applyLook(base);
      const withStops = draw(width);
      // Missing stops force the original single-run path even if the capability gate regresses.
      applyLook({ ...base, slots: { ...base.slots, "glow.purple": "", "glow.cyan": "" } });
      expect(draw(width)).toEqual(withStops);
    }
  });
});
