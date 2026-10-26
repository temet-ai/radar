// The cases for the fields of what a screen, an item page and a settings section hold (the chrome's are in
// every-field-is-drawn.test.tsx). A case builds the same scene with a field off and on; `on` must draw
// differently. Keys are `Label.field` as support.tsx's viewFields() names them, walked from the generated types.
import { chromeOf } from "./support.js";

export const W = 160;
export const H = 40;

export type Scene = { chrome: any; screen?: any; section?: any; size?: [number, number]; extra?: any[]; needle?: string; plainText?: boolean };
export type Case = (on: boolean) => Scene;

const t = (s: string, over: Record<string, unknown> = {}) => [{ text: s, ...over }];
const pick = <T,>(on: boolean, off: T, yes: T): T => (on ? yes : off);

const screenOf = (panes: any[], over: Record<string, unknown> = {}) => ({ id: "n", scope: "n", panes, actions: [], ...over });
const paneOf = (body: any, over: Record<string, unknown> = {}) => ({ pane: "middle", title: "PANE", body, ...over });
const sceneOf = (screen: any, size?: [number, number]): Scene => ({ chrome: chromeOf(size?.[0] ?? W, size?.[1] ?? H), screen, size });
const doc = (...lines: string[]) => ({ node: "document", lines: (lines.length ? lines : ["BODYLINE"]).map((l) => t(l)) });
const action = (f: (a: any, on: boolean) => void): Case => (on) => {
  const a = { id: "watch.refresh", label: "refresh", key: { key: { char: "r" } } };
  f(a, on);
  return sceneOf(screenOf([paneOf({ node: "hologram" })], { actions: [a] }));
};

/** A pane built by `f` on a screen of its own. */
const pane = (f: (p: any, on: boolean) => void, size?: [number, number]): Case => (on) => {
  const p: any = paneOf(doc());
  f(p, on);
  return sceneOf(screenOf([p]), size);
};
/** A node built by `f` (from `base`) in the middle pane. */
const node = (base: () => any, f: (n: any, on: boolean) => void): Case => (on) => {
  const n = base();
  f(n, on);
  return sceneOf(screenOf([paneOf(n)]));
};

// ---- base nodes -----------------------------------------------------------------------------------------------
const listBase = () => ({
  node: "list",
  selected: 0,
  rows: [
    { target: "r0", text: t("ROWZERO") },
    { target: "r1", text: t("ROWONE") },
  ],
});
const treeBase = () => ({
  node: "tree",
  columns: [],
  name_width: 24,
  open: [["a"]],
  selected: ["a"],
  rows: [
    { id: "a", name: t("alpha"), children: [{ id: "a1", name: t("achild") }] },
    { id: "b", name: t("bravo") },
  ],
});
const tableBase = () => ({
  node: "table",
  columns: [
    { heading: "COLA", width: 12, keep: 0 },
    { heading: "COLB", width: 12, keep: 1 },
  ],
  rows: [{ target: "t0", cells: [{ heading: "COLA", text: t("cella") }, { heading: "COLB", text: t("cellb") }] }],
  selected: 0,
  empty: null,
});
const entriesBase = () => ({ node: "entries", selected: 0, empty: "NOENTRIES", rows: [{ target: "e0", head: t("EHEAD") }, { target: "e1", head: t("ETWO") }] });
const logBase = () => ({ node: "log", top: 0, total: 3, empty: "NOLOG", lines: [{ text: t("LOGONE"), row: 0, selected: false }] });
const diffBase = () => ({
  node: "diff",
  selected: 0,
  top: 0,
  split: false,
  language: null,
  empty: "NODIFF",
  under: [],
  lines: [
    { kind: "Hunk", old: null, new: null, text: "@@ -1,2 +1,2 @@" },
    { kind: "Removed", old: 1, new: null, text: "gone line" },
    { kind: "Added", old: null, new: 1, text: "new line" },
  ],
});
const richOf = (...blocks: any[]) => ({ node: "rich", blocks });
const para = (inlines: any[]) => ({ t: "para", text: inlines });

const tree = (f: (n: any, on: boolean) => void): Case => node(treeBase, f);
/** A tree in a narrow room, so columns and names compete for it. */
const treeNarrow = (f: (n: any, on: boolean) => void): Case => (on) => {
  const n = treeBase();
  f(n, on);
  return sceneOf(screenOf([paneOf(n)]), [50, 24]);
};
const treeRow = (f: (r: any, on: boolean) => void): Case => tree((n, on) => f(n.rows[1], on));
/** A tree whose open root holds three blocks, so the band (every other block at the band level) has one to light. */
const treeBanded = (look: (on: boolean) => any): Case => (on) => {
  const n = treeBase();
  n.selected = ["b"];
  n.rows[0]!.children = [{ id: "a1", name: t("achild") }, { id: "a2", name: t("asecond") }, { id: "a3", name: t("athird") }];
  n.look = look(on);
  return sceneOf(screenOf([paneOf(n)]));
};
/** A tree row with two name spans and chips after them, in a room wide enough for chips (from 100 cells). */
const treeChips = (chips: (on: boolean) => any, needle?: string): Case => (on) => {
  const n = treeBase();
  n.rows[1].name = [{ text: "bravo-title-that-is-long" }, { text: " main" }];
  n.rows[1].chips = chips(on);
  return { ...sceneOf(screenOf([paneOf(n)]), [240, 40]), needle };
};
const list = (f: (n: any, on: boolean) => void): Case => node(listBase, f);
const listRow = (f: (r: any, on: boolean) => void): Case => list((n, on) => f(n.rows[1], on));
const table = (f: (n: any, on: boolean) => void): Case => node(tableBase, f);
const entries = (f: (n: any, on: boolean) => void): Case => node(entriesBase, f);
const log = (f: (n: any, on: boolean) => void): Case => node(logBase, f);
const diff = (f: (n: any, on: boolean) => void): Case => node(diffBase, f);
/** A rich node holding one block built by `mk` (given on). */
const rich = (mk: (on: boolean) => any): Case => (on) => sceneOf(screenOf([paneOf(richOf({ block: "doc", doc: [mk(on)] }))]));
/** A paragraph of one inline built by `f`. */
const inline = (f: (i: any, on: boolean) => void): Case => rich((on) => {
  const i: any = { text: "word" };
  f(i, on);
  return para([i]);
});
/** A list row whose text is one segment built by `f`. */
const seg = (f: (s: any, on: boolean) => void): Case => list((n, on) => {
  const s: any = { text: "segtext" };
  f(s, on);
  n.rows[1].text = [s];
});

const pageBase = (): any => ({
  crumbs: [{ text: "CRUMB" }],
  title: t("PAGETITLE"),
  tab: 0,
  panes: [paneOf(doc("PAGEBODY"))],
});
const page = (f: (p: any, on: boolean) => void): Case => (on) => {
  const p = pageBase();
  f(p, on);
  return sceneOf(screenOf([], { page: p }));
};
const twoPanes = () => [paneOf(doc("FIRSTPANE"), { title: "FIRST" }), paneOf(doc("SECONDPANE"), { title: "SECOND" })];
const layoutPage = (layout: (on: boolean) => any): Case => (on) => {
  const p = pageBase();
  p.panes = twoPanes();
  p.layout = layout(on);
  return sceneOf(screenOf([], { page: p }));
};

const sectionOf = (over: Record<string, unknown> = {}) => ({
  version: 1,
  scope: "connection",
  body: doc("SECTIONBODY"),
  detail_title: "DETAILTITLE",
  detail: [t("DETAILBODY")],
  ...over,
});
const section = (f: (s: any, on: boolean) => void): Case => (on) => {
  const s: any = sectionOf();
  f(s, on);
  return { chrome: chromeOf(W, H), section: s };
};

// ---- the cases ------------------------------------------------------------------------------------------------

export const SCREEN_CASES: Record<string, Case> = {
  // ScreenView
  "ScreenView.panes": (on) => sceneOf(screenOf(on ? [paneOf(doc("ZZPANE"))] : [])),
  "ScreenView.medium_from": (on) => sceneOf(screenOf([paneOf(doc("NAVPANE"), { pane: "nav", title: "NAV" }), paneOf(doc("MIDPANE"))], { medium_from: on ? 60 : 150 }), [100, 30]),
  // A dragged split moves the borders between the panes ([layout], app/layout-drag.ts).
  "ScreenView.layout": (on) => sceneOf(screenOf([paneOf(doc("NAVPANE"), { pane: "nav", title: "NAV" }), paneOf(doc("MIDPANE"))], { layout: on ? { nav: 400 } : null }), [160, 30]),
  "LayoutView.nav": (on) => sceneOf(screenOf([paneOf(doc("NAVPANE"), { pane: "nav", title: "NAV" }), paneOf(doc("MIDPANE"))], { layout: { nav: on ? 400 : 100 } }), [160, 30]),
  "LayoutView.detail": (on) => sceneOf(screenOf([paneOf(doc("MIDPANE")), paneOf(doc("DETPANE"), { pane: "detail", title: "DETAIL" })], { layout: { detail: on ? 600 : 200 } }), [160, 30]),
  "ScreenView.page": (on) => sceneOf(screenOf([paneOf(doc("UNDERPAGE"))], { page: on ? pageBase() : null })),
  // A generic overlay is a titled pane over the foot of the body; its text must show, not only the focus ramp's change.
  "ScreenView.overlay": (on) => ({ ...sceneOf(screenOf([paneOf(doc("BODYUNDER"))], { overlay: on ? paneOf(doc("OVERLAYTEXT"), { title: "OVERLAY" }) : null })), needle: "OVERLAYTEXT" }),

  // Actions stay visible when the shell cannot draw a node.
  "ActionSpec.label": action((a, on) => (a.label = pick(on, "refresh", "ZZREFRESH"))),
  "ActionSpec.key": action((a, on) => (a.key.key.char = pick(on, "r", "z"))),
  "ActionSpec.heading": action((a, on) => (a.heading = on)),
  "ActionSpec.mnemonic_at": action((a, on) => (a.mnemonic_at = on ? 0 : null)),
  "ActionSpec.mnemonic_shift": action((a, on) => { a.key.key.char = "R"; a.mnemonic_at = 0; a.mnemonic_shift = on; }),

  // SectionView
  "SectionView.scope": (on) => ({ chrome: chromeOf(W, H), section: sectionOf({ scope: pick(on, "connection", "ZZSCOPE") }) }),
  "SectionView.body": section((s, on) => (s.body = doc(pick(on, "SECTIONBODY", "ZZBODY")))),
  "SectionView.detail_title": section((s, on) => (s.detail_title = pick(on, "DETAILTITLE", "ZZDETAILTITLE"))),
  "SectionView.detail": section((s, on) => (s.detail = on ? s.detail : [])),
  "SectionView.detail_blocks": section((s, on) => (s.detail_blocks = on ? [{ block: "line", text: t("ZZBLOCK") }] : [])),

  // PaneView
  "PaneView.pane": (on) => sceneOf(screenOf([paneOf(doc("ONLYPANE"), { pane: pick(on, "middle", "nav") })]), [100, 30]),
  "PaneView.title": pane((p, on) => (p.title = pick(on, "PANE", "ZZTITLE"))),
  "PaneView.title_icon": pane((p, on) => (p.title_icon = on ? "settings" : null)),
  "PaneView.body": pane((p, on) => (p.body = doc(pick(on, "BODYLINE", "ZZBODY")))),
  "PaneView.notes": pane((p, on) => (p.notes = on ? [t("ZZNOTE")] : [])),
  "PaneView.footer": pane((p, on) => (p.footer = on ? t("ZZFOOTER") : null)),
  "PaneView.width": (on) => sceneOf(screenOf([paneOf(doc("NAVPANE"), { pane: "nav", title: "NAV", width: on ? 50 : null }), paneOf(doc("MIDPANE"))])),
  "PaneView.fields": pane((p, on) => (p.fields = on ? [{ mark: "?", text: "ZZFIELD", hint: [] }] : [])),
  "PaneView.bottom_field": pane((p, on) => (p.bottom_field = on ? { mark: "?", text: "ZZBOTTOM", hint: [] } : null)),
  "PaneView.head": pane((p, on) => (p.head = on ? t("ZZHEAD") : null)),
  // `from` holds on a page: the first pane is not drawn on a page narrower than it (view_tui render_page).
  "PaneView.from": page((p, on) => {
    p.panes = twoPanes();
    p.panes[0].from = on ? 200 : 60;
  }),
  "PaneView.notes_most": pane((p, on) => {
    p.notes = ["n1", "n2", "n3", "n4", "n5"].map((x) => t(`NOTE${x}`));
    p.notes_most = on ? 1 : null;
  }),
  "PaneView.inset": pane((p, on) => (p.inset = on)),
  // Where the part moved the body's scroll (#32): the body is drawn from there.
  "PaneView.scroll": pane((p, on) => {
    p.body = { node: "extension", capability: "detail", version: 1,
      data: { header: { path: t("watch"), title: t("WATCH-12") }, sections: Array.from({ length: 60 }, (_, i) => ({ block: "line", text: t(`ROW${i}`) })) },
      fallback: doc() };
    p.scroll = on ? 30 : null;
  }),
  // FieldBar
  "FieldBar.mark": pane((p, on) => (p.fields = [{ mark: pick(on, "?", "!"), text: "field", hint: [] }])),
  "FieldBar.text": pane((p, on) => (p.fields = [{ mark: "?", text: pick(on, "field", "ZZFIELD"), hint: [] }])),
  "FieldBar.caret": pane((p, on) => (p.fields = [{ mark: "?", text: "abcdef", caret: on ? 2 : null, hint: [] }])),
  "FieldBar.hint": pane((p, on) => (p.fields = [{ mark: "?", text: "field", hint: on ? t("ZZHINT") : [] }])),

  // PageView
  "PageView.crumbs": page((p, on) => (p.crumbs = on ? p.crumbs : [])),
  "PageView.title": page((p, on) => (p.title = t(pick(on, "PAGETITLE", "ZZTITLE")))),
  "PageView.details": page((p, on) => (p.details = on ? t("ZZDETAILS") : [])),
  "PageView.under": page((p, on) => (p.under = on ? t("ZZUNDER") : null)),
  "PageView.tabs": page((p, on) => (p.tabs = on ? [{ label: "TABONE" }, { label: "TABTWO" }] : [])),
  "PageView.tab": page((p, on) => {
    p.tabs = [{ label: "TABONE" }, { label: "TABTWO" }];
    p.tab = on ? 1 : 0;
  }),
  "PageView.notice": page((p, on) => (p.notice = on ? t("ZZNOTICE") : null)),
  "PageView.panes": page((p, on) => (p.panes = on ? p.panes : [])),
  "PageView.layout": layoutPage((on) => (on ? { kind: "split", axis: "vertical", parts: [{ size: { kind: "fill", value: 1 }, layout: { kind: "pane", index: 0 } }, { size: { kind: "fill", value: 1 }, layout: { kind: "pane", index: 1 } }] } : null)),
  "TabView.label": page((p, on) => (p.tabs = [{ label: pick(on, "TABONE", "ZZTAB") }])),
  "TabView.icon": page((p, on) => (p.tabs = [{ label: "TABONE", icon: on ? "settings" : null }])),
  "TabView.attention": page((p, on) => (p.tabs = [{ label: "TABONE", attention: on }])),
  // PageLayout / LayoutPart / Size
  "PageLayout.pane.index": layoutPage((on) => ({ kind: "pane", index: on ? 1 : 0 })),
  "PageLayout.split.axis": layoutPage((on) => ({ kind: "split", axis: pick(on, "horizontal", "vertical"), parts: [{ size: { kind: "fill", value: 1 }, layout: { kind: "pane", index: 0 } }, { size: { kind: "fill", value: 1 }, layout: { kind: "pane", index: 1 } }] })),
  "PageLayout.split.parts": layoutPage((on) => ({ kind: "split", axis: "horizontal", parts: on ? [{ size: { kind: "fill", value: 1 }, layout: { kind: "pane", index: 0 } }, { size: { kind: "fill", value: 1 }, layout: { kind: "pane", index: 1 } }] : [{ size: { kind: "fill", value: 1 }, layout: { kind: "pane", index: 0 } }] })),
  "PageLayout.responsive.min_width": layoutPage((on) => ({ kind: "responsive", min_width: on ? 200 : 50, wide: { kind: "pane", index: 0 }, narrow: { kind: "pane", index: 1 } })),
  "PageLayout.responsive.wide": layoutPage((on) => ({ kind: "responsive", min_width: 50, wide: { kind: "pane", index: on ? 0 : 1 }, narrow: { kind: "pane", index: 1 } })),
  "PageLayout.responsive.narrow": layoutPage((on) => ({ kind: "responsive", min_width: 500, wide: { kind: "pane", index: 0 }, narrow: { kind: "pane", index: on ? 1 : 0 } })),
  "LayoutPart.size": layoutPage((on) => ({ kind: "split", axis: "horizontal", parts: [{ size: on ? { kind: "length", value: 30 } : { kind: "fill", value: 1 }, layout: { kind: "pane", index: 0 } }, { size: { kind: "fill", value: 1 }, layout: { kind: "pane", index: 1 } }] })),
  "LayoutPart.layout": layoutPage((on) => ({ kind: "split", axis: "horizontal", parts: [{ size: { kind: "fill", value: 1 }, layout: { kind: "pane", index: on ? 1 : 0 } }, { size: { kind: "fill", value: 1 }, layout: { kind: "pane", index: 1 } }] })),
  "Size.fill.value": layoutPage((on) => ({ kind: "split", axis: "horizontal", parts: [{ size: { kind: "fill", value: on ? 5 : 1 }, layout: { kind: "pane", index: 0 } }, { size: { kind: "fill", value: 1 }, layout: { kind: "pane", index: 1 } }] })),
  "Size.length.value": layoutPage((on) => ({ kind: "split", axis: "horizontal", parts: [{ size: { kind: "length", value: on ? 60 : 30 }, layout: { kind: "pane", index: 0 } }, { size: { kind: "fill", value: 1 }, layout: { kind: "pane", index: 1 } }] })),

  // Node.list, ListRow, Editing
  "Node.list.rows": list((n, on) => (n.rows = on ? n.rows : [])),
  "Node.list.selected": list((n, on) => (n.selected = on ? 1 : 0)),
  "Node.list.editing": list((n, on) => (n.editing = on ? { row: 1, at: 0, text: "ZZEDIT", caret: 2 } : null)),
  "Editing.row": list((n, on) => (n.editing = { row: on ? 1 : 0, at: 0, text: "edit", caret: 1 })),
  "Editing.at": list((n, on) => (n.editing = { row: 1, at: on ? 3 : 0, text: "edit", caret: 1 })),
  "Editing.text": list((n, on) => (n.editing = { row: 1, at: 0, text: pick(on, "edit", "ZZEDIT"), caret: 1 })),
  "Editing.caret": list((n, on) => (n.editing = { row: 1, at: 0, text: "edit", caret: on ? 3 : 0 })),
  "ListRow.text": listRow((r, on) => (r.text = t(pick(on, "ROWONE", "ZZTEXT")))),
  "ListRow.right": listRow((r, on) => (r.right = on ? t("ZZRIGHT") : null)),
  "ListRow.link": listRow((r, on) => (r.link = on ? "https://example.test/x" : null)),
  "ListRow.detail": listRow((r, on) => (r.detail = on ? t("ZZDETAIL") : null)),
  // The cursor rests on the next row that is no heading, so a heading under the cursor moves it.
  "ListRow.heading": list((n, on) => {
    n.selected = 1;
    n.rows[1].heading = on;
  }),

  // Node.tree and what it holds
  "Tree.header": tree((n, on) => (n.header = on ? t("ZZHEADER") : [])),
  "Tree.columns": tree((n, on) => {
    n.columns = on ? [{ heading: "COLX", width: 10, keep: 0 }] : [];
    n.rows[1].cells = [{ heading: "COLX", text: t("cellx") }];
  }),
  // 47 cells of row, names of at least 20: own COLX and COLZ fit (20 + 20), the extra COLY does not fit beside them, so it
  // is left out; fitted in declared order the lowest keep goes (the later of equals, COLZ) and COLY stays.
  "Tree.ordered_columns": treeNarrow((n, on) => {
    n.columns = [{ heading: "COLX", width: 10, keep: 0 }, { heading: "COLY", width: 10, keep: 0, extra: true }, { heading: "COLZ", width: 10, keep: 0 }];
    n.rows[1].cells = [{ heading: "COLX", text: t("cellx") }, { heading: "COLY", text: t("celly") }, { heading: "COLZ", text: t("cellz") }];
    n.name_width = 20;
    n.ordered_columns = on;
  }),
  "Tree.rows": tree((n, on) => (n.rows = on ? n.rows : [])),
  "Tree.selected": tree((n, on) => (n.selected = on ? ["b"] : ["a"])),
  "Tree.open": tree((n, on) => (n.open = on ? [["a"]] : [])),
  // Two columns of 18: both fit beside names of 8, with names of 30 the lower keep (COLX) goes.
  "Tree.name_width": treeNarrow((n, on) => {
    n.columns = [{ heading: "COLX", width: 18, keep: 0 }, { heading: "COLY", width: 18, keep: 1 }];
    n.rows[1].cells = [{ heading: "COLX", text: t("cellx") }, { heading: "COLY", text: t("celly") }];
    n.name_width = on ? 30 : 8;
  }),
  "Tree.look": tree((n, on) => (n.look = on ? { lines: "dots", band: "x", heading: "y", band_level: 1 } : null)),
  "Tree.note": tree((n, on) => (n.note = on ? { state: "empty", text: "ZZNOTE" } : null)),
  "Tree.cut_names": treeNarrow((n, on) => {
    n.columns = [{ heading: "COLX", width: 30, keep: 0 }];
    n.rows[1].cells = [{ heading: "COLX", text: t("cellx") }];
    n.name_width = 8;
    n.rows[1].name = t("averyveryverylongnamethatdoesnotfit");
    n.cut_names = on;
  }),
  "Tree.editing": tree((n, on) => (n.editing = on ? { path: ["b"], at: 0, text: "ZZEDIT", caret: 1 } : null)),
  "Note.state": tree((n, on) => (n.note = { state: pick(on, "empty", "failed"), text: "notetext" })),
  "Note.text": tree((n, on) => (n.note = { state: "empty", text: pick(on, "notetext", "ZZNOTE") })),
  "TreeEditing.path": tree((n, on) => (n.editing = { path: on ? ["b"] : ["a"], at: 0, text: "ZZEDIT", caret: 1 })),
  "TreeEditing.at": tree((n, on) => (n.editing = { path: ["b"], at: on ? 3 : 0, text: "edit", caret: 1 })),
  "TreeEditing.text": tree((n, on) => (n.editing = { path: ["b"], at: 0, text: pick(on, "edit", "ZZEDIT"), caret: 1 })),
  "TreeEditing.caret": tree((n, on) => (n.editing = { path: ["b"], at: 0, text: "edit", caret: on ? 3 : 0 })),
  // A look is slot names (Theme::spec); the lines are a foreground, the band and the heading shade backgrounds.
  "TreeLook.lines": treeBanded((on) => ({ lines: pick(on, "text.muted", "status.error"), band: "bg.overlay", heading: "bg.overlay", band_level: 1 })),
  "TreeLook.band": treeBanded((on) => ({ lines: "text.muted", band: pick(on, "bg.overlay", "bg.selection"), heading: "bg.surface", band_level: 1 })),
  "TreeLook.heading": treeBanded((on) => ({ lines: "text.muted", band: "bg.overlay", heading: pick(on, "bg.overlay", "bg.selection"), band_level: 1 })),
  "TreeLook.band_level": treeBanded((on) => ({ lines: "text.muted", band: "bg.selection", heading: "bg.surface", band_level: on ? 1 : 2 })),
  "TreeRow.name": treeRow((r, on) => (r.name = t(pick(on, "bravo", "ZZNAME")))),
  "TreeRow.cells": tree((n, on) => {
    n.columns = [{ heading: "COLX", width: 10, keep: 0 }];
    n.rows[1].cells = on ? [{ heading: "COLX", text: t("ZZCELL") }] : [];
  }),
  "TreeRow.children": tree((n, on) => {
    n.open = [["b"]];
    n.rows[1].children = on ? [{ id: "b1", name: t("ZZCHILD") }] : [];
  }),
  // The open root and its descendant must both ignore this retained compatibility field (#45).
  "TreeRow.stripe": tree((n, on) => (n.rows[0].stripe = on ? "accent" : null)),
  "TreeRow.chips": treeChips((on) => (on ? { gives: 0, text: t("ZZCHIP") } : null), "ZZCHIP"),
  "TreeRow.detail": treeRow((r, on) => (r.detail = on ? t("ZZDETAIL") : null)),
  // `gives` names the name span that gives way to the chips: span 0 is the title; none at 2, there are two spans
  "Chips.gives": treeChips((on) => ({ gives: on ? 0 : 2, text: t("ZZCHIP") }), "ZZCHIP"),
  "Chips.text": treeChips((on) => ({ gives: 0, text: t(pick(on, "chiptext", "ZZCHIP")) }), "ZZCHIP"),
  "ColumnSpec.heading": tree((n, on) => {
    n.columns = [{ heading: pick(on, "COLX", "ZZHEAD"), width: 10, keep: 0 }];
    n.rows[1].cells = [{ heading: "COLX", text: t("cellx") }];
  }),
  "ColumnSpec.width": table((n, on) => (n.columns[0].width = on ? 30 : 12)),
  "ColumnSpec.keep": treeNarrow((n, on) => {
    n.columns = [{ heading: "COLX", width: 22, keep: on ? 5 : 0 }, { heading: "COLY", width: 22, keep: on ? 0 : 5 }];
    n.rows[1].cells = [{ heading: "COLX", text: t("cellx") }, { heading: "COLY", text: t("celly") }];
    n.name_width = 20;
  }),
  "ColumnSpec.icon": table((n, on) => (n.columns[0].icon = on ? "settings" : null)),
  "ColumnSpec.flex": table((n, on) => (n.columns[0].flex = on)),
  // Fitted by keep COLZ stays (keep 5) and COLY goes; marked extra it is only added where it fits beside the part's own: not here.
  "ColumnSpec.extra": treeNarrow((n, on) => {
    n.columns = [{ heading: "COLX", width: 12, keep: 0 }, { heading: "COLY", width: 12, keep: 0 }, { heading: "COLZ", width: 12, keep: 5, extra: on }];
    n.rows[1].cells = [{ heading: "COLX", text: t("cellx") }, { heading: "COLY", text: t("celly") }, { heading: "COLZ", text: t("cellz") }];
    n.name_width = 20;
  }),
  // COLX was dragged to 30: with `natural` it gives that back in the narrow room and COLY stays; without, COLY goes.
  "ColumnSpec.natural": treeNarrow((n, on) => {
    n.columns = [{ heading: "COLX", width: 30, keep: 5, natural: on ? 8 : null }, { heading: "COLY", width: 8, keep: 0 }];
    n.rows[1].cells = [{ heading: "COLX", text: t("cellx") }, { heading: "COLY", text: t("celly") }];
    n.name_width = 12;
  }),
  "CellView.heading": table((n, on) => (n.rows[0].cells[0].heading = pick(on, "COLA", "ZZHEAD"))),
  "CellView.text": table((n, on) => (n.rows[0].cells[0].text = t(pick(on, "cella", "ZZCELL")))),

  // Node.table, entries, log, diff, state, extension, document, rich
  "Node.table.columns": table((n, on) => (n.columns = on ? n.columns : [])),
  "Node.table.rows": table((n, on) => (n.rows = on ? n.rows : [])),
  "Node.table.selected": table((n, on) => {
    n.rows.push({ target: "t1", cells: [{ heading: "COLA", text: t("rowtwo") }] });
    n.selected = on ? 1 : 0;
  }),
  "Node.table.empty": table((n, on) => {
    n.rows = [];
    n.empty = on ? "ZZEMPTY" : null;
  }),
  "TableRow.cells": table((n, on) => (n.rows[0].cells = on ? n.rows[0].cells : [])),
  "Node.entries.rows": entries((n, on) => (n.rows = on ? n.rows : [])),
  "Node.entries.selected": entries((n, on) => (n.selected = on ? 1 : 0)),
  "Node.entries.empty": entries((n, on) => {
    n.rows = [];
    n.empty = pick(on, "NOENTRIES", "ZZEMPTY");
  }),
  "EntryView.head": entries((n, on) => (n.rows[1].head = t(pick(on, "ETWO", "ZZHEAD")))),
  "EntryView.before": entries((n, on) => (n.rows[1].before = on ? [t("ZZBEFORE")] : [])),
  "EntryView.body": entries((n, on) => (n.rows[1].body = on ? [t("ZZBODY")] : [])),
  "Node.log.lines": log((n, on) => (n.lines = on ? n.lines : [])),
  // The scrollbar on the pane's border: more rows held (30) than shown (3), the thumb where `top` is.
  "Node.log.top": log((n, on) => {
    n.lines = [0, 1, 2].map((i) => ({ text: t(`LOG${i}`), row: i, selected: false }));
    n.total = 30;
    n.top = on ? 20 : 0;
  }),
  "Node.log.total": log((n, on) => (n.total = on ? 300 : 3)),
  "Node.log.empty": log((n, on) => {
    n.lines = [];
    n.empty = pick(on, "NOLOG", "ZZEMPTY");
  }),
  "LogPaintLine.text": log((n, on) => (n.lines[0].text = t(pick(on, "LOGONE", "ZZLOG")))),
  // Lines of one row count once toward what is shown (a wrapped row), so the rows hidden are total less those.
  "LogPaintLine.row": log((n, on) => {
    n.lines = [0, 1, 2, 3].map((i) => ({ text: t(`LOG${i}`), row: on ? i : 0, selected: false }));
    n.total = 8;
    n.top = 4;
  }),
  "LogPaintLine.selected": log((n, on) => (n.lines[0].selected = on)),
  "Node.diff.lines": diff((n, on) => (n.lines = on ? n.lines : [])),
  "Node.diff.under": diff((n, on) => (n.under = on ? [{ line: 1, lines: [t("ZZUNDER")] }] : [])),
  "Node.diff.selected": diff((n, on) => (n.selected = on ? 2 : 0)),
  // The view scrolls to keep the cursor's line in sight, so the cursor sits where `top` decides: far down, a tall diff.
  "Node.diff.top": diff((n, on) => {
    n.lines = Array.from({ length: 80 }, (_, i) => ({ kind: "Context", old: i + 1, new: i + 1, text: `diffline${i}` }));
    n.selected = 30;
    n.top = on ? 20 : 0;
  }),
  "Node.diff.split": diff((n, on) => (n.split = on)),
  "Node.diff.painted": diff((n, on) => (n.painted = on ? n.lines.map((l: any) => [{ text: String(l.text ?? ""), token: "keyword" }]) : [])),
  "Node.diff.words": diff((n, on) => (n.words = on ? n.lines.map(() => [[0, 3]]) : [])),
  "PaintRun.text": diff((n, on) => (n.painted = n.lines.map((l: any) => [{ text: on ? "ZZPAINT" : String(l.text ?? ""), token: "plain" }]))),
  "PaintRun.token": diff((n, on) => (n.painted = n.lines.map((l: any) => [{ text: String(l.text ?? ""), token: on ? "keyword" : "plain" }]))),
  "Node.diff.empty": diff((n, on) => {
    n.lines = [];
    n.empty = pick(on, "NODIFF", "ZZEMPTY");
  }),
  "DiffLine.kind": diff((n, on) => (n.lines[2].kind = pick(on, "Added", "Context"))),
  "DiffLine.old": diff((n, on) => (n.lines[1].old = on ? 77 : 1)),
  "DiffLine.new": diff((n, on) => (n.lines[2].new = on ? 88 : 1)),
  "DiffLine.text": diff((n, on) => (n.lines[2].text = pick(on, "new line", "ZZTEXT"))),
  "DiffUnder.line": diff((n, on) => (n.under = [{ line: on ? 2 : 1, lines: [t("underline")] }])),
  "DiffUnder.lines": diff((n, on) => (n.under = [{ line: 1, lines: on ? [t("underline")] : [] }])),
  // The field takes the block's last line (view_tui: the rect's last row), so the block holds one for it to stand on.
  "DiffUnder.input": diff((n, on) => (n.under = [{ line: 1, lines: [t("underline")], input: on ? { text: "ZZINPUT", caret: 1 } : null }])),
  "InlineInput.text": diff((n, on) => (n.under = [{ line: 1, lines: [t("underline")], input: { text: pick(on, "input", "ZZINPUT"), caret: 1 } }])),
  "InlineInput.caret": diff((n, on) => (n.under = [{ line: 1, lines: [t("underline")], input: { text: "input", caret: on ? 4 : 0 } }])),
  "InlineInput.selection": diff((n, on) => (n.under = [{ line: 1, lines: [t("underline")], input: { text: "input", caret: 2, selection: on ? [0, 3] : null } }])),
  "Node.state.text": (on) => sceneOf(screenOf([paneOf({ node: "state", state: "empty", text: pick(on, "statetext", "ZZSTATE") })])),
  "Node.extension.capability": (on) => sceneOf(screenOf([paneOf({ node: "extension", capability: pick(on, "zz.unknown", "zz.other"), version: 1, data: {}, fallback: doc("FALLBACK") })])),
  "Node.extension.fallback": (on) => sceneOf(screenOf([paneOf({ node: "extension", capability: "zz.unknown", version: 1, data: {}, fallback: doc(pick(on, "FALLBACK", "ZZFALLBACK")) })])),
  "Node.document.lines": node(() => doc(), (n, on) => (n.lines = on ? n.lines : [])),
  "Node.document.cut": node(() => ({ node: "document", lines: [t("x".repeat(300))] }), (n, on) => (n.cut = on)),
  "Node.rich.blocks": (on) => sceneOf(screenOf([paneOf(richOf(...(on ? [para([{ text: "ZZPARA" }])] : [])))])),
  "Node.rich.pictures": (on) => sceneOf(screenOf([paneOf({ node: "rich", blocks: [{ t: "media", image: true, name: "pic", src: "/x.png" }], pictures: on ? "kitty" : null })])),

  // Block (a settings detail's or a pane's rich lines)
  "Block.line.text": (on) => sceneOf(screenOf([paneOf(richOf({ block: "line", text: t(pick(on, "LINETEXT", "ZZLINE")) }))])),
  "Block.line.key": (on) => sceneOf(screenOf([paneOf(richOf({ block: "line", text: t("act"), key: on ? "Z" : null }))])),
  "Block.line.mnemonic_at": (on) => sceneOf(screenOf([paneOf(richOf({ block: "line", text: t("refresh"), key: "r", mnemonic_at: on ? 0 : null }))])),
  "Block.line.mnemonic_shift": (on) => sceneOf(screenOf([paneOf(richOf({ block: "line", text: t("refresh"), key: "R", mnemonic_at: 0, mnemonic_shift: on }))])),
  "Block.section.mark": (on) => sceneOf(screenOf([paneOf(richOf({ block: "section", title: "SECT", mark: on ? "settings" : null, blocks: [] }))])),
  "Block.section.title": (on) => sceneOf(screenOf([paneOf(richOf({ block: "section", title: pick(on, "SECT", "ZZSECT"), blocks: [] }))])),
  "Block.section.detail": (on) => sceneOf(screenOf([paneOf(richOf({ block: "section", title: "SECT", detail: on ? "ZZDETAIL" : null, blocks: [] }))])),
  "Block.section.blocks": (on) => sceneOf(screenOf([paneOf(richOf({ block: "section", title: "SECT", blocks: on ? [{ block: "line", text: t("ZZINSIDE") }] : [] }))])),
  "Block.doc.doc": (on) => sceneOf(screenOf([paneOf(richOf({ block: "doc", doc: on ? [para([{ text: "ZZDOC" }])] : [] }))])),
  "Block.doc.bar": (on) => sceneOf(screenOf([paneOf(richOf({ block: "doc", doc: [para([{ text: "docbody" }])], bar: on ? { text: "ZZBAR" } : null }))])),
  "Block.doc.most": (on) => sceneOf(screenOf([paneOf(richOf({ block: "doc", doc: ["a", "b", "c", "d"].map((x) => para([{ text: `DOC${x}` }])), most: on ? 1 : null }))])),
  "Block.doc.urls": (on) => sceneOf(screenOf([paneOf(richOf({ block: "doc", doc: [para([{ text: "see", kind: { link: { url: "https://example.test/ZZURL" } } }])], urls: on }))])),
  "Block.meter.stages": (on) => sceneOf(screenOf([paneOf(richOf({ block: "meter", stages: on ? [{ name: "ZZSTAGE", mark: "done", look: "good" }] : [] }))])),
  "Block.wrapped.text": (on) => sceneOf(screenOf([paneOf(richOf({ block: "wrapped", text: pick(on, "wrapped", "ZZWRAP"), prefix: { text: "> " }, ink: "plain" }))])),
  "Block.wrapped.prefix": (on) => sceneOf(screenOf([paneOf(richOf({ block: "wrapped", text: "wrapped", prefix: { text: pick(on, "> ", "ZZPFX") }, ink: "plain" }))])),
  "Block.wrapped.ink": (on) => sceneOf(screenOf([paneOf(richOf({ block: "wrapped", text: "wrapped", prefix: { text: "> " }, ink: pick(on, "plain", "bad") }))])),
  "MeterStage.name": (on) => sceneOf(screenOf([paneOf(richOf({ block: "meter", stages: [{ name: pick(on, "stage", "ZZSTAGE"), mark: "done", look: "good" }] }))])),
  "MeterStage.mark": (on) => sceneOf(screenOf([paneOf(richOf({ block: "meter", stages: [{ name: "stage", mark: pick(on, "done", "failed"), look: "good" }] }))])),
  "MeterStage.look": (on) => sceneOf(screenOf([paneOf(richOf({ block: "meter", stages: [{ name: "stage", mark: "done", look: pick(on, "status.success", "status.error") }] }))])),

  // RichBlock and what it holds
  "RichBlock.para.text": rich((on) => para([{ text: pick(on, "para", "ZZPARA") }])),
  "RichBlock.heading.level": rich((on) => ({ t: "heading", level: on ? 3 : 1, text: [{ text: "headtext" }] })),
  "RichBlock.heading.text": rich((on) => ({ t: "heading", level: 1, text: [{ text: pick(on, "headtext", "ZZHEAD") }] })),
  "RichBlock.list.items": rich((on) => ({ t: "list", items: on ? [{ marker: "bullet", blocks: [para([{ text: "ZZITEM" }])] }] : [] })),
  "RichBlock.quote.blocks": rich((on) => ({ t: "quote", blocks: on ? [para([{ text: "ZZQUOTE" }])] : [] })),
  "RichBlock.callout.tone": rich((on) => ({ t: "callout", tone: pick(on, "plain", "bad"), blocks: [para([{ text: "callout" }])] })),
  "RichBlock.callout.symbol": rich((on) => ({ t: "callout", tone: "plain", symbol: on ? "Z" : undefined, blocks: [para([{ text: "callout" }])] })),
  "RichBlock.callout.blocks": rich((on) => ({ t: "callout", tone: "plain", blocks: on ? [para([{ text: "ZZCALLOUT" }])] : [] })),
  "RichBlock.code.text": rich((on) => ({ t: "code", text: pick(on, "let x = 1;", "ZZCODE") })),
  "RichBlock.table.aligns": rich((on) => ({ t: "table", aligns: on ? ["right", "right"] : undefined, head: [[{ text: "HEADERLONGA" }], [{ text: "HEADERLONGB" }]], rows: [[[{ text: "ca" }], [{ text: "cb" }]]] })),
  "RichBlock.table.head": rich((on) => ({ t: "table", head: on ? [[{ text: "ZZHEAD" }]] : undefined, rows: [[[{ text: "ca" }]]] })),
  "RichBlock.table.rows": rich((on) => ({ t: "table", head: [[{ text: "HA" }]], rows: on ? [[[{ text: "ZZROW" }]]] : [] })),
  "RichBlock.details.summary": rich((on) => ({ t: "details", summary: [{ text: pick(on, "summ", "ZZSUMM") }], blocks: [] })),
  "RichBlock.details.blocks": rich((on) => ({ t: "details", summary: [{ text: "summ" }], blocks: on ? [para([{ text: "ZZDETAILS" }])] : [] })),
  "Media.image": rich((on) => ({ t: "media", image: on, name: "pic", src: "/x.png", url: "https://example.test/p" })),
  "Media.name": rich((on) => ({ t: "media", image: false, name: pick(on, "pic", "ZZNAME") })),
  "Media.url": rich((on) => ({ t: "media", image: false, name: "pic", url: on ? "https://example.test/ZZURL" : undefined })),
  "Media.src": rich((on) => ({ t: "media", image: false, name: "pic", src: on ? "/ZZSRC.png" : undefined })),
  "Item.marker": rich((on) => ({ t: "list", items: [{ marker: pick<any>(on, "bullet", "done"), blocks: [para([{ text: "item" }])] }] })),
  "Item.blocks": rich((on) => ({ t: "list", items: [{ marker: "bullet", blocks: on ? [para([{ text: "ZZITEM" }])] : [] }] })),
  "Inline.text": inline((i, on) => (i.text = pick(on, "word", "ZZINLINE"))),
  "Inline.kind": inline((i, on) => ((i.text = ":tada:"), (i.kind = on ? "emoji" : "text"))),
  "Inline.marks": inline((i, on) => (i.marks = on ? { bold: true } : {})),
  "Marks.bold": inline((i, on) => (i.marks = { bold: on })),
  "Marks.italic": inline((i, on) => (i.marks = { italic: on })),
  "Marks.strike": inline((i, on) => (i.marks = { strike: on })),
  "Marks.underline": inline((i, on) => (i.marks = { underline: on })),
  "Marks.code": inline((i, on) => (i.marks = { code: on })),
  "InlineKind.link.url": inline((i, on) => (i.kind = { link: { url: pick(on, "https://example.test/a", "https://example.test/b") } })),
  "InlineKind.mention.me": inline((i, on) => (i.kind = { mention: { me: on } })),
  "InlineKind.badge.tone": inline((i, on) => (i.kind = { badge: { tone: pick(on, "plain", "bad") } })),

  // Seg
  "Seg.text": seg((s, on) => (s.text = pick(on, "segtext", "ZZSEG"))),
  "Seg.ink": seg((s, on) => (s.ink = pick(on, "plain", "bad"))),
  "Seg.strong": seg((s, on) => (s.strong = on)),
  "Seg.marked": seg((s, on) => (s.marked = on)),
  "Seg.key_hint": (on) => ({ ...sceneOf(screenOf([paneOf({ node: "document", lines: [t("refresh", { key_hint: on ? "r" : null })] })])), plainText: true, ...(on ? { needle: "refresh [r]" } : {}) }),
  "Seg.icon": seg((s, on) => (s.icon = on ? "settings" : null)),
  "Seg.icon_lead": seg((s, on) => {
    s.icon = "icons";
    s.icon_lead = on;
  }),
  "Seg.link": seg((s, on) => (s.link = on ? "https://example.test/x" : null)),
  "Seg.look": seg((s, on) => (s.look = on ? "status.error" : null)),
  "Seg.chip": seg((s, on) => (s.chip = on)),

  // What the session sends outside a snapshot
  "Question.menu": (on) => ({
    chrome: chromeOf(W, H),
    screen: screenOf([paneOf(doc())]),
    extra: on ? [{ type: "question", id: 1, cursor: 1, menu: { id: "q", title: "ZZQUESTION", question: true, items: [{ value: "y", label: "yes", hint: "", key: "y", target: "chrome:menu:0" }], selected: 0 } }] : [],
  }),
  "InputRequest.title": (on) => ({ needle: "ZZTITLE", chrome: chromeOf(W, H), screen: screenOf([paneOf(doc())]), extra: [{ type: "input_request", id: 1, cursor: 1, title: pick(on, "branch", "ZZTITLE"), value: "v" }] }),
  "InputRequest.value": (on) => ({ needle: "ZZVALUE", chrome: chromeOf(W, H), screen: screenOf([paneOf(doc())]), extra: [{ type: "input_request", id: 1, cursor: 1, title: "branch", value: pick(on, "v", "ZZVALUE") }] }),
  "Notice.text": (on) => ({ chrome: chromeOf(W, H), screen: screenOf([paneOf(doc())]), extra: on ? [{ type: "notice", cursor: 1, text: "ZZNOTICE" }] : [] }),
};

/** Fields that carry no drawing, each with its reason. */
export const SCREEN_INVISIBLE: Record<string, string> = {
  "Node.state.state": "the reference draws every state as widgets::empty (the icon and the words, muted); a loading one also marks its panel for the shimmer (motion/node-marks.ts, test/motion/fixtures.test.ts), which is no cell of a settled frame",
  "TreeRow.glow": "the reference marks the row for the change glow (motion::row), an effect over a settled frame (motion/node-marks.ts, test/motion/fixtures.test.ts)",
  "TreeRow.heading": "a heading row has no press target and the host never selects it; its cells are drawn as any row's (ui::tree has no heading look); nodes/hits.tsx, test/nodes/hits.test.tsx",
  "RichBlock.code.lang": "ui::rich lays a code block out without a language label or colours (ui/rich.rs `code`); the reference and Ink both draw its lines alone",
  "Node.diff.language": "the host highlights by it and sends `painted` (Node.diff.painted, drawn); Ink has no grammars",
  "InlineKind.image.src": "ui::rich draws an inline image as its words, muted, and fetches src only for a picture, which comes after the old Rust renderer's removal (#14)",
  "ScreenView.version": "view contract version",
  "ScreenView.scope": "the screen's id; the chrome's status chip carries the name shown",
  "ScreenView.actions": "the actions menu is the host's MenuView; these rows only name the keys (input tests)",
  "SectionView.version": "view contract version",
  "SectionView.actions": "the actions menu is the host's MenuView; these rows only name the keys (input tests)",
  "Media.size": "the reference's no-graphics media line (ui::rich) shows no size; drawing one would break parity",
  "ActionSpec.id": "what Invoke answers with",
  "ActionSpec.icon": "the actions menu is the host's MenuView; ActionSpec rows are not drawn by the renderer",
  "ActionSpec.change": "the actions menu is the host's MenuView; ActionSpec rows are not drawn by the renderer",
  "ActionSpec.pane": "the actions menu is the host's MenuView; ActionSpec rows are not drawn by the renderer (its hint and place arrive as Choice.hint and the menu's heading rows)",
  "ActionSpec.hint": "the actions menu is the host's MenuView; ActionSpec rows are not drawn by the renderer (its hint and place arrive as Choice.hint and the menu's heading rows)",
  "ActionSpec.group": "the actions menu is the host's MenuView; ActionSpec rows are not drawn by the renderer (its hint and place arrive as Choice.hint and the menu's heading rows)",
  "ActionSpec.lead": "the actions menu is the host's MenuView; ActionSpec rows are not drawn by the renderer (its hint and place arrive as Choice.hint and the menu's heading rows)",
  "Chord.key": "a key a target presses; sent back, never drawn",
  "Chord.ctrl": "a key a target presses; sent back, never drawn",
  "Chord.alt": "a key a target presses; sent back, never drawn",
  "Chord.shift": "wire/input modifier for navigation ownership, not rendered text; shifted tree routing is covered by input/keys/tree-contract and burst",
  "Key.0.char": "a key a target presses; sent back, never drawn",
  "Marker.1.number": "drawn through Item.marker (a numbered marker is a list's numbering)",
  "CellAt.column": "where a context click opened the menu: MenuView.at covers it",
  "CellAt.row": "where a context click opened the menu: MenuView.at covers it",
  "TableRow.target": "pointer target",
  "CellView.target": "pointer target",
  "EntryView.target": "pointer target",
  "ListRow.target": "pointer target",
  "TreeRow.id": "identity for the host (fold and select targets)",
  "Tree.header_target": "pointer target",
  "Tree.pointer_action": "what a press on a row does (Select or Activate); no drawing",
  "SubjectRef.kind": "the subject a Show intent names; sent back, never drawn",
  "SubjectRef.id": "the subject a Show intent names; sent back, never drawn",
  "SubjectRef.label": "the subject a Show intent names; sent back, never drawn",
  "SubjectRef.url": "the subject a Show intent names; sent back, never drawn",
  "SubjectRef.fields": "the subject a Show intent names; sent back, never drawn",
  "Seg.target": "pointer target",
  "Question.id": "answered with Invoke; identity only",
  "Question.cursor": "stamps the snapshot the question was asked at",
  "InputRequest.id": "identity for the answer",
  "InputRequest.cursor": "stamps the snapshot the request was made at",
  "Notice.cursor": "stamps the snapshot the notice was sent at",
  "Block.line.target": "pointer target",
  "CellView.show": "the subject a press on the cell Shows; sent back, never drawn",
  "Node.extension.version": "an unknown extension draws its fallback; version and data are for the capabilities the renderer knows (work.*)",
  "Node.extension.data": "an unknown extension draws its fallback; version and data are for the capabilities the renderer knows (work.*)",
};

/**
 * Fields the renderer does not draw yet (live defects the contract found): the field and why. Each case asserts the
 * frame is still unchanged, so the entry goes red, saying "now drawn", when fixed.
 */
export const SCREEN_KNOWN_UNDRAWN: Record<string, string> = {
  "Node.rich.pictures": "pictures come after the old Rust renderer's removal (#14): a picture needs the picture protocol (media/kitty.ts), and the host's bytes",
  "Media.src": "pictures come after the old Rust renderer's removal (#14): src is what a picture fetches; without one the line `▣ image <name>  <url>` is drawn from name and url",
};
