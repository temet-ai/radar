import React, { useLayoutEffect, useRef } from "react";
import { Box, Text as InkText, type DOMElement } from "ink";
import { insideOf, type SelectArea } from "../input/selection.js";
import { screenRect } from "../workarea/hit.js";
import type { EditorData } from "../nodes/view.js";
import { Framed } from "../app/Panes.js";
import { slotStyle } from "../text/look.js";
import { textProps } from "../text/TextLine.js";
import { InputField } from "./InputField.js";
import { selectionRange } from "./selection.js";

/** view_tui.rs `draw_overlay`: an editor is a panel as tall as its lines plus four (the border, a row of air, the
 * field), at least 5 and at most two thirds of the body area, and never taller than the area. */
export function editorOverlayHeight(lineCount: number, areaHeight: number): number {
  const want = lineCount + 4;
  const cap = Math.max(5, Math.floor((areaHeight * 2) / 3));
  return Math.min(Math.min(Math.max(want, 5), cap), areaHeight);
}

/** The capturing editor over the foot of the body: a focused panel titled with `title`, the last lines that fit in
 * text.primary, then the line being typed as a field with the caret. The host owns the text; this only draws it. */
export function EditorOverlay({ data, width, areaHeight, registry }: { data: EditorData; width: number; areaHeight: number; registry?: { addSelect?(area: SelectArea): void } }) {
  const height = editorOverlayHeight(data.lines.length, areaHeight);
  const inner = Math.max(0, width - 4);
  const room = Math.max(0, height - 2 - 1);
  const shown = data.lines.slice(Math.max(0, data.lines.length - room));
  const ref = useRef<DOMElement>(null);
  // Its text is selected with the mouse inside its border (view_tui.rs draw_overlay marks `r.inner(1, 1)`), over the
  // pane under it: registered after it.
  useLayoutEffect(() => {
    const at = registry?.addSelect && ref.current ? screenRect(ref.current) : undefined;
    if (at) registry!.addSelect!({ ...insideOf({ x: at.x, y: at.y, w: width, h: height }), id: "editor", kind: "text", editor: true });
  });
  return (
    <Box ref={ref} width={width} height={height} flexShrink={0}>
    <Framed title={data.title} width={width} height={height} focused>
      {/* widgets::padded: one column of air each side inside the border (Framed itself no longer pads) */}
      <Box flexDirection="column" paddingX={1} width={Math.max(0, width - 2)}>
      {shown.map((l, i) => (
        <InkText key={i} wrap="truncate-end" {...textProps(slotStyle("text.primary"))}>
          {l || " "}
        </InkText>
      ))}
      <Box>
        <InputField text={data.line} caret={data.caret} width={inner} selection={selectionRange(data.selection)} />
      </Box>
      </Box>
    </Framed>
    </Box>
  );
}
