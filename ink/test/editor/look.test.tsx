import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3"; // keep the cells' styles
});
import React from "react";
import { afterEach, describe, expect, it } from "vitest";
import { InputField } from "../../src/editor/InputField.js";
import { EditorOverlay, editorOverlayHeight } from "../../src/editor/EditorOverlay.js";
import { applyLook, bgOf, defaultLook, slotStyle } from "../../src/text/look.js";
import { styledFrameAt } from "../nodes/helpers.js";
import { parseAnsi, type TCell } from "../nodes/ansi.js";

// the reference's input::render_field: the whole row on bg.overlay, the text in bold text.emphasis, the caret a cell
// of text.emphasis REVERSED over the character it is before (a space after the end), the text scrolled so the
// caret and one cell for it stay in view. EditorView's panel is ui::card::panel (focused) over bg.surface.
const rowOf = (el: React.ReactElement, width: number, rows = 24): TCell[][] =>
  parseAnsi(styledFrameAt(el, width, rows));
const field = (text: string, caret: number | null, width = 30) =>
  rowOf(<InputField text={text} caret={caret} width={width} />, width)[0]!;
const str = (cells: TCell[]) => cells.map((c) => c.ch).join("");

afterEach(() => applyLook(defaultLook()));

describe("InputField is the reference's render_field", () => {
  it("paints the whole row on bg.overlay, text bold in text.emphasis, caret reversed", () => {
    const cells = field("hello world", 5);
    const overlay = bgOf("bg.overlay");
    const emph = slotStyle("text.emphasis").color;
    expect(cells).toHaveLength(30);
    expect(cells.every((c) => c.bg === overlay)).toBe(true);
    expect(str(cells).trimEnd()).toBe("hello world");
    cells.slice(0, 11).forEach((c, i) => {
      if (i === 5) return;
      expect(c.fg, `cell ${i}`).toBe(emph);
      expect(c.bold, `cell ${i}`).toBe(true);
      expect(c.inverse, `cell ${i}`).toBeFalsy();
    });
    expect(cells[5]).toMatchObject({ ch: " ", fg: emph, inverse: true });
    expect(cells.filter((c) => c.inverse)).toHaveLength(1);
  });

  it("shows the caret as a reversed space after the end", () => {
    const cells = field("abc", 3);
    expect(cells[3]).toMatchObject({ ch: " ", inverse: true });
    expect(cells.filter((c) => c.inverse)).toHaveLength(1);
  });

  it("draws no caret when the field is not focused (caret null)", () => {
    expect(field("abc", null).some((c) => c.inverse)).toBe(false);
  });

  it("scrolls like render_field: offset = caret + 1 - width, the caret keeps its cell", () => {
    const text = "0123456789abcdefghij"; // 20 chars
    const cells = field(text, 20, 10);
    expect(str(cells)).toBe("bcdefghij "); // offset 11
    expect(cells[9]).toMatchObject({ inverse: true });
    const mid = field(text, 4, 10);
    expect(str(mid)).toBe("0123456789"); // caret near the start: no scroll
    expect(mid[4]).toMatchObject({ ch: "4", inverse: true });
  });

  it("takes every colour from the host's look", () => {
    const look = defaultLook();
    applyLook({ ...look, slots: { ...look.slots, "bg.overlay": "#010203", "text.emphasis": "#a1b2c3" } });
    const cells = field("hi", 1);
    expect(cells.every((c) => c.bg === "#010203")).toBe(true);
    expect(cells[0]!.fg).toBe("#a1b2c3");
  });
});

describe("the capturing editor is draw_overlay's panel", () => {
  const data = { title: "reply", lines: ["first", "second"], line: "hello wor", caret: 9 };
  for (const [w, h] of [[80, 18], [160, 30]] as const) {
    it(`is a focused rounded panel with its title, the lines, then the field at ${w}x${h}`, () => {
      const height = editorOverlayHeight(data.lines.length, h);
      expect(height).toBe(6); // lines + 4, at least 5
      const rows = rowOf(<EditorOverlay data={data} width={w} areaHeight={h} />, w, height);
      expect(rows).toHaveLength(height);
      const accent = slotStyle("accent.primary").color;
      expect(str(rows[0]!)).toBe(`╭ reply ${"─".repeat(w - 9)}╮`); // ╭ + " reply " (7) + rule + ╮ = w cells
      expect(rows[0]![0]).toMatchObject({ fg: accent, bg: bgOf("bg.surface") });
      expect(rows[0]![2]).toMatchObject({ fg: slotStyle("text.emphasis").color, bold: true });
      expect(str(rows[height - 1]!)).toBe(`╰${"─".repeat(w - 2)}╯`);
      // inside: one column of padding, then the earlier lines in text.primary on bg.surface
      expect(str(rows[1]!).slice(0, 8)).toBe("│ first ");
      expect(rows[1]![2]).toMatchObject({ fg: slotStyle("text.primary").color, bg: bgOf("bg.surface") });
      expect(str(rows[2]!).slice(0, 9)).toBe("│ second ");
      // the field row: the line being typed on bg.overlay with the caret after it
      const f = rows[3]!;
      expect(str(f).slice(2, 11)).toBe("hello wor");
      expect(f[2]!.bg).toBe(bgOf("bg.overlay"));
      expect(f[11]).toMatchObject({ ch: " ", inverse: true });
    });
  }

  it("is as tall as its lines plus four, up to two thirds of the area, and shows the last lines", () => {
    expect(editorOverlayHeight(0, 24)).toBe(5);
    expect(editorOverlayHeight(30, 24)).toBe(16);
    expect(editorOverlayHeight(30, 3)).toBe(3);
    const lines = Array.from({ length: 30 }, (_, i) => `line ${i}`);
    const h = editorOverlayHeight(lines.length, 24);
    const rows = rowOf(<EditorOverlay data={{ title: "t", lines, line: "x", caret: 1 }} width={40} areaHeight={24} />, 40, h);
    const text = rows.map(str).join("\n");
    expect(text).toContain("line 29");
    expect(text).not.toContain("line 0 ");
    expect(str(rows[h - 2]!)).toContain("x"); // the field is the last row inside
  });
});
