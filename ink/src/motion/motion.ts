// The one motion controller of the renderer: cues, input, the marks of the frame, the frame tick, and the hook that
// applies the running effects to a frame. Costs nothing while nothing runs: `apply` returns the frame it was given
// without parsing ANSI or arming a timer. Full motion retains one bounded clean frame for a possible next interaction.
import Output from "ink-output";
import { useSyncExternalStore } from "react";
import { currentLook } from "../text/look.js";
import { parseHex } from "./color.js";
import type { Rect, Grid } from "./grid.js";
import { monotonicClock } from "../notices/store.js";
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
import { Interactions, type MotionScene, type PaneMark, type TreeMark, type SceneMarks } from "./interaction.js";
import { frameMetadataBounded, sceneMetadataBounded } from "./interaction-bounds.js";
import { bounded, MAX_BASELINE_ANSI } from "./interaction-effects.js";

// Loading may cross an edge, but neither malformed coordinates nor an unbounded frame may reach the ANSI parser.
const visibleLoading = (area: Rect, width: number, height: number): boolean =>
  [area.x, area.y, area.w, area.h, area.x + area.w, area.y + area.h].every(Number.isSafeInteger) &&
  area.w > 0 && area.h > 0 && area.x < width && area.y < height && area.x + area.w > 0 && area.y + area.h > 0;
const boundedOutput = (output: string, width: number, height: number): boolean =>
  output.length <= MAX_BASELINE_ANSI && bounded({ x: 0, y: 0, w: width, h: height });

/** Painted after all effects; never retained as a clean source. */
export interface PresentationLayer {
  visible(): boolean;
  running(): boolean;
  draw(grid: Grid): Rect[];
  clear(): void;
}

export class Motion {
  engine: Engine;
  readonly interactions: Interactions;
  private committed?: SceneMarks;
  private preparing?: MotionScene;
  // Ordinary node marks use render identity even when interaction metadata is absent or rejected.
  private preparingNodes?: MotionScene;
  private committedNodes?: MotionScene;
  private invalidScenes = new WeakSet<MotionScene>();
  private firstOutput?: { scene: MotionScene; output: string; width: number; height: number };
  private paneMarks = new Map<string, { scene: MotionScene; mark: PaneMark }>();
  private treeMarks = new Map<string, { scene: MotionScene; mark: TreeMark }>();
  private marks: Marks = {};
  private marksFn: (() => Marks) | undefined;
  private nodes = new Map<
    string,
    { rows: ReadonlyArray<readonly [string, Rect]>; loading: readonly Rect[]; scene?: MotionScene }
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
  private presentation?: PresentationLayer;
  // Also pauses a live loader rejected by output bounds, until a valid output arrives.
  private presentationBlocked = false;
  private onNotice: ((text: string) => void) | undefined;

  constructor(
    level: Level = "off",
    private clock: () => number = () => performance.now(),
  ) {
    this.clock = clock = monotonicClock(clock);
    this.engine = new Engine(level, clock);
    this.interactions = new Interactions(() => this.clock());
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
  reset(clock?: () => number) {
    if (clock) this.clock = monotonicClock(clock);
    if (this.timer !== undefined) clearTimeout(this.timer);
    this.timer = undefined;
    this.armedAt = 0;
    this.engine = new Engine("off", this.clock);
    this.nodes.clear();
    this.interactions.reset();
    this.paneMarks.clear();
    this.treeMarks.clear();
    this.committed = undefined;
    this.preparing = undefined;
    this.preparingNodes = undefined;
    this.committedNodes = undefined;
    this.invalidScenes = new WeakSet();
    this.firstOutput = undefined;
    this.marksFn = undefined;
    this.marks = {};
    this.presentation?.clear();
    this.presentation = undefined;
    this.presentationBlocked = false;
  }

  /** `level` is the host's, after `motion::effective` (off over SSH and below full colour). */
  setLevel(level: Level | string) {
    const was = this.active;
    this.engine.setLevel(typeof level === "string" ? parseLevel(level) : level);
    if (this.engine.level !== "full") this.interactions.reset();
    // Off drops what ran: one more frame puts the plain one on screen.
    this.arm(was && !this.active);
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
    this.interactions.input();
    this.engine.input();
    this.arm();
  }

  intent(seq: number, cursor: number) { this.interactions.intent(seq, cursor); }

  /** Resize, theme changes, overlays or a lost connection invalidate retained geometry immediately. */
  invalidate() {
    this.interactions.clear();
    this.engine.input();
    this.arm();
  }

  private sceneMarks(scene: MotionScene): SceneMarks {
    return { scene,
      panes: [...this.paneMarks.values()].filter((p) => p.scene === scene).map((p) => p.mark),
      trees: [...this.treeMarks.values()].filter((p) => p.scene === scene).map((p) => p.mark) };
  }
  private rejectScene(scene: MotionScene) {
    this.invalidScenes.add(scene);
    // A rejected replacement must not leave an older preparing scene to claim its output.
    // Fail closed for the whole measured frame, while allowing the ordinary engine to draw it.
    this.paneMarks.clear();
    this.treeMarks.clear();
    this.preparing = undefined;
    this.committed = undefined;
    this.firstOutput = undefined;
    this.interactions.clear();
  }
  markPane(id: string, scene: MotionScene, mark?: PaneMark) {
    this.paneMarks.delete(id);
    if (!mark || this.engine.level !== "full" || this.invalidScenes.has(scene)) return;
    const next = this.sceneMarks(scene);
    next.panes = [...next.panes, mark];
    if (!frameMetadataBounded(next)) { this.rejectScene(scene); return; }
    this.paneMarks.set(id, { scene, mark });
  }
  markTree(id: string, scene: MotionScene, mark?: TreeMark) {
    this.treeMarks.delete(id);
    if (!mark || this.engine.level !== "full" || this.invalidScenes.has(scene)) return;
    const next = this.sceneMarks(scene);
    next.trees = [...next.trees, mark];
    if (!frameMetadataBounded(next)) { this.rejectScene(scene); return; }
    this.treeMarks.set(id, { scene, mark });
  }
  prepareScene(scene: MotionScene) {
    this.preparingNodes = scene;
    if (this.preparing !== scene) { this.paneMarks.clear(); this.treeMarks.clear(); }
    if (this.engine.level !== "full") { this.preparing = undefined; return; }
    if (!sceneMetadataBounded(scene)) { this.rejectScene(scene); return; }
    this.preparing = scene;
  }

  /** Parent layout effect follows child layout effects. Only this scene's measured marks may accompany its ANSI. */
  commitScene(scene: MotionScene) {
    if (scene === this.preparingNodes) this.committedNodes = scene;
    if (this.engine.level !== "full" || this.invalidScenes.has(scene)) {
      this.committed = undefined;
      this.firstOutput = undefined;
      return;
    }
    const frame = this.sceneMarks(scene);
    if (!frameMetadataBounded(frame)) { this.rejectScene(scene); return; }
    this.committed = frame;
    const initial = this.firstOutput;
    this.firstOutput = undefined;
    if (initial?.scene === scene) this.interactions.observe(initial.output, initial.width, initial.height, this.committed, true);
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
   * Replaced as a whole each time the node lays out; `owner` is any id unique to the node.
   * App nodes supply their actual render scene; direct controller fixtures may omit it. */
  markNode(
    owner: string,
    rows: ReadonlyArray<readonly [string, Rect]>,
    loading: readonly Rect[] = [],
    scene?: MotionScene,
  ) {
    if (rows.length === 0 && loading.length === 0) this.nodes.delete(owner);
    else this.nodes.set(owner, { rows, loading, scene });
  }

  private currentNode(scene?: MotionScene): boolean {
    return this.preparingNodes === this.committedNodes && (!scene || scene === this.committedNodes);
  }

  /** The idle check reads loading only; ordinary row arrays are not flattened until an engine draw is needed. */
  private loadingMarks(base: Marks, width: number, height: number): Rect[] {
    if (this.preparingNodes !== this.committedNodes || this.committedNodes?.blocked) return [];
    if (this.nodes.size === 0) return (base.loading ?? []).filter((area) => visibleLoading(area, width, height));
    const loading: Rect[] = [];
    for (const n of this.nodes.values()) if (this.currentNode(n.scene))
      for (const area of n.loading) if (visibleLoading(area, width, height)) loading.push(area);
    return loading;
  }

  private frameMarks(base: Marks, loading: readonly Rect[]): Marks {
    if (this.nodes.size === 0) return { ...base, loading };
    const rows: Array<readonly [string, Rect]> = [];
    // Row cues retain their existing ordering at Subtle, whose output can precede layout.
    for (const n of this.nodes.values()) rows.push(...n.rows);
    return { ...base, rows, loading };
  }

  private get effectsActive(): boolean { return this.engine.running() || this.interactions.running; }
  get active(): boolean { return !this.presentationBlocked && (this.effectsActive || !!this.presentation?.running()); }
  setPresentation(layer: PresentationLayer) {
    this.presentation?.clear(); this.presentation = layer; this.presentationBlocked = false; this.refreshPresentation();
  }
  refreshPresentation() {
    if (!this.active && this.timer !== undefined) { clearTimeout(this.timer); this.timer = undefined; }
    this.arm();
  }
  private late(output: string, width: number, height: number): string {
    if (!this.presentation?.visible()) { this.presentation?.clear(); this.refreshPresentation(); return output; }
    const grid = parseFrame(output, width, height, true), areas = this.presentation.draw(grid);
    this.refreshPresentation();
    return areas.length ? writeFrame(grid, output, areas) : output;
  }

  subscribe = (fn: () => void) => {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  };
  version = () => this.ticks;

  /** Ask for a frame soon, only while an effect runs. */
  private arm(force = false) {
    if (this.timer !== undefined || !(force || this.active)) return;
    this.armedAt = this.clock();
    this.timer = setTimeout(() => {
      this.timer = undefined;
      this.ticks += 1;
      for (const l of [...this.listeners]) l();
    }, FRAME_MS);
  }

  /** Apply what runs to the frame Ink drew (its string); the same string when nothing runs. */
  apply(output: string, width: number, height: number): string {
    const outputBounded = boundedOutput(output, width, height);
    const rejectedLoading = !outputBounded && this.engine.discardLoading();
    // Late layers and a rejected live loader obey the same bounds before parsing or retaining any frame.
    // Keep other cues intact for the next valid output; a malformed loading frame cannot keep scheduling work.
    if (this.presentation || this.presentationBlocked || rejectedLoading) {
      this.presentationBlocked = !outputBounded;
      if (this.presentationBlocked) {
        this.presentation?.clear();
        if (this.timer !== undefined) clearTimeout(this.timer);
        this.timer = undefined;
        this.armedAt = 0;
        return output;
      }
    }
    // An output for a replacement is already clean. Drop only the old loading effect before the commit gate;
    // it must not keep an uncommitted render alive or force ANSI parsing merely to reconcile an absent owner.
    if (this.preparingNodes !== this.committedNodes && this.engine.discardLoading() && !this.effectsActive)
      this.armedAt = 0;
    // On the first mount Ink can emit a clean frame before refs/layout effects exist. Retain it only until
    // that exact render commits; never apply an effect to geometry from a different committed scene.
    if (this.engine.level === "full" && this.preparing?.context && this.preparing !== this.committed?.scene) {
      if (!this.committed && this.engine.level === "full" && output.length <= 1_048_576 && width * height <= 65_536)
        this.firstOutput = { scene: this.preparing, output, width, height };
      else this.interactions.clear();
      return this.late(output, width, height);
    }
    if (this.engine.level === "off") {
      this.interactions.reset();
      if (this.timer !== undefined) clearTimeout(this.timer);
      this.timer = undefined;
      return this.late(output, width, height);
    }
    // A child may have laid out without an App render. Off/subtle do no interaction geometry/retention work.
    if (this.engine.level === "full") {
      if (this.committed) this.commitScene(this.committed.scene);
      this.interactions.observe(output, width, height, this.committed, true);
    }
    // Discover uncued loading on the actual full-motion output, never from registration or an active/timer getter.
    // Read the shell marks at most once and preserve the node-over-shell precedence for this exact frame.
    const mayLoad = this.engine.level === "full" && outputBounded;
    const base = this.effectsActive || mayLoad ? this.marksFn?.() ?? this.marks : undefined;
    const loading = mayLoad && base ? this.loadingMarks(base, width, height) : [];
    if (!this.effectsActive && loading.length === 0) {
      if (this.timer !== undefined) clearTimeout(this.timer);
      this.timer = undefined;
      this.armedAt = 0;
      return this.late(output, width, height);
    }
    const started = this.clock();
    const grid = parseFrame(output, width, height, loading.length > 0 || this.interactions.running || !!this.presentation?.visible());
    const areas = this.interactions.draw(grid, "fold");
    areas.push(...this.engine.draw(grid, this.frameMarks(base!, loading), this.colours()));
    areas.push(...this.interactions.draw(grid, "focus"));
    if (this.presentation?.visible()) areas.push(...this.presentation.draw(grid));
    else this.presentation?.clear();
    const out = areas.length > 0 ? writeFrame(grid, output, areas) : output;
    const lag = this.timer === undefined && this.armedAt ? Math.max(0, started - this.armedAt - FRAME_MS) : 0;
    this.armedAt = 0;
    const why = this.engine.drewIn(
      this.clock() - started + lag,
      this.interactions.running,
    );
    if (why) { this.interactions.clear(); this.onNotice?.(why); }
    // Motion just went off with an effect on screen: one more frame puts the plain one there.
    this.arm(Boolean(why));
    if (!why && !this.active && this.timer !== undefined) {
      clearTimeout(this.timer);
      this.timer = undefined;
    }
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
  if (target !== m) target.reset();
  target = m;
  if (installed) return;
  installed = true;
  const base = (Output.prototype as any).get as () => {
    output: string;
    height: number;
  };
  (Output.prototype as any).get = function (this: any) {
    const r = base.call(this);
    const output = target.apply(r.output, this.width, this.height);
    return output === r.output ? r : { output, height: r.height };
  };
}

/** Whether a read from stdin is input that stops motion: anything but a pointer's plain motion reports (button 35, with any modifiers). */
export function isInput(chunk: string | Buffer): boolean {
  const s = typeof chunk === "string" ? chunk : chunk.toString("latin1");
  return (
    s.replace(/\x1b\[<(?:35|39|43|47|51|55|59|63);\d+;\d+[Mm]/g, "") !== ""
  );
}


/** Ink's resetAfterCommit requests output before React layout effects. Defer that existing request until all
 * geometry and the scene have committed. Coalesce only requests in this turn; no timer or extra render is added. */
export function afterMotionLayout(root: { onRender?: () => void; onImmediateRender?: () => void }, enabled: () => boolean = () => true): () => void {
  const render = root.onRender;
  const immediate = root.onImmediateRender;
  let queued = false;
  let alive = true;
  let urgent = false;
  let version = 0;
  const request = (now: boolean) => {
    if (!alive) return;
    if (!enabled()) {
      version++;
      queued = false;
      urgent = false;
      (now ? immediate ?? render : render)?.();
      return;
    }
    urgent ||= now;
    if (queued) return;
    queued = true;
    const requested = version;
    queueMicrotask(() => {
      if (!alive || requested !== version) return;
      queued = false;
      const draw = urgent ? immediate ?? render : render;
      urgent = false;
      draw?.();
    });
  };
  const deferred = () => request(false);
  const deferredImmediate = () => request(true);
  root.onRender = deferred;
  root.onImmediateRender = deferredImmediate;
  return () => {
    alive = false;
    if (root.onRender === deferred) root.onRender = render;
    if (root.onImmediateRender === deferredImmediate) root.onImmediateRender = immediate;
  };
}
