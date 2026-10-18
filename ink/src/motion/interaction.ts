// Bounded interaction state: one clean ANSI baseline, and at most one fold/focus transition.
// An idle observe retains the string reference and immutable committed marks; it never parses ANSI.
import type { MotionInteraction } from "../protocol/generated/MotionInteraction.js";
import type { MotionFold } from "../protocol/generated/MotionFold.js";
import type { MotionContext } from "../protocol/generated/MotionContext.js";
import { currentLook } from "../text/look.js";
import { parseHex } from "./color.js";
import { parseFrame } from "./frame.js";
import { sameRect, type Grid, type Rect } from "./grid.js";
import { contextKey, frameMetadataBounded } from "./interaction-bounds.js";
import {
  FOLD_MS, FOCUS_MS, MAX_BASELINE_ANSI,
  borderForegrounds, bounded, contained, composeFold, focusDrain, focusLap, sameBorder,
  type Border, type FoldRows,
} from "./interaction-effects.js";

export type PaneMark = Border & { pane: string; owns: boolean };
export type TreeMark = {
  pane: string;
  slot: string;
  area: Rect;
  rows: ReadonlyArray<{ path: readonly string[]; y: number }>;
  open: ReadonlyArray<readonly string[]>;
};
/** Created by App, committed after its children measure the actual rendered frame. */
export type MotionScene = {
  cursor: number;
  context: MotionContext | null;
  interaction?: MotionInteraction;
  blocked: boolean;
  /** Pending local dock routing changed the owner before its host acknowledgement. */
  local: boolean;
};
export type SceneMarks = { scene: MotionScene; panes: readonly PaneMark[]; trees: readonly TreeMark[] };
type Baseline = { output: string; width: number; height: number; frame: SceneMarks; context: string; look: object };
type Focus = { before: PaneMark; now: PaneMark; prior: readonly number[]; incoming: boolean };
type Active = { context: string; look: object; width: number; height: number; cursor: number; start: number; fold?: { rows: FoldRows; mark: TreeMark }; focus: Focus[]; stops: readonly number[] };
const pathEqual = (a: readonly string[], b: readonly string[]) => a.length === b.length && a.every((s, i) => s === b[i]);
const descendant = (p: readonly string[], parent: readonly string[]) => p.length > parent.length && parent.every((s, i) => p[i] === s);
const treeEqual = (a: TreeMark, b: TreeMark) => sameRect(a.area, b.area) && a.pane === b.pane && a.slot === b.slot && a.rows.length === b.rows.length && a.rows.every((r, i) => r.y === b.rows[i]!.y && pathEqual(r.path, b.rows[i]!.path)) && a.open.length === b.open.length && a.open.every((p, i) => pathEqual(p, b.open[i]!));
const owner = (frame: SceneMarks) => frame.panes.filter((p) => p.owns).length === 1 ? frame.panes.find((p) => p.owns)?.pane : undefined;
/** Both owners and both counterpart geometries must be certain before any prior ANSI is parsed. */
function focusPair(before: SceneMarks, now: SceneMarks): Array<Omit<Focus, "prior">> | undefined {
  const oldOwner = owner(before), newOwner = owner(now);
  if (oldOwner === undefined || newOwner === undefined || oldOwner === newOwner) return undefined;
  const pair: Array<Omit<Focus, "prior">> = [];
  for (const current of now.panes) {
    if (current.pane !== oldOwner && current.pane !== newOwner) continue;
    const prior = before.panes.find((pane) => pane.pane === current.pane);
    if (!prior || !sameBorder(prior, current)) return undefined;
    pair.push({ before: prior, now: current, incoming: current.pane === newOwner });
  }
  return pair.length === 2 ? pair : undefined;
}
const SLOTS = ["status.error", "data.number", "status.warning", "status.success", "glow.cyan", "status.info", "glow.purple"];

/** The affected suffix is validated using full row paths, not counts, labels or sync keys. */
export function foldArea(before: TreeMark, now: TreeMark, fold: MotionFold): { area: Rect; descendants: number } | undefined {
  if (!sameRect(before.area, now.area) || !bounded(now.area) || before.pane !== fold.pane || now.pane !== fold.pane || before.slot !== fold.slot || now.slot !== fold.slot) return undefined;
  if (fold.before_open === fold.after_open || before.open.some((p) => pathEqual(p, fold.path)) !== fold.before_open || now.open.some((p) => pathEqual(p, fold.path)) !== fold.after_open) return undefined;
  const oldParent = before.rows.findIndex((r) => pathEqual(r.path, fold.path));
  const newParent = now.rows.findIndex((r) => pathEqual(r.path, fold.path));
  if (oldParent < 0 || oldParent !== newParent || before.rows[oldParent]!.y !== now.rows[newParent]!.y) return undefined;
  if (!before.rows.slice(0, oldParent + 1).every((r, i) => r.y === now.rows[i]!.y && pathEqual(r.path, now.rows[i]!.path))) return undefined;
  const expanded = fold.after_open ? now : before;
  const compact = fold.after_open ? before : now;
  let n = 0;
  for (const row of expanded.rows.slice(newParent + 1)) { if (!descendant(row.path, fold.path)) break; n++; }
  if (n === 0) return undefined;
  // Every surviving suffix row visible in both frames must preserve its identity and order.
  const a = expanded.rows.slice(newParent + 1 + n), b = compact.rows.slice(newParent + 1);
  if (b.some((r) => descendant(r.path, fold.path)) || !a.every((r, i) => !!b[i] && pathEqual(r.path, b[i]!.path))) return undefined;
  const y = now.rows[newParent]!.y + 1;
  const area = { x: now.area.x, y, w: now.area.w, h: now.area.y + now.area.h - y };
  if (!bounded(area) || !contained(area, now.area)) return undefined;
  // A viewport is a sequence of actual one-cell rows, never a wrapped or clipped anchor.
  for (const tree of [before, now]) if (!tree.rows.every((r, i) => r.y === tree.area.y + i)) return undefined;
  return { area, descendants: Math.min(n, area.h) };
}

export class Interactions {
  private baseline?: Baseline;
  private active_?: Active;
  private settle = false;
  private generation = 0;
  private sent?: { first: number; last: number; cursor: number; generation: number };
  private consumedFold?: string;
  constructor(private clock: () => number, private parse: typeof parseFrame = (text, width, height) => parseFrame(text, width, height, true)) {}

  get running(): boolean { return !!this.active_ || this.settle; }
  get inputGeneration(): number { return this.generation; }
  /** Even idle input invalidates response causality. Keep the last clean baseline for this new action. */
  input() {
    this.generation++;
    this.sent = undefined;
    if (this.active_) this.settle = true;
    this.active_ = undefined;
  }
  intent(seq: number, cursor: number) {
    if (this.sent?.generation === this.generation && this.sent.cursor === cursor) this.sent.last = seq;
    else this.sent = { first: seq, last: seq, cursor, generation: this.generation };
  }
  clear() {
    if (this.active_) this.settle = true;
    this.active_ = undefined;
    this.baseline = undefined;
    this.sent = undefined;
    this.consumedFold = undefined;
  }
  reset() { this.clear(); this.settle = false; this.generation = 0; }
  private accepts(seq: number | null | undefined): boolean {
    return seq != null && !!this.sent && this.sent.generation === this.generation && seq >= this.sent.first && seq <= this.sent.last;
  }

  observe(output: string, width: number, height: number, frame: SceneMarks | undefined, enabled: boolean) {
    const look = currentLook();
    if (!enabled || look.tier !== "truecolor" || !frame || frame.scene.blocked || output.length > MAX_BASELINE_ANSI || !bounded({ x: 0, y: 0, w: width, h: height }) || !frameMetadataBounded(frame)) {
      this.clear(); return;
    }
    const key = contextKey(frame.scene.context)!;
    const viewport = { x: 0, y: 0, w: width, h: height };
    // Invalid/partially clipped geometry never becomes a retained source.
    if (frame.panes.some((p) => !bounded(p.area) || !contained(p.area, viewport)) || frame.trees.some((t) => !bounded(t.area) || !contained(t.area, viewport))) { this.clear(); return; }
    const previous = this.baseline;
    const compatible = !!previous && previous.width === width && previous.height === height && previous.context === key && previous.look === look;
    if (!compatible) { this.active_ = undefined; this.settle = false; }
    if (this.active_) {
      const a = this.active_;
      if (a.cursor !== frame.scene.cursor || a.focus.length > 0 && owner(frame) !== a.focus.find((f) => f.incoming)?.now.pane || a.focus.some((f) => !frame.panes.some((p) => p.pane === f.now.pane && p.owns === f.now.owns && sameBorder(p, f.now))) || a.fold && !frame.trees.some((t) => treeEqual(t, a.fold!.mark))) {
        this.active_ = undefined; this.settle = true;
      }
    }
    if (compatible && previous) {
      const semantic = frame.scene.interaction;
      const fold = semantic?.fold;
      const foldId = fold ? JSON.stringify([contextKey(fold.context), fold.pane, fold.slot, fold.path, fold.before_open, fold.after_open, fold.origin_seq, fold.origin_cursor]) : undefined;
      const acceptedFold = fold && foldId !== this.consumedFold && this.accepts(fold.origin_seq) && fold.origin_cursor === previous.frame.scene.cursor && contextKey(fold.context) === key;
      // A local focus transfer may precede its snapshot, but its acknowledgement has the same owner and cannot replay it.
      const pair = focusPair(previous.frame, frame);
      const acceptedFocus = pair && (this.accepts(semantic?.origin_seq) || frame.scene.local && !!this.sent && this.sent.generation === this.generation);
      if (foldId) this.consumedFold = foldId;
      if (acceptedFold || acceptedFocus) {
        const stops = SLOTS.map((s) => parseHex(look.slots[s]));
        const next: Active = { context: key, look, width, height, cursor: frame.scene.cursor, start: this.clock(), focus: [], stops };
        let priorGrid: Grid | undefined;
        let currentGrid: Grid | undefined;
        const oldGrid = () => priorGrid ??= this.parse(previous.output, width, height);
        const newGrid = () => currentGrid ??= this.parse(output, width, height);
        if (acceptedFold && fold) {
          const before = previous.frame.trees.find((t) => t.pane === fold.pane && t.slot === fold.slot);
          const now = frame.trees.find((t) => t.pane === fold.pane && t.slot === fold.slot);
          const suffix = before && now ? foldArea(before, now, fold) : undefined;
          if (suffix && now) {
            const old = oldGrid().copyOf(suffix.area), current = newGrid().copyOf(suffix.area);
            next.fold = { rows: { ...suffix, expanded: fold.after_open ? current : old, compact: fold.after_open ? old : current, open: fold.after_open }, mark: now };
          }
        }
        if (acceptedFocus && pair && stops.every((n) => n >= 0)) {
          const captured = pair.map((focus) => ({ ...focus, prior: borderForegrounds(oldGrid(), focus.before) }));
          if (captured.every((focus) => focus.prior !== undefined)) next.focus = captured as Focus[];
        }
        if (next.fold || next.focus.length > 0) { this.active_ = next; this.settle = false; }
      }
    }
    // Keep only the newest clean source. An active fold owns only its clipped suffix cells, never an old full grid.
    this.baseline = frame.panes.length || frame.trees.length ? { output, width, height, frame, context: key, look } : undefined;
  }

  draw(grid: Grid, phase: "fold" | "focus" | "all" = "all"): Rect[] {
    if (phase !== "focus") this.settle = false;
    const a = this.active_;
    if (!a) return [];
    const elapsed = Math.max(0, Math.floor(this.clock() - a.start));
    const areas: Rect[] = [];
    if (phase !== "focus" && a.fold && elapsed < FOLD_MS && composeFold(grid, { x: 0, y: 0, w: a.width, h: a.height }, a.fold.rows, elapsed)) areas.push(a.fold.rows.area);
    for (const focus of phase === "fold" ? [] : a.focus) {
      const painted = focus.incoming ? focusLap(grid, focus.before, focus.now, focus.prior, a.stops, elapsed) : focusDrain(grid, focus.before, focus.now, focus.prior, elapsed);
      if (painted) areas.push(focus.now.area);
    }
    if (phase !== "fold" && elapsed >= (a.focus.length ? FOCUS_MS : FOLD_MS)) { this.active_ = undefined; this.settle = true; }
    return areas;
  }
}
