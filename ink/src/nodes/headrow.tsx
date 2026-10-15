import React from "react";
import { Box, Text as InkText } from "ink";
import type { Placed } from "./columns.js";
import { ICONS } from "../text/icons.js";

/** The heading of each column in capitals (with its icon) over its column, bold and muted (ui::Columns::header). */
export function ColumnHeadings({
  columns,
  plain,
}: {
  columns: Placed[];
  plain?: boolean;
}) {
  return (
    <>
      {columns.map(({ column, width }, i) => (
        <Box key={`${column.heading}${i}`} width={width} flexShrink={0}>
          <Box width={Math.max(1, width - 1)} overflow="hidden">
            <InkText bold dimColor={!plain} wrap="truncate-end">
              {(column.icon && ICONS[column.icon]
                ? `${ICONS[column.icon]} `
                : "") + column.heading.toUpperCase()}
            </InkText>
          </Box>
        </Box>
      ))}
    </>
  );
}
