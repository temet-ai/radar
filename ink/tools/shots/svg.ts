// Ink's styled cells as an SVG: every cell at its colours, so a wrong colour or a lost highlight shows.
// Each cell is 10x20 units; a background is one rect per run, text one <text> per run pinned to its columns
// with textLength, a wide character takes two columns, underline and strike-through are lines. The same cells
// always give the same bytes, so two renders compare with cmp.
import type { TCell } from "../../test/nodes/ansi.js";
import { charWidth } from "../../src/chrome/width.js";

export type Rgb = [number, number, number];

const ANSI: Rgb[] = [
  [0x00, 0x00, 0x00], [0xcd, 0x31, 0x31], [0x0d, 0xbc, 0x79], [0xe5, 0xe5, 0x10],
  [0x24, 0x72, 0xc8], [0xbc, 0x3f, 0xbc], [0x11, 0xa8, 0xcd], [0xe5, 0xe5, 0xe5],
  [0x66, 0x66, 0x66], [0xf1, 0x4c, 0x4c], [0x23, 0xd1, 0x8b], [0xf5, 0xf5, 0x43],
  [0x3b, 0x8e, 0xea], [0xd6, 0x70, 0xd6], [0x29, 0xb8, 0xdb], [0xff, 0xff, 0xff],
];

export const FONTS =
  "'0xProto Nerd Font Mono', 'Symbols Nerd Font Mono', 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace, 'Noto Color Emoji'";
const CELL_W = 10;
const CELL_H = 20;
const BASELINE = 15;

const hex = ([r, g, b]: Rgb) => "#" + [r, g, b].map((n) => n.toString(16).padStart(2, "0")).join("");

function indexed(i: number): Rgb {
  if (i < 16) return ANSI[i]!;
  if (i < 232) {
    const L = [0, 95, 135, 175, 215, 255];
    const n = i - 16;
    return [L[Math.floor(n / 36)]!, L[Math.floor(n / 6) % 6]!, L[n % 6]!];
  }
  const v = 8 + 10 * (i - 232);
  return [v, v, v];
}

/** `#rrggbb`, `ansi256(n)` or `ansi16(n)` (as parseAnsi names them); the default when absent. */
export function resolve(colour: string | undefined, dflt: Rgb): Rgb {
  if (!colour) return dflt;
  const h = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(colour);
  if (h) return [parseInt(h[1]!, 16), parseInt(h[2]!, 16), parseInt(h[3]!, 16)];
  const a = /^ansi(?:256|16)\((\d+)\)$/.exec(colour);
  return a ? indexed(Number(a[1])) : dflt;
}

export function parseHex(h: string, dflt: Rgb): Rgb {
  return resolve(h, dflt);
}

const blend = (a: Rgb, b: Rgb, part: number): Rgb => a.map((x, i) => Math.floor((x * (100 - part) + b[i]! * part + 50) / 100)) as Rgb;
const same = (a: Rgb, b: Rgb) => a[0] === b[0] && a[1] === b[1] && a[2] === b[2];

interface Ink { fg: Rgb; bg: Rgb; bold: boolean; italic: boolean; underline: boolean; strike: boolean }

function inkOf(c: TCell, dfg: Rgb, dbg: Rgb): Ink {
  let fg = resolve(c.fg, dfg);
  let bg = resolve(c.bg, dbg);
  if (c.inverse) [fg, bg] = [bg, fg];
  if (c.dim) fg = blend(fg, bg, 40);
  return { fg, bg, bold: !!c.bold, italic: !!c.italic, underline: !!c.underline, strike: !!c.strike };
}

const sameText = (a: Ink, b: Ink) => same(a.fg, b.fg) && a.bold === b.bold && a.italic === b.italic;

const esc = (t: string) =>
  [...t].map((c) => (c === "&" ? "&amp;" : c === "<" ? "&lt;" : c === ">" ? "&gt;" : c.codePointAt(0)! < 0x20 ? " " : c)).join("");
const cols = (s: string) => (s === "" ? 0 : Math.max(1, [...s].reduce((n, ch) => n + charWidth(ch.codePointAt(0)!), 0)));
// An icon or emoji a font may lack stands alone, so the font picked for it does not draw the words beside it.
const lone = (s: string) => [...s].some((ch) => ch.codePointAt(0)! >= 0xe000 && ch.codePointAt(0)! <= 0xf8ff) || [...s].some((ch) => ch.codePointAt(0)! >= 0x1f000);

interface Run { col: number; text: string; width: number; ink: Ink; wide: boolean }

export function cellsToSvg(rows: TCell[][], width: number, height: number, dfg: Rgb, dbg: Rgb): string {
  let backs = "";
  let texts = "";
  let lines = "";
  for (let y = 0; y < height; y++) {
    const row = rows[y] ?? [];
    const runs: Run[] = [];
    const backgrounds: Array<[number, number, Rgb]> = [];
    const unders: Array<[number, number, Rgb, boolean]> = [];
    for (let col = 0; col < width; ) {
      const cell: TCell = row[col] ?? { ch: " " };
      const ink = inkOf(cell, dfg, dbg);
      const span = Math.min(Math.max(cols(cell.ch), 1), width - col);
      for (let k = 0; k < span; k++) {
        const bg = k === 0 ? ink.bg : inkOf(row[col + k] ?? { ch: " " }, dfg, dbg).bg;
        const last = backgrounds[backgrounds.length - 1];
        if (last && same(last[2], bg) && last[0] + last[1] === col + k) last[1]++;
        else backgrounds.push([col + k, 1, bg]);
      }
      for (const [on, strike] of [[ink.underline, false], [ink.strike, true]] as const) {
        if (!on) continue;
        const last = unders[unders.length - 1];
        if (last && last[3] === strike && same(last[2], ink.fg) && last[0] + last[1] === col) last[1] += span;
        else unders.push([col, span, ink.fg, strike]);
      }
      const last = runs[runs.length - 1];
      if (cell.ch.trim() !== "") {
        const wide = span > 1 || lone(cell.ch);
        if (last && !wide && last.width > 0 && sameText(last.ink, ink) && last.col + last.width === col && !last.wide) {
          last.text += cell.ch;
          last.width += span;
        } else runs.push({ col, text: cell.ch, width: span, ink, wide });
      } else if (cell.ch === " " && last && last.col + last.width === col && !last.wide && sameText(last.ink, ink)) {
        last.text += " ";
        last.width += 1;
      }
      col += span;
    }
    for (const [start, n, rgb] of backgrounds)
      if (!same(rgb, dbg)) backs += `<rect x="${start * CELL_W}" y="${y * CELL_H}" width="${n * CELL_W}" height="${CELL_H}" fill="${hex(rgb)}"/>\n`;
    for (const run of runs) {
      const lead = run.text.length - run.text.trimStart().length;
      const text = run.text.trim();
      if (!text) continue;
      texts += `<text x="${(run.col + lead) * CELL_W}" y="${y * CELL_H + BASELINE}" textLength="${Math.max(1, [...text].reduce((n, c) => n + charWidth(c.codePointAt(0)!), 0)) * CELL_W}" fill="${hex(run.ink.fg)}"${run.ink.bold ? ' font-weight="700"' : ""}${run.ink.italic ? ' font-style="italic"' : ""}>${esc(text)}</text>\n`;
    }
    for (const [start, n, rgb, strike] of unders)
      lines += `<rect x="${start * CELL_W}" y="${y * CELL_H + (strike ? CELL_H / 2 : CELL_H - 3)}" width="${n * CELL_W}" height="1" fill="${hex(rgb)}"/>\n`;
  }
  const pw = width * CELL_W;
  const ph = height * CELL_H;
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${pw}" height="${ph}" viewBox="0 0 ${pw} ${ph}" font-family="${FONTS}" font-size="16.6" xml:space="preserve">\n<rect width="${pw}" height="${ph}" fill="${hex(dbg)}"/>\n${backs}${texts}${lines}</svg>\n`;
}
