import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { screenKey } from "../../src/input/route.js";
import type { Key } from "../../src/chrome/types.js";
import { tokenize } from "../../src/input/tokens.js";
import { InputController, type InputView } from "../../src/input/controller.js";
import type { Intent } from "../../src/chrome/types.js";
import type { UiState } from "../../src/state/ui-state.js";

// While an editor captures (Plugin::capturing, GUIDE 1.3) every key reaches the host as Intent::Key with the chord
// view_tui::chord_of makes, whatever the key is bound to elsewhere (q, ?, tab, esc, enter) and however it arrives:
// one key at a time or several in one read.
const press = (input: string, key: Record<string, boolean> = {}) => ({ input, key });
const run = (events: Array<{ input: string; key: Record<string, boolean> }>, capturing = true) => {
  const sent: Intent[] = [];
  const view: InputView = {
    menu: () => null,
    capturing: () => capturing,
    route: (chord) => ({ intent: "routed", chord }) as never,
    hits: () => [],
  };
  const c = new InputController(view, (i) => sent.push(i), { select() {} } as unknown as UiState);
  for (const e of events) c.key(e.input, e.key as never);
  return sent;
};
const k = (chord: Record<string, unknown>) => ({ intent: "key", chord });

// The class: every key of the contract (view::Key, as view_tui::chord_of maps it) with and without ctrl and alt, sent
// as a terminal sends it, through the real input path (tokenize, then the controller), reaches the host as the
// Chord the reference makes; keys chord_of has no name for (function keys, Insert) send nothing, and Shift is not in a Chord.
type NamedKey = Exclude<Key, { char: string }>;
type Bytes = (ctrl: boolean, alt: boolean, shift: boolean) => string | undefined;
const xterm = (final: string, tilde = false): Bytes => (ctrl, alt, shift) => {
  const m = 1 + (shift ? 1 : 0) + (alt ? 2 : 0) + (ctrl ? 4 : 0);
  const mods = m === 1 ? "" : `;${m}`;
  return tilde ? `\x1b[${final}${mods}~` : `\x1b[1${mods}${final}`;
};
// A byte for the key plain, ESC before it for alt; none for ctrl (a terminal has no byte for ctrl-enter and so on).
const byte = (b: string): Bytes => (ctrl, alt) => (ctrl ? undefined : alt ? `\x1b${b}` : b);
/** Every named key of the contract, as a terminal writes it. Typed `Record<NamedKey, ...>`: when the generated Key
 * gains a member this stops compiling (tsc) and the loops below would have no entry, so it cannot go unlisted. */
const ENCODE: Record<NamedKey, Bytes> = {
  up: xterm("A"), down: xterm("B"), right: xterm("C"), left: xterm("D"), home: xterm("H"), end: xterm("F"),
  page_up: xterm("5", true), page_down: xterm("6", true), delete: xterm("3", true),
  enter: byte("\r"), tab: byte("\t"), backspace: byte("\x7f"),
  esc: (ctrl, alt, shift) => (ctrl || alt || shift ? undefined : "\x1b"),
  back_tab: (ctrl, alt) => (ctrl || alt ? undefined : "\x1b[Z"),
};
const NAMED = Object.keys(ENCODE) as NamedKey[];
const seq = (name: NamedKey, ctrl: boolean, alt: boolean, shift = false) => ENCODE[name](ctrl, alt, shift);
const expectChord = (key: unknown, ctrl: boolean, alt: boolean) => ({ intent: "key", chord: { key, ...(ctrl ? { ctrl: true } : {}), ...(alt ? { alt: true } : {}) } });
const typed = (bytes: string, capturing = true) => {
  const sent: Intent[] = [];
  const view: InputView = { menu: () => null, capturing: () => capturing, route: (chord) => ({ intent: "routed", chord }) as never, hits: () => [] };
  const c = new InputController(view, (i) => sent.push(i), { select() {} } as unknown as UiState);
  for (const ev of tokenize(bytes).events) {
    if (ev.kind === "key") c.keyChord(ev.chord);
    else if (ev.kind === "paste") c.paste(ev.text);
  }
  return sent;
};
const MODS: Array<[boolean, boolean]> = [[false, false], [true, false], [false, true], [true, true]];

describe("a capturing editor gets every key of the contract as the reference's chord (characterisation of T10's input path)", () => {
  for (const name of NAMED) {
    for (const [ctrl, alt] of MODS) {
      const bytes = seq(name, ctrl, alt);
      it(`${name}${ctrl ? " +ctrl" : ""}${alt ? " +alt" : ""}${bytes === undefined ? " (no terminal encoding: nothing to send)" : ""}`, () => {
        if (bytes === undefined) return;
        expect(typed(bytes)).toEqual([expectChord(name, ctrl, alt)]);
      });
    }
    const shifted = seq(name, false, false, true);
    // Shift is not in a Chord: shift+arrow is the plain arrow. Keys with no shift encoding (enter, esc...) have none.
    if (shifted !== undefined && shifted !== seq(name, false, false)) {
      it(`${name} with shift is the plain key (a Chord carries no shift)`, () => {
        expect(typed(shifted)).toEqual([expectChord(name === "tab" ? "tab" : name, false, false)]);
      });
    }
  }

  it("every letter, plain, shifted, ctrl, alt and ctrl+alt", () => {
    for (let i = 0; i < 26; i++) {
      const lower = String.fromCharCode(97 + i);
      const upper = lower.toUpperCase();
      const ctrlByte = String.fromCharCode(i + 1);
      // \r, \t and \b (0x08) are Enter, Tab and Backspace on a terminal, not ctrl-m, ctrl-i and ctrl-h
      const ctrlIsLetter = !["m", "i", "h", "j"].includes(lower);
      expect(typed(lower)).toEqual([expectChord({ char: lower }, false, false)]);
      expect(typed(upper)).toEqual([expectChord({ char: upper }, false, false)]);
      expect(typed(`\x1b${lower}`)).toEqual([expectChord({ char: lower }, false, true)]);
      if (ctrlIsLetter) {
        expect(typed(ctrlByte)).toEqual([expectChord({ char: lower }, true, false)]);
        expect(typed(`\x1b${ctrlByte}`)).toEqual([expectChord({ char: lower }, true, true)]);
      }
    }
  });

  it("punctuation, space, digits and non-ASCII are characters, even the ones bound elsewhere (q ? / space)", () => {
    for (const c of ["q", "?", "/", " ", "0", "9", "é", "日", "☺", "!", "~"]) expect(typed(c), c).toEqual([expectChord({ char: c }, false, false)]);
  });

  it("function keys and Insert are no key in a Chord: nothing is sent", () => {
    for (const bytes of ["\x1bOP", "\x1bOQ", "\x1b[15~", "\x1b[17~", "\x1b[24~", "\x1b[2~", "\x1b[1;5P"]) expect(typed(bytes), JSON.stringify(bytes)).toEqual([]);
  });

  it("the same keys outside capture are routed, not forwarded", () => {
    expect(typed("q", false)).toEqual([{ intent: "routed", chord: { key: { char: "q" } } }]);
  });

  it("a bracketed paste is one char key per character, line breaks one space", () => {
    expect(typed("\x1b[200~a b/é\x1b[201~")).toHaveLength(5);
    expect(typed("\x1b[200~ab\ncd\x1b[201~").map((s) => (s as { chord: { key: { char: string } } }).chord.key.char).join("")).toBe("ab cd");
    expect(typed("\x1b[200~\n\nx\r\n\x1b[201~").map((s) => (s as { chord: { key: { char: string } } }).chord.key.char)).toEqual(["x"]);
  });

  it("a read holding many keys is the same intents as the keys one at a time", () => {
    const keys = ["h", "i", "\x7f", "\x1b[1;5D", "!", "\r", "\x1b[3~", "\x1b"];
    expect(typed(keys.join(""))).toEqual(keys.flatMap((k) => typed(k)));
    expect(typed(keys.join(""))).toHaveLength(keys.length);
  });
});

describe("the overlay view of intents.json sends every key to the editor", () => {
  it("screenKey on that view is Intent::Key for every chord, whatever the list under it binds", () => {
    const fixture = JSON.parse(readFileSync(new URL("../input/intents.json", import.meta.url), "utf8"));
    const entry = (fixture.screens as Array<{ name: string; view: { overlay?: unknown }; focus: string }>).find((e) => e.name === "overlay")!;
    expect(entry.view.overlay).toBeTruthy();
    const chords = NAMED.map((key) => ({ key })).concat(["j", "k", "g", "G", "+", "-", " "].map((c) => ({ key: { char: c } })) as never);
    for (const chord of chords) expect(screenKey(entry.view as never, entry.focus, chord as never), JSON.stringify(chord)).toEqual({ intent: "key", chord });
  });
});

// plugin_boundary's editor journey: type, esc (a draft asks before it is dropped), answer `yes` as an intent.
describe("leaving a draft asks, and the answer is an intent", () => {
  const ask = {
    id: "drop", title: "drop this reply draft?", question: true, selected: 0,
    items: [{ value: "yes", label: "yes, drop it", key: "y" }, { value: "no", label: "keep editing", key: "n" }],
  };
  const drive = (events: Array<{ input: string; key: Record<string, boolean> }>) => {
    const sent: Intent[] = [];
    let menu: unknown = null;
    let capturing = true;
    const view: InputView = { menu: () => menu as never, capturing: () => capturing, route: () => ({ intent: "routed" }) as never, hits: () => [] };
    const c = new InputController(view, (i) => {
      sent.push(i);
      if ((i as { chord?: { key?: unknown } }).chord?.key === "esc" && capturing) {
        // the host answers the first esc with the question and stops capturing
        menu = ask;
        capturing = false;
      }
    }, { select() {} } as unknown as UiState);
    for (const e of events) c.key(e.input, e.key as never);
    return sent;
  };

  it("esc reaches the editor, then y answers the question with Invoke yes", () => {
    expect(drive([press("h"), press("", { escape: true }), press("y")])).toEqual([
      k({ key: { char: "h" } }), k({ key: "esc" }), { intent: "invoke", action: "yes" },
    ]);
  });

  it("⏎ answers with the highlighted choice, in the same read as the esc", () => {
    expect(drive([press("", { escape: true }), press("", { return: true })])).toEqual([
      k({ key: "esc" }), { intent: "invoke", action: "yes" },
    ]);
  });

  it("n keeps editing", () => {
    expect(drive([press("", { escape: true }), press("n")]).at(-1)).toEqual({ intent: "invoke", action: "no" });
  });
});

describe("the key list cannot fall behind the contract", () => {
  it("ENCODE has exactly the named members of the generated Key type (protocol/generated/Key.ts)", () => {
    const src = readFileSync(new URL("../../src/protocol/generated/Key.ts", import.meta.url), "utf8");
    const decl = src.slice(src.indexOf("export type Key"));
    const named = [...decl.matchAll(/"([a-z_]+)"/g)].map((m) => m[1]!).filter((n) => n !== "char");
    expect([...named].sort()).toEqual([...NAMED].sort());
    expect(decl).toContain('{ "char": string }');
  });
});
