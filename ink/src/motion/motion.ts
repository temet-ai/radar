// The one motion controller of the renderer: cues, input, the marks of the frame, the frame tick, and the hook that
// applies the running effects to a frame. Costs nothing while nothing runs: `apply` returns the frame it was given
// after one boolean test, no timer is armed, and nothing is parsed.
import Output from "ink-output";
import { useSyncExternalStore } from "react";
import { currentLook } from "../text/look.js";
import { parseHex } from "./color.js";
import type { Rect } from "./grid.js";
import {
  Engine,
  FRAME_MS,
  parseLevel,
  type Cue,
  type InkColours,
  type Level,
  type Marks,
} from "./engine.js";
import { parseFrame, writeFrame } from "./frame.js";

const SYSTEM_START = Date.now();

export class Motion {
  engine: Engine;
  private marks: Marks = {};
  private marksFn: (() => Marks) | undefined;
  private nodes = new Map<
    string,
    { rows: ReadonlyArray<readonly [string, Rect]>; loading: readonly Rect[] }
  >();
  private slotNames = {
    base: "bg.base",
    surface: "bg.surface",
    overlay: "bg.overlay",
    glow: "accent.primary",
    text: "text.primary",
  };
  private timer: ReturnType<typeof setTimeout> | undefined;
  private listeners = new Set<() => void>();
  private ticks = 0;
  private armedAt = 0;
  private onNotice: ((text: string) => void) | undefined;

  constructor(
    level: Level = "off",
    private clock: () => number = () => Date.now() - SYSTEM_START,
  ) {
    this.engine = new Engine(level, clock);
  }

  /** The theme's colours effects blend with, read from the host's look when a cue starts and a frame is applied. */
  colours(): InkColours {
    const s = currentLook().slots;
    const n = this.slotNames;
    return {
      base: parseHex(s[n.base]),
      surface: parseHex(s[n.surface]),
      overlay: parseHex(s[n.overlay]),
      glow: parseHex(s[n.glow]),
      text: parseHex(s[n.text]),
    };
  }

  /** The look's slot names the host blends with (Snapshot.motion.slots). */
  setSlots(slots: {
    base: string;
    surface: string;
    overlay: string;
    glow: string;
    text: string;
  }) {
    this.slotNames = { ...slots };
  }

  /** Back to nothing running, level off and no marks (a test starts from this). */
  reset() {
    if (this.timer !== undefined) clearTimeout(this.timer);
    this.timer = undefined;
    this.engine = new Engine("off", this.clock);
    this.nodes.clear();
    this.marksFn = undefined;
    this.marks = {};
  }

  /** `level` is the host's, after `motion::effective` (off over SSH and below full colour). */
  setLevel(level: Level | string) {
    const was = this.engine.running();
    this.engine.setLevel(typeof level === "string" ? parseLevel(level) : level);
    // Off drops what ran: one more frame puts the plain one on screen.
    this.arm(was && !this.engine.running());
  }

  /** Where a notice from the engine ("motion is off: this terminal is slow to draw") goes. */
  onSlow(fn: (text: string) => void) {
    this.onNotice = fn;
  }

  cue(cue: Cue) {
    this.engine.cue(cue, this.colours());
    this.arm();
  }

  /** A key, click or paste: what runs gives way and the next frame is plain. */
  input() {
    if (!this.engine.running()) return;
    this.engine.input();
    this.arm();
  }

  /** Where the frame being drawn put its body, popup and message (set directly, for tests). */
  setMarks(marks: Marks) {
    this.marks = marks;
  }

  /** The shell's places, worked out when a frame is applied (attach.ts). */
  marksFrom(fn: () => Marks) {
    this.marksFn = fn;
  }

  /** What a node drew where: its rows by sync key (`TreeRow.glow`) and its panels waiting for data, in screen cells.
   * Replaced as a whole each time the node lays out; `owner` is any id unique to the node. */
  markNode(
    owner: string,
    rows: ReadonlyArray<readonly [string, Rect]>,
    loading: readonly Rect[] = [],
  ) {
    if (rows.length === 0 && loading.length === 0) this.nodes.delete(owner);
    else this.nodes.set(owner, { rows, loading });
  }

  private frameMarks(): Marks {
    const base = this.marksFn?.() ?? this.marks;
    if (this.nodes.size === 0) return base;
    const rows: Array<readonly [string, Rect]> = [];
    const loading: Rect[] = [];
    for (const n of this.nodes.values()) {
      rows.push(...n.rows);
      loading.push(...n.loading);
    }
    return { ...base, rows, loading };
  }

  get active(): boolean {
    return this.engine.running();
  }

  subscribe = (fn: () => void) => {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  };
  version = () => this.ticks;

  /** Ask for a frame soon, only while an effect runs. */
  private arm(force = false) {
    if (this.timer !== undefined || !(force || this.engine.running())) return;
    this.armedAt = this.clock();
    this.timer = setTimeout(() => {
      this.timer = undefined;
      this.ticks += 1;
      for (const l of [...this.listeners]) l();
    }, FRAME_MS);
  }

  /** Apply what runs to the frame Ink drew (its string); the same string when nothing runs. */
  apply(output: string, width: number, height: number): string {
    if (!this.engine.running()) return output;
    const started = this.clock();
    const grid = parseFrame(output, width, height);
    const areas = this.engine.draw(grid, this.frameMarks(), this.colours());
    const out = areas.length > 0 ? writeFrame(grid, output, areas) : output;
    const why = this.engine.drewIn(
      this.clock() - started + Math.max(0, started - this.armedAt - FRAME_MS),
    );
    if (why) this.onNotice?.(why);
    // Motion just went off with an effect on screen: one more frame puts the plain one there.
    this.arm(Boolean(why));
    return out;
  }
}

/** The renderer's motion. Its level is off until the host says otherwise. */
export const motion = new Motion();

/** Mount once in the app: re-renders the frame on the engine's tick, and only then. */
export function useMotionFrame(): number {
  return useSyncExternalStore(motion.subscribe, motion.version);
}

let installed = false;
let target: Motion = motion;
/** Wrap Ink's Output.get (after fast-output's) so a frame passes through `m.apply`. Installed once; a later call
 * only changes which motion the wrapper asks (the renderer has one; a test brings its own). */
export function installMotionOutput(m: Motion = motion) {
  target = m;
  if (installed) return;
  installed = true;
  const base = (Output.prototype as any).get as () => {
    output: string;
    height: number;
  };
  (Output.prototype as any).get = function (this: any) {
    const r = base.call(this);
    if (!target.active) return r;
    return {
      output: target.apply(r.output, this.width, this.height),
      height: r.height,
    };
  };
}

/** Whether a read from stdin is input that stops motion: anything but a pointer's plain motion reports (button 35, with any modifiers). */
export function isInput(chunk: string | Buffer): boolean {
  const s = typeof chunk === "string" ? chunk : chunk.toString("latin1");
  return (
    s.replace(/\x1b\[<(?:35|39|43|47|51|55|59|63);\d+;\d+[Mm]/g, "") !== ""
  );
}
