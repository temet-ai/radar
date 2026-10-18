import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3"; // the pane frames' colours say which has the focus
});
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { freezeClock, mount, tick, until } from "../app/harness.js";
import { parseAnsi } from "../nodes/ansi.js";
import { STATES } from "./states.js";
import type { DockView } from "../../src/dock/model.js";

// Live, the keys and the wheel were confused between the panes and the dock under the middle pane: ↓ in the composer
// moved the tree, the wheel over a pane typed j and k into the composer, the wheel over the detail moved the middle
// pane. The rule: the wheel scrolls what is under the pointer and nothing else; keys scroll what has the focus and
// nothing else; the transcript follows new output only while it is at its foot.
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

const dir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../fixtures");
const load = (f: string) => JSON.parse(fs.readFileSync(path.join(dir, f), "utf8"));
const plain = (frame: string) => parseAnsi(frame).map((row) => row.map((c) => c.ch).join("").trimEnd());

const notices = (n: number) => Array.from({ length: n }, (_, i) => ({ item: "notice" as const, id: `n${i}`, text: `line ${i}` }));
const many = (n = 40, over: Partial<DockView> = {}): DockView => ({ ...STATES.empty!, composer: { ...STATES.empty!.composer!, history: [] }, transcript: notices(n), ...over });

const KEY = { down: "\x1b[B", up: "\x1b[A", pageDown: "\x1b[6~", pageUp: "\x1b[5~", home: "\x1b[H", end: "\x1b[F" };
/** SGR wheel at a 0-based cell. */
const wheel = (up: boolean, x: number, y: number) => `\x1b[<${up ? 64 : 65};${x + 1};${y + 1}M`;
const press = (x: number, y: number) => `\x1b[<0;${x + 1};${y + 1}M\x1b[<0;${x + 1};${y + 1}m`;

async function open(dock: DockView, cols = 160, rows = 40, screen = `gitlab_repos_${cols}x${rows}.json`) {
  const m = mount(cols, rows);
  live.push(m.stop);
  m.welcome();
  const shown = { chrome: load(`host_chrome_${cols}x${rows}.json`), screen: { ...load(screen), dock } };
  m.snapshot(shown, 1);
  await tick(40);
  const all = () => m.intents().map((i) => i.intent);
  const docks = () => all().filter((i) => i.intent === "dock");
  const others = () => all().filter((i) => i.intent !== "dock");
  // Modern hosts receive presentation metadata once per physical key, even when the dock consumes that key.
  // Keep it in `others`: exact assertions below must still catch duplicate notices or a key escaping its owner.
  const noticeInput = (escape = false) => ({ intent: "notice_input", escape, through_id: shown.chrome.notices.through_id });
  const text = () => plain(m.frame());
  return { m, all, docks, others, noticeInput, text, shown };
}

/** Where things are on a 160-wide gitlab_repos frame: the columns the panes start at, the dock's title row. */
function layout(rows: string[]) {
  const top = rows.findIndex((r) => r.startsWith("╭"));
  const corners = [...rows[top]!.matchAll(/╭/g)].map((m) => m.index!);
  const dockTop = rows.findIndex((r) => r.includes("╭ agents"));
  return { top, nav: corners[0]!, middle: corners[1]!, detail: corners[2]!, dockTop };
}

const composerLine = (rows: string[]) => rows.find((r) => /│> /.test(r)) ?? "";

describe("keys go to what has the focus, and nothing else", () => {
  it("an empty modern notice state still receives one watermark per physical key without taking dock ownership", async () => {
    const o = await open(many());
    const notices = { ...o.shown.chrome.notices, active: [], hidden: 0, revision: o.shown.chrome.notices.revision + 1 };
    o.m.snapshot({ ...o.shown, chrome: { ...o.shown.chrome, notices } }, 2);
    await tick(30);
    expect(o.m.session.noticeStore.active()).toEqual([]);
    o.m.type(KEY.down);
    await until(() => o.all().length >= 1);
    expect(o.all()).toEqual([o.noticeInput()]);
    o.m.type("`x");
    await until(() => o.all().length >= 5);
    expect(o.all()).toEqual([
      o.noticeInput(),
      o.noticeInput(), { intent: "dock", dock: "hide" },
      o.noticeInput(), { intent: "key", chord: { key: { char: "x" } } },
    ]);
    expect(o.m.session.noticeStore.active()).toEqual([]);
  });

  it("with the dock focused, ↑ ↓ PgUp PgDn Home End move nothing outside the dock", async () => {
    const o = await open(many());
    for (const k of [KEY.down, KEY.up, KEY.pageUp, KEY.pageDown, KEY.home, KEY.end, KEY.down, KEY.down]) {
      o.m.type(k);
      await tick(15);
    }
    await tick(60);
    expect(o.all()).toEqual(Array.from({ length: 8 }, () => o.noticeInput()));
  });

  it("↓ past the end of the sent history stays in the composer", async () => {
    const o = await open({ ...many(), composer: { ...STATES.empty!.composer!, history: ["hello"] } });
    o.m.type(KEY.up);
    await until(() => composerLine(o.text()).includes("> hello"));
    o.m.type(KEY.down);
    await tick(30);
    o.m.type(KEY.down);
    await tick(60);
    expect(o.all()).toEqual(Array.from({ length: 3 }, () => o.noticeInput()));
  });

  it("with the dock unfocused, ↓ moves the focused pane and PgUp leaves the transcript where it is", async () => {
    const o = await open(many(40, { focused: false }));
    const dockRows = () => o.text().filter((r) => r.includes("line "));
    const before = dockRows();
    o.m.type(KEY.down);
    await until(() => o.others().length >= 2);
    expect(o.others()).toEqual([o.noticeInput(), expect.objectContaining({ intent: "select" })]);
    o.m.type(KEY.pageUp);
    await tick(80);
    expect(dockRows()).toEqual(before);
    expect(o.docks()).toEqual([]);
    expect(o.all()).toEqual([
      o.noticeInput(), expect.objectContaining({ intent: "select" }),
      o.noticeInput(), { intent: "key", chord: { key: "page_up" } },
    ]);
  });

  it("keys after an esc in the same read go to the pane, not back into the dock", async () => {
    const o = await open(many());
    o.m.type("\x1b");
    await until(() => o.docks().length > 0, 1000);
    expect(o.docks()).toEqual([{ intent: "dock", dock: "blur" }]);
    // The host has not answered yet: Ink already knows the dock gave the keys back.
    o.m.type("x");
    await until(() => o.all().length >= 4, 1000);
    expect(o.all()).toEqual([
      o.noticeInput(true), { intent: "dock", dock: "blur" },
      o.noticeInput(), { intent: "key", chord: { key: { char: "x" } } },
    ]);
    expect(composerLine(o.text())).not.toContain("x");
  });

  it("no pane draws focused while the dock has the keys", async () => {
    const border = (rows: ReturnType<typeof parseAnsi>, x: number, y: number) => JSON.stringify({ ...rows[y]![x], ch: "" });
    for (const focused of [true, false]) {
      const o = await open(many(40, { focused }));
      const cells = parseAnsi(o.m.frame());
      const at = layout(o.text());
      const same = border(cells, at.middle, at.top) === border(cells, at.nav, at.top);
      expect(same, `middle pane drawn as unfocused while the dock ${focused ? "has" : "has not"} the keys`).toBe(focused);
    }
  });

  it("on an item page the dock is not drawn, so it takes no keys there", async () => {
    const o = await open(many(), 160, 40, "gitlab_repos-page2-overview-changes-threads-pipeline-commits-ticket_160x40.json");
    expect(o.text().join("\n")).not.toContain("agents");
    o.m.type("x");
    await until(() => o.others().length >= 2, 1000);
    expect(o.others()).toEqual([o.noticeInput(), { intent: "key", chord: { key: { char: "x" } } }]);
    expect(o.docks()).toEqual([]);
  });
});

describe("the wheel scrolls what is under the pointer, and nothing else", () => {
  it("over the middle pane with the dock focused it moves the tree and types nothing into the composer", async () => {
    const o = await open(many());
    const at = layout(o.text());
    o.m.type(wheel(false, at.middle + 10, at.top + 3));
    await until(() => o.others().length > 0);
    expect(o.others()[0]).toMatchObject({ intent: "select" });
    expect(String((o.others()[0] as any).target)).toMatch(/^t:/);
    await tick(40);
    expect(composerLine(o.text())).toMatch(/│>\s*│/);
    expect(o.docks()).toEqual([]);
  });

  it("over the detail pane it focuses the detail and leaves the middle pane's selection alone", async () => {
    for (const focused of [true, false]) {
      const o = await open(many(40, { focused }));
      const at = layout(o.text());
      o.m.type(wheel(false, at.detail + 10, at.top + 10));
      await until(() => o.others().length > 0);
      await tick(40);
      expect(o.others()[0]).toEqual({ intent: "select", target: "pane:detail" });
      expect(o.others().filter((i: any) => i.intent === "select" && String(i.target).startsWith("t:"))).toEqual([]);
      expect(o.others().filter((i: any) => i.intent === "key")).toEqual([]);
      expect(composerLine(o.text())).toMatch(/│>\s*│/);
    }
  });

  it("over the nav pane it focuses the nav and moves its rows", async () => {
    const o = await open(many(40, { focused: false }));
    const at = layout(o.text());
    o.m.type(wheel(false, at.nav + 5, at.top + 2));
    await until(() => o.others().length > 1);
    expect(o.others()[0]).toEqual({ intent: "select", target: "pane:nav" });
    expect(o.others()[1]).toMatchObject({ intent: "select", target: "view:1" });
  });

  it("over the dock it scrolls the transcript whether or not the dock has the keys, and moves no focus", async () => {
    for (const focused of [true, false]) {
      const o = await open(many(40, { focused }));
      const at = layout(o.text());
      expect(o.text().join("\n")).toContain("line 39");
      o.m.type(wheel(true, at.middle + 20, at.dockTop + 4));
      await until(() => !o.text().join("\n").includes("line 39"));
      expect(o.text().join("\n")).toContain("line 36");
      await tick(40);
      expect(o.all()).toEqual([]);
    }
  });

  it("over the dock's top edge it scrolls the transcript, not the pane above", async () => {
    const o = await open(many());
    const at = layout(o.text());
    o.m.type(wheel(true, at.middle + 20, at.dockTop));
    await until(() => !o.text().join("\n").includes("line 39"));
    await tick(40);
    expect(o.all()).toEqual([]);
  });

  it("after the dock grows, the row above its new top is the pane's and its top row is the dock's", async () => {
    const o = await open(many(40, { focused: false }));
    const was = layout(o.text()).dockTop;
    o.m.snapshot({ ...o.shown, screen: { ...o.shown.screen, dock: { ...o.shown.screen.dock, rows: 24 } } }, 2);
    await until(() => layout(o.text()).dockTop < was - 4);
    const at = layout(o.text());
    o.m.type(wheel(true, at.middle + 20, at.dockTop + 3));
    await until(() => !o.text().join("\n").includes("line 39"));
    await tick(40);
    expect(o.all()).toEqual([]);
    o.m.type(wheel(false, at.middle + 20, at.dockTop - 1));
    await until(() => o.others().length > 0);
    expect(o.others()[0]).toMatchObject({ intent: "select" });
    expect(String((o.others()[0] as any).target)).toMatch(/^t:/);
  });

  it("on a narrow terminal the dock covers the middle pane and the wheel over it scrolls only the dock", async () => {
    const o = await open(many(), 80, 24);
    const rows = o.text();
    const top = rows.findIndex((r) => r.includes("╭ agents"));
    expect(top).toBeGreaterThan(0);
    expect(o.text().join("\n")).toContain("line 39");
    o.m.type(wheel(true, 40, top + 4));
    await until(() => !o.text().join("\n").includes("line 39"));
    await tick(40);
    expect(o.all()).toEqual([]);
  });

  it("on settings, the wheel over the dock beside the section scrolls it; over the detail it focuses and moves the detail", async () => {
    const m = mount(160, 40);
    live.push(m.stop);
    m.welcome();
    m.snapshot({ chrome: load("host_chrome_160x40.json"), section: load("git_settings-branches_160x40.json"), dock: many(40, { focused: false }) }, 1);
    await tick(40);
    const rows = plain(m.frame());
    const dockTop = rows.findIndex((r) => r.includes("╭ agents"));
    const x = rows[dockTop]!.indexOf("╭ agents") + 10;
    expect(rows.join("\n")).toContain("line 39");
    m.type(wheel(true, x, dockTop + 4));
    await until(() => !plain(m.frame()).join("\n").includes("line 39"));
    await tick(40);
    expect(m.intents()).toEqual([]);
    // The detail: the last frame on the top row of the body.
    const top = rows.findIndex((r) => r.includes("╭"));
    const detail = rows[top]!.lastIndexOf("╭");
    m.type(wheel(false, detail + 5, top + 3));
    await until(() => m.intents().length > 1);
    expect(m.intents().map((i) => i.intent)).toEqual([
      { intent: "select", target: "pane:detail" },
      { intent: "key", chord: { key: { char: "j" } } },
    ]);
  });
});

describe("a press outside the dock", () => {
  it("on a pane's empty space focuses that pane, after taking the keys from the dock", async () => {
    const o = await open(many());
    const at = layout(o.text());
    o.m.type(press(at.detail + 10, at.top + 30));
    await until(() => o.all().length > 1);
    expect(o.all()).toEqual([
      { intent: "dock", dock: "blur" },
      { intent: "select", target: "pane:detail" },
    ]);
  });

  it("on a row takes the keys from the dock and selects the row", async () => {
    const o = await open(many());
    const rows = o.text();
    const y = rows.findIndex((r) => r.includes("team-a"));
    const x = rows[y]!.indexOf("team-a");
    o.m.type(press(x, y));
    await until(() => o.all().length > 1);
    expect(o.all()[0]).toEqual({ intent: "dock", dock: "blur" });
    expect(o.all()[1]).toMatchObject({ intent: "select" });
  });
});

describe("the transcript follows new output only at its foot", () => {
  it("scrolled up, new rows keep the view where it was; at the foot, it follows them", async () => {
    const o = await open(many(40, { focused: false }));
    const at = layout(o.text());
    o.m.type(wheel(true, at.middle + 20, at.dockTop + 4));
    await until(() => !o.text().join("\n").includes("line 39"));
    const shown = () => o.text().filter((r) => r.includes("line ")).map((r) => /line \d+/.exec(r)![0]);
    const before = shown();
    o.m.snapshot({ ...o.shown, screen: { ...o.shown.screen, dock: many(45, { focused: false }) } }, 2);
    await until(() => true);
    await tick(60);
    expect(shown()).toEqual(before);
    // Back to the foot: the newest rows show, and the next ones follow.
    for (let i = 0; i < 4; i++) o.m.type(wheel(false, at.middle + 20, at.dockTop + 4));
    await until(() => o.text().join("\n").includes("line 44"));
    o.m.snapshot({ ...o.shown, screen: { ...o.shown.screen, dock: many(48, { focused: false }) } }, 3);
    await until(() => o.text().join("\n").includes("line 47"));
    expect(o.text().join("\n")).toContain("line 47");
  });
});
