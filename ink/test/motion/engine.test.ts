// Ink's motion engine keeps to motion.rs's rules (crates/radar-core/src/motion/tests.rs): off draws what was drawn,
// each effect runs its time and no longer, subtle has fewer cues, input drops what runs, the frame after an effect is
// exactly the one without motion, a slow terminal turns motion off. The cells themselves are parity.test.ts's.
import { afterEach, describe, expect, test, vi } from "vitest";
import {
  Engine,
  FRAME_MS,
  GLOW_MS,
  MESSAGE_GONE_MS,
  MESSAGE_MS,
  PAGE_MS,
  POPUP_MS,
  SCREEN_MS,
  cueRuns,
  parseLevel,
  type Cue,
  type InkColours,
  type Level,
  type Marks,
} from "../../src/motion/engine.js";
import { Grid } from "../../src/motion/grid.js";
import { parseFrame, writeFrame } from "../../src/motion/frame.js";
import * as frames from "../../src/motion/frame.js";
import type { MotionScene, PaneMark, TreeMark } from "../../src/motion/interaction.js";
import { Motion, isInput, motion, installMotionOutput } from "../../src/motion/motion.js";
import { parseHex } from "../../src/motion/color.js";

const ink: InkColours = {
  base: 0x0a0812,
  surface: 0x12101a,
  overlay: 0x1a162a,
  glow: 0xe135ff,
  text: 0xf8f8f2,
};
const W = 30;
const H = 8;
const AREA = { x: 0, y: 0, w: W, h: H };
const POPUP = { x: 5, y: 2, w: 12, h: 4 };
const row = (y: number) => ({ x: 0, y, w: W, h: 1 });
const marks: Marks = {
  body: AREA,
  popup: POPUP,
  message: row(7),
  rows: Array.from({ length: H }, (_, y) => [`row:${y}`, row(y)] as const),
};

function plain(): Grid {
  const g = new Grid(W, H);
  for (const c of g.cells) c.bg = ink.surface;
  for (let y = 0; y < H; y++)
    [...`row ${y} with some text`].forEach((ch, x) =>
      Object.assign(g.at(x, y), { ch, fg: ink.text }),
    );
  return g;
}
const sameCells = (a: Grid, b: Grid) =>
  JSON.stringify(a.cells) === JSON.stringify(b.cells);
function rig(level: Level) {
  const t = { now: 0 };
  const e = new Engine(level, () => t.now);
  const draw = (m: Marks = marks) => {
    const g = plain();
    e.draw(g, m, ink);
    return g;
  };
  return { e, t, draw };
}
const everyCue: Cue[] = [
  { kind: "screen" },
  { kind: "page" },
  { kind: "popup" },
  { kind: "message" },
  { kind: "message_gone" },
  { kind: "changed", changed: ["row:1"], added: ["row:2"] },
];

describe("motion engine", () => {
  test("off draws exactly what was drawn, whatever happens, and asks for no frame", () => {
    const { e, t, draw } = rig("off");
    for (const c of everyCue) e.cue(c, ink);
    expect(e.running()).toBe(false);
    expect(e.nextFrame()).toBeUndefined();
    for (let i = 0; i < 3; i++) {
      expect(sameCells(draw({ ...marks, loading: [AREA] }), plain())).toBe(
        true,
      );
      t.now += 50;
    }
    expect(e.running()).toBe(false);
  });

  test("each effect runs for its time and no longer, then one plain frame follows", () => {
    for (const [cue, ms] of [
      [{ kind: "screen" }, SCREEN_MS],
      [{ kind: "page" }, PAGE_MS],
      [{ kind: "popup" }, POPUP_MS],
      [{ kind: "message" }, MESSAGE_MS],
    ] as [Cue, number][]) {
      const { e, t, draw } = rig("full");
      e.cue(cue, ink);
      expect(e.nextFrame()).toBe(FRAME_MS);
      draw();
      t.now += ms - 20;
      draw();
      expect(e.running(), `${cue.kind} still runs before ${ms} ms`).toBe(true);
      t.now += 20;
      draw();
      expect(e.running(), "one plain frame follows the last").toBe(true);
      expect(
        sameCells(draw(), plain()),
        `${cue.kind} ends exactly as drawn`,
      ).toBe(true);
      expect(e.running(), `${cue.kind} is over at ${ms} ms`).toBe(false);
      expect(e.nextFrame()).toBeUndefined();
      expect(ms).toBeGreaterThanOrEqual(100);
      expect(ms).toBeLessThanOrEqual(450);
    }
  });

  test("a popup moves only its own area", () => {
    const { e, draw } = rig("subtle");
    e.cue({ kind: "popup" }, ink);
    const first = draw();
    const p = plain();
    first.cells.forEach((c, i) => {
      const x = i % W;
      const y = Math.floor(i / W);
      const inside =
        x >= POPUP.x &&
        x < POPUP.x + POPUP.w &&
        y >= POPUP.y &&
        y < POPUP.y + POPUP.h;
      if (!inside)
        expect(c, `only the popup moves, not ${x},${y}`).toEqual(p.cells[i]);
    });
    expect(first.at(POPUP.x, POPUP.y).fg).toBe(ink.base);
  });

  test("subtle has switches, popups and the glow only; full has everything but message_gone without a message", () => {
    for (const cue of everyCue) {
      const subtle = rig("subtle").e;
      subtle.cue(cue, ink);
      expect(subtle.running(), `${cue.kind} at subtle`).toBe(
        cue.kind === "screen" || cue.kind === "popup" || cue.kind === "changed",
      );
      expect(cueRuns(cue, "subtle")).toBe(subtle.running());
      const full = rig("full").e;
      full.cue(cue, ink);
      expect(full.running(), `${cue.kind} at full`).toBe(
        cue.kind !== "message_gone",
      );
    }
  });

  test("a key during an effect drops it and the next frame is plain", () => {
    const { e, t, draw } = rig("full");
    e.cue({ kind: "screen" }, ink);
    expect(sameCells(draw(), plain())).toBe(false);
    t.now += 30;
    e.input();
    expect(e.running(), "one frame to put it right").toBe(true);
    expect(sameCells(draw(), plain())).toBe(true);
    expect(e.running()).toBe(false);
  });

  test("rows a sync changed glow where they are drawn and fade; a row not shown is let go", () => {
    const { e, t, draw } = rig("subtle");
    e.cue({ kind: "changed", changed: ["row:3", "row:gone"], added: [] }, ink);
    const first = draw();
    expect(first.at(20, 3).bg).not.toBe(ink.surface);
    expect(first.at(0, 3).fg, "the background, not the text").toBe(ink.text);
    for (const y of [0, 1, 2, 4, 5, 6, 7])
      expect(first.at(20, y).bg, `row ${y} does not`).toBe(ink.surface);
    t.now += GLOW_MS;
    draw();
    expect(sameCells(draw(), plain())).toBe(true);
    expect(e.running()).toBe(false);
    // A row that moves glows where it now is.
    const m = rig("subtle");
    m.e.cue({ kind: "changed", changed: ["row:x"], added: [] }, ink);
    const at = (y: number): Marks => ({ rows: [["row:x", row(y)]] });
    expect(m.draw(at(1)).at(20, 1).bg).not.toBe(ink.surface);
    m.t.now += 40;
    const moved = m.draw(at(5));
    expect(moved.at(20, 5).bg).not.toBe(ink.surface);
    expect(moved.at(20, 1).bg).toBe(ink.surface);
    m.t.now += 40;
    m.draw({});
    m.draw({});
    expect(m.e.running(), "rows no longer drawn are let go").toBe(false);
  });

  test("new rows sweep in at full and glow at subtle", () => {
    const cue: Cue = { kind: "changed", changed: [], added: ["row:4"] };
    const f = rig("full");
    f.e.cue(cue, ink);
    const first = f.draw();
    expect(first.at(0, 4).fg, "the sweep starts hidden").toBe(ink.surface);
    expect(first.at(25, 4).fg, "text comes in").not.toBe(-1);
    const s = rig("subtle");
    s.e.cue(cue, ink);
    const g = s.draw();
    expect(g.at(0, 4).fg).toBe(ink.text);
    expect(g.at(0, 4).bg).not.toBe(ink.surface);
  });

  test("a waiting panel shimmers at full while marked; input does not stop it; not at subtle", () => {
    const { e, t, draw } = rig("full");
    const wait: Marks = { loading: [AREA] };
    draw(wait);
    expect(e.running(), "frames keep coming while it waits").toBe(true);
    let changed = false;
    for (let i = 0; i < 10; i++) {
      t.now += 100;
      changed ||= !sameCells(draw(wait), plain());
    }
    expect(changed, "a band of light crosses it").toBe(true);
    e.input();
    draw(wait);
    expect(e.running(), "a key does not stop the wait showing").toBe(true);
    draw({});
    draw({});
    expect(e.running(), "the data came: it stops").toBe(false);
    const subtle = rig("subtle");
    expect(sameCells(subtle.draw(wait), plain())).toBe(true);
    expect(subtle.e.running()).toBe(false);
  });

  test("a message that leaves fades out from what was shown", () => {
    const { e, t } = rig("full");
    const shown = plain();
    e.draw(shown, { message: row(7) }, ink);
    e.cue({ kind: "message_gone" }, ink);
    const next = new Grid(W, H);
    [...`keys keys keys`].forEach((ch, x) =>
      Object.assign(next.at(x, 7), { ch, fg: 0x888888, bg: ink.surface }),
    );
    e.draw(next, {}, ink);
    expect(next.at(0, 7).ch, "the old text, fading").toBe("r");
    t.now += MESSAGE_GONE_MS;
    e.draw(plain(), {}, ink);
    e.draw(plain(), {}, ink);
    expect(e.running()).toBe(false);
  });

  test("a slow terminal turns motion off for the session", () => {
    const { e } = rig("full");
    e.cue({ kind: "screen" }, ink);
    expect(e.drewIn(100)).toBeUndefined();
    expect(e.drewIn(5), "a fast frame starts again").toBeUndefined();
    expect(e.drewIn(100)).toBeUndefined();
    expect(e.drewIn(100)).toBeUndefined();
    expect(e.drewIn(100), "the third slow one in a row").toBeDefined();
    expect(e.level).toBe("off");
    expect(e.running()).toBe(false);
    e.setLevel("full");
    expect(e.level, "it stays off").toBe("off");
  });

  test("the level words are the config's", () => {
    expect(parseLevel("full")).toBe("full");
    expect(parseLevel("off")).toBe("off");
    expect(parseLevel("none")).toBe("off");
    expect(parseLevel("whatever")).toBe("subtle");
  });
});

describe("motion over Ink's frame string", () => {
  const frame = [
    "\x1b[48;2;18;16;26m\x1b[38;2;248;248;242mrow 0\x1b[1m bold\x1b[22m\x1b[0m",
    "",
    "\x1b[31mred\x1b[39m 日本",
  ].join("\n");

  test("the frame reads into cells and writes back the same picture", () => {
    const g = parseFrame(frame, 20, 4);
    expect(g.at(0, 0)).toMatchObject({ ch: "r", fg: 0xf8f8f2, bg: 0x12101a });
    expect(g.at(6, 0).attr).toBe("1");
    expect(g.at(0, 2).attr).toBe("31");
    expect(g.at(4, 2).ch).toBe("日");
    expect(g.at(5, 2).ch, "the second half of a wide one").toBe("");
    const again = parseFrame(writeFrame(g, frame, [g.area]), 20, 4);
    expect(again.cells).toEqual(g.cells);
  });

  test("nothing runs: the frame is the very string Ink gave, nothing parsed", () => {
    const m = new Motion("subtle", () => 0);
    expect(m.apply(frame, 20, 4)).toBe(frame);
  });

  test("a running fade rewrites colours and leaves the text alone", () => {
    let now = 0;
    const m = new Motion("subtle", () => now);
    m.setMarks({ body: { x: 0, y: 0, w: 20, h: 4 } });
    m.cue({ kind: "screen" });
    expect(m.active).toBe(true);
    const out = m.apply(frame, 20, 4);
    expect(out).not.toBe(frame);
    const text = (f: string) =>
      f
        .replace(/\x1b\[[0-9;]*m/g, "")
        .split("\n")
        .map((l) => l.trimEnd())
        .join("\n")
        .trimEnd();
    expect(text(out)).toBe(text(frame));
    now += SCREEN_MS;
    m.apply(frame, 20, 4);
    expect(m.apply(frame, 20, 4), "the frame after is Ink's own again").toBe(
      frame,
    );
    expect(m.active).toBe(false);
  });

  test("motion going off with an effect on screen draws one more frame, so the plain one is left there", async () => {
    let now = 0;
    const m = new Motion("subtle", () => now);
    m.setMarks({ body: { x: 0, y: 0, w: 20, h: 4 } });
    let ticks = 0;
    m.subscribe(() => ticks++);
    m.cue({ kind: "screen" });
    m.apply(frame, 20, 4);
    expect(m.active).toBe(true);
    m.setLevel("off");
    expect(m.active, "nothing runs now").toBe(false);
    await new Promise((r) => setTimeout(r, 80));
    expect(ticks, "but a frame was asked for").toBeGreaterThan(0);
  });

  test("pointer motion is not input; keys, clicks and pastes are", () => {
    expect(isInput("\x1b[<35;10;5M")).toBe(false);
    expect(isInput("\x1b[<35;10;5M\x1b[<35;11;5M")).toBe(false);
    expect(isInput("\x1b[<0;10;5M")).toBe(true);
    expect(isInput("\x1b[<64;10;5M")).toBe(true);
    expect(isInput("j")).toBe(true);
    expect(isInput("\x1b[<35;10;5Mj")).toBe(true);
    expect(parseHex("#0a0812")).toBe(0x0a0812);
  });
});

describe("idle cost", () => {
  test("ticks only while an effect runs: none before a cue, one per frame while it runs, none after it ends", async () => {
    const frame = "\x1b[38;2;248;248;242mrow\x1b[0m\nsecond";
    let now = 0;
    const m = new Motion("subtle", () => now);
    m.setMarks({ body: { x: 0, y: 0, w: 20, h: 2 } });
    let ticks = 0;
    // The app redraws on a tick: Ink asks Output.get, which calls apply.
    m.subscribe(() => {
      ticks++;
      now += 30;
      m.apply(frame, 20, 2);
    });
    await new Promise((r) => setTimeout(r, 120));
    expect(ticks, "idle: no tick").toBe(0);
    expect(m.apply(frame, 20, 2), "idle: the very string").toBe(frame);
    m.cue({ kind: "screen" });
    m.apply(frame, 20, 2);
    await new Promise((r) => setTimeout(r, 600));
    expect(ticks, "ticked while it ran").toBeGreaterThan(3);
    expect(m.active, "and it is over").toBe(false);
    const done = ticks;
    await new Promise((r) => setTimeout(r, 400));
    expect(ticks, "no tick once it ended").toBe(done);
  });
});


// Real scheduler/output ownership belongs to the renderer contract, not the pure effect-math tier.
describe("motion controller disposal and inactive guards", () => {
  afterEach(() => {
    installMotionOutput(motion);
    motion.reset();
    vi.useRealTimers();
    vi.restoreAllMocks();
  });
  const scene: MotionScene = { cursor: 1, context: { kind: "history" }, local: false, blocked: false };
  const plain = "\x1b[38;2;248;248;242mWatch\x1b[0m";

  const pane = (name = "nav"): PaneMark => ({ pane: name, owns: name === "nav", area: { x: 0, y: 0, w: 1, h: 1 }, keep: [] });
  const tree = (slot = "body"): TreeMark => ({ pane: "nav", slot, area: { x: 0, y: 0, w: 1, h: 1 }, rows: [{ path: ["watch"], y: 0 }], open: [] });
  type RegistrationCase = { name: string; panes?: PaneMark[]; trees?: TreeMark[] } & ({ kind: "pane"; bad: PaneMark } | { kind: "tree"; bad: TreeMark });
  const invalidRegistrations: RegistrationCase[] = [
    { name: "oversize pane key", kind: "pane", bad: { ...pane(), pane: "x".repeat(1025) } },
    { name: "65 protected rectangles", kind: "pane", bad: { ...pane(), keep: Array.from({ length: 65 }, () => AREA) } },
    { name: "duplicate pane key", kind: "pane", panes: [pane()], bad: pane() },
    { name: "33 pane marks", kind: "pane", panes: Array.from({ length: 32 }, (_, i) => pane(`watch-${i}`)), bad: pane() },
    { name: "65,537 total perimeter cells", kind: "pane", panes: [pane("watch")], bad: { ...pane(), area: { x: 0, y: 0, w: 1, h: 65536 } } },
    { name: "oversize tree key", kind: "tree", bad: { ...tree(), pane: "x".repeat(1025) } },
    { name: "oversize tree slot", kind: "tree", bad: tree("x".repeat(1025)) },
    { name: "4,097-byte row ID in one cell", kind: "tree", bad: { ...tree(), rows: [{ path: ["x".repeat(4097)], y: 0 }] } },
    { name: "65 row-path components", kind: "tree", bad: { ...tree(), rows: [{ path: Array.from({ length: 65 }, () => ""), y: 0 }] } },
    { name: "oversize open path", kind: "tree", bad: { ...tree(), open: [["x".repeat(4097)]] } },
    { name: "rows beyond viewport", kind: "tree", bad: { ...tree(), rows: [{ path: ["a"], y: 0 }, { path: ["b"], y: 1 }] } },
    { name: "open paths beyond rows", kind: "tree", bad: { ...tree(), open: [["a"], ["b"]] } },
    { name: "duplicate tree key", kind: "tree", trees: [tree()], bad: tree() },
    { name: "17 tree marks", kind: "tree", trees: Array.from({ length: 16 }, (_, i) => tree(`watch-${i}`)), bad: tree() },
    { name: "65,537 total path bytes", kind: "tree", trees: [{ ...tree("watch"), area: { x: 0, y: 0, w: 1, h: 16 }, rows: Array.from({ length: 16 }, (_, y) => ({ path: ["x".repeat(4096)], y })) }], bad: { ...tree(), rows: [{ path: ["x"], y: 0 }] } },
  ];

  test.each(invalidRegistrations)("registration rejects $name before storing the mark or first output", ({ kind, bad, panes = [], trees = [] }) => {
    vi.useFakeTimers();
    const m = new Motion("full", () => 0), current = { ...scene };
    const parse = vi.spyOn(frames, "parseFrame"), observe = vi.spyOn(m.interactions, "observe");
    m.prepareScene(current);
    panes.forEach((mark, i) => m.markPane(`pane-${i}`, current, mark));
    trees.forEach((mark, i) => m.markTree(`tree-${i}`, current, mark));
    m.apply(plain, W, H); // The pre-layout clean output must also be discarded on rejection.
    const stored = vi.spyOn(Map.prototype, "set"), serialized = vi.spyOn(JSON, "stringify");
    let retained: boolean, serializationCount: number;
    try {
      if (kind === "pane") m.markPane("bad", current, bad);
      else m.markTree("bad", current, bad);
      retained = stored.mock.calls.some(([, value]) => value?.mark === bad);
      serializationCount = serialized.mock.calls.length;
    } finally { stored.mockRestore(); serialized.mockRestore(); }
    expect(retained).toBe(false);
    expect(serializationCount).toBe(0);
    // A later valid child cannot rehabilitate a rejected render or preserve its earlier marks.
    m.markPane("later", current, pane("detail"));
    m.commitScene(current);
    expect(observe).not.toHaveBeenCalled();
    expect(m.apply(plain, W, H)).toBe(plain);
    expect(observe).toHaveBeenCalledExactlyOnceWith(plain, W, H, undefined, true);
    expect(parse).not.toHaveBeenCalled();
    expect(m.active).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });

  test.each([
    { kind: "screen", plugin: "watch", screen: "patrol", page: null, depth: 65, tab: null },
    { kind: "settings", plugin: null, section: "x".repeat(16385) },
  ] as const)("prepare rejects over-budget $kind context before marks or first-output retention", (context) => {
    const m = new Motion("full", () => 0), current: MotionScene = { ...scene, context };
    const parse = vi.spyOn(frames, "parseFrame"), observe = vi.spyOn(m.interactions, "observe");
    m.prepareScene(current);
    m.markPane("nav", current, pane());
    m.apply(plain, W, H);
    m.commitScene(current);
    expect(observe).toHaveBeenCalledExactlyOnceWith(plain, W, H, undefined, true);
    expect(parse).not.toHaveBeenCalled();
    expect(m.active).toBe(false);
  });

  test.each([
    { name: "unkeyed", context: null },
    { name: "oversize", context: { kind: "settings", plugin: null, section: "x".repeat(16385) } },
  ] as const)("a valid to $name replacement clears the prior scene and still draws/settles ordinary cues", ({ context }) => {
    vi.useFakeTimers();
    let now = 0;
    const m = new Motion("full", () => now), previous = { ...scene };
    m.setMarks({ body: AREA });
    m.prepareScene(previous);
    m.markPane("nav", previous, pane());
    m.commitScene(previous);
    m.apply(plain, W, H);
    const next: MotionScene = { ...scene, cursor: 2, context };
    m.cue({ kind: "screen" });
    m.prepareScene(next);
    m.commitScene(next);
    const parse = vi.spyOn(frames, "parseFrame"), draw = vi.spyOn(m.engine, "draw"), observe = vi.spyOn(m.interactions, "observe");
    m.apply(plain, W, H);
    expect(observe).toHaveBeenCalledExactlyOnceWith(plain, W, H, undefined, true);
    expect(parse).toHaveBeenCalledTimes(1);
    expect(draw).toHaveBeenCalledTimes(1);
    expect(m.active).toBe(true);
    now = SCREEN_MS + 1;
    m.apply(plain, W, H);
    expect(m.apply(plain, W, H)).toBe(plain);
    expect(m.active).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
    // No output from the rejected replacement was held under the prior scene's identity.
    observe.mockClear();
    m.commitScene(previous);
    expect(observe).not.toHaveBeenCalled();
  });

  test.each(["off", "subtle"] as const)("%s skips interaction geometry, retention and ANSI parsing", (level) => {
    vi.useFakeTimers();
    const m = new Motion(level, () => 0);
    const parse = vi.spyOn(frames, "parseFrame");
    const observe = vi.spyOn(m.interactions, "observe");
    const collect = vi.fn(() => { throw new Error("inactive geometry must not be collected"); });
    m.marksFrom(collect);
    m.prepareScene(scene);
    m.markPane("watch", scene, { pane: "nav", owns: true, area: AREA, keep: [] });
    m.commitScene(scene);
    expect(m.apply(plain, W, H)).toBe(plain);
    expect(parse).not.toHaveBeenCalled();
    expect(observe).not.toHaveBeenCalled();
    expect(collect).not.toHaveBeenCalled();
    expect(m.active).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });

  test("a clean frame after disabling clears settling state and an already queued tick", () => {
    vi.useFakeTimers();
    const m = new Motion("full", () => 0);
    const ticks = vi.fn();
    m.subscribe(ticks);
    m.setMarks({ body: AREA });
    m.cue({ kind: "screen" });
    m.input(); // This leaves a final clean-frame request in the old engine.
    m.setLevel("off");
    expect(m.active).toBe(false);
    const parse = vi.spyOn(frames, "parseFrame");
    expect(m.apply(plain, W, H)).toBe(plain);
    expect(parse).not.toHaveBeenCalled();
    expect(vi.getTimerCount()).toBe(0);
    vi.advanceTimersByTime(90);
    expect(ticks).not.toHaveBeenCalled();
    expect(m.active).toBe(false);
  });

  test("replacing the output target disposes its timer, scene and pending input generation", () => {
    vi.useFakeTimers();
    const old = new Motion("full", () => 0), next = new Motion("off", () => 0);
    installMotionOutput(old);
    const ticks = vi.fn();
    old.subscribe(ticks);
    old.prepareScene(scene);
    old.commitScene(scene);
    old.input();
    old.intent(3, 1);
    old.cue({ kind: "screen" });
    expect(old.interactions.inputGeneration).toBe(1);
    expect(vi.getTimerCount()).toBe(1);
    installMotionOutput(next);
    expect(old.engine.level).toBe("off");
    expect(old.active).toBe(false);
    expect(old.interactions.inputGeneration).toBe(0);
    expect(vi.getTimerCount()).toBe(0);
    vi.advanceTimersByTime(90);
    expect(ticks).not.toHaveBeenCalled();
  });

  test("slow-terminal shutdown retains exactly one clean-frame request", () => {
    vi.useFakeTimers();
    const m = new Motion("subtle", () => 0);
    m.setMarks({ body: AREA });
    m.cue({ kind: "screen" });
    const notice = vi.fn();
    const ticks = vi.fn(() => { expect(m.apply(plain, W, H)).toBe(plain); });
    m.onSlow(notice);
    m.subscribe(ticks);
    vi.spyOn(m.engine, "drewIn").mockImplementationOnce(() => {
      m.engine.setLevel("off");
      return "slow terminal";
    });
    m.apply(plain, W, H);
    expect(notice).toHaveBeenCalledExactlyOnceWith("slow terminal");
    expect(vi.getTimerCount()).toBe(1);
    vi.advanceTimersByTime(FRAME_MS);
    expect(ticks).toHaveBeenCalledTimes(1);
    expect(vi.getTimerCount()).toBe(0);
    expect(m.active).toBe(false);
  });
});
