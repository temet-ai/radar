import type { Modes } from "./screen.js";

/** Ctrl+] (0x1d) gives the keys back to the dock; no program binds it (telnet's escape). */
export const LEAVE = "\x1d";

export interface InputContext {
  modes: Modes;
  /** Where the surface's top-left cell is on the terminal, 1-based (as SGR reports count). Without it mouse reports are dropped. */
  origin?: { row: number; col: number };
  columns: number;
  rows: number;
}

export interface Translated {
  /** Bytes for the program. */
  data: string;
  /** Ctrl+] was typed: hand the keys back. */
  leave: boolean;
  /** A press landed outside the surface. */
  outside: boolean;
  /** The mouse reports outside the surface, as sent: radar's own (a tab's close mark, a pane), handled once the keys are back. */
  passOn: string;
  /** Lines to look back into the scrollback (positive: older), from the wheel when the program takes no mouse. */
  scroll: number;
  /** Anything but a wheel or motion was typed: follow the program again. */
  typed: boolean;
}

const SGR = /^\x1b\[<(\d+);(\d+);(\d+)([Mm])/;
const WHEEL_LINES = 3;

/**
 * Turns what the terminal sent while the surface has the keys into what the program should receive. Bytes pass
 * through: radar's own key tokenizer drops what it has no chord for (function keys, Ctrl+], unknown sequences), and a
 * program needs all of them. What changes: cursor keys in application mode, pastes (kept bracketed only if the
 * program asked), focus reports (only if asked), and mouse reports (moved into the surface's own coordinates, and
 * only the kinds the program asked for).
 */
export function translateInput(raw: string, ctx: InputContext): Translated {
  const out: Translated = { data: "", leave: false, outside: false, passOn: "", scroll: 0, typed: false };
  const m = ctx.modes;
  let i = 0;
  while (i < raw.length) {
    const rest = raw.slice(i);
    if (rest[0] === LEAVE) {
      out.leave = true;
      break;
    }
    if (rest[0] !== "\x1b") {
      let j = i + 1;
      while (j < raw.length && raw[j] !== "\x1b" && raw[j] !== LEAVE) j++;
      out.data += raw.slice(i, j);
      out.typed = true;
      i = j;
      continue;
    }
    const mouse = SGR.exec(rest);
    if (mouse) {
      i += mouse[0].length;
      const b = Number(mouse[1]);
      const col = Number(mouse[2]);
      const row = Number(mouse[3]);
      const press = mouse[4] === "M";
      if (!ctx.origin) continue;
      const x = col - ctx.origin.col + 1;
      const y = row - ctx.origin.row + 1;
      const inside = x >= 1 && x <= ctx.columns && y >= 1 && y <= ctx.rows;
      if (!inside) {
        if (press && !(b & 32) && !(b & 64)) out.outside = true;
        out.passOn += mouse[0];
        continue;
      }
      const wheel = (b & 64) !== 0;
      const motion = (b & 32) !== 0;
      if (m.mouse === "none") {
        if (wheel && !m.altScreen) out.scroll += (b & 1) === 0 ? WHEEL_LINES : -WHEEL_LINES;
        continue;
      }
      const buttonHeld = (b & 3) !== 3;
      const wanted =
        m.mouse === "x10" ? press && !wheel && !motion
        : m.mouse === "vt200" ? !motion
        : m.mouse === "drag" ? !motion || buttonHeld
        : true;
      if (!wanted) continue;
      out.data += `\x1b[<${b};${x};${y}${mouse[4]}`;
      if (!wheel && !motion) out.typed = true;
      continue;
    }
    if (rest.startsWith("\x1b[200~")) {
      const end = rest.indexOf("\x1b[201~");
      const body = end < 0 ? rest.slice(6) : rest.slice(6, end);
      i += end < 0 ? rest.length : end + 6;
      out.data += m.bracketedPaste ? `\x1b[200~${body}\x1b[201~` : body;
      out.typed = true;
      continue;
    }
    if (rest.startsWith("\x1b[I") || rest.startsWith("\x1b[O")) {
      if (m.focusReports) out.data += rest.slice(0, 3);
      i += 3;
      continue;
    }
    // A plain cursor key (no parameters): application mode wants SS3.
    const cursor = /^\x1b\[([ABCDHF])/.exec(rest);
    if (cursor && m.applicationCursorKeys) {
      out.data += `\x1bO${cursor[1]}`;
      out.typed = true;
      i += 3;
      continue;
    }
    // Any other escape: itself, up to the next ESC (so a following mouse report is still seen).
    let j = i + 1;
    while (j < raw.length && raw[j] !== "\x1b" && raw[j] !== LEAVE) j++;
    out.data += raw.slice(i, j);
    out.typed = true;
    i = j;
  }
  return out;
}
