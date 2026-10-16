// The tour's card (crates/radar-core/src/shell/tour.rs): a panel titled `tour · N of M` at the body's lower right, a cell
// in from the right and from the bottom, over the app, with a shadow. Its lines: the title in bold emphasis, a blank
// line, the stop's lines, a blank line, `try_it` in muted when there is one, then the keys.
import { cellWidth } from "./width.js";
import { textWords } from "../text/TextLine.js";
import type { TourCard } from "../protocol/generated/TourCard.js";

export type TourLine = { kind: "title" | "blank" | "text" | "try" | "keys"; text?: TourCard["keys"]; words?: string };
export type TourLaid = { x: number; y: number; w: number; h: number; title: string; lines: TourLine[] };

/** tour.rs lines_of: each line of the card in its order. */
export function tourLines(card: TourCard): TourLine[] {
  const out: TourLine[] = [{ kind: "title", words: card.title }, { kind: "blank" }];
  for (const t of card.lines ?? []) out.push({ kind: "text", text: t });
  out.push({ kind: "blank" });
  if (card.try_it) out.push({ kind: "try", words: card.try_it });
  out.push({ kind: "keys", text: card.keys });
  return out;
}

const widthOf = (l: TourLine): number => cellWidth(l.words ?? (l.text ? textWords(l.text) : ""));

/** tour.rs place: the card's lines' width plus 4 (at least 40, at most the body less 2) and height plus 2 (at most the
 * body less 1), at the lower right of a body `bodyW` by `bodyH` (0-based, from the body's top-left). Null when it would
 * be under 10 cells wide or 4 tall (tour.rs card draws nothing then). */
export function layoutTour(card: TourCard, bodyW: number, bodyH: number): TourLaid | null {
  const lines = tourLines(card);
  const widest = Math.max(0, ...lines.map(widthOf));
  const w = Math.min(Math.max(widest + 4, 40), Math.max(0, bodyW - 2));
  const h = Math.min(lines.length + 2, Math.max(0, bodyH - 1));
  if (w < 10 || h < 4) return null;
  return { x: Math.max(0, bodyW - (w + 1)), y: Math.max(0, bodyH - (h + 1)), w, h, title: `tour · ${card.stop} of ${card.of}`, lines };
}
