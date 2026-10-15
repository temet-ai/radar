import xtermPkg from "@xterm/headless";
import type { IBufferCell, Terminal as XTerminal } from "@xterm/headless";

const Terminal = xtermPkg.Terminal;

/** A run of cells drawn alike. Colours are Ink's `color` strings (a name, `ansi256(n)` or `#rrggbb`); none is the
 * terminal's own. */
export interface Run {
  text: string;
  color?: string;
  backgroundColor?: string;
  bold?: boolean;
  dim?: boolean;
  italic?: boolean;
  underline?: boolean;
  inverse?: boolean;
  strike?: boolean;
}

export type Row = Run[];

export type MouseTracking = "none" | "x10" | "vt200" | "drag" | "any";

/** What the program asked of its terminal, which decides how input is translated for it. */
export interface Modes {
  mouse: MouseTracking;
  bracketedPaste: boolean;
  focusReports: boolean;
  applicationCursorKeys: boolean;
  altScreen: boolean;
  cursorVisible: boolean;
}

const NAMES = [
  "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
  "blackBright", "redBright", "greenBright", "yellowBright", "blueBright", "magentaBright", "cyanBright", "whiteBright",
];

const hex = (n: number) => `#${n.toString(16).padStart(6, "0")}`;

function colourOf(cell: IBufferCell, fg: boolean): string | undefined {
  const rgb = fg ? cell.isFgRGB() : cell.isBgRGB();
  const pal = fg ? cell.isFgPalette() : cell.isBgPalette();
  const v = fg ? cell.getFgColor() : cell.getBgColor();
  if (rgb) return hex(v);
  if (pal) return v < 16 ? NAMES[v] : `ansi256(${v})`;
  return undefined;
}

/**
 * The program's screen: the bytes its terminal receives, parsed by xterm's headless engine, read back as rows of
 * styled runs. Draws nothing itself. Scrollback is the engine's; `scroll` looks back into it.
 */
export class TerminalScreen {
  private term: XTerminal;
  private cursorVisible = true;
  /** Lines the view is scrolled up into the scrollback (0 = following the program). */
  scrollOffset = 0;
  onChange: () => void = () => {};

  constructor(
    public columns: number,
    public rows: number,
    scrollback = 2000,
  ) {
    this.term = new Terminal({ cols: columns, rows, scrollback, allowProposedApi: true });
    // DECTCEM is not in the engine's public modes: watch it.
    this.term.parser.registerCsiHandler({ prefix: "?", final: "h" }, (p) => {
      if (p.includes(25)) this.cursorVisible = true;
      return false;
    });
    this.term.parser.registerCsiHandler({ prefix: "?", final: "l" }, (p) => {
      if (p.includes(25)) this.cursorVisible = false;
      return false;
    });
    this.term.onWriteParsed(() => this.onChange());
  }

  /** Feeds output bytes; resolves once they are parsed. */
  write(bytes: Uint8Array | string): Promise<void> {
    if (this.scrollOffset > 0 && typeof bytes !== "string" && bytes.length > 0) {
      // The engine keeps the viewport at the bottom; an offset into the scrollback stays the same lines.
    }
    return new Promise((resolve) => this.term.write(bytes, resolve));
  }

  resize(columns: number, rows: number) {
    columns = Math.max(1, columns);
    rows = Math.max(1, rows);
    if (columns === this.columns && rows === this.rows) return;
    this.columns = columns;
    this.rows = rows;
    this.term.resize(columns, rows);
    this.scrollOffset = Math.min(this.scrollOffset, this.scrollbackLines());
    this.onChange();
  }

  /** A restart: a clean screen. */
  reset() {
    this.term.reset();
    this.cursorVisible = true;
    this.scrollOffset = 0;
    this.onChange();
  }

  scrollbackLines(): number {
    const b = this.term.buffer.active;
    return b.type === "alternate" ? 0 : b.baseY;
  }

  /** Looks `lines` further up (negative: back down). */
  scroll(lines: number) {
    const next = Math.max(0, Math.min(this.scrollbackLines(), this.scrollOffset + lines));
    if (next === this.scrollOffset) return;
    this.scrollOffset = next;
    this.onChange();
  }

  get modes(): Modes {
    const m = this.term.modes;
    return {
      mouse: m.mouseTrackingMode,
      bracketedPaste: m.bracketedPasteMode,
      focusReports: m.sendFocusMode,
      applicationCursorKeys: m.applicationCursorKeysMode,
      altScreen: this.term.buffer.active.type === "alternate",
      cursorVisible: this.cursorVisible,
    };
  }

  /** The cursor's cell on the visible rows, or none when hidden or the view is scrolled back. */
  cursor(): { x: number; y: number } | undefined {
    if (!this.cursorVisible || this.scrollOffset > 0) return undefined;
    const b = this.term.buffer.active;
    return { x: b.cursorX, y: b.cursorY };
  }

  /** The text of the visible rows, trailing blanks cut (for tests and for finding the last used row). */
  text(): string[] {
    const b = this.term.buffer.active;
    const out: string[] = [];
    for (let y = 0; y < this.rows; y++) out.push(b.getLine(b.viewportY - this.scrollOffset + y)?.translateToString(true) ?? "");
    return out;
  }

  /** Index of the last row holding text or the cursor, for the screen of a program that ended. */
  lastUsedRow(): number {
    const t = this.text();
    let last = -1;
    t.forEach((s, i) => s.trim() !== "" && (last = i));
    return Math.max(last, this.term.buffer.active.cursorY, 0);
  }

  /** The visible rows as styled runs; every row is `columns` cells wide. `cursor` draws the cursor cell inverted. */
  runs(drawCursor = false): Row[] {
    const b = this.term.buffer.active;
    const cur = drawCursor ? this.cursor() : undefined;
    const rows: Row[] = [];
    let cell: IBufferCell | undefined;
    for (let y = 0; y < this.rows; y++) {
      const line = b.getLine(b.viewportY - this.scrollOffset + y);
      const row: Row = [];
      let run: Run | undefined;
      for (let x = 0; x < this.columns; x++) {
        const c = line?.getCell(x, cell);
        if (c) cell = c;
        if (!c || c.getWidth() === 0) continue; // the right half of a wide character
        const chars = c.getChars() || " ";
        const atCursor = !!cur && cur.x === x && cur.y === y;
        const style: Run = { text: "" };
        const fg = colourOf(c, true);
        const bg = colourOf(c, false);
        if (fg) style.color = fg;
        if (bg) style.backgroundColor = bg;
        if (c.isBold()) style.bold = true;
        if (c.isDim()) style.dim = true;
        if (c.isItalic()) style.italic = true;
        if (c.isUnderline()) style.underline = true;
        if (c.isStrikethrough()) style.strike = true;
        if (!!c.isInverse() !== atCursor) style.inverse = true;
        if (c.isInvisible()) style.dim = true;
        if (run && sameStyle(run, style)) run.text += chars;
        else {
          run = { ...style, text: chars };
          row.push(run);
        }
      }
      rows.push(row);
    }
    return rows;
  }

  dispose() {
    this.term.dispose();
  }
}

function sameStyle(a: Run, b: Run): boolean {
  return (
    a.color === b.color && a.backgroundColor === b.backgroundColor && !!a.bold === !!b.bold && !!a.dim === !!b.dim &&
    !!a.italic === !!b.italic && !!a.underline === !!b.underline && !!a.inverse === !!b.inverse && !!a.strike === !!b.strike
  );
}
