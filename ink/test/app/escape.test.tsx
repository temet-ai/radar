import { afterEach, describe, expect, it } from "vitest";
import { mount, tick, until } from "./harness.js";

let live: Array<() => void> = [];
afterEach(() => {
  live.forEach((f) => f());
  live = [];
});

const rows = ["!1", "!2", "!3"].map((t) => ({ target: `row:${t}`, text: [{ text: t }] }));
const screen = {
  id: "mrs",
  title: "mrs",
  panes: [{ pane: "middle", title: "mrs", body: { node: "list", selected: 0, rows } }],
  actions: [],
};

async function ready() {
  const m = mount(80, 24);
  live.push(m.stop);
  m.welcome();
  m.snapshot({ screen }, 1);
  await tick(50);
  return m;
}
const intents = (m: Awaited<ReturnType<typeof ready>>) => m.intents().map((i) => i.intent);

describe("a lone Esc", () => {
  it("is sent at once, not after a wait for the rest of a sequence", async () => {
    const m = await ready();
    m.type("\x1b");
    await tick(5);
    expect(intents(m)).toEqual([{ intent: "key", chord: { key: "esc" } }]);
  });

  it("is Esc, and the letter that follows in its own read is that letter (not Alt-letter)", async () => {
    const m = await ready();
    m.type("\x1b");
    // The host answers the Esc with its state before the next key (a key can move its focus).
    m.snapshot({ screen }, 2);
    await tick(10);
    m.type("j");
    await until(() => m.intents().length >= 2);
    expect(intents(m)).toEqual([
      { intent: "key", chord: { key: "esc" } },
      { intent: "select", target: "row:!2" },
    ]);
  });

  it("twice in two reads is two Esc", async () => {
    const m = await ready();
    m.type("\x1b");
    m.type("\x1b");
    await until(() => m.intents().length >= 2);
    expect(intents(m)).toEqual([
      { intent: "key", chord: { key: "esc" } },
      { intent: "key", chord: { key: "esc" } },
    ]);
  });
});

describe("escape sequences in one read still parse", () => {
  it.each([
    ["Alt-x", "\x1bx", { key: { char: "x" }, alt: true }],
    ["Up", "\x1b[A", { key: "up" }],
    ["Down", "\x1b[B", { key: "down" }],
    ["Right", "\x1b[C", { key: "right" }],
    ["Left", "\x1b[D", { key: "left" }],
    ["PgUp", "\x1b[5~", { key: "page_up" }],
    ["PgDn", "\x1b[6~", { key: "page_down" }],
  ])("%s", async (_n, bytes, chord) => {
    const m = await ready();
    m.type(bytes);
    await until(() => m.intents().length >= 1);
    const first = intents(m)[0] as any;
    // A row movement is a Select on the list; the rest are keys, with the chord as the host reads it.
    if (first.intent === "key") expect(first.chord).toEqual(chord);
    else expect(first.intent).toBe("select");
    expect(intents(m).length).toBeLessThanOrEqual(1);
  });

  it("Esc, then an arrow sequence in the next read, is Esc and the arrow", async () => {
    const m = await ready();
    m.type("\x1b");
    m.snapshot({ screen }, 2);
    await tick(10);
    m.type("\x1b[B");
    await until(() => m.intents().length >= 2);
    expect(intents(m)[0]).toEqual({ intent: "key", chord: { key: "esc" } });
    expect((intents(m)[1] as any).intent).toBe("select");
  });
});
