import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3"; // Ink's colour library reads this when it loads: keep the cells' styles
});
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import chalk from "chalk";
import { afterEach, describe, expect, it } from "vitest";
import { mount, tick, until } from "./harness.js";
import { shellRenderNode } from "../../src/app/Body.js";
import { paintHover } from "../../src/app/hover.js";
import * as fastOutput from "../../src/app/fast-output.js";
import { frameOf } from "../../src/chrome/Chrome.js";
import { applyLook, currentLook, defaultLook, slotStyle } from "../../src/text/look.js";
import { parseAnsi, type TCell } from "../nodes/ansi.js";
import { booksChrome } from "../chrome/fixture.js";
import { adoptInkFrame, updatingGoldens, type SceneFile } from "../support/goldens.js";

// ui::hover::paint (shell.rs hover): what a click would act on is lit with bg.hover, over the finished buffer. Ink lit it
// in the components, from a hover state in App: every pointer move to another row was a whole-app React render and every
// row's cells built again (about 17 ms of CPU a frame at 280x75; the renderer was 83% busy under a moving mouse and a
// click waited behind it). Now the hover is painted over the finished frame: no React render, one row restyled.
const HOVER = "#2a263e";
const SELECTION = "#44475a";
const here = path.dirname(fileURLToPath(import.meta.url));

let live: Array<() => void> = [];
afterEach(() => {
  live.forEach((f) => f());
  live = [];
  applyLook(defaultLook());
});

const row = (frame: string, y: number): TCell[] => parseAnsi(frame)[y] ?? [];
const move = (x: number, y: number) => `\x1b[<35;${x + 1};${y + 1}M`;

describe("the hover paint over a finished frame (ui::hover::paint)", () => {
  const ACCENT = slotStyle("accent.primary").color!;
  // abc␠ sel band tab 界 x: 0-3, 4-6, 7-10, 11-13, 14-15, 16
  const frame = [
    chalk.hex("#ffffff")("abc ") + chalk.bgHex(SELECTION)("sel") + chalk.bgHex("#1a162a")("band") + chalk.bgHex(ACCENT)("tab") + "界x",
    "untouched " + chalk.red("red"),
  ].join("\n");

  it("lights every cell of the area with bg.hover but the cursor's (bg.selection) and the focused tab's (the accent); the foreground stays", () => {
    const out = paintHover(frame, 20, { x: 0, y: 0, w: 20, h: 1 });
    const cells = row(out, 0);
    expect(cells.slice(0, 4).map((c) => [c.bg, c.fg])).toEqual(Array(4).fill([HOVER, "#ffffff"]));
    expect(cells.slice(4, 7).every((c) => c.bg === SELECTION)).toBe(true);
    expect(cells.slice(7, 11).every((c) => c.bg === HOVER)).toBe(true);
    expect(cells.slice(11, 14).every((c) => c.bg === ACCENT.toLowerCase() || c.bg === ACCENT)).toBe(true);
    expect(cells.slice(14, 20).every((c) => c.bg === HOVER)).toBe(true); // the wide glyph, x and the blanks to the edge
    expect(cells.map((c) => c.ch).join("")).toBe("abc selbandtab界x   ");
    expect(out.split("\n")[1]).toBe(frame.split("\n")[1]); // other rows are the frame's own strings
  });

  it("a chip on a state's ground (the status line's attention chip) keeps it; the key before it is lit", () => {
    for (const slot of ["status.warning", "status.success", "status.error"]) {
      const ground = slotStyle(slot).color!;
      const out = paintHover("` " + chalk.bgHex(ground)(" x needs you "), 20, { x: 0, y: 0, w: 15, h: 1 });
      const cells = row(out, 0);
      expect(cells[0]!.bg, slot).toBe(HOVER);
      expect(cells.slice(2, 15).every((c) => c.bg?.toLowerCase() === ground.toLowerCase()), slot).toBe(true);
    }
  });

  it("only the area: a cell before and after it keep their look, and a wide glyph cut by its edge is lit whole", () => {
    const cells = row(paintHover(frame, 20, { x: 15, y: 0, w: 2, h: 1 }), 0);
    expect(cells[13]!.bg).not.toBe(HOVER);
    expect([cells[14]!.bg, cells[15]!.bg, cells[16]!.bg]).toEqual([HOVER, HOVER, HOVER]);
    expect(cells[17]?.bg).toBeUndefined();
  });

  it("underlines instead in the mono tier, where no colour can mark it", () => {
    const l = currentLook();
    applyLook({ ...l, slots: Object.fromEntries(Object.keys(l.slots).map((k) => [k, ""])) });
    const cells = row(paintHover("abc", 5, { x: 0, y: 0, w: 5, h: 1 }), 0);
    expect(cells.slice(0, 3).every((c) => c.underline && c.bg === undefined)).toBe(true);
  });
});

const rows = ["!1", "!2", "!3"].map((t) => ({ target: `row:${t}`, text: [{ text: t }] }));
const list = { node: "list", selected: 0, rows };
const listScreen = { screen: { id: "mrs", title: "mrs", panes: [{ pane: "middle", title: "mrs", body: list }], actions: [] } };

describe("a pointer over a row", () => {
  it("lights the row over the frame Ink already composed: no React render, no row drawn again, only that line changes; off every row and on focus out it is gone; the host hears nothing", async () => {
    let renders = 0;
    const m = mount(80, 24, { renderNode: (node: any, cx: any) => (renders++, shellRenderNode(node, cx)) });
    live.push(m.stop);
    m.welcome();
    m.snapshot(listScreen, 1);
    await tick(60);
    const plain = m.frame();
    const lines = plain.split("\n");
    const y = lines.findIndex((l) => l.includes("!3"));
    const x = lines[y]!.indexOf("!3");
    const before = renders;
    const reused = fastOutput.reusedFrames;

    m.type(move(x, y));
    await until(() => m.frame() !== plain);
    const lit = m.frame();
    expect(renders).toBe(before); // red before: the hover was App state, a render of everything
    expect(fastOutput.reusedFrames).toBeGreaterThan(reused); // the frame was not composed again
    const changed = lit.split("\n").map((l, i) => (l === lines[i] ? -1 : i)).filter((i) => i >= 0);
    expect(changed).toEqual([y]);
    const cells = row(lit, y);
    expect(cells.slice(x, x + 2).every((c) => c.bg === HOVER)).toBe(true);

    m.type(move(70, 20)); // off every row
    await until(() => m.frame() === plain);
    expect(m.frame()).toBe(plain);
    m.type(move(x, y));
    await until(() => m.frame() === lit);
    m.type("\x1b[O"); // the terminal lost focus
    await until(() => m.frame() === plain);
    expect(m.frame()).toBe(plain);
    expect(renders).toBe(before);
    expect(m.intents()).toEqual([]);
  });

  it("the cursor row under the pointer keeps its own look", async () => {
    const m = mount(80, 24);
    live.push(m.stop);
    m.welcome();
    m.snapshot(listScreen, 1);
    await tick(60);
    const plain = m.frame();
    const lines = plain.split("\n");
    const y = lines.findIndex((l) => l.includes("!1"));
    m.type(move(lines[y]!.indexOf("!1"), y));
    await tick(40);
    expect(m.frame()).toBe(plain);
  });

  it("a snapshot that moves another row under the still pointer lights that one, as the pointer is where it was", async () => {
    const m = mount(80, 24);
    live.push(m.stop);
    m.welcome();
    m.snapshot(listScreen, 1);
    await tick(60);
    const lines = m.frame().split("\n");
    const y = lines.findIndex((l) => l.includes("!3"));
    m.type(move(lines[y]!.indexOf("!3"), y));
    await until(() => row(m.frame(), y).some((c) => c.bg === HOVER));
    // the list loses its last row: nothing is under the pointer now
    m.snapshot({ screen: { ...listScreen.screen, panes: [{ pane: "middle", title: "mrs", body: { ...list, rows: rows.slice(0, 2) } }] } }, 2);
    await until(() => !parseAnsi(m.frame()).some((r) => r.some((c) => c.bg === HOVER)));
    expect(parseAnsi(m.frame()).some((r) => r.some((c) => c.bg === HOVER))).toBe(false);
  });
});

describe("the chrome under the pointer", () => {
  const chrome = booksChrome({ width: 120, height: 30 });
  const hits = frameOf(chrome, null, 120, 30).top.hits;
  const start = async () => {
    const m = mount(120, 30);
    live.push(m.stop);
    m.welcome();
    m.snapshot({ chrome, ...listScreen }, 1);
    await tick(60);
    return m;
  };
  it("a top-bar tab is lit over its hit and no other cell; the showing tab's own cells keep their look, its padding is lit", async () => {
    const m = await start();
    const plain = m.frame();
    const tab = hits.find((h) => h.target === chrome.top_bar.tabs.find((t) => !t.active)!.target)!;
    m.type(move(tab.x + 1, 0));
    await until(() => m.frame() !== plain);
    const top = row(m.frame(), 0);
    for (let x = 0; x < top.length; x++) expect(top[x]!.bg === HOVER, `cell ${x}`).toBe(x >= tab.x && x < tab.x + tab.w);
    const active = hits.find((h) => h.target === chrome.top_bar.tabs.find((t) => t.active)!.target)!;
    const was = row(plain, 0);
    m.type(move(active.x + 1, 0));
    await until(() => row(m.frame(), 0)[active.x]?.bg !== top[active.x]?.bg);
    const now = row(m.frame(), 0);
    const inside = (x: number) => x >= active.x && x < active.x + active.w;
    expect(was.some((c, x) => inside(x) && c.bg === SELECTION)).toBe(true);
    now.forEach((c, x) => {
      if (!inside(x) || was[x]!.bg === SELECTION) expect(c, `cell ${x}`).toEqual(was[x]);
      else expect(c.bg, `cell ${x}`).toBe(HOVER);
    });
  });
  it("nothing is lit while a menu is up", async () => {
    const m = await start();
    const menu = { id: "m", title: "Actions", question: false, items: [{ value: "a", label: "an action", target: "chrome:menu:0" }] };
    m.snapshot({ chrome: { ...chrome, menu }, ...listScreen }, 2);
    await tick(60);
    const withMenu = m.frame();
    const tab = hits.find((h) => !h.target.startsWith("chrome:tab:0"))!;
    m.type(move(tab.x + 1, 0));
    await tick(60);
    expect(m.frame()).toBe(withMenu);
    expect(parseAnsi(m.frame()).some((r) => r.some((c) => c.bg === HOVER))).toBe(false);
  });
});

// The styled buffer with the pointer on a tree row (ink/test/cells, scene highlight_hovered-row; first drawn by Ratatui,
// now Ink's golden, rewritten from Ink by INK_UPDATE_GOLDENS=1): the whole App draws the same view data, the pointer is
// moved to the same cell, and the row is compared cell for cell.
interface RatCell {
  ch?: string;
  fg?: string;
  bg?: string;
  mods?: string[];
}
describe("the hovered tree row against Ratatui's cells", () => {
  for (const size of ["80x24", "160x40"]) {
    it(`is lit as Ratatui lights it at ${size}`, async () => {
      const file = path.resolve(here, `../cells/highlight_hovered-row-${size}.json`);
      expect(fs.existsSync(file), `${file} is a committed golden`).toBe(true);
      const scene = JSON.parse(fs.readFileSync(file, "utf8")) as {
        canvas: { bg: string };
        colours: Record<string, { hex: string }>;
        view: { screen: any; pointer: { x: number; y: number } };
        rows: RatCell[][];
      };
      applyLook(defaultLook("off"));
      const [cols, rowsN] = size.split("x").map(Number) as [number, number];
      const chrome = JSON.parse(fs.readFileSync(path.resolve(here, `../fixtures/host_chrome_${size}.json`), "utf8"));
      const m = mount(cols, rowsN);
      live.push(m.stop);
      m.welcome();
      m.snapshot({ chrome, screen: scene.view.screen, focus: "middle" }, 1);
      await tick(60);
      const text = (cells: Array<{ ch?: string }>) => cells.map((c) => (c.ch === undefined || c.ch === "" ? " " : c.ch)).join("");
      const at = scene.view.pointer;
      let want = scene.rows[at.y]!;
      // the host chrome's rows above the body differ from the scene's, so Ink's row is found by its text
      const iy = parseAnsi(m.frame()).findIndex((r) => text(r).trimEnd() === text(want).trimEnd());
      expect(iy, "Ink drew the row").toBeGreaterThan(-1);
      const plain = m.frame();
      m.type(move(at.x, iy));
      await until(() => m.frame() !== plain);
      const got = row(m.frame(), iy);
      if (updatingGoldens()) {
        // the golden is Ink's frame with the pointer on the row where Ink drew it
        scene.view.pointer = { x: at.x, y: iy };
        adoptInkFrame(file, scene as unknown as SceneFile, parseAnsi(m.frame()));
        want = scene.rows[iy]!;
      }
      const hex = (n: string | undefined) => (scene.colours[(n ?? scene.canvas.bg).split("/")[0]!]?.hex ?? currentLook().slots[(n ?? scene.canvas.bg).split("/")[0]!])?.toLowerCase();
      const canvas = hex(scene.canvas.bg);
      const bad: string[] = [];
      want.forEach((c, x) => {
        if (c.ch === "" || got[x]?.ch === "") return;
        const g = got[x]?.bg?.toLowerCase() ?? canvas;
        if (g !== hex(c.bg)) bad.push(`col ${x}: bg ${g} for ${hex(c.bg)} (${c.bg})`);
        if ((got[x]?.underline ?? false) !== (c.mods ?? []).includes("underlined")) bad.push(`col ${x}: underline`);
      });
      expect(bad).toEqual([]);
      expect(want.filter((c) => c.bg === "bg.hover").length).toBeGreaterThan(10);
    });
  }
});
