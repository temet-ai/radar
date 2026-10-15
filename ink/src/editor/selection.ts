import type { Intent } from "../protocol/generated/Intent.js";

// Text selection in the capturing editor. The host owns the text, the caret and the selection (view::Editor.selection
// = [anchor, extent], counted in code points as `caret` is, the caret at the extent); the renderer only turns a
// shift-movement key into Intent::EditSelect and draws the range. Plain keys still go to the host as chords.

/** What `Editor` carries for selection. */
export interface SelectionState {
  line: string;
  caret: number;
  selection?: [number, number] | null;
}

/** The range [start, end) to paint, ascending, or null when nothing (or an empty range) is selected. */
export function selectionRange(sel: [number, number] | null | undefined): [number, number] | null {
  if (!sel) return null;
  const [a, e] = sel;
  return a === e ? null : [Math.min(a, e), Math.max(a, e)];
}

export type SelectMove = "left" | "right" | "home" | "end";

/**
 * The Intent for a shift+movement key while an editor captures: the selection grows or shrinks from its anchor (the
 * caret where the selection began) to where the caret moves, one code point for left and right, the line's ends
 * for home and end. The host moves the caret to the extent; an anchor equal to the extent is no selection.
 */
export function selectIntent(editor: SelectionState, move: SelectMove): Intent {
  const len = Array.from(editor.line).length;
  const anchor = editor.selection ? editor.selection[0] : editor.caret;
  const at = editor.selection ? editor.selection[1] : editor.caret;
  const extent = move === "left" ? Math.max(0, at - 1) : move === "right" ? Math.min(len, at + 1) : move === "home" ? 0 : len;
  return { intent: "edit_select", anchor: Math.min(anchor, len), extent };
}
