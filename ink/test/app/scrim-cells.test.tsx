import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3"; // Ink's colour library reads this when it loads: keep the cells' styles
});
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { mount, tick } from "./harness.js";
import { applyLook, currentLook, defaultLook } from "../../src/text/look.js";
import { parseAnsi } from "../nodes/ansi.js";

// Ratatui's styled buffer with a menu up (tests/ratatui_cells.rs, scene highlight_menu): the screen behind dims and the
// menu throws a one-cell shadow (ui::scrim). The whole App draws the same view data with the same menu, and the body's
// cells are compared colour for colour, the shadow's glyphs too.
const here = path.dirname(fileURLToPath(import.meta.url));
interface RatCell {
  ch?: string;
  fg?: string;
  bg?: string;
}

let live: Array<() => void> = [];
afterEach(() => {
  live.forEach((f) => f());
  live = [];
  applyLook(defaultLook());
});

describe("the scrim and shadow behind a menu against Ratatui's cells", () => {
  for (const size of ["80x24", "160x40"]) {
    it(`dims and shades as Ratatui does at ${size}`, async () => {
      const scene = JSON.parse(fs.readFileSync(path.resolve(here, `../cells/highlight_menu-${size}.json`), "utf8")) as {
        canvas: { fg: string; bg: string };
        colours: Record<string, { hex: string }>;
        view: { screen: any; menu: any };
        rows: RatCell[][];
      };
      applyLook(defaultLook());
      const [cols, rowsN] = size.split("x").map(Number) as [number, number];
      const chrome = JSON.parse(fs.readFileSync(path.resolve(here, `../fixtures/host_chrome_${size}.json`), "utf8"));
      const m = mount(cols, rowsN);
      live.push(m.stop);
      m.welcome();
      m.snapshot({ chrome: { ...chrome, menu: scene.view.menu }, screen: scene.view.screen, focus: "nav" }, 1);
      await tick(60);
      const got = parseAnsi(m.frame());
      const hex = (n: string | undefined, dflt: string) => {
        const name = (n ?? dflt).split("/")[0]!;
        return (scene.colours[name]?.hex ?? currentLook().slots[name])?.toLowerCase();
      };
      const ch = (c: { ch?: string } | undefined) => (c?.ch === undefined || c.ch === "" ? " " : c.ch);
      const text = (r: Array<{ ch?: string }>) => r.map(ch).join("");
      // Each renderer's menu box, found by its corners. The recorded MenuView is thinner than the menu Ratatui drew
      // (its keys and headings are left out), so the two boxes differ; the screen around them is what is compared.
      const box = (rows: Array<Array<{ ch?: string }>>) => {
        // below the panes' top border, which is also titled (the middle pane is "Everything" too)
        const title = (r: Array<{ ch?: string }>) => text(r).search(new RegExp(`(?<=.)╭ (\\S+ )?${scene.view.menu.title}`));
        const y = rows.findIndex((r, i) => i > 2 && title(r) > 0);
        const x = title(rows[y] ?? []);
        const right = text(rows[y] ?? []).indexOf("╮", x);
        const bottom = rows.findIndex((r, i) => i > y && ch(r[x]) === "╰");
        return { x, y, w: right - x + 1, h: bottom - y + 1 };
      };
      const ink = box(got);
      const rat = box(scene.rows);
      expect(ink.y, "Ink drew the menu").toBeGreaterThan(-1);
      // The host chrome's bars differ from the scene's (it has a sources row): rows are matched by the box's top edge.
      const dy = ink.y - rat.y;
      const inBox = (b: typeof ink, x: number, y: number) => x >= b.x && x < b.x + b.w && y >= b.y && y < b.y + b.h;
      const inShadow = (b: typeof ink, x: number, y: number) =>
        (x === b.x + b.w && y > b.y && y <= b.y + b.h) || (y === b.y + b.h && x > b.x && x <= b.x + b.w);
      const bad: string[] = [];
      let scrimmed = 0;
      let shaded = 0;
      for (let y = 2; y < rowsN - 1; y++) {
        const g0 = got[y + dy];
        if (!g0) continue;
        scene.rows[y]!.forEach((want, x) => {
          const g = g0[x];
          const iy = y + dy;
          if (want.ch === "" || g?.ch === "" || inBox(rat, x, y) || inBox(ink, x, iy)) return;
          // A cell under one renderer's shadow only is left out; under both, its colours must agree.
          const both = inShadow(rat, x, y) && inShadow(ink, x, iy);
          if (!both && (inShadow(rat, x, y) || inShadow(ink, x, iy))) return;
          if (ch(g) !== ch(want)) return; // the body under the boxes differs where the boxes do
          if ((g?.bg?.toLowerCase() ?? hex(undefined, scene.canvas.bg)) !== hex(want.bg, scene.canvas.bg))
            bad.push(`(${x},${y}): bg ${g?.bg} for ${hex(want.bg, scene.canvas.bg)}`);
          // ✓ and ✗ on a repo row: Ratatui draws them in the text colour, Ink in their status colour (drift older than
          // the scrim, the same without a menu).
          if (ch(want) !== " " && !"✓✗".includes(ch(want)) && g?.fg?.toLowerCase() !== hex(want.fg, scene.canvas.fg))
            bad.push(`(${x},${y}): fg ${g?.fg} for ${hex(want.fg, scene.canvas.fg)}`);
          if (both) shaded++;
          else scrimmed++;
        });
      }
      expect(bad.slice(0, 20)).toEqual([]);
      expect(scrimmed, "cells behind the menu compared").toBeGreaterThan(cols * (rowsN - 3) * 0.25);
      expect(shaded, "cells under both shadows compared").toBeGreaterThan(20);
    });
  }
});
