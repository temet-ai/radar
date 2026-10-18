import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { frameOf } from "../../src/chrome/Chrome.js";
import { layoutMenu, layoutStatus, layoutTopBar, type Cell } from "../../src/chrome/layout.js";
import { ATTENTION_DOT } from "../../src/text/icons.js";
import { bgOf, slotStyle } from "../../src/text/look.js";
import { cellWidth } from "../../src/chrome/width.js";
import type { ChromeView } from "../../src/chrome/types.js";

// Chrome view data recorded from the real host (radar with the git part, a scratch clone of acme/team-a/api under
// [git] roots, 80x24, read off the IPC socket by a relay): `checkout_row` is the Branches screen with the clone
// row selected in the middle pane; `context_menu` is the same screen with the row's actions menu open.
const load = (name: string): ChromeView => JSON.parse(readFileSync(fileURLToPath(new URL(`./fixtures/${name}.json`, import.meta.url)), "utf8"));
const line = (cells: Cell[]) => cells.map((c) => c.text).join("");

describe("status line on a checkout row (radar-git)", () => {
  const chrome = load("checkout_row_80x24");

  it("the host sends the row's key first, ahead of the page keys", () => {
    expect(chrome.status.hints[0]).toMatchObject({ keys: "⏎", label: "use it" });
  });

  it.each([80, 100, 160])("at %i columns the row key follows the scope chip and the line fits", (w) => {
    const text = line(layoutStatus(chrome, w, 23).cells);
    expect(text).toMatch(/^ BRANCHES {3}⏎ use it {3}/);
    expect(cellWidth(text)).toBe(w);
  });

  it("at 60 columns the row key stays while the page keys give way", () => {
    const text = line(layoutStatus(chrome, 60, 23).cells);
    expect(text).toContain("⏎ use it");
    expect(text).not.toContain("navigate");
    expect(text).toContain("x");
  });

  it("the row key is a pointer target when the host gives it one", () => {
    const withTarget = structuredClone(chrome);
    withTarget.status.hints[0]!.keys_of = [{ label: "⏎", chord: { key: "enter" }, target: "chrome:key:0:0" }];
    const laid = layoutStatus(withTarget, 80, 23);
    expect(laid.hits.map((h) => h.target)).toContain("chrome:key:0:0");
  });
});

describe("a context menu opened at the pointer (menu.at), kept inside the body", () => {
  const chrome = load("context_menu_80x24");
  const menu = chrome.menu!;
  const at = (column: number, row: number) => ({ ...menu, at: { column, row } });
  const body = { top: 1, height: 22 };

  it("the recorded menu has no pointer cell and is centred", () => {
    expect(menu.at ?? null).toBeNull();
    const m = layoutMenu(menu, 80, body.top, body.height);
    expect(m.x).toBe(Math.floor((80 - m.w) / 2));
  });

  it.each([
    ["the bottom-right corner", 79, 23],
    ["the top-left corner", 0, 0],
    ["the right edge, mid-height", 79, 8],
    ["the bottom edge, left", 2, 23],
    ["well inside", 3, 4],
  ])("%s: the whole frame is on screen and inside the body", (_n, column, row) => {
    const m = layoutMenu(at(column, row), 80, body.top, body.height);
    expect(m.x).toBeGreaterThanOrEqual(0);
    expect(m.y).toBeGreaterThanOrEqual(body.top);
    expect(m.x + m.w).toBeLessThanOrEqual(80);
    expect(m.y + m.h).toBeLessThanOrEqual(body.top + body.height);
    expect(m.lines).toHaveLength(m.h);
    for (const l of m.lines) expect(cellWidth(line(l))).toBe(m.w);
    for (const h of m.hits) {
      expect(h.x).toBeGreaterThanOrEqual(m.x);
      expect(h.x + h.w).toBeLessThanOrEqual(m.x + m.w);
      expect(h.y).toBeGreaterThanOrEqual(m.y);
      expect(h.y + h.h).toBeLessThanOrEqual(m.y + m.h);
    }
  });

  it("the bottom-right corner puts the frame flush against the corner (Ratatui's clamp)", () => {
    const m = layoutMenu(at(79, 23), 80, body.top, body.height);
    expect(m.x + m.w).toBe(80);
    expect(m.y + m.h).toBe(body.top + body.height);
  });

  it("inside the body the top-left corner goes where the pointer was", () => {
    const m = layoutMenu(at(3, 4), 80, body.top, body.height);
    expect([m.x, m.y]).toEqual([3, 4]);
  });

  it("through the whole frame the same holds with a sources row, and the status line stays live", () => {
    const withSources = { ...chrome, sources: [{ label: "GitLab", active: true, target: "chrome:source:0" }] };
    const frame = frameOf(withSources, at(79, 23), 80, 24);
    const m = frame.menu!;
    expect(frame.bodyTop).toBe(2);
    expect(m.y).toBeGreaterThanOrEqual(2);
    expect(m.y + m.h).toBeLessThanOrEqual(23);
    expect(m.x + m.w).toBeLessThanOrEqual(80);
    expect(frame.hits.some((h) => h.y === 23)).toBe(true);
  });
});

// shell.rs paints a count in widgets::tone, a chrome word in ink_style, and a menu's title in
// bold emphasis with its icons in their own style, on the popup's shade (menu.rs Layout::new).
describe("chrome styles are the ones shell.rs and menu.rs paint", () => {
  const cell = (cells: Cell[], text: string) => cells.find((c) => c.text.includes(text));
  const withCounts = () => {
    const chrome = load("checkout_row_80x24");
    chrome.status.counts = [
      { text: "0 open", ink: "plain" },
      { text: "2 to review", ink: "accent" },
      { text: "1 failed to sync", ink: "bad" },
      { text: "3 late", ink: "warn" },
      { text: "4 fine", ink: "good" },
      { text: "5 quiet", ink: "muted" },
    ];
    return chrome;
  };

  it("counts: plain is primary text, accent, warn and bad are bold, the rest are not", () => {
    const cells = layoutStatus(withCounts(), 240, 39).cells;
    expect(cell(cells, "0 open")?.style).toMatchObject({ color: slotStyle("text.primary").color });
    expect(cell(cells, "0 open")?.style?.bold).toBeFalsy();
    expect(cell(cells, "2 to review")?.style).toMatchObject({ color: slotStyle("accent.primary").color, bold: true });
    expect(cell(cells, "1 failed to sync")?.style).toMatchObject({ color: slotStyle("status.error").color, bold: true });
    expect(cell(cells, "3 late")?.style).toMatchObject({ color: slotStyle("status.warning").color, bold: true });
    expect(cell(cells, "4 fine")?.style?.bold).toBeFalsy();
    expect(cell(cells, "5 quiet")?.style).toMatchObject({ color: slotStyle("text.muted").color });
  });

  it("a tab's attention dot uses the warning slot", () => {
    const chrome = load("checkout_row_80x24");
    chrome.top_bar.tabs[0]!.attention = true;
    const dot = layoutTopBar(chrome, 160).cells.find((c) => c.text === ATTENTION_DOT);
    expect(dot?.style).toMatchObject({ color: slotStyle("status.warning").color });
  });

  it("the menu title is bold emphasis on the popup shade, its icons keep their own style, only the frame is accent", () => {
    const menu = load("context_menu_80x24").menu!;
    const top = layoutMenu(menu, 80, 1, 22).lines[0]!;
    const words = top.find((c) => c.text.includes(menu.title))!;
    expect(words.style).toMatchObject({ color: slotStyle("text.emphasis").color, bold: true, backgroundColor: bgOf("bg.overlay") });
    const frame = top[0]!;
    expect(frame.text).toBe("╭");
    expect(frame.style?.color).toBe(slotStyle("accent.primary").color);
    expect(top.at(-1)!.text.endsWith("╮")).toBe(true);
    expect(cellWidth(line(top))).toBe(layoutMenu(menu, 80, 1, 22).w);
  });

  it("a write question's title is its words only (the host's change kind is not drawn in it)", () => {
    const m = layoutMenu({ id: "c", title: "post comment?", question: true, change: "add", items: [{ value: "y", label: "post it", hint: "", key: "y", target: "chrome:menu:0" }] }, 80, 1, 22);
    expect(line(m.lines[0]!)).toContain("post comment?");
    expect(line(m.lines[0]!)).not.toContain("(add)");
  });

  it("a confirm's body notes keep their case (the host sends a note with heading and note both true); a real heading is capitals", () => {
    const m = layoutMenu(
      {
        id: "w",
        title: "post comment?",
        question: true,
        items: [
          { value: "", label: "hello", hint: "", heading: true, note: true, shown_only: true },
          { value: "", label: "radar is read only (set by RADAR_MODE): nothing will be sent", hint: "", heading: true, note: true, shown_only: true },
          { value: "", label: "where", hint: "", heading: true, shown_only: true },
          { value: "y", label: "yes", hint: "", key: "y", target: "chrome:menu:3" },
        ],
      } as never,
      80,
      1,
      22,
    );
    const body = m.lines.map(line).join("\n");
    expect(body).toContain(" hello");
    expect(body).not.toContain("HELLO");
    expect(body).toContain("radar is read only (set by RADAR_MODE)");
    expect(body).toContain("WHERE");
  });

  it("a title wider than the popup is the first lines of its body, wrapped, and the border carries none", () => {
    const title = "this is a long question about something the user must read before answering it";
    const m = layoutMenu({ id: "c", title, question: true, items: [{ value: "y", label: "yes", hint: "", key: "y", target: "chrome:menu:0" }] }, 40, 1, 22);
    expect(line(m.lines[0]!)).toMatch(/^╭─+╮$/);
    const body = m.lines.slice(1).map(line).join("\n");
    expect(body).toContain("this is a long question");
    expect(body).toContain("answering it");
    for (const l of m.lines) expect(cellWidth(line(l))).toBe(m.w);
  });
});
