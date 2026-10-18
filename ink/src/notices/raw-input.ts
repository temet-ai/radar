import { tokenize } from "../input/tokens.js";
import { PARTIAL_FLUSH_MS } from "../input/stdin-tap.js";
/** Read-only PTY observer. Never claims input, changes bytes, or reroutes keys to the host. */
export class RawNoticeInput {
  private rest = "";
  private timer?: ReturnType<typeof setTimeout>;
  private longPaste = false;
  constructor(private notify: (escape: boolean) => void) {}
  take(text: string): void {
    if (this.timer !== undefined) clearTimeout(this.timer); this.timer = undefined;
    let input = this.rest + text; this.rest = "";
    if (this.longPaste) {
      const end = input.indexOf("\x1b[201~");
      if (end < 0) { this.rest = input.slice(-5); return; }
      this.longPaste = false; input = input.slice(end + 6);
    }
    // A read may split a terminal report after ESC. Only a lone settled Esc clears all.
    const trailingEscape = input.endsWith("\x1b");
    if (trailingEscape) input = input.slice(0, -1);
    const parsed = tokenize(input), keys = parsed.events.filter((event) => event.kind === "key");
    if (keys.length) this.notify(keys.some((event) => event.kind === "key" && event.chord.key === "esc"));
    this.rest = parsed.rest + (trailingEscape ? "\x1b" : "");
    if (this.rest.startsWith("\x1b[200~") && this.rest.length > 4096) { this.longPaste = true; this.rest = this.rest.slice(-5); return; }
    if (this.rest && !this.rest.startsWith("\x1b[200~")) {
      this.rest = this.rest.slice(0, 4096);
      this.timer = setTimeout(() => { this.timer = undefined; if (this.rest === "\x1b") this.notify(true); this.rest = ""; }, PARTIAL_FLUSH_MS);
    }
  }
  reset(): void { if (this.timer !== undefined) clearTimeout(this.timer); this.timer = undefined; this.rest = ""; this.longPaste = false; }
}
