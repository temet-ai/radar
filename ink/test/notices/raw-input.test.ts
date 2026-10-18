import { afterEach, describe, expect, it, vi } from "vitest";
import { RawNoticeInput } from "../../src/notices/raw-input.js";
import { MouseBus } from "../../src/input/stdin-tap.js";
afterEach(() => vi.useRealTimers());
describe("read-only notice input under raw PTY ownership", () => {
  it("observes ~ without changing a byte or routing a host history key", () => {
    const bus = new MouseBus(), notices: boolean[] = [], bytes: string[] = [], observe = new RawNoticeInput((escape) => notices.push(escape));
    bus.onRawInput((text) => observe.take(text)); bus.claimRaw((text) => bytes.push(text)); bus.emitRaw("~"); bus.emitRaw("\x1b[A");
    expect(bytes).toEqual(["~", "\x1b[A"]); expect(notices).toEqual([false, false]); observe.reset();
  });
  it("does not interpret complete or split SGR/arrow reports as Escape", () => {
    vi.useFakeTimers(); const notices: boolean[] = [], observe = new RawNoticeInput((escape) => notices.push(escape));
    observe.take("\x1b"); observe.take("[<0;12;"); observe.take("21M"); observe.take("\x1b[<0;12;21m"); observe.take("\x1b"); observe.take("[A");
    expect(notices).toEqual([false]); vi.advanceTimersByTime(40); expect(notices).toEqual([false]); expect(vi.getTimerCount()).toBe(0); observe.reset();
  });
  it("settles lone Escape once and tears down ambiguous-prefix timers", () => {
    vi.useFakeTimers(); const notices: boolean[] = [], observe = new RawNoticeInput((escape) => notices.push(escape));
    observe.take("\x1b"); expect(notices).toEqual([]); vi.advanceTimersByTime(30); expect(notices).toEqual([true]); observe.take("\x1b["); observe.reset(); vi.advanceTimersByTime(30); expect(notices).toEqual([true]); expect(vi.getTimerCount()).toBe(0);
  });
  it("does not turn bracketed pasted escapes or a large split paste into keys", () => {
    const notices: boolean[] = [], observe = new RawNoticeInput((escape) => notices.push(escape));
    observe.take("\x1b[200~Watch ~ \x1b[A\x1b[201~"); observe.take("\x1b[200~" + "x".repeat(5000)); observe.take("~".repeat(5000)); observe.take("\x1b[20"); observe.take("1~~"); expect(notices).toEqual([false]); expect((observe as any).rest.length).toBeLessThanOrEqual(4096); observe.reset();
  });
});
