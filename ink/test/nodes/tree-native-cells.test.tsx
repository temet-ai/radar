import { vi } from "vitest";
vi.hoisted(() => { process.env.FORCE_COLOR = "3"; });
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { mount, until } from "../app/harness.js";
import { parseAnsi, withoutEdgeDividers, type TCell } from "./ansi.js";
import { frameOf } from "../../src/chrome/Chrome.js";
import type { ChromeView } from "../../src/chrome/types.js";
import { applyHostLook, applyLook, currentLook, defaultLook, type HostLook } from "../../src/text/look.js";
import type { Tree, TreeRow } from "../../src/nodes/view.js";
import { adoptInkFrame, updatingGoldens, type SceneFile } from "../support/goldens.js";

interface RatCell { ch?: string; fg?: string; bg?: string; mods?: string[] }
interface Scene {
  scene: string;
  width: number;
  height: number;
  canvas: { fg: string; bg: string };
  colours: Record<string, { hex: string }>;
  view: { screen: any; chrome: ChromeView; focus: string; look: HostLook };
  rows: RatCell[][];
}

const cellsDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../cells");
const textOf = (cells: { ch?: string }[]) => cells.map((c) => c.ch ?? " ").join("");
const samePath = (a: string[], b: string[]) => a.length === b.length && a.every((id, i) => id === b[i]);
const modifiers = { bold: "bold", dim: "dim", italic: "italic", underline: "underline", inverse: "reversed", strike: "crossed_out" } as const;
let cleanup: Array<() => void> = [];
afterEach(() => { cleanup.forEach((stop) => stop()); cleanup = []; applyLook(defaultLook()); });

/** Replay the golden's exact selected/folded view; INK_UPDATE_GOLDENS=1 rewrites its cells from Ink. */
describe("a folded tree against actual native cells", () => {
  for (const [width, height] of [[80, 24], [160, 40]] as const) {
    it(`${width}x${height}: held count, semantic colours, modifiers and every blank ground`, async () => {
      const file = path.join(cellsDir, `highlight_folded-row-${width}x${height}.json`);
      expect(fs.existsSync(file), `the committed golden ${file} is missing`).toBe(true);
      const scene = JSON.parse(fs.readFileSync(file, "utf8")) as Scene;
      expect([scene.scene, scene.width, scene.height]).toEqual(["highlight_folded-row", width, height]);
      expect(scene.view.look, "use the captured host look").toBeDefined();
      expect(scene.view.chrome, "use the captured host chrome").toBeDefined();
      const pane = scene.view.screen.panes.find((p: any) => p.pane === scene.view.focus);
      expect(pane?.body.node).toBe("tree");
      const tree = pane.body as Tree;
      expect(tree.selected).toBeDefined();
      let siblings = tree.rows;
      let selected: TreeRow | undefined;
      for (const id of tree.selected!) {
        const matches = siblings.filter((row) => row.id === id);
        expect(matches, "the captured selected path resolves uniquely").toHaveLength(1);
        selected = matches[0]!;
        siblings = selected.children ?? [];
      }
      expect(selected?.heading).not.toBe(true);
      expect(selected?.children?.length).toBeGreaterThan(0);
      expect((tree.open ?? []).some((p) => samePath(p, tree.selected!)), "the host closed the selected group").toBe(false);
      const pending = [...selected!.children!];
      let descendants = 0;
      while (pending.length) { const row = pending.pop()!; descendants++; pending.push(...row.children ?? []); }
      expect(descendants, "the fixture exercises recursively held rows, not just direct children").toBeGreaterThan(selected!.children!.length);
      const marker = `⋯ ${descendants}`;

      applyHostLook(scene.view.look);
      const m = mount(width, height); cleanup.push(m.stop);
      m.welcome();
      m.snapshot({ screen: scene.view.screen, chrome: scene.view.chrome, focus: scene.view.focus }, 1);
      await until(() => parseAnsi(m.frame()).some((row) => textOf(row).includes(marker)));
      // the native recording predates the column-edge dividers
      const actual = withoutEdgeDividers(parseAnsi(m.frame()), scene.rows);
      if (updatingGoldens()) adoptInkFrame(file, scene as unknown as SceneFile, actual);
      const frame = frameOf(scene.view.chrome, null, width, height);
      const titleRow = scene.rows[frame.bodyTop]!;
      const borders = titleRow.flatMap((cell, x) => cell.ch === "╭" ? [x] : []);
      const left = borders.find((x) => {
        const end = titleRow.findIndex((cell, i) => i > x && cell.ch === "╮");
        return textOf(titleRow.slice(x, end + 1)).includes(pane.title);
      });
      expect(left, "the native frame identifies the tree pane").toBeDefined();
      const right = titleRow.findIndex((cell, x) => x > left! && cell.ch === "╮");
      expect(right).toBeGreaterThan(left!);
      const nativeMarkerY = scene.rows.findIndex((row) => textOf(row.slice(left! + 1, right)).includes(marker));
      expect(nativeMarkerY, "the native row paints the entire held-descendant count").toBeGreaterThan(frame.bodyTop);

      expect(textOf(actual[nativeMarkerY]?.slice(left! + 1, right) ?? [])).toContain(marker);
      const colour = (name: string | undefined, fallback: string) => {
        const n = name ?? fallback;
        return (scene.colours[n]?.hex ?? scene.colours[n.split("/")[0]!]?.hex ?? currentLook().slots[n.split("/")[0]!] ?? n).toLowerCase();
      };
      const bad: string[] = [];
      // The complete pane interior: header, folded/open rows, empty rows and notes. Frame ramps belong to chrome.
      for (let y = frame.bodyTop + 1; y < frame.bodyTop + frame.bodyHeight - 1; y++) for (let x = left! + 1; x < right; x++) {
        const want = scene.rows[y]![x]!, got: TCell | undefined = actual[y]?.[x];
        const wantCh = want.ch ?? " ", gotCh = got?.ch ?? " ";
        if (wantCh !== gotCh) bad.push(`${x},${y}: character ${JSON.stringify(gotCh)} instead of ${JSON.stringify(wantCh)}`);
        const bg = colour(want.bg, scene.canvas.bg), gotBg = colour(got?.bg, scene.canvas.bg);
        if (bg !== gotBg) bad.push(`${x},${y}: background ${gotBg} instead of ${bg}`);
        // Foreground and emphasis on blank padding are invisible; its ground must still match cell for cell.
        if (wantCh.trim()) {
          const fg = colour(want.fg, scene.canvas.fg), gotFg = colour(got?.fg, scene.canvas.fg);
          if (fg !== gotFg) bad.push(`${x},${y}: foreground ${gotFg} instead of ${fg}`);
          for (const [ink, native] of Object.entries(modifiers)) {
            if (!!got?.[ink as keyof typeof modifiers] !== !!want.mods?.includes(native)) bad.push(`${x},${y}: ${ink} differs`);
          }
        }
      }
      expect(bad.slice(0, 100), `${bad.length} native tree-pane cell differences`).toEqual([]);
    });
  }
});
