import { MOUSE_OFF, MOUSE_ON } from "../input/mouse.js";

const ALT_ON = "\x1b[?1049h";
const ALT_OFF = "\x1b[?1049l";
const CURSOR_SHOW = "\x1b[?25h";
const CURSOR_HIDE = "\x1b[?25l";
const PASTE_ON = "\x1b[?2004h";
const PASTE_OFF = "\x1b[?2004l";
const FOCUS_ON = "\x1b[?1004h";
const FOCUS_OFF = "\x1b[?1004l";
const RESET = "\x1b[0m";

export type Writer = { write(s: string): unknown };

/** What leaving the terminal always does, on every exit path: idempotent, safe to repeat. */
export const RESTORE = MOUSE_OFF + FOCUS_OFF + PASTE_OFF + RESET + CURSOR_SHOW + ALT_OFF;

/**
 * The renderer owns the TTY while it runs: alternate screen, mouse reporting, cursor and raw
 * mode. Ink enters the alternate screen (`alternateScreen: true`) and sets raw mode; this
 * module adds mouse reporting and guarantees the restore sequence runs once on any exit.
 */
export class Terminal {
  private entered = false;
  private restored = false;

  constructor(private readonly out: Writer) {}

  enter() {
    if (this.entered) return;
    this.entered = true;
    // The same modes, in the same order, as the Ratatui runner (runner.rs).
    this.out.write(PASTE_ON);
    this.out.write(MOUSE_ON);
    this.out.write(FOCUS_ON);
  }

  restore() {
    if (this.restored) return;
    this.restored = true;
    this.out.write(RESTORE);
  }

  get isRestored() {
    return this.restored;
  }

  /** OSC 52: put text on the terminal's clipboard (only the TTY's owner writes it). */
  clipboard(text: string) {
    const b64 = Buffer.from(text, "utf8").toString("base64");
    this.out.write(`\x1b]52;c;${b64}\x07`);
  }
}

export { ALT_ON, ALT_OFF, CURSOR_HIDE, CURSOR_SHOW };
