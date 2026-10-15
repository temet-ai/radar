import React from "react";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { render } from "./render.js";
import { WorkArea } from "../../src/workarea/WorkArea.js";
import { sectionsOf, type ScreenLike, type ViewNode } from "../../src/workarea/model.js";
import { renderNode } from "../../src/nodes/index.js";
import { textWords } from "../../src/text/TextLine.js";
import { ratatuiTitles } from "./panels.js";

// Defect 7 (T13's survey): Work cut its explanatory text with "…" where Ratatui wraps it, and dropped the
// panes' footers ("Groups · page 1"). The class: every recorded Work view, at both sizes, through the same
// node renderer the app passes in.
const dir = "../../../crates/radar-workbench/tests/fixtures/";
const views = ["work_view.json", "work_view_pending.json"].map((n) => ({
  name: n,
  screen: JSON.parse(readFileSync(fileURLToPath(new URL(dir + n, import.meta.url)), "utf8")) as ScreenLike,
}));

const draw = (screen: ScreenLike, columns: number, rows: number) => {
  const r = render(
    <WorkArea
      screen={screen}
      capabilities={{ columns, rows, plain_text: false }}
      renderNode={(node: ViewNode, c) => renderNode(unwrap(node) as never, { width: c.columns, focused: c.focused } as never)}
    />,
  );
  const f = r.lastFrame() ?? "";
  r.unmount();
  return f;
};
const unwrap = (n: ViewNode): ViewNode =>
  n.node === "extension" && String(n.capability).startsWith("work.") && n.fallback ? (n.fallback as ViewNode) : n;

const squash = (s: string) => s.replace(/\s+/g, " ");

describe.each(views)("$name", ({ name, screen }) => {
  // A pane Ratatui does not paint at this size (its Narrow layout shows the focused pane alone) is not asked of Ink.
  const shown = (s: { kind: string; title: string }, columns: number, rows: number) =>
    s.kind !== "other" || ratatuiTitles(name.replace(".json", ""), columns, rows).includes(s.title);
  for (const [columns, rows] of [
    [80, 24],
    [160, 40],
  ] as const) {
    it(`${columns}x${rows}: no document line is cut with an ellipsis`, () => {
      const drawn = draw(screen, columns, rows);
      const f = squash(drawn);
      for (const s of sectionsOf(screen)) {
        const body = unwrap(s.body);
        if (body.node !== "document" || !shown(s, columns, rows)) continue;
        // Ratatui draws no M11 panel under 84 columns (its Narrow layout is the focused pane alone); Ink adds only a pending question there.
        if (s.kind !== "other" && !s.pending && columns < 84) continue;
        // A panel a short terminal has no room for is named on the line that says so; every panel that is drawn is whole.
        if (s.kind !== "other" && drawn.split("\n").some((l) => l.startsWith("+ ") && l.includes(s.title))) continue;
        for (const line of (body.lines as never[][]) ?? []) {
          const words = squash(textWords(line as never)).trim();
          if (!words) continue;
          // Side by side, wrapped lines interleave with the neighbouring column: the tail is what a cut loses.
          const tail = words.split(" ").pop()!;
          expect(f, `section ${s.key}: tail of "${words}"`).toContain(tail);
          if (columns < 100) expect(f, `section ${s.key}`).toContain(words);
        }
      }
    });

    it(`${columns}x${rows}: every pane head and footer the view carries is drawn`, () => {
      const f = draw(screen, columns, rows);
      for (const [i, p] of (screen.panes ?? []).entries()) {
        if (!shown(sectionsOf(screen)[i]!, columns, rows)) continue;
        for (const part of [p.head, p.footer] as (never[] | undefined)[]) {
          if (part && part.length) expect(f).toContain(textWords(part as never));
        }
      }
    });
  }

  it("the Groups footer is there", () => {
    expect(draw(screen, 160, 40)).toContain("Groups · page 1");
  });
});
