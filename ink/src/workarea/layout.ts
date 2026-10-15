import type { WorkSection } from "./model.js";

/** Below this many columns the work area is one column. */
export const NARROW_BELOW = 100;

/** Reading order when collapsed: what needs the user first, then what to do, then context. */
const COLLAPSED_ORDER = ["rail", "agent", "tabs", "timeline", "map", "other"] as const;

/** Wide arrangement: columns left to right; sections in a column stack. */
const WIDE_COLUMNS: string[][] = [["map", "timeline"], ["tabs"], ["agent"], ["rail"]];

export interface Column {
  sections: WorkSection[];
  /** Relative width; the columns share the terminal in these proportions. */
  weight: number;
}

export interface Arrangement {
  mode: "wide" | "collapsed";
  columns: Column[];
}

export function isNarrow(columns: number): boolean {
  return columns < NARROW_BELOW;
}

/**
 * Where sections go. Collapsed: one column, pending sections first (in view
 * order), then the rest by COLLAPSED_ORDER. Wide: up to three columns, empty
 * columns dropped, pending sections never hidden (they just sit in their column).
 * `weights` are the user's resized proportions, one per drawn column.
 */
export function arrange(
  sections: WorkSection[],
  columns: number,
  weights?: number[],
): Arrangement {
  if (isNarrow(columns)) {
    const rank = (s: WorkSection) => COLLAPSED_ORDER.indexOf(s.kind);
    const sorted = [...sections].sort((a, b) =>
      a.pending !== b.pending ? (a.pending ? -1 : 1) : rank(a) - rank(b),
    );
    return { mode: "collapsed", columns: [{ sections: sorted, weight: 1 }] };
  }
  const cols: Column[] = [];
  for (const kinds of WIDE_COLUMNS) {
    const inCol = sections.filter((s) => kinds.includes(s.kind));
    if (inCol.length) cols.push({ sections: inCol, weight: 1 });
  }
  cols.forEach((c, i) => {
    const w = weights?.[i];
    c.weight = w !== undefined && w > 0 ? w : i === 1 ? 2 : 1;
  });
  return { mode: "wide", columns: cols };
}

/** Move `delta` of weight from column i+1 to column i (resizing the divider between them). Each keeps at least 1. */
export function resize(weights: number[], divider: number, delta: number): number[] {
  const next = [...weights];
  const a = next[divider];
  const b = next[divider + 1];
  if (a === undefined || b === undefined) return next;
  const d = Math.max(1 - a, Math.min(b - 1, delta));
  next[divider] = a + d;
  next[divider + 1] = b - d;
  return next;
}

/** Character widths for each column of an arrangement (summing to `total`). */
export function columnWidths(weights: number[], total: number): number[] {
  const sum = weights.reduce((x, y) => x + y, 0) || 1;
  const widths = weights.map((w) => Math.max(1, Math.floor((total * w) / sum)));
  const left = total - widths.reduce((x, y) => x + y, 0);
  if (widths.length) widths[widths.length - 1]! += left;
  return widths;
}
