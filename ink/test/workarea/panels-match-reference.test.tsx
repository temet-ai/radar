// Ink's Work against the recorded reference panels: crates/radar-workbench/src/snapshots/*work_empty_<w>x<h>.snap is what the
// reference shell paints for a Work view with no items (radar-workbench's test `work_empty`). The recorded view
// work_view_pending.json carries the same three panes (and, besides, a question the M11 layout puts first), so
// for each size every panel the reference draws must be drawn by Ink with the same top border and title, the same text
// in it, and the same borders around it. What the reference does not draw at that size (its Narrow layout shows the
// focused pane alone, its recorded fit) is not asked of Ink either; the M11 panels are Ink's addition.
import React from "react";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { render } from "./render.js";
import { WorkArea } from "../../src/workarea/WorkArea.js";
import type { ScreenLike, ViewNode } from "../../src/workarea/model.js";
import { renderNode } from "../../src/nodes/index.js";
import { applyLook, defaultLook } from "../../src/text/look.js";
import { panels } from "./panels.js";
import type { PaneJump } from "../../src/protocol/generated/PaneJump.js";

const read = (rel: string) =>
  readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");
const FIX = "../../../crates/radar-workbench/tests/fixtures/";
const load = (name: string) =>
  JSON.parse(read(`${FIX}${name}.json`)) as ScreenLike;
const unwrap = (n: ViewNode): ViewNode =>
  n.node === "extension" &&
  String(n.capability).startsWith("work.") &&
  n.fallback
    ? (n.fallback as ViewNode)
    : n;

/** The painted text of an insta snapshot: header and status lines taken off. */
const reference = (view: string, size: string): string[] => {
  // written by radar-workbench's tests/work_area.rs from the reference shell (radar_core::shell::to_text) for the same view, drift-checked there
  const lines = read(`${FIX}${view}_reference_${size}.txt`)
    .replace(/\n$/, "")
    .split("\n");
  return lines.slice(1, -1);
};

for (const view of ["work_view", "work_view_pending"]) {
  for (const [columns, rows] of [
    [80, 24],
    [160, 40],
  ] as const) {
    describe(`${view} ${columns}x${rows}`, () => {
      // The reference recording had icons off; compare like with like.
      applyLook(defaultLook("off"));
      const screen = load(view);
      const ref = panels(reference(view, `${columns}x${rows}`));
      const r = render(
        <WorkArea
          screen={screen}
          paneJumps={JSON.parse(read(`${FIX}${view}_pane_jumps_${columns}x${rows}.json`)) as PaneJump[]}
          capabilities={{ columns, rows, plain_text: false }}
          renderNode={(node, c) =>
            renderNode(
              unwrap(node as ViewNode) as never,
              { width: c.columns, focused: c.focused } as never,
            )
          }
        />,
      );
      const ink = panels(r.lastFrame().split("\n"));
      r.unmount();

      it("the reference draws at least one panel (the reference is read)", () => {
        expect(ref.length).toBeGreaterThan(0);
      });

      // A menu or question drawn over the screen is the host's chrome (the app's, not the work area's); what it covers is not asked.
      for (const [i, p] of ref.entries()) {
        if (p.overlay) continue;
        it(`panel ${i + 1} "${p.title}": same top border and full title, same text, same bottom border`, () => {
          const mine = ink.find((q) => q.top === p.top && q.start === p.start);
          expect(
            mine,
            `an Ink panel with the reference's top border ${p.top}`,
          ).toBeDefined();
          if (!p.covered)
            expect(mine!.bottom, "closed under the same columns").toBe(
              p.bottom,
            );
          // Each recording mints new item ids; the Rust test blanks them in the text it writes.
          const ids = (t: string[]) =>
            t.map((l) =>
              l.replace(
                /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/g,
                "<id>",
              ),
            );
          expect(ids(mine!.text.slice(0, p.text.length))).toEqual(ids(p.text));
        });
      }

      it("the panels Ink adds are the M11 ones; the rest are exactly the reference's, in order", () => {
        const own = ink.filter(
          (q) => !/^(map|timeline|tabs|question|actions)\b/.test(q.title),
        );
        expect(own.map((q) => q.title)).toEqual(
          ref.filter((p) => !p.overlay).map((p) => p.title),
        );
      });
    });
  }
}
