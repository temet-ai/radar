import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3"; // Ink's colour library reads this when it loads: keep the cells' styles
});
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { mount, tick, until } from "./harness.js";
import { applyHostLook, applyLook, defaultLook, type HostLook } from "../../src/text/look.js";
import { parseAnsi } from "../nodes/ansi.js";
import { adoptInkFrame, updatingGoldens, type SceneFile } from "../support/goldens.js";

// The styled buffer with text selected by a drag across two rows (ink/test/cells, scene highlight_selection, on
// bg.selection; first drawn by the reference, now Ink's golden, rewritten from Ink by INK_UPDATE_GOLDENS=1): from the press
// to the pane's right edge, then from its left edge
// to the pointer. The whole App draws the same view data, the same drag is made with the mouse, and every cell of the
// body is compared for being on bg.selection: the selected cells, the cells beside the pane (its border and
// scrollbar), the rows above and below the drag, and the cells before the press and after the pointer.
const here = path.dirname(fileURLToPath(import.meta.url));
type Pt = { x: number; y: number };
interface Scene {
  canvas: { fg: string; bg: string };
  colours: Record<string, { hex: string }>;
  view: { screen: any; chrome: any; focus: string; look: HostLook; selection: { anchor: Pt; head: Pt; bound: Pt & { width: number; height: number } } };
  rows: Array<Array<{ ch?: string; bg?: string }>>;
}

let live: Array<() => void> = [];
afterEach(() => {
  live.forEach((f) => f());
  live = [];
  applyLook(defaultLook());
});

const SELECTION = "#44475a";
const mouse = (code: number, p: Pt, up = false) => `\x1b[<${code};${p.x + 1};${p.y + 1}${up ? "m" : "M"}`;

describe("a drag across two rows of text against the reference's cells", () => {
  for (const size of ["80x24", "160x40"]) {
    it(`selects the same cells at ${size}`, async () => {
      const file = path.resolve(here, `../cells/highlight_selection-${size}.json`);
      const scene = JSON.parse(fs.readFileSync(file, "utf8")) as Scene;
      applyHostLook(scene.view.look);
      const [cols, rowsN] = size.split("x").map(Number) as [number, number];
      const chrome = scene.view.chrome;
      const { anchor, head, bound } = scene.view.selection;
      const m = mount(cols, rowsN);
      live.push(m.stop);
      m.welcome();
      m.snapshot({ chrome, screen: scene.view.screen, focus: scene.view.focus }, 1);
      await tick(60);

      // Use the exact captured chrome/focus/look, and match the same pane frame.
      const ch = (c: { ch?: string } | undefined) => (c?.ch === undefined || c.ch === "" ? " " : c.ch);
      const before = parseAnsi(m.frame());
      const topOf = (rows: Array<Array<{ ch?: string }>>) => rows.findIndex((r, y) => y > 0 && ch(r[bound.x - 1]) === "╭" && ch(r[bound.x + bound.width]) === "╮");
      const dy = topOf(before) - topOf(scene.rows);
      expect(topOf(before), "Ink drew the pane's frame where the selection's bound is").toBeGreaterThan(-1);
      const at = (p: Pt) => ({ x: p.x, y: p.y + dy });
      const plain = m.frame();

      m.type(mouse(0, at(anchor)));
      m.type(mouse(32, at(head)));
      await until(() => m.frame() !== plain);
      const got = parseAnsi(m.frame());
      if (updatingGoldens()) {
        // the golden is Ink's frame with the drag made, and the drag's cells where Ink drew the pane; the next run compares
        const moved = (p: Pt) => ({ x: p.x, y: p.y + dy });
        scene.view.selection = { anchor: moved(anchor), head: moved(head), bound: { ...bound, y: bound.y + dy } };
        adoptInkFrame(file, scene as unknown as SceneFile, got);
        return;
      }

      const rat =(n: string | undefined) => (scene.colours[n ?? ""]?.hex ?? "").toLowerCase();
      const ratSelected = (x: number, y: number) => (scene.rows[y]![x]!.bg ?? "").split("/").includes("bg.selection");
      const inkSelected = (x: number, y: number) => got[y + dy]?.[x]?.bg?.toLowerCase() === SELECTION;
      expect(scene.colours["bg.selection"]!.hex.toLowerCase()).toBe(SELECTION);
      const bad: string[] = [];
      let selected = 0;
      // the body: every row between the top bar and the status line, all of its columns (the screen has its own
      // bg.selection cells, a list's cursor row: they are compared too, and only the drag's are counted)
      for (let y = 2; y < rowsN - 1; y++) {
        if (!got[y + dy]) continue;
        for (let x = 0; x < cols; x++) {
          const want = ratSelected(x, y);
          if (want && y >= anchor.y && y <= head.y && x >= bound.x && x < bound.x + bound.width) selected++;
          if (inkSelected(x, y) !== want) bad.push(`(${x},${y}): ${want ? "not selected in Ink" : `selected in Ink, ${rat(scene.rows[y]![x]!.bg)} in the reference`}`);
        }
      }
      expect(bad.slice(0, 20)).toEqual([]);
      // two rows: the press to the right edge, the left edge to the pointer
      expect(selected).toBe(bound.x + bound.width - anchor.x + (head.x - bound.x + 1));
      // just outside the pane: the cells either side on both rows
      for (const y of [anchor.y, head.y]) {
        expect(inkSelected(bound.x - 1, y), `left of the pane on row ${y}`).toBe(false);
        expect(inkSelected(bound.x + bound.width, y), `right of the pane on row ${y}`).toBe(false);
      }
    });
  }
});
