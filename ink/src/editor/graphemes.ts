// Unicode-aware text measurement for the editor: grapheme clusters (combining marks and emoji sequences stay one
// cursor stop) and terminal cell widths (East Asian wide and emoji take two cells, combining marks none).
const segmenter = new Intl.Segmenter(undefined, { granularity: "grapheme" });

export function graphemes(text: string): string[] {
  if (/^[\x20-\x7e]*$/.test(text)) return text.split("");
  return Array.from(segmenter.segment(text), (s) => s.segment);
}

const WIDE: [number, number][] = [
  [0x1100, 0x115f], [0x2e80, 0x303e], [0x3041, 0x33ff], [0x3400, 0x4dbf], [0x4e00, 0x9fff], [0xa000, 0xa4cf],
  [0xac00, 0xd7a3], [0xf900, 0xfaff], [0xfe30, 0xfe6f], [0xff00, 0xff60], [0xffe0, 0xffe6], [0x1f300, 0x1f64f],
  [0x1f900, 0x1f9ff], [0x20000, 0x3fffd],
];

function isCombining(cp: number): boolean {
  return (cp >= 0x300 && cp <= 0x36f) || (cp >= 0x1ab0 && cp <= 0x1aff) || (cp >= 0x20d0 && cp <= 0x20ff) ||
    (cp >= 0xfe00 && cp <= 0xfe0f) || cp === 0x200d || (cp >= 0xfe20 && cp <= 0xfe2f);
}

/** Cells one grapheme takes on a terminal. */
export function graphemeWidth(g: string): number {
  const first = g.codePointAt(0) ?? 0;
  if (first < 0x20 || (first >= 0x7f && first < 0xa0)) return 0;
  if (isCombining(first)) return 0;
  // An emoji presentation selector or a joiner sequence makes the cluster wide.
  if (g.includes("️") || g.includes("‍")) return 2;
  return WIDE.some(([lo, hi]) => first >= lo && first <= hi) ? 2 : 1;
}

export function textWidth(text: string): number {
  return graphemes(text).reduce((n, g) => n + graphemeWidth(g), 0);
}

/** The code-point index (Rust `char` offset, how the view counts a caret) -> grapheme index in `text`. */
export function graphemeAtChar(text: string, charIndex: number): number {
  let chars = 0;
  const gs = graphemes(text);
  for (let i = 0; i < gs.length; i++) {
    if (chars >= charIndex) return i;
    chars += Array.from(gs[i]).length;
  }
  return gs.length;
}
