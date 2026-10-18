import type { Session } from "../app/session.js";
import { layoutSources } from "../chrome/layout.js";
import { parseHex } from "../motion/color.js";
import { type Cell, type Grid, type Rect } from "../motion/grid.js";
import type { PresentationLayer } from "../motion/motion.js";
import { currentLook, type Style } from "../text/look.js";
import { layoutNotices, noticeContent, noticeHits, type NoticeRow } from "./layout.js";
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
export function noticesVisible(session: Session): boolean {
  const state = session.state, snap = state.snapshot, chrome = snap?.chrome;
  return !!snap && state.phase === "ready" && chrome?.notices_visible === true && !chrome?.notice_history && !chrome?.history && !chrome?.tour && !chrome?.menu && !snap.splash && !snap.screen?.overlay && !state.inputRequest && state.questions.length === 0;
}
export function noticePresentation(session: Session, size: { columns?: number; rows?: number }, motion: () => boolean): PresentationLayer {
  const rows = () => {
    if (!noticesVisible(session)) { session.noticeHits = []; return []; }
    const width = size.columns ?? 80, height = size.rows ?? 24, sources = !!layoutSources(session.state.snapshot!.chrome, width, 1);
    return layoutNotices(session.noticeStore.active(), session.noticeStore.hidden, noticeContent(width, height, sources), session.noticeStore.now(), motion());
  };
  return {
    visible: () => rows().length > 0,
    running: () => motion() && currentLook().tier === "truecolor" && rows().length > 0,
    draw: (grid) => { const laid = rows(); session.noticeHits = noticeHits(laid); return paintNoticeRows(grid, laid); },
    clear: () => { session.noticeHits = []; },
  };
}
