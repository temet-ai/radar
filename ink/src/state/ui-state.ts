import type { Hit } from "../chrome/types.js";

/**
 * Transient renderer state keyed by target: focus, scroll and pane widths. Nothing here is the
 * host's truth (the host owns selection); it only keeps what the renderer alone knows from
 * moving when a snapshot arrives, when the terminal resizes, or after a restart.
 */
export class UiState {
  /** The target the user last selected or activated (the cursor the renderer shows). */
  focus: string | undefined;
  /** The pane the host says has the keys (from the newest snapshot). */
  focusPane: string | undefined;
  scroll = new Map<string, number>();
  paneWidth = new Map<string, number>();

  /** A snapshot arrived. Focus stays on its target while the target still exists, else it is dropped. */
  applySnapshot(snapshot: { focus?: unknown }, known: Iterable<string>) {
    if (typeof snapshot.focus === "string") this.focusPane = snapshot.focus;
    if (this.focus !== undefined) {
      const set = known instanceof Set ? (known as Set<string>) : new Set(known);
      if (set.size > 0 && !set.has(this.focus)) this.focus = undefined;
    }
  }

  select(target: string) {
    this.focus = target;
  }

  scrollOf(key: string): number {
    return this.scroll.get(key) ?? 0;
  }
  setScroll(key: string, offset: number) {
    this.scroll.set(key, Math.max(0, offset));
  }
  /** A resize never moves focus; it only drops scroll offsets that can no longer be reached. */
  resize(maxScroll: (key: string) => number) {
    for (const [k, v] of this.scroll) this.scroll.set(k, Math.min(v, Math.max(0, maxScroll(k))));
  }

  /** What survives a renderer restart is re-sent by the host in its first snapshot; this is the local part. */
  save(): { focus?: string; scroll: Record<string, number> } {
    return { ...(this.focus ? { focus: this.focus } : {}), scroll: Object.fromEntries(this.scroll) };
  }
  restore(saved: { focus?: string; scroll?: Record<string, number> }) {
    this.focus = saved.focus;
    this.scroll = new Map(Object.entries(saved.scroll ?? {}));
  }
}

/** Where a pane is drawn, frame included: what the wheel over it scrolls and a press on its empty space focuses. */
export type PaneArea = { x: number; y: number; w: number; h: number; pane: string };

/** Hit rectangles the body's node components register each frame (T11/T12 call `add`), and the panes' areas
 * (`addPane`), kept apart so a pane never covers its own rows. */
export class HitRegistry {
  private hits: Hit[] = [];
  private areas: PaneArea[] = [];
  private cleared = false;
  /** A frame is about to register its hits. The old ones stay answerable until the first new one arrives (or `settle`),
   * so a render that yields to the event loop between its render phase and its commit does not leave a key or click
   * that arrives in that gap with nothing to hit. */
  clear() {
    this.cleared = true;
  }
  private fresh() {
    if (this.cleared) {
      this.hits = [];
      this.areas = [];
      this.cleared = false;
    }
  }
  add(hit: Hit) {
    this.fresh();
    this.hits.push(hit);
  }
  addPane(area: PaneArea) {
    this.fresh();
    this.areas.push(area);
  }
  /** The frame's registrations are done (App's layout effect, after its children's): a frame that registered none has none. */
  settle() {
    this.fresh();
  }
  all(): Hit[] {
    return this.hits;
  }
  /** The pane drawn at the cell, if any (the last registered wins, as for hits). */
  paneAt(x: number, y: number): string | undefined {
    for (let i = this.areas.length - 1; i >= 0; i--) {
      const a = this.areas[i]!;
      if (x >= a.x && x < a.x + a.w && y >= a.y && y < a.y + a.h) return a.pane;
    }
    return undefined;
  }
}
