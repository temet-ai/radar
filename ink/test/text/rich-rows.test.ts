import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { docRows, rowsText } from "../../src/text/richRows.js";
import { slotStyle } from "../../src/text/look.js";
import type { RichBlock } from "../../src/nodes/view.js";

// ui::rich's own layout of the same documents at several widths (ink/test/fixtures/dock/rich-layout.json, written by
// radar-core's rich_layout_reference): every line Ink lays out for the dock must be the one the reference lays out.
const ref = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../fixtures/dock/rich-layout.json");
const samples = JSON.parse(fs.readFileSync(ref, "utf8")) as Array<{ name: string; doc: RichBlock[]; widths: Record<string, string[]> }>;

describe("a document as rows of cells, as ui::rich lays it out", () => {
  it("has samples at every width", () => {
    expect(samples.length).toBeGreaterThan(1);
    for (const s of samples) expect(Object.keys(s.widths).length).toBeGreaterThanOrEqual(5);
  });
  for (const s of samples)
    for (const [w, lines] of Object.entries(s.widths))
      it(`${s.name} at ${w} cells`, () => {
        const rows = docRows(s.doc, Number(w));
        expect(rowsText(rows)).toEqual(lines);
        for (const r of rows) expect(r.reduce((n, c) => n + c.w, 0)).toBeLessThanOrEqual(Number(w));
      });

  const review = samples.find((s) => s.name === "review")!;
  const rows = docRows(review.doc, 76);
  const at = (word: string) => {
    for (const r of rows) {
      const t = r.map((c) => c.ch).join("");
      const i = t.indexOf(word);
      if (i >= 0) {
        let x = 0;
        for (const c of r) {
          if (x === i) return c.s;
          x += c.ch.length;
        }
      }
    }
    throw new Error(`no ${word}`);
  };
  it("draws each part in its slot: headings and bold emphasised, code and links in their accents", () => {
    expect(at("Review")).toMatchObject({ ...slotStyle("text.emphasis"), bold: true });
    expect(at("Verdict")).toMatchObject({ bold: true });
    expect(at("origin/main").color).toBe(slotStyle("accent.secondary").color);
    expect(at("two lines")).toMatchObject({ italic: true });
    expect(at("the plan")).toMatchObject({ color: slotStyle("accent.primary").color, underline: true });
    expect(at("(https://").color).toBe(slotStyle("text.muted").color);
    expect(at("│").color).toBe(slotStyle("accent.muted").color);
  });
  it("shows no markdown marks", () => {
    const text = rowsText(rows).join("\n");
    expect(text).not.toMatch(/\*\*|`|^#+ |^\s*[-*] /m);
  });
  it("colours code by the host's highlighting where it sent it", () => {
    const code = review.doc.find((b) => b.t === "code") as Extract<RichBlock, { t: "code" }>;
    const lines = code.text.split("\n").filter((l, i, a) => i < a.length - 1 || l !== "");
    const painted = [lines.map((l) => (l.startsWith("fn") ? [{ text: "fn", token: "keyword" }, { text: l.slice(2), token: "plain" }] : [{ text: l, token: "plain" }]))];
    const hl = docRows(review.doc, 76, painted);
    const row = hl.find((r) => r.map((c) => c.ch).join("").startsWith("  fn "))!;
    expect(row[2]!.s).toMatchObject({ color: slotStyle("accent.primary").color, bold: true });
    expect(rowsText(hl)).toEqual(rowsText(rows));
  });
});
