// Renderer contract: actual native Buffer cells against Ink's production cell
// conversion for the same serialized Segs and host-resolved Look. No host frame.
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { applyHostLook, applyLook, currentLook, type HostLook } from "../../src/text/look.js";
import { lineCells } from "../../src/text/cells.js";
import type { Text } from "../../src/nodes/view.js";

type Run = [text: string, fg: string, bg: string, modifiers: string];
type Capture = { format: number; cases: Array<{ name: string; look: HostLook; rows: Array<{ text: Text; width: number; runs: Run[] }> }> };
const capture = JSON.parse(fs.readFileSync(fileURLToPath(new URL("../fixtures/chips/tone-acceptance.json", import.meta.url)), "utf8")) as Capture;
const original = currentLook();
afterEach(() => applyLook(original));

describe("native chip tone cells", () => {
  it("keeps the reviewed six-case, five-tone matrix complete", () => {
    expect(capture.format).toBe(1);
    expect(capture.cases.map((c) => c.name)).toEqual(["truecolor-off", "truecolor-icons", "256-icons", "16-icons", "mono-icons", "custom-rgb-icons"]);
    for (const c of capture.cases)
      expect(c.rows.map((r) => [r.text[1]!.text, r.text[1]!.ink])).toEqual([
        ["passed", "good"], ["needs review", "warn"], ["failed", "bad"], ["running", "info"], ["merged", "muted"],
      ]);
  });

  it.each(capture.cases.map((c) => [c.name, c] as const))("matches every glyph, foreground, ground and modifier: %s", (_name, c) => {
    applyHostLook(c.look);
    for (const row of c.rows) {
      const expected = row.runs.flatMap(([text, fg, bg, modifiers]) => Array.from(text, (ch) => [ch, fg, bg, modifiers]));
      const cells = lineCells(row.text);
      expect(cells.every((cell) => cell.w === 1), "this compact capture uses only single-cell glyphs").toBe(true);
      expect(expected).toHaveLength(row.width);
      expect(cells).toHaveLength(row.width);
      expect(cells.map((cell) => [cell.ch, cell.s.color ?? "", cell.s.backgroundColor ?? "", (["bold", "dim", "italic", "underline", "inverse", "strike"] as const).filter((m) => cell.s[m]).join(" ")])).toEqual(expected);
      expect(expected[0]).toEqual(["|", "", "", ""]);
      expect(expected[expected.length - 1]).toEqual(["|", "", "", ""]);
    }
  });
});
