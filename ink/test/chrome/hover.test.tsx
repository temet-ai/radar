import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3";
});
import { describe, expect, it } from "vitest";
import { layoutNav } from "../../src/chrome/layout.js";
import { booksChrome, settingsNav } from "./fixture.js";
import type { Hit } from "../../src/chrome/types.js";
import { InputController, type InputView } from "../../src/input/controller.js";

// The pointer's area is painted over the frame (app/hover.ts, test/app/hover.test.tsx); the controller says which area,
// as host.rs hover_at picks it.
describe("the settings navigation", () => {
  it("has a target for every row to be lit by", () => {
    const rows = layoutNav(settingsNav as never, 1, 2, 22, 30).rows;
    expect(rows.every((r) => r.target.startsWith("chrome:"))).toBe(true);
  });
});

const controller = (hits: () => Hit[], over: Partial<InputView> = {}) => {
  const lit: Array<Hit | null> = [];
  const view: InputView = { menu: () => null, capturing: () => false, route: () => ({}) as never, hits, ...over };
  const c = new InputController(view, () => {}, { select() {} } as never, undefined, (h) => lit.push(h));
  return { c, lit, targets: () => lit.map((h) => h?.target ?? null) };
};

describe("a redraw that moves a row under a still pointer", () => {
  it("lights the new row without the pointer moving (shell.rs hover asks again each frame)", () => {
    let hits = [{ x: 0, y: 5, w: 20, h: 1, target: "row:a" }];
    const { c, targets } = controller(() => hits);
    c.mouseEvent({ kind: "motion", x: 3, y: 5 } as never);
    hits = [{ x: 0, y: 5, w: 20, h: 1, target: "row:b" }]; // the list scrolled under the mouse
    c.refreshHover();
    hits = [];
    c.refreshHover();
    expect(targets()).toEqual(["row:a", "row:b", null]);
  });
});

describe("the area under the pointer (host.rs hover_at)", () => {
  it("is the hit's row the pointer is on, not the whole of a taller hit", () => {
    const { c, lit } = controller(() => [{ x: 2, y: 4, w: 30, h: 3, target: "entry:1" }]);
    c.mouseEvent({ kind: "motion", x: 9, y: 5 } as never);
    expect(lit).toEqual([{ x: 2, y: 5, w: 30, h: 1, target: "entry:1" }]);
  });
  it("while text is typed, only the status line's keys", () => {
    const hits = [
      { x: 0, y: 5, w: 20, h: 1, target: "row:a" },
      { x: 0, y: 23, w: 4, h: 1, target: "chrome:key:1:0" },
    ];
    const { c, targets } = controller(() => hits, { capturing: () => true });
    c.mouseEvent({ kind: "motion", x: 3, y: 5 } as never);
    c.mouseEvent({ kind: "motion", x: 1, y: 23 } as never);
    expect(targets()).toEqual([null, "chrome:key:1:0"]);
  });
  it("never a pane's own area", () => {
    const { c, targets } = controller(() => [{ x: 0, y: 0, w: 40, h: 10, target: "pane:work" }]);
    c.mouseEvent({ kind: "motion", x: 3, y: 5 } as never);
    expect(targets()).toEqual([null]);
  });
});

import { layoutTopBar } from "../../src/chrome/layout.js";
describe("the top-bar tab rectangles (shell.rs tabs)", () => {
  it("run from x - lead to x + w + 1, the first one taking the two cells after `radar`: tabs tile with no overlap", () => {
    const hits = layoutTopBar(booksChrome({ width: 120, height: 30 }), 120).hits.filter((h) => h.target.startsWith("chrome:") && h.y === 0 && h.h === 1);
    const tabs = hits.slice(0, 3);
    const brand = ` ${booksChrome({ width: 120, height: 30 }).top_bar.brand} `.length;
    expect(tabs[0]!.x).toBe(brand);
    for (let i = 1; i < tabs.length; i++) expect(tabs[i - 1]!.x + tabs[i - 1]!.w).toBe(tabs[i]!.x);
  });
});
