import { afterEach, describe, expect, it } from "vitest";
import { chromeOf, mount, tick, until, unmountAll } from "../../contract/support.js";

/**
 * Class: keys that arrive in one read go where the same keys typed one by one go. A key the renderer cannot
 * predict (→ moves the host's focus from the nav pane to the middle pane) leaves the keys after it unroutable from
 * the old snapshot: routed there, ↓ ↓ ↓ ⏎ selected and opened rows of the NAV pane (the burst
 * "Right Down Down Down Enter Enter Enter" landed on !264 where the reference lands on !263). After such a key the rest
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

describe("a burst of shared tree keys", () => {
  const body = (open = [["watch"], ["watch", "cases"]], selected = ["watch", "cases", "patrol"]) => ({
    node: "tree", columns: [], open, selected,
    rows: [{ id: "watch", name: [{ text: "Watch" }], children: [
      { id: "cases", name: [{ text: "Cases" }], children: [{ id: "patrol", name: [{ text: "Patrol" }] }] },
      { id: "archive", name: [{ text: "Archive" }] },
    ] }],
  });
  const open = async (w: number, h: number, tree = body(), section = false) => {
    const m = mount(w, h);
    m.snapshot(section
      ? { chrome: { ...chromeOf(w, h), settings: { groups: [], section_title: "Watch", detail_title: "Watch" } }, section: { scope: "watch", body: tree, detail_title: "Watch", detail: [] }, focus: "middle" }
      : { chrome: chromeOf(w, h), screen: { id: "watch", scope: "watch", panes: [{ pane: "middle", title: "Watch", body: tree }], actions: [] }, focus: "middle" });
    await m.settle();
    return m;
  };

  for (const [w, h] of [[80, 24], [160, 40]] as const) {
    it(`collapses the selected descendant to its parent before repeating minus (${w}x${h})`, async () => {
      const m = await open(w, h);
      m.type("--\x1b[C\x1b[C\r");
      await until(() => m.intents().length >= 5, 500);
      expect(m.intents()).toEqual([
        { intent: "fold", target: "watch\u001fcases", open: false },
        { intent: "fold", target: "watch", open: false },
        { intent: "fold", target: "watch", open: true },
        { intent: "select", target: "watch\u001fcases" },
        { intent: "activate", target: "watch\u001fcases" },
      ]);
    });

    it(`defers buffered keys after opening every held group to the host (${w}x${h})`, async () => {
      const m = await open(w, h, body([], ["watch"]));
      m.type("=\x1b[C\x1b[C\r");
      await until(() => m.intents().length >= 4, 500);
      expect(m.intents()).toEqual([
        { intent: "open_all" },
        { intent: "key", chord: { key: "right" } },
        { intent: "key", chord: { key: "right" } },
        { intent: "key", chord: { key: "enter" } },
      ]);
    });
  }

  it("left selects the parent first and only the next left closes it", async () => {
    const m = await open(80, 24);
    m.type("\x1b[D\x1b[D");
    await until(() => m.intents().length >= 2, 500);
    expect(m.intents()).toEqual([
      { intent: "select", target: "watch\u001fcases" },
      { intent: "fold", target: "watch\u001fcases", open: false },
    ]);
  });

  it("preserves shifted arrow ownership instead of converting it to a tree fold", async () => {
    const m = await open(80, 24, body([], ["watch"]));
    m.type("\x1b[1;2C");
    await until(() => m.intents().length >= 1, 500);
    expect(m.intents()).toEqual([{ intent: "key", chord: { key: "right", shift: true } }]);
  });

  it("does not optimistically open an ambiguous ancestor before buffered right keys", async () => {
    const duplicate = body([], ["watch"]);
    duplicate.rows.push({ id: "watch", name: [{ text: "Duplicate Watch" }], children: [] });
    const m = await open(80, 24, duplicate);
    m.type("\x1b[C\x1b[C");
    await until(() => m.intents().length >= 2, 500);
    expect(m.intents()).toEqual([
      { intent: "fold", target: "watch", open: true },
      { intent: "fold", target: "watch", open: true },
    ]);
  });

  for (const level of ["legacy", "off", "subtle", "full"]) {
    it(`does not predict OpenAll on an anonymous item page with ${level} motion metadata`, async () => {
      const m = mount(80, 24);
      const motion = level === "legacy" ? undefined : {
        level, cues: [], timings: {},
        slots: { base: "bg.base", surface: "bg.surface", overlay: "bg.overlay", glow: "accent.primary", text: "text.primary" },
        ...(level === "full" ? { interaction: { context: null, processed_seq: null } } : {}),
      };
      m.conn.receive({
        type: "snapshot", cursor: 1, chrome: chromeOf(80, 24), focus: "middle", section: null,
        screen: { id: "watch", scope: "watch", panes: [], actions: [], page: {
          crumbs: [], title: [{ text: "Anonymous case" }], tabs: [], tab: 0,
          panes: [{ pane: "middle", title: "Watch", body: body([], ["watch"]) }],
        } },
        ...(motion ? { motion } : {}),
      });
      await m.settle();
      m.type("=\x1b[C");
      await until(() => m.intents().length >= 2, 500);
      expect(m.intents()).toEqual([
        { intent: "open_all" },
        { intent: "key", chord: { key: "right" } },
      ]);
    });
  }

  it("settings folds use the same optimistic ancestor selection", async () => {
    const m = await open(80, 24, body(), true);
    m.type("--");
    await until(() => m.intents().length >= 2, 500);
    expect(m.intents()).toEqual([
      { intent: "fold", target: "watch\u001fcases", open: false },
      { intent: "fold", target: "watch", open: false },
    ]);
  });

  for (const kind of ["list", "tree"]) {
    it(`sends raw editor keys when an inline ${kind} editor is in another pane`, async () => {
      const m = mount(80, 24);
      const editor = kind === "tree"
        ? { ...body([], ["watch"]), editing: { path: ["watch"], at: 0, text: "Watch", caret: 0 } }
        : { node: "list", rows: [{ target: "watch", text: [{ text: "Watch" }] }], selected: 0, editing: { row: 0, at: 0, text: "Watch", caret: 0 } };
      m.snapshot({
        chrome: chromeOf(80, 24), focus: "middle",
        screen: { id: "watch", scope: "watch", actions: [], panes: [
          { pane: "nav", title: "Edit", body: editor },
          { pane: "middle", title: "Watch", body: body([], ["watch"]) },
        ] },
      });
      await m.settle();
      m.type("=\x1b[C\x1b[B");
      await until(() => m.intents().length >= 3, 500);
      expect(m.intents()).toEqual([
        { intent: "key", chord: { key: { char: "=" } } },
        { intent: "key", chord: { key: "right" } },
        { intent: "key", chord: { key: "down" } },
      ]);
    });
  }
});
