import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { layoutMenu, layoutSources, layoutStatus, layoutTopBar, layoutNav, type Cell } from "../../src/chrome/layout.js";
import { layoutHistory } from "../../src/chrome/history.js";
import { cellWidth } from "../../src/chrome/width.js";
import type { ChromeView } from "../../src/chrome/types.js";
import { actionsMenu, booksChrome, confirm } from "./fixture.js";

// GUIDE section 2, the chrome rules, as render tests over every recorded chrome fixture (host_chrome_<w>x<h>.json) at
// 80x24 and 160x40, plus the hand-made Books chrome and menus at both sizes.
const dir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../fixtures");
const recorded = (w: number, h: number): ChromeView => JSON.parse(fs.readFileSync(path.join(dir, `host_chrome_${w}x${h}.json`), "utf8"));
const SIZES: [number, number][] = [
  [80, 24],
  [160, 40],
];
const text = (cells: Cell[]) => cells.map((c) => c.text).join("");
const MARKERS = /[▶▸▾›▼►]|\[[+-]\]/;

const sceneOf = (w: number, h: number) => {
  const out: { name: string; chrome: ChromeView }[] = [
    { name: `host_chrome_${w}x${h}`, chrome: recorded(w, h) },
    { name: `books ${w}x${h}`, chrome: booksChrome({ width: w, height: h }) },
  ];
  return out;
};

describe.each(SIZES)("chrome rules over every chrome fixture at %ix%i", (w, h) => {
  for (const { name, chrome } of sceneOf(w, h)) {
    const top = layoutTopBar(chrome, w, 0);
    const status = layoutStatus(chrome, w, h - 1);
    const sources = layoutSources(chrome, w, 1);
    const rows: [string, Cell[]][] = [["top bar", top.cells], ["status", status.cells], ...(sources ? ([["sources", sources.cells]] as [string, Cell[]][]) : [])];

    it(`${name}: 2.1 the status line is one fixed row that fits, and every key in it is a pointer target`, () => {
      expect(cellWidth(text(status.cells))).toBeLessThanOrEqual(w);
      for (const hint of chrome.status.hints) {
        for (const k of hint.keys_of ?? []) {
          // A hint dropped to fit takes its keys with it; one that is shown has every key pointable.
          if (text(status.cells).includes(hint.keys)) expect(status.hits.map((x) => x.target), hint.keys).toContain(k.target);
        }
      }
    });

    it(`${name}: 2.12 no chevrons, [+]/[-] or marker glyphs; selection is a shade, never inverse`, () => {
      for (const [row, cells] of rows) {
        expect(text(cells), row).not.toMatch(MARKERS);
        for (const c of cells) expect(c.inverse, `${row}: ${c.text}`).toBeFalsy();
      }
    });

    it(`${name}: 2.5 the top bar and sources are rows of exactly the width, with a target per tab`, () => {
      expect(cellWidth(text(top.cells))).toBe(w);
      for (const tab of chrome.top_bar.tabs) expect(top.hits.map((x) => x.target)).toContain(tab.target);
      if (sources) expect(cellWidth(text(sources.cells))).toBe(w);
    });
  }

  it("2.12 and 2.16 menus: no markers or inverse; the selected row is the selection shade and its rows are targets", () => {
    for (const menu of [actionsMenu, confirm]) {
      const m = layoutMenu(menu as never, w, 1, h - 2);
      for (const line of m.lines) {
        expect(text(line)).not.toMatch(MARKERS);
        for (const c of line) expect(c.inverse).toBeFalsy();
      }
      expect(m.x + m.w).toBeLessThanOrEqual(w);
      expect(m.hits.length).toBeGreaterThan(0);
    }
  });

  it("2.12 settings navigation rows: tree lines, no markers", () => {
    const nav = {
      groups: [
        { id: "a", name: "General", open: true, target: "chrome:group:a", sections: [{ id: "s", name: "views", target: "chrome:section:s" }] },
        { id: "b", name: "Git", open: false, target: "chrome:group:b", sections: [] },
      ],
      selected: "chrome:section:s",
      section_title: "s",
      detail_title: "d",
    };
    const rows = layoutNav(nav as never, 1, 2, 22, 10).rows;
    for (const r of rows) {
      expect(r.text).not.toMatch(MARKERS);
      expect(text(r.cells)).not.toMatch(MARKERS);
    }
    expect(rows.some((r) => /[├└]─/.test(text(r.cells)))).toBe(true);
  });

  it("2.12 and 2.1 the History overlay: no markers, footer keys pointable, fits the width", () => {
    const view = {
      version: 1, width: w, height: h - 1, title: "History", boundary: "b", scope: "s", notes: [], focus: "events", split: w >= 120,
      events_title: "events", columns: ["a", "b", "c", "d"],
      rows: [{ observed: "t", action: "a", subject: "s", outcome: "ok", outcome_ink: "good", target: "chrome:history:row:0" }],
      selected: 0, list_scroll: 0, state: "", detail_title: "d", detail: [], detail_scroll: 0, help_title: "k", help_lines: [], help_scroll: 0,
      footer: [{ keys: "esc", label: "close", keys_of: [{ label: "esc", chord: { key: "esc" }, target: "chrome:history:key:esc" }] }],
    };
    const l = layoutHistory(view as never, w, h - 1, 1);
    for (const line of l.lines) {
      expect(cellWidth(text(line))).toBe(w);
      expect(text(line)).not.toMatch(MARKERS);
    }
    expect(l.hits.map((x) => x.target)).toContain("chrome:history:key:esc");
  });
});

describe("2.18 nothing visual is hard-coded in src/chrome", () => {
  const src = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../src/chrome");
  for (const f of fs.readdirSync(src).filter((n) => /\.tsx?$/.test(n))) {
    it(`${f}: no hex colours, ANSI colour names or Color literals`, () => {
      const code = fs.readFileSync(path.join(src, f), "utf8").replace(/\/\*[\s\S]*?\*\/|\/\/.*$/gm, "");
      expect(code).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
      expect(code).not.toMatch(/\bColor::/);
      expect(code).not.toMatch(/(?:color|backgroundColor|ink)\s*[:=]\s*["'](?:red|green|blue|yellow|cyan|magenta|white|black|gray|grey)\w*["']/);
    });
  }
});
