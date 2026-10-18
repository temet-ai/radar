import type { Block, DetailContent } from "./view.js";
import { cellsOf, clipCells, lineCells, padCells, paragraphCells, widthOfCells, type Cell } from "../text/cells.js";
import { detailFactRows, detailHeaderRows, fitDetailCells, survivingFacts } from "./detail.js";
import { lineRows, sectionHeading } from "./index.js";
import { docRows } from "../text/richRows.js";
import { slotStyle, specStyle } from "../text/look.js";
import { wrapWords } from "../text/wrap.js";
import { segWords } from "../text/TextLine.js";
import { cellWidth } from "../chrome/width.js";
import type { HitSpec } from "./hits.js";

export interface DetailRows { rows: Cell[][]; hits: HitSpec[] }
const mark = { done: "✓", failed: "✗", running: "●", waiting: "◷", manual: "▷", stopped: "⊘", skipped: "»" };
const textOf = (cells: Cell[]) => cells.map((c) => c.ch).join("");

/** Exact shared row geometry, used for painting, scrolling and every inline hit. */
export function detailBodyRows(content: DetailContent, width: number, plain = false): DetailRows {
  if (width <= 0) return { rows: [], hits: [] };
  const pad = width >= 3 ? 1 : 0, room = width - 2 * pad;
  const rows = detailHeaderRows(content, width, plain), hits: HitSpec[] = [];
  const facts = survivingFacts(content.facts), sections = content.sections ?? [];
  if (facts.length || sections.length) rows.push([]);
  rows.push(...detailFactRows(facts, width, plain));
  if (facts.length && sections.length) rows.push([]);
  const append = (block: Block, w: number, left: number) => {
    if (w <= 0) return;
    const start = rows.length;
    let made: Cell[][] = [];
    switch (block.block) {
      case "section":
        rows.push([...cellsOf(" ".repeat(left)), ...fitDetailCells(sectionHeading(block, w, plain), w, plain ? {} : slotStyle("text.muted"))]);
        for (const child of block.blocks) append(child, Math.max(0, w - 2), left + 2);
        return;
      case "line": {
        made = lineRows(block, w, w, plain);
        if (block.target) for (const [i, row] of made.entries()) if (widthOfCells(row)) hits.push({ dx: left, dy: start + i, w, h: 1, target: block.target });
        // Wrapping only consumes gaps. Match graphemes back to the complete rendered source,
        // including key prefixes, so wide/combining characters retain truthful target cells.
        const source = textOf(lineRows(block, w, Number.MAX_SAFE_INTEGER, plain)[0] ?? []);
        const base = textOf(lineCells(block.text, plain));
        let offset = Math.max(0, source.indexOf(base));
        const targets: { from: number; to: number; target: string }[] = [];
        for (const seg of block.text) {
          const shown = textOf(lineCells([seg], plain));
          if (seg.target) targets.push({ from: offset, to: offset + shown.length, target: seg.target });
          offset += shown.length;
        }
        let cursor = 0;
        made.forEach((row, i) => {
          const shown = textOf(row), found = source.indexOf(shown, cursor);
          if (found < 0) return;
          let at = found, x = left;
          for (const cell of row) {
            const end = at + cell.ch.length;
            for (const target of targets) if (target.from < end && target.to > at && cell.w > 0) {
              const last = hits[hits.length - 1];
              if (last && last.dy === start + i && last.target === target.target && last.dx + last.w === x) last.w += cell.w;
              else hits.push({ dx: x, dy: start + i, w: cell.w, h: 1, target: target.target });
            }
            at = end; x += cell.w;
          }
          cursor = found + shown.length;
        });
        break;
      }
      case "doc": {
        const bar = block.bar ? lineCells([block.bar], plain) : [];
        made = docRows(block.doc, Math.max(1, w - 1 - widthOfCells(bar)), [], block.urls ?? true);
        if (block.most != null && made.length > block.most) made = [...made.slice(0, block.most), cellsOf("…", plain ? {} : slotStyle("text.muted"))];
        made = made.flatMap((row) => paragraphCells([...bar, ...row], w));
        break;
      }
      case "wrapped": {
        const prefix = lineCells([block.prefix], plain);
        made = wrapWords(block.text, w, cellWidth(segWords(block.prefix))).flatMap((value) => paragraphCells([...prefix, ...lineCells([{ text: value, ink: block.ink }], plain)], w));
        break;
      }
      case "meter": {
        if (!block.stages.length) break;
        const n = block.stages.length, all = Math.max(n, w);
        const bars: Cell[] = [], names: Cell[] = [];
        block.stages.forEach((stage, i) => {
          const cols = Math.round(((i + 1) * (all + 1) - n) / n) - Math.round((i * (all + 1)) / n);
          const style = plain ? {} : specStyle(stage.look);
          bars.push(...cellsOf("▀".repeat(cols), style), ...cellsOf(i < n - 1 ? " " : ""));
          names.push(...padCells(clipCells(cellsOf(`${mark[stage.mark]} ${stage.name}`, style), cols), cols, style), ...cellsOf(i < n - 1 ? " " : ""));
        });
        made = [bars, names].flatMap((row) => paragraphCells(row, w));
        break;
      }
    }
    rows.push(...made.map((row) => [...cellsOf(" ".repeat(left)), ...clipCells(row, w)]));
  };
  sections.forEach((section, i) => { if (i > 0) rows.push([]); append(section, room, pad); });
  return { rows, hits };
}
