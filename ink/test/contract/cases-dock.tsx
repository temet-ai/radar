// The cases for the fields of the agent dock (DockView and what it holds) and of a terminal tab (TerminalView), reached
// through ScreenView.dock. Same shape as cases-screen.tsx: a scene with the field off and on; `on` must draw differently
// (or show the case's needle). Keys are `Label.field`, a tagged-union member's fields `Label.variant.field`.
// The shapes follow the host-recorded fixtures in test/fixtures/dock/.
import { chromeOf } from "./support.js";
import type { Case } from "./cases-screen.js";

const W = 160;
const H = 40;

const t = (s: string) => [{ text: s }];
const screenWith = (dock: any) => ({
  id: "doc",
  scope: "doc",
  panes: [{ pane: "middle", title: "doc", body: { node: "document", lines: [t("BODYLINE")] } }],
  actions: [],
  ...(dock === undefined ? {} : { dock }),
});

const tabOf = (over: Record<string, unknown> = {}) => ({ id: "s1", provider: "copilot", icon: "bot", title: "review dock", state: "idle", ...over });
const dockOf = (over: Record<string, unknown> = {}): any => ({
  visible: true,
  focused: true,
  tabs: [tabOf()],
  active: "s1",
  transcript: [],
  composer: { can_send: true, can_stop: false },
  status: {},
  ...over,
});

/** A dock built by `f` (from the base) in a screen of its own. `needle` is the text the on-state must show. */
const dock = (f: (d: any, on: boolean) => void, needle?: string): Case => (on) => {
  const d = dockOf();
  f(d, on);
  return { chrome: chromeOf(W, H), screen: screenWith(d), needle: on ? needle : undefined };
};
/** The transcript holds the item `make(on)` returns. */
const item = (make: (on: boolean) => any, needle?: string): Case => dock((d, on) => (d.transcript = [make(on)]), needle);
const pick = <T,>(on: boolean, off: T, yes: T): T => (on ? yes : off);
const ptyTab = (over: Record<string, unknown> = {}) =>
  tabOf({ id: "t1", provider: "pty", icon: "terminal", title: "codex login", terminal: { id: "t1", socket: "", columns: 78, rows: 12, state: { state: "running" }, command: "codex --version", cwd: "~/projects/radar", ...over } });
/** A terminal tab, its view built by `f`. */
const terminal = (f: (v: any, on: boolean) => void, needle?: string): Case =>
  dock((d, on) => {
    const tab: any = ptyTab();
    f(tab.terminal, on);
    d.tabs = [tab];
    d.active = "t1";
  }, needle);
const picker = (f: (p: any, on: boolean) => void, needle?: string): Case =>
  dock((d, on) => {
    const p: any = { kind: "model", choices: [{ id: "m1", label: "gpt-5", current: false }, { id: "m2", label: "claude", current: false }] };
    f(p, on);
    d.picker = p;
  }, needle);
/** As `dock`, with a message in the transcript (the commands list over its foot; an empty one draws a hint instead). */
const cmds = (f: (d: any, on: boolean) => void, needle?: string): Case =>
  dock((d, on) => {
    d.transcript = [{ item: "user", id: "u1", text: "hello" }];
    f(d, on);
  }, needle);
/** A question item waiting, one question with two choices; `f` changes the item (`q`) and its question (`a`). */
const question = (f: (q: any, a: any, on: boolean) => void, needle?: string): Case =>
  item((on) => {
    const a: any = { id: "0", text: "ASKTEXT", choices: [{ label: "first" }, { label: "second" }], free_text: false, multiple: false, answered: false };
    const q: any = { item: "question", id: "q1", questions: [a], state: "waiting" };
    f(q, a, on);
    return q;
  }, needle);
const perm = (f: (p: any, on: boolean) => void, needle?: string): Case =>
  item((on) => {
    const p: any = { item: "permission", id: "p1", kind: "shell", target: "cargo test", actions: ["approve_once", "deny"] };
    f(p, on);
    return p;
  }, needle);

export const DOCK_CASES: Record<string, Case> = {
  "ScreenView.dock": (on) => ({ chrome: chromeOf(W, H), screen: screenWith(on ? dockOf() : undefined) }),

  "DockView.visible": dock((d, on) => (d.visible = on)),
  "DockView.focused": dock((d, on) => (d.focused = on)),
  "DockView.rows": dock((d, on) => (d.rows = pick(on, 14, 20))),
  "DockView.tabs": dock((d, on) => (d.tabs = on ? [tabOf()] : []), "review dock"),
  "DockView.active": dock((d, on) => {
    d.tabs = [tabOf(), tabOf({ id: "s2", title: "second one" })];
    d.active = on ? "s2" : "s1";
  }, "agents · second one"),
  "DockView.transcript": dock((d, on) => (d.transcript = on ? [{ item: "user", id: "u1", text: "TRANSCRIPTTXT" }] : []), "TRANSCRIPTTXT"),
  "DockView.composer": dock((d, on) => (d.composer = on ? { draft: "COMPOSERTXT", can_send: true, can_stop: false } : { can_send: true, can_stop: false }), "COMPOSERTXT"),
  "DockView.status": dock((d, on) => (d.status = on ? { provider: "STATUSTXT" } : {}), "STATUSTXT"),
  "DockView.picker": dock((d, on) => (d.picker = on ? { kind: "model", choices: [{ id: "m1", label: "PICKTXT", current: false }] } : null), "PICKTXT"),

  // ---- tabs ------------------------------------------------------------------------------------------------
  "DockTab.title": dock((d, on) => (d.tabs = [tabOf({ title: pick(on, "plain", "TABTITLE") })]), "TABTITLE"),
  "DockTab.icon": dock((d, on) => (d.tabs = [tabOf({ icon: pick(on, null, "terminal") })])),
  "DockTab.provider": dock((d, on) => (d.tabs = [tabOf({ provider: pick(on, "copilot", "codex") })])),
  "DockTab.state": dock((d, on) => (d.tabs = [tabOf({ state: pick(on, "idle", "streaming") })])),
  "DockTab.terminal": dock((d, on) => {
    const tab: any = on ? ptyTab() : tabOf({ id: "t1", provider: "pty", title: "codex login" });
    d.tabs = [tab];
    d.active = "t1";
  }, "codex --version"),
  "TerminalView.command": terminal((v, on) => (v.command = pick(on, "plain", "TERMCMD")), "TERMCMD"),
  "TerminalView.cwd": terminal((v, on) => (v.cwd = pick(on, "~/a", "~/b"))),
  "TerminalView.state": terminal((v, on) => (v.state = pick(on, { state: "running" }, { state: "failed" })), "failed"),

  // ---- the transcript ----------------------------------------------------------------------------------------
  "DockItem.user.text": item((on) => ({ item: "user", id: "u1", text: pick(on, "plain", "USERTXT") }), "USERTXT"),
  "DockItem.user.queued": item((on) => ({ item: "user", id: "u1", text: "USERTXT", ...(on ? { queued: "here" } : {}) }), "sent when this turn ends"),
  "DockItem.assistant.text": item((on) => ({ item: "assistant", id: "a1", text: pick(on, "plain", "ASSISTTXT"), saved: true }), "ASSISTTXT"),
  "DockItem.assistant.saved": item((on) => ({ item: "assistant", id: "a1", text: "ASSISTTXT", saved: on })),
  "DockItem.assistant.doc": item((on) => ({ item: "assistant", id: "a1", text: "## DOCHEAD", saved: true, doc: on ? [{ t: "heading", level: 2, text: [{ text: "DOCHEAD" }] }] : undefined })),
  "DockItem.assistant.painted": item((on) => ({
    item: "assistant",
    id: "a1",
    text: "```rust\nfn x\n```",
    saved: true,
    doc: [{ t: "code", lang: "rust", text: "fn x" }],
    painted: on ? [[[{ text: "fn", token: "keyword" }, { text: " x", token: "plain" }]]] : [],
  })),
  "DockItem.reasoning.text": item((on) => ({ item: "reasoning", id: "r1", text: pick(on, "plain", "REASONTXT"), open: true }), "REASONTXT"),
  "DockItem.reasoning.open": item((on) => ({ item: "reasoning", id: "r1", text: "REASONTXT\nsecond", open: on }), "REASONTXT"),
  "DockItem.tool.title": item((on) => ({ item: "tool", id: "t1", title: pick(on, "plain", "TOOLTITLE"), kind: "shell", state: "done", open: false }), "TOOLTITLE"),
  "DockItem.tool.kind": item((on) => ({ item: "tool", id: "t1", title: "TOOLTITLE", kind: pick(on, "shell", "read"), state: "done", open: false })),
  "DockItem.tool.state": item((on) => ({ item: "tool", id: "t1", title: "TOOLTITLE", kind: "shell", state: pick(on, "done", "failed"), open: false })),
  "DockItem.tool.open": item((on) => ({ item: "tool", id: "t1", title: "TOOLTITLE", kind: "shell", state: "done", open: on, result: ["RESULTTXT"] }), "RESULTTXT"),
  "DockItem.tool.result": item((on) => ({ item: "tool", id: "t1", title: "TOOLTITLE", kind: "shell", state: "done", open: true, result: pick(on, [], ["RESULTTXT"]) }), "RESULTTXT"),
  "DockItem.tool.diff": item((on) => ({ item: "tool", id: "t1", title: "a.rs", kind: "Edit", state: "done", open: false, diff: pick(on, [], ["-old", "+new", "+more"]) }), "+2 −1"),
  "DockItem.error.text": item((on) => ({ item: "error", id: "e1", text: pick(on, "plain", "ERRORTXT") }), "ERRORTXT"),
  "DockItem.notice.text": item((on) => ({ item: "notice", id: "n1", text: pick(on, "plain", "NOTICETXT") }), "NOTICETXT"),

  "DockPermission.kind": perm((p, on) => (p.kind = pick(on, "shell", "PERMKIND")), "allow PERMKIND?"),
  "DockPermission.target": perm((p, on) => (p.target = pick(on, "plain", "PERMTARGET")), "PERMTARGET"),
  "DockPermission.actions": perm((p, on) => (p.actions = on ? ["approve_once", "deny"] : []), "approve once"),
  "DockPermission.answered": perm((p, on) => (p.answered = on ? "deny" : null), "shell denied"),
  // Answered for a call in the transcript, the request draws nothing of its own: the call's row tells it.
  "DockPermission.tool": dock((d, on) => {
    d.transcript = [
      { item: "tool", id: "t1", title: "cargo test", kind: "shell", state: "done", open: false },
      { item: "permission", id: "p1", kind: "shell", target: "PERMTARGET", actions: ["approve_once", "deny"], answered: "approve_once", tool: on ? "t1" : null },
    ];
  }),

  "DockQuestion.questions": question((q, a, on) => (q.questions = on ? [a, { ...a, id: "1", text: "LATERASK" }] : [a]), "then: LATERASK"),
  "DockQuestion.state": question((q, _a, on) => (q.state = pick(on, "waiting", "declined")), "→ declined"),
  "DockAsk.header": question((_q, a, on) => (a.header = pick(on, null, "ASKHEAD")), "ASKHEAD: ASKTEXT"),
  "DockAsk.text": question((_q, a, on) => (a.text = pick(on, "plain", "ASKTXT2")), "ASKTXT2"),
  "DockAsk.choices": question((_q, a, on) => (a.choices = on ? [{ label: "CHOSEN1" }] : []), "1 CHOSEN1"),
  "DockAsk.free_text": question((_q, a, on) => (a.free_text = on), "or type your own answer"),
  "DockAsk.multiple": question((_q, a, on) => (a.multiple = on), "⏎ submit"),
  "DockAsk.answered": question((q, a, on) => (q.questions = [{ ...a, answered: on, answer: on ? ["ANSWERED1"] : [] }, { ...a, id: "1", text: "NEXTASK" }]), "→ ANSWERED1"),
  "DockAsk.answer": question((q, a, on) => ((q.state = "answered"), (a.answered = true), (a.answer = on ? ["ANSWERTXT"] : [])), "→ ANSWERTXT"),
  "DockAskChoice.label": question((_q, a, on) => (a.choices[0].label = pick(on, "plain", "CHOICELBL")), "1 CHOICELBL"),
  "DockAskChoice.detail": question((_q, a, on) => (a.choices[0].detail = pick(on, "", "CHOICEDTL")), "CHOICEDTL"),

  // ---- the composer ------------------------------------------------------------------------------------------
  "DockComposer.draft": dock((d, on) => (d.composer = { draft: pick(on, "", "DRAFTTXT"), can_send: true, can_stop: false }), "DRAFTTXT"),
  "DockComposer.commands": cmds((d, on) => (d.composer = { draft: "/m", commands: on ? [{ name: "model", detail: "pick a model" }] : [], can_send: true, can_stop: false }), "pick a model"),
  "DockCommand.name": cmds((d, on) => (d.composer = { draft: "/m", commands: [{ name: pick(on, "other", "mzname"), detail: "d" }], can_send: true, can_stop: false }), "/mzname"),
  "DockCommand.detail": cmds((d, on) => (d.composer = { draft: "/m", commands: [{ name: "model", detail: pick(on, "", "CMDDETAIL") }], can_send: true, can_stop: false }), "CMDDETAIL"),
  "DockComposer.can_stop": dock((d, on) => (d.composer = { can_send: false, can_stop: on }), "esc stop"),
  "DockComposer.disabled": dock((d, on) => (d.composer = { can_send: false, can_stop: false, disabled: on ? "OFFTXT" : null }), "OFFTXT"),

  // ---- the picker ---------------------------------------------------------------------------------------------
  "DockPicker.kind": picker((p, on) => (p.kind = pick(on, "model", "session")), "session ·"),
  "DockPicker.choices": picker((p, on) => (p.choices = on ? [{ id: "m1", label: "CHOICETXT", current: false }] : []), "CHOICETXT"),
  "DockPicker.reading": picker((p, on) => (p.reading = on), "reading what the CLI offers"),
  "DockChoice.label": picker((p, on) => (p.choices[0].label = pick(on, "plain", "CHOICELABEL")), "CHOICELABEL"),
  "DockChoice.detail": picker((p, on) => (p.choices[0].detail = pick(on, "", "CHOICEDETAIL")), "CHOICEDETAIL"),
  "DockChoice.current": picker((p, on) => (p.choices[0].current = on), "• gpt-5"),

  // ---- the status line ----------------------------------------------------------------------------------------
  "DockStatus.provider": dock((d, on) => (d.status = { provider: pick(on, "", "STPROV") }), "STPROV"),
  "DockStatus.model": dock((d, on) => (d.status = { model: pick(on, "", "STMODEL") }), "STMODEL"),
  "DockStatus.agent": dock((d, on) => (d.status = { agent: pick(on, "", "STAGENT") }), "as STAGENT"),
  "DockStatus.cwd": dock((d, on) => (d.status = { cwd: pick(on, "", "STCWD") }), "STCWD"),
  "DockStatus.branch": dock((d, on) => (d.status = { branch: pick(on, null, "STBRANCH") }), "STBRANCH"),
  "DockStatus.usage": dock((d, on) => (d.status = { usage: pick(on, null, "STUSAGE") }), "STUSAGE"),

  // ---- leaving agents running: the activity line, tab states, full height ------------------------------------
  "DockView.activity": dock((d, on) => (d.activity = on ? { doing: "thinking", elapsed: 3 } : null), "thinking · 3s"),
  "DockView.full": dock((d, on) => (d.full = on)),
  "DockActivity.doing": dock((d, on) => (d.activity = { doing: pick(on, "thinking", "writing"), elapsed: 3 }), "writing · 3s"),
  "DockActivity.detail": dock((d, on) => (d.activity = { doing: "running", detail: pick(on, "", "ACTDETAIL"), elapsed: 3 }), "running ACTDETAIL"),
  "DockActivity.elapsed": dock((d, on) => (d.activity = { doing: "writing", elapsed: pick(on, 3, 72) }), "1m 12s"),
  "DockActivity.tokens": dock((d, on) => (d.activity = { doing: "writing", elapsed: 3, tokens: on ? 3400 : null }), "3.4k tokens"),
  "DockActivity.queued": dock((d, on) => (d.activity = { doing: "writing", elapsed: 3, queued: pick(on, 0, 2) }), "2 queued"),
  "DockTab.unread": dock((d, on) => (d.tabs = [tabOf({ unread: on })])),
  "DockTab.running": dock((d, on) => (d.tabs = [tabOf({ state: "streaming", running: pick(on, 0, 1) })])),
};

/** Fields the dock carries for the host or the keys, not for the eye, each with its reason. */
export const DOCK_INVISIBLE: Record<string, string> = {
  "DockTab.id": "pointer target (dock:tab:<id>) and what DockView.active names; sent back, never drawn",
  "DockItem.user.id": "identity of the row for the host's cursor; never drawn",
  "DockItem.assistant.id": "identity of the row for the host's cursor; never drawn",
  "DockItem.reasoning.id": "the fold click's target (dock:fold:<id>:<open>); sent back, never drawn",
  "DockItem.tool.id": "the fold click's target (dock:fold:<id>:<open>); sent back, never drawn",
  "DockItem.error.id": "identity of the row for the host's cursor; never drawn",
  "DockItem.notice.id": "identity of the row for the host's cursor; never drawn",
  "DockPermission.id": "what approve/deny answers with (dock intents, test/dock/composer.test.ts); never drawn",
  "DockQuestion.id": "what answer/decline carry back, and the buttons' targets (dock:choose:<id>:…); never drawn",
  "DockAsk.id": "which question an answer is for (DockIntent::Answer.question, test/dock/composer.test.ts); never drawn",
  "DockPermission.diff": "drawn only once the user opens it with v (Ink-side DockLocal.diffs, so not a function of the view); test/dock/rows.test.ts and composer.test.ts cover both halves",
  "DockComposer.history": "read by the editor's ↑/↓ walk through what was sent (dockKey in src/dock/composer.ts, test/dock/composer.test.ts); never drawn as such",
  "DockComposer.can_send": "gates ⏎ in the editor (dockKey: a send is refused when false); no cell of its own",
  "DockChoice.id": "what pick_model / pick_session answers with; never drawn",
  "TerminalView.id": "identity of the terminal for the host; never drawn",
  "TerminalView.socket": "the Unix socket the terminal surface connects to; its stream is drawn by TerminalSurface (test/terminal)",
  "TerminalView.columns": "the size the host made the terminal for; the surface resizes to the room the dock gives it (TerminalView.tsx), test/terminal",
  "TerminalView.rows": "the size the host made the terminal for; the surface resizes to the room the dock gives it (TerminalView.tsx), test/terminal",
  "DockView.attention": "the tab the dock key goes to from an empty composer (dockKey, test/dock/composer.test.ts); the status line's chip (StatusLine.attention) draws it",
  "TerminalState.exited.code": "told by the surface's own ended row from the socket's exit frame (endedText in src/terminal/TerminalView.tsx); the dock names only the state",
};

/** Fields the renderer does not draw today (asserted to stay undrawn; the case fails with "now drawn" when one is). */
export const DOCK_KNOWN_UNDRAWN: Record<string, string> = {
  "DockTab.provider": "the tab shows the host's icon and title; the provider name is not drawn (the status line has DockStatus.provider)",
  "TerminalView.cwd": "the terminal tab's fallback line names the command and state; the working directory is not drawn",
};
