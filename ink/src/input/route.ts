import type { Chord, Intent } from "../chrome/types.js";
export type { Intent };
import { visibleRows } from "../nodes/tree.js";
import type { Tree } from "../nodes/view.js";

/**
 * What a key comes to on a view, mirroring crates/radar-core/src/view_tui.rs (`key`, `section_key`)
 * and view.rs (`rows_step_by`, `tree_step`): movement along a list, table, entries or tree selects
 * the next row through the part, ⏎ activates the selection, any other key is passed on as
 * Intent::Key. test/input/ratatui-intents.json (written by a Rust test over the Ratatui adapter)
 * pins the pairs.
 */

const plain = (c: Chord) => !c.ctrl && !c.alt;
const ch = (c: Chord): string | undefined => (typeof c.key === "object" ? c.key.char : undefined);
const keyIs = (c: Chord, name: string) => c.key === name;

/** view.rs `rows_step_by`: the row a movement key goes to, or none when it is not the list's key. */
export function rowsStepBy(n: number, at: number, chord: Chord, heading: (i: number) => boolean): number | undefined {
  const can = (i: number) => i < n && !heading(i);
  let first = -1;
  for (let i = 0; i < n; i++) if (can(i)) { first = i; break; }
  let last = -1;
  for (let i = n - 1; i >= 0; i--) if (can(i)) { last = i; break; }
  if (first < 0 || last < 0) return undefined;
  if (!plain(chord)) return undefined;
  const c = ch(chord);
  if (keyIs(chord, "down") || c === "j") {
    for (let i = at + 1; i < n; i++) if (can(i)) return i;
    return Math.min(Math.max(at, first), last);
  }
  if (keyIs(chord, "up") || c === "k") {
    if (at <= first) return undefined;
    for (let i = at - 1; i >= 0; i--) if (can(i)) return i;
    return first;
  }
  if (keyIs(chord, "home") || c === "g") return first;
  if (keyIs(chord, "end") || c === "G") return last;
  return undefined;
}

const targetOf = (path: string[]) => path.join("\u001f");

/** The tree row target as the host names it: `TreeRow::target_of` joins the path with a unit separator. */
export const treeTarget = targetOf;

/** view.rs `tree_step`. */
export function treeStep(tree: Tree, chord: Chord): Intent | undefined {
  const shown = visibleRows(tree);
  if (shown.length === 0 || !plain(chord)) return undefined;
  const sel = tree.selected ?? [];
  const eq = (a: string[], b: string[]) => a.length === b.length && a.every((s, i) => s === b[i]);
  let at = shown.findIndex((s) => eq(s.path, sel));
  if (at < 0) at = 0;
  const open = tree.open ?? [];
  const isOpen = (p: string[]) => open.some((o) => eq(o, p));
  const hasChildren = (i: number) => (shown[i]?.row.children?.length ?? 0) > 0;
  const c = ch(chord);
  if ((c === "+" || c === "=") && hasChildren(at) && !isOpen(shown[at]!.path)) {
    return { intent: "fold", target: treeTarget(shown[at]!.path), open: true };
  }
  if (c === "-") {
    const path = shown[at]!.path;
    let group: string[];
    if (hasChildren(at) && isOpen(path)) group = path;
    else if (path.length > 1) group = path.slice(0, -1);
    else return undefined;
    return { intent: "fold", target: treeTarget(group), open: false };
  }
  if (c === " " && hasChildren(at)) {
    return { intent: "fold", target: treeTarget(shown[at]!.path), open: !isOpen(shown[at]!.path) };
  }
  const to = rowsStepBy(shown.length, at, chord, (i) => !!shown[i]!.row.heading);
  return to === undefined ? undefined : { intent: "select", target: treeTarget(shown[to]!.path) };
}

type PaneLike = { pane?: string; body?: any; fields?: Array<{ caret?: unknown }>; bottom_field?: { caret?: unknown } | null };
type ScreenLike = { panes?: PaneLike[]; overlay?: unknown; page?: unknown };

/**
 * The snapshot keys that arrive together are routed from: the one the previous key left (`pending.working`), until a
 * newer host snapshot (`base`) replaces it. Every router (route, scroll) starts from here; one that read the old
 * working copy after a page opened routed the first key on the screen from before it.
 */
export function workingFrom<T>(pending: { base: T; working: T } | undefined, base: T | undefined): { base: T; working: T } | undefined {
  if (pending && pending.base === base) return pending;
  return base ? { base, working: base } : undefined;
}

/** Whether a field in `pane` is being typed into: it has the caret. */
function typing(pane: PaneLike): boolean {
  return (pane.fields ?? []).some((f) => f.caret != null) || pane.bottom_field?.caret != null;
}

/** view_tui::panes: an item page's panes while one is open over the screen, else the screen's. */
function panesOf(screen: ScreenLike): PaneLike[] {
  const page = screen.page as { panes?: PaneLike[] } | null | undefined;
  return page ? (page.panes ?? []) : (screen.panes ?? []);
}

/** view_tui::key for a screen: which Intent a key is, when the frontend does the movement itself. */
export function screenKey(screen: ScreenLike, focus: string | undefined, chord: Chord, pageStep = 10): Intent {
  const key: Intent = { intent: "key", chord };
  if (screen.overlay) return key;
  // A field being typed into takes every key, whichever pane has the focus: `/` typed from the views rail filters
  // the tree, and ↓ is the field's (view_tui::key).
  if (panesOf(screen).some(typing)) return key;
  const pane = panesOf(screen).find((p) => p.pane === focus);
  if (!pane) return key;
  const body = pane.body;
  const rows = rowsOf(body);
  const page = (keyIs(chord, "page_up") || keyIs(chord, "page_down")) && !chord.ctrl;
  if (rows) {
    const { targets, selected, headings } = rows;
    if (selected == null && targets.length > 0) return key;
    const at = selected ?? 0;
    if (page && screen.page && targets.length > 0) {
      const to = keyIs(chord, "page_down") ? Math.min(at + pageStep, targets.length - 1) : Math.max(0, at - pageStep);
      return to === at ? key : { intent: "select", target: targets[to]! };
    }
    const to = rowsStepBy(targets.length, at, chord, (i) => !!headings[i]);
    if (to !== undefined) return { intent: "select", target: targets[to]! };
    if (keyIs(chord, "enter") && !chord.ctrl && !chord.alt && targets[at] !== undefined) {
      return { intent: "activate", target: targets[at]! };
    }
    return key;
  }
  if (body?.node === "tree") {
    const step = treeStep(body as Tree, chord);
    if (step) return step;
    if (keyIs(chord, "enter") && !chord.ctrl && !chord.alt && (body.selected?.length ?? 0) > 0) {
      return { intent: "activate", target: treeTarget(body.selected) };
    }
  }
  return key;
}

/** The part of nodes/index.tsx's Scroller the router uses (ui::scroll::Scroll). */
export interface ScrollerLike {
  kind: "document" | "rich";
  focused: boolean;
  at(): number;
  max(): number;
  key(name: string): boolean;
  page(down: boolean): boolean;
}

const SCROLL_NAMES: Array<[(c: Chord) => boolean, string]> = [
  [(c) => keyIs(c, "down"), "down"],
  [(c) => ch(c) === "j", "j"],
  [(c) => keyIs(c, "up"), "up"],
  [(c) => ch(c) === "k", "k"],
  [(c) => keyIs(c, "page_down"), "pageDown"],
  [(c) => keyIs(c, "page_up"), "pageUp"],
  [(c) => keyIs(c, "home"), "home"],
  [(c) => ch(c) === "g", "g"],
  [(c) => keyIs(c, "end"), "end"],
  [(c) => ch(c) === "G", "G"],
];

/**
 * view_tui::key for the panes the frontend scrolls itself (Scroll::key): PgUp/PgDn on a screen without a page scroll
 * the detail Document; the focused Document scrolls; the focused Rich pane scrolls and, moved to its end, sends
 * Intent::End so the part may bring more. "done": the frontend used the key, nothing is sent. undefined: not a
 * scrolling pane's key (a key that does not move it, ↑ at the top, goes on to screenKey and the host).
 */
export function scrollKey(screen: ScreenLike, focus: string | undefined, chord: Chord, scrollers: ScrollerLike[]): Intent | "done" | undefined {
  if (screen.overlay || !plain(chord) || focus == null || panesOf(screen).some(typing)) return undefined;
  const page = keyIs(chord, "page_up") || keyIs(chord, "page_down");
  const panes = panesOf(screen);
  if (page && !screen.page) {
    const detail = panes.find((p) => p.pane === "detail");
    if (detail?.body?.node === "document") {
      const docs = scrollers.filter((x) => x.kind === "document");
      const s = (focus === "detail" ? docs.find((x) => x.focused) : undefined) ?? docs[docs.length - 1];
      return s?.page(keyIs(chord, "page_down")) ? "done" : undefined;
    }
  }
  const pane = panes.find((p) => p.pane === focus);
  if (!pane) return undefined;
  if (pane.body?.node !== "rich" && pane.body?.node !== "document") return undefined;
  const s = [...scrollers].reverse().find((x) => x.focused && x.kind === pane.body.node);
  const name = SCROLL_NAMES.find(([is]) => is(chord))?.[1];
  if (!s || !name) return undefined;
  const moved = s.key(name);
  if (!moved) return undefined;
  if (s.kind === "rich" && s.at() >= s.max()) return { intent: "end", pane: focus as string } as Intent;
  return "done";
}

/** view_tui::section_key for a settings section. */
export function sectionKey(section: { body?: any }, chord: Chord): Intent {
  const body = section.body;
  const key: Intent = { intent: "key", chord };
  if (body?.node === "list" && !body.editing && (body.rows?.length ?? 0) > 0) {
    const rows = body.rows as Array<{ target: string; heading?: boolean }>;
    const at = body.selected ?? 0;
    const up = (keyIs(chord, "up") || ch(chord) === "k") && plain(chord);
    const to = rowsStepBy(rows.length, at, chord, (i) => !!rows[i]!.heading) ?? (up ? at : undefined);
    if (to !== undefined) return { intent: "select", target: rows[to]!.target };
  }
  if (body?.node === "tree" && (body.rows?.length ?? 0) > 0 && !body.editing) {
    const step = treeStep(body as Tree, chord);
    if (step) return step;
    if ((keyIs(chord, "up") || ch(chord) === "k") && plain(chord)) {
      return { intent: "select", target: treeTarget(body.selected ?? []) };
    }
  }
  return key;
}

function rowsOf(body: any): { targets: string[]; selected?: number | null; headings: boolean[] } | undefined {
  switch (body?.node) {
    case "list":
      return { targets: body.rows.map((r: any) => r.target), selected: body.selected, headings: body.rows.map((r: any) => !!r.heading) };
    case "table":
    case "entries":
      return { targets: body.rows.map((r: any) => r.target), selected: body.selected, headings: [] };
    default:
      return undefined;
  }
}

/**
 * The snapshot as the keys already sent would leave it. Several keys can arrive in one read, before
 * the host's next Snapshot: each must be routed from where the previous one went (Ratatui applies
 * each key before the next), so a Select or Fold the renderer sent is applied to a copy of the
 * body it targets until a newer snapshot replaces it.
 */
export function desynced<T extends { focus?: unknown }>(snap: T): T {
  return { ...snap, focus: null };
}

export function afterIntent<T extends { screen?: any; section?: any; focus?: unknown }>(snap: T, intent: Intent): T {
  // Any other intent (a key, activate, invoke...) changes the host's state in ways the renderer cannot foresee
  // (focus moving to another pane, the bar, a page opening): the keys after it in the same read are not routed
  // from this copy but sent as the plain keys the host moves itself on, as Ratatui's runner sends them.
  if (intent.intent !== "select" && intent.intent !== "fold") return desynced(snap);
  // `pane:<id>` focuses that pane and moves nothing (the host's own target: a press on a pane's empty space, the wheel).
  const pane = intent.intent === "select" ? /^pane:(nav|middle|detail)$/.exec(intent.target)?.[1] : undefined;
  if (pane) return { ...snap, focus: pane };
  const patch = (body: any): any => {
    if (!body) return body;
    if (body.node === "tree") {
      const shown = visibleRows(body as Tree);
      if (intent.intent === "select") {
        const hit = shown.find((s) => treeTarget(s.path) === intent.target);
        return hit ? { ...body, selected: hit.path } : body;
      }
      const path = intent.target.split("\u001f");
      const eq = (a: string[], b: string[]) => a.length === b.length && a.every((s, i) => s === b[i]);
      const open = ((body as Tree).open ?? []).filter((o) => !eq(o, path));
      return { ...body, open: intent.open ? [...open, path] : open };
    }
    if (intent.intent === "select" && Array.isArray(body.rows)) {
      const at = body.rows.findIndex((r: any) => r.target === intent.target);
      return at >= 0 ? { ...body, selected: at } : body;
    }
    return body;
  };
  if (snap.section) return { ...snap, section: { ...snap.section, body: patch(snap.section.body) } };
  if (snap.screen) {
    const focus = typeof snap.focus === "string" ? snap.focus : undefined;
    const onPage = !!snap.screen.page;
    const patched = (panes: any[]) => panes.map((p: any) => (p.pane === focus ? { ...p, body: patch(p.body) } : p));
    return {
      ...snap,
      screen: onPage
        ? { ...snap.screen, page: { ...snap.screen.page, panes: patched(snap.screen.page.panes ?? []) } }
        : { ...snap.screen, panes: patched(snap.screen.panes ?? []) },
    };
  }
  return snap;
}
