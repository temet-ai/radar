import { graphemes } from "../editor/graphemes.js";

/** Terminal cell width of a string: wide (CJK, emoji) count 2, combining marks 0. */
export function cellWidth(s: string): number {
  if (/^[\x20-\x7e]*$/.test(s)) return s.length;
  let w = 0;
  for (const g of graphemes(s)) w += graphemeCells(g);
  return w;
}

/** `s` as the terminal lays it out: each grapheme (`⚠` + U+FE0F, a keycap, a ZWJ family, a letter and its marks) with
 * the cells it takes. Cut, wrap and place text by these, never by code points: a code point alone counts `⚠️` one cell
 * where the terminal draws two, and a row built that way outgrows the screen. */
export function clusters(s: string): Array<[string, number]> {
  const out: Array<[string, number]> = [];
  for (const g of graphemes(s)) out.push([g, graphemeCells(g)]);
  return out;
}

/** A complete grapheme's terminal width, matching unicode-width's emoji sequences. */
export function graphemeCells(g: string): number {
  if (/^\p{Regional_Indicator}{2}$/u.test(g) || /^[#*0-9]\ufe0f?\u20e3$/u.test(g)) return 2;
  if (/\p{Emoji}\ufe0f/u.test(g) || /\p{Extended_Pictographic}\p{Emoji_Modifier}/u.test(g) || /^\p{Extended_Pictographic}(?:\ufe0f|\p{Emoji_Modifier})?(?:\u200d\p{Extended_Pictographic}(?:\ufe0f|\p{Emoji_Modifier})?)+$/u.test(g)) return 2;
  return [...g].reduce((n, ch) => n + charWidth(ch.codePointAt(0)!), 0);
}

export function charWidth(cp: number): number {
  if (cp === 0) return 0;
  if (cp < 32 || (cp >= 0x7f && cp < 0xa0)) return 0;
  if (/\p{Mark}/u.test(String.fromCodePoint(cp))) return 0;
  if (
    (cp >= 0x300 && cp <= 0x36f) ||
    (cp >= 0x200b && cp <= 0x200f) ||
    (cp >= 0xfe00 && cp <= 0xfe0f) ||
    (cp >= 0x20d0 && cp <= 0x20ff)
  )
    return 0;
  if (
    (cp >= 0x1100 && cp <= 0x115f) ||
    (cp >= 0x2329 && cp <= 0x232a) ||
    isWideSymbol(cp) ||
    (cp >= 0x2e80 && cp <= 0xa4cf) ||
    (cp >= 0xac00 && cp <= 0xd7a3) ||
    (cp >= 0xf900 && cp <= 0xfaff) ||
    (cp >= 0xfe30 && cp <= 0xfe6f) ||
    (cp >= 0xff00 && cp <= 0xff60) ||
    (cp >= 0xffe0 && cp <= 0xffe6) ||
    (cp >= 0x1f300 && cp <= 0x1faff) ||
    (cp >= 0x20000 && cp <= 0x3fffd)
  )
    return 2;
  return 1;
}

/** Cut styled segments to at most `max` cells, with no ellipsis: a cell past the edge is not drawn, as Ratatui's buffer does. */
export function clipSegments<T extends { text: string }>(segs: T[], max: number): T[] {
  const out: T[] = [];
  let w = 0;
  for (const seg of segs) {
    if (w >= max) break;
    let text = "";
    for (const ch of graphemes(seg.text)) {
      const cw = graphemeCells(ch);
      if (w + cw > max) {
        w = max;
        break;
      }
      text += ch;
      w += cw;
    }
    if (text !== "") out.push({ ...seg, text });
  }
  return out;
}

/** Cut `s` to at most `max` cells, ending with `…` when it was cut. */
export function fit(s: string, max: number): string {
  if (max <= 0) return "";
  if (cellWidth(s) <= max) return s;
  let out = "";
  let w = 0;
  for (const ch of graphemes(s)) {
    const cw = graphemeCells(ch);
    if (w + cw > max - 1) break;
    out += ch;
    w += cw;
  }
  return out + "…";
}

/** The symbols below U+2E80 that Unicode draws two cells wide (East Asian Width W: the ones with emoji presentation, ✅ ❌
 * ⚪ ⭐), and the few wide ones of the first emoji planes outside U+1F300..1FAFF. Rust's unicode-width, which Ratatui lays
 * out by, counts them 2; one cell here would leave a row two cells too long. */
const WIDE: ReadonlyArray<readonly [number, number]> = [
  [0x231a, 0x231b], [0x23e9, 0x23ec], [0x23f0, 0x23f0], [0x23f3, 0x23f3], [0x25fd, 0x25fe], [0x2614, 0x2615],
  [0x2648, 0x2653], [0x267f, 0x267f], [0x2693, 0x2693], [0x26a1, 0x26a1], [0x26aa, 0x26ab], [0x26bd, 0x26be],
  [0x26c4, 0x26c5], [0x26ce, 0x26ce], [0x26d4, 0x26d4], [0x26ea, 0x26ea], [0x26f2, 0x26f3], [0x26f5, 0x26f5],
  [0x26fa, 0x26fa], [0x26fd, 0x26fd], [0x2705, 0x2705], [0x270a, 0x270b], [0x2728, 0x2728], [0x274c, 0x274c],
  [0x274e, 0x274e], [0x2753, 0x2755], [0x2757, 0x2757], [0x2795, 0x2797], [0x27b0, 0x27b0], [0x27bf, 0x27bf],
  [0x2b1b, 0x2b1c], [0x2b50, 0x2b50], [0x2b55, 0x2b55], [0x1f004, 0x1f004], [0x1f0cf, 0x1f0cf], [0x1f18e, 0x1f18e],
  [0x1f191, 0x1f19a], [0x1f200, 0x1f2ff],
];
function isWideSymbol(cp: number): boolean {
  if (cp < 0x231a || (cp > 0x2b55 && cp < 0x1f004) || cp > 0x1f2ff) return false;
  for (const [lo, hi] of WIDE) if (cp >= lo && cp <= hi) return true;
  return false;
}
