// Class B: how the terminal chunks the input must not matter. A paste, a held key, a slow link or a
// test that writes "\x1b[B\x1b[B\r" in one go all deliver several keys in one read; the intents must be
// exactly those the same keys give one at a time, in the same order.
//
// The sequences are a table of key names; each runs in three states (a list, an open menu, the History
// overlay) once as one write per key and once as a single write of the whole sequence.
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { chromeOf, shortList, mount, tick, until, type Mounted } from "./support.js";


const SIZES: Array<[number, number]> = [
  [80, 24],
  [160, 40],
];

/** What a terminal sends for each key. A function is given the app and finds a cell to point at. */
const KEYS: Record<string, string | ((m: Mounted) => string)> = {
  up: "\x1b[A",
  down: "\x1b[B",
  right: "\x1b[C",
  left: "\x1b[D",
  enter: "\r",
  esc: "\x1b",
  tab: "\t",
  backtab: "\x1b[Z",
  pgup: "\x1b[5~",
  pgdn: "\x1b[6~",
  home: "\x1b[H",
  end: "\x1b[F",
  backspace: "\x7f",
  delete: "\x1b[3~",
  space: " ",
  j: "j",
  k: "k",
  x: "x",
  o: "o",
  G: "G",
  "ctrl-r": "\x12",
  // the pointer: a left press (and its release), the wheel, a right press, on a row of what shows
  click2: (m) => click(m, "!2", 0),
  click3: (m) => click(m, "!3", 0) + release(m, "!3"),
  rightclick: (m) => click(m, "!3", 2),
  clicktab: (m) => `\x1b[<0;${m.lines()[0]!.indexOf("Work") + 2};1M`,
  wheeldown: (m) => `\x1b[<65;5;5M`,
  wheelup: (m) => `\x1b[<64;5;5M`,
};

const cellOf = (m: Mounted, word: string) => {
  const lines = m.lines();
  const y = lines.findIndex((l, i) => i > 0 && i < lines.length - 1 && l.includes(word));
  return { x: (y >= 0 ? lines[y]!.indexOf(word) : 0) + 2, y: y + 1 };
};
const click = (m: Mounted, word: string, button: number) => {
  const { x, y } = cellOf(m, word);
  return `\x1b[<${button};${x};${y}M`;
};
const release = (m: Mounted, word: string) => {
  const { x, y } = cellOf(m, word);
  return `\x1b[<0;${x};${y}m`;
};

const SEQUENCES: string[][] = [
  ["down", "down", "enter"],
  ["up", "up", "enter"],
  ["down", "up", "down", "enter"],
  ["j", "j", "k", "enter"],
  ["down", "right", "left", "up"],
  ["tab", "backtab", "tab"],
  ["pgdn", "pgup", "end", "home"],
  ["G", "g" in KEYS ? "g" : "j", "x"],
  ["x", "o", "space", "ctrl-r"],
  ["backspace", "delete", "enter"],
  ["down", "esc"],
  ["j", "click3", "j"],
  ["down", "clicktab", "down", "enter"],
  ["click2", "enter"],
  ["down", "click2", "down", "enter"],
  ["wheeldown", "wheeldown", "wheelup", "enter"],
  ["rightclick", "down"],
  ["click2", "click3", "enter"],
  ["down", "down", "down", "down", "down", "enter"],
];

// ---- the three states -----------------------------------------------------------------------------------------

const menu = {
  id: "actions",
  title: "Pick one",
  question: false,
  items: [
    { value: "a1", label: "alpha", hint: "", key: "a", target: "chrome:menu:0" },
    { value: "b2", label: "beta", hint: "", key: "b", target: "chrome:menu:1" },
    { value: "c3", label: "gamma", hint: "", key: "c", target: "chrome:menu:2" },
  ],
  selected: 0,
};
const questionMenu = { ...structuredClone(menu), id: "confirm", title: "post it?", question: true };
const historyOf =(W: number, H: number) => ({
  version: 1,
  width: W,
  height: H - 1,
  title: "History",
  boundary: "snapshot through #1",
  scope: "recent",
  notes: [],
  focus: "events",
  events_title: "events",
  columns: ["observed UTC", "action", "subject", "outcome"],
  rows: ["!1", "!2", "!3"].map((s, i) => ({ observed: "2026-10-12", action: s, subject: "x", outcome: "ok", outcome_ink: "muted", target: `chrome:history:row:${i}` })),
  selected: 0,
  list_scroll: 0,
  state: "",
  detail_title: "detail",
  detail: [],
  detail_scroll: 0,
  help_title: "keys",
  help_lines: [],
  help_scroll: 0,
  footer: [{ keys: "esc", label: "close", keys_of: [{ label: "esc", chord: { key: "esc" }, target: "chrome:history:key:esc" }] }],
});
/**
 * The host's side of the conversation, as little as the sequences need: Select moves the list's selection,
 * Up/Down (and j/k) move a menu's. One key at a time the host has answered each key with a new snapshot
 * before the next arrives; in one read it has not, and the renderer must route the keys from where the
 * earlier ones went (the reference applies each key before the next).
 */
type Model = { sel: number; menuSel: number };
const STATES: Record<string, (md: Model, W: number, H: number) => { chrome: any; screen: any }> = {
  "a list": (md, W, H) => ({ chrome: chromeOf(W, H), screen: withSelected(shortList(4), md.sel) }),
  "an open menu": (md, W, H) => ({ chrome: chromeOf(W, H, structuredClone({ menu: { ...menu, selected: md.menuSel } })), screen: shortList(4) }),
  "the History overlay": (_md, W, H) => ({ chrome: chromeOf(W, H, { history: historyOf(W, H) }), screen: shortList(4) }),
  // the session's own transient states sit over the same list: what the host sends outside a snapshot is in EXTRA
  "a notice": (md, W, H) => ({ chrome: chromeOf(W, H), screen: withSelected(shortList(4), md.sel) }),
  // The host's snapshot carries the menu it asked (bridge.rs ask_and_tell); one without it says the question is closed.
  "a question": (md, W, H) => ({ chrome: chromeOf(W, H, { menu: structuredClone({ ...questionMenu, selected: md.menuSel }) }), screen: withSelected(shortList(4), md.sel) }),
  "a text field": (_md, W, H) => ({ chrome: chromeOf(W, H, structuredClone({ menu: { ...menu, typed: "ab", caret: 2 } })), screen: shortList(4) }),
  // The host's snapshots carry the typed menu while its input request is open; one without it says it is closed.
  "an input request": (_md, W, H) => ({ chrome: chromeOf(W, H, structuredClone({ menu: { ...menu, id: "input", title: "branch name", items: [], typed: "", caret: 0 } })), screen: shortList(4) }),
};
/** Messages the host sends alongside the snapshot (a notice, a question, an input request) when the run begins. */
const EXTRA: Record<string, () => any[]> = {
  "a notice": () => [{ type: "notice", text: "the renderer was restarted" }],
  "a question": () => [{ type: "question", id: 1, cursor: 1, menu: structuredClone(questionMenu) }],
  "an input request": () => [{ type: "input_request", id: 1, cursor: 1, title: "branch name", value: "" }],
};
const withSelected = (screen: any, sel: number) => {
  screen.panes[0].body.selected = sel;
  return screen;
};
function applyHost(md: Model, intents: any[]) {
  for (const i of intents) {
    if (i.intent === "select" && /^row:!(\d+)$/.test(i.target)) md.sel = Number(/(\d+)$/.exec(i.target)![1]) - 1;
    if (i.intent === "key" && (i.chord.key === "down" || i.chord.key?.char === "j")) md.menuSel = Math.min(2, md.menuSel + 1);
    if (i.intent === "key" && (i.chord.key === "up" || i.chord.key?.char === "k")) md.menuSel = Math.max(0, md.menuSel - 1);
  }
}

/**
 * What a run of intents does to the host's list of four rows, so a Select the renderer predicted and the plain key
 * the host would have moved by itself compare equal. After a key whose effect the renderer cannot foresee (⏎ opens
 * a page, ↑ from the first row goes to the bar, → moves the focus) the rest of a read is sent as plain keys, as
 * the reference's runner sends them, so the intents differ in kind but not in what the host does with them.
 */
function effects(intents: any[]): string[] {
  const out: string[] = [];
  let sel = 0;
  const last = 3;
  for (const i of intents) {
    const k = i.intent === "key" ? i.chord : undefined;
    const name = typeof k?.key === "string" ? k.key : undefined;
    const ch = typeof k?.key === "object" ? k.key.char : undefined;
    if (i.intent === "select" && /^row:!(\d+)$/.test(i.target)) sel = Number(/(\d+)$/.exec(i.target)![1]) - 1;
    else if (i.intent === "activate" && /^row:!(\d+)$/.test(i.target)) out.push(`activate:${Number(/(\d+)$/.exec(i.target)![1]) - 1}`);
    else if (k && !k.ctrl && !k.alt && (name === "down" || ch === "j")) sel = Math.min(last, sel + 1);
    else if (k && !k.ctrl && !k.alt && (name === "up" || ch === "k")) sel = Math.max(0, sel - 1);
    else if (k && !k.ctrl && !k.alt && (name === "home" || ch === "g")) sel = 0;
    else if (k && !k.ctrl && !k.alt && name === "end") sel = last;
    else if (k && name === "enter") out.push(`activate:${sel}`);
    else out.push(JSON.stringify(i));
  }
  return [...out, `sel:${sel}`];
}

/**
 * Wait until no new intent has been sent for 40 ms (a loaded machine delivers late). A quiet window on the clock is
 * the only signal available here: what is compared is the whole set of intents, including the ones that must NOT be
 * sent, and the input parser holds a lone Esc 20 ms and delivers other bytes through timers, so "the last expected
 * intent arrived" does not say that no further one is on its way. The cost is bounded and paid in parallel (see the
 * concurrent describe below).
 */
async function quiet(m: Mounted) {
  let n = -1;
  let still = 0;
  for (let i = 0; i < 100 && still < 4; i++) {
    await tick(10);
    if (m.intents().length === n) still++;
    else still = 0;
    n = m.intents().length;
  }
}

async function run(m: Mounted, state: string, keys: string[], chunked: boolean, W: number, H: number) {
  const md: Model = { sel: 0, menuSel: 0 };
  // what the last case left in the session (an unanswered question, a notice, an input request) is not this case's
  (m.session as any).set({ questions: [], notices: [], inputRequest: undefined });
  m.snapshot(STATES[state]!(md, W, H)); // a new snapshot: the app forgets what it predicted from the last run
  for (const msg of EXTRA[state]?.() ?? []) m.conn.receive(msg);
  await m.settle();
  const before = m.intents().length;
  const bytes = keys.map((k) => (typeof KEYS[k] === "function" ? (KEYS[k] as (m: Mounted) => string)(m) : (KEYS[k] as string)));
  if (chunked) {
    m.type(bytes.join(""));
    await quiet(m);
  } else {
    for (const b of bytes) {
      const n = m.intents().length;
      m.type(b);
      await until(() => m.intents().length > n, 300);
      await quiet(m);
      applyHost(md, m.intents().slice(n));
      m.snapshot(STATES[state]!(md, W, H)); // the host's answer, before the next key
      await tick(15);
    }
    await quiet(m);
  }
  return m.intents().slice(before);
}

// Known defects: none. (A case that differs is listed here as `state|sequence`: "reason"; it runs as `it.fails`
// and goes red, saying so, once fixed.)
const KNOWN_DEFECTS: Record<string, string> = {
};

/**
 * One test per state and size, each with its own app, run concurrently with the others: every step waits for the app
 * to be quiet on the clock, so the states overlap their waits instead of adding them (the file took 140 s one case at
 * a time). Inside a test the sequences run one after another on that state's app, and the test reports every
 * sequence that differs, not only the first. Nothing here hangs: every wait is bounded. The limit is for a loaded
 * machine (load average 30 to 100 in the gate), not a measure of speed.
 */
const STATE_TIMEOUT_MS = 180_000;

describe.concurrent.each(SIZES)("keys in one read give the intents they give one by one, at %ix%i", (W, H) => {
  for (const state of Object.keys(STATES)) {
    it(state, { timeout: STATE_TIMEOUT_MS }, async () => {
      const m = mount(W, H);
      try {
        const problems: string[] = [];
        for (const seq of SEQUENCES) {
          const name = seq.join(" ");
          const known = KNOWN_DEFECTS[`${state}|${name}`];
          const one = await run(m, state, seq, false, W, H);
          const all = await run(m, state, seq, true, W, H);
          let same = true;
          try {
            expect(effects(all)).toEqual(effects(one));
          } catch {
            same = false;
          }
          if (known && same) problems.push(`fixed: remove "${state}|${name}" from KNOWN_DEFECTS`);
          if (!known && !same) {
            problems.push(`${name}: one write of the whole sequence differs from one write per key\n  one by one: ${JSON.stringify(one)}\n  one write:  ${JSON.stringify(all)}`);
          }
        }
        expect(problems.join("\n"), "sequences that differ").toBe("");
      } finally {
        m.stop();
      }
    });
  }

  it("every known defect names a case that exists", () => {
    for (const k of Object.keys(KNOWN_DEFECTS)) {
      const [state, name] = k.split("|");
      expect(Object.keys(STATES)).toContain(state);
      expect(SEQUENCES.map((s) => s.join(" "))).toContain(name);
    }
  });

  it("the sequences use only keys the table knows", () => {
    for (const s of SEQUENCES) for (const k of s) expect(KEYS, k).toHaveProperty(k);
  });
});
