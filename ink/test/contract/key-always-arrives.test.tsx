// Class C: a key always arrives. In every transient state the app has - a notice (the restart notice
// is one), an open menu, a question, a text field, the History overlay, the moments right after a
// (re)connect - a key the view binds becomes its intent. A state that consumes a key on purpose is
// listed in CONSUMES with the reason, and the test checks the key reaches its intent once the state is gone.
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { chromeOf, shortList, mount, tick, until, unmountAll, type Mounted } from "./support.js";

afterEach(unmountAll);

// The size under test; every state runs at both (set by the describe.each below before each test).
const SIZES: Array<[number, number]> = [
  [80, 24],
  [160, 40],
];
let W = 80;
let H = 24;
const DOWN = "\x1b[B";
const ENTER = "\r";

/** Write `bytes` and return the intents it produced (none after 120 ms). */
async function press(m: Mounted, bytes: string, expectCount = 1) {
  const before = m.intents().length;
  m.type(bytes);
  await until(() => m.intents().length >= before + expectCount, 120);
  await tick(10);
  return m.intents().slice(before);
}

const menuOf = (over: Record<string, unknown> = {}) => ({
  id: "actions",
  title: "Pick one",
  question: false,
  items: [
    { value: "a1", label: "alpha", hint: "", key: "a", target: "chrome:menu:0" },
    { value: "b2", label: "beta", hint: "", key: "b", target: "chrome:menu:1" },
  ],
  selected: 0,
  ...over,
});
const historyOf = () => ({
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
  rows: [{ observed: "2026-10-12", action: "act.one", subject: "x", outcome: "ok", outcome_ink: "muted", target: "chrome:history:row:0" }],
  selected: 0,
  list_scroll: 0,
  state: "",
  detail_title: "detail",
  detail: [],
  detail_scroll: 0,
  help_title: "keys",
  help_lines: [],
  help_scroll: 0,
  footer: [],
});

/** The body keys (↓ selects the next row, ⏎ activates the selected one) on a list of four rows. */
const BODY_DOWN = { intent: "select", target: "row:!2" };
const BODY_ENTER = { intent: "activate", target: "row:!1" };

const fresh = (extra: Record<string, unknown> = {}) => {
  const m = mount(W, H);
  m.snapshot({ chrome: chromeOf(W, H, extra), screen: shortList(4) });
  return m;
};

describe.each(SIZES)("at %ix%i", (w, h) => {
beforeEach(() => {
  W = w;
  H = h;
});

describe("a key the view binds arrives in every state the app has", () => {
  it("in a plain list (the baseline)", async () => {
    const m = fresh();
    await m.settle();
    expect(await press(m, DOWN)).toEqual([BODY_DOWN]);
    expect(await press(m, ENTER)).toEqual([{ intent: "activate", target: "row:!2" }]);
  });

  it("under a notice: the key is the intent it would be, and the notice goes", async () => {
    const m = fresh();
    m.conn.receive({ type: "notice", text: "the renderer was restarted" });
    await m.settle();
    expect(m.frame()).toContain("the renderer was restarted");
    expect(await press(m, DOWN)).toEqual([BODY_DOWN]);
    expect(m.frame()).not.toContain("the renderer was restarted");
    expect(await press(m, ENTER)).toEqual([{ intent: "activate", target: "row:!2" }]);
  });

  it("under several notices: the first key reaches its intent, all of them go", async () => {
    const m = fresh();
    for (const t of ["one", "two", "three"]) m.conn.receive({ type: "notice", text: `notice ${t}` });
    await m.settle();
    expect(await press(m, ENTER)).toEqual([BODY_ENTER]);
    expect(m.session.state.notices).toEqual([]);
  });

  it("right after a restart: the first key after Welcome and the first Snapshot, with the restart notice up", async () => {
    // What the host sends a new renderer process: Welcome, the newest Snapshot, then its notice.
    const m = mount(W, H);
    m.snapshot({ chrome: chromeOf(W, H), screen: shortList(4) });
    m.conn.receive({ type: "notice", text: "renderer restarted" });
    await m.settle(); // the frame the user sees is drawn; their first key follows it
    expect(await press(m, DOWN)).toEqual([BODY_DOWN]);
  });

  it("right after a reconnect: a key before the first Snapshot is still sent (as a key; the host decides)", async () => {
    const m = mount(W, H);
    expect(await press(m, "j")).toEqual([{ intent: "key", chord: { key: { char: "j" } } }]);
  });

  it("with a menu open: a menu key invokes, an arrow and ⏎ reach the host (each in a menu of its own: an answer closes it)", async () => {
    const press1 = async (bytes: string) => {
      const m = fresh({ menu: menuOf() });
      await m.settle();
      return press(m, bytes);
    };
    expect(await press1("b")).toEqual([{ intent: "invoke", action: "b2" }]);
    expect(await press1(DOWN)).toEqual([{ intent: "key", chord: { key: "down" } }]);
    expect(await press1(ENTER)).toEqual([{ intent: "invoke", action: "a1" }]);
  });

  it("with a menu open under a notice: the menu key still invokes", async () => {
    const m = fresh({ menu: menuOf() });
    m.conn.receive({ type: "notice", text: "renderer restarted" });
    await m.settle();
    expect(await press(m, "a")).toEqual([{ intent: "invoke", action: "a1" }]);
  });

  it("with a question asked (a dialog): its key answers it", async () => {
    const m = fresh();
    m.conn.receive({
      type: "question",
      id: 1,
      cursor: 1,
      menu: menuOf({
        id: "confirm",
        title: "post comment?",
        question: true,
        items: [
          { value: "yes", label: "post it", hint: "", key: "y", target: "chrome:menu:0" },
          { value: "no", label: "drop it", hint: "", key: "n", target: "chrome:menu:1" },
        ],
      }),
    });
    await m.settle();
    expect(m.frame()).toContain("post comment?");
    expect(await press(m, "y")).toEqual([{ intent: "invoke", action: "yes" }]);
    // The question is answered: the next key is the body's again.
    await m.settle();
    expect(await press(m, DOWN)).toEqual([BODY_DOWN]);
  });

  it("with a question asked and Esc: the host is told, and the body has its keys back", async () => {
    const m = fresh();
    m.conn.receive({ type: "question", id: 1, cursor: 1, menu: menuOf({ id: "confirm", question: true }) });
    await m.settle();
    expect(await press(m, "\x1b")).toEqual([{ intent: "key", chord: { key: "esc" } }]);
    await m.settle();
    expect(await press(m, DOWN)).toEqual([BODY_DOWN]);
  });

  it("with a text field in a menu: every key goes to the host as a key", async () => {
    const m = fresh({ menu: menuOf({ typed: "ab", caret: 2 }) });
    await m.settle();
    expect(await press(m, "j")).toEqual([{ intent: "key", chord: { key: { char: "j" } } }]);
    expect(await press(m, DOWN)).toEqual([{ intent: "key", chord: { key: "down" } }]);
    expect(await press(m, ENTER)).toEqual([{ intent: "key", chord: { key: "enter" } }]);
  });

  it("with a pending input request: every key goes to the host as a key", async () => {
    const m = fresh();
    m.conn.receive({ type: "input_request", id: 1, cursor: 1, title: "branch name", value: "" });
    await m.settle();
    expect(await press(m, "x")).toEqual([{ intent: "key", chord: { key: { char: "x" } } }]);
    expect(await press(m, ENTER)).toEqual([{ intent: "key", chord: { key: "enter" } }]);
  });

  it("over the History overlay: j, ↓ and Esc reach the host as keys (it owns the overlay's navigation)", async () => {
    const m = fresh({ history: historyOf() });
    await m.settle();
    expect(await press(m, "j")).toEqual([{ intent: "key", chord: { key: { char: "j" } } }]);
    expect(await press(m, DOWN)).toEqual([{ intent: "key", chord: { key: "down" } }]);
    expect(await press(m, "\x1b")).toEqual([{ intent: "key", chord: { key: "esc" } }]);
  });

  it("over the History overlay under a notice: the key reaches the host", async () => {
    const m = fresh({ history: historyOf() });
    m.conn.receive({ type: "notice", text: "renderer restarted" });
    await m.settle();
    expect(await press(m, "j")).toEqual([{ intent: "key", chord: { key: { char: "j" } } }]);
  });

  it("with the bar holding the keys (keys_at bar): the host moves, so the key is sent whole", async () => {
    const m = fresh({ keys_at: "bar" });
    await m.settle();
    expect(await press(m, DOWN)).toEqual([{ intent: "key", chord: { key: "down" } }]);
  });

  it("a new snapshot without the menu gives the keys back to the body", async () => {
    const m = fresh({ menu: menuOf() });
    await m.settle();
    m.snapshot({ chrome: chromeOf(W, H), screen: shortList(4) });
    await m.settle();
    expect(await press(m, DOWN)).toEqual([BODY_DOWN]);
  });
});

/**
 * Input a state takes for itself on purpose: the state, what it takes and why. Each is checked, and the
 * same input is checked to do its work once the state is gone.
 */
describe("a state that consumes input on purpose says so, and the input works again after it", () => {
  const rowPress = (m: Mounted) => {
    const lines = m.lines();
    const y = lines.findIndex((l) => l.includes("!3"));
    return `\x1b[<0;${lines[y]!.indexOf("!3") + 2};${y + 1}M`;
  };

  it("an open menu takes a press away from it (it closes it: Esc to the host), a press on the top bar still acts", async () => {
    const m = fresh({ menu: menuOf() });
    await m.settle();
    expect(await press(m, rowPress(m))).toEqual([{ intent: "key", chord: { key: "esc" } }]);
    const bar = m.lines()[0]!;
    expect(await press(m, `\x1b[<0;${bar.indexOf("Work") + 2};1M`)).toEqual([{ intent: "activate", target: "chrome:tab:1" }]);
  });

  it("while text is being typed a press away from the field does not close it", async () => {
    const m = fresh({ menu: menuOf({ typed: "ab", caret: 2 }) });
    await m.settle();
    expect(await press(m, `\x1b[<0;2;${H - 2}M`)).toEqual([]);
  });

  // Live defect (owner: T11, nodes; T10 for the page): no node component registers its rows with the
  // HitRegistry (nothing calls registry.add), so a press on a list row sends nothing. Once rows are
  // pointable this goes red: remove `.fails`.
  it("once the menu is gone, a press on a list row selects it (KNOWN DEFECT: body rows are not pointable)", async () => {
    const m = fresh();
    await m.settle();
    expect(await press(m, rowPress(m))).toEqual([{ intent: "select", target: "row:!3" }]);
  });
});

// ---- the keys come from what the view declares, not from a pick of ours -------------------------------------------

/** What a terminal sends for a chord (named keys, ctrl-<letter> as a control code, alt-<char> as ESC <char>). */
const NAMED_BYTES: Record<string, string> = {
  enter: "\r",
  esc: "\x1b",
  tab: "\t",
  back_tab: "\x1b[Z",
  up: "\x1b[A",
  down: "\x1b[B",
  right: "\x1b[C",
  left: "\x1b[D",
  page_up: "\x1b[5~",
  page_down: "\x1b[6~",
  home: "\x1b[H",
  end: "\x1b[F",
  backspace: "\x7f",
  delete: "\x1b[3~",
};
const bytesOf = (chord: any): string => {
  if (typeof chord.key === "string") return NAMED_BYTES[chord.key]!;
  const c: string = chord.key.char;
  if (chord.ctrl) return String.fromCharCode(c.charCodeAt(0) & 0x1f);
  return chord.alt ? `\x1b${c}` : c;
};
const nameOf = (chord: any) => JSON.stringify(chord);

/** A menu item's `keys` string (`d`, `⏎`, `esc`, `ctrl-r`, `space`) as a chord: the table the menu itself shows. */
const chordOfKeys = (keys: string): any => {
  const ctrl = keys.startsWith("ctrl-");
  const k = ctrl ? keys.slice(5) : keys;
  const named: Record<string, string> = { "⏎": "enter", enter: "enter", esc: "esc", tab: "tab", "↑": "up", "↓": "down", "←": "left", "→": "right", pgup: "page_up", pgdn: "page_down" };
  const key = named[k] ?? { char: k === "space" ? " " : k };
  return ctrl ? { key, ctrl: true } : { key };
};

/**
 * The view this file's scenes declare keys in: the status line's `keys_of` chords (a letter, a ctrl chord, an alt
 * chord and the named keys), the screen's `actions[].key` chords, and a menu's item keys. Every key is read back
 * from the view, so adding one to the view tests it in every state.
 */
const declaredView = () => {
  const hint = (label: string, chord: any, i: number) => ({ keys: label, label: `act${i}`, keys_of: [{ label, chord, action: `act${i}`, target: `chrome:key:${i}:0` }] });
  const chords: any[] = [
    { key: { char: "q" } },
    { key: { char: "r" }, ctrl: true },
    { key: { char: "x" }, alt: true },
    { key: "tab" },
    { key: "esc" },
    { key: "enter" },
    { key: "down" },
  ];
  const chrome = (over: Record<string, unknown> = {}) => {
    const c = chromeOf(W, H, over);
    c.status.hints = chords.slice(0, 4).map((ch, i) => hint(`k${i}`, ch, i));
    return c;
  };
  const screen = shortList(4);
  screen.actions = [
    { id: "a.pick", label: "pick", key: { key: { char: "p" } } },
    { id: "a.next", label: "next", key: chords[4] },
    { id: "a.go", label: "go", key: chords[5] },
    { id: "a.down", label: "down", key: chords[6] },
  ];
  const fromView = (c: any, s: any) => [...c.status.hints.flatMap((h: any) => (h.keys_of ?? []).map((k: any) => k.chord)), ...s.actions.map((a: any) => a.key)];
  return { chrome, screen, chords: fromView(chrome(), screen) };
};

describe("every key the view declares arrives as its intent, in each state", () => {
  const view = declaredView();
  const open = (over: Record<string, unknown> = {}) => {
    const m = mount(W, H);
    m.snapshot({ chrome: view.chrome(over), screen: structuredClone(view.screen) });
    return m;
  };
  const key = (chord: any) => [{ intent: "key", chord }];
  /** In the body a list selects with ↓ and activates with ⏎; everything else is the host's key. */
  const body = (chord: any) => (chord.key === "down" ? [BODY_DOWN] : chord.key === "enter" ? [BODY_ENTER] : key(chord));
  /** Over a menu or question ⏎ takes the highlighted row; every other chord is the host's key. */
  const menu = (chord: any) => (chord.key === "enter" ? [{ intent: "invoke", action: "a1" }] : key(chord));

  const STATES: Array<{ name: string; setup: () => Mounted; expect: (chord: any) => any[]; menuKeys?: boolean }> = [
    { name: "a list", setup: () => open(), expect: body },
    {
      name: "a notice (the restart notice)",
      setup: () => {
        const m = open();
        m.conn.receive({ type: "notice", text: "renderer restarted" });
        return m;
      },
      expect: body,
    },
    { name: "the bar holding the keys", setup: () => open({ keys_at: "bar" }), expect: key },
    { name: "the History overlay", setup: () => open({ history: historyOf() }), expect: key },
    { name: "an open menu", setup: () => open({ menu: menuOf() }), expect: menu, menuKeys: true },
    {
      name: "a question",
      setup: () => {
        const m = open();
        m.conn.receive({ type: "question", id: 1, cursor: 1, menu: menuOf({ id: "confirm", title: "post it?", question: true }) });
        return m;
      },
      expect: menu,
      menuKeys: true,
    },
    { name: "a text field", setup: () => open({ menu: menuOf({ typed: "ab", caret: 2 }) }), expect: key },
    {
      name: "an input request",
      setup: () => {
        const m = open();
        m.conn.receive({ type: "input_request", id: 1, cursor: 1, title: "branch name", value: "" });
        return m;
      },
      expect: key,
    },
  ];

  it("the view declares letters, ctrl, alt and named keys (so the cases below cover each kind)", () => {
    expect(view.chords.some((c: any) => c.ctrl)).toBe(true);
    expect(view.chords.some((c: any) => c.alt)).toBe(true);
    expect(view.chords.some((c: any) => typeof c.key === "string")).toBe(true);
    expect(view.chords.some((c: any) => typeof c.key === "object" && !c.ctrl && !c.alt)).toBe(true);
  });

  for (const st of STATES) {
    it(`${st.name}: every declared chord reaches its intent`, async () => {
      for (const chord of view.chords) {
        const m = st.setup();
        await m.settle();
        expect(await press(m, bytesOf(chord)), `${nameOf(chord)} in ${st.name}`).toEqual(st.expect(chord));
        m.stop();
      }
    });
    if (st.menuKeys) {
      it(`${st.name}: each key a menu item declares invokes that item`, async () => {
        for (const item of menuOf().items) {
          const m = st.setup();
          await m.settle();
          expect(await press(m, bytesOf(chordOfKeys(item.key))), `menu key ${item.key} in ${st.name}`).toEqual([{ intent: "invoke", action: item.value }]);
          m.stop();
        }
      });
    }
  }

  it("the first key right after a restart (a new app: Welcome, the first Snapshot, its notice) is not lost", async () => {
    for (const chord of view.chords) {
      const m = open();
      m.conn.receive({ type: "notice", text: "renderer restarted" });
      await m.settle();
      expect(await press(m, bytesOf(chord)), nameOf(chord)).toEqual(body(chord));
      m.stop();
    }
  });
});

});
