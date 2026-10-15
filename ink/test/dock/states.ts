import type { DockItem, DockTab, DockView } from "../../src/dock/model.js";

/**
 * Every state the dock draws, as the host's DockView (crates/radar-core/src/dock.rs). Built here until T2's recorder writes
 * host view data into test/fixtures/dock/; the shapes are the host's, so swapping a state for a recorded one changes
 * only where it is read from.
 */

const tab = (over: Partial<DockTab> = {}): DockTab => ({ id: "s1", provider: "copilot", icon: "agent", title: "review", state: "idle", ...over });
const base = (over: Partial<DockView> = {}): DockView => ({
  visible: true,
  focused: true,
  tabs: [tab()],
  active: "s1",
  transcript: [],
  composer: { can_send: true, can_stop: false, history: ["hello"], commands: [{ name: "model", detail: "pick a model" }, { name: "clear", detail: "new context" }] },
  status: { provider: "copilot", model: "gpt-5", cwd: "~/radar", branch: "main", usage: "1.2k tokens" },
  ...over,
});

const ask: DockItem = { item: "user", id: "u1", text: "Why does the build fail on main?" };
const reply: DockItem = { item: "assistant", id: "a1", text: "Let me look at the failing job and the last commit before saying anything.", saved: true };
const toolDone: DockItem = { item: "tool", id: "t1", title: "git log -3 --oneline", kind: "shell", state: "done", open: false, result: ["f506806c Agent dock plan", "7e5c0800 Merge #30", "0bd7a64a Tint chip grounds"] };

export const STATES: Record<string, DockView> = {
  "no-sessions": base({ tabs: [], active: null, status: {}, composer: { can_send: false, can_stop: false, disabled: "no session · ctrl-t starts one" } }),
  empty: base(),
  streaming: base({
    tabs: [tab({ state: "streaming" })],
    transcript: [
      ask,
      reply,
      { item: "tool", id: "t2", title: "cargo check -p radar-core", kind: "shell", state: "running", open: false },
      { item: "assistant", id: "a2", text: "The check is running; so far it reports", saved: false },
      // Sent while the turn runs: radar keeps it until the turn ends (a provider that queues shows "sent").
      { item: "user", id: "queued-0", text: "and the clippy run?", queued: "here" },
    ],
    // A turn that runs never refuses a message: ⏎ queues it.
    composer: { can_send: true, can_stop: true },
  }),
  "tool-folded": base({ transcript: [ask, reply, toolDone, { item: "reasoning", id: "r1", text: "The last commit only touched docs.\nSo the failure is older.\nCheck CI.", open: false }] }),
  "tool-open": base({ transcript: [ask, reply, { ...toolDone, open: true } as DockItem, { item: "reasoning", id: "r1", text: "The last commit only touched docs.\nSo the failure is older.", open: true }] }),
  approval: base({
    tabs: [tab({ state: "approval" })],
    transcript: [ask, { item: "permission", id: "p1", kind: "shell", target: "cargo test -p radar-core --no-fail-fast", actions: ["approve_once", "approve_for_session", "deny", "view_diff"] }],
    composer: { can_send: true, can_stop: true },
  }),
  "approval-write": base({
    tabs: [tab({ state: "approval" })],
    transcript: [{ item: "permission", id: "p2", kind: "write", target: "crates/radar-core/src/dock.rs", diff: ["-const NARROW: u16 = 100;", "+const NARROW: u16 = 90;"], actions: ["approve_once", "deny", "view_diff"] }],
  }),
  question: base({
    tabs: [tab({ state: "approval" })],
    transcript: [
      ask,
      {
        item: "question",
        id: "q1",
        state: "waiting",
        questions: [{ id: "0", header: "Fix", text: "Which fix do you want?", choices: [{ label: "revert", detail: "undo the last commit" }, { label: "patch" }], free_text: true, multiple: false, answered: false }],
      },
    ],
    composer: { can_send: true, can_stop: true },
  }),
  "question-several": base({
    tabs: [tab({ state: "approval" })],
    transcript: [
      {
        item: "question",
        id: "q2",
        state: "waiting",
        questions: [
          { id: "Which crate?", header: "Crate", text: "Which crate?", choices: [{ label: "radar-core" }, { label: "radar-sessions" }], free_text: true, multiple: false, answered: true, answer: ["radar-core"] },
          { id: "Which checks?", header: "Checks", text: "Which checks?", choices: [{ label: "tests" }, { label: "clippy" }, { label: "fmt" }], free_text: true, multiple: true, answered: false },
          { id: "Commit?", text: "Commit?", choices: [{ label: "yes" }, { label: "no" }], free_text: true, multiple: false, answered: false },
        ],
      },
    ],
    composer: { can_send: true, can_stop: true },
  }),
  "question-answered": base({
    transcript: [ask, { item: "question", id: "q1", state: "answered", questions: [{ id: "0", text: "Which fix do you want?", choices: [{ label: "revert" }, { label: "patch" }], free_text: true, multiple: false, answered: true, answer: ["patch"] }] }],
  }),
  answered: base({ transcript: [ask, { item: "permission", id: "p1", kind: "shell", target: "rm -rf build", actions: ["approve_once", "deny"], answered: "deny" }, { item: "tool", id: "t3", title: "rm -rf build", kind: "shell", state: "denied", open: false }] }),
  error: base({ tabs: [tab({ state: "error" })], transcript: [ask, { item: "error", id: "e1", text: "connection to copilot lost; the session can be resumed" }] }),
  stopped: base({ tabs: [tab({ state: "stopped" })], transcript: [ask, reply, { item: "notice", id: "n1", text: "stopped by you" }] }),
  "two-tabs": base({
    tabs: [tab(), tab({ id: "s2", provider: "codex", icon: "agent", title: "mr-1234", state: "streaming" })],
    transcript: [ask, reply],
  }),
  pty: base({
    tabs: [tab({ id: "t1", provider: "pty", icon: "terminal", title: "codex login", terminal: { id: "t1", socket: "/tmp/x.sock", columns: 76, rows: 12, state: { state: "running" }, command: "codex login", cwd: "~/radar" } })],
    active: "t1",
  }),
  "picker-model": base({ picker: { kind: "model", choices: [{ id: "m1", label: "gpt-5", current: true }, { id: "m2", label: "claude-sonnet-4.5", detail: "premium" }, { id: "m3", label: "gemini-3" }] } }),
  "slash-commands": base({
    composer: {
      draft: "/co",
      can_send: true,
      can_stop: false,
      commands: [
        { name: "compact", detail: "[instructions]  Free up context by summarising the conversation" },
        { name: "context", detail: "Show context usage" },
        { name: "config", detail: "key=value  Change a setting" },
        { name: "model", detail: "<model>  Choose the model" },
      ],
    },
  }),
  unfocused: base({ focused: false, transcript: [ask, reply, toolDone] }),
};
