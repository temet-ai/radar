import React from "react";
import { Box, Text as InkText } from "ink";
import { bgOf, slotStyle } from "../text/look.js";
import { textProps } from "../text/TextLine.js";
import { textWidth } from "./graphemes.js";
import { BgBox } from "../text/cells.js";

const cellsOf = (s: string) => Array.from(s).reduce((n, c) => n + textWidth(c), 0);

/** The characters of `text` that fit in `cells` cells (a wide character that would cross the edge is left out). */
export function cutCells(text: string, cells: number): string {
  let n = 0;
  let out = "";
  for (const c of Array.from(text)) {
    const w = textWidth(c);
    if (n + w > cells) break;
    n += w;
    out += c;
  }
  return out;
}

/** What the host's `input::render_field` puts in a row `width` cells wide: the characters from the scroll offset
 * (one cell is kept for the caret at the end of the text), and where the caret is in them. */
export function fieldWindow(text: string, caret: number, width: number): { before: string; at: string; after: string } {
  const chars = Array.from(text);
  const col = Math.min(caret, chars.length);
  const offset = Math.max(0, col + 1 - width);
  const visible = chars.slice(offset, offset + width);
  const x = col - offset;
  return { before: visible.slice(0, x).join(""), at: visible[x] ?? " ", after: visible.slice(x + 1).join("") };
}

/** One line being typed, drawn as `input::render_field` does it: the row on bg.overlay, the words in bold
 * text.emphasis, the caret a text.emphasis cell REVERSED over the character it is before (a space after the end).
 * `caret` is a code-point index, how the view counts; null draws no caret (an unfocused field). `width` is the row's
 * cells: given, the text scrolls to keep the caret in view and the row is painted to its end. */
export function InputField({
  text,
  caret,
  mark,
  width,
  selection,
}: {
  text: string;
  caret?: number | null;
  mark?: string;
  width?: number;
  /** [start, end) in code points of `text`, drawn on bg.selection (the host's Editor.selection). */
  selection?: [number, number] | null;
}) {
  const emphasis = { ...slotStyle("text.emphasis"), bold: true, backgroundColor: bgOf("bg.overlay") };
  const caretStyle = { ...emphasis, inverse: true };
  const room = width === undefined ? undefined : Math.max(0, width - Array.from(mark ?? "").length);
  const w0 = caret === null || caret === undefined ? null : fieldWindow(text, caret, room ?? Infinity);
  // set_stringn cuts at cells, not characters: a wide character takes two, so the row is cut (and padded) in cells.
  const cutWindow = (v: NonNullable<typeof w0>) => {
    const before = cutCells(v.before, room!);
    const at = cutCells(v.at, room! - cellsOf(before));
    return { before, at, after: cutCells(v.after, room! - cellsOf(before + at)) };
  };
  const w = w0 === null || room === undefined ? w0 : cutWindow(w0);
  const shown = room === undefined ? Array.from(text) : Array.from(cutCells(text, room));
  // Where the visible window starts in `text` (code points), to place the selection in it.
  const offset = w === null ? 0 : Math.min(caret ?? 0, Array.from(text).length) - Array.from(w.before).length;
  const lo = selection ? selection[0] - offset : 0;
  const hi = selection ? selection[1] - offset : 0;
  const selected = { ...emphasis, backgroundColor: bgOf("bg.selection") };
  const body =
    w !== null && selection && hi > lo ? (
      <>
        {Array.from(w.before + w.at + w.after).map((ch, i) => {
          const caretHere = i === Array.from(w.before).length;
          const style = caretHere ? caretStyle : i >= lo && i < hi ? selected : emphasis;
          return (
            <InkText key={i} {...textProps(style)}>
              {ch}
            </InkText>
          );
        })}
      </>
    ) : w === null ? (
      <InkText {...textProps(emphasis)}>{shown.join("")}</InkText>
    ) : (
      <>
        <InkText {...textProps(emphasis)}>{w.before}</InkText>
        <InkText {...textProps(caretStyle)}>{w.at}</InkText>
        <InkText {...textProps(emphasis)}>{w.after}</InkText>
      </>
    );
  const used = w === null ? cellsOf(shown.join("")) : cellsOf(w.before + w.at + w.after);
  const pad = room === undefined ? 0 : Math.max(0, room - used);
  const row = (
    <InkText wrap="truncate-end">
      {mark ? <InkText {...textProps({ ...slotStyle("text.muted"), backgroundColor: bgOf("bg.overlay") })}>{mark}</InkText> : null}
      {body}
      {pad > 0 ? <InkText {...textProps(emphasis)}>{" ".repeat(pad)}</InkText> : null}
    </InkText>
  );
  return width === undefined ? (
    <BgBox flexGrow={1} backgroundColor={bgOf("bg.overlay")}>
      {row}
    </BgBox>
  ) : (
    <Box width={width} flexShrink={0}>
      {row}
    </Box>
  );
}
