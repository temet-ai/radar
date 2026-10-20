import type { Chord, Intent } from "../chrome/types.js";
export type { Intent };
import { visibleRows } from "../nodes/tree.js";
import { decodeDetail } from "../nodes/detail.js";
import type { Tree, TreeRow } from "../nodes/view.js";

/**
 * What a key comes to on a view, mirroring crates/radar-core/src/view_tui.rs (`key`, `section_key`)
 * and view.rs (`rows_step_by`, `tree_step`): movement along a list, table, entries or tree selects
 * the next row through the part, ⏎ activates the selection, any other key is passed on as
 * Intent::Key. test/input/intents.json (Ink's golden, first written from the Ratatui adapter)
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

const samePath = (a: string[], b: string[]) => a.length === b.length && a.every((s, i) => s === b[i]);
const beneath = (path: string[], parent: string[]) => path.length > parent.length && parent.every((s, i) => s === path[i]);

/** Tree::row: every component, including each ancestor, must identify exactly one sibling. */
function treeRowAt(tree: Tree, path: string[]): TreeRow | undefined {
  let rows = tree.rows;
  let found: TreeRow | undefined;
  for (const id of path) {
    const matches = rows.filter((row) => row.id === id);
    if (matches.length !== 1) return undefined;
    found = matches[0]!;
    rows = found.children ?? [];
  }
  return found;
}

/** Every group held by the host, parent first, including groups hidden under a closed ancestor. */
export function heldGroupPaths(tree: Tree): string[][] {
  const paths: string[][] = [];
  const walk = (rows: TreeRow[], prefix: string[]) => {
    const ids = new Map<string, number>();
    for (const row of rows) ids.set(row.id, (ids.get(row.id) ?? 0) + 1);
    for (const row of rows) {
      if (ids.get(row.id) !== 1 || !row.children?.length) continue;
      const path = [...prefix, row.id];
      paths.push(path);
      walk(row.children, path);
    }
  };
  walk(tree.rows, []);
  return paths;
}

/** view.rs `tree_step`. */
export function treeStep(tree: Tree, chord: Chord): Intent | undefined {
  if (!plain(chord) || (chord.shift && typeof chord.key === "string")) return undefined;
  const c = ch(chord);
  if (c === "=") return { intent: "open_all" };
  const shown = visibleRows(tree);
  if (shown.length === 0) return undefined;
  const sel = tree.selected ?? [];
  const found = shown.findIndex((s) => samePath(s.path, sel) && !s.row.heading);
  if (found < 0) {
    const movement = ["up", "down", "home", "end"].some((key) => keyIs(chord, key)) || ["j", "k", "g", "G"].includes(c ?? "");
    const first = shown.find((s) => !s.row.heading);
    return movement && first ? { intent: "select", target: treeTarget(first.path) } : undefined;
  }
  const at = found;
  const open = tree.open ?? [];
  const isOpen = (p: string[]) => open.some((o) => samePath(o, p));
  const hasChildren = (i: number) => (shown[i]?.row.children?.length ?? 0) > 0;
  if (found >= 0) {
    const path = shown[at]!.path;
    const group = hasChildren(at);
    if ((c === "+" || keyIs(chord, "right")) && group && !isOpen(path))
      return { intent: "fold", target: treeTarget(path), open: true };
    if (c === " " && group)
      return { intent: "fold", target: treeTarget(path), open: !isOpen(path) };
    if (c === "-" || keyIs(chord, "left")) {
      if (group && isOpen(path)) return { intent: "fold", target: treeTarget(path), open: false };
      const parent = shown.slice(0, at).reverse().find((s) => !s.row.heading && beneath(path, s.path));
      if (parent) return c === "-"
        ? { intent: "fold", target: treeTarget(parent.path), open: false }
        : { intent: "select", target: treeTarget(parent.path) };
    }
    if (keyIs(chord, "right") && group && isOpen(path)) {
      const child = shown.slice(at + 1).find((s) => !s.row.heading && beneath(s.path, path));
      if (child) return { intent: "select", target: treeTarget(child.path) };
    }
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

/** An active field or inline row editor owns keys even while legacy focus names a different pane. */
function typing(pane: PaneLike): boolean {
  const body = pane.body;
  return (pane.fields ?? []).some((f) => f.caret != null) || pane.bottom_field?.caret != null
    || ((body?.node === "list" || body?.node === "tree") && body.editing != null);
}

/** view_tui::panes: an item page's panes while one is open over the screen, else the screen's. */
function panesOf(screen: ScreenLike): PaneLike[] {
  const page = screen.page as { panes?: PaneLike[] } | null | undefined;
  return page ? (page.panes ?? []) : (screen.panes ?? []);
}

/** The first active field or inline editor owns body keys even when legacy focus has not moved. */
export function typingPane(screen: ScreenLike): string | undefined {
  return panesOf(screen).find(typing)?.pane;
}

/** view_tui::key for a screen: which Intent a key is, when the frontend does the movement itself. */
export function screenKey(screen: ScreenLike, focus: string | undefined, chord: Chord, pageStep = 10): Intent {
  const key: Intent = { intent: "key", chord };
  if (screen.overlay) return key;
  // Fields and inline list/tree editors take every key, whichever pane has legacy focus (view_tui::key).
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
  if (body?.node === "tree" && !body.editing) {
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
  let scrolling = pane.body;
  while (scrolling?.node === "extension" && scrolling.capability === "detail" && !decodeDetail(scrolling)) scrolling = scrolling.fallback;
  const kind = decodeDetail(scrolling) ? "rich" : scrolling?.node;
  if (kind !== "rich" && kind !== "document") return undefined;
  const s = [...scrollers].reverse().find((x) => x.focused && x.kind === kind);
  const name = SCROLL_NAMES.find(([is]) => is(chord))?.[1];
  if (!s || !name) return undefined;
  const moved = s.key(name);
  if (!moved) return undefined;
  if (s.kind === "rich" && s.at() >= s.max()) return { intent: "end", pane: focus as string } as Intent;
  return "done";
}

/**
 * host.rs on_key, `View::Settings`: PgUp/PgDn scroll the section's detail (the sheet of settings and of setup) whichever
 * pane has the focus, and never page the rows. "done": the sheet scrolled, nothing is sent. The plain key when it did
 * not (the host says there is nothing to scroll). undefined: not a page key.
 */
export function sheetKey(section: { detail_blocks?: unknown[] | null }, chord: Chord, scrollers: ScrollerLike[]): Intent | "done" | undefined {
  if (!plain(chord) || !(keyIs(chord, "page_up") || keyIs(chord, "page_down"))) return undefined;
  const kind = section.detail_blocks?.length ? "rich" : "document";
  const s = [...scrollers].reverse().find((x) => x.kind === kind);
  return s?.page(keyIs(chord, "page_down")) ? "done" : { intent: "key", chord };
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
  if (body?.node === "tree" && !body.editing) {
    const step = treeStep(body as Tree, chord);
    if (step) return step;
    if ((keyIs(chord, "up") || ch(chord) === "k") && plain(chord) && !chord.shift) {
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
  // OpenAll may be rejected for an unidentified page, or stop early on an owner effect/data change. Interaction
  // metadata is optional, so even an absent context is not proof the whole expansion will complete. Buffered keys
  // after it go raw to the host and use the state it actually reached, just as terminal input does.
  if (intent.intent !== "select" && intent.intent !== "fold") return desynced(snap);
  // `pane:<id>` focuses that pane and moves nothing (the host's own target: a press on a pane's empty space, the wheel).
  const pane = intent.intent === "select" ? /^pane:(nav|middle|detail)$/.exec(intent.target)?.[1] : undefined;
  if (pane) return { ...snap, focus: pane };
  const patch = (body: any): any => {
    if (!body) return body;
    if (body.node === "tree") {
      const shown = visibleRows(body as Tree);
      if (intent.intent === "select") {
        if (!treeRowAt(body as Tree, intent.target.split("\u001f"))) return body;
        const hits = shown.filter((s) => !s.row.heading && treeTarget(s.path) === intent.target);
        return hits.length === 1 ? { ...body, selected: hits[0]!.path } : body;
      }
      const path = intent.target.split("\u001f");
      if (!treeRowAt(body as Tree, path)?.children?.length) return body;
      const open = ((body as Tree).open ?? []).filter((o) => !samePath(o, path));
      const selected = !intent.open && beneath(body.selected ?? [], path) ? path : body.selected;
      return { ...body, selected, open: intent.open ? [...open, path] : open };
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
