// Ink's frame as a Grid and back. Ink hands the renderer one string per frame (rows joined by "\n", styled with SGR);
// while an effect runs the string is read into cells, the engine changes their colours and the cells are written
// out again. Nothing here runs when no effect does.
import { charWidth, graphemeCells } from "../chrome/width.js";

const ZWJ = 0x200d;
import { RESET } from "./color.js";
import { Grid, type Cell, type Rect } from "./grid.js";

type Pen = {
  fg: number;
  bg: number;
  flags: Set<number>;
  fgx: string;
  bgx: string;
};
const newPen = (): Pen => ({
  fg: RESET,
  bg: RESET,
  flags: new Set(),
  fgx: "",
  bgx: "",
});
const attrOf = (p: Pen): string =>
  [
    ...[...p.flags].sort((a, b) => a - b),
    ...(p.fgx ? [p.fgx] : []),
    ...(p.bgx ? [p.bgx] : []),
  ].join(";");

const parsed = new Map<string, number[]>();
function applySgr(params: string, p: Pen) {
  if (params === "") params = "0";
  let v = parsed.get(params);
  if (v === undefined) {
    v = params.split(";").map((s) => (s === "" ? 0 : Number(s)));
    if (parsed.size > 4096) parsed.clear();
    parsed.set(params, v);
  }
  for (let i = 0; i < v.length; i++) {
    const n = v[i]!;
    if (n === 0) Object.assign(p, newPen(), { flags: new Set<number>() });
    else if (n === 1 || n === 2 || n === 3 || n === 4 || n === 7 || n === 9)
      p.flags.add(n);
    else if (n === 22) {
      p.flags.delete(1);
      p.flags.delete(2);
    } else if (n === 23) p.flags.delete(3);
    else if (n === 24) p.flags.delete(4);
    else if (n === 27) p.flags.delete(7);
    else if (n === 29) p.flags.delete(9);
    else if ((n >= 30 && n <= 37) || (n >= 90 && n <= 97)) {
      p.fg = RESET;
      p.fgx = String(n);
    } else if ((n >= 40 && n <= 47) || (n >= 100 && n <= 107)) {
      p.bg = RESET;
      p.bgx = String(n);
    } else if (n === 39) {
      p.fg = RESET;
      p.fgx = "";
    } else if (n === 49) {
      p.bg = RESET;
      p.bgx = "";
    } else if (n === 38 || n === 48) {
      const mode = v[i + 1];
      if (mode === 2) {
        const c =
          ((v[i + 2] ?? 0) << 16) | ((v[i + 3] ?? 0) << 8) | (v[i + 4] ?? 0);
        if (n === 38) {
          p.fg = c;
          p.fgx = "";
        } else {
          p.bg = c;
          p.bgx = "";
        }
        i += 4;
      } else if (mode === 5) {
        const x = `${n};5;${v[i + 2] ?? 0}`;
        if (n === 38) {
          p.fg = RESET;
          p.fgx = x;
        } else {
          p.bg = RESET;
          p.bgx = x;
        }
        i += 2;
      }
    }
  }
}

/** Read Ink's frame into cells, `width` wide and as many rows as the frame has (at least `height`). */
export function parseFrame(
  output: string,
  width: number,
  height: number,
  clipToViewport = false,
): Grid {
  const lines = output.split("\n");
  if (clipToViewport && lines.length > height) lines.length = height;
  const grid = new Grid(width, clipToViewport ? height : Math.max(height, lines.length));
  for (let y = 0; y < lines.length; y++) {
    const line = lines[y]!;
    const pen = newPen();
    // the pen's other parameters as a cell keeps them: worked out when an SGR changes the pen, not for every cell
    let attr = "";
    let x = 0;
    let prev: Cell | undefined;
    for (let i = 0; i < line.length;) {
      const code = line.charCodeAt(i);
      if (code === 0x1b) {
        if (line.charCodeAt(i + 1) === 0x5b) {
          let j = i + 2;
          while (
            j < line.length &&
            !(line.charCodeAt(j) >= 0x40 && line.charCodeAt(j) <= 0x7e)
          )
            j++;
          if (line.charCodeAt(j) === 0x6d) {
            applySgr(line.slice(i + 2, j), pen);
            attr = attrOf(pen);
          }
          i = j + 1;
        } else i += 2;
        continue;
      }
      const cp = line.codePointAt(i)!;
      const len = cp > 0xffff ? 2 : 1;
      const ch = line.slice(i, i + len);
      i += len;
      const w = charWidth(cp);
      if (w === 0 || (prev && prev.ch.codePointAt(prev.ch.length - 1) === ZWJ)) {
        // joined to the cell before, which may now take two cells (`⚠` + U+FE0F, a keycap, a ZWJ sequence), as the
        // frame was laid out (app/fast-output.ts) and as the terminal draws it
        if (prev) {
          prev.ch += ch;
          if (graphemeCells(prev.ch) === 2 && x < width && grid.at(x - 1, y) === prev) {
            const tail = grid.at(x, y);
            tail.ch = "";
            tail.fg = prev.fg;
            tail.bg = prev.bg;
            tail.attr = prev.attr;
            x += 1;
          }
        }
        continue;
      }
      if (x + w > width) break;
      const cell = grid.at(x, y);
      cell.ch = ch;
      cell.fg = pen.fg;
      cell.bg = pen.bg;
      cell.attr = attr;
      if (w === 2) {
        const tail = grid.at(x + 1, y);
        tail.ch = "";
        tail.fg = pen.fg;
        tail.bg = pen.bg;
        tail.attr = attr;
      }
      prev = cell;
      x += w;
    }
  }
  return grid;
}

const toRgb = (c: number) => `${(c >> 16) & 255};${(c >> 8) & 255};${c & 255}`;
const sgr = (c: Cell): string =>
  `\x1b[0${c.attr ? ";" + c.attr : ""}${c.fg >= 0 ? ";38;2;" + toRgb(c.fg) : ""}${c.bg >= 0 ? ";48;2;" + toRgb(c.bg) : ""}m`;
const plain = (c: Cell) =>
  c.ch === " " && c.fg < 0 && c.bg < 0 && c.attr === "";

/** Write cells back as Ink's frame string. Rows no area touched are Ink's own lines, byte for byte (so Ink's
 * incremental output rewrites only what an effect changed); the others are written as runs of equal style, ending
 * where the plain tail starts. */
export function writeFrame(
  grid: Grid,
  original: string,
  areas: Rect[],
): string {
  const lines = original.split("\n");
  const touched = new Set<number>();
  for (const a of areas) for (let y = a.y; y < a.y + a.h; y++) touched.add(y);
  const rows = Math.max(lines.length, grid.height);
  const out: string[] = [];
  for (let y = 0; y < rows; y++) {
    if (!touched.has(y) && y < lines.length) {
      out.push(lines[y]!);
      continue;
    }
    let end = grid.width;
    while (end > 0 && plain(grid.at(end - 1, y))) end--;
    let line = "";
    let cur: Cell | undefined;
    for (let x = 0; x < end; x++) {
      const c = grid.at(x, y);
      if (c.ch === "") continue;
      if (!cur || c.fg !== cur.fg || c.bg !== cur.bg || c.attr !== cur.attr) {
        line += sgr(c);
        cur = c;
      }
      line += c.ch;
    }
    if (cur && !plain(cur)) line += "\x1b[0m";
    out.push(line);
  }
  return out.join("\n");
}
