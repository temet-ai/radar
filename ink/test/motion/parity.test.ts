// Ink's motion engine against the recorded reference (crates/radar-core/src/motion.rs, tachyonfx): the same cue over the same plain
// frame, the same clock, and every cell compared at each frame. The reference is fixtures/motion-engine.json,
// written once by a generator that is now deleted (motion.rs's own Engine and ManualClock on motion/tests.rs's 30x8 frame).
import { describe, expect, test } from "vitest";
import { readFileSync } from "node:fs";
import {
  Engine,
  type Cue,
  type InkColours,
  type Level,
  type Marks,
} from "../../src/motion/engine.js";
import { parseHex } from "../../src/motion/color.js";
import { Grid } from "../../src/motion/grid.js";

type RefCell = [string, string, string, string];
type Ref = {
  colours: Record<keyof InkColours | "muted", string>;
  scenarios: {
    name: string;
    level: Level;
    frames: { t: number; running: boolean; cells: RefCell[] }[];
  }[];
};
const ref: Ref = JSON.parse(
  readFileSync(
    new URL("fixtures/motion-engine.json", import.meta.url),
    "utf8",
  ),
);
const ink: InkColours = {
  base: parseHex(ref.colours.base),
  surface: parseHex(ref.colours.surface),
  overlay: parseHex(ref.colours.overlay),
  glow: parseHex(ref.colours.glow),
  text: parseHex(ref.colours.text),
};

const W = 30;
const H = 8;
const AREA = { x: 0, y: 0, w: W, h: H };
const MESSAGE = { x: 0, y: 7, w: 14, h: 1 };
const rowRect = (y: number) => ({ x: 0, y, w: W, h: 1 });
const marks: Marks = {
  body: AREA,
  popup: { x: 5, y: 2, w: 12, h: 4 },
  message: MESSAGE,
  rows: Array.from({ length: H }, (_, y) => [`row:${y}`, rowRect(y)] as const),
};

/** motion/tests.rs `plain`: row text on the surface colour. */
function plain(): Grid {
  const g = new Grid(W, H);
  for (const c of g.cells) c.bg = ink.surface;
  for (let y = 0; y < H; y++) put(g, 0, y, `row ${y} with some text`, ink.text);
  return g;
}
/** motion/tests.rs `panels_frame`: two rounded panels side by side, rows of text inside. */
function panelsFrame(): Grid {
  const g = new Grid(W, H);
  for (const c of g.cells) c.bg = ink.surface;
  for (const [x0, w] of [[0, 12], [12, 18]] as const) {
    const x1 = x0 + w - 1;
    for (let x = x0; x <= x1; x++) { g.at(x, 0).ch = "─"; g.at(x, H - 1).ch = "─"; }
    for (let y = 1; y < H - 1; y++) { g.at(x0, y).ch = "│"; g.at(x1, y).ch = "│"; }
    g.at(x0, 0).ch = "╭"; g.at(x1, 0).ch = "╮"; g.at(x0, H - 1).ch = "╰"; g.at(x1, H - 1).ch = "╯";
    for (let y = 1; y < H - 1; y++) put(g, x0 + 1, y, `row ${y} text`, ink.text);
  }
  return g;
}
/** The frame a message leaves behind (motion/tests.rs): only the key hints, on the last row. */
function afterMessage(): Grid {
  const g = new Grid(W, H);
  for (const c of g.cells) c.bg = ink.surface;
  put(g, 0, 7, "keys keys keys", parseHex(ref.colours.muted));
  return g;
}
function put(g: Grid, x: number, y: number, s: string, fg: number) {
  [...s].forEach((ch, i) => {
    const c = g.at(x + i, y);
    c.ch = ch;
    c.fg = fg;
  });
}
// The reference's `Buffer::empty` + set_style(bg) leaves unwritten cells with a reset foreground; the fixture records it.

const cues: Record<string, Cue | undefined> = {
  plain: undefined,
  screen: { kind: "screen" },
  assemble: { kind: "screen" },
  page: { kind: "page" },
  popup: { kind: "popup" },
  message: { kind: "message" },
  message_gone: { kind: "message_gone" },
  changed_subtle: { kind: "changed", changed: ["row:3"], added: ["row:5"] },
  changed_full: { kind: "changed", changed: ["row:3"], added: ["row:5"] },
  shimmer: undefined,
};

/** Per channel; the reference quantises its colour cache by alpha/255, so a level may differ by a step or two. */
const TOL = 1;
function close(a: number, b: string, where: string) {
  const want = parseHex(b === "White" ? "#c0c0c0" : b); // a Reset foreground the sweep sets to Color::White
  if (want < 0 || a < 0)
    return expect(a < 0 ? "reset" : "rgb", where).toBe(
      want < 0 ? "reset" : "rgb",
    );
  for (const sh of [16, 8, 0])
    expect(
      Math.abs(((a >> sh) & 255) - ((want >> sh) & 255)),
      `${where} ${sh} ${a.toString(16)} vs ${b}`,
    ).toBeLessThanOrEqual(TOL);
}

describe("Ink motion matches the recorded tachyonfx effects", () => {
  for (const sc of ref.scenarios) {
    test(`${sc.name} (${sc.level})`, () => {
      let now = 0;
      const e = new Engine(sc.level, () => now);
      const waiting = sc.name === "shimmer";
      const mk: Marks = waiting ? { ...marks, loading: [AREA] } : marks;
      if (sc.name === "message_gone") e.draw(plain(), mk, ink);
      const cue = cues[sc.name];
      if (cue) e.cue(cue, ink);
      for (const fr of sc.frames) {
        now = fr.t;
        const g = sc.name === "message_gone" ? afterMessage() : sc.name === "assemble" ? panelsFrame() : plain();
        e.draw(g, mk, ink);
        expect(e.running(), `${sc.name} t=${fr.t} running`).toBe(fr.running);
        fr.cells.forEach(([sym, fg, bg], i) => {
          const c = g.cells[i]!;
          const where = `${sc.name} t=${fr.t} cell ${i % W},${Math.floor(i / W)}`;
          if (c.ch !== "") expect(c.ch, where).toBe(sym);
          close(c.fg, fg, `${where} fg`);
          close(c.bg, bg, `${where} bg`);
        });
      }
    });
  }
});
