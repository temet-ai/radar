/**
 * Selecting text with the mouse, inside a pane (issue #51; the host's selection rules).
 *
 * radar captures the mouse, so the terminal cannot select text; the renderer does it instead. A selection belongs to the
 * area the drag started in: a pane's inside (its border, title row and scrollbar column are not part of it), the dock's
 * transcript, the editor's text. It runs from the start cell to the area's right edge, then from its left edge on the
 * rows after, as a terminal does, with the area's edges in place of the screen's. A head dragged outside is clamped to
 * the area. The text copied is what the frame shows in those cells, each row trimmed of trailing padding, a wide
 * character once.
 *
 * Everything here is pure arithmetic over cells but `Selector`, which keeps the press, the clicks and the selection
 * showing, and tells its host what to show, copy and click.
 */
import type { Chord } from "../chrome/types.js";
import type { MouseEvent } from "./mouse.js";

export type Cell = { x: number; y: number };
export type Rect = { x: number; y: number; w: number; h: number };
/** Text: double-click selects a word, triple-click a line. Rows: a list, tree or table, where a double-click opens. */
export type SelectKind = "text" | "rows";
/** An area a selection can be made in, in screen cells. `editor`: the capturing editor (shift+arrows stay its own). */
export type SelectArea = Rect & { id: string; kind: SelectKind; editor?: boolean };
export type Selection = { area: SelectArea; anchor: Cell; head: Cell };
/** The cells of one row of the frame showing: a character per cell, "" for the second cell of a wide one. */
export type RowCells = readonly string[];

/** The node kinds that hold text to read (view.rs `Node::is_text`): a double-click there selects a word. In the others
 * (a list, tree, table, entries, a diff) a double-click opens the row (GUIDE 2.5). */
const TEXT_NODES = new Set(["document", "rich", "log", "state", "extension"]);

/** What a pane's body is to a selection, as the host marks it (view.rs `Node::is_text`). */
export function selectKindOf(node: any): SelectKind {
  return TEXT_NODES.has(node?.node) ? "text" : "rows";
}

/** The inside of a framed area `x,y,w,h` (its border and title row off, and `right` more columns at its right edge). */
export function insideOf(r: Rect, right = 0): Rect {
  return { x: r.x + 1, y: r.y + 1, w: Math.max(0, r.w - 2 - right), h: Math.max(0, r.h - 2) };
}

export const within = (r: Rect, x: number, y: number) => x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h;

/** The area at a cell, the last registered winning (an overlay registers after what it covers). */
export function areaAt(areas: readonly SelectArea[], x: number, y: number): SelectArea | undefined {
  for (let i = areas.length - 1; i >= 0; i--) if (areas[i]!.w > 0 && areas[i]!.h > 0 && within(areas[i]!, x, y)) return areas[i];
  return undefined;
}

/** A cell clamped to the area: a head dragged out of the pane stops at its nearest edge. */
export function clampTo(r: Rect, x: number, y: number): Cell {
  return { x: Math.min(r.x + r.w - 1, Math.max(r.x, x)), y: Math.min(r.y + r.h - 1, Math.max(r.y, y)) };
}

/** select.rs `moved`: onto another row, or two cells along. A hand that slips one cell while clicking still clicks. */
export function moved(anchor: Cell, head: Cell): boolean {
  return anchor.y !== head.y || Math.abs(anchor.x - head.x) >= 2;
}

/** Start and end in reading order. */
export function ends(s: Selection): [Cell, Cell] {
  const { anchor: a, head: h } = s;
  return h.y < a.y || (h.y === a.y && h.x < a.x) ? [h, a] : [a, h];
}

/** The cells the selection covers, row by row: x from..to inclusive, inside its area. */
export function spans(s: Selection): Array<{ y: number; from: number; to: number }> {
  const [a, e] = ends(s);
  const left = s.area.x;
  const right = s.area.x + s.area.w - 1;
  const out: Array<{ y: number; from: number; to: number }> = [];
  for (let y = a.y; y <= e.y; y++) {
    const from = y === a.y ? a.x : left;
    const to = y === e.y ? e.x : right;
    if (to >= from) out.push({ y, from: Math.max(left, from), to: Math.min(right, to) });
  }
  return out;
}

/** select.rs `text`: one line per row, without the spaces that pad a row out to the edge; a wide character once (a span
 * that starts on its second cell takes it whole). */
export function textOf(s: Selection, rowAt: (y: number) => RowCells): string {
  const lines: string[] = [];
  for (const { y, from, to } of spans(s)) {
    const row = rowAt(y);
    let x = from;
    while (x > s.area.x && row[x] === "") x--;
    let line = "";
    for (; x <= to && x < row.length; x++) line += row[x] ?? "";
    lines.push(line.trimEnd());
  }
  return lines.join("\n");
}

/** One character of a row inside an area and the cells it covers (select.rs `Glyph`). */
type Glyph = { x: number; to: number; ch: string };

function glyphs(row: RowCells, r: Rect): Glyph[] {
  const out: Glyph[] = [];
  const right = r.x + r.w;
  for (let x = r.x; x < right; ) {
    const c = row[x] ?? " ";
    let w = 1;
    while (x + w < right && row[x + w] === "") w++;
    // a wide character cut by the area's left edge is a blank
    out.push({ x, to: x + w - 1, ch: c === "" ? " " : ([...c][0] ?? " ") });
    x += w;
  }
  return out;
}

type Class = "space" | "word" | "other";
const LETTER = /[\p{L}\p{N}]/u;
/** select.rs `class`: letters, digits and what joins them in a path or a name, as iTerm2's word characters do
 * (`/-+\~_.`), so a file path selects whole. */
function classOf(ch: string): Class {
  if (/\s/u.test(ch) || ch === "\0") return "space";
  if (LETTER.test(ch) || "/-+\\~_.".includes(ch)) return "word";
  return "other";
}

/** The glyph run around `i` whose characters `of` takes. */
function run(g: Glyph[], i: number, of: (ch: string) => boolean): [number, number] {
  let s = i;
  while (s > 0 && of(g[s - 1]!.ch)) s--;
  let e = i;
  while (e + 1 < g.length && of(g[e + 1]!.ch)) e++;
  return [s, e];
}

/** select.rs `word_range`: a run of word characters, of spaces, or of one other character; a link (a token with `://`)
 * runs to the next space, less the brackets, quotes and stops around it; a stop or dash after a word ends it. */
function wordRange(g: Glyph[], i: number): [number, number] {
  const here = classOf(g[i]!.ch);
  if (here === "space") return run(g, i, (c) => classOf(c) === "space");
  const [ts, te] = run(g, i, (c) => classOf(c) !== "space");
  if (g.slice(ts, te + 1).map((x) => x.ch).join("").includes("://")) {
    let s = ts;
    let e = te;
    while (s < e && "([{<\"'`".includes(g[s]!.ch)) s++;
    while (e > s && ")]}>\"'`.,;:!?".includes(g[e]!.ch)) e--;
    if (i >= s && i <= e) return [s, e];
  }
  if (here === "word") {
    const [s, e0] = run(g, i, (c) => classOf(c) === "word");
    let e = e0;
    while (e > Math.max(s, i) && (g[e]!.ch === "." || g[e]!.ch === "-")) e--;
    return [s, e];
  }
  const same = g[i]!.ch;
  return run(g, i, (c) => c === same);
}

/** The word at `x` on a row, inside the area, as a double-click selects it (select.rs `word`): [from, to] cells
 * inclusive; none outside the area. */
export function wordAt(row: RowCells, r: Rect, x: number): [number, number] | null {
  const g = glyphs(row, r);
  const i = g.findIndex((c) => c.x <= x && x <= c.to);
  if (i < 0) return null;
  const [s, e] = wordRange(g, i);
  return [g[s]!.x, g[e]!.to];
}

/** The line at row `y` as a triple-click selects it (select.rs `line`): from the area's left edge to the end of what
 * the row says. */
export function lineAt(row: RowCells, r: Rect): [number, number] | null {
  const g = glyphs(row, r);
  if (g.length === 0) return null;
  let last = -1;
  for (let k = g.length - 1; k >= 0; k--)
    if (classOf(g[k]!.ch) !== "space") {
      last = k;
      break;
    }
  return [g[0]!.x, last < 0 ? g[0]!.x : g[last]!.to];
}

export type ExtendMove = "left" | "right" | "up" | "down" | "home" | "end";

/** select.rs `extend` and `extend_to_edge`: shift+←/→ move the head a cell, shift+↑/↓ a row, shift+home/end to the
 * row's ends, always inside the area. */
export function extend(s: Selection, move: ExtendMove): Selection {
  const r = s.area;
  let { x, y } = s.head;
  if (move === "left") x--;
  else if (move === "right") x++;
  else if (move === "up") y--;
  else if (move === "down") y++;
  else if (move === "home") x = r.x;
  else x = r.x + r.w - 1;
  return { ...s, head: clampTo(r, x, y) };
}

/** What the selection's owner does for it. */
export interface SelectHost {
  /** The area a press at the cell starts a selection in (none: the press is the controller's as before). */
  areaAt(x: number, y: number): SelectArea | undefined;
  /** The cells of row `y` of the frame showing. */
  row(y: number): RowCells;
  /** Show the selection (null: none), and whether a drag is making it now (the status line names the terminal's own
   * selection then). */
  show(sel: Selection | null, dragging: boolean): void;
  /** Copy the text (the host's Intent::Copy). */
  copy(text: string): void;
  /** A press held until it proved to be a click: handle it now, and its release. */
  click(press: MouseEvent, release?: MouseEvent): void;
}

/** Two presses on one cell within this many milliseconds are one double-click (as the controller's rows). */
export const MULTI_CLICK_MS = 400;
/** The host caps what it copies (ipc::MAX_CLIPBOARD, bytes); a selection longer than that is cut at a character. */
export const MAX_COPY_BYTES = 1024 * 1024;

export function capped(text: string): string {
  if (Buffer.byteLength(text, "utf8") <= MAX_COPY_BYTES) return text;
  let out = "";
  let bytes = 0;
  for (const c of text) {
    const b = Buffer.byteLength(c, "utf8");
    if (bytes + b > MAX_COPY_BYTES) break;
    out += c;
    bytes += b;
  }
  return out;
}

/**
 * The selection's state: a left press inside an area is held until it moves (a selection) or is let go (a click,
 * handed back through `host.click`). A second press on the same cell in text selects the word, a third the line; on
 * rows they are clicks (the controller makes two of them ⏎). Keys: shift+arrows extend the head, ctrl+c copies again,
 * any other key clears it.
 */
export class Selector {
  private sel: Selection | null = null;
  private held: { press: MouseEvent; area: SelectArea; selecting: boolean } | null = null;
  /** A word or line just selected by clicks: its release is not a click. */
  private swallow = false;
  private clicks: { x: number; y: number; at: number; n: number } | null = null;

  constructor(
    private readonly host: SelectHost,
    private readonly now: () => number = Date.now,
  ) {}

  get selection(): Selection | null {
    return this.sel;
  }
  /** A press is held: the pointer reports after it are the selection's until its release. */
  get pressing(): boolean {
    return this.held !== null || this.swallow;
  }

  /** Nothing selected and no press held (a key between two clicks makes them two clicks). */
  clear() {
    this.clicks = null;
    if (this.sel === null && !this.held) return;
    this.flush();
    this.set(null, false);
  }

  /** A press still held (a key came before its release) is a click after all. */
  flush() {
    const h = this.held;
    this.held = null;
    this.swallow = false;
    if (h && !h.selecting) this.host.click(h.press);
  }

  private set(sel: Selection | null, dragging: boolean) {
    this.sel = sel;
    this.host.show(sel, dragging);
  }

  /** A pointer report. True when the selector took it; false: the controller handles it as before. */
  mouse(ev: MouseEvent, menuOpen: boolean): boolean {
    if (ev.kind === "press") {
      // A press anywhere clears what is selected; a left press inside an area may start a new one.
      const had = this.sel !== null;
      if (this.held) this.flush();
      this.swallow = false;
      const area = ev.button === "left" && !menuOpen ? this.host.areaAt(ev.x, ev.y) : undefined;
      if (!area) {
        this.clicks = null;
        if (had) this.set(null, false);
        return false;
      }
      const t = this.now();
      const c = this.clicks;
      const n = c && c.x === ev.x && c.y === ev.y && t - c.at <= MULTI_CLICK_MS ? (c.n % 3) + 1 : 1;
      this.clicks = { x: ev.x, y: ev.y, at: t, n };
      if (area.kind === "text" && n >= 2) {
        const sel = n === 2 ? this.word(area, ev.x, ev.y) : this.line(area, ev.y);
        this.swallow = true;
        this.set(sel, false);
        if (sel) this.copy();
        return true;
      }
      if (had) this.set(null, false);
      this.held = { press: ev, area, selecting: false };
      return true;
    }
    if (this.swallow && (ev.kind === "release" || (ev.kind === "motion" && ev.button === "left"))) {
      if (ev.kind === "release") this.swallow = false;
      return true;
    }
    const h = this.held;
    if (!h) return false;
    if (ev.kind === "motion") {
      if (ev.button !== "left") return false;
      const head = clampTo(h.area, ev.x, ev.y);
      const anchor = { x: h.press.x, y: h.press.y };
      if (!h.selecting && !moved(anchor, head)) return true;
      h.selecting = true;
      this.clicks = null;
      const sel = { area: h.area, anchor, head };
      if (!this.sel || this.sel.head.x !== head.x || this.sel.head.y !== head.y) this.set(sel, true);
      return true;
    }
    if (ev.kind === "release") {
      this.held = null;
      if (!h.selecting) {
        this.host.click(h.press, ev);
        return true;
      }
      const sel = { area: h.area, anchor: { x: h.press.x, y: h.press.y }, head: clampTo(h.area, ev.x, ev.y) };
      if (!moved(sel.anchor, sel.head)) {
        this.set(null, false);
        return true;
      }
      const text = textOf(sel, (y) => this.host.row(y));
      if (text.trim() === "") {
        this.set(null, false);
        return true;
      }
      this.set(sel, false);
      this.host.copy(capped(text));
      return true;
    }
    // the wheel while a press is held is the controller's
    return false;
  }

  /** A key while a selection shows: "took" when it was the selection's (shift extends, ctrl+c copies again); otherwise
   * the selection is cleared and the key is the controller's. `capturing`: the editor has the keys, shift+arrows are
   * its own selection. */
  key(chord: Chord, shift: boolean, capturing: boolean): "took" | "pass" {
    if (this.held) this.flush();
    this.clicks = null;
    const sel = this.sel;
    if (!sel) return "pass";
    if (chord.ctrl && !chord.alt && typeof chord.key === "object" && chord.key.char === "c") {
      this.copy();
      return "took";
    }
    const k = chord.key;
    if (shift && !chord.ctrl && !chord.alt && !capturing && !sel.area.editor && typeof k === "string" && (["left", "right", "up", "down", "home", "end"] as string[]).includes(k)) {
      this.set(extend(sel, k as ExtendMove), false);
      return "took";
    }
    this.set(null, false);
    return "pass";
  }

  /** The wheel: what is under a selection scrolls away, so it ends (runner.rs: any other key or the wheel). A drag
   * still held goes on. */
  wheel() {
    if (this.held) return;
    this.clicks = null;
    if (this.sel) this.set(null, false);
  }

  /** The frame was laid out anew at another size: a selection in screen cells means nothing there. */
  resized() {
    this.held = null;
    this.swallow = false;
    this.clicks = null;
    if (this.sel) this.set(null, false);
  }

  private copy() {
    if (!this.sel) return;
    const text = textOf(this.sel, (y) => this.host.row(y));
    if (text.trim() !== "") this.host.copy(capped(text));
  }

  private word(area: SelectArea, x: number, y: number): Selection | null {
    const w = wordAt(this.host.row(y), area, x);
    return w ? { area, anchor: { x: w[0], y }, head: { x: w[1], y } } : null;
  }

  private line(area: SelectArea, y: number): Selection | null {
    const l = lineAt(this.host.row(y), area);
    return l ? { area, anchor: { x: l[0], y }, head: { x: l[1], y } } : null;
  }
}
