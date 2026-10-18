// Real App + fast Output contract: controller-only marks cannot prove React owner cleanup or layout ordering.
import { vi } from "vitest";
vi.hoisted(() => { process.env.FORCE_COLOR = "3"; });
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { mount } from "../app/harness.js";
import { booksChrome } from "../chrome/fixture.js";
import { motion } from "../../src/motion/motion.js";
import { attachMotion } from "../../src/motion/attach.js";
import { FRAME_MS, type Level } from "../../src/motion/engine.js";
import { parseFrame } from "../../src/motion/frame.js";
import type { Rect } from "../../src/motion/grid.js";
import type { Node } from "../../src/nodes/view.js";
import type { MotionContext } from "../../src/protocol/generated/MotionContext.js";
import { applyLook, currentLook, defaultLook } from "../../src/text/look.js";
import { HitRegistry } from "../../src/state/ui-state.js";
import * as measurements from "../../src/workarea/hit.js";

const CONTEXT: MotionContext = { kind: "screen", plugin: "watch", screen: "cases", page: null, depth: 0, tab: null };
const slots = { base: "bg.base", surface: "bg.surface", overlay: "bg.overlay", glow: "accent.primary", text: "text.primary" };
const LOADING: Node = { node: "state", state: "loading", text: "Reading Watch cases" };
const DOCUMENT: Node = { node: "document", lines: [[{ text: "Vimes has the case." }]] };
const MENU = { id: "watch-actions", title: "Watch actions", question: false,
  items: [{ value: "watch:open", label: "open the case", hint: "", target: "chrome:menu:0" }], selected: 0 };
const REPLACEMENTS: Node[] = [
  DOCUMENT,
  { node: "list", rows: [{ target: "watch:vimes", text: [{ text: "Vimes has the case." }] }] },
  { node: "rich", blocks: [{ block: "line", text: [{ text: "Vimes has the case." }] }] },
];
const tree = (loading: boolean): Node => ({ node: "tree", columns: [], rows: [{ id: "vimes", name: [{ text: "Vimes has the case." }] }],
  ...(loading ? { note: { state: "loading", text: "Reading Watch cases" } } : {}) });
const screen = (body: Node) => ({ version: 1, scope: "watch", medium_from: 40, actions: [], panes: [
  { pane: "nav", title: "Cases", body: { node: "document", lines: [[{ text: "Ankh-Morpork" }]] } },
  { pane: "middle", title: "The Watch", body },
  { pane: "detail", title: "Notes", body: { node: "document", lines: [[{ text: "Nothing is missing." }]] } },
] });
const live: Array<() => void> = [];
const originalLook = currentLook();
beforeEach(() => vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] }));
afterEach(() => {
  live.splice(0).reverse().forEach((stop) => stop());
  motion.reset();
  applyLook(originalLook);
  vi.restoreAllMocks();
  vi.useRealTimers();
});

/** Flush actual React/Ink work and controlled timers until the observable condition holds. */
async function flushed(ok: () => boolean) {
  for (let i = 0; !ok() && i < 300; i++) await vi.advanceTimersByTimeAsync(10);
  expect(ok(), "the real App reached the requested output").toBe(true);
}

async function rig(columns = 80, rows = 24, context: unknown = CONTEXT, preloaded = false) {
  let now = 0, cursor = 1;
  applyLook(defaultLook("off"));
  motion.reset(() => now);
  const registry = new HitRegistry();
  const order: string[] = [];
  const committed = new WeakSet<object>();
  const commit = motion.commitScene.bind(motion);
  vi.spyOn(motion, "commitScene").mockImplementation((scene) => {
    commit(scene);
    if (!committed.has(scene)) { committed.add(scene); order.push("layout"); }
  });
  const applyFrame = motion.apply.bind(motion);
  const apply = vi.spyOn(motion, "apply").mockImplementation((...args) => {
    order.push("output");
    return applyFrame(...args);
  });
  const mark = vi.spyOn(motion, "markNode");
  const draw = motion.engine.draw.bind(motion.engine);
  const engineDraw = vi.spyOn(motion.engine, "draw").mockImplementation((...args) => {
    order.push("draw");
    return draw(...args);
  });
  const cue = vi.spyOn(motion, "cue");
  const ticks = vi.fn();
  live.push(motion.subscribe(ticks));
  const payload = (body: Node, level: Level = "full", menu = false) => ({ screen: screen(body), focus: "middle",
    chrome: booksChrome({ width: columns, height: rows, sources: [], ...(menu ? { menu: MENU } : {}) }),
    motion: { level, slots, timings: {}, cues: [], interaction: { context, processed_seq: null } } });
  const m = mount(columns, rows, { registry }, {}, undefined, ({ session, mouse, out, conn }) => {
    live.push(attachMotion(session, mouse, out));
    if (preloaded) {
      conn.receive({ type: "welcome", protocol: 1, view: 1, cursor: 0 });
      conn.receive({ type: "snapshot", cursor, section: null, ...payload(LOADING) });
    }
  });
  live.push(m.stop);
  const words = () => parseFrame(m.frame(), columns, rows).cells.map((c) => c.ch).join("");
  const clean = () => apply.mock.calls.at(-1)![0];
  const snapshot = async (body: Node, level: Level = "full", menu = false) => {
    const calls = apply.mock.calls.length;
    m.snapshot(payload(body, level, menu), ++cursor);
    await flushed(() => apply.mock.calls.length > calls && words().includes(body === LOADING || body.node === "tree" && body.note ? "Reading Watch cases" : "Vimes has the case."));
  };
  if (preloaded) await flushed(() => words().includes("Reading Watch cases") && motion.active);
  else {
    m.welcome();
    await snapshot(DOCUMENT);
  }
  const advance = async (ms: number) => {
    now += ms;
    const calls = apply.mock.calls.length;
    await vi.advanceTimersByTimeAsync(FRAME_MS);
    await flushed(() => apply.mock.calls.length > calls);
  };
  const loading = () => {
    const registered = mark.mock.calls.filter((call) => call[2]?.length).at(-1);
    expect(registered, "the real HitRegion measured and registered its loading node").toBeDefined();
    return { owner: registered![0], area: registered![2]![0]!, scene: registered![3] };
  };
  const settled = async () => {
    expect(motion.active).toBe(false);
    expect(parseFrame(m.frame(), columns, rows).cells).toEqual(parseFrame(clean(), columns, rows).cells);
    const count = ticks.mock.calls.length, frames = m.out.frames.length;
    await vi.advanceTimersByTimeAsync(5 * FRAME_MS);
    expect(ticks).toHaveBeenCalledTimes(count);
    expect(m.out.frames).toHaveLength(frames);
    expect(vi.getTimerCount(), "no loading or output timer survives its owner").toBe(0);
  };
  return { ...m, columns, rows, registry, apply, mark, engineDraw, cue, ticks, order, words, clean, snapshot, advance, loading, settled };
}

function movingOnlyInside(ansi: string, clean: string, columns: number, rows: number, area: Rect) {
  const actual = parseFrame(ansi, columns, rows), plain = parseFrame(clean, columns, rows);
  const glyphs = (cells: typeof actual.cells) => cells.map(({ ch, attr }) => ({ ch, attr }));
  expect(glyphs(actual.cells)).toEqual(glyphs(plain.cells));
  const changed: number[] = [];
  for (let i = 0; i < actual.cells.length; i++) {
    const cell = actual.cells[i]!, before = plain.cells[i]!;
    if (cell.fg !== before.fg || cell.bg !== before.bg) changed.push(i);
  }
  expect(changed.length, "the uncued loading shimmer actually changes cells").toBeGreaterThan(0);
  expect(changed.every((i) => i % columns >= area.x && i % columns < area.x + area.w &&
    Math.floor(i / columns) >= area.y && Math.floor(i / columns) < area.y + area.h)).toBe(true);
}

describe("uncued loading through committed App layout", () => {
  it.each([[80, 24], [160, 40]])("wakes at %ix%i only after layout and clears the same wrapper for document/list/rich bodies", async (columns, rows) => {
    const m = await rig(columns, rows);
    await m.settled();
    let probed = false;
    const commit = vi.mocked(motion.commitScene).getMockImplementation()!;
    vi.spyOn(motion, "commitScene").mockImplementation((scene) => {
      if (!probed && scene.cursor === 3) {
        probed = true;
        const { area, scene: measured } = m.loading();
        expect(measured).toBe(scene);
        expect(area.w).toBeGreaterThan(0);
        expect(area.h).toBeGreaterThan(0);
        const before = m.engineDraw.mock.calls.length, old = m.clean();
        expect(motion.apply(old, columns, rows), "prepared child geometry cannot paint before the actual parent commit").toBe(old);
        expect(m.engineDraw).toHaveBeenCalledTimes(before);
        expect(motion.active).toBe(false);
      }
      commit(scene);
    });
    let owner: string | undefined;
    for (const body of REPLACEMENTS) {
      await m.snapshot(LOADING);
      expect(motion.active).toBe(true);
      const loading = m.loading();
      owner ??= loading.owner;
      expect(loading.owner, "all bodies use the same real HitRegion owner").toBe(owner);
      const hits = m.registry.all().map((hit) => ({ ...hit }));
      await m.advance(300);
      movingOnlyInside(m.frame(), m.clean(), columns, rows, loading.area);
      expect(m.registry.all()).toEqual(hits);
      await m.snapshot(body);
      expect(m.words()).not.toContain("Reading Watch cases");
      expect(m.mark.mock.calls.filter((call) => call[0] === owner).at(-1)?.slice(1, 3)).toEqual([[], []]);
      await m.settled();
    }
    expect(probed).toBe(true);
    expect(m.cue).not.toHaveBeenCalled();
  });

  it.each([null, { kind: "screen", plugin: "", screen: "cases", page: null, depth: 0, tab: null }])("loads a tree note without a usable semantic interaction context: %j", async (context) => {
    const m = await rig(80, 24, context);
    await m.snapshot(tree(true));
    expect(motion.active).toBe(true);
    const { area } = m.loading();
    const hits = m.registry.all().map((hit) => ({ ...hit }));
    await m.advance(300);
    movingOnlyInside(m.frame(), m.clean(), 80, 24, area);
    expect(m.registry.all()).toEqual(hits);
    expect(motion.interactions.running).toBe(false);
    await m.snapshot(tree(false));
    await m.settled();
    expect(m.cue).not.toHaveBeenCalled();
  });

  it("clears a loading owner's marks when measurement disappears or motion turns off, and disposes on unmount", async () => {
    const m = await rig();
    await m.snapshot(LOADING);
    expect(motion.active).toBe(true);
    const { owner, area } = m.loading();
    const actualMeasure = measurements.screenRect;
    const measure = vi.spyOn(measurements, "screenRect");
    // Inject unavailable/invalid measurement results; restore actual Yoga measurement before each retry.
    for (const unusable of [undefined, { ...area, w: 0 }, { ...area, h: 0 }, { ...area, x: area.x + 0.5 }]) {
      measure.mockReturnValue(unusable);
      await m.snapshot(LOADING);
      expect(m.mark.mock.calls.filter((call) => call[0] === owner).at(-1)?.slice(1, 3)).toEqual([[], []]);
      await m.settled();
      measure.mockImplementation(actualMeasure);
      await m.snapshot(LOADING);
      expect(motion.active).toBe(true);
    }
    measure.mockRestore();
    await m.snapshot(LOADING, "off");
    expect(m.mark.mock.calls.filter((call) => call[0] === owner).at(-1)?.slice(1, 3)).toEqual([[], []]);
    await m.settled();
    await m.snapshot(LOADING);
    expect(motion.active).toBe(true);
    m.stop();
    expect(motion.active).toBe(false);
    const ticks = m.ticks.mock.calls.length;
    await vi.advanceTimersByTimeAsync(5 * FRAME_MS);
    expect(m.ticks).toHaveBeenCalledTimes(ticks);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("keeps loading behind a real menu idle and resumes only after the overlay closes", async () => {
    const m = await rig();
    await m.snapshot(LOADING, "full", true);
    expect(m.words()).toContain("Watch actions");
    await m.settled();
    await m.snapshot(LOADING);
    expect(motion.active).toBe(true);
    await m.advance(300);
    await m.snapshot(LOADING, "full", true);
    await m.settled();
    await m.snapshot(LOADING);
    expect(motion.active).toBe(true);
    await m.snapshot(DOCUMENT);
    await m.settled();
  });

  it.each([CONTEXT, null])("starts a preloaded loading mount only from its actual committed layout: %j", async (context) => {
    const m = await rig(80, 24, context, true);
    expect(m.order.indexOf("output"), "Ink's first preloaded output precedes layout").toBeLessThan(m.order.indexOf("layout"));
    expect(m.order.indexOf("draw"), "that first output cannot discover uncommitted loading marks").toBeGreaterThan(m.order.indexOf("layout"));
    expect(m.order.slice(m.order.indexOf("layout") + 1, m.order.indexOf("draw"))).toContain("output");
    expect(motion.active).toBe(true);
    const { area } = m.loading();
    await m.advance(300);
    movingOnlyInside(m.frame(), m.clean(), 80, 24, area);
    await m.snapshot(DOCUMENT);
    await m.settled();
    expect(m.cue).not.toHaveBeenCalled();
  });
});
