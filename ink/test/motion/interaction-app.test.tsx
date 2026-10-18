// Real App + fast Output contract. Pure math tests cannot establish React commit/ANSI source ordering.
import { vi } from "vitest";
vi.hoisted(() => { process.env.FORCE_COLOR = "3"; });
import { afterEach, describe, expect, it } from "vitest";
import { mount, tick, until } from "../app/harness.js";
import { booksChrome } from "../chrome/fixture.js";
import { motion } from "../../src/motion/motion.js";
import { attachMotion } from "../../src/motion/attach.js";
import { MouseBus } from "../../src/input/stdin-tap.js";
import { applyLook, currentLook, defaultLook } from "../../src/text/look.js";
import { parseFrame } from "../../src/motion/frame.js";
import type { MotionContext } from "../../src/protocol/generated/MotionContext.js";
import type { MotionInteraction } from "../../src/protocol/generated/MotionInteraction.js";
import { parseHex } from "../../src/motion/color.js";
import { STATES } from "../dock/states.js";
import { HitRegistry } from "../../src/state/ui-state.js";
import fs from "node:fs";
import path from "node:path";

const CONTEXT: MotionContext = { kind: "screen", plugin: "watch", screen: "cases", page: null, depth: 0, tab: null };
const slots = { base: "bg.base", surface: "bg.surface", overlay: "bg.overlay", glow: "accent.primary", text: "text.primary" };
const tree = (open: boolean) => ({ node: "tree", columns: [], name_width: 30, selected: ["watch"], open: open ? [["watch"]] : [], rows: [
  { id: "watch", name: [{ text: "The Watch" }], children: [
    { id: "vimes", name: [{ text: "Vimes 界", strong: true }] },
    { id: "carrot", name: [{ text: "Carrot é" }] },
    { id: "angua", name: [{ text: "Angua" }] },
    { id: "detritus", name: [{ text: "Detritus" }] },
  ] },
  { id: "tail", name: [{ text: "The next case" }] },
] });
const screen = (open: boolean) => ({ version: 1, scope: "watch", medium_from: 40, actions: [], panes: [
  { pane: "nav", title: "Cases", body: { node: "document", lines: [[{ text: "Ankh-Morpork" }]] } },
  { pane: "middle", title: "The Watch", body: tree(open) },
  { pane: "detail", title: "Notes", body: { node: "document", lines: [[{ text: "Nothing is missing." }]] } },
] });
const live: Array<() => void> = [];
const originalLook = currentLook();
afterEach(() => {
  live.splice(0).reverse().forEach((stop) => stop());
  motion.reset();
  applyLook(originalLook);
  vi.restoreAllMocks();
});

async function rig(columns = 80, rows = 24, level = "full") {
  let now = 0;
  applyLook(defaultLook("off"));
  motion.reset(() => now);
  const registry = new HitRegistry();
  const m = mount(columns, rows, { registry });
  // The real App calls Session.input for every token; the raw bus contract has its own byte-forwarding test.
  const off = attachMotion(m.session, new MouseBus(), m.out);
  live.push(m.stop, off);
  m.welcome();
  const snapshot = async (open: boolean, cursor: number, interaction: MotionInteraction, focus = "middle", over: Record<string, unknown> = {}) => {
    const frames = m.out.frames.length;
    m.snapshot({ screen: screen(open), focus, chrome: booksChrome({ width: columns, height: rows, sources: [] }), motion: { level, slots, timings: {}, interaction }, ...over }, cursor);
    await until(() => m.out.frames.length > frames);
    expect(m.out.frames.length).toBeGreaterThan(frames);
  };
  await snapshot(true, 1, { context: CONTEXT, processed_seq: null });
  const words = () => parseFrame(m.frame(), columns, rows).cells.map((c) => c.ch).join("");
  const advance = async (ms: number) => {
    now = ms;
    const frames = m.out.frames.length;
    (m.session as unknown as { set(patch: object): void }).set({});
    await tick(25);
    // An exact same clean frame need not result in a terminal write.
    expect(m.out.frames.length).toBeGreaterThanOrEqual(frames);
  };
  const command = async (input: string) => {
    const count = m.intents().length;
    m.type(input);
    await until(() => m.intents().length > count);
    expect(m.intents().length).toBe(count + 1);
    return m.intents().at(-1)!;
  };
  return { ...m, registry, snapshot, words, advance, command };
}

/** Optional offline evidence is the actual App output above, not a separately composed illustration. */
function capture(name: string, ansi: string, width: number, height: number) {
  const directory = process.env.RADAR_INTERACTION_SHOTS;
  if (!directory) return;
  fs.mkdirSync(directory, { recursive: true });
  fs.writeFileSync(path.join(directory, `ink-${name}-${width}x${height}.ansi`), ansi);
  fs.writeFileSync(path.join(directory, `ink-${name}-${width}x${height}.json`), JSON.stringify({ source: "actual App + fast Output", width, height, cells: parseFrame(ansi, width, height).cells }));
}

const folding = (seq: number, cursor: number, before: boolean, after: boolean): MotionInteraction => ({
  context: CONTEXT, processed_seq: seq, origin_seq: seq,
  fold: { context: CONTEXT, pane: "middle", slot: "body", path: ["watch"], before_open: before, after_open: after, origin_seq: seq, origin_cursor: cursor },
});

describe("committed interaction motion through App and fast Output", () => {
  it.each([[80, 24], [160, 40]])("retains disappearing descendants after an idle %ix%i frame, shifts tails, then restores exact clean output", async (columns, rows) => {
    const m = await rig(columns, rows);
    expect(m.words()).toContain("Detritus");
    const idleFrames = m.out.frames.length;
    await tick(45);
    expect(m.out.frames.length).toBe(idleFrames);
    const sent = await m.command(" ");
    expect(sent.intent).toEqual({ intent: "fold", target: "watch", open: false });
    await m.snapshot(false, 2, folding(sent.seq, 1, true, false));
    expect(motion.interactions.running, "the first changed output uses matching marks and the prior clean ANSI").toBe(true);
    expect(m.words()).toContain("Detritus");
    expect(m.words()).toContain("The next case");
    capture("close-0", m.frame(), columns, rows);
    await m.advance(80);
    capture("close-80", m.frame(), columns, rows);
    expect(m.words()).toContain("Vimes 界");
    expect(m.words()).toContain("Carrot é");
    expect(m.words()).not.toContain("Detritus");
    expect(m.words()).not.toContain("Angua");
    expect(m.words()).toContain("The next case");
    await m.advance(160);
    capture("close-160", m.frame(), columns, rows);
    expect(m.words()).not.toContain("Vimes");
    const clean = m.frame();
    await m.advance(190);
    expect(motion.active).toBe(false);
    expect(m.frame()).toBe(clean);
    // A new fold is routed once from the actual compact model, then opens from the clean compact baseline.
    const reopened = await m.command(" ");
    expect(reopened.intent).toEqual({ intent: "fold", target: "watch", open: true });
    await m.snapshot(true, 3, folding(reopened.seq, 2, false, true));
    expect(m.words()).not.toContain("Vimes");
    await m.advance(270);
    expect(m.words()).toContain("Carrot é");
    expect(m.words()).not.toContain("Detritus");
    await m.advance(350);
    expect(m.words()).toContain("Detritus");
  });

  it("cancels each input before dispatch, preserves burst actions, and rejects a delayed older fold", async () => {
    const m = await rig();
    const sent = await m.command(" ");
    // A later key handled before the first reply must invalidate that reply even while idle.
    const next = await m.command("q");
    expect(next.seq).toBe(sent.seq + 1);
    await m.snapshot(false, 2, folding(sent.seq, 1, true, false));
    expect(motion.interactions.running).toBe(false);
    expect(m.words()).not.toContain("Vimes");
    const reopened = await m.command(" ");
    await m.snapshot(true, 3, folding(reopened.seq, 2, false, true));
    expect(motion.interactions.running).toBe(true);
    const count = m.intents().length;
    m.type("jk");
    await until(() => m.intents().length === count + 2);
    expect(m.intents().slice(count).map((i) => i.intent)).toEqual([
      { intent: "select", target: "watch\u001fvimes" },
      { intent: "select", target: "watch" },
    ]);
    await m.advance(10);
    expect(motion.active).toBe(false);
    expect(m.words()).toContain("Detritus");
  });

  it("runs a title-origin lap and outgoing quiet drain on an actual focus response, preserving title/content cells", async () => {
    const m = await rig(160, 40);
    const before = parseFrame(m.frame(), 160, 40);
    const sent = await m.command("\x1b[D");
    await m.snapshot(true, 2, { context: CONTEXT, processed_seq: sent.seq, origin_seq: sent.seq }, "nav");
    expect(motion.interactions.running).toBe(true);
    const first = parseFrame(m.frame(), 160, 40);
    // The nav title occupies x=1..7, so the first drawable cell after it starts red.
    expect(first.at(8, 1).fg).toBe(parseHex(currentLook().slots["status.error"]));
    expect(first.at(1, 2)).toEqual(before.at(1, 2));
    capture("focus-0", m.frame(), 160, 40);
    await m.advance(150);
    capture("focus-150", m.frame(), 160, 40);
    await m.advance(300);
    capture("focus-300", m.frame(), 160, 40);
    const settled = parseFrame(m.frame(), 160, 40);
    expect(settled.at(26, 1).fg).toBe(parseHex(currentLook().slots["accent.muted"]));
    expect(settled.at(1, 2)).toEqual(first.at(1, 2));
    await m.advance(330);
    // An acknowledgement-only/background snapshot cannot restart the same ownership transition.
    await m.snapshot(true, 3, { context: CONTEXT, processed_seq: sent.seq }, "middle");
    expect(motion.active).toBe(false);
  });

  it.each(["subtle", "off"])("settles folds immediately without extra ticks at %s", async (level) => {
    const m = await rig(80, 24, level);
    const sent = await m.command(" ");
    await m.snapshot(false, 2, folding(sent.seq, 1, true, false));
    expect(m.words()).not.toContain("Vimes");
    expect(motion.active).toBe(false);
    const count = m.out.frames.length;
    await tick(45);
    expect(m.out.frames.length).toBe(count);
  });
  it("unmounting an active real App disposes retained cells and queued output", async () => {
    const m = await rig();
    const sent = await m.command(" ");
    await m.snapshot(false, 2, folding(sent.seq, 1, true, false));
    expect(motion.interactions.running).toBe(true);
    m.stop();
    expect(motion.active).toBe(false);
    expect(motion.engine.level).toBe("off");
    const count = m.out.frames.length;
    await tick(60);
    expect(m.out.frames.length).toBe(count);
  });

  it("animates pending visible dock focus before host echo and does not replay the acknowledgement", async () => {
    const m = await rig(160, 40);
    const docked = (focused: boolean) => ({ screen: { ...screen(true), dock: { ...STATES.empty, focused } } });
    await m.snapshot(true, 2, { context: CONTEXT, processed_seq: null }, "middle", docked(false));
    const hit = m.registry.all().find((h) => h.target === "dock:body")!;
    expect(hit).toBeDefined();
    const before = m.out.frames.length;
    const sent = await m.command(`\x1b[<0;${hit.x + 3};${hit.y + hit.h - 1}M`);
    expect(sent.intent).toEqual({ intent: "dock", dock: "focus" });
    await until(() => m.out.frames.length > before);
    expect(motion.interactions.running, "local owner change has its own causal action before any new snapshot").toBe(true);
    await m.snapshot(true, 3, { context: CONTEXT, processed_seq: sent.seq, origin_seq: sent.seq }, "middle", docked(true));
    expect(motion.interactions.running, "same-owner acknowledgement settles rather than restarting the lap").toBe(false);
    const clean = m.frame();
    await m.advance(300);
    expect(m.frame()).toBe(clean);
  });

});
