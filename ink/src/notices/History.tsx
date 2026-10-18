import React from "react";
import { Box } from "ink";
import { Rows } from "../text/cells.js";
import type { NoticeHistoryLayout } from "./layout.js";
/** Explicit notice history surface, not the unsupported generic overlay. */
export function NoticeHistoryOverlay({ layout }: { layout: NoticeHistoryLayout }) {
  const rows = new Rows();
  for (const line of layout.lines) { rows.line(); for (const cell of line) rows.cell(cell); }
  return <Box flexDirection="column" flexShrink={0}>{rows.element("truncate-end")}</Box>;
}
