import { describe, expect, it } from "vitest";
import {
  Selector,
  capped,
  clampTo,
  extend,
  insideOf,
  lineAt,
  moved,
  selectKindOf,
  spans,
  textOf,
  wordAt,
  MAX_COPY_BYTES,
  type SelectArea,
  type Selection,
} from "../../src/input/selection.js";
import type { MouseEvent } from "../../src/input/mouse.js";

// A screen of two panes side by side, as cells: the list pane's inside is x 1..10, the detail's x 13..28 (each framed:
// a border column either side, a title row above, a border row below).
const SCREEN = [
  "╭ list ────╮╭ detail ───────╮",
  "│!263 build││fix the build  │",
  "│!264 docs ││  stage test 1m│",
  "│          ││src/a.ts:12 ok │",
  "╰──────────╯╰───────────────╯",
];
const cellsOf = (line: string): string[] => {
  const out: string[] = [];
  for (const c of line) {
    out.push(c);
    if (c === "界" || c === "✅") out.push("");
  }
  return out;
};
const rows = SCREEN.map(cellsOf);
const rowAt = (y: number) => rows[y] ?? [];
const list: SelectArea = { ...insideOf({ x: 0, y: 0, w: 12, h: 5 }), id: "pane:middle", kind: "rows" };
const detail: SelectArea = { ...insideOf({ x: 12, y: 0, w: 17, h: 5 }), id: "pane:detail", kind: "text" };
const sel = (area: SelectArea, ax: number, ay: number, hx: number, hy: number): Selection => ({ area, anchor: { x: ax, y: ay }, head: { x: hx, y: hy } });

describe("a selection inside a pane (select.rs, with the pane's edges for the screen's)", () => {
  it("the inside of a framed pane leaves out its border, its title row and a scrollbar column", () => {
    expect(insideOf({ x: 12, y: 0, w: 17, h: 5 })).toEqual({ x: 13, y: 1, w: 15, h: 3 });
    expect(insideOf({ x: 0, y: 0, w: 26, h: 10 }, 1)).toEqual({ x: 1, y: 1, w: 23, h: 8 });
  });

  it("a drag on one row copies the cells between", () => {
    expect(textOf(sel(detail, 17, 1, 19, 1), rowAt)).toBe("the");
  });

  it("across rows it runs to the pane's right edge and resumes at its left edge, not the screen's", () => {
    expect(spans(sel(detail, 17, 1, 19, 2))).toEqual([
      { y: 1, from: 17, to: 27 },
      { y: 2, from: 13, to: 19 },
    ]);
    expect(textOf(sel(detail, 17, 1, 19, 2), rowAt)).toBe("the build\n  stage");
  });

  it("dragging backwards selects the same", () => {
    expect(textOf(sel(detail, 19, 2, 17, 1), rowAt)).toBe(textOf(sel(detail, 17, 1, 19, 2), rowAt));
  });

  it("a head dragged out of the pane is clamped to its nearest edge: past the left edge it stops at the pane's first column, never the list beside it", () => {
    expect(clampTo(detail, 3, 2)).toEqual({ x: 13, y: 2 });
    expect(clampTo(detail, 40, 9)).toEqual({ x: 27, y: 3 });
    expect(clampTo(detail, 15, 0)).toEqual({ x: 15, y: 1 });
    const s = sel(detail, 22, 2, 13, 2);
    expect(textOf({ ...s, head: clampTo(detail, 3, 2) }, rowAt)).toBe("  stage te");
    expect(textOf({ ...s, head: clampTo(detail, 3, 2) }, rowAt)).not.toContain("docs");
  });

  it("each row is trimmed of the padding out to the edge; a wide character is copied once, also from its second cell", () => {
    const wide = cellsOf("│✅ ok 界x  │");
    const a: SelectArea = { x: 1, y: 0, w: 10, h: 1, id: "a", kind: "text" };
    expect(textOf(sel(a, 1, 0, 10, 0), () => wide)).toBe("✅ ok 界x");
    expect(textOf(sel(a, 2, 0, 10, 0), () => wide)).toBe("✅ ok 界x");
    expect(textOf(sel(list, 1, 3, 10, 3), rowAt)).toBe("");
  });

  it("a slip of one cell is still a click; two cells along or another row select", () => {
    expect(moved({ x: 3, y: 1 }, { x: 3, y: 1 })).toBe(false);
    expect(moved({ x: 3, y: 1 }, { x: 4, y: 1 })).toBe(false);
    expect(moved({ x: 4, y: 1 }, { x: 3, y: 1 })).toBe(false);
    expect(moved({ x: 3, y: 1 }, { x: 5, y: 1 })).toBe(true);
    expect(moved({ x: 3, y: 1 }, { x: 3, y: 2 })).toBe(true);
  });

  it("text panes are what the host calls text (view.rs Node::is_text): documents, rich text, logs, states, extensions; lists, trees, tables, entries and diffs are rows", () => {
    for (const n of ["list", "tree", "table", "entries", "diff"]) expect(selectKindOf({ node: n })).toBe("rows");
    for (const n of ["document", "rich", "log", "state"]) expect(selectKindOf({ node: n })).toBe("text");
    expect(selectKindOf({ node: "extension", capability: "work.map", fallback: { node: "list" } })).toBe("text");
    expect(selectKindOf(undefined)).toBe("rows");
  });
});

describe("a word as iTerm2 takes it", () => {
  const area: SelectArea = { x: 0, y: 0, w: 100, h: 1, id: "t", kind: "text" };
  const word = (line: string, at: string) => {
    const r = cellsOf(line);
    const w = wordAt(r, area, line.indexOf(at));
    return w ? r.slice(w[0], w[1] + 1).join("") : null;
  };
  it("a plain word, a path, a URL with its query, each whole (as select.rs word_range)", () => {
    expect(word("fix the build now", "build")).toBe("build");
    expect(word("see src/app/App.tsx:120 here", "App")).toBe("src/app/App.tsx");
    expect(word("see src/app/App.tsx:120 here", "120")).toBe("120");
    expect(word("a ::: b", ":")).toBe(":::");
    expect(word("open https://gitlab.com/acme/x/-/merge_requests/263?tab=diffs ok", "gitlab")).toBe("https://gitlab.com/acme/x/-/merge_requests/263?tab=diffs");
    expect(word("~/projects/gg-ink_live.sh", "gg")).toBe("~/projects/gg-ink_live.sh");
  });
  it("a sentence's full stop and brackets and quotes are not part of it, around a link too; a run of blanks is one", () => {
    expect(word("edit src/a.ts.", "src")).toBe("src/a.ts");
    expect(word("(see README.md)", "README")).toBe("README.md");
    expect(word('"quoted" word', "quoted")).toBe("quoted");
    expect(word("(https://x.io/a?b=1).", "x.io")).toBe("https://x.io/a?b=1");
    expect(wordAt(cellsOf("a  b"), area, 1)).toEqual([1, 2]);
  });
  it("a triple-click's line runs from the area's left edge to the end of what the row says", () => {
    const a: SelectArea = { x: 1, y: 0, w: 11, h: 1, id: "a", kind: "text" };
    expect(lineAt(cellsOf("│ see it    │"), a)).toEqual([1, 7]);
    expect(lineAt(cellsOf("│           │"), a)).toEqual([1, 1]);
  });
  it("wide characters belong to the word, and a press on a second cell finds it", () => {
    const r = cellsOf("x 界界ok y");
    expect(wordAt(r, area, 3)).toEqual([2, 7]);
    expect(wordAt(r, area, 2)).toEqual([2, 7]);
  });
  it("stays inside the pane", () => {
    const r = cellsOf("│abc││def│");
    expect(wordAt(r, { x: 1, y: 0, w: 3, h: 1, id: "a", kind: "text" }, 2)).toEqual([1, 3]);
  });
});

describe("shift extends the head inside the pane", () => {
  const s = sel(detail, 17, 2, 17, 2);
  it("a cell left and right, a row up and down, the row's ends (select.rs extend, extend_to_edge)", () => {
    expect(extend(s, "left").head).toEqual({ x: 16, y: 2 });
    expect(extend(s, "right").head).toEqual({ x: 18, y: 2 });
    expect(extend(s, "up").head).toEqual({ x: 17, y: 1 });
    expect(extend(s, "down").head).toEqual({ x: 17, y: 3 });
    expect(extend(s, "home").head).toEqual({ x: 13, y: 2 });
    expect(extend(s, "end").head).toEqual({ x: 27, y: 2 });
    // at the pane's edge it stays there, on its row
    expect(extend(sel(detail, 17, 2, 13, 2), "left").head).toEqual({ x: 13, y: 2 });
    expect(extend(sel(detail, 17, 2, 27, 2), "right").head).toEqual({ x: 27, y: 2 });
  });
  it("never leaves the pane", () => {
    expect(extend(sel(detail, 17, 1, 17, 1), "up").head).toEqual({ x: 17, y: 1 });
    expect(extend(sel(detail, 17, 3, 17, 3), "down").head).toEqual({ x: 17, y: 3 });
    expect(extend(sel(detail, 17, 1, 13, 1), "left").head).toEqual({ x: 13, y: 1 });
    expect(extend(sel(detail, 17, 3, 27, 3), "right").head).toEqual({ x: 27, y: 3 });
  });
});

describe("the copy is capped as the host caps it (ipc::MAX_CLIPBOARD)", () => {
  it("text over the cap is cut at a character", () => {
    expect(capped("abc")).toBe("abc");
    const long = "é".repeat(MAX_COPY_BYTES);
    const c = capped(long);
    expect(Buffer.byteLength(c, "utf8")).toBeLessThanOrEqual(MAX_COPY_BYTES);
    expect(c.length).toBe(MAX_COPY_BYTES / 2);
  });
});

// The selector: a press in an area is held until it moves or is let go.
const ev = (kind: MouseEvent["kind"], x: number, y: number, button: MouseEvent["button"] = "left"): MouseEvent => ({ kind, button, x, y, ctrl: false, alt: false, shift: false });
function harness(areas: SelectArea[] = [list, detail]) {
  let t = 1000;
  const log: string[] = [];
  const copies: string[] = [];
  const shown: Array<{ sel: Selection | null; dragging: boolean }> = [];
  const s = new Selector(
    {
      areaAt: (x, y) => areas.find((a) => x >= a.x && x < a.x + a.w && y >= a.y && y < a.y + a.h),
      row: rowAt,
      show: (sel, dragging) => shown.push({ sel, dragging }),
      copy: (text) => copies.push(text),
      click: (p, r) => log.push(`click ${p.x},${p.y}${r ? " up" : ""}`),
    },
    () => t,
  );
  return { s, log, copies, shown, at: (ms: number) => (t = ms), later: (ms: number) => (t += ms), showing: () => shown.at(-1)?.sel ?? null };
}

describe("the selector", () => {
  it("a press that does not move is a click when it is let go, a one-cell slip too; nothing is selected or copied", () => {
    const h = harness();
    expect(h.s.mouse(ev("press", 17, 1), false)).toBe(true);
    expect(h.log).toEqual([]); // held
    h.s.mouse(ev("motion", 18, 1), false);
    expect(h.s.mouse(ev("release", 18, 1), false)).toBe(true);
    expect(h.log).toEqual(["click 17,1 up"]);
    expect(h.copies).toEqual([]);
    expect(h.showing()).toBeNull();
  });

  it("a drag selects as it goes (the status line is told a drag is on), and letting go copies the pane's text and keeps it showing", () => {
    const h = harness();
    h.s.mouse(ev("press", 17, 1), false);
    h.s.mouse(ev("motion", 19, 2), false);
    expect(h.shown.at(-1)).toEqual({ sel: sel(detail, 17, 1, 19, 2), dragging: true });
    h.s.mouse(ev("release", 19, 2), false);
    expect(h.copies).toEqual(["the build\n  stage"]);
    expect(h.shown.at(-1)).toEqual({ sel: sel(detail, 17, 1, 19, 2), dragging: false });
    expect(h.log).toEqual([]); // no click
  });

  it("a drag that runs past the pane's left edge copies only that pane's text", () => {
    const h = harness();
    h.s.mouse(ev("press", 24, 2), false);
    h.s.mouse(ev("motion", 4, 2), false);
    h.s.mouse(ev("release", 2, 2), false);
    expect(h.copies).toEqual(["  stage test"]);
  });

  it("a drag over blanks only copies nothing and shows nothing", () => {
    const h = harness();
    h.s.mouse(ev("press", 1, 3), false);
    h.s.mouse(ev("motion", 9, 3), false);
    h.s.mouse(ev("release", 9, 3), false);
    expect(h.copies).toEqual([]);
    expect(h.showing()).toBeNull();
  });

  it("a press on no area is the controller's at once, a right press too, and either clears the selection", () => {
    const h = harness();
    h.s.mouse(ev("press", 17, 1), false);
    h.s.mouse(ev("motion", 22, 1), false);
    h.s.mouse(ev("release", 22, 1), false);
    expect(h.showing()).not.toBeNull();
    expect(h.s.mouse(ev("press", 12, 0), false)).toBe(false); // a border
    expect(h.showing()).toBeNull();
    h.s.mouse(ev("press", 17, 1), false);
    h.s.mouse(ev("motion", 22, 1), false);
    h.s.mouse(ev("release", 22, 1), false);
    expect(h.s.mouse(ev("press", 17, 1, "right"), false)).toBe(false);
    expect(h.showing()).toBeNull();
  });

  it("a click elsewhere in an area clears the selection and is a click", () => {
    const h = harness();
    h.s.mouse(ev("press", 17, 1), false);
    h.s.mouse(ev("motion", 22, 1), false);
    h.s.mouse(ev("release", 22, 1), false);
    h.s.mouse(ev("press", 3, 2), false);
    expect(h.showing()).toBeNull();
    h.s.mouse(ev("release", 3, 2), false);
    expect(h.log).toEqual(["click 3,2 up"]);
  });

  it("under an open menu nothing is selected: the press is the controller's", () => {
    const h = harness();
    expect(h.s.mouse(ev("press", 17, 1), true)).toBe(false);
  });

  it("in text a double-click selects the word and copies it, a triple-click the line; neither is a click", () => {
    const h = harness();
    h.s.mouse(ev("press", 14, 3), false);
    h.s.mouse(ev("release", 14, 3), false);
    h.later(100);
    h.s.mouse(ev("press", 14, 3), false);
    expect(h.showing()).toEqual(sel(detail, 13, 3, 20, 3));
    expect(h.copies).toEqual(["src/a.ts"]);
    h.s.mouse(ev("release", 14, 3), false);
    h.later(100);
    h.s.mouse(ev("press", 14, 3), false);
    expect(h.showing()).toEqual(sel(detail, 13, 3, 26, 3));
    expect(h.copies.at(-1)).toBe("src/a.ts:12 ok");
    h.s.mouse(ev("release", 14, 3), false);
    expect(h.log).toEqual(["click 14,3 up"]); // only the first press
  });

  it("two presses further apart than the double-click time, or on another cell, are two clicks", () => {
    const h = harness();
    h.s.mouse(ev("press", 14, 3), false);
    h.s.mouse(ev("release", 14, 3), false);
    h.later(500);
    h.s.mouse(ev("press", 14, 3), false);
    h.s.mouse(ev("release", 14, 3), false);
    h.s.mouse(ev("press", 15, 3), false);
    h.s.mouse(ev("release", 15, 3), false);
    expect(h.log).toEqual(["click 14,3 up", "click 14,3 up", "click 15,3 up"]);
    expect(h.copies).toEqual([]);
  });

  it("on rows a double-click is two clicks (the controller makes the second ⏎), no word", () => {
    const h = harness();
    h.s.mouse(ev("press", 2, 1), false);
    h.s.mouse(ev("release", 2, 1), false);
    h.s.mouse(ev("press", 2, 1), false);
    h.s.mouse(ev("release", 2, 1), false);
    expect(h.log).toEqual(["click 2,1 up", "click 2,1 up"]);
    expect(h.copies).toEqual([]);
  });

  it("keys with a selection showing: shift+arrows extend it, ctrl+c copies it again, any other key clears it and is the controller's", () => {
    const h = harness();
    h.s.mouse(ev("press", 17, 1), false);
    h.s.mouse(ev("motion", 19, 1), false);
    h.s.mouse(ev("release", 19, 1), false);
    expect(h.s.key({ key: "right" }, true, false)).toBe("took");
    expect(h.showing()).toEqual(sel(detail, 17, 1, 20, 1));
    expect(h.s.key({ key: "down" }, true, false)).toBe("took");
    expect(h.s.key({ key: { char: "c" }, ctrl: true }, false, false)).toBe("took");
    expect(h.showing()).toEqual(sel(detail, 17, 1, 20, 2));
    expect(h.copies.at(-1)).toBe("the build\n  stage");
    expect(h.s.key({ key: "down" }, false, false)).toBe("pass");
    expect(h.showing()).toBeNull();
    // with nothing selected every key is the controller's: ctrl+c too
    expect(h.s.key({ key: { char: "c" }, ctrl: true }, false, false)).toBe("pass");
    expect(h.s.key({ key: "right" }, true, false)).toBe("pass");
  });

  it("while the editor has the keys, or in the editor's own text, shift+arrows are the editor's: the selection clears", () => {
    const editor: SelectArea = { ...detail, id: "editor", editor: true };
    for (const [areas, capturing] of [[[editor], false], [[detail], true]] as const) {
      const h = harness([...areas]);
      h.s.mouse(ev("press", 17, 1), false);
      h.s.mouse(ev("motion", 19, 1), false);
      h.s.mouse(ev("release", 19, 1), false);
      expect(h.s.key({ key: "left" }, true, capturing)).toBe("pass");
      expect(h.showing()).toBeNull();
    }
  });

  it("a key before a held press is let go makes it a click first", () => {
    const h = harness();
    h.s.mouse(ev("press", 17, 1), false);
    expect(h.s.key({ key: "down" }, false, false)).toBe("pass");
    expect(h.log).toEqual(["click 17,1"]);
    expect(h.s.mouse(ev("release", 17, 1), false)).toBe(false);
  });

  it("a resize drops the selection", () => {
    const h = harness();
    h.s.mouse(ev("press", 17, 1), false);
    h.s.mouse(ev("motion", 19, 1), false);
    h.s.mouse(ev("release", 19, 1), false);
    h.s.resized();
    expect(h.showing()).toBeNull();
  });
});
