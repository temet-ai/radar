import React from "react";
import { Box, Text as InkText } from "ink";
import type { State } from "../nodes/view.js";
import { iconOf, slotStyle, specStyle } from "./look.js";
import { CellLine, cellsOf, padCells, wrapCells } from "./cells.js";

/** Nothing to show, and why: widgets::empty — the empty icon (when the pack has one) then the words, muted, wrapped
 * between words inside the pane's one-cell padding, for every state (view_tui draws all three the same way). Plain-text
 * mode names the state. */
export function StateView({ state, text, plain, width = 80 }: { state: State; text: string; plain?: boolean; width?: number }) {
  if (plain) return <InkText>{`${state}: ${text}`}</InkText>;
  const icon = iconOf("empty");
  const lead = icon?.glyph ? cellsOf(`${icon.glyph} `, specStyle(icon.style)) : [];
  const all = [...lead, ...cellsOf(text, slotStyle("text.muted"))];
  const rows = wrapCells(all, Math.max(1, width - 2));
  return (
    <Box flexDirection="column">
      {rows.map((r, i) => (
        <CellLine key={i} cells={padCells([...cellsOf(" "), ...r], width)} />
      ))}
    </Box>
  );
}
