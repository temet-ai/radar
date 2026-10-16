// The mouse selection painted over the finished frame (issue #51), as the hover is (app/hover.ts): a drag that moves
// the head is one frame Ink takes from the last one with the selected cells restyled, no React render and no pane
// drawn again. The cells take `bg.selection`, or are reversed where the tier has no colour for it (the mono tier,
// plain text). The frame under the paint is kept too: it is what a selection copies.
import { bgOf, monoTier } from "../text/look.js";
import { bgParams, frameRow, restyleRows, styleId, styleOf } from "./fast-output.js";
import { spans, type RowCells, type Selection } from "../input/selection.js";

const INVERSE = 16;

/** The selected cells of a finished frame on bg.selection (reversed in the mono tier and in plain text). */
export function paintSelection(output: string, width: number, sel: Selection, plain = false): string {
  const colour = bgOf("bg.selection");
  const reverse = plain || monoTier() || !colour;
  const lit = reverse ? "" : bgParams(colour);
  const memo = new Map<number, number>();
  return restyleRows(output, width, spans(sel), (id) => {
    let to = memo.get(id);
    if (to === undefined) {
      const s = styleOf(id);
      to = reverse ? styleId(s.fg, s.bg, s.attr | INVERSE) : styleId(s.fg, lit, s.attr);
      memo.set(id, to);
    }
    return to;
  });
}

/** The selection showing and the frame it is over. One per renderer, as the hover. */
export class Selected {
  private sel: Selection | null = null;
  private plain = false;
  private redraw: (() => void) | undefined;
  /** The last frame composed, before anything was painted over it. */
  private frame: { output: string; width: number } | null = null;

  set(sel: Selection | null, plain = this.plain) {
    if (sel === this.sel && plain === this.plain) return;
    this.sel = sel;
    this.plain = plain;
    this.redraw?.();
  }
  get selection(): Selection | null {
    return this.sel;
  }
  /** The cells of row `y` of the frame showing (blank before the first frame). */
  row(y: number): RowCells {
    const f = this.frame;
    return f ? frameRow(f.output, f.width, y) : [];
  }
  attach(redraw: () => void) {
    this.redraw = redraw;
  }
  detach(redraw: () => void) {
    if (this.redraw !== redraw) return;
    this.redraw = undefined;
    this.sel = null;
    this.frame = null;
  }
  /** Keep the frame (what a selection copies) and paint the selection over it. */
  paint = (output: string, width: number, painted: string): string => {
    this.frame = { output, width };
    const s = this.sel;
    return s ? paintSelection(painted, width, s, this.plain) : painted;
  };
}

/** The renderer's selection: fast-output paints it over every frame, over the hover (app/hover.ts). */
export const selected = new Selected();
