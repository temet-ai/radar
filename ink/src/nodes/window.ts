import { createContext, useContext } from "react";

/** The rows of a node that are laid out: `count` rows from `first`. A node taller than its room draws only these
 * (with a margin), so a snapshot of a 200-row list costs a screenful, as the host's list and tree draw only what shows. */
export interface Window {
  first: number;
  count: number;
}
export const WindowContext = createContext<Window | null>(null);
export const useWindow = () => useContext(WindowContext);

/** Rows drawn beyond the room, each side. */
export const MARGIN = 0;

/** The scroll rule (ui::rows::list): keep the first row as it was, moving it only to bring the cursor into view,
 * and never past the last full screen. `first` is the row shown first last time. */
export function firstRow(
  total: number,
  cursor: number,
  room: number,
  first: number,
): number {
  if (room <= 0 || total <= room) return 0;
  let f = first;
  if (cursor < f) f = cursor;
  if (cursor >= f + room) f = cursor + 1 - room;
  return Math.max(0, Math.min(f, total - room));
}

export function windowOf<T>(
  rows: T[],
  w: Window | null,
): { shown: T[]; first: number } {
  if (!w) return { shown: rows, first: 0 };
  return { shown: rows.slice(w.first, w.first + w.count), first: w.first };
}
