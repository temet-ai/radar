import { describe, expect, it } from "vitest";
import { dockClick, dockClose, dockCloseKey, dockHint, dockKey, dockPaste, dockScroll, emptyLocal, isCloseChord, matchingCommands, menuWindow, type DockLocal } from "../../src/dock/composer.js";
import { pickKey } from "../../src/dock/rows.js";
import { STATES } from "./states.js";
import type { DockView } from "../../src/dock/model.js";
import type { Chord } from "../../src/chrome/types.js";

const c = (s: string, mods: Partial<Chord> = {}): Chord => ({ key: { char: s }, ...mods });
const view = (over: Partial<DockView> = {}): DockView => ({
  visible: true,
  focused: true,
  tabs: [
    { id: "a", provider: "copilot", title: "one", state: "idle" },
    { id: "b", provider: "codex", title: "two", state: "idle" },
  ],
  active: "a",
  composer: { can_send: true, can_stop: false, history: ["first", "second"], commands: [{ name: "model", detail: "" }, { name: "clear", detail: "" }] },
  ...over,
});
const type = (l: DockLocal, v: DockView, s: string) => [...s].reduce((acc, ch) => dockKey(acc, v, c(ch)).local, l);

describe("dock keys", () => {
  it("types, edits and sends", () => {
    const v = view();
    let l = type(emptyLocal(), v, "helo");
    l = dockKey({ ...l, caret: 3 }, v, c("l")).local;
    expect(l.text).toBe("hello");
    const r = dockKey(l, v, { key: "enter" });
    expect(r.intents).toEqual([{ dock: "send", text: "hello" }]);
    expect(r.local.text).toBe("");
  });
  it("does not send an empty message, or while sending is off, and then keeps the text and says why", () => {
    expect(dockKey(emptyLocal(), view(), { key: "enter" }).intents).toEqual([]);
    const off = view({ composer: { can_send: false, can_stop: false, disabled: "disconnected from the provider" } });
    const r = dockKey(emptyLocal("and?"), off, { key: "enter" });
    expect(r.intents).toEqual([]);
    expect(r.local.text).toBe("and?");
    expect(dockHint(off, r.local.refused)).toBe("not sent: disconnected from the provider");
    // The next key clears the words; the text was never lost.
    expect(dockKey(r.local, view(), c("!")).local.refused).toBeUndefined();
  });
  it("sends while a turn runs: ⏎ is never ignored for a busy session (the host queues what the provider cannot take)", () => {
    const busy = { ...STATES.streaming!, focused: true };
    let l = type(emptyLocal(), busy, "and?");
    const r = dockKey(l, busy, { key: "enter" });
    expect(r.intents).toEqual([{ dock: "send", text: "and?" }]);
    expect(r.local.text).toBe("");
    expect(dockHint(busy)).toContain("esc stop");
    // A permission that waits keeps its letter keys for an empty composer, and ⏎ still sends what is typed.
    l = type(emptyLocal(), STATES.approval!, "use main");
    expect(dockKey(l, STATES.approval!, { key: "enter" }).intents).toEqual([{ dock: "send", text: "use main" }]);
  });
  it("up and down move the caret by line in a multi-line message, and only the ends reach the history", () => {
    const v = view();
    let l = { ...emptyLocal("one\ntwo is longer\nthree"), caret: 23 }; // the end of "three", column 5
    l = dockKey(l, v, { key: "up" }).local;
    expect(l.caret).toBe(9); // column 5 of "two is longer"
    l = dockKey(l, v, { key: "up" }).local;
    expect(l.caret).toBeLessThanOrEqual(3);
    expect(l.text).toBe("one\ntwo is longer\nthree");
    const down = dockKey(l, v, { key: "down" }).local;
    expect(down.caret).toBeGreaterThan(3);
    const top = dockKey({ ...l, caret: 1 }, v, { key: "up" }).local;
    expect(top.text).toBe("second");
  });
  it("alt-enter breaks the line", () => {
    expect(dockKey(emptyLocal("ab"), view(), { key: "enter", alt: true }).local.text).toBe("ab\n");
  });
  it("esc gives the keys back", () => {
    expect(dockKey(emptyLocal("x"), view(), { key: "esc" }).intents).toEqual([{ dock: "blur" }]);
  });
  it("walks the history up and back down to what was being written", () => {
    const v = view();
    let r = dockKey(emptyLocal("draft"), v, { key: "up" });
    expect(r.local.text).toBe("second");
    r = dockKey(r.local, v, { key: "up" });
    expect(r.local.text).toBe("first");
    r = dockKey(r.local, v, { key: "down" });
    r = dockKey(r.local, v, { key: "down" });
    expect(r.local.text).toBe("draft");
  });
  it("answers an approval with letters only while nothing is typed", () => {
    const v = view({ transcript: [{ item: "permission", id: "p1", kind: "shell", target: "ls", actions: ["approve_once", "deny", "view_diff"] }] });
    expect(dockKey(emptyLocal(), v, c("a")).intents).toEqual([{ dock: "approve", id: "p1" }]);
    expect(dockKey(emptyLocal(), v, c("d")).intents).toEqual([{ dock: "deny", id: "p1" }]);
    expect(dockKey(emptyLocal(), v, c("v")).local.diffs).toEqual(["p1"]);
    expect(dockKey(emptyLocal("x"), v, c("a")).intents).toEqual([]);
    // s̲ession only where the request offers it; elsewhere s is typed.
    expect(dockKey(emptyLocal(), v, c("s")).intents).toEqual([]);
    const s = view({ transcript: [{ item: "permission", id: "p1", kind: "shell", target: "ls", actions: ["approve_once", "approve_for_session", "deny"] }] });
    expect(dockKey(emptyLocal(), s, c("s")).intents).toEqual([{ dock: "approve_for_session", id: "p1" }]);
    expect(dockKey(emptyLocal("x"), s, c("s")).intents).toEqual([]);
    expect(dockClick(emptyLocal(), s, "dock:session:p1")!.intents).toEqual([{ dock: "approve_for_session", id: "p1" }]);
    expect(dockClick(emptyLocal(), v, "dock:session:p1")).toBeUndefined();
  });
  it("a click on an approval's buttons does what its keys do", () => {
    const v = view({ transcript: [{ item: "permission", id: "p:1", kind: "write", target: "a", diff: ["+x"], actions: ["approve_once", "deny", "view_diff"] }] });
    expect(dockClick(emptyLocal(), v, "dock:approve:p%3A1")!.intents).toEqual([{ dock: "approve", id: "p:1" }]);
    expect(dockClick(emptyLocal(), v, "dock:deny:p%3A1")!.intents).toEqual([{ dock: "deny", id: "p:1" }]);
    const open = dockClick(emptyLocal(), v, "dock:diff:p%3A1")!;
    expect(open.intents).toEqual([]);
    expect(open.local.diffs).toEqual(["p:1"]);
    expect(dockClick(open.local, v, "dock:diff:p%3A1")!.local.diffs).toEqual([]);
    // a request that no longer waits, or another one, takes no click
    expect(dockClick(emptyLocal(), view({ transcript: [{ item: "permission", id: "p1", kind: "shell", target: "ls", actions: ["approve_once", "deny"], answered: "deny" }] }), "dock:approve:p1")).toBeUndefined();
    expect(dockClick(emptyLocal(), v, "dock:approve:other")).toBeUndefined();
  });
  it("a waiting question: a number answers with its choice, ⏎ sends the typed words, esc declines", () => {
    const v = { ...STATES.question!, composer: { can_send: false, can_stop: true } };
    expect(dockKey(emptyLocal(), v, c("2")).intents).toEqual([{ dock: "answer", id: "q1", question: "0", choices: ["patch"] }]);
    expect(dockKey(emptyLocal(), v, c("7")).local.text).toBe("7");
    const typed = dockKey(emptyLocal("my own fix"), v, { key: "enter" });
    expect(typed.intents).toEqual([{ dock: "answer", id: "q1", question: "0", text: "my own fix" }]);
    expect(typed.local.text).toBe("");
    // esc declines before it would stop the turn
    expect(dockKey(emptyLocal(), v, { key: "esc" }).intents).toEqual([{ dock: "decline", id: "q1" }]);
    expect(dockClick(emptyLocal(), v, "dock:choose:q1:0:revert")!.intents).toEqual([{ dock: "answer", id: "q1", question: "0", choices: ["revert"] }]);
    expect(dockClick(emptyLocal(), v, "dock:decline:q1")!.intents).toEqual([{ dock: "decline", id: "q1" }]);
    expect(dockClick(emptyLocal(), v, "dock:choose:q1:0:nope")).toBeUndefined();
  });
  it("a question that takes several choices: numbers and clicks pick, ⏎ or submit sends the picks", () => {
    const v = STATES["question-several"]!;
    let r = dockKey(emptyLocal(), v, c("1"));
    r = dockKey(r.local, v, c("3"));
    r = dockClick(r.local, v, "dock:choose:q2:Which%20checks%3F:tests")!; // unpicks
    expect(r.intents).toEqual([]);
    expect(r.local.picks[pickKey("q2", "Which checks?")]).toEqual(["fmt"]);
    const sent = dockKey(r.local, v, { key: "enter" });
    expect(sent.intents).toEqual([{ dock: "answer", id: "q2", question: "Which checks?", choices: ["fmt"] }]);
    expect(sent.local.picks).toEqual({});
    const clicked = dockClick(dockKey(emptyLocal(), v, c("2")).local, v, "dock:submit:q2:Which%20checks%3F")!;
    expect(clicked.intents).toEqual([{ dock: "answer", id: "q2", question: "Which checks?", choices: ["clippy"] }]);
    // ⏎ with nothing picked or typed sends nothing
    expect(dockKey(emptyLocal(), v, { key: "enter" }).intents).toEqual([]);
  });
  it("never auto-answers: an answered request takes no letter", () => {
    const v = view({ transcript: [{ item: "permission", id: "p1", kind: "shell", target: "ls", actions: ["approve_once", "deny"], answered: "deny" }] });
    expect(dockKey(emptyLocal(), v, c("a")).local.text).toBe("a");
  });
  it("ctrl-c stops a running turn and otherwise passes", () => {
    expect(dockKey(emptyLocal(), view({ composer: { can_send: false, can_stop: true } }), c("c", { ctrl: true })).intents).toEqual([{ dock: "stop" }]);
    expect(dockKey(emptyLocal(), view(), c("c", { ctrl: true })).handled).toBe(false);
  });
  it("esc stops a running turn and gives the keys back when none runs", () => {
    expect(dockKey(emptyLocal(), view({ composer: { can_send: false, can_stop: true } }), { key: "esc" }).intents).toEqual([{ dock: "stop" }]);
    expect(dockKey(emptyLocal(), view(), { key: "esc" }).intents).toEqual([{ dock: "blur" }]);
  });
  it("the dock key hides the dock from an empty composer and types in a full one", () => {
    expect(dockKey(emptyLocal(), view(), c("`")).intents).toEqual([{ dock: "hide" }]);
    expect(dockKey(emptyLocal("a"), view(), c("`")).local.text).toBe("a`");
  });
  it("switches tabs with alt-arrows, wrapping", () => {
    expect(dockKey(emptyLocal(), view(), { key: "right", alt: true }).intents).toEqual([{ dock: "switch_tab", id: "b" }]);
    expect(dockKey(emptyLocal(), view(), { key: "left", alt: true }).intents).toEqual([{ dock: "switch_tab", id: "b" }]);
  });
  describe("closing a tab", () => {
    it("alt-w closes the active tab, whatever is written; the draft goes with an idle tab", () => {
      const r = dockKey(emptyLocal("half a thought"), view(), c("w", { alt: true }));
      expect(r.intents).toEqual([{ dock: "close_tab", id: "a" }]);
      expect(r.local.text).toBe("");
    });
    it("a tab whose turn runs is asked about by the host, so its draft stays", () => {
      const running = view({ tabs: [{ id: "a", provider: "copilot", title: "one", state: "streaming" }, { id: "b", provider: "codex", title: "two", state: "idle" }] });
      const r = dockKey(emptyLocal("keep me"), running, c("w", { alt: true }));
      expect(r.intents).toEqual([{ dock: "close_tab", id: "a" }]);
      expect(r.local.text).toBe("keep me");
    });
    it("ctrl-w closes from an empty composer and deletes the word before the caret otherwise", () => {
      expect(dockKey(emptyLocal(), view(), c("w", { ctrl: true })).intents).toEqual([{ dock: "close_tab", id: "a" }]);
      const l = dockKey(emptyLocal("fix the  build"), view(), c("w", { ctrl: true }));
      expect(l.intents).toEqual([]);
      expect(l.local.text).toBe("fix the  ");
      expect(dockKey({ ...emptyLocal("fix the  "), caret: 9 }, view(), c("w", { ctrl: true })).local.text).toBe("fix ");
    });
    it("closing from the pointer is the same intent for any tab, the draft kept unless it is the active idle one", () => {
      const v = view();
      expect(dockClose(emptyLocal("x"), v, "b")).toEqual({ local: emptyLocal("x"), intents: [{ dock: "close_tab", id: "b" }], handled: true });
      expect(dockClose(emptyLocal("x"), v, "a").local.text).toBe("");
    });
    it("with no tab left alt-w hides the dock, so close leaves no empty window", () => {
      const empty = view({ tabs: [], active: null });
      expect(dockKey(emptyLocal(), empty, c("w", { alt: true })).intents).toEqual([{ dock: "hide" }]);
      expect(dockCloseKey(emptyLocal(), empty).intents).toEqual([{ dock: "hide" }]);
      expect(dockKey(emptyLocal(), empty, c("w", { ctrl: true })).intents).toEqual([]);
    });
    it("alt-w works without the dock's focus (the app asks dockCloseKey), and only alt-w is the chord", () => {
      expect(dockCloseKey(emptyLocal("x"), view({ focused: false })).intents).toEqual([{ dock: "close_tab", id: "a" }]);
      expect(isCloseChord(c("w", { alt: true }))).toBe(true);
      expect(isCloseChord(c("w"))).toBe(false);
      expect(isCloseChord(c("w", { ctrl: true }))).toBe(false);
      expect(isCloseChord(c("x", { alt: true }))).toBe(false);
      expect(isCloseChord({ key: "left", alt: true })).toBe(false);
    });
    it("the status line names the key beside what else the keys do now, only while the dock has them", () => {
      expect(dockHint(view())).toBe("alt-f full · alt-w close");
      expect(dockHint(view({ composer: { can_send: false, can_stop: true } }))).toBe("esc stop · alt-f full · alt-w close");
      expect(dockHint(view({ full: true }))).toBe("alt-f split · alt-w close");
      expect(dockHint(view({ focused: false }))).toBeUndefined();
      expect(dockHint(view({ tabs: [], active: null }))).toBe("alt-f full");
    });
  });
  it("alt-f asks the host for the whole middle column, and back; the text being written stays", () => {
    const l = emptyLocal("half a thought");
    for (const full of [false, true]) {
      const r = dockKey(l, view({ full }), c("f", { alt: true }));
      expect(r.intents).toEqual([{ dock: "toggle_full" }]);
      expect(r.local.text).toBe("half a thought");
    }
  });
  it("the dock key goes to the tab that wants you out of sight, else hides the dock", () => {
    expect(dockKey(emptyLocal(), view({ attention: "b" }), c("`")).intents).toEqual([{ dock: "switch_tab", id: "b" }]);
    expect(dockKey(emptyLocal(), view({ attention: null }), c("`")).intents).toEqual([{ dock: "hide" }]);
    // The tab showing is never out of sight; with text written the key is typed.
    expect(dockKey(emptyLocal(), view({ attention: "a" }), c("`")).intents).toEqual([{ dock: "hide" }]);
    expect(dockKey(emptyLocal("x"), view({ attention: "b" }), c("`")).local.text).toBe("x`");
  });
  it("leaves a terminal tab's keys to the terminal", () => {
    const v = view({ tabs: [{ id: "t", provider: "pty", title: "sh", state: "idle", terminal: { id: "t", socket: "s", columns: 1, rows: 1, state: { state: "running" }, command: "sh", cwd: "/" } }], active: "t" });
    expect(dockKey(emptyLocal(), v, c("a")).handled).toBe(false);
  });
  it("moves a picker's choice and picks it", () => {
    const v = view({ picker: { kind: "model", choices: [{ id: "m1", label: "one" }, { id: "m2", label: "two" }] } });
    const l = dockKey(emptyLocal(), v, { key: "down" }).local;
    expect(dockKey(l, v, { key: "enter" }).intents).toEqual([{ dock: "pick_model", id: "m2" }]);
    expect(dockKey(l, v, { key: "esc" }).intents).toEqual([{ dock: "close_picker" }]);
  });
  it("opens the agent and skill pickers; an agent is picked by the host, a skill is asked for in the composer", () => {
    expect(dockKey(emptyLocal(), view(), c("a", { alt: true })).intents).toEqual([{ dock: "open_picker", kind: "agent" }]);
    expect(dockKey(emptyLocal(), view(), c("k", { alt: true })).intents).toEqual([{ dock: "open_picker", kind: "skill" }]);
    const agents = view({ picker: { kind: "agent", choices: [{ id: "", label: "the CLI's own agent" }, { id: "reviewer", label: "reviewer" }] } });
    const l = dockKey(emptyLocal(), agents, { key: "down" }).local;
    expect(dockKey(l, agents, { key: "enter" }).intents).toEqual([{ dock: "pick_agent", id: "reviewer" }]);
    const skills = view({ picker: { kind: "skill", choices: [{ id: "notes", label: "notes" }] } });
    const r = dockKey(emptyLocal("then sum up"), skills, { key: "enter" });
    expect(r.intents).toEqual([{ dock: "pick_skill", id: "notes" }]);
    expect(r.local.text).toBe("then sum upUse the notes skill. ");
  });
  it("completes the slash menu's selected command with tab", () => {
    const v = view();
    expect(matchingCommands("/mo", v.composer!.commands)).toHaveLength(1);
    expect(dockKey(emptyLocal("/mo"), v, { key: "tab" }).local.text).toBe("/model ");
    // With several matches tab takes the selected one (the first until the arrows move it); with none it does nothing.
    expect(dockKey(emptyLocal("/"), v, { key: "tab" }).local.text).toBe("/model ");
    expect(dockKey({ ...emptyLocal("/"), cmd: 1 }, v, { key: "tab" }).local.text).toBe("/clear ");
    expect(dockKey(emptyLocal("/zz"), v, { key: "tab" }).local.text).toBe("/zz");
    expect(dockKey(emptyLocal("plain"), v, { key: "tab" }).local.text).toBe("plain");
  });
  describe("the slash menu", () => {
    const many = view({ composer: { can_send: true, can_stop: false, history: ["older"], commands: ["compact", "context", "config", "model", "review"].map((name) => ({ name, detail: `does ${name}` })) } });
    it("opens on a slash and narrows to the commands the word starts", () => {
      expect(matchingCommands("/", many.composer!.commands).map((c) => c.name)).toEqual(["compact", "context", "config", "model", "review"]);
      expect(matchingCommands("/co", many.composer!.commands).map((c) => c.name)).toEqual(["compact", "context", "config"]);
      expect(matchingCommands("/CON", many.composer!.commands).map((c) => c.name)).toEqual(["context", "config"]);
      // Closed once a word is done, on a path, and on text that is not a command.
      expect(matchingCommands("/compact now", many.composer!.commands)).toEqual([]);
      expect(matchingCommands("/etc/hosts", many.composer!.commands)).toEqual([]);
      expect(matchingCommands("hello /co", many.composer!.commands)).toEqual([]);
    });
    it("moves the selection with the arrows, stopping at both ends, and starts again at the top when the text changes", () => {
      let l = emptyLocal("/co");
      l = dockKey(l, many, { key: "down" }).local;
      l = dockKey(l, many, { key: "down" }).local;
      l = dockKey(l, many, { key: "down" }).local;
      expect(l.cmd).toBe(2);
      expect(l.text).toBe("/co");
      l = dockKey(l, many, { key: "up" }).local;
      expect(l.cmd).toBe(1);
      l = dockKey(l, many, { key: "up" }).local;
      l = dockKey(l, many, { key: "up" }).local;
      expect(l.cmd).toBe(0);
      l = dockKey({ ...l, cmd: 2 }, many, c("n")).local;
      expect(l).toMatchObject({ text: "/con", cmd: 0 });
    });
    it("takes the arrows from the history only while it is open", () => {
      // Closed: up walks back through what was sent.
      expect(dockKey(emptyLocal(""), many, { key: "up" }).local.text).toBe("older");
      expect(dockKey(emptyLocal("/zz"), many, { key: "up" }).local.text).toBe("older");
      // Open: up and down are the menu's.
      expect(dockKey(emptyLocal("/"), many, { key: "up" }).local.text).toBe("/");
    });
    it("tab and ⏎ on a half-typed command finish it; ⏎ on the whole word sends", () => {
      const down = dockKey(emptyLocal("/co"), many, { key: "down" }).local;
      expect(dockKey(down, many, { key: "tab" }).local.text).toBe("/context ");
      const entered = dockKey(down, many, { key: "enter" });
      expect(entered.local.text).toBe("/context ");
      expect(entered.intents).toEqual([]);
      // The finished word, with or without its space, is sent as it stands.
      expect(dockKey(emptyLocal("/context "), many, { key: "enter" }).intents).toEqual([{ dock: "send", text: "/context" }]);
      expect(dockKey(emptyLocal("/model"), many, { key: "enter" }).intents).toEqual([{ dock: "send", text: "/model" }]);
      // Text that is no command is sent as written.
      expect(dockKey(emptyLocal("/etc/hosts"), many, { key: "enter" }).intents).toEqual([{ dock: "send", text: "/etc/hosts" }]);
    });
    it("is not there while the composer is off", () => {
      const off = view({ composer: { ...many.composer!, disabled: "starting the session" } });
      expect(dockKey(emptyLocal("/co"), off, { key: "tab" }).local.text).toBe("/co");
      expect(dockKey(emptyLocal("/co"), off, { key: "enter" }).intents).toEqual([]);
    });
    it("keeps the selected row in a window of six", () => {
      expect(menuWindow(5, 0)).toEqual({ first: 0, count: 5 });
      expect(menuWindow(20, 0)).toEqual({ first: 0, count: 6 });
      expect(menuWindow(20, 5)).toEqual({ first: 0, count: 6 });
      expect(menuWindow(20, 9)).toEqual({ first: 4, count: 6 });
      expect(menuWindow(20, 19)).toEqual({ first: 14, count: 6 });
      expect(menuWindow(20, 19, 2)).toEqual({ first: 18, count: 2 });
      expect(menuWindow(0, 0)).toEqual({ first: 0, count: 0 });
    });
  });
  it("pages the transcript, stopping at its ends", () => {
    const v = view();
    const up = dockKey(emptyLocal(), v, { key: "page_up" }, 7).local;
    expect(up.scroll).toBe(5);
    expect(dockKey(up, v, { key: "page_up" }, 7).local.scroll).toBe(7);
    expect(dockKey(up, v, { key: "page_down" }).local.scroll).toBe(0);
  });
  it("pages from where new output left a scrolled view, not from where it was before the output", () => {
    const v = view();
    // Scrolled 5 up when the transcript had 20 rows; 4 rows came in since: the view is 9 up now, a page down makes it 4.
    const up = dockKey(emptyLocal(), v, { key: "page_up" }, { max: 10, total: 20 }).local;
    expect(up).toMatchObject({ scroll: 5, anchor: 20 });
    expect(dockScroll(up, 24)).toBe(9);
    expect(dockKey(up, v, { key: "page_down" }, { max: 14, total: 24 }).local).toMatchObject({ scroll: 4, anchor: 24 });
    // At the foot it follows: nothing is added.
    expect(dockScroll(emptyLocal(), 99)).toBe(0);
  });
  it("↑ and ↓ with no history to walk are the composer's, never the pane's", () => {
    const none = view({ composer: { can_send: true, can_stop: false, history: [] } });
    for (const key of ["up", "down", "page_up", "page_down", "home", "end"] as const) expect(dockKey(emptyLocal(), none, { key }).handled).toBe(true);
    // ↓ past the foot of the history, back at the message being written
    const v = view();
    const back = dockKey(dockKey(emptyLocal(), v, { key: "up" }).local, v, { key: "down" }).local;
    expect(back.walk).toBeNull();
    expect(dockKey(back, v, { key: "down" })).toMatchObject({ handled: true, intents: [] });
  });
  it("alt-o folds the latest tool call", () => {
    const v = view({ transcript: [{ item: "tool", id: "t1", title: "ls", kind: "shell", state: "done", open: false, result: ["a"] }] });
    expect(dockKey(emptyLocal(), v, c("o", { alt: true })).intents).toEqual([{ dock: "fold", item: "t1", open: true }]);
  });
  it("pastes with line breaks kept", () => {
    expect(dockPaste(emptyLocal("a"), "b\r\nc").text).toBe("ab\nc");
  });
});
