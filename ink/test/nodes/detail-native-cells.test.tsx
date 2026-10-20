import { vi } from "vitest";
vi.hoisted(() => { process.env.FORCE_COLOR = "3"; });
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { mount, until } from "../app/harness.js";
import { parseAnsi } from "./ansi.js";
import { applyHostLook, applyLook, currentLook, defaultLook, type HostLook } from "../../src/text/look.js";
import { frameOf } from "../../src/chrome/Chrome.js";
import { detailWidth } from "../../src/app/Panes.js";
import { decodeDetail } from "../../src/nodes/detail.js";
import { HitRegistry } from "../../src/state/ui-state.js";
import type { ChromeView } from "../../src/chrome/types.js";
import { adoptInkFrame, updatingGoldens, type SceneFile } from "../support/goldens.js";

interface RatCell { ch?: string; fg?: string; bg?: string; mods?: string[] }
interface Scene {
  canvas: { fg: string; bg: string };
  colours: Record<string, { hex: string }>;
  view: { screen: any; chrome: ChromeView; focus: string; look: HostLook; detail_foot_hits: Array<{ x: number; y: number; width: number; height: number; target: string }> };
  rows: RatCell[][];
}
// The committed goldens (ink/test/cells); INK_UPDATE_GOLDENS=1 rewrites them from Ink's own frame and foot hits.
const dir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../cells");
const textOf = (cells: { ch?: string }[]) => cells.map((c) => c.ch ?? " ").join("");
let cleanup: Array<() => void> = [];
afterEach(() => { cleanup.forEach((stop) => stop()); cleanup = []; applyLook(defaultLook()); });

/** Expected cells are actual native output only. Missing/stale producers fail; no synthetic baselines. */
describe("actual native detail captures replay in Ink", () => {
  for (const owner of ["gitlab", "jira", "slack"]) for (const [columns, height] of [[80, 24], [160, 40]]) {
    it(`${owner} ${columns}x${height}: characters, blank grounds, bold, underline, pinned controls and logical pane number`, async () => {
      const file = path.join(dir, `detail_${owner}-${columns}x${height}.json`);
      expect(fs.existsSync(file), `the committed golden ${file} is missing`).toBe(true);
      const scene = JSON.parse(fs.readFileSync(file, "utf8")) as Scene;
      expect(scene.view.focus).toBe("detail");
      expect(scene.view.look, "replay the recorded host look").toBeDefined();
      expect(scene.view.chrome, "replay matching displayed host chrome").toBeDefined();
      applyHostLook(scene.view.look);
      const detail = scene.view.screen.panes.find((pane: any) => pane.pane === "detail");
      expect(decodeDetail(detail.body), "actual owner emitted the shared typed detail").not.toBeNull();
      const registry = new HitRegistry();
      const m = mount(columns!, height!, { registry }); cleanup.push(m.stop);
      m.welcome();
      m.snapshot({ chrome: scene.view.chrome, screen: scene.view.screen, focus: scene.view.focus }, 1);
      await until(() => parseAnsi(m.frame()).some((row) => textOf(row).includes("extra actions")));
      const actual = parseAnsi(m.frame());
      if (updatingGoldens()) {
        const foot = registry.all().filter((hit) => hit.target.startsWith("detail-foot:")).map(({ w, h, ...hit }) => ({ ...hit, width: w, height: h }));
        scene.view.detail_foot_hits = foot;
        adoptInkFrame(file, scene as unknown as SceneFile, actual);
      }
      const frame = frameOf(scene.view.chrome, null, columns!, height!);
      const paneW = columns! >= 120 ? detailWidth(columns!) : columns!;
      const left = columns! - paneW;
      const colour = (name: string | undefined, fallback: string) => {
        const n = name ?? fallback;
        return (scene.colours[n]?.hex ?? scene.colours[n.split("/")[0]!]?.hex ?? currentLook().slots[n.split("/")[0]!] ?? n).toLowerCase();
      };
      const bad: string[] = [];
      for (let y = frame.bodyTop + 1; y < frame.bodyTop + frame.bodyHeight - 1; y++) for (let x = left + 1; x < columns! - 1; x++) {
        const wanted = scene.rows[y]![x]!, got = actual[y]?.[x];
        const wantCh = wanted.ch ?? " ", gotCh = got?.ch ?? " ";
        if (wantCh !== gotCh) bad.push(`${x},${y}: ${JSON.stringify(gotCh)} instead of ${JSON.stringify(wantCh)}`);
        const bg = colour(wanted.bg, scene.canvas.bg), actualBg = (got?.bg ?? colour(undefined, scene.canvas.bg)).toLowerCase();
        if (bg !== actualBg) bad.push(`${x},${y}: background ${actualBg} instead of ${bg}`);
        // Foreground on blanks is invisible; their background is essential to the full-width shaded header.
        if (wantCh.trim()) {
          const fg = colour(wanted.fg, scene.canvas.fg), actualFg = (got?.fg ?? colour(undefined, scene.canvas.fg)).toLowerCase();
          if (fg !== actualFg) bad.push(`${x},${y}: foreground ${actualFg} instead of ${fg}`);
          for (const mod of ["bold", "underline"] as const) if (!!got?.[mod] !== !!wanted.mods?.includes(mod)) bad.push(`${x},${y}: ${mod} differs`);
        }
      }
      expect(bad.slice(0, 100), `${bad.length} native cell differences`).toEqual([]);
      expect(textOf(actual[frame.bodyTop]!)).toContain("3 ");
      const controls = registry.all().filter((hit) => hit.target.startsWith("detail-foot:"));
      expect(controls.length).toBeGreaterThan(0);
      const footY = frame.bodyTop + frame.bodyHeight - 2;
      expect(controls.every((hit) => hit.y === footY && hit.h === 1 && hit.x >= left + 1 && hit.x + hit.w <= columns! - 1)).toBe(true);
      expect(registry.all().filter((hit) => !hit.target.startsWith("detail-foot:") && hit.x >= left + 1).every((hit) => hit.y + hit.h <= footY - 1)).toBe(true);
      expect(scene.view.detail_foot_hits, "native shell recorded actual control rectangles after painting").toBeDefined();
      expect(controls).toEqual(scene.view.detail_foot_hits.map(({ width, height, ...hit }) => ({ ...hit, w: width, h: height })));
      const supplied = [...(detail.foot_actions?.actions ?? []), detail.foot_actions?.menu].filter(Boolean);
      for (const hit of controls) expect(supplied.some((control) => control.target === hit.target && control.disabled == null)).toBe(true);
    });
  }
});
