import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3"; // Ink's colour library reads this when it loads: keep the cells' styles
});
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { mount, tick } from "../app/harness.js";
import { parseAnsi, type TCell } from "./ansi.js";
import { applyHostLook, applyLook, currentLook, defaultLook, type HostLook } from "../../src/text/look.js";
import { sectionHeading } from "../../src/nodes/index.js";

/**
 * A detail pane's sections (view Block::Section, view_tui section_rows over ui::card::section): the heading in muted
 * capitals, its detail in secondary words, a muted rule to the room's edge, the blocks two cells in, and each
 * action's supplied mnemonic underlined in its word. ink/test/cells/highlight_detail-sections-160x40.json is Ratatui's styled buffer of a
 * GitLab merge request selected on the first screen (written by tests/ratatui_cells.rs); the whole App draws the
 * same view data and the detail pane's rows, from its first section to the foot of the pane, are compared cell for
 * cell (character, colours, bold, underline).
 */
const here = path.dirname(fileURLToPath(import.meta.url));
const cellsDir = process.env.RADAR_CELLS_DIR ?? path.resolve(here, "../cells");
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
  view: { screen: any; chrome?: any; focus?: string; look?: HostLook };
  rows: RatCell[][];
}

const hexOf = (scene: Scene, name: string | undefined, canvas: string) => {
  const n = name ?? canvas;
  return (scene.colours[n.split("/")[0]!]?.hex ?? currentLook().slots[n.split("/")[0]!] ?? n).toLowerCase();
};

const rowText = (cells: Array<{ ch?: string }>) =>
  cells.map((c) => (c.ch === undefined || c.ch === "" ? " " : c.ch)).join("");

describe("a detail pane's sections", () => {
  const size = "160x40";
  const file = path.join(cellsDir, `highlight_detail-sections-${size}.json`);
  it(`Ink's section rows equal Ratatui's cells at ${size}`, async () => {
    expect(fs.existsSync(file), `${file} is written by tests/ratatui_cells.rs (scene highlight_detail-sections)`).toBe(true);
    const scene = JSON.parse(fs.readFileSync(file, "utf8")) as Scene;
    if (scene.view.look) applyHostLook(scene.view.look); else applyLook(defaultLook("off"));
    const [cols, rows] = size.split("x").map(Number) as [number, number];
    const chrome = scene.view.chrome ?? JSON.parse(fs.readFileSync(path.join(fixtures, `host_chrome_${size}.json`), "utf8"));
    const m = mount(cols, rows);
    live.push(m.stop);
    m.welcome();
    m.snapshot({ chrome, screen: scene.view.screen, focus: scene.view.focus ?? "middle" }, 1);
    await tick(60);
    const ink = parseAnsi(m.frame());

    const at = scene.rows.findIndex((r) => rowText(r).includes(" STATE ─"));
    expect(at, "Ratatui drew the STATE section").toBeGreaterThan(-1);
    const inkAt = ink.findIndex((r) => rowText(r).includes(" STATE ─"));
    expect(inkAt, "Ink drew the STATE section").toBeGreaterThan(-1);
    // From the detail pane's left border to the cell before its right edge (the scrollbar is not a section's).
    const left = rowText(scene.rows[at]!).lastIndexOf("│", rowText(scene.rows[at]!).indexOf(" STATE ─"));
    const detail = scene.view.screen.panes.find((pane: { pane: string }) => pane.pane === "detail");
    if (detail.body.node === "extension" && detail.body.capability === "detail") {
      // #46 moves the same keyed action from a scrolling section to its pinned row.
      // Keep the supplied offset, original key, availability reason and native underline assertions.
      const update = detail.foot_actions.actions.find((item: any) => item.action.id === "gitlab.repos.rebase");
      expect(update.action).toMatchObject({ label: "update", key: { key: { char: "u" } }, mnemonic_at: 0 });
      expect(update.disabled).toBe("not behind main");
      const canonical = scene.view.screen.actions.find((action: any) => action.id === update.action.id);
      expect(canonical.label).toContain("update by rebasing");
      const updateRow = scene.rows[rows - 3]!;
      const updateX = rowText(updateRow).indexOf("update", left);
      expect(updateX, "the native capture paints the pinned update caption").toBeGreaterThan(left);
      expect(updateRow[updateX]).toMatchObject({ ch: "u", mods: ["underline"] });
      expect(rowText(updateRow)).not.toContain("[u]");
      expect(ink[rows - 3]![updateX]).toMatchObject({ ch: "u", underline: true });
    } else {
      // Preserve the pre-extension reference until the native producer is deliberately regenerated.
      const caption = "update by rebasing · not behind main";
      const actions = detail.body.blocks.find((block: { title?: string }) => block.title === "ACTIONS");
      const update = actions.blocks.find((block: { key?: string }) => block.key === "u");
      expect(update, "the capture embeds the host's exact action mnemonic").toMatchObject({ key: "u", mnemonic_at: 0, target: "act:u" });
      expect(update.text.map((seg: { text?: string }) => seg.text ?? "").join("")).toBe(caption);
      const updateRow = scene.rows.find((r) => rowText(r).includes(caption));
      expect(updateRow, "the native capture paints the current action caption").toBeDefined();
      const updateX = rowText(updateRow!).indexOf(caption);
      expect(updateRow![updateX]).toMatchObject({ ch: "u", mods: ["underline"] });
      expect(rowText(updateRow!)).not.toContain("[u]");
    }

    const canvasBg = hexOf(scene, scene.canvas.bg, scene.canvas.bg);
    const bad: string[] = [];
    for (let dy = 0; at + dy < rows - 2; dy++) {
      const y = at + dy;
      const iy = inkAt + dy;
      const row = scene.rows[y]!;
      for (let x = left; x < cols - 1; x++) {
        const c = row[x]!;
        const i: TCell | undefined = ink[iy]?.[x];
        if (c.ch === "" || i?.ch === "") continue;
        const ch = c.ch ?? " ";
        const blank = ch.trim() === "";
        const mods = c.mods ?? [];
        const want = { bg: hexOf(scene, c.bg, scene.canvas.bg), fg: hexOf(scene, c.fg, scene.canvas.fg), bold: mods.includes("bold"), underline: mods.includes("underline") };
        const got = { bg: i?.bg?.toLowerCase(), fg: i?.fg?.toLowerCase(), bold: !!i?.bold, underline: !!i?.underline };
        const why: string[] = [];
        if ((i?.ch ?? " ") !== ch && !(blank && (i?.ch ?? " ").trim() === "")) why.push(`char ${JSON.stringify(i?.ch)} for ${JSON.stringify(ch)}`);
        if ((got.bg ?? canvasBg) !== want.bg) why.push(`bg ${got.bg} for ${want.bg}`);
        if (!blank) {
          if (got.fg !== undefined && got.fg !== want.fg) why.push(`fg ${got.fg} for ${want.fg}`);
          if (got.bold !== want.bold) why.push(`bold ${got.bold} for ${want.bold}`);
          if (got.underline !== want.underline) why.push(`underline ${got.underline} for ${want.underline}`);
        }
        if (why.length) bad.push(`row ${y} col ${x}: ${why.join(", ")}`);
      }
    }
    expect(bad).toEqual([]);
  });
});

describe("sectionHeading (ui::card::section)", () => {
  const words = (cells: Array<{ ch: string }>) => cells.map((c) => c.ch).join("");
  it("the title, its detail where it fits, and a rule to the room's edge", () => {
    expect(words(sectionHeading({ title: "PIPELINE", detail: "passed · 6m 12s" }, 30, true))).toBe("PIPELINE  passed · 6m 12s ────");
    expect(words(sectionHeading({ title: "REPLIES", detail: "2 replies · last 2d ago" }, 12, true))).toBe("REPLIES ────");
    expect(words(sectionHeading({ title: "ACTIONS" }, 8, true))).toBe("ACTIONS");
  });
  it("every cell is bold, the mark's too (widgets::heading's line style is under every span)", () => {
    applyLook(defaultLook("unicode"));
    const cells = sectionHeading({ mark: "settings", title: "STATE", detail: "now" }, 24);
    expect(cells.length).toBeGreaterThan(20);
    expect(cells.filter((c) => !c.s.bold).map((c) => c.ch)).toEqual([]);
    applyLook(defaultLook());
  });
});
