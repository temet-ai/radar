// The maintainer's screenshot: Ink's Work was stacked plain text; the reference draws every pane as a bordered, titled
// panel. Every section the view carries is a panel here, at both sizes, in both recorded Work views.
import React from "react";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { render } from "./render.js";
import { WorkArea } from "../../src/workarea/WorkArea.js";
import { sectionsOf, type ScreenLike, type ViewNode } from "../../src/workarea/model.js";
import { renderNode } from "../../src/nodes/index.js";
import { panels, referenceTitles } from "./panels.js";



const dir = "../../../crates/radar-workbench/tests/fixtures/";
const unwrap = (n: ViewNode): ViewNode =>
  n.node === "extension" && String(n.capability).startsWith("work.") && n.fallback ? (n.fallback as ViewNode) : n;

describe.each(["work_view.json", "work_view_pending.json"])("%s", (name) => {
  const screen = JSON.parse(readFileSync(fileURLToPath(new URL(dir + name, import.meta.url)), "utf8")) as ScreenLike;
  for (const [columns, rows] of [
    [80, 24],
    [160, 40],
  ] as const) {
    it(`${columns}x${rows}: each panel has a border and its title, and the frame fits the terminal`, () => {
      const r = render(
        <WorkArea
          screen={screen}
          capabilities={{ columns, rows, plain_text: false }}
          renderNode={(node, c) => renderNode(unwrap(node as ViewNode) as never, { width: c.columns, focused: c.focused } as never)}
        />,
      );
      const f = r.lastFrame();
      r.unmount();
      const lines = f.split("\n");
      for (const l of lines) expect([...l].length).toBeLessThanOrEqual(columns);
      const drawn = panels(lines);
      // Each panel is closed: a top border (╭ … ╮ on one row) with a bottom border (╰ … ╯) of the same columns below it.
      for (const p of drawn) {
        expect(p.bottom, `panel ${p.title} closes`).toMatch(/^╰─*╯$/);
        expect([...p.bottom].length, `panel ${p.title} closes under its own columns`).toBe(p.end - p.start + 1);
      }
      const hint = lines.find((l) => l.startsWith("+ ")) ?? "";
      for (const s of sectionsOf(screen)) {
        // Under 84 columns the reference shows the focused pane alone (its recorded fit); see panels-match-reference.test.tsx.
        if (s.kind === "other" && !referenceTitles(name.replace(".json", ""), columns, rows).includes(s.title)) continue;
        const title = s.status && s.kind !== "other" ? `${s.title} (${s.status})` : s.title;
        const mine = drawn.find((p) => p.title === title);
        // The reference draws no M11 panel under 84 columns; only a pending question is added there.
        if (s.kind !== "other" && !s.pending && columns < 84) {
          expect(mine, `${title} is not drawn where the reference draws only the focused pane`).toBeUndefined();
          continue;
        }
        // Every shown section is a panel with its full title, or (a short terminal) is named on the line that says it has no room.
        if (!mine) expect(hint, `${title} has no panel and is not named as left out`).toContain(s.title);
        if (s.pending) expect(mine, "a pending question always has its panel").toBeDefined();
      }
      expect(lines.length).toBeLessThanOrEqual(rows);
    });
  }
});
