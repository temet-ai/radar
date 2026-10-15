import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { chipGround, chipRuns, chipText } from "../../src/text/chips.js";
import { segRunWords, textLineString } from "../../src/text/TextLine.js";
import { lineCells } from "../../src/text/cells.js";
import { inkStyle } from "../../src/text/theme.js";
import { ICONS } from "../../src/text/icons.js";
import type { Seg } from "../../src/nodes/view.js";

// ui::chips::chip_spans: ` ` + icon and gap + `words `, every cell on the chip's ground (its tone's tint, #30).
const discussed: Seg = { icon: "thread", text: "discussed · 1", ink: "muted", chip: true };
const folder: Seg = { icon: "folder", text: "local folder", ink: "plain", chip: true };
const bare: Seg = { text: "needs you", ink: "good", chip: true };

describe("labelled chips", () => {
  it("an icon chip draws its icon and its label, not the icon alone", () => {
    expect(ICONS.thread).toBeTruthy();
    expect(chipText(discussed)).toBe(` ${ICONS.thread} discussed · 1 `);
    expect(segRunWords(discussed)).toBe(` ${ICONS.thread} discussed · 1 `);
    expect(lineCells([discussed]).map((c) => c.ch).join("")).toBe(` ${ICONS.thread} discussed · 1 `);
  });

  it("every cell of a chip sits on its tone's tint; the icon keeps its own colour, the label the tone's", () => {
    for (const seg of [discussed, folder, bare]) {
      const runs = chipRuns(seg);
      const label = runs[runs.length - 1]!.style;
      for (const r of runs) expect(r.style.backgroundColor).toBe(chipGround(label.color));
    }
    // widgets::chip_ground: status.success #50fa7b 16% over bg.overlay #1a162a, as Ratatui's test has it
    expect(chipRuns(bare)[0]!.style.backgroundColor).toBe("#233a37");
    expect(chipRuns(discussed)[0]!.style.backgroundColor).not.toBe(chipRuns(bare)[0]!.style.backgroundColor);
    const [pad, icon, label] = chipRuns(discussed);
    expect(label!.style.color).toBe(pad!.style.color);
    expect(icon!.text).toBe(`${ICONS.thread} `);
  });

  it("a chip with no icon is ` words ` and the line string carries the label", () => {
    expect(chipText(bare)).toBe(" needs you ");
    expect(textLineString([{ text: "◉ you" }, { text: " " }, discussed])).toContain("discussed · 1");
  });
});

// Ratatui's own cells for the chip (captured from the real radar, Jira My tickets, ABC-1552's ' ⇆ discussed · 1 ', as
// char, fg, bg). Ink's cells for the same segment must match at every column; the icon's glyph depends on the pack in
// use, so there only its background is compared (its colour is the host look's, equal in the live capture).
describe("a chip against Ratatui's cells", () => {
  const ref = JSON.parse(
    fs.readFileSync(path.join(path.dirname(fileURLToPath(import.meta.url)), "../fixtures/chips/jira-discussed.ratatui.json"), "utf8"),
  ) as { cells: Array<[string, string | null, string | null]> };
  const hex = (c: string | undefined) => (c ? c.toLowerCase() : null);

  it("draws the same character in every cell, and the fixture's shades as the look's slots", () => {
    // The capture is in the operator's theme; the unit test's look is the built-in one. Same slots, other RGB: the
    // tone's tint stands for the capture's chip background (all its cells) and the tone's colour for the label's.
    const cells = lineCells([discussed]);
    expect(cells.length).toBe(ref.cells.length);
    const tone = inkStyle("muted").color;
    const shade = chipGround(tone);
    cells.forEach((c, i) => {
      const [ch, fg, bg] = ref.cells[i]!;
      if (i !== 1) expect(c.ch, `cell ${i} char`).toBe(ch);
      expect(bg, "the capture's chip cells all sit on one shade").toBe(ref.cells[0]![2]);
      expect(c.s.backgroundColor, `cell ${i} bg`).toBe(shade);
      if (i > 1 && ch !== " ") {
        expect(fg, "the capture's label has one colour").toBe(ref.cells[3]![1]);
        expect(c.s.color, `cell ${i} (${c.ch}) fg`).toBe(tone);
      }
    });
  });
});
