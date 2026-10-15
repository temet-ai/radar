import { describe, expect, it } from "vitest";
import { TerminalScreen } from "../../src/terminal/screen.js";

const ESC = "\x1b";

describe("TerminalScreen", () => {
  it("reads styled runs: palette, 256, true colour, and attributes", async () => {
    const s = new TerminalScreen(20, 3);
    await s.write(`${ESC}[31mred${ESC}[0m ${ESC}[1;38;5;200mpink${ESC}[0m ${ESC}[48;2;1;2;3mbg${ESC}[0m${ESC}[4mu${ESC}[0m`);
    const row = s.runs()[0]!;
    expect(row.map((r) => [r.text, r.color, r.backgroundColor, !!r.bold, !!r.underline])).toEqual([
      ["red", "red", undefined, false, false],
      [" ", undefined, undefined, false, false],
      ["pink", "ansi256(200)", undefined, true, false],
      [" ", undefined, undefined, false, false],
      ["bg", undefined, "#010203", false, false],
      ["u", undefined, undefined, false, true],
      [" ".repeat(20 - 12), undefined, undefined, false, false],
    ]);
  });

  it("every row is exactly as wide as the screen, wide characters included", async () => {
    const s = new TerminalScreen(10, 2);
    await s.write("a漢b");
    const width = (runs: { text: string }[]) => runs.reduce((n, r) => n + [...r.text].reduce((m, c) => m + (/[一-鿿]/.test(c) ? 2 : 1), 0), 0);
    for (const row of s.runs()) expect(width(row)).toBe(10);
  });

  it("draws the cursor as an inverted cell, only while it is visible", async () => {
    const s = new TerminalScreen(5, 1);
    await s.write("ab");
    expect(s.runs(true)[0]!.filter((r) => r.inverse).map((r) => r.text)).toEqual([" "]);
    await s.write(`${ESC}[?25l`);
    expect(s.runs(true)[0]!.some((r) => r.inverse)).toBe(false);
    await s.write(`${ESC}[?25h`);
    expect(s.modes.cursorVisible).toBe(true);
  });

  it("follows the program's modes", async () => {
    const s = new TerminalScreen(10, 3);
    expect(s.modes).toMatchObject({ mouse: "none", bracketedPaste: false, applicationCursorKeys: false, altScreen: false });
    await s.write(`${ESC}[?1000h${ESC}[?2004h${ESC}[?1h${ESC}[?1049h`);
    expect(s.modes).toMatchObject({ mouse: "vt200", bracketedPaste: true, applicationCursorKeys: true, altScreen: true });
  });

  it("looks back into the scrollback and returns to the program", async () => {
    const s = new TerminalScreen(10, 3);
    await s.write(Array.from({ length: 10 }, (_, i) => `line${i}`).join("\r\n"));
    expect(s.text()).toEqual(["line7", "line8", "line9"]);
    s.scroll(4);
    expect(s.text()).toEqual(["line3", "line4", "line5"]);
    expect(s.cursor()).toBeUndefined();
    s.scroll(-100);
    expect(s.text()).toEqual(["line7", "line8", "line9"]);
  });

  it("a resize keeps the text and reset clears it", async () => {
    const s = new TerminalScreen(10, 3);
    await s.write("hello");
    s.resize(20, 5);
    expect(s.text()[0]).toBe("hello");
    expect(s.runs()).toHaveLength(5);
    s.reset();
    expect(s.text().join("")).toBe("");
  });
});
