import { afterEach, describe, expect, it } from "vitest";
import { mount, tick, until } from "./harness.js";
import { booksChrome, settingsNav } from "../chrome/fixture.js";

// A field a settings section asks for (radar-sessions' "always allow"): the host sends it as an input request, then
// every snapshot carries it as chrome.menu.typed until ⏎ or esc closes it. Once a snapshot has no typed menu it must
// be gone from the screen and give the keys back (it stayed drawn and took every key: Dawid, 2026-10-14).

let live: Array<() => void> = [];
afterEach(() => {
  live.forEach((f) => f());
  live = [];
});

const W = 160;
const H = 40;
const TITLE = "Always allow, as kind(argument)";

const section = (allowed: string) => ({
  scope: "agents",
  detail_title: "always allow",
  detail: [],
  actions: [],
  body: {
    node: "list",
    selected: 1,
    rows: [
      { target: "def:local_only", text: [{ text: "local only            yes" }] },
      { target: "def:allow", text: [{ text: `always allow          ${allowed}` }] },
      { target: "def:deny", text: [{ text: "always deny           none" }] },
    ],
  },
});

const typingHints = [
  { keys: "⏎", label: "save", keys_of: [{ label: "⏎", chord: { key: "enter" }, target: "chrome:key:0:0" }] },
  { keys: "esc", label: "cancel", keys_of: [{ label: "esc", chord: { key: "esc" }, target: "chrome:key:1:0" }] },
];

const chrome = (typed: string | null) => {
  const base = booksChrome({ width: W, height: H });
  return {
    ...base,
    settings: settingsNav,
    status: typed === null ? base.status : { ...base.status, scope: "SETTINGS", hints: typingHints },
    menu: typed === null ? null : { id: "agents.allow", title: TITLE, question: false, items: [], typed, caret: [...typed].length },
  };
};

let cursor = 0;
function show(m: ReturnType<typeof mount>, typed: string | null, allowed = "none") {
  cursor += 1;
  m.snapshot({ chrome: chrome(typed), section: section(allowed), screen: null, focus: "middle" }, cursor);
}

async function opened() {
  const m = mount(W, H);
  live.push(m.stop);
  m.welcome();
  cursor = 0;
  show(m, null);
  await tick(40);
  // ⏎ on the row: the host asks for the field, then its snapshot carries it.
  cursor += 1;
  m.conn.receive({ type: "input_request", id: 1, cursor, title: TITLE, value: "" });
  show(m, "");
  await until(() => m.frame().includes(TITLE));
  return m;
}

const line = (m: ReturnType<typeof mount>, word: string) => m.frame().split("\n").find((l) => l.includes(word)) ?? "";

describe("a typed settings field", () => {
  it("draws its prompt, a field with a caret and how to finish it", async () => {
    const m = await opened();
    const rows = m.frame().split("\n");
    const top = rows.findIndex((l) => l.includes(`╭ ${TITLE} ─`));
    expect(top).toBeGreaterThan(0);
    // the reference's field (input::render_field): blank, the caret a reversed cell, not a bar glyph.
    expect(rows[top + 1]).toMatch(/│ +│/);
    expect(rows.join("\n")).not.toContain("▏");
    expect(line(m, "save")).toMatch(/⏎\s+save\s+esc\s+cancel/);
  });

  it("takes typing, ⏎ saves it and the field is gone with the row reading it; the keys are the section's again", async () => {
    const m = await opened();
    m.type("s");
    await until(() => m.intents().length >= 1);
    expect(m.intents()[0]!.intent).toEqual({ intent: "key", chord: { key: { char: "s" } } });
    show(m, "shell(ls)");
    await until(() => m.frame().split("\n").some((l) => /│ shell\(ls\) +│/.test(l)));

    m.type("\r");
    await until(() => m.intents().length >= 2);
    expect(m.intents()[1]!.intent).toEqual({ intent: "key", chord: { key: "enter" } });
    show(m, null, "shell(ls)");
    await until(() => !m.frame().includes(TITLE));
    expect(line(m, "always allow")).toContain("always allow          shell(ls)");
    expect(m.session.state.inputRequest).toBeUndefined();

    // Not captured any more: ↓ moves the section's selection.
    m.type("\x1b[B");
    await until(() => m.intents().length >= 3);
    expect(m.intents()[2]!.intent).toEqual({ intent: "select", target: "def:deny" });
  });

  it("esc cancels: the field is gone and nothing captures the keys", async () => {
    const m = await opened();
    m.type("\x1b");
    await until(() => m.intents().length >= 1);
    expect(m.intents()[0]!.intent).toEqual({ intent: "key", chord: { key: "esc" } });
    show(m, null);
    await until(() => !m.frame().includes(TITLE));
    m.type("\x1b[A");
    await until(() => m.intents().length >= 2);
    expect(m.intents()[1]!.intent).toEqual({ intent: "select", target: "def:local_only" });
  });

  it("a press away from it closes it; a press on its field does not", async () => {
    const m = await opened();
    const rows = m.frame().split("\n");
    const top = rows.findIndex((l) => l.includes(TITLE));
    const fieldX = rows[top]!.indexOf(`╭ ${TITLE}`) + 3;
    m.type(`\x1b[<0;${fieldX + 1};${top + 2}M\x1b[<0;${fieldX + 1};${top + 2}m`);
    await tick(40);
    expect(m.intents()).toEqual([]);
    m.type(`\x1b[<0;3;${H - 4}M\x1b[<0;3;${H - 4}m`);
    await until(() => m.intents().length >= 1);
    expect(m.intents().map((i) => i.intent)).toEqual([{ intent: "key", chord: { key: "esc" } }]);
  });
});
