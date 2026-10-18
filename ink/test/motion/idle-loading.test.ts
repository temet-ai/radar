// Regression contract for the bounded #42 idle-loading lifecycle repair.
// Uses production Motion, fake timers, and small synthetic ANSI; mounts no App.
// This is a controller/output contract, not an effect-math-only tier test.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { FRAME_MS, type Level } from "../../src/motion/engine.js";
import * as frames from "../../src/motion/frame.js";
import type { Rect } from "../../src/motion/grid.js";
import type { MotionScene } from "../../src/motion/interaction.js";
import { Motion } from "../../src/motion/motion.js";

const W = 20, H = 3;
const WAIT = { x: 1, y: 1, w: 18, h: 1 };
const INK = { base: 0x0a0812, surface: 0x12101a, overlay: 0x1a162a, glow: 0xe135ff, text: 0xf8f8f2 };
const CLEAN = ["The Watch", " Reading cases...", "Ankh-Morpork"]
  .map((line) => `\x1b[48;2;18;16;26m\x1b[38;2;98;114;164m${line}\x1b[0m`).join("\n");
const controllers: Motion[] = [];
beforeEach(() => vi.useFakeTimers());
afterEach(() => {
  controllers.splice(0).forEach((m) => m.reset());
  vi.restoreAllMocks();
  vi.useRealTimers();
});

function rig(level: Level = "full") {
  let now = 0;
  const m = new Motion(level, () => now);
  controllers.push(m);
  vi.spyOn(m, "colours").mockReturnValue(INK);
  const tick = vi.fn();
  m.subscribe(tick);
  const draw = () => m.apply(CLEAN, W, H);
  const step = () => {
    now += FRAME_MS;
    vi.advanceTimersByTime(FRAME_MS);
    return draw();
  };
  return { m, draw, step, tick };
}

describe("uncued loading at the Motion controller boundary", () => {
  it("starts only on a drawn full-motion loading frame, advances, then stops when the mark leaves", () => {
    const { m, draw, step, tick } = rig();
    const parse = vi.spyOn(frames, "parseFrame");
    const engineDraw = vi.spyOn(m.engine, "draw");
    expect(draw()).toBe(CLEAN);
    expect(parse).not.toHaveBeenCalled();
    expect(engineDraw).not.toHaveBeenCalled();
    expect(vi.getTimerCount()).toBe(0);

    m.markNode("watch-middle", [], [WAIT]);
    expect(vi.getTimerCount(), "registration alone must not schedule an uncommitted frame").toBe(0);
    draw(); // No cue, input, notice, focus transition, or direct Engine.draw priming.
    expect(engineDraw).toHaveBeenCalledTimes(1);
    expect(m.active, "current code returns before discovering loading").toBe(true);
    expect(vi.getTimerCount()).toBe(1);

    let moving = CLEAN;
    for (let i = 0; i < 10; i++) {
      m.markNode("watch-middle", [], [WAIT]); // Real nodes register on every layout.
      moving = step();
      expect(vi.getTimerCount(), "one scheduler, never a timer per mark").toBe(1);
    }
    expect(frames.parseFrame(moving, W, H).cells).not.toEqual(frames.parseFrame(CLEAN, W, H).cells);
    m.input();
    expect(m.active, "existing input contract keeps a genuine loading shimmer").toBe(true);
    m.markNode("watch-middle", [], []);
    expect(draw(), "the new content is immediately clean").toBe(CLEAN);
    expect(m.active).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
    const stopped = tick.mock.calls.length;
    vi.advanceTimersByTime(5 * FRAME_MS);
    expect(tick).toHaveBeenCalledTimes(stopped);
  });

  it.each(["off", "subtle"] as const)("%s never wakes or parses for loading alone", (level) => {
    const { m, draw } = rig(level);
    const parse = vi.spyOn(frames, "parseFrame");
    const engineDraw = vi.spyOn(m.engine, "draw");
    m.markNode("watch-middle", [], [WAIT]);
    expect(draw()).toBe(CLEAN);
    expect(m.active).toBe(false);
    expect(parse).not.toHaveBeenCalled();
    expect(engineDraw).not.toHaveBeenCalled();
    expect(vi.getTimerCount()).toBe(0);
  });

  it.each([
    { x: 1, y: 1, w: 0, h: 1 },
    { x: W, y: 1, w: 4, h: 1 },
    { x: 1, y: H, w: 4, h: 1 },
  ])("does not wake for an empty or invisible loading rectangle %j", (area) => {
    const { m, draw } = rig();
    const parse = vi.spyOn(frames, "parseFrame");
    m.markNode("watch-middle", [], [area]);
    expect(draw()).toBe(CLEAN);
    expect(parse).not.toHaveBeenCalled();
    expect(m.active).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("does not borrow a preparing scene's loading geometry before its matching commit", () => {
    const { m, draw } = rig();
    const previous: MotionScene = { cursor: 1, context: { kind: "history" }, local: false, blocked: false };
    const next: MotionScene = { ...previous, cursor: 2 };
    m.prepareScene(previous);
    m.commitScene(previous);
    expect(draw()).toBe(CLEAN);
    m.prepareScene(next);
    m.markNode("watch-middle", [], [WAIT]);
    expect(draw()).toBe(CLEAN);
    expect(m.active).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
    m.commitScene(next);
    draw();
    expect(m.active).toBe(true);
    expect(vi.getTimerCount()).toBe(1);
  });

  it.each([
    { name: "negative width", area: { ...WAIT, w: -1 } },
    { name: "zero height", area: { ...WAIT, h: 0 } },
    { name: "fully left", area: { ...WAIT, x: -18 } },
    { name: "fully above", area: { ...WAIT, y: -1 } },
    ...(["x", "y", "w", "h"] as const).flatMap((key) => [NaN, Infinity, -Infinity, 0.5].map((value) => ({
      name: `${key} is ${String(value)}`, area: { ...WAIT, [key]: value },
    }))),
    { name: "unsafe right edge", area: { ...WAIT, x: 1, w: Number.MAX_SAFE_INTEGER } },
    { name: "unsafe lower edge", area: { ...WAIT, y: 1, h: Number.MAX_SAFE_INTEGER } },
  ])("does not parse or schedule malformed loading: $name", ({ area }) => {
    const { m, draw } = rig();
    const parse = vi.spyOn(frames, "parseFrame"), engineDraw = vi.spyOn(m.engine, "draw");
    m.markNode("watch-middle", [], [area]);
    expect(draw()).toBe(CLEAN);
    expect(parse).not.toHaveBeenCalled();
    expect(engineDraw).not.toHaveBeenCalled();
    expect(m.active).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });

  it.each([
    { name: "zero width", width: 0, height: H },
    { name: "negative width", width: -1, height: H },
    { name: "fractional width", width: 1.5, height: H },
    { name: "nonfinite width", width: Infinity, height: H },
    { name: "NaN width", width: NaN, height: H },
    { name: "zero height", width: W, height: 0 },
    { name: "negative height", width: W, height: -1 },
    { name: "fractional height", width: W, height: 1.5 },
    { name: "nonfinite height", width: W, height: Infinity },
    { name: "NaN height", width: W, height: NaN },
    { name: "over cell budget", width: 257, height: 256 },
    { name: "unsafe width", width: Number.MAX_SAFE_INTEGER + 1, height: 1 },
  ])("rejects $name before collecting loading or parsing", ({ width, height }) => {
    const { m } = rig();
    const parse = vi.spyOn(frames, "parseFrame"), collect = vi.fn(() => ({ loading: [WAIT] }));
    m.marksFrom(collect);
    expect(m.apply(CLEAN, width, height)).toBe(CLEAN);
    expect(collect).not.toHaveBeenCalled();
    expect(parse).not.toHaveBeenCalled();
    expect(m.active).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("disposes an active loader before a malformed next frame can reach the parser", () => {
    const { m, draw } = rig();
    m.markNode("watch-middle", [], [WAIT]);
    draw();
    expect(m.active).toBe(true);
    const parse = vi.spyOn(frames, "parseFrame");
    expect(() => m.apply(CLEAN, 1.5, H)).not.toThrow();
    expect(parse).not.toHaveBeenCalled();
    expect(m.active).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
    // The engine no longer owns a shimmer; a repeated malformed output must still stay unparsed.
    expect(m.apply(CLEAN, 1.5, H)).toBe(CLEAN);
    expect(parse).not.toHaveBeenCalled();
    draw();
    expect(m.active, "valid current loading can resume on a later real output").toBe(true);
  });

  it("a rejected loading frame preserves an ordinary cue and recovers on a clean valid frame", () => {
    const { m, draw } = rig();
    m.setMarks({ body: { x: 0, y: 0, w: W, h: H } });
    m.markNode("watch-middle", [], [WAIT]);
    m.cue({ kind: "screen" });
    draw();
    const parse = vi.spyOn(frames, "parseFrame");
    expect(m.apply(CLEAN, 1.5, H)).toBe(CLEAN);
    expect(m.apply(CLEAN, 1.5, H)).toBe(CLEAN);
    expect(parse).not.toHaveBeenCalled();
    expect(m.active, "invalid loading output pauses scheduling until valid output").toBe(false);
    expect(vi.getTimerCount()).toBe(0);
    m.markNode("watch-middle", [], []);
    const engineDraw = vi.spyOn(m.engine, "draw");
    expect(draw()).not.toBe(CLEAN);
    expect(engineDraw.mock.calls[0]![1].loading).toEqual([]);
    expect(m.active, "the clean valid frame unblocks the surviving ordinary cue").toBe(true);
    m.input();
    expect(draw()).toBe(CLEAN);
    expect(m.active).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
    m.markNode("watch-middle", [], [WAIT]);
    draw();
    expect(m.active, "subsequent current loading is discovered normally").toBe(true);
  });

  it("rejects over-budget ANSI before collecting loading or parsing", () => {
    const { m } = rig();
    const parse = vi.spyOn(frames, "parseFrame"), collect = vi.fn(() => ({ loading: [WAIT] }));
    m.marksFrom(collect);
    const oversized = "x".repeat(1_048_577);
    expect(m.apply(oversized, W, H)).toBe(oversized);
    expect(collect).not.toHaveBeenCalled();
    expect(parse).not.toHaveBeenCalled();
    expect(m.active).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });

  it.each(["direct", "callback"] as const)("discovers %s loading marks and uses one metadata snapshot", (source) => {
    const { m, draw } = rig();
    const collect = vi.fn().mockReturnValueOnce({ loading: [WAIT] }).mockReturnValue({});
    if (source === "direct") m.setMarks({ loading: [WAIT] });
    else m.marksFrom(collect);
    const engineDraw = vi.spyOn(m.engine, "draw");
    expect(m.active).toBe(false);
    expect(collect).not.toHaveBeenCalled();
    draw();
    expect(engineDraw).toHaveBeenCalledTimes(1);
    expect(engineDraw.mock.calls[0]![1].loading).toEqual([WAIT]);
    expect(collect).toHaveBeenCalledTimes(source === "callback" ? 1 : 0);
    expect(m.active).toBe(true);
    expect(vi.getTimerCount()).toBe(1);
  });

  it("does not traverse ordinary rows to check idle readiness or change node-over-shell precedence", () => {
    const { m, draw } = rig();
    const rows = new Proxy<ReadonlyArray<readonly [string, Rect]>>([["watch", WAIT]], {
      get(target, property, receiver) {
        if (property === Symbol.iterator) throw new Error("idle readiness must not flatten tree rows");
        return Reflect.get(target, property, receiver);
      },
    });
    const parse = vi.spyOn(frames, "parseFrame");
    m.setMarks({ loading: [WAIT] });
    m.markNode("watch-tree", rows, []);
    expect(m.active).toBe(false);
    expect(draw()).toBe(CLEAN);
    expect(parse).not.toHaveBeenCalled();
    expect(vi.getTimerCount()).toBe(0);
    m.markNode("watch-tree", [], []);
    draw();
    expect(m.active, "shell loading resumes only once no node overrides it").toBe(true);
  });

  it.each([
    { name: "unkeyed", context: null },
    { name: "rejected interaction", context: { kind: "settings", plugin: null, section: "x".repeat(16_385) } },
    { name: "local dock routing", context: { kind: "history" }, local: true },
  ] as const)("requires an actual commit but permits ordinary loading for $name scenes", ({ context, ...options }) => {
    const { m, draw } = rig();
    const scene: MotionScene = { cursor: 1, context, local: "local" in options, blocked: false };
    const parse = vi.spyOn(frames, "parseFrame");
    m.prepareScene(scene);
    m.markNode("watch-middle", [], [WAIT], scene);
    expect(draw()).toBe(CLEAN);
    expect(parse).not.toHaveBeenCalled();
    expect(m.active).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
    m.commitScene(scene);
    draw();
    expect(m.active).toBe(true);
    expect(vi.getTimerCount()).toBe(1);
  });

  it("never borrows old node geometry or lets a stale commit qualify it for a new scene", () => {
    const { m, draw } = rig();
    const old: MotionScene = { cursor: 1, context: { kind: "history" }, local: false, blocked: false };
    const next = { ...old }; // Same cursor/context is not the same actual render.
    const parse = vi.spyOn(frames, "parseFrame");
    m.prepareScene(old);
    m.markNode("watch-middle", [], [WAIT], old);
    m.commitScene(old);
    m.prepareScene(next);
    m.commitScene(next);
    expect(draw()).toBe(CLEAN);
    expect(parse).not.toHaveBeenCalled();
    m.markNode("watch-middle", [], [WAIT], old); // A stale layout cannot revive it either.
    m.commitScene(old);
    expect(draw()).toBe(CLEAN);
    expect(parse).not.toHaveBeenCalled();
    expect(m.active).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
    m.markNode("watch-middle", [], [WAIT], next);
    m.commitScene(next);
    draw();
    expect(m.active).toBe(true);
  });

  it.each([true, false])("stops a prior shimmer on a %s-keyed replacement output before commit", (keyed) => {
    const { m, draw, tick } = rig();
    const previous: MotionScene = { cursor: 1, context: { kind: "history" }, local: false, blocked: false };
    m.prepareScene(previous);
    m.markNode("watch-middle", [], [WAIT], previous);
    m.commitScene(previous);
    draw();
    expect(m.active).toBe(true);
    expect(vi.getTimerCount()).toBe(1);
    const next: MotionScene = { ...previous, cursor: 2, context: keyed ? previous.context : null };
    m.prepareScene(next);
    m.markNode("watch-middle", [], [WAIT], next);
    const parse = vi.spyOn(frames, "parseFrame");
    expect(draw()).toBe(CLEAN);
    expect(parse).not.toHaveBeenCalled();
    expect(m.active, "an uncommitted replacement cannot keep the prior loading owner alive").toBe(false);
    expect(vi.getTimerCount()).toBe(0);
    vi.advanceTimersByTime(5 * FRAME_MS);
    expect(tick).not.toHaveBeenCalled();
    m.commitScene(next);
    draw();
    expect(m.active, "the replacement may start its own shimmer only once committed").toBe(true);
    expect(vi.getTimerCount()).toBe(1);
  });

  it("clearing precommit loading preserves an ordinary cue for the committed replacement", () => {
    const { m, draw } = rig();
    const previous: MotionScene = { cursor: 1, context: { kind: "history" }, local: false, blocked: false };
    m.setMarks({ body: { x: 0, y: 0, w: W, h: H } });
    m.prepareScene(previous);
    m.markNode("watch-middle", [], [WAIT], previous);
    m.commitScene(previous);
    draw();
    m.cue({ kind: "screen" });
    const next = { ...previous, cursor: 2 };
    m.prepareScene(next);
    m.markNode("watch-middle", [], [], next);
    const parse = vi.spyOn(frames, "parseFrame");
    expect(draw()).toBe(CLEAN);
    expect(parse).not.toHaveBeenCalled();
    expect(m.active, "ordinary cue ownership is unchanged by loading cleanup").toBe(true);
    m.commitScene(next);
    expect(draw()).not.toBe(CLEAN);
    m.input();
    expect(draw()).toBe(CLEAN);
    expect(m.active).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("a scene-qualified mark without a corresponding prepared and committed render stays idle", () => {
    const { m, draw } = rig();
    const scene: MotionScene = { cursor: 1, context: null, local: false, blocked: false };
    const parse = vi.spyOn(frames, "parseFrame");
    m.markNode("watch-middle", [], [WAIT], scene);
    m.commitScene(scene);
    expect(draw()).toBe(CLEAN);
    expect(parse).not.toHaveBeenCalled();
    expect(m.active).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("blocked foreground scenes neither wake nor retain an old loading shimmer", () => {
    const { m, draw } = rig();
    const blocked: MotionScene = { cursor: 1, context: { kind: "history" }, local: false, blocked: true };
    const parse = vi.spyOn(frames, "parseFrame");
    m.prepareScene(blocked);
    m.markNode("watch-middle", [], [WAIT], blocked);
    m.commitScene(blocked);
    expect(draw()).toBe(CLEAN);
    expect(parse).not.toHaveBeenCalled();
    expect(m.active).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
    const open = { ...blocked, blocked: false };
    m.prepareScene(open);
    m.markNode("watch-middle", [], [WAIT], open);
    m.commitScene(open);
    draw();
    expect(m.active).toBe(true);
    m.prepareScene(blocked);
    m.markNode("watch-middle", [], [WAIT], blocked);
    m.commitScene(blocked);
    expect(draw()).toBe(CLEAN);
    expect(m.active).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("blocked loading leaves ordinary cues available and presentation painting last", () => {
    const { m, draw } = rig();
    const scene: MotionScene = { cursor: 1, context: { kind: "history" }, local: false, blocked: true };
    m.prepareScene(scene);
    m.markNode("watch-middle", [], [WAIT], scene);
    m.commitScene(scene);
    m.setMarks({ body: { x: 0, y: 0, w: W, h: H } });
    const engineDraw = vi.spyOn(m.engine, "draw");
    const top = { x: 1, y: 1, w: 1, h: 1 };
    m.setPresentation({
      visible: () => true, running: () => false, clear: vi.fn(),
      draw: (grid) => { grid.at(1, 1).fg = 0x123456; return [top]; },
    });
    m.cue({ kind: "screen" });
    const actual = frames.parseFrame(draw(), W, H);
    expect(engineDraw).toHaveBeenCalledTimes(1);
    expect(engineDraw.mock.calls[0]![1].loading).toEqual([]);
    expect(m.active, "foreground ownership does not reject ordinary cues").toBe(true);
    expect(actual.at(1, 1).fg, "the late presentation is painted after ordinary effects").toBe(0x123456);
    expect(actual.at(0, 0).fg).not.toBe(frames.parseFrame(CLEAN, W, H).at(0, 0).fg);
  });

  it("passes only eligible loading rectangles to the engine and clears a later invalid replacement", () => {
    const { m, draw } = rig();
    const engineDraw = vi.spyOn(m.engine, "draw");
    m.markNode("watch-middle", [], [WAIT, { ...WAIT, x: NaN }, { ...WAIT, y: H }]);
    draw();
    expect(engineDraw.mock.calls[0]![1].loading).toEqual([WAIT]);
    expect(m.active).toBe(true);
    m.markNode("watch-middle", [], [{ ...WAIT, w: 0.5 }]);
    expect(draw()).toBe(CLEAN);
    expect(engineDraw.mock.calls[1]![1].loading).toEqual([]);
    expect(m.active).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
    const parse = vi.spyOn(frames, "parseFrame");
    expect(draw()).toBe(CLEAN);
    expect(parse).not.toHaveBeenCalled();
  });

  it("clips a visible loading mark to the bounded viewport without changing other cells", () => {
    const { m, draw, step } = rig();
    const clipped = { x: -2, y: 1, w: 6, h: 1 };
    m.markNode("watch-middle", [], [clipped]);
    draw();
    let output = CLEAN;
    for (let i = 0; i < 10; i++) output = step();
    const actual = frames.parseFrame(output, W, H), clean = frames.parseFrame(CLEAN, W, H);
    expect(actual.cells).not.toEqual(clean.cells);
    for (let y = 0; y < H; y++) for (let x = 0; x < W; x++) {
      expect(actual.at(x, y).ch).toBe(clean.at(x, y).ch);
      if (y !== 1 || x >= 4) expect(actual.at(x, y)).toEqual(clean.at(x, y));
    }
  });

  it("clips surplus ANSI lines rather than allocating beyond the loading viewport", () => {
    const { m } = rig();
    m.markNode("watch-middle", [], [WAIT]);
    const engineDraw = vi.spyOn(m.engine, "draw");
    const output = `${CLEAN}\n${"\n".repeat(1000)}`;
    m.apply(output, W, H);
    expect(engineDraw.mock.calls[0]![0].cells).toHaveLength(W * H);
    expect(m.active).toBe(true);
  });

  it("does not override the sticky slow-terminal fallback when loading remains visible", () => {
    const { m, draw } = rig();
    m.markNode("watch-middle", [], [WAIT]);
    draw();
    expect(m.engine.drewIn(60)).toBeUndefined();
    expect(m.engine.drewIn(60)).toBeUndefined();
    expect(m.engine.drewIn(60)).toBeDefined();
    m.setLevel("full");
    const parse = vi.spyOn(frames, "parseFrame");
    expect(m.engine.level).toBe("off");
    expect(draw()).toBe(CLEAN);
    expect(parse).not.toHaveBeenCalled();
    expect(m.active).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("reset disposes the waiting timer and loading metadata", () => {
    const { m, draw, tick } = rig();
    m.markNode("watch-middle", [], [WAIT]);
    draw();
    expect(m.active).toBe(true);
    m.reset();
    m.setLevel("full");
    expect(draw()).toBe(CLEAN);
    expect(vi.getTimerCount()).toBe(0);
    vi.advanceTimersByTime(5 * FRAME_MS);
    expect(tick).not.toHaveBeenCalled();
  });
});

// Adjacent guard, independently reproducible without the idle-wake fix. The
// current Ink AND native engines leave a previously running shimmer at subtle.
// Scope this deliberately; mirror the engine-level assertion in Rust if fixed.
describe("loading level downgrade guard", () => {
  it("full to subtle discards an already running loading shimmer", () => {
    const { m, draw } = rig();
    m.markNode("watch-middle", [], [WAIT]);
    // Direct engine priming is intentional ONLY for this separate existing bug.
    m.engine.draw(frames.parseFrame(CLEAN, W, H), { loading: [WAIT] }, INK);
    draw();
    expect(m.active).toBe(true);
    m.setLevel("subtle");
    // A pending clean settling frame is allowed; a persistent shimmer after drawing it is not.
    expect(frames.parseFrame(draw(), W, H).cells).toEqual(frames.parseFrame(CLEAN, W, H).cells);
    expect(m.active, "the clean frame consumes the downgrade settling state").toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });
});
