import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { mount, tick, until } from "../app/harness.js";
import { sheetKey, type ScrollerLike } from "../../src/input/route.js";
import { settledCursor } from "../../src/nodes/lists.js";
import { applyLook, currentLook, defaultLook } from "../../src/text/look.js";
import { SCENES } from "./scenes.js";

const plain = (s: string) => s.replace(/\x1b\[[0-9;?]*[A-Za-z]/g, "");
const PGDN = "\x1b[6~";
const PGUP = "\x1b[5~";

let live: Array<() => void> = [];
let before = currentLook();
beforeEach(() => {
  before = currentLook();
  applyLook(defaultLook("off"));
});
afterEach(() => {
  live.forEach((f) => f());
  live = [];
  applyLook(before);
});

describe("the cursor of a list (ui::rows::settle)", () => {
  const rows = (...h: boolean[]) => h.map((heading) => ({ heading }));
  it("rests where it is when that row can be selected, else on the next one that can, else the nearest before", () => {
    expect(settledCursor(rows(true, false, false), 2)).toBe(2);
    expect(settledCursor(rows(true, false, false), 0)).toBe(1);
    expect(settledCursor(rows(false, true, true), 2)).toBe(0);
    expect(settledCursor(rows(false, true, false, true), 3)).toBe(2);
  });
  it("stays on its row when no row can be selected, and past the end it is the last", () => {
    expect(settledCursor(rows(true, true), 0)).toBe(0);
    expect(settledCursor(rows(true, true), 9)).toBe(1);
    expect(settledCursor([], 0)).toBe(0);
  });
});

describe("PgUp and PgDn scroll the sheet of setup and of settings", () => {
  const scroller = (kind: "rich" | "document", moved: boolean, log: string[]): ScrollerLike => ({
    kind,
    focused: false,
    at: () => 0,
    max: () => 5,
    key: () => false,
    page: (down) => (log.push(`${kind}:${down ? "down" : "up"}`), moved),
  });
  const chord = (key: string) => ({ key }) as any;

  it("pages the last scroller of the sheet's kind and sends nothing when it moved", () => {
    const log: string[] = [];
    const s = [scroller("rich", true, log), scroller("document", true, log), scroller("rich", true, log)];
    expect(sheetKey({ detail_blocks: [{}] }, chord("page_down"), s)).toBe("done");
    expect(sheetKey({ detail_blocks: [] }, chord("page_up"), s)).toBe("done");
    expect(log).toEqual(["rich:down", "document:up"]);
  });

  it("sends the plain key when it cannot move (the host says there is nothing to scroll), never a row page", () => {
    const k = chord("page_down");
    expect(sheetKey({ detail_blocks: [{}] }, k, [scroller("rich", false, [])])).toEqual({ intent: "key", chord: k });
    expect(sheetKey({}, k, [])).toEqual({ intent: "key", chord: k });
  });

  it("is not another key's", () => {
    expect(sheetKey({}, chord("down"), [])).toBeUndefined();
    expect(sheetKey({}, { key: "page_down", ctrl: true } as any, [])).toBeUndefined();
  });

  it("in the app: PgDn on the 80x24 scan screen scrolls the sheet (its first section leaves) and PgUp brings it back", async () => {
    const scene = SCENES.find((s) => s.name === "scan_80x24")!;
    const m = mount(scene.width, scene.height);
    live.push(m.stop);
    m.welcome();
    m.snapshot({ chrome: scene.chrome, section: scene.section, screen: null, focus: "middle" }, 1);
    await tick(80);
    expect(plain(m.frame())).toContain("WHAT IT READS");
    m.type(PGDN);
    await until(() => !plain(m.frame()).includes("WHAT IT READS"));
    expect(plain(m.frame())).not.toContain("WHAT IT READS");
    // the key was the frontend's: nothing went to the host
    expect(m.intents()).toEqual([]);
    m.type(PGUP);
    await until(() => plain(m.frame()).includes("WHAT IT READS"));
    expect(plain(m.frame())).toContain("WHAT IT READS");
    expect(m.intents()).toEqual([]);
  });
});

describe("a click on setup's rows", () => {
  const click = (x: number, y: number) => `\x1b[<0;${x};${y}M\x1b[<0;${x};${y}m`;

  it("ticks a chooser's row each time (Activate), never selecting first and never a double click's second Activate", async () => {
    const scene = SCENES.find((s) => s.name === "choose_160x40")!;
    const m = mount(scene.width, scene.height);
    live.push(m.stop);
    m.welcome();
    m.snapshot({ chrome: scene.chrome, section: scene.section, screen: null, focus: "middle" }, 1);
    await tick(80);
    // "○ local code" is the card's first row, under the sheet (a blank row, 5 text rows), a blank row, the card's tab, the heading,
    // the lead, a blank row and THIS MACHINE: row 15, 1-based 16
    expect(plain(m.frame()).split("\n")[15]).toContain("local code");
    m.stdin.type(click(20, 16));
    await until(() => m.intents().length >= 1);
    m.stdin.type(click(20, 16));
    await until(() => m.intents().length >= 2);
    await tick(30);
    expect(m.intents().map((i) => i.intent)).toEqual([
      { intent: "activate", target: "setup:choose:0" },
      { intent: "activate", target: "setup:choose:0" },
    ]);
  });

  it("selects any other row of setup, and two clicks make it Activate (the step's ⏎)", async () => {
    const scene = SCENES.find((s) => s.name === "scan_80x24")!;
    const m = mount(scene.width, scene.height);
    live.push(m.stop);
    m.welcome();
    m.snapshot({ chrome: scene.chrome, section: scene.section, screen: null, focus: "middle" }, 1);
    await tick(80);
    expect(plain(m.frame()).split("\n")[21]).toContain("no thanks");
    m.stdin.type(click(8, 22));
    await until(() => m.intents().length >= 1);
    m.stdin.type(click(8, 22));
    await until(() => m.intents().length >= 3);
    expect(m.intents().map((i) => i.intent)).toEqual([
      { intent: "select", target: "setup:scan:1" },
      { intent: "select", target: "setup:scan:1" },
      { intent: "activate", target: "setup:scan:1" },
    ]);
  });
});
