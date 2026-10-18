import { charWidth } from "../chrome/width.js";
import type { Cell } from "../text/cells.js";
import type { Style } from "../text/look.js";
const segmenter = new Intl.Segmenter(undefined, { granularity: "grapheme" });
/** Complete terminal clusters, including combining marks and joined emoji. */
export function noticeCells(text: string, s: Style = {}): Cell[] {
  const out: Cell[] = [];
  for (const { segment } of segmenter.segment(text)) {
    const cp = segment.codePointAt(0)!;
    const w = segment.includes("\u200d") || segment.includes("\ufe0f") || cp >= 0x1f1e6 && cp <= 0x1f1ff ? 2 : charWidth(cp);
    if (w > 0) out.push({ ch: segment, w, s });
  }
  return out;
}
export const noticeText = (text: string): string => text.replace(/[\u0000-\u001f\u007f-\u009f\s]+/gu, " ").trim();
export const noticeWidth = (text: string): number => noticeCells(text).reduce((n, c) => n + c.w, 0);
/** Whitespace-token wrapping with cluster-safe hard splits, shared with native notices. */
export function wrapNotice(text: string, width: number): string[] {
  if (width <= 0) return [];
  const out: string[] = [];
  for (const paragraph of text.split("\n")) {
    let line = "", used = 0;
    for (const word of noticeText(paragraph).split(/\s+/u).filter(Boolean)) {
      if (used && used + 1 + noticeWidth(word) > width) { out.push(line); line = ""; used = 0; }
      if (used) { line += " "; used++; }
      for (const cell of noticeCells(word)) {
        if (used + cell.w > width && line) { out.push(line); line = ""; used = 0; }
        if (cell.w > width) { line += "…"; used++; } else { line += cell.ch; used += cell.w; }
      }
    }
    out.push(line);
  }
  return out;
}
