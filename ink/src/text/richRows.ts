import type { Inline, RichBlock, Tone } from "../nodes/view.js";
import { cellsOf, type Cell } from "./cells.js";
import { chipWordStyle } from "./chips.js";
import { tokenStyle, type PaintRun } from "./DiffView.js";
import { emojiOf } from "./emoji.js";
import { slotStyle, type Style } from "./look.js";
import { toneInk } from "./theme.js";
import { textPieces, trimPieces, wrapPieces, type Piece } from "./wrap.js";
import { cellWidth } from "../chrome/width.js";

/**
 * ui::rich::layout as rows of cells: a document (the host's `RichBlock`s, read from markdown by ui::markdown) laid out
 * `width` cells wide exactly as Ratatui lays out a description, line for line, in theme slots only. The dock's
 * transcript is rows of cells (it scrolls by rows), so it lays out an answer with this rather than RichDoc's Boxes.
 *
 * Cells carry no OSC 8 link, so a link's address follows its words, muted, where they differ (ui::rich::Look's `urls`,
 * Ratatui's own fallback for a terminal that cannot open the words). `painted` is the host's highlighting of the
 * document's code blocks (DockItem::Assistant `painted`): a block each in document order, a line of runs per line.
 */

/** ui::rich::Look: the style of each part of a document. */
interface Look {
  text: Style;
  heading: Style;
  summary: Style;
  quote: Style;
  bar: Style;
  code: Style;
  link: Style;
  mention: Style;
  me: Style;
  emoji: Style;
  muted: Style;
  marker: Style;
  rule: Style;
}

const lookNow = (): Look => ({
  text: slotStyle("text.primary"),
  heading: { ...slotStyle("text.emphasis"), bold: true },
  summary: { ...slotStyle("text.secondary"), bold: true },
  quote: slotStyle("text.secondary"),
  bar: slotStyle("accent.muted"),
  code: slotStyle("accent.secondary"),
  link: { ...slotStyle("accent.primary"), underline: true },
  mention: slotStyle("accent.tertiary"),
  me: { ...slotStyle("status.warning"), bold: true },
  emoji: slotStyle("text.secondary"),
  muted: slotStyle("text.muted"),
  marker: slotStyle("text.secondary"),
  rule: slotStyle("text.muted"),
});

/** ui::rich::Look::tones (widgets::tone): a call-out bar's slot; accent, warn and bad bold. */
const TONE_SLOT: Record<Tone, string> = {
  plain: "text.primary",
  muted: "text.muted",
  accent: "accent.primary",
  info: "status.info",
  good: "status.success",
  warn: "status.warning",
  bad: "status.error",
};
const toneStyle = (t: Tone): Style => ({ ...slotStyle(TONE_SLOT[t]), ...(t === "accent" || t === "warn" || t === "bad" ? { bold: true } : {}) });

/** ui::rich: a table column is squeezed to this before the table becomes a block per row; a rule is at most RULE long. */
const MIN_COLUMN = 6;
const RULE = 24;

type Role = "body" | "heading" | "summary" | "head";
type P = Piece<Style>;
type Ctx = { k: "quote" } | { k: "item"; marker: string; pending: boolean } | { k: "details"; body: boolean } | { k: "callout"; s: Style };

const isBlank = (row: Cell[]) => row.every((c) => c.ch.trim() === "" || c.ch === "│" || c.ch === "▌");

/** ui::rich::finish_line: cut to `width`, trailing blanks off. */
function finishLine(row: Cell[], width: number): Cell[] {
  const out: Cell[] = [];
  let used = 0;
  for (const c of row) {
    if (used + c.w > width) break;
    out.push(c);
    used += c.w;
  }
  while (out.length && out[out.length - 1]!.ch.trim() === "") out.pop();
  return out;
}

/** ui::rich::wrap: pieces to lines of cells at most `room` wide. */
const wrapCellsOf = (p: P[], room: number): Cell[][] => wrapPieces(p, room).map((l) => l.flatMap((r) => cellsOf(r.text, r.tag)));

/** ui::rich::hard_chunks on cells: cut into chunks at most `room` wide. */
function hardChunks(cs: Cell[], room: number): Cell[][] {
  const out: Cell[][] = [];
  let cur: Cell[] = [];
  let w = 0;
  for (const c of cs) {
    if (w > 0 && w + c.w > room) {
      out.push(cur);
      cur = [];
      w = 0;
    }
    cur.push(c);
    w += c.w;
  }
  if (cur.length) out.push(cur);
  return out;
}

class Lay {
  out: Cell[][] = [];
  lastBlank = false;
  noSep = false;
  ctx: Ctx[] = [];
  depth = 0;
  quote = 0;
  code = 0;
  constructor(
    private look: Look,
    private width: number,
    private painted: PaintRun[][][],
  ) {}

  styleOf(i: Inline, role: Role): Style {
    const l = this.look;
    let s: Style = role === "heading" ? l.heading : role === "summary" ? l.summary : this.quote > 0 ? l.quote : l.text;
    if (i.marks?.code) s = l.code;
    const k = i.kind ?? "text";
    if (typeof k === "object") {
      if ("link" in k) s = l.link;
      else if ("mention" in k) s = k.mention.me ? l.me : l.mention;
      else if ("image" in k) s = l.muted;
      else if ("badge" in k) s = chipWordStyle({ text: i.text, ink: toneInk(k.badge.tone), chip: true });
    } else if (k === "emoji") s = l.emoji;
    const m = i.marks;
    if (m?.bold || role === "head") s = { ...s, bold: true };
    if (m?.italic) s = { ...s, italic: true };
    if (m?.strike) s = { ...s, strike: true };
    if (m?.underline) s = { ...s, underline: true };
    return s;
  }

  pieces(text: Inline[], role: Role): P[] {
    const out: P[] = [];
    let open: { url: string; words: string } | null = null;
    text.forEach((i, n) => {
      const k = i.kind ?? "text";
      const url = typeof k === "object" && "link" in k ? k.link.url : undefined;
      if (url !== undefined) open = open && open.url === url ? { url, words: open.words + i.text } : { url, words: i.text };
      if (k === "break") out.push({ kind: "break" });
      else if (typeof k === "object" && "badge" in k) out.push({ kind: "text", text: ` ${i.text.split(/\s+/).filter(Boolean).join(" ")} `, tag: this.styleOf(i, role) });
      else out.push(...textPieces(k === "emoji" ? (emojiOf(i.text) ?? i.text) : i.text, this.styleOf(i, role)));
      const next = text[n + 1]?.kind;
      const nextUrl = typeof next === "object" && next && "link" in next ? next.link.url : undefined;
      const o = open as { url: string; words: string } | null;
      if (o && nextUrl !== o.url) {
        // No OSC 8 in a cell: the address after the words, where they differ.
        if (o.words.trim() !== o.url && o.words.trim() !== "") {
          out.push({ kind: "space", tag: this.look.muted }, { kind: "text", text: `(${o.url})`, tag: this.look.muted });
        }
        open = null;
      }
    });
    return out;
  }

  avail(): number {
    const used = this.ctx.reduce((n, c) => n + (c.k === "quote" || c.k === "callout" ? 2 : c.k === "item" ? cellWidth(c.marker) : c.body ? 2 : 0), 0);
    return Math.max(1, this.width - used);
  }

  prefix(consume: boolean): Cell[] {
    const out: Cell[] = [];
    for (const c of this.ctx) {
      if (c.k === "quote") out.push(...cellsOf("│ ", this.look.bar));
      else if (c.k === "callout") out.push(...cellsOf("▌ ", c.s));
      else if (c.k === "item") {
        if (c.pending && consume) {
          out.push(...cellsOf(c.marker, this.look.marker));
          c.pending = false;
        } else out.push(...cellsOf(" ".repeat(cellWidth(c.marker))));
      } else if (c.body) out.push(...cellsOf("  "));
    }
    return out;
  }

  blankLine() {
    this.out.push(finishLine(this.prefix(false), this.width));
    this.lastBlank = true;
  }

  sep() {
    if (this.out.length > 0 && !this.lastBlank && !this.noSep) this.blankLine();
  }

  emit(lines: Cell[][]) {
    for (const l of lines) {
      if (l.every((c) => c.ch.trim() === "")) {
        this.blankLine();
        continue;
      }
      this.out.push(finishLine([...this.prefix(true), ...l], this.width));
      this.lastBlank = false;
      this.noSep = false;
    }
  }

  rule(n: number) {
    this.emit([cellsOf("─".repeat(Math.max(1, n)), this.look.rule)]);
  }

  /** Pieces wrapped and written as one block; the widest line's width. */
  write(p: P[]): number {
    const t = trimPieces(p);
    if (t.length === 0) return 0;
    const lines = wrapCellsOf(t, this.avail());
    const widest = Math.max(0, ...lines.map((l) => l.reduce((n, c) => n + c.w, 0)));
    this.sep();
    this.emit(lines);
    return widest;
  }

  pop(k: Ctx["k"]) {
    for (let i = this.ctx.length - 1; i >= 0; i--)
      if (this.ctx[i]!.k === k) {
        this.ctx.length = i;
        return;
      }
  }

  blocks(bs: RichBlock[]) {
    for (const b of bs) this.block(b);
  }

  block(b: RichBlock) {
    switch (b.t) {
      case "para":
        this.write(this.pieces(b.text, "body"));
        break;
      case "heading": {
        const w = this.write(this.pieces(b.text, "heading"));
        if (b.level === 1 && w > 0) this.rule(w);
        break;
      }
      case "list":
        this.list(b.items);
        break;
      case "quote":
        this.sep();
        this.ctx.push({ k: "quote" });
        this.quote++;
        this.blocks(b.blocks);
        this.quote--;
        this.pop("quote");
        break;
      case "callout": {
        this.sep();
        this.ctx.push({ k: "callout", s: toneStyle(b.tone) });
        const blocks = b.blocks.map((x) => ({ ...x })) as RichBlock[];
        if (b.symbol) {
          const mark: Inline = { text: b.symbol };
          const first = blocks[0];
          if (first && (first.t === "para" || first.t === "heading")) blocks[0] = { ...first, text: [mark, ...first.text] };
          else blocks.unshift({ t: "para", text: [mark] });
        }
        this.blocks(blocks);
        this.pop("callout");
        break;
      }
      case "code":
        this.codeBlock(b.text, this.painted[this.code++]);
        break;
      case "table":
        this.table(b.aligns ?? [], b.head ?? [], b.rows);
        break;
      case "rule":
        this.sep();
        this.rule(Math.min(RULE, this.avail()));
        break;
      case "details": {
        const c: Ctx = { k: "details", body: false };
        this.ctx.push(c);
        this.write(this.pieces(b.summary, "summary"));
        c.body = true;
        this.blocks(b.blocks);
        this.pop("details");
        break;
      }
      case "media": {
        // No picture in a row of cells: ui::rich's words for one where none is placed.
        const p: P[] = textPieces(b.image ? "▣ image" : "▤ file", this.look.marker);
        if (b.name?.trim()) p.push({ kind: "space", tag: this.look.marker }, ...textPieces(b.name, this.look.marker));
        if (b.url) p.push({ kind: "space", tag: this.look.muted }, { kind: "text", text: b.url, tag: this.look.link });
        this.write(p);
        break;
      }
    }
  }

  list(items: Extract<RichBlock, { t: "list" }>["items"]) {
    if (this.depth === 0) this.sep();
    const top = this.depth === 0;
    this.depth++;
    for (const item of items) {
      const m = item.marker;
      const marker = typeof m === "object" ? `${m.number}. ` : m === "bullet" ? (top ? "• " : "◦ ") : m === "todo" ? "☐ " : m === "done" ? "☑ " : "◇ ";
      const c: Ctx = { k: "item", marker, pending: true };
      this.ctx.push(c);
      this.noSep = true;
      this.blocks(item.blocks);
      // An item with nothing in it still shows its marker.
      if (c.pending) {
        this.out.push(finishLine(this.prefix(true), this.width));
        this.lastBlank = false;
      }
      this.pop("item");
      this.noSep = false;
    }
    this.depth--;
  }

  codeBlock(text: string, painted?: PaintRun[][]) {
    const src = text.replace(/\r/g, "").split("\n");
    if (text.endsWith("\n")) src.pop();
    const lines = src.map((l) => l.replace(/\t/g, "    ").trimEnd());
    while (lines.length && lines[lines.length - 1]!.trim() === "") lines.pop();
    if (lines.length === 0) return;
    this.sep();
    const room = Math.max(1, this.avail() - 2);
    const rows: Cell[][] = [];
    lines.forEach((l, i) => {
      if (l === "") return void rows.push([]);
      const runs = painted?.[i];
      const cells = runs && runs.length ? runs.flatMap((r) => cellsOf(r.text.replace(/\t/g, "    "), tokenStyle(r.token))) : cellsOf(l, this.look.code);
      for (const chunk of hardChunks(cells, room)) rows.push([...cellsOf("  "), ...chunk]);
    });
    this.emit(rows);
  }

  table(aligns: string[], headIn: Inline[][], rowsIn: Inline[][][]) {
    const cellsOfRow = (row: Inline[][], role: Role) => row.map((c) => trimPieces(this.pieces(c, role)));
    const head = cellsOfRow(headIn, "head");
    const rows = rowsIn.map((r) => cellsOfRow(r, "body"));
    const n = Math.max(head.length, ...rows.map((r) => r.length));
    if (n === 0) return;
    const avail = this.avail();
    const natural = (cell: P[]) => Math.max(0, ...wrapPieces(cell, Number.MAX_SAFE_INTEGER / 4).map((l) => l.reduce((w, r) => w + cellWidth(r.text), 0)));
    const column = (c: number) => [...(head[c] ? [head[c]!] : []), ...rows.filter((r) => r[c]).map((r) => r[c]!)];
    const nat = Array.from({ length: n }, (_, c) => Math.max(1, ...column(c).map(natural)));
    const mins = nat.map((w) => Math.min(w, MIN_COLUMN));
    const gaps = 2 * (n - 1);
    this.sep();
    if (mins.reduce((a, b) => a + b, 0) + gaps > avail) return this.tableBlocks(head, rows);
    const longest = (cell: P[]) => Math.max(0, ...cell.map((p) => (p.kind === "text" ? cellWidth(p.text) : 0)));
    const keeps = Array.from({ length: n }, (_, c) => Math.min(nat[c]!, Math.max(mins[c]!, ...column(c).map(longest))));
    const ws = [...nat];
    for (const floors of [keeps, mins]) {
      while (ws.reduce((a, b) => a + b, 0) + gaps > avail) {
        let at = -1;
        // Iterator::max_by_key: the last of the widest.
        ws.forEach((w, i) => {
          if (w > floors[i]! && (at < 0 || w >= ws[at]!)) at = i;
        });
        if (at < 0) break;
        ws[at]!--;
      }
    }
    const rowLines = (cells: P[][]): Cell[][] => {
      const wrapped = ws.map((w, c) => (cells[c] ? wrapCellsOf(cells[c]!, w) : []));
      const height = Math.max(1, ...wrapped.map((x) => x.length));
      return Array.from({ length: height }, (_, i) => {
        const line: Cell[] = [];
        ws.forEach((w, c) => {
          if (c > 0) line.push(...cellsOf("  "));
          const seg = wrapped[c]![i] ?? [];
          const pad = Math.max(0, w - seg.reduce((x, s) => x + s.w, 0));
          const [left, right] = aligns[c] === "right" ? [pad, 0] : aligns[c] === "center" ? [pad >> 1, pad - (pad >> 1)] : [0, pad];
          line.push(...cellsOf(" ".repeat(left)), ...seg, ...cellsOf(" ".repeat(right)));
        });
        return line;
      });
    };
    if (head.length) {
      this.emit(rowLines(head));
      this.emit([cellsOf(ws.map((w) => "─".repeat(w)).join("  "), this.look.rule)]);
    }
    for (const r of rows) this.emit(rowLines(r));
  }

  /** A table with no room for columns: a block per row, `Heading: value`. */
  tableBlocks(head: P[][], rows: P[][][]) {
    const muted = this.look.rule;
    const avail = this.avail();
    rows.forEach((row, r) => {
      if (r > 0) this.blankLine();
      row.forEach((cell, c) => {
        if (cell.length === 0) return;
        const label = (head[c] ?? []).map((p) => (p.kind === "text" ? p.text : " ")).join("");
        const p: P[] = [];
        if (label.trim()) p.push({ kind: "text", text: `${label.trim()}:`, tag: muted }, { kind: "space", tag: muted });
        p.push(...cell);
        this.emit(wrapCellsOf(p, avail));
      });
    });
  }

  finish(): Cell[][] {
    while (this.out.length && isBlank(this.out[this.out.length - 1]!)) this.out.pop();
    let lead = 0;
    while (lead < this.out.length && isBlank(this.out[lead]!)) lead++;
    return this.out.slice(lead);
  }
}

/** A document laid out `width` cells wide, as ui::rich::layout lays it out (with link addresses shown). */
export function docRows(blocks: RichBlock[], width: number, painted: PaintRun[][][] = []): Cell[][] {
  const l = new Lay(lookNow(), Math.max(1, width), painted);
  l.blocks(blocks);
  return l.finish();
}

/** The rows' text, a line each (tests). */
export const rowsText = (rows: Cell[][]): string[] => rows.map((r) => r.map((c) => c.ch).join(""));
