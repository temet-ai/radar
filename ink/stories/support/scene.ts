// Finding a component's place on Ratatui's screen, and Ratatui's text recordings as cells.
import { charWidth } from "../../src/chrome/width";
import type { Cell } from "./CellGrid";
import type { Crop } from "./Compare";

const rowText = (row: Cell[]) => row.map((c) => (c.ch === "" ? "" : c.ch || " ")).join("");

/** The rectangle of the pane titled `title` (`╭ title ───╮` down to `╰───╯`) in a screen of cells. The `n`th match
 * when the same title is on screen twice. */
export function paneRect(cells: Cell[][], title: string, n = 0): Crop | undefined {
  let seen = 0;
  for (let y = 0; y < cells.length; y++) {
    const row = cells[y]!;
    for (let x = 0; x < row.length; x++) {
      if (row[x]!.ch !== "╭") continue;
      const after = rowText(row.slice(x + 1, x + 2 + title.length * 2));
      if (!after.startsWith(` ${title}`)) continue;
      if (seen++ < n) continue;
      let xr = x + 1;
      while (xr < row.length && row[xr]!.ch !== "╮") xr++;
      let yb = y + 1;
      while (yb < cells.length && cells[yb]![x]?.ch !== "╰") yb++;
      return { x, y, w: xr - x + 1, h: Math.min(yb, cells.length - 1) - y + 1 };
    }
  }
  return undefined;
}

/** Where the body starts on a Ratatui screen: under the top bar, and under the sources row when the screen has one
 * (the demo host's GitLab screens do; the recorded host_chrome fixture has none). */
export function bodyTopOf(cells: Cell[][]): number {
  const second = rowText(cells[1] ?? []);
  return /^\s+GitLab\s+GitHub/.test(second) ? 2 : 1;
}

/** A text recording (Ratatui's screen as text: tests/snapshots/mix__*.snap after the second `---`, or the
 * ink/test/fixtures/diff/*.txt files) as cells with no style. */
export function textCells(text: string): Cell[][] {
  return text.split("\n").map((line) => {
    const row: Cell[] = [];
    for (const ch of line) {
      const w = charWidth(ch.codePointAt(0)!);
      if (w === 0 && row.length) row[row.length - 1]!.ch += ch;
      else {
        row.push({ ch });
        if (w === 2) row.push({ ch: "" });
      }
    }
    return row;
  });
}

/** The screen in an insta snapshot: the body after the second `---`. */
export function snapBody(snap: string): string {
  const parts = snap.split(/^---$/m);
  return (parts[2] ?? "").replace(/^\n/, "").replace(/\n$/, "");
}
