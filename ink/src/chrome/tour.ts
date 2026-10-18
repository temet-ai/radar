// The tour's card (crates/radar-core/src/shell/tour.rs): a panel titled `tour · N of M` at the body's lower right, a cell
// in from the right and from the bottom, over the app, with a shadow. Its lines: the title in bold emphasis, a blank
// line, the stop's lines, a blank line, `try_it` in muted when there is one, then the keys.
import { cellWidth, fit } from "./width.js";
import { textLineString } from "../text/TextLine.js";
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

/** Keep the exact shortcut suffix readable even when a plain tour caption is clipped. */
export function plainTourText(text: TourCard["keys"], width: number): string {
  const full = textLineString(text, true);
  if (cellWidth(full) <= width) return full;
  // Keep each key beside its own phrase. A clipped phrase reserves its own suffix;
  // later phrases are omitted instead of collecting their unrelated keys at the end.
  const units: Array<{ words: string; key?: string }> = [];
  let words = "";
  for (const seg of text) {
    words += textLineString([{ ...seg, key_hint: undefined }], true);
    if (seg.key_hint) { units.push({ words, key: seg.key_hint }); words = ""; }
  }
  if (words) units.push({ words });
  let shown = "";
  for (const unit of units) {
    const suffix = unit.key ? ` [${unit.key}]` : "";
    const room = Math.max(0, width - cellWidth(shown));
    if (cellWidth(unit.words + suffix) <= room) { shown += unit.words + suffix; continue; }
    const caption = fit(unit.words, Math.max(0, room - cellWidth(suffix)));
    if (!suffix || (room > cellWidth(suffix) && /[\p{Letter}\p{Number}]/u.test(caption))) shown += caption + suffix;
    else if (room > 0) shown += "…";
    break;
  }
  return shown;
}

const widthOf = (l: TourLine, plain: boolean): number => cellWidth(l.words ?? (l.text ? textLineString(l.text, plain) : ""));

/** tour.rs place: the card's lines' width plus 4 (at least 40, at most the body less 2) and height plus 2 (at most the
 * body less 1), at the lower right of a body `bodyW` by `bodyH` (0-based, from the body's top-left). Null when it would
 * be under 10 cells wide or 4 tall (tour.rs card draws nothing then). */
export function layoutTour(card: TourCard, bodyW: number, bodyH: number, plain = false): TourLaid | null {
  const lines = tourLines(card);
  const widest = Math.max(0, ...lines.map((l) => widthOf(l, plain)));
  const w = Math.min(Math.max(widest + 4, 40), Math.max(0, bodyW - 2));
  const h = Math.min(lines.length + 2, Math.max(0, bodyH - 1));
  if (w < 10 || h < 4) return null;
  return { x: Math.max(0, bodyW - (w + 1)), y: Math.max(0, bodyH - (h + 1)), w, h, title: `tour · ${card.stop} of ${card.of}`, lines };
}
