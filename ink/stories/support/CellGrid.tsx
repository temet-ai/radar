import React from "react";
import { cssColour } from "./colour";

/** One terminal cell: the character and its style (the model of Ratatui's `Buffer` cells and of
 * test/nodes/ansi.ts). A wide character is followed by a cell with `ch: ""`. */
export interface Cell {
  ch: string;
  fg?: string;
  bg?: string;
  bold?: boolean;
  dim?: boolean;
  italic?: boolean;
  underline?: boolean;
  inverse?: boolean;
  strike?: boolean;
}

export interface Canvas {
  /** The terminal's own foreground and background (Ratatui's canvas: text.primary on bg.base). */
  fg: string;
  bg: string;
}

/** Glyphs with a stroke from the cell's top to its bottom. A terminal draws them to fill the cell (Ghostty draws box
 * glyphs itself); a browser font leaves a gap above and below, so they are stretched to the row (terminal.css). */
const TALL = new Set([..."│┃┆┇┊┋╎╏║╭╮╯╰├┤┬┴┼┌┐└┘╔╗╚╝╠╣╦╩╬╟╢╞╡▌▐█▏▎▍▋▊▉▕"]);

/** A fixed grid of `cols` x `rows` cells, each one `1ch` wide and one line high whatever glyph it holds, so a
 * glyph from a fallback font never moves a column. Missing cells are the canvas. */
export function CellGrid({ cells, cols, rows, canvas, label }: { cells: Cell[][]; cols: number; rows: number; canvas: Canvas; label: string }) {
  const lines: React.ReactElement[] = [];
  for (let y = 0; y < rows; y++) {
    const row = cells[y] ?? [];
    const spans: React.ReactElement[] = [];
    for (let x = 0; x < cols; x++) {
      const c = row[x];
      if (c && c.ch === "") continue; // the right half of a wide character
      const wide = row[x + 1]?.ch === "" ? 2 : 1;
      let fg = cssColour(c?.fg) ?? canvas.fg;
      let bg = cssColour(c?.bg) ?? canvas.bg;
      if (c?.inverse) [fg, bg] = [bg, fg];
      const style: React.CSSProperties = {
        width: `${wide}ch`,
        color: fg,
        background: bg,
        ...(c?.bold ? { fontWeight: 700 } : {}),
        ...(c?.dim ? { opacity: 0.6 } : {}),
        ...(c?.italic ? { fontStyle: "italic" } : {}),
        ...(c?.underline || c?.strike
          ? { textDecorationLine: [c.underline ? "underline" : "", c.strike ? "line-through" : ""].join(" ").trim() }
          : {}),
      };
      spans.push(
        <span key={x} className={c && TALL.has(c.ch) ? "radar-cell radar-tall" : "radar-cell"} style={style}>
          {c && c.ch !== " " ? c.ch : " "}
        </span>,
      );
    }
    lines.push(
      <div key={y} className="radar-row">
        {spans}
      </div>,
    );
  }
  return (
    <div className="radar-term" role="img" aria-label={label} style={{ background: canvas.bg, width: `${cols}ch` }}>
      {lines}
    </div>
  );
}
