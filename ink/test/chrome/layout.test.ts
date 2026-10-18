import { describe, expect, it } from "vitest";
import { hitAt, layoutMenu, layoutSources, layoutStatus, layoutTopBar, type Cell } from "../../src/chrome/layout.js";
import { withNotice } from "../../src/chrome/Chrome.js";
import { applyLook, currentLook, defaultLook, slotStyle } from "../../src/text/look.js";
import { actionsMenu, booksChrome, confirm } from "./fixture.js";
import { ATTENTION_DOT } from "../../src/text/icons.js";
import { cellWidth } from "../../src/chrome/width.js";

const line = (cells: Cell[]) => cells.map((c) => c.text).join("");

describe.each([80, 160])("chrome layout at %i columns", (w) => {
  const chrome = booksChrome({ width: w });

  it("top bar fills exactly the width and underlines the mnemonic", () => {
    const bar = layoutTopBar(chrome, w);
    expect(cellWidth(line(bar.cells))).toBe(w);
    expect(line(bar.cells)).toMatchSnapshot();
    const books = bar.cells.find((c) => c.style?.underline);
    expect(books?.text).toBe("B");
  });

  it("every tab and the mode chip is a pointer target", () => {
    const bar = layoutTopBar(chrome, w);
    expect(bar.hits.map((h) => h.target)).toEqual(["chrome:tab:0", "chrome:tab:1", "chrome:history", "chrome:settings", "chrome:mode"]);
    const tab1 = bar.hits[1]!;
    expect(hitAt(bar.hits, tab1.x + 1, 0)?.target).toBe("chrome:tab:1");
    expect(hitAt(bar.hits, tab1.x, 1)).toBeUndefined();
  });

  it("status line hints, keys are targets, least needed give way first", () => {
    const st = layoutStatus(chrome, w, w === 80 ? 23 : 39);
    expect(line(st.cells)).toMatchSnapshot();
    expect(cellWidth(line(st.cells))).toBeLessThanOrEqual(w);
    expect(st.hits.map((h) => h.target)).toContain("chrome:key:1:0");
  });
});

describe("narrow status line", () => {
  it("drops least_needed hints before the keys that matter", () => {
    const st = layoutStatus(booksChrome(), 50, 23);
    const text = line(st.cells);
    expect(text).toContain("quit");
    expect(text).not.toContain("navigate");
    expect(cellWidth(text)).toBeLessThanOrEqual(50);
  });

  it("a message takes the place of the keys", () => {
    const chrome = booksChrome();
    chrome.status.message = { text: "nothing to scroll here" };
    const text = line(layoutStatus(chrome, 80, 23).cells);
    expect(text).toContain("nothing to scroll here");
    expect(text).not.toContain("quit");
  });

  it("a notice is drawn in the status.info slot as the host's message is, not as a warning", () => {
    const chrome = withNotice(booksChrome(), "cannot comment here: no line");
    const cells = layoutStatus(chrome, 80, 23).cells;
    const at = cells.find((c) => c.text.includes("cannot"));
    expect(at?.style?.color).toBe(slotStyle("status.info").color);
    expect(slotStyle("status.info").color).not.toBe(slotStyle("status.warning").color);
  });
});

describe("sources and menus", () => {
  it("sources row shows each source with its target", () => {
    const s = layoutSources(booksChrome(), 80, 1)!;
    // shell.rs source_tabs: `  label  ` a tab, on the overlay shade the whole row.
    expect(line(s.cells)).toBe(`  GitLab    GitHub  ${" ".repeat(80 - 20)}`);
    expect(s.hits.map((h) => h.target)).toEqual(["chrome:source:0", "chrome:source:1"]);
    expect(layoutSources(booksChrome({ sources: [] }), 80, 1)).toBeNull();
  });

  it("a menu is centred in the body and its choosable rows are targets (headings are not)", () => {
    const m = layoutMenu(actionsMenu, 80, 1, 22);
    expect(m.x).toBeGreaterThan(0);
    expect(m.hits.map((h) => h.target)).toEqual(["chrome:menu:1", "chrome:menu:2"]);
    expect(m.hits[0]!.y).toBe(m.y + 2); // the border (the title is in it), the heading, then the first choice
  });

  it("a question puts its choices in one row of buttons", () => {
    const m = layoutMenu(confirm, 80, 1, 22);
    const row = m.lines.map((l) => l.map((c) => c.text).join("")).join("\n");
    expect(row).toContain("y post it");
    expect(row).toContain("n drop it");
  });

  it("a context-click menu opens at its cell (kept on screen), scrolls to its first row, and draws the caret inside the field", () => {
    const at = layoutMenu({ ...actionsMenu, at: { column: 60, row: 5 } }, 80, 1, 22);
    expect(at.y).toBe(5);
    expect(at.x + at.w).toBeLessThanOrEqual(80);
    const far = layoutMenu({ ...actionsMenu, at: { column: 79, row: 30 } }, 80, 1, 22);
    expect(far.x + far.w).toBeLessThanOrEqual(80);
    expect(far.y + far.h).toBeLessThanOrEqual(23);
    const scrolled = layoutMenu({ ...actionsMenu, scroll: 1 }, 80, 1, 4);
    const text = (m: { lines: Array<Array<{ text: string }>> }) => m.lines.map((l) => l.map((c) => c.text).join("")).join("|");
    expect(text(scrolled)).toContain("merge");
    expect(text(scrolled)).not.toContain("│ !263");
    const typed = layoutMenu({ id: "t", title: "name", question: false, items: [], typed: "abcd", caret: 2 }, 80, 1, 22);
    expect(typed.lines[1]!.map((c) => c.text).join("")).toContain(" abcd");
    // The caret is one reversed cell over the character it is before, on the overlay shade (input::render_field).
    const caret = typed.lines[1]!.filter((c) => c.inverse);
    expect(caret.map((c) => c.text)).toEqual(["c"]);
  });
});

describe("status line keeps the keys Ratatui keeps (shell.rs status_line)", () => {
  const hint = (keys: string, label: string) => ({ keys, label, keys_of: [] });
  const base = () => {
    const chrome = booksChrome();
    chrome.status.scope = "REPOSITORIES";
    chrome.status.counts = [
      { text: "12 open", ink: "muted" },
      { text: "4 to review", ink: "muted" },
      { text: "1 broken", ink: "warn" },
    ];
    chrome.status.least_needed = ["navigate", "scroll", "quit", "expand/collapse"];
    return chrome;
  };
  const labels = (chrome: ReturnType<typeof base>, w: number) => {
    const text = line(layoutStatus(chrome, w, 0).cells);
    return { text, shown: chrome.status.hints.filter((h) => text.includes(`${h.keys} ${h.label}`)).map((h) => h.label) };
  };

  it("a list at 80 columns: the counts give way before any key, then navigate and scroll", () => {
    const chrome = base();
    chrome.status.hints = [hint("PgUp/PgDn", "scroll"), hint("b", "bulk"), hint("+/-", "expand/collapse"), hint("q", "quit"), hint("x", "menu"), hint("←→↑↓", "navigate")];
    const { text, shown } = labels(chrome, 80);
    expect(shown).toEqual(["bulk", "expand/collapse", "quit", "menu"]);
    expect(text).not.toContain("open");
    expect(cellWidth(text)).toBeLessThanOrEqual(80);
  });

  it("with a row key in front navigate and scroll go, the rest stay", () => {
    const chrome = base();
    chrome.status.hints = [hint("e", "approve"), hint("PgUp/PgDn", "scroll"), hint("b", "bulk"), hint("+/-", "expand/collapse"), hint("q", "quit"), hint("x", "menu"), hint("←→↑↓", "navigate")];
    expect(labels(chrome, 80).shown).toEqual(["approve", "bulk", "expand/collapse", "quit", "menu"]);
  });

  it("page keys at 80 columns: PgUp/PgDn, q, x and the arrows, the counts dropped", () => {
    const chrome = base();
    chrome.status.hints = [hint("PgUp/PgDn", "scroll"), hint("q", "quit"), hint("x", "menu"), hint("←→↑↓", "navigate")];
    const { text, shown } = labels(chrome, 80);
    expect(shown).toEqual(["scroll", "quit", "menu", "navigate"]);
    expect(text).not.toContain("broken");
  });

  it("at 160 columns every hint and every count fits, in the host's order", () => {
    const chrome = base();
    chrome.status.hints = [hint("PgUp/PgDn", "scroll"), hint("b", "bulk"), hint("+/-", "expand/collapse"), hint("q", "quit"), hint("x", "menu"), hint("←→↑↓", "navigate")];
    const { text, shown } = labels(chrome, 160);
    expect(shown).toEqual(chrome.status.hints.map((h) => h.label));
    expect(text).toContain("1 broken");
  });

  it("a message keeps the counts", () => {
    const chrome = base();
    chrome.status.message = { text: "configured repositories are readable" };
    expect(line(layoutStatus(chrome, 80, 0).cells)).toContain("1 broken");
  });
});

describe("a note or heading of a menu with an icon (shell/menu.rs: the icon leads its first line)", () => {
  it("draws the glyph and its gap after the cell of air, before the words, and makes room for it", () => {
    const before = currentLook();
    applyLook(defaultLook("nerd"));
    try {
      const items = [{ value: "n", label: "a program and its arguments", hint: "", note: true, heading: true, icon: "warning" }] as never;
      const m = layoutMenu({ id: "t", title: "add", question: false, items, typed: "", caret: 0 }, 80, 1, 22);
      const note = m.lines.map((l) => l.map((c) => c.text).join("")).find((l) => l.includes("a program"))!;
      expect(note).toMatch(/^│ \S{1,2} a program and its arguments/);
    } finally {
      applyLook(before);
    }
  });
});


describe("names-only top-bar tabs", () => {
  it.each([60, 80, 160])("draws only explicit attention, including on the active tab, at %i columns", (width) => {
    const before = currentLook();
    applyLook(defaultLook());
    try {
      for (const keys_at of ["body", "bar"] as const) {
        const chrome = booksChrome({ width, keys_at });
        chrome.top_bar.tabs[0] = { label: "Books", mnemonic: "B", active: true, attention: true, target: "chrome:tab:0" };
        chrome.top_bar.tabs[1] = { label: "Work", mnemonic: "W", attention: false, target: "chrome:tab:1" };
        const bar = layoutTopBar(chrome, width);
        const text = line(bar.cells);
        expect(cellWidth(text)).toBe(width);
        expect(text.slice(0, text.indexOf("synced") < 0 ? text.length : text.indexOf("synced"))).not.toMatch(/\d/);
        const dots = bar.cells.filter((c) => c.text === ATTENTION_DOT);
        expect(dots).toHaveLength(1);
        expect(dots[0]!.style?.color).toBe(slotStyle("status.warning").color);
        expect(dots[0]!.style?.backgroundColor).toBe(keys_at === "bar" ? slotStyle("accent.primary").color : slotStyle("bg.selection").color);
        const dotAt = cellWidth(text.slice(0, text.indexOf("●")));
        expect(hitAt(bar.hits, dotAt, 0)?.target).toBe("chrome:tab:0");
        expect(bar.hits.map((h) => h.target)).toEqual([...chrome.top_bar.tabs.map((t) => t.target), "chrome:mode"]);
      }
    } finally {
      applyLook(before);
    }
  });

  it.each([0, 1, 2, 10, 11, 12, 20, 30, 40, 50])("clips tabs and pointer targets while keeping the fitting mode anchored at %i columns", (width) => {
    const chrome = booksChrome({ width });
    chrome.top_bar.tabs[0]!.attention = true;
    const bar = layoutTopBar(chrome, width);
    const text = line(bar.cells);
    const mode = ` ${chrome.top_bar.mode.text} `;
    expect(cellWidth(text)).toBe(width);
    for (const hit of bar.hits) {
      expect(hit.x).toBeGreaterThanOrEqual(0);
      expect(hit.w).toBeGreaterThan(0);
      expect(hit.x + hit.w).toBeLessThanOrEqual(width);
    }
    if (cellWidth(mode) < width) {
      expect(text.endsWith(mode)).toBe(true);
      const modeX = width - cellWidth(mode);
      for (let x = Math.max(0, modeX - 1); x < width; x++) expect(hitAt(bar.hits, x, 0)?.target).toBe("chrome:mode");
    } else {
      expect(bar.hits.some((h) => h.target === "chrome:mode")).toBe(false);
    }
    if (text.includes(ATTENTION_DOT)) {
      const dotX = cellWidth(text.slice(0, text.indexOf(ATTENTION_DOT)));
      expect(hitAt(bar.hits, dotX, 0)?.target).toBe("chrome:tab:0");
      expect(bar.cells.find((c) => c.text === ATTENTION_DOT)?.style?.color).toBe(slotStyle("status.warning").color);
    }
  });

  it.each(["unicode", "nerd", "emoji", "off"] as const)("keeps the attention mark visible and one cell wide with the %s pack", (pack) => {
    const before = currentLook();
    applyLook(defaultLook(pack));
    try {
      const chrome = booksChrome();
      chrome.top_bar.tabs[0]!.attention = true;
      const dots = layoutTopBar(chrome, 160).cells.filter((c) => c.text === ATTENTION_DOT);
      expect(dots).toHaveLength(1);
      expect(cellWidth(dots[0]!.text)).toBe(1);
      expect(dots[0]!.style?.color).toBe(slotStyle("status.warning").color);
    } finally {
      applyLook(before);
    }
  });

  it("does not infer attention or counts from a legacy badge", () => {
    const chrome = booksChrome();
    chrome.top_bar.tabs[0] = { ...chrome.top_bar.tabs[0]!, badge: { text: "99", ink: "bad" } } as never;
    const text = line(layoutTopBar(chrome, 160).cells);
    expect(text).toContain("Books");
    expect(text).not.toContain("99");
    expect(text).not.toContain("●");
  });

  it("measures wide names, combining mnemonics and wide icons in terminal cells", () => {
    const before = currentLook();
    applyLook(defaultLook("emoji"));
    try {
      const chrome = booksChrome();
      chrome.top_bar.tabs = [
        { label: "図書", mnemonic: "図", icon: "repo", attention: true, target: "chrome:tab:0" },
        { label: "Cafe\u0301", mnemonic: "e\u0301", target: "chrome:tab:1" },
      ];
      const bar = layoutTopBar(chrome, 80);
      const text = line(bar.cells);
      expect(cellWidth(text)).toBe(80);
      expect(bar.cells.find((c) => c.text === "e\u0301")?.style?.underline).toBe(true);
      const dotAt = cellWidth(text.slice(0, text.indexOf(ATTENTION_DOT)));
      expect(hitAt(bar.hits, dotAt, 0)?.target).toBe("chrome:tab:0");
      const nextAt = cellWidth(text.slice(0, text.indexOf("Cafe")));
      expect(hitAt(bar.hits, nextAt, 0)?.target).toBe("chrome:tab:1");
    } finally {
      applyLook(before);
    }
  });
});
