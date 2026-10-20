// The panels (╭ title ─╮ … ╰─╯) on a drawn frame, found by their borders.

export interface Panel {
  title: string;
  top: string;
  start: number;
  end: number;
  /** Text rows inside the border, side bars and scrollbar taken off, blank rows dropped. */
  text: string[];
  bottom: string;
  /** Another panel is drawn over this one's lower part (a menu or question over the screen): its text below that is not this panel's. */
  covered: boolean;
  /** This panel is drawn over another. */
  overlay: boolean;
}

/** Every panel with a top border on `lines`: found by its `╭ … ╮`, followed down to its `╰ … ╯`. A panel drawn over
 * another (a menu or question over the screen) is an `overlay`, and the rows it covers are not the covered panel's text. */
export function panels(lines: string[]): Panel[] {
  const out: Array<Panel & { y: number }> = [];
  lines.forEach((line, y) => {
    const cells = [...line];
    for (let i = 0; i < cells.length; i++) {
      if (cells[i] !== "╭") continue;
      const j = cells.indexOf("╮", i);
      if (j < 0) continue;
      let bottom = "";
      let by = lines.length;
      for (let r = y + 1; r < lines.length; r++) {
        const row = [...lines[r]!];
        if (row[i] === "╰" && row.slice(i + 1, j).every((c) => c === "─") && row[j] === "╯") {
          bottom = row.slice(i, j + 1).join("");
          by = r;
          break;
        }
      }
      const top = cells.slice(i, j + 1).join("");
      out.push({ title: top.replace(/^╭ /, "").replace(/ ?─*╮$/, ""), top, start: i, end: j, text: [], bottom, covered: false, overlay: false, y, by } as never);
      i = j;
    }
  });
  for (const b of out as Array<Panel & { y: number; by: number }>) {
    for (const a of out as Array<Panel & { y: number; by: number }>) {
      if (a !== b && a.y < b.y && b.y < a.by && b.start <= a.end && b.end >= a.start) {
        b.overlay = true;
        a.covered = true;
        (a as { coverY?: number }).coverY = Math.min((a as { coverY?: number }).coverY ?? Infinity, b.y);
      }
    }
  }
  for (const a of out as Array<Panel & { y: number; by: number; coverY?: number }>) {
    const stop = Math.min(a.by, a.coverY ?? Infinity);
    for (let r = a.y + 1; r < stop && r < lines.length; r++) {
      const inner = [...lines[r]!].slice(a.start + 1, a.end).join("").trim();
      if (inner) a.text.push(inner);
    }
  }
  return out;
}

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

/** The titles of the panels the reference paints for a recorded Work view at a size (written by radar-workbench's tests/work_area.rs). */
export function referenceTitles(view: string, columns: number, rows: number): string[] {
  const url = new URL(`../../../crates/radar-workbench/tests/fixtures/${view}_reference_${columns}x${rows}.txt`, import.meta.url);
  const lines = readFileSync(fileURLToPath(url), "utf8").replace(/\n$/, "").split("\n").slice(1, -1);
  return panels(lines).map((p) => p.title);
}
