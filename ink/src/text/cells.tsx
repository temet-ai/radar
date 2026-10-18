import React, { createContext, useContext, useRef } from "react";
import { Box, type BoxProps } from "ink";
import chalk from "chalk";
import { foregroundColorNames } from "chalk";

/** Ink's colorize (not exported by the package): a named colour, #hex, rgb(r,g,b) or ansi256(n). */
function colorize(
  text: string,
  color: string | undefined,
  type: "foreground" | "background",
): string {
  if (!color) return text;
  const fg = type === "foreground";
  if ((foregroundColorNames as readonly string[]).includes(color))
    return (chalk as any)[
      fg ? color : `bg${color[0]!.toUpperCase()}${color.slice(1)}`
    ](text);
  if (color.startsWith("#"))
    return fg ? chalk.hex(color)(text) : chalk.bgHex(color)(text);
  const a = /^ansi256\(\s?(\d+)\s?\)$/.exec(color);
  if (a)
    return fg ? chalk.ansi256(+a[1]!)(text) : chalk.bgAnsi256(+a[1]!)(text);
  const r = /^rgb\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*\)$/.exec(color);
  if (r)
    return fg
      ? chalk.rgb(+r[1]!, +r[2]!, +r[3]!)(text)
      : chalk.bgRgb(+r[1]!, +r[2]!, +r[3]!)(text);
  return text;
}
import type { Text } from "../nodes/view.js";
import { cellWidth, charWidth, graphemeCells } from "../chrome/width.js";
import { currentLook, type Look, type Style } from "./look.js";
import { segRunStyle, segRunWords } from "./TextLine.js";
import { chipRuns, chipWordStyle } from "./chips.js";
import { graphemes } from "../editor/graphemes.js";
import { mnemonicRange, type Mnemonic } from "./mnemonic.js";

/** One terminal cell as Ratatui's buffer holds it: a character and its style. A wide character is one `Cell` of
 * width 2 (the cell after it is part of it). Rows built from cells are exactly as wide as asked, so backgrounds,
 * bands and selection reach every cell the way `Buffer::set_style` does. */
export interface Cell {
  ch: string;
  w: number;
  s: Style;
}

const KEYS = [
  "color",
  "backgroundColor",
  "bold",
  "dim",
  "italic",
  "underline",
  "inverse",
  "strike",
] as const;

/** Style::patch: what `over` sets replaces what `base` has; modifiers add. */
export function patch(base: Style, over: Style): Style {
  const out: Style = { ...base };
  for (const k of KEYS)
    if (over[k] !== undefined) (out as Record<string, unknown>)[k] = over[k];
  return out;
}

const sameStyle = (a: Style, b: Style) => KEYS.every((k) => a[k] === b[k]);

export function cellsOf(text: string, s: Style = {}): Cell[] {
  const out: Cell[] = [];
  for (const ch of graphemes(text)) {
    const w = graphemeCells(ch);
    // a combining mark (width 0) belongs to the cell before it
    if (w === 0 && out.length > 0) out[out.length - 1]!.ch += ch;
    else if (w > 0 || /\p{Mark}|\u200d/u.test(ch)) out.push({ ch, w, s });
  }
  return out;
}

export const widthOfCells = (cs: Cell[]) => cs.reduce((n, c) => n + c.w, 0);

/** ui::clip / ui::take: cut to `w` cells; a wide character that would cross the edge is left out whole. */
export function clipCells(cs: Cell[], w: number): Cell[] {
  const out: Cell[] = [];
  let used = 0;
  for (const c of cs) {
    if (used + c.w > w) break;
    out.push(c);
    used += c.w;
  }
  return out;
}

/** Pad with spaces (in `s`) to `w` cells. */
export function padCells(cs: Cell[], w: number, s: Style = {}): Cell[] {
  const n = widthOfCells(cs);
  return n >= w ? cs : [...cs, ...cellsOf(" ".repeat(w - n), s)];
}

/** ui::fit on cells: cut with a closing `…` (in the last cell's style) when wider than `w`. */
export function ellipsize(cs: Cell[], w: number): Cell[] {
  if (widthOfCells(cs) <= w) return cs;
  if (w <= 0) return [];
  const head = clipCells(cs, w - 1);
  const style = cs[head.length - 1]?.s ?? cs[0]?.s ?? {};
  return [...head, { ch: "…", w: 1, s: style }];
}

export function patchCells(cs: Cell[], over: Style): Cell[] {
  return cs.map((c) => ({ ...c, s: patch(c.s, over) }));
}

/** The cells of a line of view text (icons from the pack, chips on their shade), as view_tui::spans lays it out. */
export function lineCells(text: Text, plain?: boolean, literal = false): Cell[] {
  const out: Cell[] = [];
  for (const seg of text) {
    if (seg.chip && seg.icon && !plain) {
      for (const r of chipRuns(seg, literal)) out.push(...cellsOf(r.text, r.style));
      continue;
    }
    const words = segRunWords(seg, plain, literal);
    // widgets::chip: a chip with no icon is ` words ` on the overlay shade too, not only the labelled one.
    let style = plain ? {} : seg.chip ? chipWordStyle(seg) : segRunStyle(seg);
    if (!plain && seg.marked) style = { ...style, inverse: true };
    if (words) out.push(...cellsOf(words, style));
  }
  return joinCombiningCells(out);
}

function joinCombiningCells(cells: Cell[]): Cell[] {
  if (!cells.some((c) => /[^\x20-\x7e]/.test(c.ch))) return cells;
  // A source style boundary may split one grapheme (an accent, flag or joined emoji).
  // Walk source cells once; retain the base scalar's colours and compose modifiers.
  const out: Cell[] = [];
  let source = 0, sourceAt = 0, at = 0;
  for (const g of graphemes(cells.map((c) => c.ch).join(""))) {
    const end = at + g.length;
    while (source < cells.length && sourceAt + cells[source]!.ch.length <= at) {
      sourceAt += cells[source]!.ch.length; source++;
    }
    const base = cells[source];
    if (!base) break;
    const style = { ...base.s };
    let scan = source, scanAt = sourceAt;
    while (scan < cells.length && scanAt < end) {
      for (const key of ["bold", "dim", "italic", "underline", "inverse", "strike"] as const) if (cells[scan]!.s[key]) style[key] = true;
      scanAt += cells[scan]!.ch.length; scan++;
    }
    out.push({ ch: g, w: graphemeCells(g), s: style });
    at = end;
  }
  return out;
}

/** Rich-line mnemonic offsets count all source text, including replaceable icon fallbacks. */
export function mnemonicLineCells(text: Text, keys: string, meta: Mnemonic, plain?: boolean): { cells: Cell[]; valid: boolean } {
  const source = text.map((s) => s.text ?? "").join("");
  const range = mnemonicRange(source, keys, meta.mnemonic_at);
  let at = 0;
  const eligible = range && text.some((seg) => {
    const start = at;
    at += [...(seg.text ?? "")].length;
    return meta.mnemonic_at! >= start && meta.mnemonic_at! < at && (!seg.icon || seg.chip);
  });
  if (!range || !eligible || plain) return { cells: lineCells(text, plain, !!keys), valid: !!range && !!eligible };
  at = 0;
  const cells: Cell[] = [];
  for (const seg of text) {
    const raw = seg.text ?? "";
    const start = at;
    at += [...raw].length;
    const runs = lineCells([seg], false, true);
    if ((!seg.icon || seg.chip) && start < range[1] && at > range[0]) {
      // Glyphs and chip padding consume display cells but no source scalars.
      const words = segRunWords(seg, false, true);
      const rawAt = words.indexOf(raw);
      const before = rawAt < 0 ? 0 : [...words.slice(0, rawAt)].length;
      let scalar = -before + start;
      for (const cell of runs) {
        const end = scalar + [...cell.ch].length;
        if (scalar < range[1] && end > range[0] && end > start && scalar < at) cell.s = { ...cell.s, underline: true };
        scalar = end;
      }
    }
    cells.push(...runs);
  }
  return { cells: joinCombiningCells(cells), valid: true };
}

const MARK = "\u0001";
const paramsOf = (t: string): string => {
  const at = t.indexOf(MARK);
  return [...t.slice(0, at).matchAll(/\x1b\[([0-9;]*)m/g)]
    .map((m) => m[1])
    .filter(Boolean)
    .join(";");
};

/** A style as the SGR parameters chalk (so Ink's <Text>) would use for it, split into colours and attributes. */
export interface Info {
  fg: string;
  bg: string;
  attr: number;
  /** fast-output's interned style id for this info */
  sid?: number;
}
const ATTRS: [keyof Style, number, (t: string) => string, number][] = [
  ["bold", 1, (t) => chalk.bold(t), 22],
  ["dim", 2, (t) => chalk.dim(t), 22],
  ["italic", 4, (t) => chalk.italic(t), 23],
  ["underline", 8, (t) => chalk.underline(t), 24],
  ["inverse", 16, (t) => chalk.inverse(t), 27],
  ["strike", 32, (t) => chalk.strikethrough(t), 29],
];
const attrOpen = new Map<number, string>();
for (const [, bit, f] of ATTRS) attrOpen.set(bit, paramsOf(f(MARK)));
const infos = new Map<string, Info>();
const DEFAULT: Info = { fg: "", bg: "", attr: 0 };
function infoOf(s: Style, plain?: boolean, bg?: string): Info {
  const key = `${plain ? 1 : 0}|${s.color ?? ""}|${(plain ? bg : (s.backgroundColor ?? bg)) ?? ""}|${s.bold ? 1 : 0}${s.dim ? 1 : 0}${s.italic ? 1 : 0}${s.underline ? 1 : 0}${s.inverse ? 1 : 0}${s.strike ? 1 : 0}`;
  let i = infos.get(key);
  if (i) return i;
  let attr = 0;
  for (const [k, bit] of ATTRS)
    if (s[k] && (k !== "dim" || !plain) && attrOpen.get(bit)) attr |= bit;
  i = {
    fg: plain ? "" : paramsOf(colorize(MARK, s.color, "foreground")),
    bg: paramsOf(
      colorize(MARK, plain ? bg : (s.backgroundColor ?? bg), "background"),
    ),
    attr,
  };
  infos.set(key, i);
  return i;
}
const moves = new Map<Info, Map<Info, string>>();
function move(a: Info, b: Info): string {
  let m = moves.get(a);
  if (!m) moves.set(a, (m = new Map()));
  let r = m.get(b);
  if (r !== undefined) return r;
  const p: string[] = [];
  // attributes that go off (22 clears bold and dim together, so the other is set again below)
  let have = a.attr;
  for (const [, bit, , off] of ATTRS)
    if (have & bit && !(b.attr & bit)) {
      p.push(String(off));
      have &= off === 22 ? ~3 : ~bit;
    }
  for (const [, bit] of ATTRS)
    if (b.attr & bit && !(have & bit)) p.push(attrOpen.get(bit)!);
  if (a.bg !== b.bg) p.push(b.bg || "49");
  if (a.fg !== b.fg) p.push(b.fg || "39");
  r = p.length ? `\x1b[${p.join(";")}m` : "";
  m.set(b, r);
  return r;
}

/** Writes a line's SGR as the fewest escapes: `to(style)` gives only what changes from the style before. Ink scans every
 * escape of every Text on every frame (sanitize, tokenize, width), so fewer and shorter ones are cheaper. */
export class Pen {
  private cur = DEFAULT;
  constructor(
    private plain?: boolean,
    private bg?: string,
  ) {}
  to(s: Style): string {
    return this.toInfo(infoOf(s, this.plain, this.bg));
  }
  toInfo(n: Info): string {
    if (n === this.cur) return "";
    const r = move(this.cur, n);
    this.cur = n;
    return r;
  }
  /** Back to the default style. */
  end(): string {
    const from = this.cur;
    this.cur = DEFAULT;
    return move(from, DEFAULT);
  }
}

/** The background of the nearest <BgBox> above: a Box's backgroundColor is where cells and runs with no background of
 * their own sit (Ink's <Text> inherited it through its own context, which a row of plain text with style runs cannot). */
export const BgContext = createContext<string | undefined>(undefined);
export const useBg = () => useContext(BgContext);
/** <Box> that tells the rows inside it which background they sit on. */
export function BgBox(props: BoxProps & { children?: React.ReactNode }) {
  const inherited = useContext(BgContext);
  return (
    <BgContext value={props.backgroundColor || inherited}>
      <Box {...props} />
    </BgContext>
  );
}

/** The style runs of the lines of a Text: where, in cells from the line's start, each style begins. The text itself is
 * plain (no escape codes), so Ink's sanitize, width and wrap see only characters, and fast-output.ts reads `rows` off the
 * transformer to colour the cells it writes (ink/src/app/fast-output.ts). */
export interface RowRuns {
  cols: number[];
  infos: Info[];
  /** The characters that are not printable ASCII, by column: Ink measures every Text with string-width on every frame,
   * and a line of plain ASCII takes its fast path, so these are written as `?` (of the same width) and put back at
   * write time (fast-output.ts, and the fallback transformer below). */
  g: Array<[number, string, number]>;
}
type Transformer = ((line: string, index: number) => string) & {
  radarRows?: RowRuns[];
};

const ASCII = /^[\x20-\x7e]*$/;

/** Builds the lines of one Ink <Text> as plain text plus style runs. */
export class Rows {
  private lines: string[] = [];
  private rawLines: string[] = [];
  private raw: string[] = [];
  private marks = false; // a zero-width character: the text goes to Ink as it is, with no glyph table
  private rows: RowRuns[] = [];
  private cur: string[] | null = null;
  private runs: RowRuns | null = null;
  private col = 0;
  private last: Info | null = null;
  constructor(
    private plain?: boolean,
    private bg?: string,
  ) {}
  line(): this {
    this.flush();
    this.cur = [];
    this.raw = [];
    this.runs = { cols: [], infos: [], g: [] };
    this.col = 0;
    this.last = null;
    return this;
  }
  private flush() {
    if (this.cur) {
      // an empty line is one blank, in the background it sits on
      if (this.cur.length === 0) this.put(" ", {}, 1);
      this.lines.push(this.cur.join(""));
      this.rawLines.push(this.raw.join(""));
      this.rows.push(this.runs!);
    }
  }
  /** `text` of cell width `w` (counted when not given) in `style`. */
  put(text: string, style: Style, w?: number): this {
    if (!this.cur) this.line();
    if (text === "") return this;
    const info = infoOf(style, this.plain, this.bg);
    if (info !== this.last) {
      this.runs!.cols.push(this.col);
      this.runs!.infos.push(info);
      this.last = info;
    }
    if (ASCII.test(text)) {
      this.cur!.push(text);
      this.raw.push(text);
      this.col += text.length;
      return this;
    }
    let a = "";
    for (const ch of graphemes(text)) {
      const cp = ch.codePointAt(0)!;
      if (ch.length === 1 && cp >= 0x20 && cp < 0x7f) {
        a += ch;
        this.col++;
        continue;
      }
      const cw = graphemeCells(ch);
      if (cw === 0) {
        this.marks = true;
        a += ch;
        continue;
      }
      this.runs!.g.push([this.col, ch, cw]);
      a += "?".repeat(cw);
      this.col += cw;
    }
    this.cur!.push(a);
    this.raw.push(text);
    void w;
    return this;
  }
  /** A premeasured complete grapheme: preserve its cluster and cell width in the fast-output glyph table. */
  cell(cell: Cell): this {
    if (ASCII.test(cell.ch)) return this.put(cell.ch, cell.s, cell.w);
    if (!this.cur) this.line();
    const info = infoOf(cell.s, this.plain, this.bg);
    if (info !== this.last) {
      this.runs!.cols.push(this.col); this.runs!.infos.push(info); this.last = info;
    }
    this.runs!.g.push([this.col, cell.ch, cell.w]);
    this.cur!.push("?".repeat(cell.w)); this.raw.push(cell.ch); this.col += cell.w;
    return this;
  }
  /** The characters as drawn, without styles. */
  realText(): string {
    this.flush();
    this.cur = null;
    return this.rawLines.join("\n");
  }
  /** The text handed to Ink (non-ASCII written as `?`, see RowRuns.g). */
  plainText(): string {
    this.flush();
    this.cur = null;
    return (this.marks ? this.rawLines : this.lines).join("\n");
  }
  element(
    wrap: "truncate" | "truncate-end" = "truncate",
  ): React.ReactElement | null {
    const text = this.plainText();
    const rows = this.rows;
    if (this.marks) for (const r of rows) r.g = [];
    if (this.lines.length === 0) return null;
    // Ink's own get() (the fallback) applies this per line; fast-output.ts reads `radarRows` instead.
    const fn: Transformer = (line, index) => {
      const r = rows[index];
      if (!r || r.infos.length === 0) return line;
      const pen = new Pen();
      let out = "";
      let col = 0;
      let k = 0;
      let gi = 0;
      let skip = 0;
      for (const ch of line) {
        while (k < r.cols.length && r.cols[k]! <= col)
          out += pen.toInfo(r.infos[k++]!);
        if (skip > 0 && ch === "?") {
          skip--;
          col++;
          continue;
        }
        skip = 0;
        const g = ch === "?" ? r.g[gi] : undefined;
        if (g && g[0] === col) {
          gi++;
          out += g[1];
          skip = g[2] - 1;
          col++;
          continue;
        }
        out += ch;
        col += charWidth(ch.codePointAt(0)!);
      }
      return out + pen.end();
    };
    fn.radarRows = rows;
    return React.createElement(
      "ink-text" as never,
      {
        style: {
          flexGrow: 0,
          flexShrink: 1,
          flexDirection: "row",
          textWrap: wrap,
        },
        internal_transform: fn,
      } as never,
      text,
    );
  }
}

/** The rows a node drew last time, by a key that says everything the row is made from (its view data, the room, the
 * selection, the focus): a row whose key is unchanged comes back as the very same array, so a CellBlock of such rows
 * is the same element and React, Yoga and Ink's measuring skip it. A look from the host starts the cache again (the
 * cells hold its colours and glyphs). Keys not asked for in a render are dropped. */
export const rowCacheStats = { made: 0 };
export function useRowCache(): (key: string, make: () => Cell[]) => Cell[] {
  const ref = useRef<{ look: Look; rows: Map<string, Cell[]> } | null>(null);
  const look = currentLook();
  const prev = ref.current?.look === look ? ref.current.rows : undefined;
  const rows = new Map<string, Cell[]>();
  ref.current = { look, rows };
  return (key, make) => {
    let r = rows.get(key) ?? prev?.get(key);
    if (r === undefined) {
      r = make();
      rowCacheStats.made++;
    }
    rows.set(key, r);
    return r;
  };
}

/** A row of cells as one Ink <Text> of plain characters with style runs: one node for the row. */
export function CellLine({ cells, plain }: { cells: Cell[]; plain?: boolean }) {
  return <CellBlock lines={[cells]} plain={plain} />;
}

/** Many rows of cells as ONE Ink <Text>, a line per row: one React element, one Yoga node, one squash for the lot. Every
 * row is as wide as the pane (callers pad), so nothing wraps or truncates. */
export function CellBlock({
  lines,
  plain,
}: {
  lines: Cell[][];
  plain?: boolean;
}) {
  const bg = useBg();
  // The same rows as last time (useRowCache hands back the same arrays) are the same element: nothing below it renders,
  // and its Text keeps its transformer, so Yoga does not measure it again.
  const last = useRef<{ lines: Cell[][]; plain?: boolean; bg?: string; el: React.ReactElement | null } | null>(null);
  const m = last.current;
  if (m && m.plain === plain && m.bg === bg && m.lines.length === lines.length && m.lines.every((l, i) => l === lines[i])) return m.el;
  const el = blockElement(lines, plain, bg);
  last.current = { lines, plain, bg, el };
  return el;
}

function blockElement(lines: Cell[][], plain: boolean | undefined, bg: string | undefined): React.ReactElement | null {
  if (lines.length === 0) return null;
  const rows = new Rows(plain, bg);
  for (const l of lines) {
    rows.line();
    let run = "";
    let runW = 0;
    let cur: Style | undefined;
    for (const c of l) {
      if (c.s !== cur && (cur === undefined || !sameStyle(cur, c.s))) {
        if (cur) rows.put(run, cur, runW);
        run = "";
        runW = 0;
        cur = c.s;
      }
      run += c.ch;
      runW += c.w;
    }
    if (cur) rows.put(run, cur, runW);
  }
  return rows.element("truncate");
}

/** Paragraph::wrap(trim: false): break between words so each line fits `width`; a word longer than the line is cut at
 * the edge; the space where a line breaks is dropped. A line that is already short is kept as it is. */
export function wrapCells(cs: Cell[], width: number): Cell[][] {
  if (width <= 0) return [cs];
  if (widthOfCells(cs) <= width) return [cs];
  // Widths are kept as running sums and cells pushed in place: a long paragraph (an agent's answer) wraps in one pass.
  const lines: Cell[][] = [];
  let line: Cell[] = [];
  let lineW = 0;
  let word: Cell[] = [];
  let wordW = 0;
  let gap: Cell[] = [];
  let gapW = 0;
  const flushWord = () => {
    if (word.length === 0) return;
    if (lineW + gapW + wordW <= width) {
      for (const c of gap) line.push(c);
      for (const c of word) line.push(c);
      lineW += gapW + wordW;
    } else {
      if (line.length) lines.push(line);
      let rest = word;
      let restW = wordW;
      while (restW > width) {
        const head = clipCells(rest, width);
        if (head.length === 0) {
          // A wide grapheme cannot fit a one-cell column. Skip it whole, as Paragraph does.
          restW -= rest[0]!.w;
          rest = rest.slice(1);
          continue;
        }
        lines.push(head);
        rest = rest.slice(head.length);
        restW -= widthOfCells(head);
      }
      line = rest;
      lineW = restW;
    }
    word = [];
    wordW = 0;
    gap = [];
    gapW = 0;
  };
  for (const c of cs) {
    if (c.ch === " ") {
      flushWord();
      gap.push(c);
      gapW += c.w;
    } else {
      word.push(c);
      wordW += c.w;
    }
  }
  flushWord();
  if (line.length || lines.length === 0) lines.push(line);
  return lines;
}

/** Ratatui's Paragraph WordWrapper with trim:false, retaining leading/trailing padding.
 * Unlike document prose wrapping, a padded button may split its first word after the leading cell. */
export function paragraphCells<T extends Cell>(cells: T[], width: number): T[][] {
  if (width <= 0) return [];
  const lines: T[][] = [];
  let line: T[] = [], word: T[] = [], whitespace: T[] = [];
  let lineW = 0, wordW = 0, spaceW = 0;
  let nonWhitespacePrevious = false;
  for (const cell of cells) {
    const white = cell.ch === "\u200b" || (/^\s+$/u.test(cell.ch) && cell.ch !== "\u00a0");
    const cw = cell.w;
    if (cw > width) continue;
    const wordFound = nonWhitespacePrevious && white;
    const overflow = line.length === 0 && wordW + spaceW + cw > width;
    if (wordFound || overflow) {
      line.push(...whitespace, ...word);
      lineW += spaceW + wordW;
      word = []; whitespace = []; spaceW = 0; wordW = 0;
    }
    if (lineW >= width || (cw > 0 && lineW + spaceW + wordW >= width)) {
      let remaining = Math.max(0, width - lineW);
      lines.push(line); line = []; lineW = 0;
      while (whitespace.length && whitespace[0]!.w <= remaining) {
        const removed = whitespace.shift()!;
        remaining -= removed.w; spaceW -= removed.w;
      }
      if (white && whitespace.length === 0) continue;
    }
    if (white) { whitespace.push(cell); spaceW += cw; }
    else { word.push(cell); wordW += cw; }
    nonWhitespacePrevious = !white;
  }
  line.push(...whitespace, ...word);
  if (line.length) lines.push(line);
  return lines.length ? lines : [[]];
}
