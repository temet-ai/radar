import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3"; // Ink's colour library reads this when it loads: the lit row is a background
});
import { freezeClock, mount, tick } from "../../app/harness.js";
import { booksChrome, actionsMenu } from "../../chrome/fixture.js";

/**
 * A menu the host asked (ToRenderer::Question, sent once) and the same menu in every snapshot's chrome (moving
 * through it shows only there: bridge.rs ask_and_tell). The cursor the host moved must be the one drawn, and
 * Enter runs the lit item. Found live (T22): Down and j left the first item lit.
 */
let unfreeze = () => {};
beforeAll(() => {
  unfreeze = freezeClock();
});
afterAll(() => unfreeze());
let live: Array<() => void> = [];
afterEach(() => {
  live.forEach((f) => f());
  live = [];
});

const menu = (selected: number) => ({
  ...actionsMenu,
  items: [
    { value: "a", label: "alpha", hint: "", key: "1", target: "chrome:menu:0" },
    { value: "b", label: "bravo", hint: "", key: "2", target: "chrome:menu:1" },
    { value: "c", label: "charlie", hint: "", key: "3", target: "chrome:menu:2" },
  ],
  selected,
});

/** Opens the menu as the host does: a snapshot whose chrome holds it, and the question for it. */
function open(asQuestion: boolean) {
  const m = mount(100, 30);
  live.push(m.stop);
  m.welcome();
  const snap = (selected: number, cursor: number) => m.snapshot({ chrome: { ...booksChrome({ width: 100, height: 30 }), menu: menu(selected) } }, cursor);
  snap(0, 1);
  if (asQuestion) m.conn.receive({ type: "question", id: 1, cursor: 1, menu: menu(0) });
  return { m, snap };
}

/** The label of the row drawn on the selection background (the hint at the row's end is always on it). */
function lit(frame: string): string[] {
  const rows = frame.split("\n").filter((l) => /alpha|bravo|charlie/.test(l));
  const found: string[] = [];
  for (const l of rows) {
    const word = /alpha|bravo|charlie/.exec(l)![0];
    const before = l.slice(0, l.indexOf(word));
    const bg = [...before.matchAll(/\x1b\[([0-9;]*)m/g)].map((x) => x[1]).filter((c) => /(^|;)48;/.test(c)).pop();
    if (bg === "48;2;68;71;90" || /48;2;68;71;90/.test(bg ?? "")) found.push(word);
  }
  return found;
}

describe.each([
  ["a menu", false],
  ["a context menu the host asked", true],
] as const)("%s", (_name, asQuestion) => {
  it.each([
    ["down", "\x1b[B", 1],
    ["j", "j", 1],
  ])("%s moves the lit item to the one the host moved to", async (_k, bytes, to) => {
    const { m, snap } = open(asQuestion);
    await tick();
    expect(lit(m.frame())).toEqual(["alpha"]);
    m.type(bytes);
    await tick();
    snap(to, 2); // the host moved its cursor and says so in its snapshot
    await tick();
    expect(lit(m.frame())).toEqual(["bravo"]);
  });

  it.each([
    ["up", "\x1b[A"],
    ["k", "k"],
  ])("%s moves the lit item up", async (_k, bytes) => {
    const { m, snap } = open(asQuestion);
    snap(2, 2);
    await tick();
    m.type(bytes);
    await tick();
    snap(1, 3);
    await tick();
    expect(lit(m.frame())).toEqual(["bravo"]);
  });

  it("Enter runs the lit item after Down", async () => {
    const { m, snap } = open(asQuestion);
    await tick();
    m.type("\x1b[B");
    await tick();
    snap(1, 2);
    await tick();
    m.type("\r");
    await tick();
    expect(m.intents().map((i) => i.intent)).toContainEqual({ intent: "invoke", action: "b" });
  });

  it("Enter runs the lit item after two keys read together, before any snapshot", async () => {
    const { m } = open(asQuestion);
    await tick();
    m.type("\x1b[B\x1b[B\r");
    await tick();
    expect(m.intents().map((i) => i.intent)).toContainEqual({ intent: "invoke", action: "c" });
  });
});
