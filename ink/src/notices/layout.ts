import type { NoticeView } from "../protocol/generated/NoticeView.js";
import type { NoticeHistoryView } from "../protocol/generated/NoticeHistoryView.js";
import { clipCells, ellipsize, padCells, widthOfCells, type Cell } from "../text/cells.js";
import { bgOf, currentLook, hexRgb, iconOf, mix, rgbHex, slotStyle, specStyle, type Style } from "../text/look.js";
import type { Hit } from "../chrome/types.js";
import type { Rect } from "../motion/grid.js";
import { noticeCells, noticeText, wrapNotice } from "./text.js";

export type NoticeRow = { rect: Rect; cells: Cell[]; target: string };
export const NOTICE_OPEN = "chrome:notices:open";
export const noticeTarget = (id: number) => `chrome:notice:${id}`;
export function noticeContent(width: number, height: number, sources: boolean): Rect {
  const top = sources ? 2 : 1;
  return { x: 1, y: top + 1, w: Math.max(0, width - 2), h: Math.max(0, height - top - 3) };
}
/** Glyphs come from the host pack; icons-off retains readable severity. */
export function noticePrefix(notice: NoticeView, style: Style): Cell[] {
  const icon = iconOf(notice.icon ?? notice.tone);
  if (!icon?.glyph) return noticeCells(`${notice.tone} `, { ...style, ...slotStyle(`status.${notice.tone}`) });
  return noticeCells(`${icon.glyph} `, { ...style, ...specStyle(icon.style) });
}
/** A notice reads in from the right edge as it arrives and goes back into it in its last moments (ui/notices.rs EDGE_MS). */
export const NOTICE_EDGE_MS = 120;
/** Whether every notice on show reads whole in `area` (up to the 64-cell cap; ui/notices.rs `fits`). */
export function noticesFit(notices: NoticeView[], area: Rect): boolean {
  return notices.every((notice) => {
    const words = widthOfCells([...noticePrefix(notice, {}), ...noticeCells(noticeText(notice.text))]) + 2;
    return Math.min(words, 64) <= area.w;
  });
}
/** The whole row, as wide as its words (up to `room`), on its tinted ground. */
function rowCells(notice: NoticeView, room: number, now: number, older: boolean, motion: boolean, prefix = ""): Cell[] {
  const look = currentLook(), mono = look.tier === "mono", low = look.tier === "16" || look.tier === "256";
  const tone = `status.${notice.tone}`, ground = bgOf("bg.overlay");
  const style: Style = { ...slotStyle(low ? tone : "text.primary"), backgroundColor: ground,
    ...(mono ? { inverse: true } : {}), ...(older ? { dim: true } : {}), ...(!older && notice.tone === "error" ? { bold: true } : {}) };
  const words = [...noticePrefix(notice, style), ...noticeCells(prefix, style), ...noticeCells(noticeText(notice.text), style)];
  const width = Math.min(room, widthOfCells(words) + 2);
  const cells = padCells([...noticeCells(" ", style), ...ellipsize(words, Math.max(0, width - 2))], width, style);
  const a = hexRgb(ground), b = hexRgb(look.slots[tone]);
  if (look.tier !== "truecolor" || !a || !b) return cells;
  const tinted = rgbHex(mix(a, b, 180));
  const remaining = Math.max(0, notice.lifetime_ms - Math.max(0, now - notice.created_ms));
  let boundary = motion ? Math.ceil(width * remaining / Math.max(1, notice.lifetime_ms)) : width;
  let x = 0;
  for (const cell of cells) {
    if (x < boundary && x + cell.w > boundary) boundary = x;
    cell.s = { ...cell.s, backgroundColor: x < boundary ? tinted : ground }; x += cell.w;
  }
  return cells;
}
/** The right end of a row read in so far; a wide glyph the edge would cut stays hidden until it is whole. */
function revealed(notice: NoticeView, cells: Cell[], now: number, motion: boolean): Cell[] {
  const width = widthOfCells(cells), age = Math.max(0, now - notice.created_ms), left = Math.max(0, notice.lifetime_ms - age);
  const shown = motion ? Math.ceil(width * Math.min(age, left, NOTICE_EDGE_MS) / NOTICE_EDGE_MS) : width;
  const start = width - Math.min(width, shown);
  let x = 0;
  return cells.filter((cell) => { const keep = x >= start; x += cell.w; return keep; });
}
/** Four newest single rows, each as wide as its words, newest at the panel foot; overflow adds a history link row. */
export function layoutNotices(active: NoticeView[], hidden: number, content: Rect, now: number, motion: boolean): NoticeRow[] {
  if (content.w <= 0 || content.h <= 0) return [];
  const valid = active.filter((n) => now - n.created_ms < n.lifetime_ms).sort((a, b) => a.id - b.id);
  if (!valid.length) return [];
  const room = Math.min(64, content.w), right = content.x + content.w;
  let shown = valid.slice(-Math.min(4, content.h));
  let omitted = hidden + valid.length - shown.length;
  if (omitted && content.h >= 2) shown = valid.slice(-Math.min(4, content.h - 1));
  omitted = hidden + valid.length - shown.length;
  const summary = omitted > 0 && content.h >= 2;
  const y = content.y + content.h - shown.length;
  const out: NoticeRow[] = [];
  if (summary) {
    const s = { ...slotStyle("text.muted"), backgroundColor: bgOf("bg.overlay"), ...(currentLook().tier === "mono" ? { inverse: true } : {}) };
    const text = noticeCells(` +${omitted} earlier notices · ~ history`, s), w = Math.min(room, widthOfCells(text) + 1);
    out.push({ rect: { x: right - w, y: y - 1, w, h: 1 }, target: NOTICE_OPEN, cells: padCells(ellipsize(text, w), w, s) });
  }
  shown.forEach((notice, i) => {
    const cells = revealed(notice, rowCells(notice, room, now, i < shown.length - 1, motion, !summary && omitted ? `+${omitted} more · ` : ""), now, motion);
    const w = widthOfCells(cells);
    out.push({ rect: { x: right - w, y: y + i, w, h: 1 }, target: noticeTarget(notice.id), cells });
  });
  return out;
}
export const noticeHits = (rows: NoticeRow[]): Hit[] => rows.map((r) => ({ ...r.rect, target: r.target }));
export type NoticeHistoryLayout = { lines: Cell[][]; hits: Hit[] };
/** Separate session-only history; summaries and full details are inert. */
export function layoutNoticeHistory(view: NoticeHistoryView, width: number, height: number): NoticeHistoryLayout {
  if (width <= 0) return { lines: Array.from({ length: Math.max(0, height) }, () => []), hits: [] };
  const base: Style = { ...slotStyle("text.primary"), backgroundColor: bgOf("bg.base") }, w = Math.max(0, width - 2);
  const line = (cells: Cell[] = [], style: Style = base) => padCells([...noticeCells(" ", base), ...padCells(clipCells(cells, w), w, style)], width, base);
  const hits: Hit[] = [], lines = Array.from({ length: Math.max(0, height) }, () => line());
  // Native draw_line changes only text cells; header padding remains the plain body canvas.
  const set = (row: number, text: string, s: Style) => { const style = { ...base, ...s }; if (row >= 0 && row < height) lines[row] = line(ellipsize(noticeCells(text, style), w)); };
  set(0, "Recent notices", slotStyle("text.emphasis"));
  set(1, `This session only · ${view.entries.length} retained${view.omitted ? ` · ${view.omitted} older omitted` : ""}`, slotStyle("text.muted"));
  const listHeight = Math.min(Math.max(0, height - 3), Math.max(1, Math.floor(Math.max(0, height - 5) * 2 / 3)));
  const newest = [...view.entries].reverse(), selected = Math.min(view.selected, Math.max(0, newest.length - 1));
  let at = Math.min(view.scroll, Math.max(0, newest.length - listHeight));
  if (selected < at) at = selected;
  if (selected >= at + listHeight) at = Math.max(0, selected + 1 - listHeight);
  if (!newest.length) set(3, "No notices this session.", slotStyle("text.muted"));
  for (let i = 0; i < listHeight; i++) {
    const entry = newest[at + i]; if (!entry) continue;
    const s: Style = at + i === selected ? { ...slotStyle("text.emphasis"), backgroundColor: bgOf("bg.selection"), ...(currentLook().tier === "mono" ? { inverse: true } : {}) } : base;
    const cells = w <= 2 ? noticeCells(" ".repeat(w), s) : padCells([
      ...noticeCells(" ", s), ...ellipsize([...noticePrefix(entry, s), ...noticeCells(noticeText(entry.text), s)], Math.max(0, w - 2)),
    ], w, s);
    lines[3 + i] = line(cells, s); hits.push({ x: 1, y: 4 + i, w, h: 1, target: `chrome:notices:id:${entry.id}` });
  }
  if (view.detail) {
    const detail = view.detail, head = 3 + listHeight, wrapped = wrapNotice(detail.text, w), room = Math.max(0, height - head - 1);
    const scroll = Math.min(view.detail_scroll, Math.max(0, wrapped.length - room));
    set(head, `Notice details · ${detail.tone}${wrapped.length > room ? ` · lines ${scroll + 1}–${Math.min(scroll + room, wrapped.length)} of ${wrapped.length}` : ""}`, slotStyle("text.secondary"));
    for (let i = 0; i < room; i++) if (wrapped[scroll + i] !== undefined) set(head + 1 + i, wrapped[scroll + i]!, {});
  }
  return { lines, hits };
}
