import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3"; // Ink's colour library reads this when it loads: keep the cells' styles
});
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { freezeClock, mount, tick, until } from "./harness.js";
import { shellRenderNode } from "../../src/app/Body.js";
import { bypassKey } from "../../src/app/App.js";
import { paintSelection } from "../../src/app/selected.js";
import { frameRow } from "../../src/app/fast-output.js";
import { applyLook, currentLook, defaultLook, slotStyle } from "../../src/text/look.js";
import { parseAnsi, type TCell } from "../nodes/ansi.js";
import { STATES } from "../dock/states.js";
import chalk from "chalk";

// Selecting text with the mouse (issue #51) through the whole App: a drag selects inside the pane it started in, the
// selected cells are painted on bg.selection over the finished frame, letting go sends Intent::Copy with the text Ink
// drew there, and the keys and clicks after it extend, copy again or clear it.
let unfreeze = () => {};
beforeAll(() => {
  unfreeze = freezeClock();
});
afterAll(() => unfreeze());
let live: Array<() => void> = [];
afterEach(() => {
  live.forEach((f) => f());
  live = [];
  applyLook(defaultLook());
});

const SELECTION = slotStyle("bg.selection").color!.toLowerCase();
const dir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../fixtures");
const load = (f: string) => JSON.parse(fs.readFileSync(path.join(dir, f), "utf8"));
const cells = (frame: string): TCell[][] => parseAnsi(frame);
const plain = (frame: string) => cells(frame).map((r) => r.map((c) => c.ch).join(""));
/** The frame before any selection: a cell already on bg.selection there (the list's cursor row) is not the mouse's. */
let base = "";
const onSelection = (c: TCell | undefined) => c?.bg?.toLowerCase() === SELECTION;
const selectedIn = (frame: string, y: number) => {
  const was = cells(base)[y] ?? [];
  return (cells(frame)[y] ?? []).map((c, x) => (onSelection(c) && !onSelection(was[x]) ? x : -1)).filter((x) => x >= 0);
};

const at = (x: number, y: number) => `${x + 1};${y + 1}`;
const press = (x: number, y: number) => `\x1b[<0;${at(x, y)}M`;
const drag = (x: number, y: number) => `\x1b[<32;${at(x, y)}M`;
const release = (x: number, y: number) => `\x1b[<0;${at(x, y)}m`;
const click = (x: number, y: number) => press(x, y) + release(x, y);

const rows = ["!263 fix the build", "!264 docs", "!265 tests"].map((t) => ({ target: `row:${t.slice(0, 4)}`, text: [{ text: t }] }));
const doc = [
  "fix the build on main",
  "see src/app/App.tsx:120 for it",
  "and https://gitlab.com/acme/x/-/merge_requests/263 too",
];
const screen = {
  id: "mrs",
  title: "mrs",
  panes: [
    { pane: "middle", title: "merge requests", body: { node: "list", selected: 0, rows } },
    { pane: "detail", title: "detail", body: { node: "document", lines: doc.map((t) => [{ text: t }]) } },
  ],
  actions: [],
};

async function start(over: Record<string, unknown> = {}, cols = 160, h = 40) {
  let renders = 0;
  const m = mount(cols, h, { renderNode: (node: any, cx: any) => (renders++, shellRenderNode(node, cx)) });
  live.push(m.stop);
  m.welcome();
  m.snapshot({ screen, ...over }, 1);
  await tick(60);
  // the last whole frame written (a menu's field shows the cursor in a write of its own after it)
  const frame = () => [...m.out.frames].reverse().find((f) => f.includes("\n")) ?? "";
  base = frame();
  const copies = () => m.intents().map((i: any) => i.intent).filter((i: any) => i.intent === "copy").map((i: any) => i.text);
  const where = (text: string) => {
    const lines = plain(frame());
    const y = lines.findIndex((l) => l.includes(text));
    return { x: lines[y]!.indexOf(text), y };
  };
  return { m, copies, where, renders: () => renders };
}

describe("a drag selects inside its pane and copies on release", () => {
  /** The detail pane's inside: the columns between its borders. */
  const detailInside = (frame: string, y: number) => {
    const line = plain(frame)[y]!;
    const left = line.indexOf("││") + 2;
    return { left, right: line.lastIndexOf("│") - 1 };
  };

  it("a drag in the detail pane that runs past its left edge copies only the detail pane's text; the rows run to the pane's right edge, on bg.selection; no click is sent", async () => {
    const { m, copies, where, renders } = await start();
    const a = where("App.tsx");
    const first = where("fix the build on main");
    const { left, right } = detailInside(m.frame(), a.y);
    // from "App.tsx" up a row and out over the list pane to the screen's left edge
    m.type(press(a.x, a.y) + drag(a.x - 4, a.y) + drag(2, first.y));
    await until(() => selectedIn(m.frame(), first.y).length > 0);
    // painted over the frame: the first row from the pane's left edge to its right edge, the second to the head
    const range = (from: number, to: number) => Array.from({ length: to - from + 1 }, (_, i) => from + i);
    expect(selectedIn(m.frame(), first.y)).toEqual(range(left, right));
    expect(selectedIn(m.frame(), a.y)).toEqual(range(left, a.x));
    // while the drag selects, the status line names the terminal's own selection
    // (host.rs status_hints: the bypass key and "terminal's own selection" in place of every other key)
    await until(() => plain(m.frame()).at(-1)!.includes("own selection"));
    expect(plain(m.frame()).at(-1)!.trim()).toMatch(new RegExp(`^BOOKS +${bypassKey()} terminal's own selection +11 open`));
    expect(selectedIn(m.frame(), first.y)).toEqual(range(left, right)); // the hint's render keeps the selection painted
    // moving the head is a frame painted over the last one: no pane drawn again
    const before = renders();
    m.type(drag(1, first.y) + drag(0, first.y));
    await tick(40);
    expect(renders()).toBe(before);
    m.type(release(0, first.y));
    await until(() => copies().length > 0);
    expect(copies()).toEqual([" fix the build on main\n see src/app/A"]);
    expect(copies()[0]).not.toContain("!263");
    await until(() => !plain(m.frame()).at(-1)!.includes("own selection"));
    expect(plain(m.frame()).at(-1)).not.toContain("own selection");
    expect(plain(m.frame()).at(-1)).toContain("q quit");
    // the selection stays painted after the release; nothing but the copy reached the host
    expect(selectedIn(m.frame(), first.y)).toEqual(range(left, right));
    expect(m.intents().map((i: any) => i.intent.intent)).toEqual(["copy"]);
  });

  it("a press that slips one cell is still a click: the row is selected when it is let go, nothing is copied", async () => {
    const { m, copies, where } = await start();
    const r = where("!264 docs");
    m.type(press(r.x, r.y) + drag(r.x + 1, r.y));
    await tick(30);
    expect(m.intents()).toEqual([]); // held until let go (runner.rs Pointer::Up)
    m.type(release(r.x + 1, r.y));
    await until(() => m.intents().length > 0);
    expect(m.intents().map((i: any) => i.intent)).toEqual([{ intent: "select", target: "row:!264" }]);
    expect(copies()).toEqual([]);
  });

  it("a drag that starts on a pane's border or title is not a selection", async () => {
    const { m, copies, where } = await start();
    const a = where("App.tsx");
    const { left } = detailInside(m.frame(), a.y);
    m.type(press(left - 1, a.y) + drag(left + 10, a.y) + release(left + 10, a.y)); // the left border
    m.type(press(left + 3, a.y - 2) + drag(left + 10, a.y) + release(left + 10, a.y)); // the title row
    await tick(60);
    expect(copies()).toEqual([]);
    expect(selectedIn(m.frame(), a.y)).toEqual([]);
    expect(selectedIn(m.frame(), a.y - 2)).toEqual([]);
  });

  it("in text a double-click selects a word (a path whole), a triple-click the line; each is copied", async () => {
    const { m, copies, where } = await start();
    const a = where("App.tsx");
    m.type(click(a.x, a.y));
    m.type(click(a.x, a.y));
    await until(() => copies().length > 0);
    expect(copies()).toEqual(["src/app/App.tsx"]);
    const s = where("src/app");
    expect(selectedIn(m.frame(), a.y)).toEqual(Array.from({ length: "src/app/App.tsx".length }, (_, i) => s.x + i));
    m.type(click(a.x, a.y));
    await until(() => copies().length > 1);
    expect(copies()[1]).toBe(" see src/app/App.tsx:120 for it");
    // the first press was a click (it focuses the pane); the second and third were not
    expect(m.intents().map((i: any) => i.intent.intent)).toEqual(["select", "copy", "copy"]);
  });

  it("a double-click on a list row still selects and opens it (⏎), and copies nothing", async () => {
    const { m, copies, where } = await start();
    const r = where("!265 tests");
    m.type(click(r.x, r.y) + click(r.x, r.y));
    await until(() => m.intents().length >= 3);
    expect(m.intents().map((i: any) => i.intent)).toEqual([
      { intent: "select", target: "row:!265" },
      { intent: "select", target: "row:!265" },
      { intent: "activate", target: "row:!265" },
    ]);
    expect(copies()).toEqual([]);
  });

  it("with a selection showing, shift+→ and shift+↓ extend it inside the pane, ctrl+c copies it again, and any other key clears it and does what it does", async () => {
    const { m, copies, where } = await start();
    const f = where("fix the build on main");
    m.type(press(f.x, f.y) + drag(f.x + 2, f.y) + release(f.x + 2, f.y));
    await until(() => copies().length > 0);
    expect(copies()).toEqual(["fix"]);
    m.type("\x1b[1;2C");
    await until(() => selectedIn(m.frame(), f.y).length === 4);
    m.type("\x1b[1;2B");
    await until(() => selectedIn(m.frame(), f.y + 1).length > 0);
    expect(m.intents().length).toBe(1); // extending sends nothing
    m.type("\x03");
    await until(() => copies().length > 1);
    expect(copies()[1]).toBe("fix the build on main\n see");
    const sent = m.intents().length;
    m.type("j");
    await until(() => selectedIn(m.frame(), f.y).length === 0);
    expect(selectedIn(m.frame(), f.y + 1)).toEqual([]);
    await until(() => m.intents().length > sent);
    expect(m.intents().slice(sent).map((i: any) => i.intent.intent)).not.toContain("copy");
    // with nothing selected ctrl+c is what it always was, not a copy
    m.type("\x03");
    await tick(40);
    expect(copies().length).toBe(2);
  });

  it("a click elsewhere clears the selection and acts as a click", async () => {
    const { m, copies, where } = await start();
    const f = where("fix the build on main");
    m.type(press(f.x, f.y) + drag(f.x + 6, f.y) + release(f.x + 6, f.y));
    await until(() => copies().length > 0);
    const r = where("!264 docs");
    m.type(click(r.x, r.y));
    await until(() => selectedIn(m.frame(), f.y).length === 0);
    await until(() => m.intents().length > 1);
    expect(m.intents().map((i: any) => i.intent).slice(1)).toEqual([{ intent: "select", target: "row:!264" }]);
  });
});

describe("over and around a selection", () => {
  it("the wheel ends a selection showing, and scrolls as before", async () => {
    const { m, copies, where } = await start();
    const f = where("fix the build on main");
    m.type(press(f.x, f.y) + drag(f.x + 6, f.y) + release(f.x + 6, f.y));
    await until(() => copies().length > 0);
    expect(selectedIn(m.frame(), f.y).length).toBe(7);
    m.type(`\x1b[<65;${at(f.x, f.y)}M`);
    await until(() => selectedIn(m.frame(), f.y).length === 0);
    expect(selectedIn(m.frame(), f.y)).toEqual([]);
  });

  it("with a menu up only its inside is selected (nothing behind the scrim, shell.rs); a press behind still closes it", async () => {
    const menu = { id: "m", title: "Actions", question: false, items: [{ value: "a", label: "open in browser", hint: "", key: "o", target: "chrome:menu:0" }, { value: "b", label: "copy link", hint: "", key: "c", target: "chrome:menu:1" }], selected: 0 };
    const { m, copies, where } = await start({ chrome: { ...(await import("../chrome/fixture.js")).booksChrome({ width: 160, height: 40 }), menu } });
    const item = where("open in browser");
    m.type(press(item.x, item.y) + drag(item.x + 6, item.y + 1) + release(item.x + 6, item.y + 1));
    await until(() => copies().length > 0);
    expect(copies()[0]).toMatch(/^open in browser\n.*copy l/);
    expect(copies()[0]).not.toMatch(/[│╭╮]/);
    const sent = m.intents().length;
    const f = where("!264 docs"); // the list behind the scrim
    m.type(press(f.x, f.y) + drag(f.x + 6, f.y) + release(f.x + 6, f.y));
    await until(() => m.intents().length > sent);
    expect(m.intents().slice(sent).map((i: any) => i.intent)).toEqual([{ intent: "key", chord: { key: "esc" } }]);
    expect(copies().length).toBe(1);
  });
});

describe("the dock's transcript", () => {
  async function dock() {
    const r = await start({ chrome: load("host_chrome_160x40.json"), screen: { ...load("gitlab_repos_160x40.json"), dock: STATES.unfocused } });
    return r;
  }

  it("a drag over the transcript that runs past the dock's right border and down copies the transcript's text without the frame", async () => {
    const { m, copies, where } = await dock();
    const q = where("Why does the build fail");
    const line = plain(m.frame())[q.y]!;
    const border = line.lastIndexOf("│", q.x);
    m.type(press(border + 1, q.y) + drag(159, q.y + 2) + release(159, q.y + 2));
    await until(() => copies().length > 0);
    const text = copies()[0]!;
    expect(text).toContain("Why does the build fail on main?");
    expect(text).not.toMatch(/[│╭╮╰╯]/);
    expect(text.split("\n").length).toBe(3);
  });

  it("a drag that starts on the dock's border selects nothing", async () => {
    const { m, copies, where } = await dock();
    const q = where("Why does the build fail");
    const border = plain(m.frame())[q.y]!.lastIndexOf("│", q.x);
    m.type(press(border, q.y) + drag(border + 20, q.y) + release(border + 20, q.y));
    await tick(60);
    expect(copies()).toEqual([]);
  });
});

describe("the paint and the frame it reads", () => {
  it("selected cells take bg.selection and keep their foreground; reversed in the mono tier; only the selection's cells", () => {
    const frame = [chalk.hex("#ffffff")("abcdef"), "ghijkl"].join("\n");
    const area = { x: 0, y: 0, w: 6, h: 2, id: "a", kind: "text" as const };
    const out = cells(paintSelection(frame, 6, { area, anchor: { x: 2, y: 0 }, head: { x: 1, y: 1 } }));
    expect(out[0]!.map((c) => c.bg?.toLowerCase() === SELECTION)).toEqual([false, false, true, true, true, true]);
    expect(out[0]![2]!.fg).toBe("#ffffff");
    expect(out[1]!.slice(0, 3).map((c) => c.bg?.toLowerCase() === SELECTION)).toEqual([true, true, false]);
    const l = currentLook();
    applyLook({ ...l, slots: Object.fromEntries(Object.keys(l.slots).map((k) => [k, ""])) });
    const mono = cells(paintSelection("abc", 3, { area, anchor: { x: 0, y: 0 }, head: { x: 1, y: 0 } }));
    expect(mono[0]!.map((c) => !!c.inverse)).toEqual([true, true, false]);
  });

  it("a frame's row reads back as its cells, a wide character's second cell empty", () => {
    expect(frameRow("ab\n" + chalk.red("界x"), 5, 1)).toEqual(["界", "", "x", " ", " "]);
    expect(frameRow("ab", 3, 4)).toEqual([" ", " ", " "]);
  });
});
