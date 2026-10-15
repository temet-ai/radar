// The work area, as the view carries it.
//
// The Work screen (a `ScreenView`) holds panes. A pane whose body is an
// `Extension` node with capability `work.<kind>` (kind: map, timeline, tabs,
// agent, rail) is one section of the work area. The layout draws exactly the
// sections the view carries: a section the view lacks is absent, never an
// empty pane. Panes that are not work sections (the Nav/Middle/Detail panes
// of today's Work screen) are kept, in order, as "other" sections so nothing
// the view shows is lost.
//
// Extension data (all optional): `{ pending: boolean, status: string, target: string }`.
// `pending` marks a section that holds a question or approval waiting for the user.

export type SectionKind = "map" | "timeline" | "tabs" | "agent" | "rail" | "other";

/** The loose shape of a view Node; T11's components own the exact one. */
export interface ViewNode {
  node: string;
  [key: string]: unknown;
}

export interface PaneLike {
  pane?: string;
  title?: string;
  body: ViewNode;
  /** The pane's own line above and below its body, and its notes (view::PaneView). */
  head?: unknown[] | null;
  footer?: unknown[] | null;
  notes?: unknown[][] | null;
  [key: string]: unknown;
}

export interface ScreenLike {
  scope?: string;
  panes?: PaneLike[];
  [key: string]: unknown;
}

export interface WorkSection {
  kind: SectionKind;
  /** Stable key: the pane id, or kind and position. */
  key: string;
  title: string;
  body: ViewNode;
  /** Lines of view text the pane carries around its body: head first, then notes, then footer ("Groups · page 1"). */
  head: unknown[];
  notes: unknown[][];
  footer: unknown[];
  /** A question or approval is waiting here. */
  pending: boolean;
  /** One word for the section's state ("running", "stale", ...). */
  status?: string;
  /** The target focus is kept by across snapshots. */
  target: string;
}

const KINDS: SectionKind[] = ["map", "timeline", "tabs", "agent", "rail"];

function kindOf(body: ViewNode): SectionKind {
  if (body.node === "extension" && typeof body.capability === "string") {
    const m = /^work\.(.+)$/.exec(body.capability);
    if (m && (KINDS as string[]).includes(m[1]!)) return m[1] as SectionKind;
  }
  return "other";
}

function dataOf(body: ViewNode): Record<string, unknown> {
  const d = body.data;
  return d && typeof d === "object" ? (d as Record<string, unknown>) : {};
}

/** The sections a screen carries, in the order the view gives them. */
export function sectionsOf(screen: ScreenLike | null | undefined): WorkSection[] {
  const out: WorkSection[] = [];
  const panes = screen?.panes ?? [];
  const seen = new Map<string, number>();
  panes.forEach((p, i) => {
    const kind = kindOf(p.body);
    const data = kind === "other" ? {} : dataOf(p.body);
    // Several panes can share a PaneId (the real Work view puts every work.*
    // section in `detail`), so the key is unique by position and the default
    // target by first-come order within the pane id.
    const paneId = p.pane ?? kind;
    const n = seen.get(paneId) ?? 0;
    seen.set(paneId, n + 1);
    out.push({
      kind,
      key: `${paneId}:${i}`,
      title: p.title ?? kind,
      body: p.body,
      head: p.head ?? [],
      notes: p.notes ?? [],
      footer: p.footer ?? [],
      pending: data.pending === true,
      status: typeof data.status === "string" ? data.status : undefined,
      target: typeof data.target === "string" ? data.target : n === 0 ? `pane:${paneId}` : `pane:${paneId}#${n}`,
    });
  });
  return out;
}
