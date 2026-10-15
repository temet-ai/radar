import { describe, expect, it } from "vitest";
import { tokenize } from "../../src/input/tokens.js";

const keys = (s: string) => tokenize(s).events.flatMap((e) => (e.kind === "key" ? [e.chord] : e.kind === "mouse" ? [e.data] : []));

describe("tokenize: one read is keys and pointer reports, in order", () => {
  it("control bytes after text are Enter and ctrl-letter, not characters", () => {
    expect(keys("jjk\r")).toEqual([{ key: { char: "j" } }, { key: { char: "j" } }, { key: { char: "k" } }, { key: "enter" }]);
    expect(keys("x\x12")).toEqual([{ key: { char: "x" } }, { key: { char: "r" }, ctrl: true }]);
    expect(keys("\t\x7f")).toEqual([{ key: "tab" }, { key: "backspace" }]);
  });
  it("a report keeps its place between keys", () => {
    expect(keys("j\x1b[<0;5;3Mj")).toEqual([{ key: { char: "j" } }, "\x1b[<0;5;3M", { key: { char: "j" } }]);
  });
  it("sequences", () => {
    expect(keys("\x1b[A\x1b[B\x1b[C\x1b[D\x1b[5~\x1b[6~\x1b[3~\x1b[H\x1b[F\x1b[Z\x1bOA")).toEqual([
      { key: "up" }, { key: "down" }, { key: "right" }, { key: "left" }, { key: "page_up" }, { key: "page_down" },
      { key: "delete" }, { key: "home" }, { key: "end" }, { key: "back_tab" }, { key: "up" },
    ]);
    expect(keys("\x1b[1;5A")).toEqual([{ key: "up", ctrl: true }]);
  });
  it("a lone ESC is Esc, ESC and a character is Alt-character", () => {
    expect(keys("\x1b")).toEqual([{ key: "esc" }]);
    expect(keys("\x1bx")).toEqual([{ key: { char: "x" }, alt: true }]);
  });
  it("alt-w is the same chord whichever way the terminal writes it: ESC w, kitty CSI-u, modifyOtherKeys", () => {
    const altW = [{ key: { char: "w" }, alt: true }];
    expect(keys("\x1bw")).toEqual(altW);
    expect(keys("\x1b[119;3u")).toEqual(altW);
    expect(keys("\x1b[119;3:1u")).toEqual(altW); // kitty's explicit key press
    expect(keys("\x1b[27;3;119~")).toEqual(altW);
    expect(keys("\x1b[119;3:3u")).toEqual([]); // the release is no key
    expect(keys("\x1b[119;5u")).toEqual([{ key: { char: "w" }, ctrl: true }]);
    expect(keys("\x1b[119u")).toEqual([{ key: { char: "w" } }]);
    expect(keys("\x1b[119;7u")).toEqual([{ key: { char: "w" }, ctrl: true, alt: true }]);
  });
  it("kitty keys: shift makes a capital, Esc, Enter, Tab and Backspace map, a modifier key and a super chord are none", () => {
    expect(keys("\x1b[119;2u")).toEqual([{ key: { char: "W" } }]);
    expect(keys("\x1b[27u\x1b[13u\x1b[9u\x1b[127u")).toEqual([{ key: "esc" }, { key: "enter" }, { key: "tab" }, { key: "backspace" }]);
    expect(keys("\x1b[13;3u")).toEqual([{ key: "enter", alt: true }]);
    expect(keys("\x1b[57441u\x1b[119;9u")).toEqual([]);
  });
  it("a CSI-u cut off by the end of the read is held", () => {
    expect(tokenize("\x1b[119;3")).toEqual({ events: [], rest: "\x1b[119;3" });
  });
  it("a sequence cut off by the end of the read is held", () => {
    expect(tokenize("j\x1b[")).toEqual({ events: [{ kind: "key", chord: { key: { char: "j" } } }], rest: "\x1b[" });
  });
  it("focus reports and paste brackets (modes the runner turns on) are no keys; pasted text is keys", () => {
    expect(keys("\x1b[I\x1b[O")).toEqual([]);
    expect(tokenize("\x1b[I\x1b[O").events).toEqual([{ kind: "focus", on: true }, { kind: "focus", on: false }]);
    expect(tokenize("\x1b[200~ab\ncd\x1b[201~x").events).toEqual([{ kind: "paste", text: "ab\ncd" }, { kind: "key", chord: { key: { char: "x" } } }]);
    expect(tokenize("\x1b[200~ab")).toEqual({ events: [], rest: "\x1b[200~ab" });
  });
  it("multi-byte characters are one key each", () => {
    expect(keys("é☺")).toEqual([{ key: { char: "é" } }, { key: { char: "☺" } }]);
  });
});
