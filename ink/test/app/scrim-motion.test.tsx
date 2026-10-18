// The finished-frame seam: layer marks paint the scrim before popup motion sees the frame.
// App/MenuBox wiring is covered separately; this contract uses Ink's actual Output without React or real-time ticks.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import Output from "ink-output";
import chalk from "chalk";
import "../../src/app/hover.js";
import { fastOutputInstalled, useFastOutput } from "../../src/app/fast-output.js";
import { layerMark, paintScrim } from "../../src/app/scrim.js";
import { POPUP_MS } from "../../src/motion/engine.js";
import { installMotionOutput, Motion, motion } from "../../src/motion/motion.js";
import { applyLook, currentLook, defaultLook } from "../../src/text/look.js";
import { parseAnsi } from "../nodes/ansi.js";

const W = 16;
const H = 8;
const POPUP = { x: 4, y: 2, w: 7, h: 3 };
const inside = (x: number, y: number) =>
  x >= POPUP.x && x < POPUP.x + POPUP.w && y >= POPUP.y && y < POPUP.y + POPUP.h;

/** The same writes with or without the layer marker: removal must not reuse the painted cache. */
function draw(marked: boolean) {
  const out = new Output({ width: W, height: H });
  for (let y = 0; y < H; y++)
    out.write(0, y, chalk.bold.bgHex("#12101a").hex("#6272a4")("│─┼─".repeat(4)), { transformers: [] } as never);
  ["┌─────┐", "│Watch│", "└─────┘"].forEach((row, y) =>
    out.write(POPUP.x, POPUP.y + y, chalk.bgHex("#1a162a").hex("#f8f8f2")(row), {
      transformers: marked && y === 0 ? [layerMark(POPUP.w, POPUP.h)] : [],
    } as never),
  );
  return out.get();
}

let savedLook = currentLook();
let savedFast = fastOutputInstalled();
let savedColour = chalk.level;
beforeEach(() => {
  savedLook = currentLook();
  savedFast = fastOutputInstalled();
  savedColour = chalk.level;
  vi.useFakeTimers();
  applyLook(defaultLook());
  chalk.level = 3;
  useFastOutput(true);
});
afterEach(() => {
  // Switching back disposes the test controller and its pending tick; leave the normal output target idle.
  installMotionOutput(motion);
  motion.reset();
  applyLook(savedLook);
  chalk.level = savedColour;
  useFastOutput(savedFast);
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("scrim and popup motion in the finished-frame output pipeline", () => {
  it("sweeps only the popup over a once-scrimmed frame, then settles, cancels and removes the layer cleanly", () => {
    let now = 0;
    const m = new Motion("subtle", () => now);
    m.setMarks({ popup: POPUP });
    installMotionOutput(m);
    const apply = vi.spyOn(m, "apply"); // Call through, and verify motion receives the already-painted frame.
    const control = draw(false);
    const unpainted = parseAnsi(control.output);
    const onceScrimmed = paintScrim(control.output, W, H, POPUP);
    const clean = draw(true);
    expect(clean).toEqual({ output: onceScrimmed, height: H });
    expect(apply).toHaveBeenLastCalledWith(onceScrimmed, W, H);
    expect(draw(true)).toEqual(clean); // A cached frame is repainted from the unpainted source, not dimmed again.
    const cells = parseAnsi(clean.output);
    expect(cells[0]![0]).toEqual({ ch: "│", fg: "#23263b", bg: "#0f0d17" });
    expect(cells[3]![11]).toEqual({ ch: "─", fg: "#181929", bg: "#060509" }); // right shadow
    expect(cells[5]![5]).toEqual({ ch: "─", fg: "#181929", bg: "#060509" }); // bottom shadow
    expect(cells[5]![11]).toEqual({ ch: "─", fg: "#181929", bg: "#060509" }); // corner, once
    for (let y = 0; y < H; y++)
      for (let x = 0; x < W; x++) {
        expect(cells[y]![x]!.ch, `clean glyph at ${x},${y}`).toBe(unpainted[y]![x]!.ch);
        if (inside(x, y)) expect(cells[y]![x], `unpainted popup at ${x},${y}`).toEqual(unpainted[y]![x]);
      }

    const swept = () => {
      const frame = draw(true);
      expect(apply).toHaveBeenLastCalledWith(onceScrimmed, W, H);
      expect(frame.height).toBe(H);
      const moving = parseAnsi(frame.output);
      expect(moving).toHaveLength(H);
      for (let y = 0; y < H; y++) {
        expect(moving[y]).toHaveLength(W);
        for (let x = 0; x < W; x++)
          if (!inside(x, y)) expect(moving[y]![x], `outside popup at ${x},${y}, time ${now}`).toEqual(cells[y]![x]);
      }
      expect(moving, "the popup actually animates").not.toEqual(cells);
      expect(m.active).toBe(true);
    };

    m.cue({ kind: "popup" });
    swept();
    now = POPUP_MS / 2;
    swept();
    now = POPUP_MS;
    expect(parseAnsi(draw(true).output)).toEqual(cells); // Final effect frame may serialize ANSI differently.
    expect(draw(true)).toEqual(clean); // The required plain frame is byte-for-byte the once-painted output.
    expect(m.active).toBe(false);
    expect(vi.getTimerCount()).toBe(0);

    m.cue({ kind: "popup" });
    swept();
    now += POPUP_MS / 2;
    swept();
    m.input();
    expect(draw(true)).toEqual(clean);
    expect(m.active).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
    expect(draw(false)).toEqual(control); // A following frame with no marker leaves neither scrim nor stale shadow.
    expect(apply).toHaveBeenLastCalledWith(control.output, W, H);
  });
});
