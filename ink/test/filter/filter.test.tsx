import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3"; // Ink's colour library reads this when it loads: keep the cells' styles
});
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { mount, tick } from "../app/harness.js";
import { parseAnsi, type TCell } from "../nodes/ansi.js";
import { applyLook, currentLook, defaultLook } from "../../src/text/look.js";
import { bottomFieldRow, fieldRows, FIELD_ROWS } from "../../src/chrome/filter.js";

/**
 * T25: the `/` filter box. In Ratatui a pane with a `FieldBar` in `pane.fields` draws, under its title, the mark
 * ` / `, the typed text on bg.overlay with the caret cell reversed, a hint row and a blank row. Ink drew none.
 * ink/test/cells/highlight_typing-filter-<w>x<h>.json is Ratatui's styled buffer for the repos screen after `/`
 * `g` `g` (written by tests/ratatui_cells.rs); the whole App draws the same view data and the three rows of the
 * field are compared cell for cell (character, colours, bold, reversed).
 */
const here = path.dirname(fileURLToPath(import.meta.url));
const cellsDir = path.resolve(here, "../cells");
const fixtures = path.resolve(here, "../fixtures");

let live: Array<() => void> = [];
afterEach(() => {
  live.forEach((f) => f());
  live = [];
});

interface RatCell {
  ch?: string;
  fg?: string;
  bg?: string;
  mods?: string[];
}
interface Scene {
  canvas: { fg: string; bg: string };
  colours: Record<string, { hex: string }>;
  view: { screen: any };
  rows: RatCell[][];
}

const hexOf = (scene: Scene, name: string | undefined, canvas: string) => {
  const n = name ?? canvas;
  return (scene.colours[n.split("/")[0]!]?.hex ?? currentLook().slots[n.split("/")[0]!] ?? n).toLowerCase();
};

const rowText = (cells: Array<{ ch?: string }>) => cells.map((c) => (c.ch === undefined || c.ch === "" ? " " : c.ch)).join("");

describe("the '/' filter box", () => {
  for (const size of ["80x24", "160x40"]) {
    const file = path.join(cellsDir, `highlight_typing-filter-${size}.json`);
    it(`Ink's field rows equal Ratatui's cells at ${size}`, async () => {
      expect(fs.existsSync(file), `${file} is written by tests/ratatui_cells.rs (scene highlight_typing-filter)`).toBe(true);
      const scene = JSON.parse(fs.readFileSync(file, "utf8")) as Scene;
      applyLook(defaultLook("off"));
      const [cols, rows] = size.split("x").map(Number) as [number, number];
      const chrome = JSON.parse(fs.readFileSync(path.join(fixtures, `host_chrome_${size}.json`), "utf8"));
      const m = mount(cols, rows);
      live.push(m.stop);
      m.welcome();
      m.snapshot({ chrome, screen: scene.view.screen, focus: "middle" }, 1);
      await tick(60);
      const ink = parseAnsi(m.frame());

      // Ratatui's row of the bar: the one with the mark ` / ` and the typed text.
      const at = scene.rows.findIndex((r) => rowText(r).includes(" / gg"));
      expect(at, "Ratatui drew the typed filter").toBeGreaterThan(-1);
      const inkAt = ink.findIndex((r) => rowText(r).includes(" / gg"));
      expect(inkAt, "Ink drew the typed filter").toBeGreaterThan(-1);

      const canvasBg = hexOf(scene, scene.canvas.bg, scene.canvas.bg);
      const bad: string[] = [];
      for (let dy = 0; dy < FIELD_ROWS; dy++) {
        const y = at + dy;
        // the host chrome's own rows above the pane (the part's source tab row) differ in the fixture chrome, so the
        // Ink row is found by the field's own position, not by the screen row number
        const iy = inkAt + dy;
        scene.rows[y]!.forEach((c, x) => {
          const i: TCell | undefined = ink[iy]?.[x];
          if (c.ch === "" || i?.ch === "") return;
          const ch = c.ch ?? " ";
          const blank = ch.trim() === "";
          const mods = c.mods ?? [];
          const want = {
            bg: hexOf(scene, c.bg, scene.canvas.bg),
            fg: hexOf(scene, c.fg, scene.canvas.fg),
            bold: mods.includes("bold"),
            rev: mods.includes("reversed"),
          };
          const got = { bg: i?.bg?.toLowerCase(), fg: i?.fg?.toLowerCase(), bold: !!i?.bold, rev: !!i?.inverse };
          const why: string[] = [];
          if ((i?.ch ?? " ") !== ch && !(blank && (i?.ch ?? " ").trim() === "")) why.push(`char ${JSON.stringify(i?.ch)} for ${JSON.stringify(ch)}`);
          if (got.rev !== want.rev) why.push(`reversed ${got.rev} for ${want.rev}`);
          if ((got.bg ?? canvasBg) !== want.bg) why.push(`bg ${got.bg} for ${want.bg}`);
          if (!blank) {
            if (got.fg !== undefined && got.fg !== want.fg) why.push(`fg ${got.fg} for ${want.fg}`);
            if (got.bold !== want.bold) why.push(`bold ${got.bold} for ${want.bold}`);
          }
          if (why.length) bad.push(`row ${y} col ${x}: ${why.join(", ")}`);
        });
      }
      expect(bad).toEqual([]);
    });
  }
});

describe("the field rows of a pane (chrome/filter.tsx)", () => {
  const hint = [{ text: "no filter", ink: "muted" }];
  it("a bar, its hint, a blank row: three rows of the pane's width", () => {
    const rows = fieldRows({ mark: " / ", text: "gg", caret: 2, hint }, 30, 10);
    expect(rows).toHaveLength(FIELD_ROWS);
    for (const r of rows) expect(r.reduce((n, c) => n + c.w, 0)).toBe(30);
    expect(rows[0]!.map((c) => c.ch).join("").startsWith(" / gg")).toBe(true);
  });
  it("only the rows the pane has room for", () => {
    expect(fieldRows({ mark: " / ", text: "", caret: 0, hint }, 30, 1)).toHaveLength(1);
    expect(fieldRows({ mark: " / ", text: "", caret: 0, hint }, 30, 2)).toHaveLength(2);
    expect(fieldRows({ mark: " / ", text: "", caret: 0, hint }, 30, 0)).toHaveLength(0);
  });
  it("the caret scrolls into view and is the one reversed cell", () => {
    const row = fieldRows({ mark: " / ", text: "abcdefghijkl", caret: 12, hint }, 12, 3)[0]!;
    // room is inner - 4 = 8 cells: the last seven characters then the caret on a blank
    expect(row.slice(3, 11).map((c) => c.ch).join("")).toBe("fghijkl ".slice(0, 8));
    expect(row.filter((c) => c.s.inverse)).toHaveLength(1);
  });
  it("a field without the keys shows its words as set, no caret", () => {
    const row = fieldRows({ mark: " / ", text: "gg", caret: null, hint }, 30, 3)[0]!;
    expect(row.some((c) => c.s.inverse)).toBe(false);
    expect(row.slice(3, 5).map((c) => c.ch).join("")).toBe("gg");
  });
  it("a bottom field is one row of the pane's width", () => {
    expect(bottomFieldRow({ mark: " / ", text: "x", caret: 1, hint }, 20).reduce((n, c) => n + c.w, 0)).toBe(20);
  });
});
