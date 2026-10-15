import React from "react";
import { describe, expect, it } from "vitest";
import { render } from "ink-testing-library";
import { MouseBus } from "../../src/input/stdin-tap.js";
import { TerminalView } from "../../src/terminal/TerminalView.js";
import type { Frame, TerminalState } from "../../src/terminal/frames.js";
import type { TerminalLink } from "../../src/terminal/link.js";

/** A link the test drives: frames in, what the surface sent out. */
function fakeLink() {
  let onFrame: (f: Frame) => void = () => {};
  let onClose: (why: string) => void = () => {};
  const sent = { input: [] as string[], resize: [] as [number, number][], restart: 0, closed: 0 };
  const link: TerminalLink = {
    onFrame: (fn) => void (onFrame = fn),
    onClose: (fn) => void (onClose = fn),
    input: (d) => void sent.input.push(typeof d === "string" ? d : new TextDecoder().decode(d)),
    resize: (c, r) => void sent.resize.push([c, r]),
    restart: () => void sent.restart++,
    kill: () => {},
    close: () => void sent.closed++,
  };
  const state = (s: Partial<TerminalState> = {}): TerminalState => ({ state: "running", generation: 1, columns: 40, rows: 6, ...s });
  return {
    link,
    sent,
    state,
    feed: (f: Frame) => onFrame(f),
    out: (text: string) => onFrame({ kind: "output", bytes: new TextEncoder().encode(text) }),
    lose: (why: string) => onClose(why),
  };
}

/** Waits on the drawn frame, not on a clock: output is parsed and drawn a moment after it arrives. */
async function drawn(r: { lastFrame(): string | undefined }, want: (frame: string) => boolean) {
  const end = Date.now() + 3000;
  while (Date.now() < end) {
    if (want(r.lastFrame() ?? "")) return;
    await new Promise((x) => setTimeout(x, 10));
  }
  throw new Error(`never drew it; last frame:\n${r.lastFrame()}`);
}

function mount(extra: Partial<React.ComponentProps<typeof TerminalView>> = {}) {
  const f = fakeLink();
  const bus = new MouseBus();
  const left: string[] = [];
  const exited: TerminalState[] = [];
  const r = render(
    <TerminalView
      open={() => f.link}
      openKey="t"
      columns={40}
      rows={6}
      focused
      bus={bus}
      origin={{ row: 1, col: 1 }}
      onLeave={(w) => left.push(w)}
      onExit={(s) => exited.push(s)}
      {...extra}
    />,
  );
  return { f, bus, left, exited, r };
}

describe("TerminalView", () => {
  it("draws what the program wrote, in the cells it was given", async () => {
    const { f, r } = mount();
    f.feed({ kind: "state", state: f.state() });
    f.out("\x1b[1;31mhello\x1b[0m\r\nsecond line");
    await drawn(r, (x) => x.includes("second line"));
    const frame = r.lastFrame()!.split("\n");
    expect(frame[0]).toContain("hello");
    expect(frame[1]).toContain("second line");
    expect(frame).toHaveLength(6);
    r.unmount();
  });

  it("sizes the program's terminal to the surface when it attaches", () => {
    const { f, r } = mount();
    expect(f.sent.resize[0]).toEqual([40, 6]);
    r.rerender(<TerminalView open={() => f.link} openKey="t" columns={30} rows={4} focused />);
    expect(f.sent.resize.at(-1)).toEqual([30, 4]);
    r.unmount();
  });

  it("while focused every read goes to the program whole; Ctrl+] gives the keys back", async () => {
    const { f, bus, left, r } = mount();
    f.feed({ kind: "state", state: f.state() });
    bus.emitRaw("ls\r");
    bus.emitRaw("\x1b[15~");
    expect(f.sent.input).toEqual(["ls\r", "\x1b[15~"]);
    bus.emitRaw("\x1d");
    expect(left).toEqual(["chord"]);
    r.unmount();
  });

  it("not focused: the surface does not hold the keys", () => {
    const { f, bus, r } = mount({ focused: false });
    f.feed({ kind: "state", state: f.state() });
    expect(bus.emitRaw("x")).toBe(false);
    r.unmount();
  });

  it("a program that exited shows how, keeps its last lines, and r restarts it", async () => {
    const { f, bus, exited, r } = mount();
    f.feed({ kind: "state", state: f.state() });
    f.out("one\r\ntwo");
    f.feed({ kind: "state", state: f.state({ state: "exited", code: 3 }) });
    await drawn(r, (x) => x.includes("exited with status 3"));
    const frame = r.lastFrame()!;
    expect(frame).toContain("exited with status 3");
    expect(frame).toContain("restart");
    expect(frame).toContain("one");
    expect(frame.split("\n")).toHaveLength(6);
    expect(exited.map((s) => s.code)).toEqual([3]);
    bus.emitRaw("x");
    expect(f.sent.input).toEqual([]);
    bus.emitRaw("r");
    expect(f.sent.restart).toBe(1);
    r.unmount();
  });

  it("a new generation clears the screen", async () => {
    const { f, r } = mount();
    f.feed({ kind: "state", state: f.state() });
    f.out("old");
    await drawn(r, (x) => x.includes("old"));
    f.feed({ kind: "state", state: f.state({ generation: 2 }) });
    f.out("new");
    await drawn(r, (x) => x.includes("new"));
    expect(r.lastFrame()).toContain("new");
    expect(r.lastFrame()).not.toContain("old");
    r.unmount();
  });

  it("a program that could not start says why", async () => {
    const { f, r } = mount();
    f.feed({ kind: "state", state: f.state({ state: "failed", message: "could not start nope" }) });
    await drawn(r, (x) => x.includes("could not start"));
    expect(r.lastFrame()).toContain("could not start: could not start nope");
    r.unmount();
  });

  it("a lost connection is shown, with no restart offered", async () => {
    const { f, r } = mount();
    f.feed({ kind: "state", state: f.state() });
    f.lose("closed");
    await drawn(r, (x) => x.includes("connection lost"));
    expect(r.lastFrame()).toContain("connection lost (closed)");
    expect(r.lastFrame()).not.toContain("restart");
    r.unmount();
  });

  it("closes the link when it goes", () => {
    const { f, r } = mount();
    r.unmount();
    expect(f.sent.closed).toBe(1);
  });
});
