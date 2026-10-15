import React from "react";
import { Box, Text as InkText } from "ink";
import type { Seg, Text } from "./view.js";
import {
  SegRun,
  TextLine,
  segRunWords,
  segWords,
  textWords,
} from "../text/TextLine.js";
import { cellWidth, fit } from "../chrome/width.js";
import { BgBox } from "../text/cells.js";

/** Span::width of the line as drawn: a chip counts its padding. */
const widthOf = (t: Text) => cellWidth(t.map((s) => segRunWords(s)).join(""));

/** The details that stay on the right of the title (ui::page::title_line): while they are wider than half the room, the
 * first run goes (and a `·` that then leads), so what matters most (`branch → main`) stays. */
export function keptDetails(details: Text, room: number): Text {
  const right = [...details];
  while (widthOf(right) > Math.floor(room / 2) && right.length > 1) {
    right.shift();
    if (right.length > 0 && segWords(right[0]).trim() === "·") right.shift();
  }
  return right;
}

/** An item page's title line in `width` cells: the title on the left (cut with `…` to what the details leave), the
 * details on the right, one cell of air on each side. */
export function PageTitleLine({
  title,
  details,
  width,
  plain,
  base,
}: {
  title: Text;
  details: Text;
  width: number;
  plain?: boolean;
  /** The shell's canvas (bg.base): Ratatui paints it under the whole row, the air before the title and the title too (live cells). */
  base?: string;
}) {
  const room = Math.max(0, width - 2);
  const right = keptDetails(details, room);
  const rw = Math.min(widthOf(right), Math.floor(room / 2));
  const lw = Math.max(0, room - rw - 2);
  // view_tui::flat: the line takes the look of its first words that are not an icon (an icon leads, the title follows)
  const first: Seg = title.find((s) => !s.icon) ?? title[0] ?? {};
  const left: Seg = {
    ink: first.ink,
    strong: first.strong,
    look: first.look,
    text: fit(textWords(title), lw),
  };
  return (
    <Box width={width} overflow="hidden">
      <Box width={1} flexShrink={0} backgroundColor={base}>
        <InkText> </InkText>
      </Box>
      <Box width={lw} flexShrink={0} overflow="hidden" backgroundColor={base}>
        {/* ui::page::title_line: the title is fitted (padded) to its room in its own style */}
        <SegRun
          seg={{
            ...left,
            text: `${left.text ?? ""}${" ".repeat(Math.max(0, lw - cellWidth(left.text ?? "")))}`,
          }}
          plain={plain}
        />
      </Box>
      <Box flexGrow={1} backgroundColor={base} />
      <BgBox width={rw} flexShrink={0} overflow="hidden" backgroundColor={base}>
        <TextLine text={right} plain={plain} />
      </BgBox>
      <Box width={1} flexShrink={0} backgroundColor={base}>
        <InkText> </InkText>
      </Box>
    </Box>
  );
}
