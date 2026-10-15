import React from "react";
import { Box, Text as InkText } from "ink";
import type { EditorData } from "../nodes/view.js";
import { InputField } from "./InputField.js";
import { selectionRange } from "./selection.js";
import { slotStyle } from "../text/look.js";
import { textProps } from "../text/TextLine.js";
import { chordOfKey, type KeyInput, type Chord } from "./keys.js";

export { chordOfKey };
export type { KeyInput, Chord };

/** The `editor` capability: the lines written so far and the one being typed, with the caret. The host owns the
 * text (the part holds it and the write is confirmed there), so this only draws it; keystrokes are sent as
 * `{intent:"key", chord}` with `chordOfKey`, the same chords the Ratatui adapter sends while an editor captures.
 * Multi-line caret is `caret` characters into `line`, as view::Editor counts it. */
export function EditorView({ data, focused }: { data: EditorData; focused: boolean }) {
  return (
    <Box flexDirection="column">
      <InkText bold>{data.title}</InkText>
      {data.lines.map((l, i) => (
        <InkText key={i} wrap="truncate-end" {...textProps(slotStyle("text.primary"))}>
          {l || " "}
        </InkText>
      ))}
      <InputField text={data.line} caret={focused ? data.caret : null} selection={selectionRange(data.selection)} />
    </Box>
  );
}

export function isEditorData(data: unknown): data is EditorData {
  const d = data as Partial<EditorData> | null;
  return !!d && typeof d.title === "string" && Array.isArray(d.lines) && typeof d.line === "string" && typeof d.caret === "number";
}
