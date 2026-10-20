// Class D: an overlay is opaque. Whatever the screen under it holds, no character of it shows inside the
// overlay's rectangle (a menu, a question, a text field, a menu opened by a context click), and nothing of
// the body shows through the History overlay or settings, which replace everything under the top bar.
//
// The body here is a list whose every row is the sentinel "ZQXJ" repeated edge to edge, and no overlay text
// uses the letters Z, Q, X or J, so one sentinel letter inside an overlay's rectangle is a cell it did not
// write. The frame is the whole frame Ink painted (debug mode), so a cell an overlay left alone shows.
import { afterEach, describe, expect, it } from "vitest";
import { chromeOf, listScreen, mount, unmountAll } from "./support.js";

afterEach(unmountAll);

const SIZES: Array<[number, number]> = [
  [80, 24],
  [160, 40],
];
const SENTINEL = /[ZQXJ]/;

const choice = (value: string, label: string, i: number, extra: Record<string, unknown> = {}) => ({ value, label, hint: "", key: label[0], target: `chrome:menu:${i}`, ...extra });
const MENUS: Record<string, { title: string; menu: (w: number, h: number) => any }> = {
  "an actions menu": {
    title: "Actions for !263",
    menu: () => ({
      id: "actions",
      title: "Actions for !263",
      question: false,
      items: [choice("o", "open", 0, { hint: "in the browser" }), choice("m", "merge", 1, { hint: "when green" }), choice("c", "close", 2)],
      selected: 1,
    }),
  },
  "a question with a note and two buttons": {
    title: "post comment?",
    menu: () => ({
      id: "confirm",
      title: "post comment?",
      question: true,
      change: "add",
      items: [{ value: "", label: "This posts your comment to !263.", hint: "", note: true }, choice("yes", "post it", 1), choice("no", "drop it", 2)],
      selected: 2,
    }),
  },
  "a text field": {
    title: "branch name",
    menu: () => ({ id: "name", title: "branch name", question: true, items: [choice("ok", "ok", 0)], selected: 0, typed: "feature/abc", caret: 4, refused: "no spaces allowed" }),
  },
  "a menu with headings and notes": {
    title: "Section menu",
    menu: () => ({
      id: "sections",
      title: "Section menu",
      question: false,
      items: [
        { value: "", label: "First group", hint: "", heading: true },
        choice("a", "alpha", 1),
        { value: "", label: "a line drawn as written", hint: "", note: true },
        choice("b", "beta", 3),
      ],
      selected: 1,
    }),
  },
  "a menu longer than the body": {
    title: "Many rows",
    menu: (_w, h) => ({
      id: "many",
      title: "Many rows",
      question: false,
      items: Array.from({ length: h + 10 }, (_, i) => choice(`v${i}`, `row ${i}`, i)),
      selected: 0,
      lines: h + 10,
      shown: h - 4,
    }),
  },
  "a menu opened by a context click in the middle": {
    title: "Context menu",
    menu: (w, h) => ({ ...MENUS["an actions menu"]!.menu(w, h), title: "Context menu", at: { column: Math.floor(w / 3), row: 5 } }),
  },
  "a menu opened by a context click at the bottom right corner": {
    title: "Corner menu",
    menu: (w, h) => ({ ...MENUS["an actions menu"]!.menu(w, h), title: "Corner menu", at: { column: w - 2, row: h - 3 } }),
  },
};

/** The rows of the menu box whose title is `title`, each as the text between the box's side borders. */
function boxInteriors(lines: string[], title: string): string[] {
  const y = lines.findIndex((l) => l.includes(title));
  expect(y, `the menu's title "${title}" is on screen`).toBeGreaterThan(0);
  const at = lines[y]!.indexOf(title);
  // The box's left and right edges are the nearest border glyphs on the title's row. The title sits in the
  // top border (`╭ Title ───╮`) or, as it once did, on a row of its own under it (`│ Title │`).
  const row = [...lines[y]!];
  const col = (from: number, step: number, glyphs: string) => {
    for (let i = from; i >= 0 && i < row.length; i += step) if (glyphs.includes(row[i]!)) return i;
    return -1;
  };
  const left = col(at - 1, -1, "╭│");
  const right = col(at + title.length, 1, "╮│");
  expect(left, "left border").toBeGreaterThanOrEqual(0);
  expect(right, "right border").toBeGreaterThan(left);
  const titleInBorder = row[left] === "╭";
  let bottom = y;
  while (bottom < lines.length && [...lines[bottom]!][left] !== "╰") bottom++;
  const out: string[] = [];
  for (let r = titleInBorder ? y : y - 1; r <= Math.min(bottom, lines.length - 1); r++) out.push([...lines[r]!].slice(left + 1, right).join(""));
  return out;
}

const frameWith = async (w: number, h: number, chrome: Record<string, unknown>, screen: any = listScreen(h, w)) => {
  const m = mount(w, h);
  m.snapshot({ chrome: chromeOf(w, h, chrome), screen });
  await m.settle();
  return m;
};

describe("the fixture itself", () => {
  it.each(SIZES)("a body with no overlay shows the sentinel on most rows at %ix%i", async (w, h) => {
    const m = await frameWith(w, h, {});
    const rows = m.lines().filter((l) => SENTINEL.test(l));
    expect(rows.length).toBeGreaterThan(h / 2);
  });
});

describe("a menu, question or text field hides what is under it", () => {
  for (const [w, h] of SIZES) {
    for (const [name, { title, menu }] of Object.entries(MENUS)) {
      it(`${name} at ${w}x${h}: no cell of the list inside its box`, async () => {
        const m = await frameWith(w, h, { menu: menu(w, h) });
        const lines = m.lines();
        expect(lines.length).toBeLessThanOrEqual(h);
        const y = lines.findIndex((l) => l.includes(title));
        expect(y, `the menu's title "${title}" is on screen`).toBeGreaterThan(0);
        boxInteriors(lines, title).forEach((inside, i) => expect(inside, `box row ${i}: ${inside}`).not.toMatch(SENTINEL));
      });
    }
  }
});

describe("History and settings leave nothing of the body showing", () => {
  for (const [w, h] of SIZES) {
    it(`the History overlay at ${w}x${h}: no cell of the list below the top bar`, async () => {
      const history = {
        version: 1,
        width: w,
        height: h - 1,
        title: "History",
        boundary: "snapshot through #1",
        scope: "recent",
        notes: [],
        focus: "events",
        split: w >= 120,
        events_title: "events",
        columns: ["seen", "action", "about", "result"],
        rows: [{ observed: "2026-10-12", action: "act.one", subject: "s", outcome: "ok", outcome_ink: "muted", target: "chrome:history:row:0" }],
        selected: 0,
        list_scroll: 0,
        state: "",
        detail_title: "detail",
        detail: [],
        detail_scroll: 0,
        help_title: "keys",
        help_lines: [],
        help_scroll: 0,
        footer: [{ keys: "esc", label: "close" }],
      };
      const m = await frameWith(w, h, { history });
      const lines = m.lines();
      expect(lines.join("\n")).toContain("History");
      lines.slice(1).forEach((l, i) => expect(l, `row ${i + 1}`).not.toMatch(SENTINEL));
    });

    it(`settings at ${w}x${h}: nothing of the list the screen had before`, async () => {
      const m = mount(w, h);
      m.snapshot({ chrome: chromeOf(w, h), screen: listScreen(h, w) });
      await m.settle();
      expect(m.lines().some((l) => SENTINEL.test(l))).toBe(true);
      const nav = {
        groups: [{ id: "gitlab", name: "GitLab", open: true, target: "chrome:group:gitlab", sections: [{ id: "connection", name: "connection", target: "chrome:section:0" }] }],
        selected: "chrome:section:0",
        section_title: "connection",
        detail_title: "detail",
      };
      const section = { version: 1, scope: "connection", body: { node: "document", lines: [[{ text: "the section" }]] }, detail_title: "detail", detail: [[{ text: "the detail" }]] };
      m.snapshot({ chrome: chromeOf(w, h, { settings: nav }), section });
      await m.settle();
      const lines = m.lines();
      expect(lines.join("\n")).toContain("the section");
      lines.slice(1).forEach((l, i) => expect(l, `row ${i + 1}`).not.toMatch(SENTINEL));
    });
  }

  it("a menu over settings hides the section under it", async () => {
    const [w, h] = [80, 24];
    const nav = { groups: [{ id: "g", name: "GitLab", open: false, target: "chrome:group:g", sections: [] }], section_title: "connection", detail_title: "detail" };
    const section = {
      version: 1,
      scope: "connection",
      body: { node: "list", selected: 0, rows: Array.from({ length: h }, (_, i) => ({ target: `r${i}`, text: [{ text: "ZQXJ".repeat(20) }] })) },
      detail_title: "detail",
      detail: [],
    };
    const m = mount(w, h);
    m.snapshot({ chrome: chromeOf(w, h, { settings: nav, menu: MENUS["an actions menu"]!.menu(w, h) }), section });
    await m.settle();
    const lines = m.lines();
    boxInteriors(lines, "Actions for !263").forEach((inside, i) => expect(inside, `box row ${i}: ${inside}`).not.toMatch(SENTINEL));
  });
});

describe("the session's own transient states keep the body out of their rows", () => {
  for (const [w, h] of SIZES) {
    it(`a pending input request at ${w}x${h}: it is on screen and its row holds none of the list`, async () => {
      const m = await frameWith(w, h, {});
      m.conn.receive({ type: "input_request", id: 1, cursor: 1, title: "branch name", value: "feat" });
      await m.settle();
      const lines = m.lines();
      expect(lines.some((l) => l.includes("branch name")), "the input request is on screen").toBe(true);
      // Drawn as a popup: only its own box must hold none of the list (the body beside a centred popup stays visible).
      boxInteriors(lines, "branch name").forEach((inside, i) => expect(inside, `box row ${i}: ${inside}`).not.toMatch(SENTINEL));
    });

    it(`a notice at ${w}x${h}: its line is the status line's and holds none of the list`, async () => {
      const m = await frameWith(w, h, {});
      m.conn.receive({ type: "notice", text: "the renderer was restarted" });
      await m.settle();
      const lines = m.lines();
      const y = lines.findIndex((l) => l.includes("the renderer was restarted"));
      expect(y, "the notice is on screen").toBeGreaterThan(0);
      expect(lines[y]!, `row ${y}`).not.toMatch(SENTINEL);
    });
  }
});

describe("a generic overlay (any node but an editor) is a titled pane over the foot of the body", () => {
  for (const [w, h] of SIZES) {
    it(`at ${w}x${h}: a third of the body, at least 5 rows, none of the list inside it`, async () => {
      const screen = { ...listScreen(h, w), overlay: { pane: "middle", title: "OVERLAYPANE", body: { node: "document", lines: [[{ text: "overlay words" }]] } } };
      const m = await frameWith(w, h, {}, screen);
      const lines = m.lines();
      const top = lines.findIndex((l) => l.includes("OVERLAYPANE"));
      expect(top, "the overlay's title is on screen").toBeGreaterThan(0);
      expect(lines.join("\n")).toContain("overlay words");
      // the body is the frame less the top bar and the status line
      const body = h - 2;
      const rows = Math.min(body, Math.max(5, Math.floor(body / 3)));
      const bottom = lines.findIndex((l, i) => i > top && l.includes("╰"));
      expect(bottom - top + 1, "its height").toBe(rows);
      lines.slice(top + 1, bottom).forEach((l, i) => expect(l, `overlay row ${i}`).not.toMatch(SENTINEL));
    });
  }
});
