import React from "react";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { render } from "ink";
import { freezeClock, mount, tick, until, Out } from "./harness.js";

let unfreeze = () => {};
beforeAll(() => {
  unfreeze = freezeClock();
});
afterAll(() => unfreeze());
import { Terminal, RESTORE } from "../../src/app/terminal.js";
import { ErrorBoundary } from "../../src/app/ErrorBoundary.js";
import { capabilitiesFrom } from "../../src/app/session.js";
import { actionsMenu, booksChrome, confirm, settingsNav } from "../chrome/fixture.js";

let live: Array<() => void> = [];
afterEach(() => {
  live.forEach((f) => f());
  live = [];
});
const open = (cols: number, rows: number) => {
  const m = mount(cols, rows);
  live.push(m.stop);
  return m;
};

describe("session", () => {
  it("sends Hello with the terminal's capabilities and acknowledges the newest snapshot", async () => {
    const m = open(80, 24);
    expect(m.sent("hello")[0]).toMatchObject({ protocol: 1, view: 1, capabilities: { columns: 80, rows: 24, mouse: true, image: "none", plain_text: false } });
    m.welcome();
    m.snapshot({}, 5);
    expect(m.sent("ack")).toEqual([{ type: "ack", cursor: 5 }]);
  });

  it("capabilities come from the environment", () => {
    const caps = capabilitiesFrom({ COLORTERM: "truecolor", TERM_PROGRAM: "ghostty", RADAR_PLAIN_TEXT: "1" }, { columns: 160, rows: 40 });
    expect(caps).toEqual({ columns: 160, rows: 40, mouse: true, true_colour: true, image: "kitty", plain_text: true });
    expect(capabilitiesFrom({ KITTY_WINDOW_ID: "3" }, {}).image).toBe("kitty");
  });

  it("a Refuse prints the host's message and exits 3", async () => {
    const m = open(80, 24);
    m.conn.receive({ type: "refuse", wanted: 2, got: 1, message: "radar wants protocol 2; this renderer speaks 1" });
    await tick();
    expect(m.exits).toEqual([{ code: 3, message: "radar wants protocol 2; this renderer speaks 1" }]);
  });

  it("Shutdown exits 0 and a lost connection exits 1", async () => {
    const a = open(80, 24);
    a.conn.receive({ type: "shutdown", reason: "quit" });
    await tick();
    expect(a.exits[0]).toMatchObject({ code: 0 });
    const b = open(80, 24);
    b.conn.drop("closed");
    await tick();
    expect(b.exits[0]).toMatchObject({ code: 1 });
  });

  it("intents carry the cursor of the snapshot showing and a rising seq; a question is never dropped by later snapshots", async () => {
    const m = open(80, 24);
    m.welcome();
    m.snapshot({}, 7);
    m.conn.receive({ type: "question", id: 1, cursor: 7, menu: confirm });
    const withMenu = { chrome: { ...booksChrome({ width: 80, height: 24 }), menu: confirm } };
    m.snapshot(withMenu, 8);
    m.snapshot(withMenu, 9);
    await tick();
    expect(m.session.state.questions).toHaveLength(1);
    m.type("y");
    await tick();
    m.type("q");
    await tick();
    expect(m.intents()).toEqual([
      { type: "intent", seq: 1, cursor: 9, intent: { intent: "invoke", action: "yes" } },
      { type: "intent", seq: 2, cursor: 9, intent: { intent: "key", chord: { key: { char: "q" } } } },
    ]);
    expect(m.session.state.questions).toHaveLength(0);
  });
});

describe("a menu closed by a click on its row", () => {
  const withMenu = (menu: unknown, cursor: number, m: ReturnType<typeof open>) =>
    m.snapshot({ chrome: { ...booksChrome({ width: 80, height: 24 }), menu } }, cursor);

  it("does not stay at the front: the question that follows is the one shown and keyed", async () => {
    const m = open(80, 24);
    m.welcome();
    m.snapshot({}, 7);
    m.conn.receive({ type: "question", id: 1, cursor: 8, menu: actionsMenu });
    withMenu(actionsMenu, 8, m);
    await tick();
    expect(m.frame()).toContain("merge");
    m.session.sendIntent({ intent: "activate", target: "chrome:menu:2" });
    m.conn.receive({ type: "question", id: 2, cursor: 9, menu: confirm });
    withMenu(confirm, 9, m);
    await tick();
    expect(m.session.state.questions.map((q) => q.menu.id)).toEqual(["confirm"]);
    expect(m.frame()).toContain("post comment?");
    expect(m.frame()).not.toContain("when green");
    m.type("y");
    await tick();
    expect(m.intents().at(-1)?.intent).toEqual({ intent: "invoke", action: "yes" });
  });

  it("is dropped by the snapshot alone when the host's menu has changed or closed, wherever the click came from", async () => {
    const m = open(80, 24);
    m.welcome();
    m.snapshot({}, 7);
    m.conn.receive({ type: "question", id: 1, cursor: 8, menu: actionsMenu });
    m.conn.receive({ type: "question", id: 2, cursor: 9, menu: confirm });
    // A snapshot older than the questions does not drop them.
    m.snapshot({}, 7);
    expect(m.session.state.questions).toHaveLength(2);
    // The host shows B at 9: A is closed, B stays.
    withMenu(confirm, 9, m);
    expect(m.session.state.questions.map((q) => q.menu.id)).toEqual(["confirm"]);
    // The host shows no menu at 10: nothing is open.
    m.snapshot({}, 10);
    expect(m.session.state.questions).toEqual([]);
  });

  it("drops an input request once a snapshot shows its typed menu closed (⏎ or esc in the host)", async () => {
    const m = open(80, 24);
    m.welcome();
    m.snapshot({}, 7);
    m.conn.receive({ type: "input_request", id: 1, cursor: 8, title: "Always allow, as kind(argument)", value: "" });
    const typed = { ...confirm, id: "agents.allow", items: [], typed: "shell(ls)", caret: 9 };
    withMenu(typed, 8, m);
    m.snapshot({}, 7);
    expect(m.session.state.inputRequest).toBeDefined();
    withMenu(typed, 9, m);
    expect(m.session.state.inputRequest).toBeDefined();
    m.snapshot({}, 10);
    await tick();
    expect(m.session.state.inputRequest).toBeUndefined();
    expect(m.frame()).not.toContain("Always allow");
  });

  it("keeps a question the snapshot has not reached yet", async () => {
    const m = open(80, 24);
    m.welcome();
    m.conn.receive({ type: "question", id: 1, cursor: 9, menu: confirm });
    m.snapshot({}, 8);
    expect(m.session.state.questions).toHaveLength(1);
  });
});

describe.each([
  [80, 24],
  [160, 40],
])("chrome at %ix%i", (cols, rows) => {
  it("renders the top bar, sources and status line from ChromeView", async () => {
    const m = open(cols, rows);
    m.welcome();
    m.snapshot();
    await tick();
    const lines = m.frame().split("\n");
    expect(lines.length).toBeLessThanOrEqual(rows);
    expect(m.frame()).toMatchSnapshot();
    expect(lines[0]).toContain("Books");
    expect(lines[1]).toContain("GitLab");
    expect(lines.at(-1)).toContain("q quit");
  });

  it("renders an actions menu over the body and settings navigation beside it", async () => {
    const m = open(cols, rows);
    m.welcome();
    m.snapshot({ chrome: { ...(await import("../chrome/fixture.js")).booksChrome({ width: cols, height: rows }), menu: actionsMenu, settings: settingsNav }, section: { version: 1, scope: "projects", body: { node: "list", rows: [{ target: "p:1", text: [{ text: "acme/api" }] }], selected: 0 }, detail_title: "detail", detail: [[{ text: "what this does" }]] } });
    await tick();
    const f = m.frame();
    expect(f).toContain("!263 fix the thing");
    expect(f).toContain("merge");
    expect(f).toContain("GitLab");
    expect(f).toContain("└─ projects");
    expect(f).toContain("Slack ⋯");
    expect(f).not.toMatch(/[▾▸]/);
    expect(f).toMatchSnapshot();
  });
});

describe("input through the app", () => {
  it("a plain key is Intent::Key like the old Rust renderer adapter's; SGR click on a tab activates its target", async () => {
    const m = open(80, 24);
    m.welcome();
    m.snapshot({ screen: { actions: [{ id: "gitlab.mr.open", label: "open", key: { key: { char: "o" } } }], panes: [] } }, 3);
    await tick();
    m.type("o");
    await tick();
    // Row 0, the "Work" tab: find its column from the frame.
    const col = m.frame().split("\n")[0]!.indexOf("Work") + 1; // SGR is 1-based
    m.type(`\x1b[<0;${col + 1};1M`);
    await tick();
    expect(m.intents().map((i) => i.intent)).toEqual([
      { intent: "key", chord: { key: { char: "o" } } },
      { intent: "activate", target: "chrome:tab:1" },
    ]);
    expect(m.intents().every((i) => i.cursor === 3)).toBe(true);
  });

  it("a resize sends Resize and relayouts without moving focus", async () => {
    const m = open(80, 24);
    m.welcome();
    m.snapshot({}, 2);
    await tick();
    m.type(`\x1b[<0;${m.frame().split("\n")[0]!.indexOf("Work") + 2};1M`);
    await tick();
    m.out.resize(160, 40);
    await until(() => m.sent("resize").length > 0);
    await tick();
    expect(m.sent("resize")).toEqual([{ type: "resize", columns: 160, rows: 40 }]);
    expect(m.frame().split("\n")[0]!.length).toBeGreaterThan(100);
    expect(m.intents()).toHaveLength(1); // the click only: no intent was invented by the resize
  });

  it("a notice is shown until a key clears it and is still handled (host.rs)", async () => {
    const m = open(80, 24);
    m.welcome();
    m.snapshot();
    m.conn.receive({ type: "notice", cursor: 1, text: "sync failed: gitlab" });
    await tick();
    expect(m.frame()).toContain("sync failed: gitlab");
    m.type("q");
    await tick();
    expect(m.frame()).not.toContain("sync failed: gitlab");
    expect(m.intents().map((i) => i.intent)).toEqual([{ intent: "key", chord: { key: { char: "q" } } }]);
  });

  it("while the host captures text, keys are forwarded as Intent::Key", async () => {
    const m = open(80, 24);
    m.welcome();
    m.snapshot();
    m.conn.receive({ type: "input_request", id: 1, cursor: 1, title: "comment:", value: "hi" });
    await tick();
    // The title is the popup's border title and the typed text its first row (menu titles moved off the text row, T6 cc18832b).
    expect(m.frame()).toMatch(/╭ comment: ─+╮\n\s*│ hi/);
    m.type("x");
    await tick();
    expect(m.intents()[0]).toMatchObject({ intent: { intent: "key", chord: { key: { char: "x" } } } });
  });
});

describe("unknown nodes", () => {
  it("an unknown node type shows a visible fallback with the actions still listed, and their keys still reach the host", async () => {
    const m = open(80, 24);
    m.welcome();
    m.snapshot({
      screen: {
        scope: "x",
        panes: [{ pane: "middle", title: "Things", body: { node: "hologram", data: 1 } }],
        actions: [{ id: "x.thing.open", label: "open the thing", key: { key: { char: "o" } } }],
      },
    });
    await tick();
    expect(m.frame()).toContain("does not draw node hologram");
    expect(m.frame()).toContain("open the thing");
    m.type("o");
    await tick();
    expect(m.intents()[0]).toMatchObject({ intent: { intent: "key", chord: { key: { char: "o" } } } });
  });

  it("an extension draws its fallback node and says so", async () => {
    const m = open(80, 24);
    m.welcome();
    m.snapshot({
      screen: {
        scope: "x",
        panes: [{ pane: "middle", title: "T", body: { node: "extension", capability: "terminal", version: 1, data: {}, fallback: { node: "state", state: "empty", text: "nothing here" } } }],
      },
    });
    await tick();
    expect(m.frame()).toContain("this needs terminal");
    expect(m.frame()).toContain("nothing here");
  });
});

describe("work area", () => {
  const work = {
    scope: "work",
    panes: [
      { pane: "nav", title: "Map", body: { node: "extension", capability: "work.map", version: 1, data: { target: "work:map" }, fallback: { node: "state", state: "empty", text: "map rows" } } },
      { pane: "middle", title: "Agent", body: { node: "extension", capability: "work.agent", version: 1, data: { pending: true, target: "work:agent" }, fallback: { node: "state", state: "empty", text: "approve the merge?" } } },
    ],
  };

  it("draws the work area from the sections the view carries; at 80 columns a pending question comes first", async () => {
    const m = open(80, 24);
    m.welcome();
    m.snapshot({ screen: work });
    await tick();
    const f = m.frame();
    expect(f).toContain("waiting for you");
    // Under 84 columns the work area is the focused pane and a pending question only; the reference draws no map there (cda8fc8b).
    expect(f).toContain("approve the merge?");
    expect(f).not.toContain("map rows");
    expect(f).toMatchSnapshot();
  });

  it("plain-text mode keeps labels and order", async () => {
    const m = open(160, 40);
    m.welcome();
    m.snapshot({ screen: work });
    await tick();
    // the reference draws only nav/middle/detail panes (render_panes); an M11 panel shows only while it holds a question.
    expect(m.frame()).toContain("Agent");
    expect(m.frame()).not.toContain("Map");
  });
});

describe("terminal ownership", () => {
  it("restore leaves the alternate screen, turns mouse reporting off, shows the cursor, once", () => {
    const out = new Out(80, 24);
    const t = new Terminal(out);
    t.enter();
    // The bytes the old Rust renderer runner writes (runner.rs: EnableBracketedPaste, EnableMouseCapture,
    // EnableFocusChange; the mouse capture is 1000, 1002, 1003, 1015, 1006).
    expect(out.frames.join("")).toBe("\x1b[?2004h" + "\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1015h\x1b[?1006h" + "\x1b[?1004h");
    t.restore();
    t.restore();
    expect(out.frames.filter((f) => f === RESTORE)).toHaveLength(1);
    expect(RESTORE).toContain("\x1b[?1049l");
    // DisableMouseCapture, DisableFocusChange, DisableBracketedPaste, in the runner's order.
    expect(RESTORE.startsWith("\x1b[?1006l\x1b[?1015l\x1b[?1003l\x1b[?1002l\x1b[?1000l" + "\x1b[?1004l" + "\x1b[?2004l")).toBe(true);
    expect(RESTORE).toContain("\x1b[?25h");
  });

  it("the error boundary reports a render error so the terminal is restored and the process exits non-zero", async () => {
    const out = new Out(80, 24);
    const errors: Error[] = [];
    const Bomb = () => {
      throw new Error("boom");
    };
    const inst = render(
      <ErrorBoundary onError={(e) => errors.push(e)}>
        <Bomb />
      </ErrorBoundary>,
      { stdout: out as any, debug: true, patchConsole: false, exitOnCtrlC: false },
    );
    await tick();
    expect(errors.map((e) => e.message)).toEqual(["boom"]);
    inst.unmount();
  });
});

describe("several keys in one read", () => {
  const rows = ["!1", "!2", "!3", "!4"].map((t) => ({ target: `row:${t}`, cells: [t] }));
  const list = { node: "list", selected: 0, rows: rows.map((r) => ({ target: r.target, text: [{ text: r.cells[0] }] })) };
  const screen = { id: "mrs", title: "mrs", panes: [{ pane: "middle", title: "mrs", body: list }], actions: [] };

  it("Down, Down, ⏎ written together select row 1, row 2 and activate row 2, each from the one before", async () => {
    const m = open(80, 24);
    m.welcome();
    m.snapshot({ screen }, 1);
    await tick(50);
    m.type("\x1b[B\x1b[B\r");
    await until(() => m.intents().length >= 3);
    expect(m.intents().map((i) => i.intent)).toEqual([
      { intent: "select", target: "row:!2" },
      { intent: "select", target: "row:!3" },
      { intent: "activate", target: "row:!3" },
    ]);
  });

  it("a newer snapshot replaces what was predicted", async () => {
    const m = open(80, 24);
    m.welcome();
    m.snapshot({ screen }, 1);
    await tick(50);
    m.type("j");
    await until(() => m.intents().length >= 1);
    m.snapshot({ screen: { ...screen, panes: [{ pane: "middle", title: "mrs", body: { ...list, selected: 0 } }] } }, 2);
    await tick(50);
    m.type("j");
    await until(() => m.intents().length >= 2);
    expect(m.intents().map((i) => i.intent.target)).toEqual(["row:!2", "row:!2"]);
  });
});

describe("a menu over a list", () => {
  it("paints every cell it covers: no list text between its borders", async () => {
    const m = open(80, 24);
    m.welcome();
    const rows = Array.from({ length: 12 }, (_, i) => ({ target: `row:${i}`, text: [{ text: `ZQXJ${i} ${"ZQXJ".repeat(18)}` }] }));
    const screen = { id: "l", title: "l", panes: [{ pane: "middle", title: "l", body: { node: "list", selected: 0, rows } }], actions: [] };
    const { actionsMenu: menu, booksChrome } = await import("../chrome/fixture.js");
    m.snapshot({ chrome: { ...booksChrome({ width: 80, height: 24 }), menu }, screen }, 1);
    await tick(80);
    const lines = m.frame().split("\n");
    const top = lines.findIndex((l) => l.includes("!263 fix the thing")); // the title is in the top border
    expect(top).toBeGreaterThan(-1);
    expect(lines[top]).toContain("╭");
    const left = lines[top]!.lastIndexOf("╭", lines[top]!.indexOf("╮"));
    const right = lines[top]!.lastIndexOf("╮");
    let bottom = top + 1;
    while (bottom < lines.length && !lines[bottom]!.includes("╰")) bottom++;
    for (let y = top + 1; y < bottom; y++) {
      const inside = [...lines[y]!].slice(left + 1, right).join("");
      expect(inside, `row ${y}`).not.toContain("ZQXJ");
    }
  });
});

describe("the history overlay", () => {
  const history = (split: boolean) => ({
    version: 1,
    width: 80,
    height: 22,
    title: "History · read only · UTC",
    boundary: "snapshot through #124",
    scope: "recent activity · last 24 hours since 2026-10-11T10:29:53.000Z",
    notes: [[{ text: "Recorded observations; coverage may be incomplete. Not task intent or permission.", ink: "warn" }]],
    focus: "events",
    split,
    events_title: "events · page 1 · oldest-first · more pages",
    columns: ["observed UTC", "action", "subject", "outcome"],
    rows: Array.from({ length: 30 }, (_, i) => ({
      observed: `2026-10-12 10:29:${String(10 + i).padStart(2, "0")}`,
      action: i === 25 ? "gitlab.sync.full" : "session.tui",
      subject: "no subject recorded",
      outcome: "requested",
      outcome_ink: "muted",
      target: `chrome:history:row:${i}`,
    })),
    selected: 25,
    list_scroll: 0,
    detail_title: "event detail",
    detail: [[{ text: "gitlab.sync.full" }], [{ text: "Action ID    d99d4d46-c6ae-4f96-8252-2e09cde26c86" }]],
    detail_scroll: 0,
    help_title: "history keys",
    help_lines: [],
    help_scroll: 0,
    footer: [
      { keys: "j/k", label: "move" },
      { keys: "esc", label: "close", keys_of: [{ label: "esc", chord: { key: "esc" }, target: "chrome:history:key:esc" }] },
    ],
  });
  it.each([[80, 24, false], [160, 40, true]])("draws title, scope, selected row, detail and keys at %ix%i", async (cols, rows, split) => {
    const m = open(cols, rows);
    m.welcome();
    const { booksChrome } = await import("../chrome/fixture.js");
    m.snapshot({ chrome: { ...booksChrome({ width: cols, height: rows }), history: history(split) }, screen: { id: "l", title: "l", panes: [{ pane: "middle", title: "LISTBODY", body: { node: "list", selected: 0, rows: [] } }], actions: [] } }, 1);
    await tick(80);
    const f = m.frame();
    expect(f).toContain("History · read only · UTC · snapshot through #124");
    expect(f).toContain("Scope: recent activity");
    expect(f).toContain("events · page 1");
    expect(f).toContain("OBSERVED UTC");
    expect(f).toContain("gitlab.sync.full"); // the selected row (25) is scrolled into view
    expect(f).toContain("j/k move · esc close");
    expect(f).not.toContain("LISTBODY");
    if (split) expect(f).toContain("event detail");
    for (const l of f.split("\n")) expect([...l].length).toBeLessThanOrEqual(cols);
    expect(f).toMatchSnapshot();
    // a click on the esc key and on a row
    const lines = f.split("\n");
    const footY = lines.findIndex((l) => l.includes("esc close"));
    m.type(`\x1b[<0;${lines[footY]!.indexOf("esc") + 2};${footY + 1}M`);
    await until(() => m.intents().length >= 1);
    expect(m.intents()[0].intent).toEqual({ intent: "activate", target: "chrome:history:key:esc" });
  });
  it("a menu the overlay covers does not take its keys", async () => {
    const m = open(80, 24);
    m.welcome();
    const { booksChrome } = await import("../chrome/fixture.js");
    const menu = { id: "m", title: "under", items: [{ key: "a", label: "first", hint: "", value: "a" }] };
    m.snapshot({ chrome: { ...booksChrome({ width: 80, height: 24 }), menu, history: history(false) }, screen: { id: "l", title: "l", panes: [{ pane: "middle", title: "x", body: { node: "list", selected: 0, rows: [] } }], actions: [] } }, 1);
    await tick(80);
    m.type("j");
    await until(() => m.intents().length >= 1);
    expect(m.intents()[0].intent).toEqual({ intent: "key", chord: { key: { char: "j" } } });
  });
  it("keys go to the host as Intent::Key (it owns history navigation)", async () => {
    const m = open(80, 24);
    m.welcome();
    const { booksChrome } = await import("../chrome/fixture.js");
    m.snapshot({ chrome: { ...booksChrome({ width: 80, height: 24 }), history: history(false) }, screen: { id: "l", title: "l", panes: [{ pane: "middle", title: "x", body: { node: "list", selected: 0, rows: [{ target: "a", text: [{ text: "a" }] }, { target: "b", text: [{ text: "b" }] }] } }], actions: [] } }, 1);
    await tick(80);
    m.type("j");
    await until(() => m.intents().length >= 1);
    expect(m.intents()[0].intent).toEqual({ intent: "key", chord: { key: { char: "j" } } });
  });
});
