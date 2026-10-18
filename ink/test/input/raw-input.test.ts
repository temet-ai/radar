import { describe, expect, it } from "vitest";
import { MouseBus, tapMouse } from "../../src/input/stdin-tap.js";

// A caller-owned read queue exercises the tap without a process, clock or live terminal.
const source = (chunks: Array<string | Buffer>) => ({ read: (): string | Buffer | null => chunks.shift() ?? null });

describe("raw terminal input observers", () => {
  it("observes a claimed read exactly once before forwarding every byte unchanged", () => {
    const bus = new MouseBus();
    const order: string[][] = [];
    const text = "\x00\x1b[200~Watch\r\n\x1b[201~\x1b[<35;3;2M\x1b[29~";
    bus.onRawInput((bytes) => order.push(["observe", bytes]));
    bus.claimRaw((bytes) => order.push(["terminal", bytes]));
    const stdin = source([Buffer.from(text.slice(0, 13)), text.slice(13)]);
    tapMouse(stdin, bus);
    expect(stdin.read()).toBeNull();
    expect(order).toEqual([["observe", text], ["terminal", text]]);
    expect(stdin.read()).toBeNull();
    expect(order).toHaveLength(2);
  });

  it("leaves pointer-only raw filtering to the observer and never changes the raw claim", () => {
    const bus = new MouseBus();
    const observed: string[] = [];
    const received: string[] = [];
    bus.onRawInput((text) => observed.push(text));
    bus.claimRaw((text) => received.push(text));
    const motion = "\x1b[<35;3;2M";
    expect(bus.emitRaw(motion)).toBe(true);
    expect(observed).toEqual([motion]);
    expect(received).toEqual([motion]);
  });

  it("does not double-report tokenized reads or take ownership with only an observer", () => {
    const bus = new MouseBus();
    const observed: string[] = [];
    bus.onRawInput((text) => observed.push(text));
    expect(bus.emitInputReady()).toBe(false);
    expect(bus.emitRaw("j")).toBe(false);
    const untouched = source(["j"]);
    tapMouse(untouched, bus);
    expect(untouched.read()).toBe("j");
    const events: unknown[] = [];
    bus.onInput((input) => events.push(...input));
    const stdin = source(["j"]);
    tapMouse(stdin, bus);
    expect(stdin.read()).toBeNull();
    expect(events).toEqual([{ kind: "key", chord: { key: { char: "j" } } }]);
    expect(observed).toEqual([]);
  });

  it("detaches observers and raw claims independently without replaying prior reads", () => {
    const bus = new MouseBus();
    const received: string[] = [];
    const release = bus.claimRaw((text) => received.push(text));
    bus.emitRaw("before");
    const observed: string[] = [];
    const detach = bus.onRawInput((text) => observed.push(text));
    expect(observed).toEqual([]);
    bus.emitRaw("during");
    detach();
    detach();
    bus.emitRaw("after");
    release();
    expect(bus.emitRaw("unclaimed")).toBe(false);
    expect(observed).toEqual(["during"]);
    expect(received).toEqual(["before", "during", "after"]);
  });

  it("delivers the observed event to the claim selected at its start", () => {
    const bus = new MouseBus();
    const order: string[] = [];
    bus.claimRaw((text) => order.push(`first:${text}`));
    bus.onRawInput(() => { bus.claimRaw((text) => order.push(`next:${text}`)); });
    bus.emitRaw("a");
    bus.emitRaw("b");
    expect(order).toEqual(["first:a", "next:b"]);
  });
});
