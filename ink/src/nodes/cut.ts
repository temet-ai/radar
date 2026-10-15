import { type Cell, cellsOf, widthOfCells } from "../text/cells.js";
import { cellWidth, fit } from "../chrome/width.js";

const LEAST = 4;

const textOf = (run: Cell[]) => run.map((c) => c.ch).join("");

/** view_tui::shrink: runs (one per span of a name) in `budget` cells by cutting the widest run with `…` until they
 * fit, so a short run after a long one (a branch word after a repo name) stays whole. A run is never cut under four
 * cells; when nothing is wide enough the line is dropped from the end. */
export function shrinkRuns(input: Cell[][], budget: number): Cell[][] {
  let runs = input.map((r) => [...r]);
  for (let guard = 0; guard < 200; guard++) {
    const widths = runs.map(widthOfCells);
    const total = widths.reduce((a, b) => a + b, 0);
    if (total <= budget) return runs;
    const excess = total - budget;
    let at = -1;
    let w = 0;
    widths.forEach((x, i) => {
      if (x >= w) {
        at = i;
        w = x; // max_by_key keeps the last of equals
      }
    });
    if (at < 0 || w <= LEAST) {
      if (runs.length > 1) {
        runs = runs.slice(0, -1);
        continue;
      }
      return runs;
    }
    const to = Math.max(w - excess, LEAST);
    const style = runs[at]![runs[at]!.length - 1]?.s ?? {};
    const cut = fit(textOf(runs[at]!).trimEnd(), to).trimEnd();
    runs[at] = cellsOf(cut, style);
    if (cellWidth(cut) >= w) return runs.slice(0, at + 1);
  }
  return runs;
}
