import { describe, expect, it } from "vitest";
import { lineCells, wrapCells } from "../../src/text/cells.js";
import { bgOf, slotStyle } from "../../src/text/look.js";
import { chipGround, chipWordStyle } from "../../src/text/chips.js";
import type { Text } from "../../src/nodes/view.js";

// GitHub repo detail, the line under the title (radar-git's Point::Line): a labelled chip, then plain chips with no icon
// (the path, ` · on`). the reference draws every chip as ` words ` on bg.overlay (widgets::chip), so the shade runs on from the
// '⌂ local folder' chip under '~/projects/radar' to the end of ` · on `; live capture, cells 26;22;42 under the path
// where Ink left the pane fill 18;16;26.
const line: Text = [
  { chip: true, icon: "local", ink: "muted", text: "local folder  " },
  { chip: true, ink: "plain", text: "~/projects/radar" },
  { chip: true, ink: "muted", text: " · on" },
];

describe("a document line of chips", () => {
  // Each chip's ground is its tone's tint (widgets::chip_ground, #30): the muted ones on one, the plain path on another.
  const grounds = line.flatMap((seg) => lineCells([seg]).map(() => chipGround(chipWordStyle({ ...seg, icon: null }).color)));

  it("every cell of every chip, with an icon or without, is on its chip's ground", () => {
    expect(bgOf("bg.overlay")).toBeTruthy();
    const cells = lineCells(line);
    expect(cells.map((c) => c.ch).join("")).toContain("~/projects/radar");
    expect(cells.map((c) => c.s.backgroundColor)).toEqual(grounds);
    expect(new Set(grounds).size).toBe(2);
  });

  it("the ground is kept when the line is wrapped into rows", () => {
    const wrapped = wrapCells(lineCells(line), 20).flat().filter((c) => c.ch !== " " || c.s.backgroundColor);
    for (const c of wrapped) expect(grounds, `(${c.ch})`).toContain(c.s.backgroundColor);
  });
});

describe("a chip's words", () => {
  it("a plain chip is text.primary on the overlay shade (widgets::tone Plain), not the terminal's own text", () => {
    const [, path] = [0, line[1]!];
    const cells = lineCells([path]);
    const primary = slotStyle("text.primary").color;
    expect(primary).toBeTruthy();
    for (const c of cells) expect(c.s.color, `(${c.ch})`).toBe(primary);
  });
});

describe("a chip's words", () => {
  it("a plain chip is text.primary on the overlay shade (widgets::tone Plain), not the terminal's own text", () => {
    const primary = slotStyle("text.primary").color;
    expect(primary).toBeTruthy();
    for (const c of lineCells([line[1]!])) expect(c.s.color, `(${c.ch})`).toBe(primary);
  });
});

describe("a chip's tone weight (widgets::tone)", () => {
  const bold = (ink: "accent" | "warn" | "bad" | "good" | "info" | "muted" | "plain") =>
    lineCells([{ chip: true, ink, text: "x" }]).every((c) => c.s.bold === true);
  it.each(["accent", "warn", "bad"] as const)("a %s chip's words are bold", (ink) => expect(bold(ink)).toBe(true));
  it.each(["good", "info", "muted", "plain"] as const)("a %s chip's words are not bold", (ink) => expect(bold(ink)).toBe(false));
  it("an icon chip's words are bold too, its icon is not", () => {
    const cells = lineCells([{ chip: true, icon: "local", ink: "accent", text: "go" }]);
    expect(cells.filter((c) => c.ch === "g" || c.ch === "o").every((c) => c.s.bold === true)).toBe(true);
  });
});
