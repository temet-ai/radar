/** Terminal cell width of a string: wide (CJK, emoji) count 2, combining marks 0. */
export function cellWidth(s: string): number {
  let w = 0;
  for (const ch of s) w += charWidth(ch.codePointAt(0)!);
  return w;
}

export function charWidth(cp: number): number {
  if (cp === 0) return 0;
  if (cp < 32 || (cp >= 0x7f && cp < 0xa0)) return 0;
  if (
    (cp >= 0x300 && cp <= 0x36f) ||
    (cp >= 0x200b && cp <= 0x200f) ||
    (cp >= 0xfe00 && cp <= 0xfe0f) ||
    (cp >= 0x20d0 && cp <= 0x20ff)
  )
    return 0;
  if (
    (cp >= 0x1100 && cp <= 0x115f) ||
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
    for (const ch of seg.text) {
      const cw = charWidth(ch.codePointAt(0)!);
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
  for (const ch of s) {
    const cw = charWidth(ch.codePointAt(0)!);
    if (w + cw > max - 1) break;
    out += ch;
    w += cw;
  }
  return out + "…";
}
