import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3";
});
import { describe, expect, it } from "vitest";
import { renderNode, type Node } from "../../src/nodes/index.js";
import { bgOf } from "../../src/text/look.js";
import { styledFrameAt } from "../nodes/helpers.js";
import { parseAnsi } from "../nodes/ansi.js";

// docs/GUIDE.md 2.10: "Editing is normal text editing ... visible cursor where you type". 2.12: no chevrons or
// +/- marker glyphs; the field is shown by its background, not by a marker. A row being edited in a settings list or
// tree is a field: one caret, on the characters typed, however long the text and whatever the width.
const t = (text: string) => [{ text }] as never;
const SIZES = [[80, 24], [160, 40]] as const;
const CHEVRONS = /[▶▸▾›]|\[[+-]\]/;

const list = (text: string, caret: number): Node =>
  ({ node: "list", selected: 1, editing: { row: 1, at: 0, text, caret }, rows: [{ target: "a", text: t("alpha") }, { target: "b", text: t("beta") }, { target: "c", text: t("gamma") }] }) as never;
const tree = (text: string, caret: number): Node =>
  ({
    node: "tree", columns: [], selected: ["b"], open: [], editing: { path: ["b"], at: 0, text, caret },
    rows: [{ id: "a", name: t("alpha") }, { id: "b", name: t("beta") }, { id: "c", name: t("gamma") }],
  }) as never;

for (const [name, make] of [["list", list], ["tree", tree]] as const) {
  describe(`a ${name} row being edited is a field (GUIDE 2.10, 2.12)`, () => {
    for (const [w, h] of SIZES) {
      const rowsOf = (text: string, caret: number) => parseAnsi(styledFrameAt(renderNode(make(text, caret), { width: w - 4, focused: true }), w, h));
      for (const [text, caret] of [["", 0], ["abc", 0], ["abc", 2], ["abc", 3], ["x".repeat(w * 2), w * 2], ["x".repeat(w * 2), 5]] as const) {
        it(`shows one caret and the typed words at ${w}x${h}: ${text.length} chars, caret ${caret}`, () => {
          const rows = rowsOf(text, caret);
          const carets = rows.flatMap((r) => r.filter((c) => c.inverse));
          expect(carets).toHaveLength(1);
          const edited = rows.find((r) => r.some((c) => c.inverse))!;
          expect(edited.length).toBeLessThanOrEqual(w - 4);
          const shown = edited.map((c) => c.ch).join("");
          if (caret > 0 && caret <= w - 6) expect(shown).toContain(text.slice(0, Math.min(caret, 3)));
          expect(edited.some((c) => c.bg === bgOf("bg.overlay"))).toBe(true);
          expect(rows.map((r) => r.map((c) => c.ch).join("")).join("\n")).not.toMatch(CHEVRONS);
        });
      }
    }
  });
}

// GUIDE 2.18 "nothing visual is hard-coded": the editor draws from the host's look, so its sources name no colour.
describe("the editor chooses no colour or glyph of its own (GUIDE 2.18)", () => {
  it("src/editor holds no hex colour, ANSI colour name or chevron", async () => {
    const { readdirSync, readFileSync } = await import("node:fs");
    const dir = new URL("../../src/editor/", import.meta.url);
    for (const f of readdirSync(dir).filter((n) => /\.tsx?$/.test(n))) {
      const code = readFileSync(new URL(f, dir), "utf8").replace(/\/\*[\s\S]*?\*\/|\/\/.*$/gm, "");
      expect(code, f).not.toMatch(/#[0-9a-fA-F]{6}\b/);
      expect(code, f).not.toMatch(/(color|backgroundColor)="?(red|green|blue|yellow|cyan|magenta|white|black|gray|grey)/);
      expect(code, f).not.toMatch(CHEVRONS);
    }
  });
});
