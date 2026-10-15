import { afterEach, describe, expect, it } from "vitest";
import { mount, tick, until } from "./harness.js";
import { shellRenderNode } from "../../src/app/Body.js";

let live: Array<() => void> = [];
afterEach(() => {
  live.forEach((f) => f());
  live = [];
});

const rows = ["!1", "!2", "!3"].map((t) => ({ target: `row:${t}`, text: [{ text: t }] }));
const list = { node: "list", selected: 0, rows };
const tree = {
  node: "tree",
  selected: ["a"],
  open: ["a"],
  rows: [
    { name: [{ text: "alpha" }], children: [{ name: [{ text: "beta" }], children: [] }], path: ["a"] },
  ],
};
void tree;

describe("a right click on a row asks the host for that row's menu", () => {
  it.each([80, 160])("a list row at %i columns: Select is not sent, Context carries the row's own target", async (cols) => {
    const m = mount(cols, 24);
    live.push(m.stop);
    m.welcome();
    m.snapshot({ screen: { id: "mrs", title: "mrs", panes: [{ pane: "middle", title: "mrs", body: list }], actions: [] } }, 1);
    await tick(60);
    const lines = m.frame().split("\n");
    const y = lines.findIndex((l) => l.includes("!3"));
    const x = lines[y]!.indexOf("!3");
    m.type(`\x1b[<2;${x + 1};${y + 1}M\x1b[<2;${x + 1};${y + 1}m`);
    await until(() => m.intents().length >= 1);
    expect(m.intents().map((i) => i.intent)).toEqual([{ intent: "context", target: "row:!3" }]);
  });

  it("a right click where nothing is does nothing", async () => {
    const m = mount(80, 24);
    live.push(m.stop);
    m.welcome();
    m.snapshot({ screen: { id: "mrs", title: "mrs", panes: [{ pane: "middle", title: "mrs", body: list }], actions: [] } }, 1);
    await tick(60);
    m.type(`\x1b[<2;70;20M`);
    await tick(60);
    expect(m.intents()).toEqual([]);
  });
});

describe("the host's look message", () => {
  it("is applied: the next frame is drawn in the host's slots and icons, and the session counts it", async () => {
    const { slotStyle, defaultLook, applyLook } = await import("../../src/text/look.js");
    const m = mount(80, 24);
    live.push(m.stop);
    m.welcome();
    const before = slotStyle("accent.primary").color;
    m.conn.receive({ type: "look", slots: { "accent.primary": "#123456" }, modifiers: {}, icons: {}, icon_width: 0, pack: "off", tier: "truecolor" });
    try {
      await tick(20);
      expect(slotStyle("accent.primary").color).toBe("#123456");
      expect(slotStyle("accent.primary").color).not.toBe(before);
      expect(m.session.state.look).toBe(1);
    } finally {
      applyLook(defaultLook());
    }
  });
});

describe("a capturing editor over the body", () => {
  const editor = { title: "reply to !263", lines: ["first line"], line: "typed text", caret: 3 };
  const withEditor = {
    screen: {
      id: "mrs",
      title: "mrs",
      panes: [{ pane: "middle", title: "mrs", body: list }],
      actions: [],
      overlay: { pane: "middle", title: "reply", body: { node: "extension", capability: "editor", version: 1, data: editor, fallback: null } },
    },
  };
  it.each([80, 160])("is drawn at the foot of the body at %i columns and a press behind it sends nothing (GUIDE 1.3)", async (cols) => {
    const m = mount(cols, 24);
    live.push(m.stop);
    m.welcome();
    m.snapshot(withEditor, 1);
    await tick(80);
    expect(m.frame()).toContain("reply to !263");
    expect(m.frame()).toContain("typed text");
    const lines = m.frame().split("\n");
    const y = lines.findIndex((l) => l.includes("!2"));
    m.type(`\x1b[<0;${lines[y]!.indexOf("!2") + 1};${y + 1}M`);
    await tick(80);
    expect(m.intents()).toEqual([]);
  });
});
