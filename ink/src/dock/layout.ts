/**
 * Where the agent dock sits: under the middle pane, about 40% of the
 * middle column, its top edge dragged to resize; below 100 columns it opens full height over the middle pane.
 * Pure arithmetic over cells, so it is tested without drawing. The host owns whether the dock is shown and any
 * height the user chose (`requested`); Ink only places it.
 */

/** Below this many columns the dock covers the middle pane instead of sharing its column. */
export const DOCK_FULL_BELOW = 100;
/** The dock's share of the middle column when the user has not dragged its edge. */
export const DOCK_SHARE = 0.4;
/** The least rows a docked dock keeps (tab row, one transcript row, composer, status line, frame). */
export const DOCK_MIN_ROWS = 7;
/** The least rows the pane above the dock keeps. */
export const ABOVE_MIN_ROWS = 4;

export type DockSplit = {
  /** The dock covers the whole middle pane (a narrow terminal). */
  full: boolean;
  /** Rows for the dock, frame included; 0 when it does not fit or is hidden. */
  dock: number;
  /** Rows left for the pane above it; 0 when `full`. */
  above: number;
};

/** Split the middle column's `rows` for a dock; `requested` is the height the user dragged the top edge to (rows);
 * `full` is the host's full-height toggle (alt-f): the dock takes the whole column, as on a narrow terminal. */
export function splitDock(rows: number, columns: number, visible: boolean, requested?: number | null, full?: boolean): DockSplit {
  if (!visible || rows < 3) return { full: false, dock: 0, above: rows };
  if (full || columns < DOCK_FULL_BELOW) return { full: true, dock: rows, above: 0 };
  const max = rows - ABOVE_MIN_ROWS;
  if (max < DOCK_MIN_ROWS) return { full: true, dock: rows, above: 0 };
  const want = requested != null && requested > 0 ? requested : Math.round(rows * DOCK_SHARE);
  const dock = Math.min(max, Math.max(DOCK_MIN_ROWS, want));
  return { full: false, dock, above: rows - dock };
}

/** The dock height a drag of the top edge to screen row `y` asks for, given the column's first row `top` and `rows`. */
export function dragHeight(top: number, rows: number, y: number): number {
  return Math.min(rows - ABOVE_MIN_ROWS, Math.max(DOCK_MIN_ROWS, top + rows - y));
}
