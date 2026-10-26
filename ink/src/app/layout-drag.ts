// Pane borders and columns dragged with the mouse (#25). A press on a border, a heading or the cell between two
// headings holds it; while the pointer moves Ink draws what it would look like (a preview laid over the host's view),
// and on release it sends one intent the host keeps in `[layout]`. A double click on a border or an edge puts the
// default back. The host owns the result: the preview goes as soon as the next snapshot carries it.
import type { Hit } from "../chrome/types.js";
import type { MouseEvent } from "../input/mouse.js";
import type { Shape } from "./pointer.js";

export type Edge = "nav" | "detail";

/** What a drag shows until the host has it. */
export type Preview =
  | { kind: "split"; edge: Edge; share: number }
  | { kind: "width"; key: string; heading: string; width: number }
  | { kind: "move"; key: string; heading: string; order: string[]; float: number }
  | { kind: "names"; key: string; width: number };

/** layout::key: where a pane's columns are kept. `<screen>/<pane>`, on an item's page `<screen>/page/<tab>/<pane>`. */
export function layoutKey(screen: any, pane: string): string {
  const page = screen?.page;
  if (!page) return `${screen?.scope}/${pane}`;
  const tab: string = page.tabs?.[page.tab]?.label ?? "";
  return tab ? `${screen.scope}/page/${tab.toLowerCase()}/${pane}` : `${screen.scope}/page/${pane}`;
}

export const MIN_COLUMN = 3;
export const MAX_COLUMN = 120;
/** layout::MIN_NAMES. */
export const MIN_NAMES = 8;
const DOUBLE_MS = 400;
const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));
/** A share of the width in thousandths, as layout::LayoutIntent::Split takes it. */
export const clampShare = (s: number) => clamp(Math.round(s), 50, 800);

/** The columns of a pane's body as the preview has them. */
function previewBody(body: any, p: Preview): any {
  if (!body || (body.node !== "table" && body.node !== "tree") || p.kind === "split") return body;
  if (p.kind === "names") return body.node === "tree" ? { ...body, names: p.width } : body;
  const columns: any[] = body.columns ?? [];
  if (p.kind === "width")
    return { ...body, columns: columns.map((c) => (c.heading === p.heading ? { ...c, width: p.width, lit: true } : c)) };
  const shown = p.order.map((h) => columns.find((c) => c.heading === h)).filter(Boolean);
  const rest = columns.filter((c) => !p.order.includes(c.heading));
  const moved = [...shown, ...rest].map((c) => (c.heading === p.heading ? { ...c, lit: true, float: p.float } : c));
  return { ...body, columns: moved, ...(body.node === "tree" ? { ordered_columns: true } : {}) };
}

/** A screen with a drag's preview laid over it. */
export function previewScreen(screen: any, p: Preview | null): any {
  if (!screen || !p) return screen;
  if (p.kind === "split") return { ...screen, layout: { ...(screen.layout ?? {}), [p.edge]: p.share } };
  const panes = (list: any[] | undefined) =>
    list?.map((pane) => (layoutKey(screen, pane.pane) === p.key ? { ...pane, body: previewBody(pane.body, p) } : pane));
  return screen.page
    ? { ...screen, page: { ...screen.page, panes: panes(screen.page.panes) } }
    : { ...screen, panes: panes(screen.panes) };
}

/** A snapshot with a drag's preview laid over its screen, and the dock at `dockRows` while its edge is dragged. */
export function previewSnapshot(snap: any, p: Preview | null, dockRows: number | null): any {
  if (!snap || (!p && dockRows === null)) return snap;
  const dock = (d: any) => (d && dockRows !== null ? { ...d, rows: dockRows, full: false } : d);
  const screen = snap.screen ? previewScreen(snap.screen, p) : snap.screen;
  return { ...snap, screen: screen && screen.dock ? { ...screen, dock: dock(screen.dock) } : screen, dock: dock(snap.dock) };
}

type Slot = { heading: string; x: number; width: number };
type Held =
  | { kind: "split"; target: string; edge: Edge; x0: number; width: number; share?: number }
  | { kind: "width"; target: string; key: string; heading: string; sign: number; x: number; start: number; most: number; width?: number }
  | { kind: "move"; key: string; heading: string; slots: Slot[]; grab: number; x: number; order?: string[] }
  | { kind: "names"; target: string; key: string; x: number; start: number; most: number; width?: number };

export interface DragHost {
  send(intent: any): void;
  show(p: Preview | null): void;
  /** The screen showing, for the key of the pane under the pointer. */
  screen(): any;
  paneAt(x: number, y: number): string | undefined;
  hits(): Hit[];
  now?(): number;
}

export class LayoutDrag {
  private held: Held | null = null;
  private last: { target: string; at: number } | null = null;
  /** What was last shown: a report that changes nothing on screen draws nothing. */
  private shown = "";

  constructor(private readonly host: DragHost) {}

  get holding(): boolean {
    return this.held !== null;
  }

  /** The pointer's shape while something is held. */
  shape(): Shape | null {
    if (!this.held) return null;
    return this.held.kind === "move" ? "grabbing" : "col-resize";
  }

  /** A pointer report: true when a drag took it. */
  mouse(ev: MouseEvent, hit: Hit | undefined): boolean {
    if (this.held) {
      if (ev.kind === "motion") this.moveTo(ev.x);
      else if (ev.kind === "release") this.drop();
      return true;
    }
    if (ev.kind !== "press" || ev.button !== "left" || !hit?.target.startsWith("layout:")) return false;
    const now = this.host.now?.() ?? Date.now();
    const [, kind, ...rest] = hit.target.split(":");
    const again = this.last && this.last.target === hit.target && now - this.last.at < DOUBLE_MS;
    this.last = null;
    if (kind === "split") {
      const edge = rest[0] as Edge;
      if (again) return this.send({ layout: "split", edge, share: null });
      this.held = { kind, target: hit.target, edge, x0: Number(rest[1]), width: Math.max(1, Number(rest[2])) };
      return true;
    }
    const screen = this.host.screen();
    const pane = this.host.paneAt(ev.x, ev.y);
    if (!screen || !pane) return false;
    const key = layoutKey(screen, pane);
    const heading = decodeURIComponent(rest[0] ?? "");
    const slots = this.host
      .hits()
      .filter((h) => h.y === hit.y && h.target.startsWith("layout:head:") && this.host.paneAt(h.x, h.y) === pane)
      .sort((a, b) => a.x - b.x)
      .map((h) => ({ heading: decodeURIComponent(h.target.slice("layout:head:".length)), x: h.x, width: h.w + 1 }));
    if (kind === "names") {
      // A tree's names: their edge follows the pointer; a double click lets them take what the columns leave again.
      if (again) return this.send({ layout: "names_width", key, width: null });
      const start = Number(rest[0]);
      this.held = { kind: "names", target: hit.target, key, x: ev.x, start, most: Math.max(start, Number(rest[1]) || start) };
      return true;
    }
    if (kind === "edge") {
      const slot = slots.find((s) => s.heading === heading);
      if (!slot) return false;
      const most = Number(rest[2]);
      if (again) {
        // A double click fits the column to its widest value, never past the room the others leave; on a column that
        // fits already it puts the part's width back.
        const fit = clamp(Number(rest[3]) || slot.width, MIN_COLUMN, Math.min(MAX_COLUMN, Number.isFinite(most) && most > 0 ? Math.max(most, slot.width) : MAX_COLUMN));
        return this.send({ layout: "column_width", key, heading, width: fit === slot.width ? null : fit });
      }
      this.held = { kind: "width", target: hit.target, key, heading, sign: Number(rest[1]) < 0 ? -1 : 1, x: ev.x, start: slot.width, most: Number.isFinite(most) && most > 0 ? Math.max(most, slot.width) : MAX_COLUMN };
      return true;
    }
    if (kind === "head") {
      const slot = slots.find((s) => s.heading === heading);
      if (!slot) return false;
      this.held = { kind: "move", key, heading, slots, grab: ev.x - slot.x, x: ev.x };
      return true;
    }
    return false;
  }

  private show(p: Preview) {
    const key = JSON.stringify(p);
    if (key === this.shown) return;
    this.shown = key;
    this.host.show(p);
  }

  private send(layout: any): boolean {
    this.host.send({ intent: "layout", ...layout });
    return true;
  }

  private moveTo(x: number) {
    const h = this.held!;
    if (h.kind === "split") {
      h.share = clampShare(((h.edge === "nav" ? x - h.x0 + 1 : h.x0 + h.width - x) * 1000) / h.width);
      this.show({ kind: "split", edge: h.edge, share: h.share });
    } else if (h.kind === "names") {
      h.width = clamp(h.start + (x - h.x), MIN_NAMES, h.most);
      this.show({ kind: "names", key: h.key, width: h.width });
    } else if (h.kind === "width") {
      // never wider than the room the names (or a table's title) can give: no other column is pushed off the line
      h.width = clamp(h.start + h.sign * (x - h.x), MIN_COLUMN, Math.min(MAX_COLUMN, h.most));
      this.show({ kind: "width", key: h.key, heading: h.heading, width: h.width });
    } else {
      if (h.order === undefined && x === h.x) return;
      const others = h.slots.filter((s) => s.heading !== h.heading);
      const float = x - h.grab;
      // the gap goes where the floating heading's left edge is nearest a column boundary
      let gx = h.slots[0]!.x;
      let best = 0;
      let bestX = gx;
      for (let k = 0; k <= others.length; k++) {
        if (Math.abs(float - gx) < Math.abs(float - bestX)) {
          best = k;
          bestX = gx;
        }
        if (k < others.length) gx += others[k]!.width;
      }
      h.order = [...others.slice(0, best).map((s) => s.heading), h.heading, ...others.slice(best).map((s) => s.heading)];
      this.show({ kind: "move", key: h.key, heading: h.heading, order: h.order, float: float - bestX });
    }
  }

  private drop() {
    const h = this.held!;
    this.held = null;
    this.shown = "";
    const now = this.host.now?.() ?? Date.now();
    let sent = false;
    if (h.kind === "split") {
      if (h.share !== undefined) sent = this.send({ layout: "split", edge: h.edge, share: h.share });
      else this.last = { target: h.target, at: now };
    } else if (h.kind === "names") {
      if (h.width !== undefined && h.width !== h.start) sent = this.send({ layout: "names_width", key: h.key, width: h.width });
      else if (h.width === undefined) this.last = { target: h.target, at: now };
    } else if (h.kind === "width") {
      if (h.width !== undefined && h.width !== h.start) sent = this.send({ layout: "column_width", key: h.key, heading: h.heading, width: h.width });
      else if (h.width === undefined) this.last = { target: h.target, at: now };
    } else if (h.order && h.order.join("\u001f") !== h.slots.map((s) => s.heading).join("\u001f")) {
      sent = this.send({ layout: "column_order", key: h.key, order: h.order });
    }
    // What was sent stays drawn until the snapshot that carries it replaces the preview (App drops it then).
    if (!sent) this.host.show(null);
  }
}
