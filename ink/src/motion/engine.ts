// Ink's port of crates/radar-core/src/motion.rs `Engine`: cues start short effects, each frame applies them over the
// cells just drawn, input drops them, and the frame after the last is exactly the one without motion. The engine
// changes nothing of the drawing: it only rewrites colours of a grid it is handed, and does nothing at all (no
// allocation, no cell read) while nothing runs.
import {
  Shimmer,
  FadeFromFg,
  FadeOut,
  Glow,
  SweepIn,
  WHITE,
  type Effect,
  type Reads,
} from "./effects.js";
import { lerp } from "./color.js";
import {
  intersection,
  isEmpty,
  sameRect,
  type Cell,
  type Grid,
  type Rect,
} from "./grid.js";

export type Level = "full" | "subtle" | "off";

/** What happened, as the host reports it. `changed`/`added` hold row keys (`kind:id`). */
export type Cue =
  | { kind: "screen" }
  | { kind: "page" }
  | { kind: "popup" }
  | { kind: "message" }
  | { kind: "message_gone" }
  | { kind: "changed"; changed: readonly string[]; added: readonly string[] };

/** Where things were drawn in the frame. */
export type Marks = {
  body?: Rect;
  popup?: Rect;
  message?: Rect;
  rows?: ReadonlyArray<readonly [string, Rect]>;
  loading?: readonly Rect[];
};

/** The theme's colours the effects blend with (slots bg.base, bg.surface, bg.overlay, accent.primary, text.primary). */
export type InkColours = {
  base: number;
  surface: number;
  overlay: number;
  glow: number;
  text: number;
};

export const SCREEN_MS = 100;
/** How far from the background toward the text a screen switch's words start (motion::SCREEN_FROM): dimmed. */
export const SCREEN_FROM = 0.5;
export const PAGE_MS = 300;
export const POPUP_MS = 180;
export const MESSAGE_MS = 250;
export const MESSAGE_GONE_MS = 200;
export const GLOW_MS = 450;
export const ADDED_MS = 320;
export const SHIMMER_MS = 1400;
/** How often to redraw while an effect runs. */
export const FRAME_MS = 30;
const SLOW_MS = 60;

export const parseLevel = (word: string): Level => {
  const w = word.trim();
  return w === "full" ? "full" : w === "off" || w === "none" ? "off" : "subtle";
};

export function cueRuns(cue: Cue, level: Level): boolean {
  if (level === "off") return false;
  if (level === "full") return true;
  return (
    cue.kind === "screen" || cue.kind === "popup" || cue.kind === "changed"
  );
}

type Target =
  | { t: "body" }
  | { t: "popup" }
  | { t: "message" }
  | { t: "row"; key: string }
  | { t: "area"; rect: Rect };
const sameTarget = (a: Target, b: Target): boolean =>
  a.t === b.t &&
  (a.t === "row"
    ? a.key === (b as typeof a).key
    : a.t === "area"
      ? sameRect(a.rect, (b as typeof a).rect)
      : true);

function find(target: Target, marks: Marks): Rect | undefined {
  switch (target.t) {
    case "body":
      return marks.body;
    case "popup":
      return marks.popup;
    case "message":
      return marks.message;
    case "row":
      return marks.rows?.find(([k]) => k === target.key)?.[1];
    case "area":
      return target.rect;
  }
}

type Running = { target: Target; fx: Effect; shimmer: boolean };

export class Engine {
  private running_: Running[] = [];
  private last: number | undefined;
  private settle = false;
  private message: { rect: Rect; cells: Cell[] } | undefined;
  private slow = 0;
  private tooSlow = false;

  constructor(
    private level_: Level,
    /** Milliseconds since some fixed start. */
    private clock: () => number,
  ) {}

  get level(): Level {
    return this.level_;
  }

  setLevel(level: Level) {
    this.level_ = this.tooSlow ? "off" : level;
    if (this.level_ === "off") {
      this.running_ = [];
      this.last = undefined;
      this.message = undefined;
    }
  }

  /** Whether to draw again soon: an effect runs, or the plain frame after one is still to come. */
  running(): boolean {
    return this.running_.length > 0 || this.settle;
  }

  /** The frame tick in ms, only while `running()`. */
  nextFrame(): number | undefined {
    return this.running() ? FRAME_MS : undefined;
  }

  /** A key, click or paste: what runs is dropped and the next frame is plain. A loading shimmer goes on. */
  input() {
    const before = this.running_.length;
    this.running_ = this.running_.filter((r) => r.shimmer);
    if (this.running_.length !== before) this.settle = true;
  }

  cue(cue: Cue, ink: InkColours) {
    if (!cueRuns(cue, this.level_)) return;
    const reads: Reads = { fg: ink.text, bg: ink.base };
    switch (cue.kind) {
      case "screen":
        this.replace(
          { t: "body" },
          // The terminal's own text colour is read as tachyonfx reads a Reset foreground, as the fade does per cell.
          new FadeFromFg(lerp(ink.base, ink.text < 0 ? WHITE : ink.text, SCREEN_FROM), SCREEN_MS, "quadOut", reads),
        );
        break;
      case "page":
        this.replace(
          { t: "body" },
          new SweepIn("leftToRight", 24, ink.base, PAGE_MS, "quadOut"),
        );
        break;
      case "popup":
        this.replace(
          { t: "popup" },
          new SweepIn("upToDown", 3, ink.base, POPUP_MS, "quadOut"),
        );
        break;
      case "message":
        this.replace(
          { t: "message" },
          new SweepIn("leftToRight", 8, ink.overlay, MESSAGE_MS, "quadOut"),
        );
        break;
      case "message_gone": {
        const m = this.message;
        this.message = undefined;
        if (m)
          this.replace(
            { t: "area", rect: m.rect },
            new FadeOut(m.cells, m.rect.w, ink.overlay),
          );
        break;
      }
      case "changed":
        for (const key of cue.changed)
          this.replace({ t: "row", key }, new Glow(ink.glow, reads));
        for (const key of cue.added)
          this.replace(
            { t: "row", key },
            this.level_ === "full"
              ? new SweepIn("leftToRight", 12, ink.surface, ADDED_MS, "quadOut")
              : new Glow(ink.glow, reads),
          );
        break;
    }
  }

  private replace(target: Target, fx: Effect) {
    this.running_ = this.running_.filter((r) => !sameTarget(r.target, target));
    this.running_.push({ target, fx, shimmer: false });
  }

  /** Apply what runs to the frame just drawn into `grid`, with the marks made while drawing it. The areas it changed, none when it
   * changed any cell (the plain frame after an effect, and a frame with nothing running, change none). */
  draw(grid: Grid, marks: Marks, ink: InkColours): Rect[] {
    this.settle = false;
    if (this.level_ === "off") return [];
    const reads: Reads = { fg: ink.text, bg: ink.base };
    if (this.level_ === "full") {
      const loading = marks.loading ?? [];
      this.running_ = this.running_.filter(
        (r) =>
          !r.shimmer ||
          (r.target.t === "area" &&
            loading.some((a) =>
              sameRect(a, (r.target as { rect: Rect }).rect),
            )),
      );
      for (const area of loading) {
        if (
          !this.running_.some(
            (r) =>
              r.shimmer &&
              r.target.t === "area" &&
              sameRect(r.target.rect, area),
          )
        )
          this.running_.push({
            target: { t: "area", rect: area },
            fx: new Shimmer(ink.overlay, ink.text, reads),
            shimmer: true,
          });
      }
      this.message = marks.message
        ? ((r) => ({ rect: r, cells: grid.copyOf(r) }))(
            intersection(marks.message, grid.area),
          )
        : undefined;
    }
    if (this.running_.length === 0) {
      this.last = undefined;
      return [];
    }
    const now = this.clock();
    const dt = this.last === undefined ? 0 : Math.max(0, now - this.last);
    const applied: Rect[] = [];
    const kept: Running[] = [];
    for (const r of this.running_) {
      const found = find(r.target, marks);
      if (!found) continue;
      const area = intersection(found, grid.area);
      if (isEmpty(area)) continue;
      r.fx.process(dt, grid, area);
      applied.push(area);
      if (!r.fx.done()) kept.push(r);
    }
    this.running_ = kept;
    this.settle = kept.length === 0;
    this.last = kept.length > 0 ? now : undefined;
    return applied;
  }

  /** How long the frame took to reach the terminal; three slow ones in a row while effects run turn motion off for
   * the session (the reason is returned once, for the status line). */
  drewIn(ms: number): string | undefined {
    if (!this.running() || this.tooSlow) return undefined;
    if (ms < SLOW_MS) {
      this.slow = 0;
      return undefined;
    }
    this.slow += 1;
    if (this.slow < 3) return undefined;
    this.tooSlow = true;
    this.setLevel("off");
    return "motion is off: this terminal is slow to draw";
  }
}
