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
import type { MenuView } from "../../src/chrome/types.js";
import { adoptInkFrame, updatingGoldens, type SceneFile } from "../support/goldens.js";

// The styled buffer with a menu up (ink/test/cells, scene highlight_menu; first drawn by the reference, now Ink's golden,
// rewritten from Ink by INK_UPDATE_GOLDENS=1): the screen behind dims and the menu throws a one-cell shadow. The whole
// App draws the same view data with the same menu, and the body's cells are compared colour for colour, the shadow's
// glyphs too.
const here = path.dirname(fileURLToPath(import.meta.url));
interface RatCell {
  ch?: string;
  fg?: string;
  bg?: string;
  mods?: string[];
}

interface MenuScene {
  canvas: { fg: string; bg: string };
  colours: Record<string, { hex: string }>;
  view: { screen: any; menu: MenuView };
  rows: RatCell[][];
}

/** Locate the menu independently in both actual painted frames, below the pane titles. */
function menuBox(rows: Array<Array<{ ch?: string }>>, title: string) {
  const text = (r: Array<{ ch?: string }>) => r.map((c) => c.ch || " ").join("");
  const start = (r: Array<{ ch?: string }>) => text(r).search(new RegExp(`(?<=.)╭ (\\S+ )?${title}`));
  const y = rows.findIndex((r, i) => i > 2 && start(r) > 0);
  const x = start(rows[y] ?? []);
  const right = text(rows[y] ?? []).indexOf("╮", x);
  const bottom = rows.findIndex((r, i) => i > y && r[x]?.ch === "╰");
  return { x, y, w: right - x + 1, h: bottom - y + 1 };
}

let live: Array<() => void> = [];
afterEach(() => {
  live.forEach((f) => f());
  live = [];
  applyLook(defaultLook());
});

describe("the scrim and shadow behind a menu against the reference's cells", () => {
  for (const size of ["80x24", "160x40"]) {
    it(`dims and shades as the reference does at ${size}`, async () => {
      const file = path.resolve(here, `../cells/highlight_menu-${size}.json`);
      const scene = JSON.parse(fs.readFileSync(file, "utf8")) as {
        canvas: { fg: string; bg: string };
        colours: Record<string, { hex: string }>;
        view: { screen: any; menu: any };
        rows: RatCell[][];
      };
      applyLook(defaultLook("off")); // the scene was captured with icons off
      const [cols, rowsN] = size.split("x").map(Number) as [number, number];
      const chrome = JSON.parse(fs.readFileSync(path.resolve(here, `../fixtures/host_chrome_${size}.json`), "utf8"));
      const m = mount(cols, rowsN);
      live.push(m.stop);
      m.welcome();
      // This native scene has its source-tab row; the generic host_chrome fixture is now the Work screen without it.
      // Preserve the scene's body height so its menu and the shadow land over the same underlying cells.
      const sources = ["GitLab", "GitHub", "Local workspaces"].map((label, i) => ({ label, active: i === 0, target: `chrome:source:${i}` }));
      m.snapshot({ chrome: { ...chrome, sources, menu: scene.view.menu }, screen: scene.view.screen, focus: "nav" }, 1);
      await tick(60);
      const got = parseAnsi(m.frame());
      if (updatingGoldens()) adoptInkFrame(file, scene as unknown as SceneFile, got);
      const hex = (n: string | undefined, dflt: string) => {
        const name = (n ?? dflt).split("/")[0]!;
        return (scene.colours[name]?.hex ?? currentLook().slots[name])?.toLowerCase();
      };
      const ch = (c: { ch?: string } | undefined) => (c?.ch === undefined || c.ch === "" ? " " : c.ch);
      const text = (r: Array<{ ch?: string }>) => r.map(ch).join("");
      // The scrim check stays scoped to the surrounding screen; the separate test below
      // compares the menu itself, using the complete host-authored MenuView in the capture.
      const ink = menuBox(got, scene.view.menu.title);
      const rat = menuBox(scene.rows, scene.view.menu.title);
      expect(ink.y, "Ink drew the menu").toBeGreaterThan(-1);
      // Align the actual pane origins, not the centered menus: adding one menu row can move its top edge without
      // moving the body underneath. Matching source rows must keep the original scrim/shadow coverage below.
      const bodyTop = (rows: Array<Array<{ ch?: string }>>) => rows.findIndex((r) => text(r).startsWith("╭"));
      const inkTop = bodyTop(got), ratTop = bodyTop(scene.rows);
      expect(inkTop, "Ink drew the pane borders").toBeGreaterThan(-1);
      expect(ratTop, "the reference recorded the pane borders").toBeGreaterThan(-1);
      expect(inkTop, "the scene's source row preserves its body geometry").toBe(ratTop);
      const dy = inkTop - ratTop;
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
          // ✓ and ✗ on a repo row: the reference draws them in the text colour, Ink in their status colour (drift older than
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
    it(`draws the native menu's mnemonic words, styles and geometry at ${size}`, async () => {
      const file = path.resolve(here, `../cells/highlight_menu-${size}.json`);
      const scene = JSON.parse(fs.readFileSync(file, "utf8")) as MenuScene;
      expect(scene.view.menu.items.some((item) => item.mnemonic_at != null), "capture embeds the complete host MenuView, including mnemonics").toBe(true);
      applyLook(defaultLook("off")); // match the native capture, including its complete menu payload
      const [cols, rowsN] = size.split("x").map(Number) as [number, number];
      const chrome = JSON.parse(fs.readFileSync(path.resolve(here, `../fixtures/host_chrome_${size}.json`), "utf8"));
      const sources = ["GitLab", "GitHub", "Local workspaces"].map((label, i) => ({ label, active: i === 0, target: `chrome:source:${i}` }));
      const m = mount(cols, rowsN);
      live.push(m.stop);
      m.welcome();
      m.snapshot({ chrome: { ...chrome, sources, menu: scene.view.menu }, screen: scene.view.screen, focus: "nav" }, 1);
      await tick(60);
      const got = parseAnsi(m.frame());
      if (updatingGoldens()) adoptInkFrame(file, scene as unknown as SceneFile, got);
      const ink = menuBox(got, scene.view.menu.title), rat =menuBox(scene.rows, scene.view.menu.title);
      expect(rat.x).toBeGreaterThanOrEqual(0);
      expect(rat.w).toBeGreaterThan(2);
      expect(rat.h).toBeGreaterThan(2);
      expect(ink, "the actual menu border, placement and height match native paint").toEqual(rat);
      const colour = (value: string | undefined, fallback: string) => {
        const name = (value ?? fallback).split("/")[0]!;
        return (name.startsWith("#") ? name : scene.colours[name]?.hex ?? currentLook().slots[name])?.toLowerCase();
      };
      const shiftY = scene.rows.findIndex((row) => row.map((c) => c.ch ?? " ").join("").includes("shift recent activity"));
      expect(shiftY, "the native menu includes its unselected shifted mnemonic").toBeGreaterThanOrEqual(rat.y);
      const shiftX = scene.rows[shiftY]!.map((c) => c.ch ?? " ").join("").indexOf("shift recent activity");
      expect(got[shiftY]!.slice(shiftX, shiftX + 6).map((c) => c.ch).join("")).toBe("shift ");
      expect(got[shiftY]!.slice(shiftX, shiftX + 6).map((c) => c.bg?.toLowerCase()), "every Shift-prefix cell inherits the overlay background").toEqual(Array(6).fill(colour("bg.overlay", scene.canvas.bg)));
      const modifiers = ["bold", "dim", "italic", "underline", "inverse", "strike"] as const;
      const bad: Array<{ at: [number, number]; got: unknown; want: unknown }> = [];
      let underlined = 0;
      for (let y = rat.y; y < rat.y + rat.h; y++) {
        for (let x = rat.x; x < rat.x + rat.w; x++) {
          const want = scene.rows[y]![x]!;
          const actual = got[y]?.[x];
          const w = {
            ch: want.ch ?? " ",
            fg: (want.ch ?? " ") === " " ? undefined : colour(want.fg, scene.canvas.fg),
            bg: colour(want.bg, scene.canvas.bg),
            mods: modifiers.filter((mod) => want.mods?.includes(mod === "inverse" ? "reversed" : mod)),
          };
          const g = {
            ch: actual?.ch ?? " ",
            fg: (actual?.ch ?? " ") === " " ? undefined : colour(actual?.fg, scene.canvas.fg),
            bg: colour(actual?.bg, scene.canvas.bg),
            mods: modifiers.filter((mod) => actual?.[mod]),
          };
          if (want.mods?.includes("underline")) underlined++;
          if (JSON.stringify(g) !== JSON.stringify(w)) bad.push({ at: [x, y], got: g, want: w });
        }
      }
      expect(underlined, "native mnemonic underline cells are actually compared").toBeGreaterThan(0);
      expect(bad.slice(0, 30), `${bad.length} native menu cell differences`).toEqual([]);
    });
  }
});
