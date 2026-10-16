// The cases for the fields of what a screen, an item page and a settings section hold (the chrome's are in
// every-field-is-drawn.test.tsx). A case builds the same scene with a field off and on; `on` must draw
// differently. Keys are `Label.field` as support.tsx's viewFields() names them, walked from the generated types.
import { chromeOf } from "./support.js";

export const W = 160;
export const H = 40;

export type Scene = { chrome: any; screen?: any; section?: any; size?: [number, number]; extra?: any[]; needle?: string };
export type Case = (on: boolean) => Scene;

const t = (s: string, over: Record<string, unknown> = {}) => [{ text: s, ...over }];
const pick = <T,>(on: boolean, off: T, yes: T): T => (on ? yes : off);

const screenOf = (panes: any[], over: Record<string, unknown> = {}) => ({ id: "n", scope: "n", panes, actions: [], ...over });
const paneOf = (body: any, over: Record<string, unknown> = {}) => ({ pane: "middle", title: "PANE", body, ...over });
const sceneOf = (screen: any, size?: [number, number]): Scene => ({ chrome: chromeOf(size?.[0] ?? W, size?.[1] ?? H), screen, size });
const doc = (...lines: string[]) => ({ node: "document", lines: (lines.length ? lines : ["BODYLINE"]).map((l) => t(l)) });

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
/** A tree row in a room wide enough for chips (the renderer shows them from 100 cells). */
const treeWide = (f: (r: any, on: boolean) => void, needle?: string): Case => (on) => {
  const n = treeBase();
  f(n.rows[1], on);
  return { ...sceneOf(screenOf([paneOf(n)]), [240, 40]), needle };
};
const treeRow = (f: (r: any, on: boolean) => void): Case => tree((n, on) => f(n.rows[1], on));
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
  "ScreenView.page": (on) => sceneOf(screenOf([paneOf(doc("UNDERPAGE"))], { page: on ? pageBase() : null })),
  "ScreenView.overlay": (on) => sceneOf(screenOf([paneOf(doc("BODYUNDER"))], { overlay: on ? paneOf(doc("OVERLAYTEXT"), { title: "OVERLAY" }) : null })),

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
  "PaneView.from": (on) => sceneOf(screenOf([paneOf(doc("NAVPANE"), { pane: "nav", title: "NAV", from: on ? 150 : 60 }), paneOf(doc("MIDPANE"))]), [100, 30]),
  "PaneView.notes_most": pane((p, on) => {
    p.notes = ["n1", "n2", "n3", "n4", "n5"].map((x) => t(`NOTE${x}`));
    p.notes_most = on ? 1 : null;
  }),
  "PaneView.inset": pane((p, on) => (p.inset = on)),
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
  "ListRow.heading": listRow((r, on) => (r.heading = on)),

  // Node.tree and what it holds
  "Tree.header": tree((n, on) => (n.header = on ? t("ZZHEADER") : [])),
  "Tree.columns": tree((n, on) => {
    n.columns = on ? [{ heading: "COLX", width: 10, keep: 0 }] : [];
    n.rows[1].cells = [{ heading: "COLX", text: t("cellx") }];
  }),
  "Tree.ordered_columns": treeNarrow((n, on) => {
    n.columns = [{ heading: "COLX", width: 10, keep: 0 }, { heading: "COLY", width: 10, keep: 0 }];
    n.rows[1].cells = [{ heading: "COLX", text: t("cellx") }, { heading: "COLY", text: t("celly") }];
    n.ordered_columns = on;
  }),
  "Tree.rows": tree((n, on) => (n.rows = on ? n.rows : [])),
  "Tree.selected": tree((n, on) => (n.selected = on ? ["b"] : ["a"])),
  "Tree.open": tree((n, on) => (n.open = on ? [["a"]] : [])),
  "Tree.name_width": treeNarrow((n, on) => {
    n.columns = [{ heading: "COLX", width: 30, keep: 0 }];
    n.rows[1].cells = [{ heading: "COLX", text: t("cellx") }];
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
  "TreeLook.lines": tree((n, on) => (n.look = { lines: pick(on, "plain", "dots"), band: "x", heading: "y", band_level: 1 })),
  "TreeLook.band": tree((n, on) => (n.look = { lines: "plain", band: pick(on, "x", "accent"), heading: "y", band_level: 1 })),
  "TreeLook.heading": tree((n, on) => {
    n.rows[1].heading = true;
    n.look = { lines: "plain", band: "x", heading: pick(on, "y", "accent"), band_level: 1 };
  }),
  "TreeLook.band_level": tree((n, on) => (n.look = { lines: "plain", band: "accent", heading: "y", band_level: on ? 1 : 0 })),
  "TreeRow.name": treeRow((r, on) => (r.name = t(pick(on, "bravo", "ZZNAME")))),
  "TreeRow.heading": treeRow((r, on) => (r.heading = on)),
  "TreeRow.cells": tree((n, on) => {
    n.columns = [{ heading: "COLX", width: 10, keep: 0 }];
    n.rows[1].cells = on ? [{ heading: "COLX", text: t("ZZCELL") }] : [];
  }),
  "TreeRow.children": tree((n, on) => {
    n.open = [["b"]];
    n.rows[1].children = on ? [{ id: "b1", name: t("ZZCHILD") }] : [];
  }),
  "TreeRow.stripe": treeRow((r, on) => (r.stripe = on ? "accent" : null)),
  "TreeRow.chips": treeWide((r, on) => (r.chips = on ? { gives: 4, text: t("ZZCHIP") } : null), "ZZCHIP"),
  "TreeRow.glow": treeRow((r, on) => (r.glow = on ? "accent" : null)),
  "TreeRow.detail": treeRow((r, on) => (r.detail = on ? t("ZZDETAIL") : null)),
  "Chips.gives": treeWide((r, on) => (r.chips = { gives: on ? 12 : 4, text: t("chiptext") })),
  "Chips.text": treeWide((r, on) => (r.chips = { gives: 8, text: t(pick(on, "chiptext", "ZZCHIP")) }), "ZZCHIP"),
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
  "ColumnSpec.extra": treeNarrow((n, on) => {
    n.columns = [{ heading: "COLX", width: 12, keep: 0 }, { heading: "COLY", width: 12, keep: 0 }, { heading: "COLZ", width: 12, keep: 0, extra: on }];
    n.rows[1].cells = [{ heading: "COLX", text: t("cellx") }, { heading: "COLY", text: t("celly") }, { heading: "COLZ", text: t("cellz") }];
    n.name_width = 20;
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
  "Node.log.top": log((n, on) => {
    n.lines = [0, 1, 2].map((i) => ({ text: t(`LOG${i}`), row: i, selected: false }));
    n.total = 3;
    n.top = on ? 2 : 0;
  }),
  "Node.log.total": log((n, on) => (n.total = on ? 300 : 3)),
  "Node.log.empty": log((n, on) => {
    n.lines = [];
    n.empty = pick(on, "NOLOG", "ZZEMPTY");
  }),
  "LogPaintLine.text": log((n, on) => (n.lines[0].text = t(pick(on, "LOGONE", "ZZLOG")))),
  "LogPaintLine.row": log((n, on) => (n.lines[0].row = on ? 5 : 0)),
  "LogPaintLine.selected": log((n, on) => (n.lines[0].selected = on)),
  "Node.diff.lines": diff((n, on) => (n.lines = on ? n.lines : [])),
  "Node.diff.under": diff((n, on) => (n.under = on ? [{ line: 1, lines: [t("ZZUNDER")] }] : [])),
  "Node.diff.selected": diff((n, on) => (n.selected = on ? 2 : 0)),
  "Node.diff.top": diff((n, on) => (n.top = on ? 1 : 0)),
  "Node.diff.split": diff((n, on) => (n.split = on)),
  "Node.diff.language": diff((n, on) => (n.language = on ? "rust" : null)),
  "Node.diff.painted": diff((n, on) => (n.painted = on ? n.lines.map((l: any) => [{ text: String(l.text ?? ""), token: "keyword" }]) : [])),
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
  "DiffUnder.input": diff((n, on) => (n.under = [{ line: 1, lines: [], input: on ? { text: "ZZINPUT", caret: 1 } : null }])),
  "InlineInput.text": diff((n, on) => (n.under = [{ line: 1, lines: [], input: { text: pick(on, "input", "ZZINPUT"), caret: 1 } }])),
  "InlineInput.caret": diff((n, on) => (n.under = [{ line: 1, lines: [], input: { text: "input", caret: on ? 4 : 0 } }])),
  "InlineInput.selection": diff((n, on) => (n.under = [{ line: 1, lines: [], input: { text: "input", caret: 2, selection: on ? [0, 3] : null } }])),
  "Node.state.state": (on) => sceneOf(screenOf([paneOf({ node: "state", state: pick(on, "empty", "failed"), text: "statetext" })])),
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
  "MeterStage.look": (on) => sceneOf(screenOf([paneOf(richOf({ block: "meter", stages: [{ name: "stage", mark: "done", look: pick(on, "good", "bad") }] }))])),

  // RichBlock and what it holds
  "RichBlock.para.text": rich((on) => para([{ text: pick(on, "para", "ZZPARA") }])),
  "RichBlock.heading.level": rich((on) => ({ t: "heading", level: on ? 3 : 1, text: [{ text: "headtext" }] })),
  "RichBlock.heading.text": rich((on) => ({ t: "heading", level: 1, text: [{ text: pick(on, "headtext", "ZZHEAD") }] })),
  "RichBlock.list.items": rich((on) => ({ t: "list", items: on ? [{ marker: "bullet", blocks: [para([{ text: "ZZITEM" }])] }] : [] })),
  "RichBlock.quote.blocks": rich((on) => ({ t: "quote", blocks: on ? [para([{ text: "ZZQUOTE" }])] : [] })),
  "RichBlock.callout.tone": rich((on) => ({ t: "callout", tone: pick(on, "plain", "bad"), blocks: [para([{ text: "callout" }])] })),
  "RichBlock.callout.symbol": rich((on) => ({ t: "callout", tone: "plain", symbol: on ? "Z" : undefined, blocks: [para([{ text: "callout" }])] })),
  "RichBlock.callout.blocks": rich((on) => ({ t: "callout", tone: "plain", blocks: on ? [para([{ text: "ZZCALLOUT" }])] : [] })),
  "RichBlock.code.lang": rich((on) => ({ t: "code", lang: on ? "rust" : undefined, text: "let x = 1;" })),
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
  "InlineKind.image.src": inline((i, on) => (i.kind = { image: { src: pick(on, "/a.png", "/b.png") } })),

  // Seg
  "Seg.text": seg((s, on) => (s.text = pick(on, "segtext", "ZZSEG"))),
  "Seg.ink": seg((s, on) => (s.ink = pick(on, "plain", "bad"))),
  "Seg.strong": seg((s, on) => (s.strong = on)),
  "Seg.marked": seg((s, on) => (s.marked = on)),
  "Seg.icon": seg((s, on) => (s.icon = on ? "settings" : null)),
  "Seg.icon_lead": seg((s, on) => {
    s.icon = "icons";
    s.icon_lead = on;
  }),
  "Seg.link": seg((s, on) => (s.link = on ? "https://example.test/x" : null)),
  "Seg.look": seg((s, on) => (s.look = on ? "bad" : null)),
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
  "ScreenView.version": "view contract version",
  "ScreenView.scope": "the screen's id; the chrome's status chip carries the name shown",
  "ScreenView.actions": "the actions menu is the host's MenuView; these rows only name the keys (input tests)",
  "SectionView.version": "view contract version",
  "SectionView.actions": "the actions menu is the host's MenuView; these rows only name the keys (input tests)",
  "Media.size": "Ratatui's no-graphics media line (ui::rich) shows no size; drawing one would break parity",
  "ActionSpec.id": "what Invoke answers with",
  "ActionSpec.label": "the actions menu is the host's MenuView; ActionSpec rows are not drawn by the renderer",
  "ActionSpec.key": "the actions menu is the host's MenuView; ActionSpec rows are not drawn by the renderer",
  "ActionSpec.heading": "the actions menu is the host's MenuView; ActionSpec rows are not drawn by the renderer",
  "ActionSpec.icon": "the actions menu is the host's MenuView; ActionSpec rows are not drawn by the renderer",
  "ActionSpec.change": "the actions menu is the host's MenuView; ActionSpec rows are not drawn by the renderer",
  "Chord.key": "a key a target presses; sent back, never drawn",
  "Chord.ctrl": "a key a target presses; sent back, never drawn",
  "Chord.alt": "a key a target presses; sent back, never drawn",
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
 * Fields the renderer does not draw yet (live defects the contract found): the field, why, and the owning
 * task. Each case asserts the frame is still unchanged, so the entry goes red, saying "now drawn", when fixed.
 * T11 owns ink/src/nodes, ink/src/text and ink/src/app/Panes.tsx; T12 ink/src/workarea and ink/src/media.
 */
export const SCREEN_KNOWN_UNDRAWN: Record<string, string> = {
  "Node.state.state": "T11: a state node looks the same whether empty or failed (it differed at e24c123: regressed; nodes/index.tsx state)",
  "Tree.ordered_columns": "T11: no frame difference with columns declared in order, in a 50-column room (nodes/columns.ts); T11 to confirm or fix",
  "Tree.name_width": "T11: no frame difference when name_width forces a column to drop, in a 50-column room (nodes/tree.tsx); T11 to confirm or fix",
  "ColumnSpec.extra": "T11: no frame difference when a column is marked extra, in a 50-column room (nodes/columns.ts); T11 to confirm or fix",
  "ListRow.heading": "T11: a list row's heading look is not drawn (it was, at e24c123: regressed; nodes/lists.tsx)",
  "TreeRow.heading": "T11: a tree row's heading look is not drawn (it was, at e24c123: regressed; nodes/tree.tsx)",
  "Node.diff.top": "T11: a diff's scroll offset no longer changes the frame (it did at e24c123: regressed; text/DiffView.tsx)",
  "DiffUnder.input": "T11: an inline input under a diff line is not drawn (it was at e24c123: regressed; text/DiffView.tsx)",
  "InlineInput.text": "T11: an inline input under a diff line is not drawn (it was at e24c123: regressed; text/DiffView.tsx)",
  "InlineInput.caret": "T11: an inline input under a diff line is not drawn (it was at e24c123: regressed; text/DiffView.tsx)",
  "InlineInput.selection": "T8: an inline input under a diff line is not drawn, so neither is its selection (text/DiffView.tsx)",
  "Seg.marked": "T11: a marked segment is not drawn differently (it was at e24c123: regressed; text/TextLine.tsx)",
  "Seg.link": "T11: a link segment is not drawn differently (underline, it was at e24c123: regressed; text/TextLine.tsx)",
  "Seg.look": "T11: a segment's look is not applied (it was at e24c123: regressed; text/TextLine.tsx)",
  "TreeRow.chips": "T11: a row's chips text never appears, at 160 or 240 columns (nodes/tree.tsx)",
  "Chips.text": "T11: a row's chips text never appears, at 160 or 240 columns (nodes/tree.tsx)",
  "PaneView.from": "T11: a pane's `from` (the width it shows from) is not applied (app/Panes.tsx)",
  "TreeEditing.at": "T11: an inline edit's start column is not applied (nodes/tree.tsx)",
  "ScreenView.overlay": "T11: an editor overlay over the foot of the body is never drawn (App.tsx does not read screen.overlay)",
  "ListRow.link": "T11: a list row's link is not drawn (nodes/lists.tsx)",
  "TreeLook.lines": "T11: the tree's look (TreeLook) is not read by nodes/tree.tsx",
  "TreeLook.band": "T11: the tree's look (TreeLook) is not read by nodes/tree.tsx",
  "TreeLook.heading": "T11: the tree's look (TreeLook) is not read by nodes/tree.tsx",
  "TreeLook.band_level": "T11: the tree's look (TreeLook) is not read by nodes/tree.tsx",
  "TreeRow.glow": "T11: a tree row's glow is not drawn (nodes/tree.tsx)",
  "Chips.gives": "T11: a row's chips are drawn but never sized by how many cells they give up (nodes/tree.tsx)",
  "Node.log.top": "T11: a log's scroll offset is not drawn (nodes/index.tsx log)",
  "LogPaintLine.row": "T11: a log line's row number is not drawn",
  "Node.diff.language": "T11: a diff's language (highlighting) is not applied (text/DiffView.tsx)",
  "Node.rich.pictures": "T12: a rich node's picture protocol is not read (media is capability-graded elsewhere)",
  "Block.doc.most": "T11: a doc block's line cap is not applied (text/RichDoc.tsx)",
  "Block.doc.urls": "T11: a doc block's urls flag is not applied (text/RichDoc.tsx)",
  "Block.wrapped.ink": "T11: a wrapped block's ink slot is not applied (nodes/index.tsx)",
  "MeterStage.look": "T11: a meter stage's look is not applied (nodes/index.tsx)",
  "Media.src": "T12: a media item's source is not drawn unless the image path takes it (media/MediaSurface.tsx)",
  "InlineKind.link.url": "T11: an inline link's address is not shown (underline only) (text/RichDoc.tsx)",
  "InlineKind.image.src": "T11: an inline image's source is not shown (text/RichDoc.tsx)",
};
