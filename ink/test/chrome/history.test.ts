import { describe, expect, it } from "vitest";
import { layoutHistory } from "../../src/chrome/history.js";
import { hitAt, type Cell } from "../../src/chrome/layout.js";
import { cellWidth } from "../../src/chrome/width.js";
import { bgOf, slotStyle } from "../../src/text/look.js";
import type { HistoryView } from "../../src/protocol/generated/HistoryView.js";

const view = (w: number, h: number, over: Partial<HistoryView> = {}): HistoryView =>
  ({
    version: 1,
    width: w,
    height: h,
    title: "History",
    boundary: "snapshot through #12",
    scope: "all recorded events",
    notes: [[{ text: "coverage may be incomplete", ink: "warn" }]],
    focus: "events",
    split: w >= 120,
    events_title: "events",
    columns: ["observed UTC", "action", "subject", "outcome"],
    rows: [
      { observed: "2026-10-12T10:00:00Z", action: "git.push", subject: "main", outcome: "ok", outcome_ink: "good", target: "chrome:history:row:0" },
      { observed: "2026-10-12T10:01:00Z", action: "git.pull", subject: "main", outcome: "failed", outcome_ink: "bad", target: "chrome:history:row:1" },
    ],
    selected: 0,
    list_scroll: 0,
    state: "",
    detail_title: "details",
    detail: [[{ text: "kind git.push" }], [{ text: "scope  s/a  all" }]],
    detail_keys_line: 1,
    detail_keys: [{ label: "s", chord: { key: { char: "s" } }, target: "chrome:history:key:s" }, { label: "a", chord: { key: { char: "a" } }, target: "chrome:history:key:a" }],
    detail_scroll: 0,
    help_title: "keys",
    help_lines: [],
    help_scroll: 0,
    footer: [{ keys: "esc", label: "close", keys_of: [{ label: "esc", chord: { key: "esc" }, target: "chrome:history:key:esc" }] }],
    ...over,
  }) as unknown as HistoryView;

const text = (cells: Cell[]) => cells.map((c) => c.text).join("");
const find = (lines: Cell[][], word: string) => {
  for (const l of lines) {
    const c = l.find((c) => c.text.includes(word));
    if (c) return { cell: c, line: l };
  }
  throw new Error(`no cell with ${word}`);
};

describe.each([
  [80, 24],
  [160, 40],
])("history overlay at %ix%i draws with the host's look", (w, h) => {
  const lay = layoutHistory(view(w, h - 1), w, h - 1, 1);

  it("every line is the full width and nothing is an Ink colour name", () => {
    for (const l of lay.lines) {
      expect(cellWidth(text(l))).toBe(w);
      for (const c of l) expect(c.ink, text(l)).toBeUndefined();
    }
  });

  it("the selected row is the selection shade across the row, never inverse", () => {
    // Only the events panel: the left 3/5 when the detail sits beside it.
    const events = w >= 120 ? Math.floor((w * 3) / 5) : w;
    let used = 0;
    const row = find(lay.lines, "git.push  ").line.filter((c) => {
      const keep = used < events;
      used += cellWidth(c.text);
      return keep;
    });
    const sel = bgOf("bg.selection");
    const panelCells = row.filter((c) => !/[│╭╮╰╯]/.test(c.text));
    expect(panelCells.some((c) => c.style?.backgroundColor === sel)).toBe(true);
    for (const c of row) expect(c.inverse, c.text).toBeFalsy();
    for (const c of row.filter((c) => c.text.trim() !== "" && !/[│]/.test(c.text))) expect(c.style?.backgroundColor, c.text).toBe(sel);
    const other = find(lay.lines, "git.pull").line;
    expect(other.some((c) => c.style?.backgroundColor === sel)).toBe(false);
  });

  it("outcomes carry the outcome's colour, not the selection's, on an unselected row", () => {
    expect(find(lay.lines, "failed").cell.style?.color).toBe(slotStyle("status.error").color);
  });

  it("the focused panel's border is accent.primary, the other's accent.muted, the title in the panel's look", () => {
    const top = lay.lines.find((l) => text(l).includes("╭"))!;
    const border = top.find((c) => c.text.startsWith("╭"))!;
    expect(border.style?.color).toBe(slotStyle("accent.primary").color);
    expect(border.style?.backgroundColor).toBe(bgOf("bg.surface"));
    const title = top.find((c) => c.text.includes("events"))!;
    expect(title.style?.bold).toBe(true);
    if (w >= 120) {
      const d = top.find((c) => c.text.includes("details"))!;
      expect(d.style?.color).toBe(slotStyle("text.secondary").color);
      const bd = [...top].reverse().find((c) => c.text.endsWith("╮"))!;
      expect(bd.style?.color).toBe(slotStyle("accent.muted").color);
    }
  });

  it("the header is on bg.base and the footer keys are bold accent.secondary with muted words", () => {
    expect(lay.lines[0]![0]!.style?.backgroundColor).toBe(bgOf("bg.base"));
    expect(find(lay.lines, "History").cell.style?.bold).toBe(true);
    const foot = lay.lines[lay.lines.length - 1]!;
    const key = foot.find((c) => c.text === "esc")!;
    expect(key.style?.color).toBe(slotStyle("accent.secondary").color);
    expect(key.style?.bold).toBe(true);
    expect(foot.find((c) => c.text === " close")!.style?.color).toBe(slotStyle("text.muted").color);
  });

  it("the scope keys on the detail's line are pointable where they are drawn", () => {
    const v = view(w, h - 1, { focus: "detail", split: false });
    const l = layoutHistory(v, w, h - 1, 1);
    const targets = l.hits.map((x) => x.target);
    expect(targets).toContain("chrome:history:key:s");
    expect(targets).toContain("chrome:history:key:a");
    const hit = l.hits.find((x) => x.target === "chrome:history:key:a")!;
    expect(text(l.lines[hit.y - 1]!).slice(hit.x, hit.x + 1)).toBe("a");
    expect(hitAt(l.hits, hit.x, hit.y)?.target).toBe("chrome:history:key:a");
  });
});

// ui::table::Table lays its columns out in the panel less the border and padding (inner = width - 4): Length(20),
// Length(23), Min(11), Length(10) with one cell between. Measured on Ratatui (radar-ink-live, History, H): OUTCOME at
// 1-based column 69 of 80 and 85 of 160 (events panel 80 / 96 wide); the SUBJECT column starts at 48 either way.
describe.each([
  [80, 24, 80],
  [160, 40, 96],
])("history events table columns at %ix%i (panel %i wide) are Ratatui's", (w, h, panel) => {
  const lay = layoutHistory(view(w, h - 1), w, h - 1, 1);
  const header = text(find(lay.lines, "OBSERVED UTC").line);

  it("SUBJECT starts at cell 47 and OUTCOME 12 cells before the panel's right edge", () => {
    expect(header.indexOf("SUBJECT")).toBe(47);
    expect(header.indexOf("OUTCOME")).toBe(panel - 12);
  });

  it("a subject of 20 cells is not cut", () => {
    const rows = [{ observed: "2026-10-12 21:06:42", action: "session.tui", subject: "no subject recorded", outcome: "requested", outcome_ink: "plain", target: "chrome:history:row:0" }];
    const l = layoutHistory(view(w, h - 1, { rows } as never), w, h - 1, 1);
    expect(text(find(l.lines, "session.tui").line)).toContain("no subject recorded ");
  });
});

describe("history events scrollbar (table.rs: scroll::scrollbar on the panel's right border)", () => {
  const many = Array.from({ length: 60 }, (_, i) => ({ observed: `2026-10-12 10:${i}`, action: "a", subject: "s", outcome: "ok", outcome_ink: "plain", target: `chrome:history:row:${i}` }));

  it("rows past the list put a thumb (█) on a track (║) down the border at 80x24, between the corners", () => {
    const lay = layoutHistory(view(80, 23, { rows: many, split: false } as never), 80, 23, 1);
    const top = lay.lines.findIndex((l) => text(l).startsWith("╭"));
    const all = lay.lines.slice(top).map((l) => [...text(l)][79]!);
    const col = all.slice(0, all.indexOf("╯") + 1);
    expect(col[0]).toBe("╮");
    expect(col.slice(1, -1).every((c) => c === "█" || c === "║")).toBe(true);
    expect(col.slice(1, -1)).toContain("█");
    expect(col.slice(1, -1)).toContain("║");
    expect(col.at(-1)).toBe("╯");
  });

  it("everything fits: the border is plain", () => {
    const lay = layoutHistory(view(80, 23, { split: false }), 80, 23, 1);
    const col = lay.lines.map((l) => [...text(l)][79]!);
    expect(col).not.toContain("█");
    expect(col).not.toContain("║");
  });
});

describe("history detail and help scrollbars (history.rs: scroll::paragraph puts the bar on the panel's border)", () => {
  const long = Array.from({ length: 40 }, (_, i) => [{ text: `line ${i}` }]);
  const borderOf = (lay: ReturnType<typeof layoutHistory>) => {
    const top = lay.lines.findIndex((l) => text(l).startsWith("╭"));
    const all = lay.lines.slice(top).map((l) => [...text(l)][79]!);
    return all.slice(1, all.indexOf("╯"));
  };

  it("a detail longer than its panel: a thumb at the top, then at the end once scrolled past the last row", () => {
    const at0 = borderOf(layoutHistory(view(80, 23, { focus: "detail", split: false, detail: long, detail_keys_line: null } as never), 80, 23, 1));
    expect(at0[0]).toBe("█");
    expect(at0.at(-1)).toBe("║");
    const end = borderOf(layoutHistory(view(80, 23, { focus: "detail", split: false, detail: long, detail_keys_line: null, detail_scroll: 999 } as never), 80, 23, 1));
    expect(end[0]).toBe("║");
    expect(end.at(-1)).toBe("█");
  });

  it("the help text too; short text has none", () => {
    const help = borderOf(layoutHistory(view(80, 23, { help: true, help_lines: long } as never), 80, 23, 1));
    expect(help[0]).toBe("█");
    expect(help).toContain("║");
    const short = borderOf(layoutHistory(view(80, 23, { help: true, help_lines: long.slice(0, 3) } as never), 80, 23, 1));
    expect(short.join("")).not.toMatch(/[█║]/);
  });
});
