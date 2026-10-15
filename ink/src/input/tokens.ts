import type { Chord, Key } from "../chrome/types.js";

/** One thing a read of stdin held, in the order it arrived. */
export type InputEvent = { kind: "key"; chord: Chord; /** xterm's shift modifier on a movement key (not part of the wire Chord). */ shift?: boolean } | { kind: "mouse"; data: string } | { kind: "focus"; on: boolean } | { kind: "paste"; text: string };

const MOUSE = /^\x1b\[<\d+;\d+;\d+[Mm]/;
const CSI_NAMED: Record<string, Key> = {
  A: "up",
  B: "down",
  C: "right",
  D: "left",
  H: "home",
  F: "end",
  Z: "back_tab",
};
const CSI_TILDE: Record<string, Key> = {
  "1": "home",
  "2": "tab", // Insert has no chord; never produced by a terminal for a key radar binds
  "3": "delete",
  "4": "end",
  "5": "page_up",
  "6": "page_down",
  "7": "home",
  "8": "end",
};

const withMods = (key: Key, m: number): Chord => {
  // xterm's modifier parameter: 1 + (shift 1, alt 2, ctrl 4, meta 8). Shift is carried by the key itself.
  const bits = Math.max(0, m - 1);
  return { key, ...(bits & 4 ? { ctrl: true } : {}), ...(bits & 2 || bits & 8 ? { alt: true } : {}) };
};

/** The chord of a lone code point: a control byte, or the character itself. */
function chordOfChar(c: string, alt: boolean): Chord | undefined {
  const code = c.codePointAt(0)!;
  const a = alt ? { alt: true } : {};
  if (c === "\r" || c === "\n") return { key: "enter", ...a };
  if (c === "\t") return { key: "tab", ...a };
  if (c === "\x7f" || c === "\x08") return { key: "backspace", ...a };
  if (code >= 1 && code <= 26) return { key: { char: String.fromCharCode(code + 96) }, ctrl: true, ...a };
  if (code < 32) return undefined; // other control bytes are no key radar binds
  return { key: { char: c }, ...a };
}

/**
 * A key a terminal reports by code point instead of by byte: kitty's `CSI code ; mods u` and xterm's
 * modifyOtherKeys `CSI 27 ; mods ; code ~` (a terminal or multiplexer that asks for them: Ghostty with the kitty
 * protocol on, tmux with extended-keys). Alt-w arrives as `\x1b[119;3u` there and as `\x1bw` otherwise; both are
 * the same chord. A key release is no key, and a code point that is no character (a modifier key, the keypad) is dropped.
 */
function chordOfCode(code: number, mods: number): Chord | undefined {
  const bits = Math.max(0, mods - 1);
  const alt = !!(bits & 2);
  const ctrl = !!(bits & 4);
  const shift = !!(bits & 1);
  if (!Number.isInteger(code) || code <= 0 || code > 0x10ffff || (code >= 0xd800 && code <= 0xdfff) || (code >= 0xe000 && code <= 0xf8ff)) return undefined;
  if (bits & 0xf8) return undefined; // super, hyper, meta, caps and num lock: not a chord radar binds
  const mod = { ...(ctrl ? { ctrl: true } : {}), ...(alt ? { alt: true } : {}) };
  if (code === 27) return { key: "esc", ...mod };
  if (code === 13) return { key: "enter", ...mod };
  if (code === 9) return shift ? { key: "back_tab" } : { key: "tab", ...mod };
  if (code === 127 || code === 8) return { key: "backspace", ...mod };
  if (code < 32) return undefined;
  const c = String.fromCodePoint(code);
  return { key: { char: shift && c.toLowerCase() !== c.toUpperCase() ? c.toUpperCase() : c }, ...mod };
}

/**
 * Splits what one read of stdin held into keys and mouse reports, in order. A terminal writes a key's whole
 * sequence in one go, so a sequence cut off at the end of the read is returned as `rest` for the caller to
 * join to the next read; a lone ESC is the Esc key.
 */
export function tokenize(text: string): { events: InputEvent[]; rest: string } {
  const events: InputEvent[] = [];
  const chars = [...text];
  let i = 0;
  const key = (chord: Chord | undefined, shift = false) => chord && events.push({ kind: "key", chord, ...(shift ? { shift: true } : {}) });
  while (i < chars.length) {
    const c = chars[i]!;
    if (c !== "\x1b") {
      key(chordOfChar(c, false));
      i++;
      continue;
    }
    const tail = chars.slice(i).join("");
    const mouse = MOUSE.exec(tail);
    if (mouse) {
      events.push({ kind: "mouse", data: mouse[0] });
      i += [...mouse[0]].length;
      continue;
    }
    if (tail.startsWith("\x1b[200~")) {
      // Bracketed paste (mode 2004, which the runner turns on): everything up to the closing bracket is text.
      const end = tail.indexOf("\x1b[201~");
      if (end < 0) return { events, rest: tail };
      events.push({ kind: "paste", text: tail.slice(6, end) });
      i += [...tail.slice(0, end + 6)].length;
      continue;
    }
    const n = chars[i + 1];
    if (n === undefined) {
      key({ key: "esc" });
      i++;
    } else if (n === "[") {
      // CSI: parameter bytes 0x30-0x3f, intermediates 0x20-0x2f, one final byte 0x40-0x7e.
      let j = i + 2;
      while (j < chars.length && /[\x30-\x3f\x20-\x2f]/.test(chars[j]!)) j++;
      if (j >= chars.length) return { events, rest: chars.slice(i).join("") };
      const params = chars.slice(i + 2, j).join("");
      const final = chars[j]!;
      if (params === "" && (final === "I" || final === "O")) {
        events.push({ kind: "focus", on: final === "I" }); // the terminal gained or lost focus (mode 1004)
        i = j + 1;
        continue;
      }
      const m = Number(params.split(";")[1] ?? 1) || 1;
      const named = CSI_NAMED[final];
      // CSI 1;2D is shift-left: shift is carried beside the chord for the editor's selection keys.
      const shift = (m - 1) % 2 === 1 && (named === "left" || named === "right" || named === "home" || named === "end");
      if (named) key(named === "back_tab" ? { key: "back_tab" } : withMods(named, m), shift);
      else if (final === "u") {
        // kitty: code[:shifted[:base]] ; mods[:event] . Event 3 is a release, which is no key.
        const [codes, modsField = "1"] = params.split(";");
        const [mod, event] = modsField.split(":");
        if (event !== "3") key(chordOfCode(Number(codes!.split(":")[0]), Number(mod) || 1));
      } else if (final === "~" && params.split(";")[0] === "27") {
        const [, mods = "1", code = ""] = params.split(";");
        key(chordOfCode(Number(code), Number(mods) || 1));
      } else if (final === "~") {
        const k = CSI_TILDE[params.split(";")[0]!];
        if (k && k !== "tab") key(withMods(k, m));
      }
      i = j + 1;
    } else if (n === "O") {
      const f = chars[i + 2];
      if (f === undefined) return { events, rest: chars.slice(i).join("") };
      const named = CSI_NAMED[f];
      if (named) key({ key: named });
      i += 3;
    } else if (n === "\x1b") {
      // ESC ESC: an Esc, then whatever follows.
      key({ key: "esc" });
      i++;
    } else {
      key(chordOfChar(n, true));
      i += 2;
    }
  }
  return { events, rest: "" };
}
