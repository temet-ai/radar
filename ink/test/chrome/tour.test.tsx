import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3"; // Ink's colour library reads this when it loads: keep the cells' styles
});
import { afterEach, describe, expect, it } from "vitest";
import { mount, tick } from "../app/harness.js";
import { parseAnsi } from "../nodes/ansi.js";
import { booksChrome } from "./fixture.js";
import { layoutTour, tourLines } from "../../src/chrome/tour.js";
import type { TourCard } from "../../src/protocol/generated/TourCard.js";

/**
 * The tour's card (crates/radar-core/src/shell/tour.rs): a panel `tour · N of M` at the body's lower right, a cell in
 * from the right and from the bottom, over the app, with the kit's shadow. The expected rows and colours are what the
 * the reference renderer drew for `radar setup tour` at 160x40, stop 1, read through scripts/ink-live (text and colour).
 */
let live: Array<() => void> = [];
afterEach(() => {
  live.forEach((f) => f());
  live = [];
});

const t = (text: string, ink?: string) => [{ text, ...(ink ? { ink } : {}) }] as any;
const pad = (s: string, n: number) => s + " ".repeat(n - [...s].length);
const stop1: TourCard = {
  stop: 1,
  of: 5,
  title: "Every screen has a letter.",
  lines: [
    t("underlined in its name; press it from anywhere"),
    [],
    t(pad("R repositories  J jira          S slack         B branches", 64)),
    t(pad("I work          H history       , settings", 64)),
    [],
    t("↑ at the top of a pane reaches the bar, then ← →"),
  ],
  try_it: "try it: press J",
  keys: t("→ next   ← back   esc end the tour"),
};

// The card as the reference drew it, from its left edge (column 91 at 160 wide).
const REFERENCE = [
  "╭ tour · 1 of 5 ───────────────────────────────────────────────────╮",
  "│ Every screen has a letter.                                       │",
  "│                                                                  │",
  "│ underlined in its name; press it from anywhere                   │",
  "│                                                                  │",
  "│ R repositories  J jira          S slack         B branches       │",
  "│ I work          H history       , settings                       │",
  "│                                                                  │",
  "│ ↑ at the top of a pane reaches the bar, then ← →                 │",
  "│                                                                  │",
  "│ try it: press J                                                  │",
  "│ → next   ← back   esc end the tour                               │",
  "╰──────────────────────────────────────────────────────────────────╯",
];

describe("where the card goes (tour.rs place)", () => {
  it("is the widest line plus 4 wide and the lines plus 2 high, a cell in from the body's lower right", () => {
    expect(tourLines(stop1)).toHaveLength(11);
    const l = layoutTour(stop1, 160, 37)!;
    expect(l).toMatchObject({ w: 68, h: 13, x: 160 - 68 - 1, y: 37 - 13 - 1, title: "tour · 1 of 5" });
  });
  it("is at least 40 wide, at most the body less 2, and absent under 10 by 4", () => {
    const small: TourCard = { stop: 2, of: 5, title: "x", lines: [], keys: t("k") };
    expect(layoutTour(small, 100, 30)!.w).toBe(40);
    expect(layoutTour(stop1, 50, 30)!.w).toBe(48);
    expect(layoutTour(stop1, 80, 12)!.h).toBe(11);
    expect(layoutTour(stop1, 11, 30)).toBeNull();
    expect(layoutTour(small, 80, 4)).toBeNull();
  });
});

async function draw(width: number, height: number, tour: TourCard | undefined, over: Record<string, unknown> = {}) {
  const m = mount(width, height);
  live.push(m.stop);
  m.welcome();
  m.snapshot({ chrome: booksChrome({ width, height, ...(tour ? { tour } : {}) }), ...over }, 1);
  await tick(60);
  return parseAnsi(m.frame());
}
const text = (row: Array<{ ch: string }>) => row.map((c) => c.ch || "").join("");

describe("the card over the app", () => {
  it("draws the reference's rows at the body's lower right", async () => {
    const cells = await draw(160, 40, stop1);
    const top = cells.findIndex((r) => text(r).includes("tour · 1 of 5"));
    // The body runs from row 2 (under the sources row) to row 38; the card ends one row above its last.
    expect(top).toBe(2 + 37 - 13 - 1);
    expect(cells.slice(top, top + 13).map((r) => text(r).slice(91, 91 + 68))).toEqual(REFERENCE);
  });
  it("is on bg.surface with an accent border, the title in bold emphasis and what to try in muted", async () => {
    const cells = await draw(160, 40, stop1);
    const top = cells.findIndex((r) => text(r).includes("tour · 1 of 5"));
    const at = (dy: number, x: number) => cells[top + dy]![91 + x]!;
    expect(at(0, 0)).toMatchObject({ ch: "╭", fg: "#e135ff", bg: "#12101a" });
    expect(at(1, 2)).toMatchObject({ ch: "E", fg: "#ffffff", bold: true, bg: "#12101a" });
    expect(at(10, 2)).toMatchObject({ ch: "t", fg: "#6272a4", bg: "#12101a" });
    expect(at(5, 40)).toMatchObject({ bg: "#12101a" });
  });
  it("throws a shadow right of it and below it, dimming and never erasing", async () => {
    const without = await draw(160, 40, undefined);
    const cells = await draw(160, 40, stop1);
    const top = cells.findIndex((r) => text(r).includes("tour · 1 of 5"));
    // Right of the card from its second row down, and under it from its second column across.
    for (let dy = 1; dy <= 13; dy++) {
      const c = cells[top + dy]![91 + 68]!;
      expect(c.bg).toBeDefined();
      expect(c.bg).not.toBe(without[top + dy]?.[91 + 68]?.bg);
    }
    for (let x = 1; x < 68; x++) expect(cells[top + 13]![91 + x]!.bg).not.toBe(without[top + 13]?.[91 + x]?.bg);
    // Nothing else dims: the top-left of the screen is as without the card.
    expect(cells[3]![5]).toEqual(without[3]![5]);
    expect(cells[top]![90]).toEqual({ ch: " " });
  });
  it("draws at 80x24 too, whole, in the lower right", async () => {
    const small: TourCard = { ...stop1, lines: [t("press the letter of a screen")] };
    const cells = await draw(80, 24, small);
    const l = layoutTour(small, 80, 21)!;
    const top = cells.findIndex((r) => text(r).includes("tour · 1 of 5"));
    expect(top).toBe(2 + l.y);
    expect(text(cells[top]!).slice(l.x, l.x + 1)).toBe("╭");
    expect(text(cells[top + l.h - 1]!).slice(l.x + l.w - 1, l.x + l.w)).toBe("╯");
  });
  it("is drawn before a menu: the menu is over it and dims it with the rest", async () => {
    const menu = { id: "actions", title: "Pick", question: false, items: [{ value: "a", label: "alpha", hint: "", key: "a", target: "chrome:menu:0" }], selected: 0 };
    const withTour = await draw(160, 40, stop1, { chrome: booksChrome({ width: 160, height: 40, tour: stop1, menu } as any) });
    const top = withTour.findIndex((r) => text(r).includes("tour · 1 of 5"));
    // The card's border is no longer its own accent: the menu's scrim mixed it toward bg.base.
    expect(withTour[top]![91]!.fg).not.toBe("#e135ff");
    expect(text(withTour[top + 3]!).slice(91, 91 + 68)).toBe(REFERENCE[3]);
  });
});
