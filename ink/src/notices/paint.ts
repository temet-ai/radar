import type { Session } from "../app/session.js";
import { layoutSources } from "../chrome/layout.js";
import { parseHex } from "../motion/color.js";
import { type Cell, type Grid, type Rect } from "../motion/grid.js";
import type { PresentationLayer } from "../motion/motion.js";
import { currentLook, type Style } from "../text/look.js";
import { layoutNotices, noticeContent, noticeHits, noticesFit, type NoticeRow } from "./layout.js";
const names = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white", "blackBright", "redBright", "greenBright", "yellowBright", "blueBright", "magentaBright", "cyanBright", "whiteBright"];
function colourAttr(value: string | undefined, background: boolean): string {
  if (!value || parseHex(value) >= 0) return "";
  const indexed = /^ansi256\((\d+)\)$/.exec(value);
  if (indexed) return `${background ? 48 : 38};5;${indexed[1]}`;
  const at = names.indexOf(value === "gray" || value === "grey" ? "blackBright" : value);
  return at < 0 ? "" : String((background ? 40 : 30) + at % 8 + (at >= 8 ? 60 : 0));
}
export function noticeStyle(style: Style): Omit<Cell, "ch"> {
  const flags = [style.bold && "1", style.dim && "2", style.italic && "3", style.underline && "4", style.inverse && "7", style.strike && "9", colourAttr(style.color, false), colourAttr(style.backgroundColor, true)].filter(Boolean);
  return { fg: parseHex(style.color), bg: parseHex(style.backgroundColor), attr: flags.join(";") };
}
/** Late painting never enters clean-frame retention. */
export function paintNoticeRows(grid: Grid, rows: NoticeRow[]): Rect[] {
  const areas: Rect[] = [];
  for (const row of rows) {
    const { x, y, w } = row.rect;
    if (x < 0 || y < 0 || x + w > grid.width || y >= grid.height) continue;
    // Clear only the symbol of an underlying wide half crossing an edge; preserve its outside style.
    if (x > 0 && grid.at(x, y).ch === "") grid.at(x - 1, y).ch = " ";
    if (x + w < grid.width && grid.at(x + w, y).ch === "") grid.at(x + w, y).ch = " ";
    let column = x;
    for (const cell of row.cells) {
      const style = noticeStyle(cell.s); Object.assign(grid.at(column, y), style, { ch: cell.ch });
      for (let tail = 1; tail < cell.w; tail++) Object.assign(grid.at(column + tail, y), style, { ch: "" });
      column += cell.w;
    }
    areas.push(row.rect);
  }
  return areas;
}
const BOTTOM_RIGHT = new Set(["╯", "┘", "┛"]), BOTTOM_LEFT = new Set(["╰", "└", "┗"]), TOP_LEFT = new Set(["╭", "┌", "┏"]), SIDE = new Set(["│", "┃", "├", "┣"]);
/** The inside of the panel standing on the status line at the right edge, read from the frame just drawn
 * (shell/notices.rs `anchor`): notices belong inside its border. `fallback` when no such panel is drawn there. */
export function noticeAnchor(grid: Grid, fallback: Rect): Rect {
  const { width, height } = grid;
  if (width < 3 || height < 4) return fallback;
  const right = width - 1, bottom = height - 2, ch = (x: number, y: number) => grid.at(x, y).ch;
  if (!BOTTOM_RIGHT.has(ch(right, bottom))) return fallback;
  let left = right - 1;
  while (left >= 0 && !BOTTOM_LEFT.has(ch(left, bottom)) && !BOTTOM_RIGHT.has(ch(left, bottom))) left--;
  if (left < 0 || !BOTTOM_LEFT.has(ch(left, bottom))) return fallback;
  let top = bottom;
  for (;;) {
    if (top <= 0) return fallback;
    top--;
    if (TOP_LEFT.has(ch(left, top))) break;
    if (!SIDE.has(ch(left, top))) return fallback;
  }
  const inner = { x: left + 1, y: top + 1, w: right - left - 1, h: bottom - top - 1 };
  return inner.w > 0 && inner.h > 0 ? inner : fallback;
}
export function noticesVisible(session: Session): boolean {
  const state = session.state, snap = state.snapshot, chrome = snap?.chrome;
  return !!snap && state.phase === "ready" && chrome?.notices_visible === true && !chrome?.notice_history && !chrome?.history && !chrome?.tour && !chrome?.menu && !snap.splash && !snap.screen?.overlay && !state.inputRequest && state.questions.length === 0;
}
export function noticePresentation(session: Session, size: { columns?: number; rows?: number }, motion: () => boolean): PresentationLayer {
  const rows = (grid?: Grid) => {
    if (!noticesVisible(session)) { session.noticeHits = []; return []; }
    const width = size.columns ?? 80, height = size.rows ?? 24, sources = !!layoutSources(session.state.snapshot!.chrome, width, 1);
    const content = noticeContent(width, height, sources);
    // In the bottom-right panel when every notice reads whole there; across the content otherwise.
    const active = session.noticeStore.active(), panel = grid ? noticeAnchor(grid, content) : content;
    return layoutNotices(active, session.noticeStore.hidden, noticesFit(active, panel) ? panel : content, session.noticeStore.now(), motion());
  };
  return {
    visible: () => rows().length > 0,
    running: () => motion() && currentLook().tier === "truecolor" && rows().length > 0,
    draw: (grid) => { const laid = rows(grid); session.noticeHits = noticeHits(laid); return paintNoticeRows(grid, laid); },
    clear: () => { session.noticeHits = []; },
  };
}
