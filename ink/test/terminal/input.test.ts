import { describe, expect, it } from "vitest";
import { translateInput, type InputContext } from "../../src/terminal/input.js";
import type { Modes } from "../../src/terminal/screen.js";

const plain: Modes = { mouse: "none", bracketedPaste: false, focusReports: false, applicationCursorKeys: false, altScreen: false, cursorVisible: true };
const ctx = (m: Partial<Modes> = {}, origin = { row: 10, col: 20 }): InputContext => ({ modes: { ...plain, ...m }, origin, columns: 40, rows: 8 });

describe("translateInput", () => {
  it("passes keys through whole, including what radar's key tokenizer would drop", () => {
    // F5, Alt-x, Home with ctrl: none of these may be lost on the way to the program.
    const raw = "abc\x1b[15~\x1bx\x1b[1;5H\r";
    expect(translateInput(raw, ctx()).data).toBe(raw);
  });

  it("Ctrl+] hands the keys back and nothing after it reaches the program", () => {
    const t = translateInput("ab\x1dcd", ctx());
    expect(t).toMatchObject({ data: "ab", leave: true });
  });

  it("cursor keys follow the program's mode", () => {
    expect(translateInput("\x1b[A\x1b[1;5A", ctx({ applicationCursorKeys: true })).data).toBe("\x1bOA\x1b[1;5A");
    expect(translateInput("\x1b[A", ctx()).data).toBe("\x1b[A");
  });

  it("a paste is bracketed only when the program asked", () => {
    expect(translateInput("\x1b[200~a\nb\x1b[201~", ctx({ bracketedPaste: true })).data).toBe("\x1b[200~a\nb\x1b[201~");
    expect(translateInput("\x1b[200~a\nb\x1b[201~", ctx()).data).toBe("a\nb");
  });

  it("focus reports go only to a program that asked", () => {
    expect(translateInput("\x1b[I", ctx()).data).toBe("");
    expect(translateInput("\x1b[I", ctx({ focusReports: true })).data).toBe("\x1b[I");
  });

  describe("mouse", () => {
    // Surface origin is (row 10, col 20), 1-based; a click at screen column 25, row 12 is cell (6, 3) in it.
    const click = "\x1b[<0;25;12M";
    it("moves a report into the surface's coordinates when the program tracks the mouse", () => {
      expect(translateInput(click, ctx({ mouse: "vt200" })).data).toBe("\x1b[<0;6;3M");
    });
    it("sends nothing when the program does not track the mouse", () => {
      expect(translateInput(click, ctx()).data).toBe("");
    });
    it("sends only the kinds the program asked for", () => {
      const move = "\x1b[<35;25;12M";
      expect(translateInput(move, ctx({ mouse: "vt200" })).data).toBe("");
      expect(translateInput(move, ctx({ mouse: "any" })).data).toBe("\x1b[<35;6;3M");
      expect(translateInput("\x1b[<0;25;12m", ctx({ mouse: "x10" })).data).toBe("");
    });
    it("a press outside gives the keys back; motion outside is dropped", () => {
      expect(translateInput("\x1b[<0;5;2M", ctx({ mouse: "any" }))).toMatchObject({ data: "", outside: true });
      expect(translateInput("\x1b[<35;5;2M", ctx({ mouse: "any" }))).toMatchObject({ data: "", outside: false });
    });
    it("reports outside the surface are passed on to radar as sent (a click on a tab's close mark closes it)", () => {
      const t = translateInput("\x1b[<0;5;2M\x1b[<0;5;2m\x1b[<0;25;12M", ctx({ mouse: "any" }));
      expect(t.passOn).toBe("\x1b[<0;5;2M\x1b[<0;5;2m");
      expect(t.data).toBe("\x1b[<0;6;3M");
    });
    it("the wheel looks back into the scrollback when the program takes no mouse", () => {
      expect(translateInput("\x1b[<64;25;12M", ctx()).scroll).toBe(3);
      expect(translateInput("\x1b[<65;25;12M", ctx()).scroll).toBe(-3);
      expect(translateInput("\x1b[<64;25;12M", ctx({ altScreen: true })).scroll).toBe(0);
    });
    it("without a known origin a mouse report is dropped", () => {
      expect(translateInput(click, { ...ctx({ mouse: "any" }), origin: undefined }).data).toBe("");
    });
    it("typing follows the program again; the wheel and motion do not", () => {
      expect(translateInput("x", ctx()).typed).toBe(true);
      expect(translateInput("\x1b[<64;25;12M", ctx()).typed).toBe(false);
    });
  });
});
