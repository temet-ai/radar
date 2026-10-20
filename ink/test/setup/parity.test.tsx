import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mount, tick } from "../app/harness.js";
import { applyLook, currentLook, defaultLook, slotStyle } from "../../src/text/look.js";
import { setupAreas, MOST_HIGH, MOST_WIDE } from "../../src/app/SetupBody.js";
import { parseAnsi } from "../nodes/ansi.js";
import { detailRows } from "../../src/nodes/index.js";

vi.hoisted(() => {
  process.env.FORCE_COLOR = "3"; // Ink's colour library reads this when it loads: keep the cells' styles
});
import { SCENES } from "./scenes.js";

/**
 * Setup: the sheet above the card (no rail), as the goldens in ./goldens read: Ink's own, frozen from the old Rust renderer
 * text of the same screens before radar-core's setup tests moved to view text (e52f4ce).
 * The scenes are the view data the host sends for them (./scenes.ts). The goldens draw no icons.
 */
const here = path.dirname(fileURLToPath(import.meta.url));
const goldens = path.resolve(here, "goldens");

/** A golden's lines, each without its trailing blanks. */
function reference(name: string): string[] {
  return trimmed(fs.readFileSync(path.join(goldens, `${name}.txt`), "utf8").split("\n"));
}
const trimmed = (lines: string[]): string[] => {
  const out = lines.map((l) => l.trimEnd());
  while (out.length > 0 && out[out.length - 1] === "") out.pop();
  return out;
};
const plain = (s: string) => s.replace(/\x1b\[[0-9;?]*[A-Za-z]/g, "");

let live: Array<() => void> = [];
let before = currentLook();
beforeEach(() => {
  before = currentLook();
  applyLook(defaultLook("off"));
});
afterEach(() => {
  live.forEach((f) => f());
  live = [];
  applyLook(before);
});

async function draw(scene: (typeof SCENES)[number]) {
  const m = mount(scene.width, scene.height);
  live.push(m.stop);
  m.welcome();
  m.snapshot({ chrome: scene.chrome, section: scene.section, screen: null, focus: "middle" }, 1);
  await tick(80);
  return m;
}

describe("setup: Ink draws the text the reference draws", () => {
  for (const scene of SCENES) {
    it(`${scene.name}: the top bar, the card, the sheet and the key bar`, async () => {
      const m = await draw(scene);
      expect(trimmed(plain(m.frame()).split("\n"))).toEqual(reference(scene.name));
    });
  }
});

describe("setup: the frame", () => {
  const rect = (r: { x: number; y: number; w: number; h: number }) => [r.x, r.y, r.w, r.h];

  it("is the body less a cell each side, at most 140 x 40, centred (shell/setup.rs frame)", () => {
    expect(rect(setupAreas(80, 22).frame)).toEqual([1, 0, 78, 22]);
    expect(rect(setupAreas(160, 38).frame)).toEqual([10, 0, MOST_WIDE, 38]);
    expect(rect(setupAreas(260, 68).frame)).toEqual([1 + (258 - MOST_WIDE) / 2, (68 - MOST_HIGH) / 2, MOST_WIDE, MOST_HIGH]);
    expect(rect(setupAreas(261, 69).frame)).toEqual([1 + Math.floor((259 - MOST_WIDE) / 2), Math.floor((69 - MOST_HIGH) / 2), MOST_WIDE, MOST_HIGH]);
  });

  it("puts the sheet on top, as high as its text plus a row (at least 2; at most the larger of a third of the frame and the frame less 22 rows, which the card keeps), then the card, at every width", () => {
    // 160x40: one blank row, the sheet of 9 text rows + 1, a blank row, the card's tab, the card
    const wide = setupAreas(160, 38, 9);
    expect(rect(wide.sheet)).toEqual([10, 1, 140, 10]);
    expect(rect(wide.card)).toEqual([10, 13, 140, 25]);
    // 80x24: the body is 22 rows, a third is 7: the sheet is 7 however long its text
    const narrow = setupAreas(80, 22, 10);
    expect(rect(narrow.sheet)).toEqual([1, 1, 78, 7]);
    expect(rect(narrow.card)).toEqual([1, 10, 78, 12]);
    // the cap of 12, and the least of 2
    expect(setupAreas(160, 38, 30).sheet.h).toBe(16); // 38 - CARD_LEAST: a tall frame keeps the card its 22 rows
    expect(setupAreas(160, 24, 30).sheet.h).toBe(8); // a short one gives the sheet a third
    expect(setupAreas(80, 22, 30).sheet.h).toBe(7);
    expect(setupAreas(160, 38, 0).sheet.h).toBe(2);
    expect(setupAreas(160, 38, 3).sheet.h).toBe(4);
    // never side by side: at 121 or 260 cells the card is still under the sheet
    for (const w of [121, 122, 260]) {
      const a = setupAreas(w, 38, 5);
      expect(a.card.y).toBe(a.sheet.y + a.sheet.h + 2);
      expect(a.card.x).toBe(a.sheet.x);
      expect(a.card.w).toBe(a.sheet.w);
    }
  });

  it("reserves no rows for a rail, even when the chrome still carries stops", async () => {
    const scene = SCENES.find((s) => s.name === "choose_160x40")!;
    const withStops = {
      ...scene,
      chrome: {
        ...scene.chrome,
        setup: { ...scene.chrome.setup!, rail: [{ id: "look", label: "look & terminal", icon: null, state: "current", result: null, target: "chrome:setup:0" }] },
      },
    } as typeof scene;
    expect(plain((await draw(withStops)).frame())).toBe(plain((await draw(scene)).frame()));
  });

  it("stands the tabs one row above their panels, at the panels' x, filled with the slot's colour", async () => {
    const colours = {
      chip: slotStyle("accent.primary").color!.toLowerCase(),
      sheetChip: slotStyle("glow.cyan").color!.toLowerCase(),
      ground: slotStyle("bg.base").color!.toLowerCase(),
      card: slotStyle("bg.surface").color!.toLowerCase(),
      sheet: slotStyle("bg.overlay").color!.toLowerCase(),
    };
    const m = await draw(SCENES.find((s) => s.name === "choose_160x40")!);
    const rows = parseAnsi(m.frame());
    const a = setupAreas(160, 38, 5);
    const top = 1; // under the top bar
    // the sheet's tab on the frame's first row: ` about this step ` on glow.cyan from the sheet's x, bold bg.base on it;
    // the card's tab on the blank row under the sheet: ` choose ` on accent.primary from the card's x
    const sheetTab = rows[top + a.frame.y]!;
    expect(sheetTab[a.sheet.x - 1]!.bg).toBe(colours.ground);
    for (let i = 0; i < " about this step ".length; i++) {
      expect(sheetTab[a.sheet.x + i]!.bg, `sheet tab cell ${i}`).toBe(colours.sheetChip);
      expect(sheetTab[a.sheet.x + i]!.fg).toBe(colours.ground);
      expect(sheetTab[a.sheet.x + i]!.bold).toBe(true);
    }
    expect(sheetTab[a.sheet.x + " about this step ".length]!.bg).toBe(colours.ground);
    const tab = rows[top + a.card.y - 1]!;
    expect(tab[a.card.x - 1]!.bg).toBe(colours.ground);
    for (let i = 0; i < " choose ".length; i++) {
      expect(tab[a.card.x + i]!.bg, `tab cell ${i}`).toBe(colours.chip);
      expect(tab[a.card.x + i]!.fg).toBe(colours.ground);
      expect(tab[a.card.x + i]!.bold).toBe(true);
    }
    expect(tab[a.card.x + " choose ".length]!.bg).toBe(colours.ground);
    // the panels: every cell of the card on bg.surface, of the sheet on bg.overlay, the canvas around on bg.base
    for (let y = 0; y < a.card.h; y++) {
      const r = rows[top + a.card.y + y]!;
      expect(r[a.card.x]!.bg, `card row ${y}`).toBe(colours.card);
      expect(r[a.card.x + a.card.w - 1]!.bg).toBe(colours.card);
      expect(r[a.card.x - 1]!.bg).toBe(colours.ground);
      expect(r[a.card.x + a.card.w]!.bg).toBe(colours.ground);
    }
    for (let y = 0; y < a.sheet.h; y++) {
      const r = rows[top + a.sheet.y + y]!;
      expect(r[a.sheet.x]!.bg, `sheet row ${y}`).toBe(colours.sheet);
      expect(r[a.sheet.x + a.sheet.w - 1]!.bg).toBe(colours.sheet);
      expect(r[a.sheet.x - 1]!.bg).toBe(colours.ground);
      expect(r[a.sheet.x + a.sheet.w]!.bg).toBe(colours.ground);
    }
  });

  it("centres the frame in a larger body and fills the rest with bg.base", async () => {
    const scene = { ...SCENES.find((s) => s.name === "choose_160x40")!, width: 260, height: 70 };
    scene.chrome = { ...scene.chrome, width: 260, height: 70 };
    const m = await draw(scene);
    const rows = parseAnsi(m.frame());
    const a = setupAreas(260, 68, 5);
    const ground = slotStyle("bg.base").color!.toLowerCase();
    const chipRow = rows[1 + a.frame.y]!;
    expect(chipRow.map((c) => c.ch).join("").indexOf(" about this step ")).toBe(a.sheet.x);
    expect(a.frame.x).toBe(60);
    expect(rows[1 + a.frame.y + 5]![3]!.bg).toBe(ground);
    expect(rows[1 + a.frame.y - 1]![100]!.bg).toBe(ground);
    expect(rows[1 + a.frame.y + a.frame.h]![100]!.bg).toBe(ground);
  });
});

describe("setup: how many rows the sheet's text takes (about_rows)", () => {
  const rows = (name: string, frameW: number) => {
    const s = SCENES.find((x) => x.name === name)!.section;
    return detailRows({ node: "rich", blocks: s.detail_blocks }, frameW - 4);
  };
  it("counts a section's heading and its lines at the frame's width less four", () => {
    expect(rows("scan_80x24", 78)).toBe(10);
    expect(rows("ready_80x24", 78)).toBe(7);
    expect(rows("choose_160x40", MOST_WIDE)).toBe(5);
    expect(rows("part_step_160x40", MOST_WIDE)).toBe(2);
  });
  it("wraps a long line to more rows at a narrow width", () => {
    const wide = rows("scan_80x24", 78);
    expect(rows("scan_80x24", 30)).toBeGreaterThan(wide);
  });
});

describe("setup: the card's head", () => {
  it("is the heading in bold text.emphasis, the lead in text.muted, a blank row, then the rows", async () => {
    const scene = SCENES.find((s) => s.name === "choose_160x40")!;
    const rows = parseAnsi((await draw(scene)).frame());
    const a = setupAreas(160, 38, 5);
    const x = a.card.x + 2;
    const y = 1 + a.card.y + 1;
    const heading = rows[y]![x]!;
    expect(heading.ch).toBe("W");
    expect(heading.fg).toBe(slotStyle("text.emphasis").color!.toLowerCase());
    expect(heading.bold).toBe(true);
    const lead = rows[y + 1]![x]!;
    expect(lead.ch).toBe("T");
    expect(lead.fg).toBe(slotStyle("text.muted").color!.toLowerCase());
    expect(rows[y + 2]!.map((c) => c.ch).join("").slice(x, x + 30).trim()).toBe("");
    expect(rows[y + 3]!.map((c) => c.ch).join("")).toContain("THIS MACHINE");
  });

  it("is cut to the inside width, and a card with neither line starts its rows at the top", async () => {
    const scene = SCENES.find((s) => s.name === "scan_80x24")!;
    const long = { ...scene, chrome: { ...scene.chrome, setup: { ...scene.chrome.setup!, heading: "H".repeat(200), lead: "" } } } as typeof scene;
    const lines = plain((await draw(long)).frame()).split("\n");
    const card = setupAreas(80, 22, 10).card;
    expect(lines[card.y + 2]!.trimEnd()).toBe(" ".repeat(3) + "H".repeat(card.w - 4));
    const bare = { ...scene, chrome: { ...scene.chrome, setup: { ...scene.chrome.setup!, heading: "", lead: "" } } } as typeof scene;
    expect(plain((await draw(bare)).frame()).split("\n")[card.y + 2]).toContain("WHERE IT LOOKS");
  });
});
