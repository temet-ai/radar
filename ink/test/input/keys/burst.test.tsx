import { afterEach, describe, expect, it } from "vitest";
import { chromeOf, mount, tick, until, unmountAll } from "../../contract/support.js";

/**
 * Class: keys that arrive in one read go where the same keys typed one by one go. A key the renderer cannot
 * predict (→ moves the host's focus from the nav pane to the middle pane) leaves the keys after it unroutable from
 * the old snapshot: routed there, ↓ ↓ ↓ ⏎ selected and opened rows of the NAV pane (the burst
 * "Right Down Down Down Enter Enter Enter" landed on !264 where Ratatui lands on !263). After such a key the rest
 * of the read goes to the host as plain keys, which it applies to its own state in order.
 */
afterEach(unmountAll);

const pane = (name: string, prefix: string) => ({
  pane: name,
  title: name,
  body: { node: "list", selected: 0, rows: ["a", "b", "c", "d"].map((s) => ({ target: `${prefix}:${s}`, text: [{ text: s }] })) },
});

describe("a burst of keys after a focus-moving key", () => {
  for (const [w, h] of [[80, 24], [160, 40]] as const) {
    it(`sends the keys after → as plain keys, not rows of the old pane (${w}x${h})`, async () => {
      const m = mount(w, h);
      m.snapshot({
        chrome: chromeOf(w, h),
        screen: { id: "s", scope: "s", panes: [pane("nav", "nav"), pane("middle", "mid")], actions: [] },
        focus: "nav",
      });
      await m.settle();
      m.type("\x1b[C\x1b[B\x1b[B\x1b[B\r\r\r");
      await until(() => m.intents().length >= 7, 500);
      await tick(10);
      const sent = m.intents();
      expect(sent.map((i: any) => i.intent)).toEqual(Array(7).fill("key"));
      expect(sent.map((i: any) => i.chord.key)).toEqual(["right", "down", "down", "down", "enter", "enter", "enter"]);
    });
  }

  it("still predicts a run of ↓ from a snapshot (no unpredictable key before it)", async () => {
    const m = mount(80, 24);
    m.snapshot({ chrome: chromeOf(80, 24), screen: { id: "s", scope: "s", panes: [pane("middle", "mid")], actions: [] }, focus: "middle" });
    await m.settle();
    m.type("\x1b[B\x1b[B\r");
    await until(() => m.intents().length >= 3, 500);
    expect(m.intents()).toEqual([
      { intent: "select", target: "mid:b" },
      { intent: "select", target: "mid:c" },
      { intent: "activate", target: "mid:c" },
    ]);
  });
});
