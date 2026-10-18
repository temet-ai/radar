import type { NoticeView } from "../protocol/generated/NoticeView.js";
import type { NoticeStateView } from "../protocol/generated/NoticeStateView.js";

export const NOTICE_LIMIT = 256;
/** Durations never depend on wall-clock corrections; clamp faulty injected clocks too. */
export function monotonicClock(read: () => number = () => performance.now()): () => number {
  let last = 0;
  return () => { const value = read(); if (Number.isFinite(value)) last = Math.max(last, value); return last; };
}

/** Bounded presentation mirror. The host owns occurrences, history and authoritative removals. */
export class NoticeStore {
  private readonly clock: () => number;
  private anchor = { host: 0, local: 0 };
  private synchronized = false;
  private through = 0;
  private revision = -1;
  private entries = new Map<number, NoticeView>();
  private retired = new Set<number>();
  private seen = 0;
  private clearedThrough = 0;
  private errorsThrough = 0;
  hidden = 0;
  constructor(clock: () => number = () => performance.now()) {
    this.clock = monotonicClock(clock); this.anchor.local = this.clock();
  }
  now(): number { return this.synchronized ? this.anchor.host + Math.max(0, this.clock() - this.anchor.local) : 0; }
  private syncClock(now: number) {
    const local = this.clock();
    this.anchor = { host: Math.max(this.now(), now), local }; this.synchronized = true;
  }
  private allowed(notice: NoticeView): boolean {
    return notice.id > this.clearedThrough && (notice.tone !== "error" || notice.id > this.errorsThrough);
  }
  private bounded() {
    this.entries = new Map([...this.entries].sort(([a], [b]) => a - b).slice(-NOTICE_LIMIT));
    // The host retains at most 256 active occurrences. Older IDs can no longer be replayed as active.
    for (const id of this.retired) if (id <= this.seen - NOTICE_LIMIT) this.retired.delete(id);
  }
  reconcile(state: NoticeStateView): void {
    if (state.revision < this.revision || state.through_id < this.through) return;
    this.syncClock(state.now_ms);
    this.revision = state.revision; this.through = state.through_id; this.seen = Math.max(this.seen, state.through_id);
    const authoritative = new Set(state.active.map((n) => n.id));
    for (const id of this.entries.keys()) if (id <= this.through && !authoritative.has(id)) this.entries.delete(id);
    for (const notice of state.active) if (this.allowed(notice) && !this.retired.has(notice.id)) this.entries.set(notice.id, notice);
    this.hidden = state.hidden; this.bounded(); this.active();
  }
  immediate(notice: NoticeView, revision: number, now: number): void {
    // A snapshot already decided whether older IDs are active. Replayed delivery cannot revive one.
    if (!this.allowed(notice) || notice.id <= this.through || revision < this.revision || this.entries.has(notice.id) || this.retired.has(notice.id)) return;
    this.syncClock(now); this.entries.set(notice.id, notice); this.seen = Math.max(this.seen, notice.id); this.bounded(); this.active();
  }
  active(): NoticeView[] {
    const now = this.now();
    for (const [id, notice] of this.entries) if (notice.lifetime_ms <= 0 || now - notice.created_ms >= notice.lifetime_ms) {
      this.entries.delete(id); this.retired.add(id);
    }
    return [...this.entries.values()].sort((a, b) => a.id - b.id);
  }
  get throughId(): number { return this.seen; }
  dismiss(id: number): boolean {
    this.active(); if (!this.entries.delete(id)) return false; this.retired.add(id); return true;
  }
  input(escape: boolean): boolean {
    let changed = false;
    const active = this.active();
    // Fences cover hidden IDs never present in the latest-four mirror, not just currently painted occurrences.
    if (escape) this.clearedThrough = Math.max(this.clearedThrough, this.seen);
    this.errorsThrough = Math.max(this.errorsThrough, this.seen);
    for (const notice of active) if (escape || notice.tone === "error") changed = this.dismiss(notice.id) || changed;
    return changed;
  }
}
