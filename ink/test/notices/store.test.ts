import { describe, expect, it, vi } from "vitest";
import { monotonicClock, NoticeStore } from "../../src/notices/store.js";
import { FakeConnection } from "../../src/transport/fake.js";
import { Session, capabilitiesFrom } from "../../src/app/session.js";
import type { NoticeView } from "../../src/protocol/generated/NoticeView.js";
import { notice } from "./fixture.js";
const state = (active: NoticeView[], over = {}) => ({ now_ms: 0, revision: 1, through_id: Math.max(0, ...active.map((n) => n.id)), active, hidden: 0, ...over });
describe("bounded occurrence presentation reconciliation", () => {
  it("merges immediate occurrences with coalesced snapshots and preserves identical words as distinct IDs", () => {
    const store = new NoticeStore(() => 0);
    store.immediate(notice(1), 1, 0); store.immediate(notice(2), 2, 0); store.immediate(notice(2), 2, 0);
    store.reconcile(state([notice(1)])); expect(store.active().map((n) => n.id)).toEqual([1, 2]);
    store.reconcile(state([notice(1), notice(2)], { revision: 2 })); expect(store.active().map((n) => n.id)).toEqual([1, 2]); expect(store.throughId).toBe(2);
  });
  it("does not extend lifetime on duplicate/unchanged snapshots, immediate replay, or backward host time", () => {
    let now = 100; const store = new NoticeStore(() => now); store.reconcile(state([notice(1)]));
    now += 2500; store.reconcile(state([notice(1)])); store.immediate(notice(1), 1, 0); expect(store.now()).toBe(2500);
    now += 2499; store.reconcile(state([notice(1)], { now_ms: 2000 })); expect(store.active()).toHaveLength(1);
    now++; expect(store.active()).toEqual([]); store.reconcile(state([notice(1)])); expect(store.active()).toEqual([]);
  });
  it("starts at the first host timestamp, excluding time spent connecting", () => {
    let now = 0; const store = new NoticeStore(() => now); now = 20000; store.immediate(notice(1), 1, 0);
    expect(store.now()).toBe(0); expect(store.active()).toHaveLength(1);
  });
  it("keeps stable-ID dismissal tombstones across stale snapshots and hidden/revealed occurrences", () => {
    const store = new NoticeStore(() => 0); store.reconcile(state([notice(1)])); expect(store.dismiss(1)).toBe(true);
    store.reconcile(state([notice(1)])); store.reconcile(state([notice(2)], { revision: 2, hidden: 1 }));
    store.reconcile(state([notice(1)], { revision: 3, through_id: 2 })); store.immediate(notice(1), 4, 0);
    expect(store.active()).toEqual([]); expect(store.dismiss(1)).toBe(false);
  });
  it.each([false, true])("fences hidden IDs revealed by a delayed newer snapshot (Esc=%s)", (escape) => {
    const store = new NoticeStore(() => 0);
    store.reconcile(state([5, 6, 7, 8].map((id) => notice(id)), { revision: 8, through_id: 8, hidden: 4 })); store.input(escape);
    store.reconcile(state([1, 2, 3, 4].map((id) => notice(id, { tone: "error", lifetime_ms: 10000 })), { revision: 9, through_id: 8, now_ms: 5000 }));
    expect(store.active()).toEqual([]); store.immediate(notice(9, { tone: "error", created_ms: 5000 }), 10, 5000); expect(store.active().map((n) => n.id)).toEqual([9]);
    const infos = new NoticeStore(() => 0); infos.reconcile(state([notice(8)], { through_id: 8, hidden: 1 })); infos.input(escape);
    infos.reconcile(state([notice(1)], { revision: 2, through_id: 8 })); expect(infos.active().map((n) => n.id)).toEqual(escape ? [] : [1]);
  });
  it("accepts authoritative removals and rejects older revisions or watermarks", () => {
    const store = new NoticeStore(() => 0); store.reconcile(state([notice(1)], { revision: 2 }));
    store.reconcile(state([], { revision: 3, through_id: 1 })); store.reconcile(state([notice(1)], { revision: 2 })); store.immediate(notice(1), 4, 0); expect(store.active()).toEqual([]);
  });
  it("reconnects with only the host's unexpired active occurrences", () => {
    const store = new NoticeStore(() => 100);
    store.reconcile(state([notice(2), notice(3, { tone: "error", lifetime_ms: 10000 })], { now_ms: 6000, through_id: 3 })); expect(store.active().map((n) => n.id)).toEqual([3]);
  });
  it("ordinary keys dismiss errors, Esc clears all, and expiry uses exact 5s/10s boundaries", () => {
    let now = 0; const store = new NoticeStore(() => now), error = notice(2, { tone: "error", lifetime_ms: 10000 });
    store.reconcile(state([notice(1), error])); expect(store.input(false)).toBe(true); expect(store.active().map((n) => n.id)).toEqual([1]); expect(store.input(true)).toBe(true); expect(store.active()).toEqual([]);
    const other = new NoticeStore(() => now); other.reconcile(state([notice(1), error]));
    now = 4999; expect(other.active()).toHaveLength(2); now = 5000; expect(other.active()).toHaveLength(1); now = 9999; expect(other.active()).toHaveLength(1); now = 10000; expect(other.active()).toHaveLength(0);
  });
  it("bounds pending occurrences and tombstones across coalesced delivery", () => {
    const store = new NoticeStore(() => 0); for (let i = 1; i <= 1000; i++) store.immediate(notice(i), i, 0);
    expect(store.active()).toHaveLength(256); store.input(true); expect((store as any).retired.size).toBeLessThanOrEqual(256);
    store.immediate(notice(1001), 1001, 0); expect((store as any).retired.size).toBeLessThanOrEqual(256);
  });
  it("clamps backward injected time and is immune to wall-clock changes", () => {
    let now = 100; const read = monotonicClock(() => now); expect(read()).toBe(100); now = -500; expect(read()).toBe(100); now = NaN; expect(read()).toBe(100);
    const spy = vi.spyOn(Date, "now").mockReturnValue(-1e12), actual = monotonicClock(); const before = actual(); spy.mockReturnValue(1e12); expect(actual()).toBeGreaterThanOrEqual(before); spy.mockRestore();
  });
  it("keeps rich delivery separate from legacy strings and sends one bounded presentation intent", () => {
    const conn = new FakeConnection(), session = new Session(conn, capabilitiesFrom({}, {}), undefined, false, () => 0);
    conn.receive({ type: "notice", cursor: 0, text: "Watch updated", occurrence: notice(7, { tone: "error" }), revision: 1, now_ms: 0 });
    expect(session.state.notices).toEqual([]); expect(session.noticeStore.active()[0]?.tone).toBe("error");
    session.noticeInput(false); session.sendIntent({ intent: "key", chord: { key: "esc" } });
    expect(conn.sent.map((m: any) => m.intent)).toEqual([{ intent: "notice_input", escape: false, through_id: 7 }, { intent: "key", chord: { key: "esc" } }]);
    session.dismissNoticeId(7); expect(conn.sent).toHaveLength(2);
  });
});
