// A faster Output for Ink (T7, speed).
//
// Ink composes every frame in Output.get(): for each write it parses the text into one object per character
// (@alcalzone/ansi-tokenize), rebuilds strings from them to clip, applies the Text transformers (chalk), parses again,
// writes the characters into a grid and finally turns each row back into a string character by character, diffing the
// ANSI codes of neighbours. At 160x40 with colour that was 50-100 ms of CPU a frame, about 85% of the renderer's work,
// while Ratatui draws the same screen in about 1 ms.
//
// This replaces `get()` with the same job done on a grid of (character, style id): the text is scanned once for SGR
// codes, a Text transformer is not run per line but probed once (it wraps its input in SGR codes: `open + text + close`),
// and a row is written as its runs of equal style. What it draws is what Ink's get() draws for every transformer chalk
// makes; if a transformer is anything else (not `SGR + text + SGR`) the frame goes to Ink's own get(), unchanged.
//
// `ink-output` is Ink's own output.js (an alias set in package.json `bundle` and vitest.config.ts: Ink's `exports`
// does not offer it), so the class patched is the one Ink instantiates.
import Output from "ink-output";
import chalk from "chalk";
import { charWidth, graphemeCells } from "../chrome/width.js";

const ZWJ = 0x200d;
/** The cells a cell takes once `ch` (zero width: a variation selector, keycap, mark or joiner) joins it: `⚠` is one cell
 * and `⚠` + U+FE0F two, as the layout (chrome/width.ts) and the terminal count it. Counting it as one left a row a cell
 * wider on screen than in the grid; a full-width row then wrapped, the frame scrolled a line and every row the
 * incremental write did not touch stayed a line too high. */
const joinedCells = (cell: string, w: number) => Math.max(w, graphemeCells(cell));

type RadarRows = {
  cols: number[];
  infos: Array<{ fg: string; bg: string; attr: number; link?: string; sid?: number }>;
  g: Array<[number, string, number]>;
};
type Clip = { x1?: number; x2?: number; y1?: number; y2?: number };
type Op =
  | {
      type: "write";
      x: number;
      y: number;
      text: string;
      transformers: Array<(s: string, i: number) => string>;
    }
  | { type: "clip"; clip: Clip }
  | { type: "unclip" };

const BOLD = 1,
  DIM = 2,
  ITALIC = 4,
  UNDERLINE = 8,
  INVERSE = 16,
  STRIKE = 32;
const ATTR_CODES: Array<[number, number]> = [
  [BOLD, 1],
  [DIM, 2],
  [ITALIC, 3],
  [UNDERLINE, 4],
  [INVERSE, 7],
  [STRIKE, 9],
];

/** A cell's style: colours, attributes, and the address it opens (an OSC 8 hyperlink; "" for none). A link travels
 * with the style, as Ratatui's ui::link carries it in the cell, so clipping, the hover, the selection and the scrim keep
 * it, and a row closes it where its cells end. */
type Sgr = { fg: string; bg: string; attr: number; link?: string };

/** `ESC ] 8 ; params ; url` (the text between `ESC ]` and its terminator) as the address it opens; undefined for any
 * other OSC. */
function oscLink(body: string): string | undefined {
  if (!body.startsWith("8;")) return undefined;
  const at = body.indexOf(";", 2);
  return at < 0 ? undefined : body.slice(at + 1);
}

/** Apply the parameters of one `ESC [ ... m` to `s`. */
const splitCache = new Map<string, string[]>();
function applySgr(params: string, s: Sgr) {
  if (params === "") {
    s.fg = s.bg = "";
    s.attr = 0;
    return;
  }
  let p = splitCache.get(params);
  if (p === undefined) {
    p = params.split(";");
    if (splitCache.size > 4096) splitCache.clear();
    splitCache.set(params, p);
  }
  for (let i = 0; i < p.length; i++) {
    const n = p[i] === "" ? 0 : Number(p[i]);
    if (n === 0) {
      s.fg = s.bg = "";
      s.attr = 0;
    } else if (n === 1) s.attr |= BOLD;
    else if (n === 2) s.attr |= DIM;
    else if (n === 3) s.attr |= ITALIC;
    else if (n === 4) s.attr |= UNDERLINE;
    else if (n === 7) s.attr |= INVERSE;
    else if (n === 9) s.attr |= STRIKE;
    else if (n === 22) s.attr &= ~(BOLD | DIM);
    else if (n === 23) s.attr &= ~ITALIC;
    else if (n === 24) s.attr &= ~UNDERLINE;
    else if (n === 27) s.attr &= ~INVERSE;
    else if (n === 29) s.attr &= ~STRIKE;
    else if ((n >= 30 && n <= 37) || (n >= 90 && n <= 97)) s.fg = String(n);
    else if (n === 39) s.fg = "";
    else if ((n >= 40 && n <= 47) || (n >= 100 && n <= 107)) s.bg = String(n);
    else if (n === 49) s.bg = "";
    else if (n === 38 || n === 48) {
      const mode = p[i + 1];
      let v: string;
      if (mode === "5") {
        v = `${n};5;${p[i + 2]}`;
        i += 2;
      } else if (mode === "2") {
        v = `${n};2;${p[i + 2]};${p[i + 3]};${p[i + 4]}`;
        i += 4;
      } else continue;
      if (n === 38) s.fg = v;
      else s.bg = v;
    }
  }
}

// Interned styles: id 0 is the default.
const styleKey = new Map<string, number>([["||0", 0]]);
const styles: Sgr[] = [{ fg: "", bg: "", attr: 0 }];
const transitions = new Map<number, string>();
const styleTree = new Map<string, Map<string, Map<number, number>>>();
function styleId(fg: string, bg: string, attr: number, link = ""): number {
  if (link !== "") {
    // Linked cells are few: looked up by their whole key.
    const key = `${fg}|${bg}|${attr}|${link}`;
    let id = styleKey.get(key);
    if (id === undefined) {
      id = styles.length;
      styles.push({ fg, bg, attr, link });
      styleKey.set(key, id);
    }
    return id;
  }
  let byBg = styleTree.get(fg);
  if (byBg === undefined) styleTree.set(fg, (byBg = new Map()));
  let byAttr = byBg.get(bg);
  if (byAttr === undefined) byBg.set(bg, (byAttr = new Map()));
  let id = byAttr.get(attr);
  if (id !== undefined) return id;
  const key = `${fg}|${bg}|${attr}`;
  id = styleKey.get(key);
  if (id === undefined) {
    id = styles.length;
    styles.push({ fg, bg, attr });
    styleKey.set(key, id);
  }
  byAttr.set(attr, id);
  return id;
}
const ATTR_OFF: Array<[number, number]> = [
  [BOLD, 22],
  [DIM, 22],
  [ITALIC, 23],
  [UNDERLINE, 24],
  [INVERSE, 27],
  [STRIKE, 29],
];
/** The SGR that turns style `a` into style `b`: one escape per change and no full reset, as Ink writes them (colours
 * are set and cleared with 3x/4x and 39/49), so a reader of the stream can tell the colour in force at any cell. */
function transition(a: number, b: number): string {
  const k = a * 65536 + b;
  let t = transitions.get(k);
  if (t !== undefined) return t;
  const A = styles[a]!;
  const B = styles[b]!;
  let out = "";
  const code = (c: string | number) => (out += `\x1b[${c}m`);
  // Attributes that go off (22 clears bold and dim together, so the other is set again below).
  let have = A.attr;
  for (const [bit, off] of ATTR_OFF) {
    if (have & bit && !(B.attr & bit)) {
      code(off);
      if (off === 22) have &= ~(BOLD | DIM);
      else have &= ~bit;
    }
  }
  for (const [bit, on] of ATTR_CODES)
    if (B.attr & bit && !(have & bit)) code(on);
  // Background first, then foreground: the order Ink writes them in.
  if (B.bg !== A.bg) code(B.bg === "" ? "49" : B.bg);
  if (B.fg !== A.fg) code(B.fg === "" ? "39" : B.fg);
  // The link last: opened where it starts, closed where it ends (a row's end goes back to style 0).
  if ((B.link ?? "") !== (A.link ?? "")) out += `\x1b]8;;${B.link ?? ""}\x1b\\`;
  transitions.set(k, out);
  return out;
}

const SGR_ONLY = /^(?:\x1b\[[0-9;]*m)*$/;
const MARK = "\u0001";
/** What a transformer puts around its text, as a style; undefined when it does anything else. */
function probe(fn: (s: string, i: number) => string): Sgr | undefined {
  let r: string;
  try {
    r = fn(MARK, 0);
  } catch {
    return undefined;
  }
  const at = r.indexOf(MARK);
  if (at < 0 || r.indexOf(MARK, at + 1) >= 0) return undefined;
  const open = r.slice(0, at);
  const close = r.slice(at + 1);
  if (!SGR_ONLY.test(open) || !SGR_ONLY.test(close)) return undefined;
  // It must leave the text itself alone (a transformer that changes letters is not plain SGR).
  try {
    if (fn("aB 9é", 0) !== `${open}aB 9é${close}`) return undefined;
  } catch {
    return undefined;
  }
  const s: Sgr = { fg: "", bg: "", attr: 0 };
  for (const m of open.matchAll(/\x1b\[([0-9;]*)m/g)) applySgr(m[1]!, s);
  return s;
}

/** An op's text as cells: for each line, each cell's character (with any combining marks after it), its width and its
 * style id, from the op's own column. */
type Decoded = Array<{ ch: string[]; w: number[]; sid: number[] }>;

function decode(text: string, base: Sgr, rowsOf: RadarRows[] | undefined): Decoded {
  const out: Decoded = [];
  let line = { ch: [] as string[], w: [] as number[], sid: [] as number[] };
  out.push(line);
  const st: Sgr = { fg: "", bg: "", attr: 0 };
  const baseId = styleId(base.fg, base.bg, base.attr);
  let id = baseId;
  let y = 0; // the line, from the op's first
  let col = 0; // the column, from the op's x
  let prev = false; // a cell was put on this line since the last newline, for combining marks

  let ri = 0; // the style run in force on this line (rowsOf)
  const put = (ch: string, w: number) => {
    if (rowsOf) {
      const r = rowsOf[y];
      if (r) {
        if (col === 0 || ri >= r.cols.length || r.cols[ri]! > col) ri = 0;
        while (ri + 1 < r.cols.length && r.cols[ri + 1]! <= col) ri++;
        const inf = r.infos[ri];
        // An Info is shared by every Text that uses its style (text/cells.tsx infoOf), so the id it keeps is the one
        // on no base; under an outer style it is looked up with that base each time.
        if (inf && r.cols[ri]! <= col)
          id =
            baseId === 0
              ? (inf.sid ??= styleId(inf.fg, inf.bg, inf.attr, inf.link))
              : styleId(inf.fg || base.fg, inf.bg || base.bg, inf.attr | base.attr, inf.link);
        else id = baseId;
      }
    }
    line.ch.push(ch);
    line.w.push(w);
    line.sid.push(id);
    col += w;
    prev = true;
  };

  let i = 0;
  let gi = 0; // the next glyph of this line (rowsOf[..].g)
  const n = text.length;
  while (i < n) {
    const code = text.charCodeAt(i);
    if (code === 0x3f && rowsOf) {
      // `?` standing for a character that is not printable ASCII (text/cells.tsx Rows): put it back
      const g = rowsOf[y]?.g;
      if (g) {
        while (gi < g.length && g[gi]![0] < col) gi++;
        if (gi < g.length && g[gi]![0] === col) {
          const [, glyph, gw] = g[gi++]!;
          i++;
          for (let k = gw - 1; k > 0 && text.charCodeAt(i) === 0x3f; k--) i++;
          put(glyph, gw);
          continue;
        }
      }
    }
    if (code === 0x1b) {
      const next = text.charCodeAt(i + 1);
      if (next === 0x5b) {
        // CSI: parameters, then a final byte.
        let j = i + 2;
        while (j < n) {
          const c = text.charCodeAt(j);
          if (c >= 0x40 && c <= 0x7e) break;
          j++;
        }
        if (text.charCodeAt(j) === 0x6d) {
          applySgr(text.slice(i + 2, j), st);
          id = styleId(st.fg || base.fg, st.bg || base.bg, st.attr | base.attr, st.link);
        }
        i = j + 1;
        continue;
      }
      if (next === 0x5d) {
        // OSC: up to BEL or ESC \. A hyperlink (OSC 8) goes into the style of the cells after it; any other is dropped.
        let j = i + 2;
        while (j < n && text.charCodeAt(j) !== 0x07 && !(text.charCodeAt(j) === 0x1b && text.charCodeAt(j + 1) === 0x5c)) j++;
        const link = oscLink(text.slice(i + 2, j));
        if (link !== undefined) {
          st.link = link;
          id = styleId(st.fg || base.fg, st.bg || base.bg, st.attr | base.attr, st.link);
        }
        i = j + (text.charCodeAt(j) === 0x07 ? 1 : 2);
        continue;
      }
      i += 2;
      continue;
    }
    if (code === 0x0a) {
      y++;
      gi = 0;
      col = 0;
      prev = false;
      line = { ch: [], w: [], sid: [] };
      out.push(line);
      i++;
      continue;
    }
    const cp = text.codePointAt(i)!;
    const len = cp > 0xffff ? 2 : 1;
    i += len;
    if (cp < 0x20 || cp === 0x7f) continue;
    const w = charWidth(cp);
    const last = line.ch.length - 1;
    if (w === 0 || (prev && line.ch[last]!.codePointAt(line.ch[last]!.length - 1) === ZWJ)) {
      // A combining mark, variation selector or joiner, or what a joiner joins: part of the cell before it (and dropped
      // with it where a clip cuts that cell), which may now take more cells.
      if (prev) {
        line.ch[last] += String.fromCodePoint(cp);
        const cells = joinedCells(line.ch[last]!, line.w[last]!);
        col += cells - line.w[last]!;
        line.w[last] = cells;
      }
      continue;
    }
    put(len === 2 ? String.fromCodePoint(cp) : text[i - 1]!, w);
  }
  return out;
}

/** Ops' cells by their text and base style: most of a frame is written in the same words as the frame before (pane
 * backgrounds, borders, rows that did not change), and reading it into cells was most of what composing cost. A Text
 * with style runs (text/cells.tsx Rows) is kept by its runs, which a new Rows replaces. */
const decodedPlain = new Map<string, Decoded>();
const decodedRuns = new WeakMap<RadarRows[], Map<string, Decoded>>();
/** Cells held in decodedPlain: it starts again past about a dozen 280x75 screens' worth. */
let plainCells = 0;
const PLAIN_CELLS = 250_000;
function decoded(text: string, base: Sgr, rowsOf: RadarRows[] | undefined): Decoded {
  let m = decodedPlain;
  if (rowsOf) {
    m = decodedRuns.get(rowsOf)!;
    if (!m) decodedRuns.set(rowsOf, (m = new Map()));
  }
  const key = `${base.fg}|${base.bg}|${base.attr}|${text}`;
  let d = m.get(key);
  if (d === undefined) {
    d = decode(text, base, rowsOf);
    if (m === decodedPlain) {
      const cells = d.reduce((n, l) => n + l.ch.length, 0);
      if (plainCells + cells > PLAIN_CELLS) {
        decodedPlain.clear();
        plainCells = 0;
      }
      plainCells += cells;
    } else if (m.size > 64) m.clear();
    m.set(key, d);
  }
  return d;
}

/** Frames handed back to Ink's own get() because a transformer was not plain SGR around its text. */
export let fallbackFrames = 0;
const originalGet: () => { output: string; height: number } = (
  Output.prototype as any
).get;

function fastGet(this: any): { output: string; height: number } {
  const width: number = this.width;
  const height: number = this.height;
  const ops: Op[] = this.operations;

  // Each op's transformer styles first: any we cannot read sends the frame to Ink's get().
  const bases = new Map<Op, { fg: string; bg: string; attr: number }>();
  const radar = new Map<Op, RadarRows[]>();
  for (const op of ops) {
    if (op.type !== "write") continue;
    let fg = "",
      bg = "",
      attr = 0;
    for (const t of op.transformers) {
      // The mark of a layer (app/scrim.ts layerMark): the text as it is.
      if ((t as any).radarLayer) continue;
      // A Text of plain characters whose style runs ride on its transformer (text/cells.tsx Rows): read at write time.
      if ((t as any).radarRows) {
        radar.set(op, (t as any).radarRows);
        continue;
      }
      const s = probe(t);
      if (!s) {
        fallbackFrames++;
        return originalGet.call(this);
      }
      // Innermost first: an outer layer shows only where the inner ones have no colour.
      if (fg === "") fg = s.fg;
      if (bg === "") bg = s.bg;
      attr |= s.attr;
    }
    bases.set(op, { fg, bg, attr });
  }

  const chars: string[][] = [];
  const sty: Int32Array[] = [];
  for (let y = 0; y < height; y++) {
    chars.push(new Array<string>(width).fill(" "));
    sty.push(new Int32Array(width));
  }

  const clips: Clip[] = [];
  for (const op of ops) {
    if (op.type === "clip") {
      const o = clips[clips.length - 1];
      const c = op.clip;
      clips.push(
        o
          ? {
              x1:
                c.x1 === undefined
                  ? o.x1
                  : o.x1 === undefined
                    ? c.x1
                    : Math.max(o.x1, c.x1),
              x2:
                c.x2 === undefined
                  ? o.x2
                  : o.x2 === undefined
                    ? c.x2
                    : Math.min(o.x2, c.x2),
              y1:
                c.y1 === undefined
                  ? o.y1
                  : o.y1 === undefined
                    ? c.y1
                    : Math.max(o.y1, c.y1),
              y2:
                c.y2 === undefined
                  ? o.y2
                  : o.y2 === undefined
                    ? c.y2
                    : Math.min(o.y2, c.y2),
            }
          : c,
      );
      continue;
    }
    if (op.type === "unclip") {
      clips.pop();
      continue;
    }
    const clip = clips[clips.length - 1];
    const x1 = Math.max(0, clip?.x1 ?? 0);
    const x2 = Math.min(width, clip?.x2 ?? width);
    const y1 = Math.max(0, clip?.y1 ?? 0);
    const y2 = Math.min(height, clip?.y2 ?? height);
    if (x1 >= x2 || y1 >= y2) continue;
    const lines = decoded(op.text, bases.get(op)!, radar.get(op));
    // The op's cells from (op.x, op.y), inside the clip.
    for (let ly = Math.max(0, y1 - op.y); ly < lines.length; ly++) {
      const y = op.y + ly;
      if (y >= y2) break;
      const { ch, w: ws, sid } = lines[ly]!;
      const row = chars[y]!;
      const srow = sty[y]!;
      let c0: number = op.x;
      for (let k = 0; k < ch.length && c0 < x2; k++) {
        const w = ws[k]!;
        if (c0 + w > x1) {
          const full = c0 >= x1 && c0 + w <= x2; // a wide character cut by a clip edge shows as blanks
          for (let j = 0; j < w; j++) {
            const c = c0 + j;
            if (c < x1 || c >= x2) continue;
            // Overwriting one half of an older wide character blanks the other half.
            if (row[c] === "" && c > 0) row[c - 1] = " ";
            if (j === w - 1 && row[c + 1] === "" && c + 1 < width) row[c + 1] = " ";
            row[c] = full ? (j === 0 ? ch[k]! : "") : " ";
            srow[c] = sid[k]!;
          }
        }
        c0 += w;
      }
    }
  }

  const lines: string[] = [];
  for (let y = 0; y < height; y++) lines.push(rowString(chars[y]!, sty[y]!, width));
  return { output: lines.join("\n"), height };
}

/** One row of cells as Ink writes it: runs of equal style, each opened by the SGR that changes the one before, the plain
 * tail left off. */
function rowString(row: string[], srow: ArrayLike<number>, width: number): string {
  let end = width;
  while (end > 0 && row[end - 1] === " " && srow[end - 1] === 0) end--;
  let line = "";
  let cur = 0;
  // A run of cells of one style is joined at once (a cell of a wide character's second half is "" and adds nothing).
  for (let x = 0; x < end;) {
    const s = srow[x]!;
    const start = x;
    while (x < end && srow[x] === s) x++;
    const seg = x - start === 1 ? row[start]! : row.slice(start, x).join("");
    if (seg === "") continue;
    if (s !== cur) {
      line += transition(cur, s);
      cur = s;
    }
    line += seg;
  }
  if (cur !== 0) line += transition(cur, 0);
  return line;
}

/** A row of a finished frame read back into cells (character, style id), `width` wide: the inverse of rowString. */
function parseRow(line: string, width: number): { chars: string[]; sty: Int32Array } {
  const chars = new Array<string>(width).fill(" ");
  const sty = new Int32Array(width);
  const st: Sgr = { fg: "", bg: "", attr: 0 };
  let id = 0;
  let x = 0;
  let prev = -1;
  const n = line.length;
  for (let i = 0; i < n && x < width;) {
    const code = line.charCodeAt(i);
    if (code === 0x1b) {
      const next = line.charCodeAt(i + 1);
      if (next === 0x5b) {
        let j = i + 2;
        while (j < n) {
          const c = line.charCodeAt(j);
          if (c >= 0x40 && c <= 0x7e) break;
          j++;
        }
        if (line.charCodeAt(j) === 0x6d) {
          applySgr(line.slice(i + 2, j), st);
          id = styleId(st.fg, st.bg, st.attr, st.link);
        }
        i = j + 1;
        continue;
      }
      if (next === 0x5d) {
        let j = i + 2;
        while (j < n && line.charCodeAt(j) !== 0x07 && !(line.charCodeAt(j) === 0x1b && line.charCodeAt(j + 1) === 0x5c)) j++;
        const link = oscLink(line.slice(i + 2, j));
        if (link !== undefined) {
          st.link = link;
          id = styleId(st.fg, st.bg, st.attr, st.link);
        }
        i = j + (line.charCodeAt(j) === 0x07 ? 1 : 2);
        continue;
      }
      i += 2;
      continue;
    }
    const cp = line.codePointAt(i)!;
    const len = cp > 0xffff ? 2 : 1;
    const ch = len === 2 ? String.fromCodePoint(cp) : line[i]!;
    i += len;
    if (cp < 0x20 || cp === 0x7f) continue;
    const w = charWidth(cp);
    if (w === 0 || (prev >= 0 && chars[prev]!.codePointAt(chars[prev]!.length - 1) === ZWJ)) {
      if (prev >= 0) {
        chars[prev] += ch;
        const cells = Math.min(joinedCells(chars[prev]!, x - prev), width - prev);
        for (let k = x; k < prev + cells; k++) {
          chars[k] = "";
          sty[k] = sty[prev]!;
        }
        x = Math.max(x, prev + cells);
      }
      continue;
    }
    if (x + w > width) break;
    chars[x] = ch;
    sty[x] = id;
    for (let k = 1; k < w; k++) {
      chars[x + k] = "";
      sty[x + k] = id;
    }
    prev = x;
    x += w;
  }
  return { chars, sty };
}

/** The style of an interned id. */
export const styleOf = (id: number): Readonly<Sgr> => styles[id]!;
export { styleId, BOLD, DIM, UNDERLINE };
export type { Sgr };

/** The SGR parameters chalk (so Ink's <Text backgroundColor>) writes for a background colour, as the cells of a frame
 * carry it ("48;2;r;g;b" in truecolor); "" for no colour. */
const bgCache = new Map<string, string>();
export function bgParams(color: string | undefined): string {
  if (!color) return "";
  let p = bgCache.get(color);
  if (p === undefined) {
    p = probe((s) => colorize(s, color))?.bg ?? "";
    bgCache.set(color, p);
  }
  return p;
}
const colorize = (s: string, color: string) =>
  color.startsWith("#") ? chalk.bgHex(color)(s) : (((chalk as any)[`bg${color[0]!.toUpperCase()}${color.slice(1)}`] as ((t: string) => string) | undefined)?.(s) ?? s);

/** Restyle the cells of `rect` in a finished frame: `map` gives each cell's new style id from its old one. Only the rows
 * the rectangle covers are read and written again; the others are the frame's own strings. */
export function restyle(
  output: string,
  width: number,
  rect: { x: number; y: number; w: number; h: number },
  map: (id: number) => number,
): string {
  const rows: Array<{ y: number; from: number; to: number }> = [];
  for (let y = rect.y; y < rect.y + rect.h; y++) rows.push({ y, from: rect.x, to: rect.x + rect.w - 1 });
  return restyleRows(output, width, rows, map);
}

/** The cells of row `y` of a finished frame: a character per cell, "" for the second cell of a wide one. */
export function frameRow(output: string, width: number, y: number): string[] {
  let at = 0;
  for (let i = 0; i < y; i++) {
    at = output.indexOf("\n", at);
    if (at < 0) return new Array<string>(width).fill(" ");
    at++;
  }
  const end = output.indexOf("\n", at);
  return parseRow(output.slice(at, end < 0 ? undefined : end), width).chars;
}

/** Restyle runs of cells in a finished frame, each `from..to` inclusive on row `y` (a selection's rows): as `restyle`,
 * each row read and written again once. */
export function restyleRows(
  output: string,
  width: number,
  rows: ReadonlyArray<{ y: number; from: number; to: number }>,
  map: (id: number) => number,
): string {
  const lines = output.split("\n");
  for (const r of rows) {
    const y = r.y;
    const x0 = Math.max(0, r.from);
    const x1 = Math.min(width, r.to + 1);
    if (x1 <= x0 || y < 0 || y >= lines.length) continue;
    const { chars, sty } = parseRow(lines[y]!, width);
    // A wide character cut by the rectangle's edge keeps one style for both halves.
    let from = x0;
    while (from > 0 && chars[from] === "") from--;
    let to = x1;
    while (to < width && chars[to] === "") to++;
    for (let x = from; x < to; x++) sty[x] = map(sty[x]!);
    lines[y] = rowString(chars, sty, width);
  }
  return lines.join("\n");
}

type Frame = { output: string; height: number };
/** A layer over the screen (a menu's box) as the frame marks it, in cells. */
export type Layer = { x: number; y: number; w: number; h: number; shadowOnly?: boolean };
/** What is painted over every finished frame (the scrim and the hover, app/scrim.ts, app/hover.ts), given the layer the
 * frame's own writes mark, if any; the frame string comes back changed or as is. */
let paintOver: ((output: string, width: number, height: number, layers: Layer[]) => string) | undefined;
export function setFramePaint(fn: typeof paintOver) {
  paintOver = fn;
}

/** The layers a frame's writes mark (app/scrim.ts layerMark): its size rides on the mark, its place is where the mark was
 * written (the box's top-left cell). Read from the frame, so two renderers in one process (the tests) never take each
 * other's. */
function layersOf(ops: Op[]): Layer[] {
  const out: Layer[] = [];
  for (const op of ops) {
    if (op.type !== "write") continue;
    for (const t of op.transformers) {
      const l = (t as { radarLayer?: { w: number; h: number; shadowOnly?: boolean } }).radarLayer;
      if (l) out.push({ x: op.x, y: op.y, w: l.w, h: l.h, ...(l.shadowOnly ? { shadowOnly: true } : {}) });
    }
  }
  // A shadow alone (the tour's card) is painted before a menu's scrim, as Ratatui draws the card first and the scrim after.
  return out.sort((a, b) => Number(!!b.shadowOnly) - Number(!!a.shadowOnly));
}

/** Two frames' operations draw the same when every write has the same place, text and transformer functions (the same
 * Text nodes, not drawn again since) and the clips are the same. */
function sameOps(a: Op[], b: Op[]): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    const p = a[i]!;
    const q = b[i]!;
    if (p.type !== q.type) return false;
    if (p.type === "write") {
      const w = q as typeof p;
      if (p.x !== w.x || p.y !== w.y || p.text !== w.text || p.transformers.length !== w.transformers.length) return false;
      for (let k = 0; k < p.transformers.length; k++) if (p.transformers[k] !== w.transformers[k]) return false;
    } else if (p.type === "clip") {
      const c = (q as typeof p).clip;
      if (p.clip.x1 !== c.x1 || p.clip.x2 !== c.x2 || p.clip.y1 !== c.y1 || p.clip.y2 !== c.y2) return false;
    }
  }
  return true;
}

/** The last frame composed, by the operations that made it: a frame Ink asks for again with nothing drawn anew (the
 * hover moved, a timer poked Ink) is the same string, so it is not composed again. */
let last: { get: unknown; width: number; height: number; ops: Op[]; frame: Frame } | undefined;
/** Frames taken from `last` instead of being composed (for tests). */
export let reusedFrames = 0;

let fast = true;
function get(this: any): Frame {
  const base = fast ? fastGet : originalGet;
  const ops: Op[] = this.operations;
  let frame: Frame;
  if (last && last.get === base && last.width === this.width && last.height === this.height && sameOps(last.ops, ops)) {
    frame = last.frame;
    reusedFrames++;
  } else {
    frame = base.call(this);
    last = { get: base, width: this.width, height: this.height, ops, frame };
  }
  return paintOver ? { output: paintOver(frame.output, this.width, this.height, layersOf(ops)), height: frame.height } : frame;
}
(Output.prototype as any).get = get;

/** Use the fast get() (the default once this module is imported); `false` puts Ink's own back. */
export function useFastOutput(on = true) {
  fast = on;
}
export const fastOutputInstalled = () => fast;
/** Ink's own Output.get, as Ink has it (the patch's tests read it). */
export const inkOutputGet = originalGet;
// RADAR_INK_SLOW_OUTPUT=1 keeps Ink's own get() (a way back if a frame ever draws wrong).
useFastOutput(!process.env.RADAR_INK_SLOW_OUTPUT);
