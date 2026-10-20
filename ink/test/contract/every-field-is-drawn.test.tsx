// Class A: every field the host can send is drawn, or is listed here with the reason it is not.
//
// The fields are read from the generated protocol types (src/protocol/generated/*.ts), not from a list
// kept here: a field added on the Rust side appears in `fieldsOf(...)`, has no case below, and the
// "every field has a case" test fails until someone writes a fixture for it (or lists it as invisible
// with a reason). A case renders the same scene twice, with the field off and on, and the frames must
// differ. A field the renderer reads nowhere (ChromeView.history, before it was drawn) fails its case.
//
// A field known to be undrawn today is in KNOWN_UNDRAWN with the name of the defect. Its case then
// asserts that nothing differs (`expect(on).toBe(off)`), so the suite is green now and goes red, with the
// message "now drawn: remove it from KNOWN_UNDRAWN", the day the renderer draws it.
// INTENTIONALLY_UNDRAWN also keeps a paired on/off case, but asserts a deliberate compatibility invariant rather
// than a live defect: restoring deprecated drawing would violate the shared visual contract.
import { describe, expect, it, vi } from "vitest";

// Colour on, so a field that changes only a style (an ink slot, bold, inverse) changes the frame.
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3";
});

import { applyLook, defaultLook } from "../../src/text/look.js";
import { Motion } from "../../src/motion/motion.js";
import { noticePresentation } from "../../src/notices/paint.js";
import { NOTICE_CASES, NOTICE_INVISIBLE } from "./cases-notices.js";
import { chromeOf, fieldsOf, mount, nodeKinds, plain, unionOf, viewFields } from "./support.js";
import { SCREEN_CASES, SCREEN_INVISIBLE, SCREEN_KNOWN_UNDRAWN } from "./cases-screen.js";
import { DOCK_CASES, DOCK_INVISIBLE, DOCK_KNOWN_UNDRAWN } from "./cases-dock.js";

// Every frame is made in an app of its own and stops it, so the cases run concurrently.

const W = 160;
const H = 40;

// Links as the setup in use writes them (Ghostty: OSC 8 on), so a link is part of the frame (Seg.link, ListRow.link).
applyLook({ ...defaultLook(), hyperlinks: true });

import type { Scene, Case } from "./cases-screen.js";

const text = (t: string, over: Record<string, unknown> = {}) => [{ text: t, ...over }];
const docScreen = (lines = ["BODYLINE"]) => ({
  id: "doc",
  scope: "doc",
  panes: [{ pane: "middle", title: "doc", body: { node: "document", lines: lines.map((l) => text(l)) } }],
  actions: [],
});
/** Footer scenes exercise actual frame paint rather than declaring new fields invisible. */
const footer = (change: (foot: any, on: boolean) => void): Case => (on) => {
  const foot: any = {
    actions: [{ action: { id: "watch.approve", label: "approve", change: "edit", key: { key: { char: "a" } }, mnemonic_at: 0 }, target: "detail-foot:1:0" }],
    menu: { action: { id: "detail.extra_actions", label: "extra actions", key: { key: { char: "x" } }, mnemonic_at: 1 }, target: "detail-foot:1:menu" },
  };
  change(foot, on);
  return { chrome: chromeOf(W, H, { write_mode: "edit" }), screen: { ...docScreen(), panes: [{ pane: "detail", title: "Watch report", body: { node: "document", lines: [text("BODYLINE")] }, foot_actions: foot }] } };
};
const section = { version: 1, scope: "connection", body: { node: "document", lines: [text("SECTIONBODY")] }, detail_title: "detail", detail: [text("DETAILBODY")] };

// ---- the scenes, one builder per object the chrome holds ------------------------------------------------

const bar = (f: (b: any, on: boolean) => void): Case => (on) => {
  const c = chromeOf(W, H);
  f(c.top_bar, on);
  return { chrome: c, screen: docScreen() };
};
const tab = (f: (t: any, on: boolean) => void): Case => bar((b, on) => f(b.tabs[1], on));
const status = (f: (s: any, on: boolean) => void): Case => (on) => {
  const c = chromeOf(W, H);
  f(c.status, on);
  return { chrome: c, screen: docScreen() };
};
const hint = (f: (h: any, on: boolean) => void): Case => status((s, on) => f(s.hints[0], on));
const sources = (f: (s: any, on: boolean) => void): Case => (on) => {
  const c = chromeOf(W, H, {
    sources: [
      { label: "GitLab", active: false, target: "chrome:source:0" },
      { label: "GitHub", target: "chrome:source:1" },
    ],
  });
  f(c.sources[0], on);
  return { chrome: c, screen: docScreen() };
};
const menuBase = (): any => ({
  id: "actions",
  title: "Pick one",
  question: false,
  items: [
    { value: "a", label: "alpha", hint: "", key: "a", target: "chrome:menu:0" },
    { value: "b", label: "beta", hint: "", key: "b", target: "chrome:menu:1" },
  ],
  selected: 0,
});
const menu = (f: (m: any, on: boolean) => void, question = false): Case => (on) => {
  const m = menuBase();
  m.question = question;
  f(m, on);
  return { chrome: chromeOf(W, H, { menu: m }), screen: docScreen() };
};
const choice = (f: (c: any, on: boolean) => void): Case => menu((m, on) => f(m.items[1], on));
const nav = (f: (n: any, on: boolean) => void): Case => (on) => {
  const n: any = {
    groups: [
      { id: "gitlab", name: "GitLab", open: true, target: "chrome:group:gitlab", sections: [{ id: "connection", name: "connection", target: "chrome:section:0" }, { id: "views", name: "views", target: "chrome:section:1" }] },
      { id: "slack", name: "Slack", open: false, target: "chrome:group:slack", sections: [{ id: "alerts", name: "alerts", target: "chrome:section:2" }] },
    ],
    section_title: "connection",
    detail_title: "detail",
  };
  f(n, on);
  return { chrome: chromeOf(W, H, { settings: n }), section };
};
const group = (f: (g: any, on: boolean) => void): Case => nav((n, on) => f(n.groups[1], on));
const navSection = (f: (s: any, on: boolean) => void): Case => nav((n, on) => f(n.groups[0].sections[1], on));

const historyBase = (): any => ({
  version: 1,
  width: W,
  height: H - 1,
  title: "History · read only · UTC",
  boundary: "snapshot through #12",
  scope: "recent activity",
  notes: [],
  focus: "events",
  split: true,
  events_title: "events · page 1",
  columns: ["observed UTC", "action", "subject", "outcome"],
  rows: [
    { observed: "2026-10-12 10:00:01", action: "act.one", subject: "subj one", outcome: "requested", outcome_ink: "muted", target: "chrome:history:row:0" },
    { observed: "2026-10-12 10:00:02", action: "act.two", subject: "subj two", outcome: "requested", outcome_ink: "muted", target: "chrome:history:row:1" },
  ],
  selected: 0,
  list_scroll: 0,
  state: "",
  detail_title: "event detail",
  detail: [text("DETAIL ONE"), text("DETAIL TWO"), text("DETAIL THREE")],
  detail_scroll: 0,
  help_title: "history keys",
  help_lines: [text("HELP ONE"), text("HELP TWO"), text("HELP THREE")],
  help_scroll: 0,
  footer: [{ keys: "j/k", label: "move" }],
});
const history = (f: (h: any, on: boolean) => void): Case => (on) => {
  const h = historyBase();
  f(h, on);
  return { chrome: chromeOf(W, H, { history: h }), screen: docScreen() };
};
const historyRow = (f: (r: any, on: boolean) => void): Case => history((h, on) => f(h.rows[1], on));

const pick = <T,>(on: boolean, off: T, yes: T): T => (on ? yes : off);

// Setup: the chrome holds a title, a step and the card's heading and lead; the card and the sheet are the snapshot's
// section. The rail is never sent (the host's rail is always empty), so it is not drawn.
const setupBase = (): any => ({
  title: "step title",
  step: null,
  rail: [],
  current: null,
  heading: "head",
  lead: "lead",
});
const setup = (f: (s: any, on: boolean) => void): Case => (on) => {
  const s = setupBase();
  f(s, on);
  return { chrome: chromeOf(W, H, { setup: s }), section };
};

// The tour's card over the body (shell/tour.rs): a counter in its border, a title, lines, what to try, its keys.
const tourBase = (): any => ({ stop: 2, of: 5, title: "tour title", lines: [text("tour line")], try_it: "try it: press J", keys: text("next key") });
const tour = (f: (t: any, on: boolean) => void): Case => (on) => {
  const t = tourBase();
  f(t, on);
  return { chrome: chromeOf(W, H, { tour: t }), screen: docScreen() };
};
const healthChip = () => ({ part: "svc", label: "svc", state: "ok", ink: "good", target: "chrome:health:svc" });
const attentionChip = () => ({ text: "review needs you", icon: "warning", ink: "warn", keys: "`", target: "chrome:attention" });

// ---- the cases, by "Type.field" --------------------------------------------------------------------------

const CASES: Record<string, Case> = {
  // ChromeView
  "ChromeView.pane_jumps": (on) => ({ chrome: chromeOf(W, H, { pane_jumps: on ? [{ pane: "middle", key: "2" }] : [] }), screen: docScreen(), ...(on ? { needle: "2 doc" } : {}) }),
  "ChromeView.write_mode": (on) => {
    const scene = footer(() => {})(on);
    scene.chrome.write_mode = on ? "edit" : "read_only";
    return scene;
  },
  "PaneJump.pane": (on) => ({ chrome: chromeOf(W, H, { pane_jumps: [{ pane: on ? "middle" : "nav", key: "2" }] }), screen: docScreen() }),
  "PaneJump.key": (on) => ({ chrome: chromeOf(W, H, { pane_jumps: [{ pane: "middle", key: on ? "2" : "9" }] }), screen: docScreen() }),
  "PaneView.foot_actions": (on) => {
    const scene = footer(() => {})(on);
    if (!on) delete scene.screen.panes[0].foot_actions;
    return scene;
  },
  "FootActions.actions": footer((foot, on) => { if (!on) foot.actions = []; }),
  "FootActions.menu": footer((foot, on) => { foot.menu.action.label = on ? "extra actions" : "options"; }),
  "FootAction.action": footer((foot, on) => { foot.actions[0].action.label = on ? "approve" : "allow"; }),
  "FootAction.disabled": footer((foot, on) => { if (on) foot.actions[0].disabled = "waiting for review"; }),
  "ChromeView.top_bar": (on) => ({ chrome: chromeOf(W, H, { top_bar: { brand: on ? "ZZBRAND" : "", tabs: [], mode: { text: "", ink: "plain", target: "chrome:mode" } } }), screen: docScreen() }),
  "ChromeView.sources": (on) => ({ chrome: chromeOf(W, H, { sources: on ? [{ label: "ZZSRC", target: "chrome:source:0" }] : [] }), screen: docScreen() }),
  "ChromeView.status": (on) => ({ chrome: chromeOf(W, H, { status: { scope: on ? "ZZSTATUS" : "" } }), screen: docScreen() }),
  "ChromeView.menu": (on) => ({ chrome: chromeOf(W, H, { menu: on ? menuBase() : null }), screen: docScreen() }),
  "ChromeView.settings": (on) => ({ chrome: chromeOf(W, H, { settings: on ? { groups: [{ id: "g", name: "ZZGROUP", open: false, target: "chrome:group:g", sections: [] }], section_title: "s", detail_title: "d" } : null }), section }),
  "ChromeView.keys_at": (on) => ({ chrome: chromeOf(W, H, { keys_at: on ? "bar" : "body" }), screen: docScreen() }),
  "ChromeView.tour": (on) => ({ chrome: chromeOf(W, H, { tour: on ? tourBase() : null }), screen: docScreen() }),
  "ChromeView.setup": (on) => ({ chrome: chromeOf(W, H, { setup: on ? setupBase() : null }), section }),
  "ChromeView.history": (on) => ({ chrome: chromeOf(W, H, { history: on ? historyBase() : null }), screen: docScreen() }),

  // TopBar
  "TopBar.brand": bar((b, on) => (b.brand = pick(on, "radar", "ZZBRAND"))),
  "TopBar.tabs": bar((b, on) => (b.tabs = on ? b.tabs : [])),
  "TopBar.words": bar((b, on) => (b.words = on ? { text: "ZZWORDS", ink: "muted" } : null)),
  "TopBar.terminal": bar((b, on) => (b.terminal = on ? { text: "ZZTERM", ink: "muted" } : null)),
  "TopBar.busy": bar((b, on) => (b.busy = on)),
  "TopBar.mode": bar((b, on) => (b.mode = { ...b.mode, text: pick(on, "read-only", "ZZMODE") })),
  // BarTab
  "BarTab.label": tab((t, on) => (t.label = pick(on, "Work", "ZZLABEL"))),
  "BarTab.mnemonic": tab((t, on) => (t.mnemonic = pick(on, "W", "k"))),
  "BarTab.icon": tab((t, on) => (t.icon = on ? "settings" : null)),
  "BarTab.attention": tab((t, on) => (t.attention = on)),
  "BarTab.active": tab((t, on) => (t.active = on)),
  // Badge (the words, the terminal chip and the status counts all hold one)
  "Badge.text": bar((b, on) => (b.words = { text: pick(on, "5", "ZZ7"), ink: "warn" })),
  "Badge.ink": bar((b, on) => (b.words = { text: "5", ink: pick(on, "muted", "bad") })),
  // ModeChip
  "ModeChip.text": bar((b, on) => (b.mode.text = pick(on, "read-only", "ZZMODE"))),
  "ModeChip.ink": bar((b, on) => (b.mode.ink = pick(on, "warn", "good"))),
  "ModeChip.strong": bar((b, on) => (b.mode.strong = on)),
  "ModeChip.icon": bar((b, on) => (b.mode.icon = on ? "mode_edit" : null)),
  // SourceTab
  "SourceTab.label": sources((s, on) => (s.label = pick(on, "GitLab", "ZZSRC"))),
  "SourceTab.active": sources((s, on) => (s.active = on)),
  // StatusLine
  "StatusLine.scope": status((s, on) => (s.scope = pick(on, "BOOKS", "ZZSCOPE"))),
  "StatusLine.hints": status((s, on) => (s.hints = on ? s.hints : [])),
  "StatusLine.least_needed": (on) => {
    // Too many hints for the width: which ones give way is decided by `least_needed`.
    const c = chromeOf(40, 12);
    c.status.hints = ["alpha", "bravo", "charlie", "delta", "echo"].map((w, i) => ({ keys: `${i}`, label: w, keys_of: [] }));
    c.status.least_needed = on ? ["alpha", "bravo"] : [];
    return { chrome: c, screen: docScreen(), size: [40, 12] } as Scene;
  },
  "StatusLine.counts": status((s, on) => (s.counts = on ? [{ text: "ZZ9 open", ink: "muted" }] : [])),
  "StatusLine.message": status((s, on) => (s.message = on ? { text: "ZZMESSAGE", ink: "warn" } : null)),
  "StatusLine.health": status((s, on) => (s.health = on ? [healthChip()] : [])),
  "StatusLine.attention": status((s, on) => (s.attention = on ? attentionChip() : null)),
  "AttentionChip.text": status((s, on) => (s.attention = { ...attentionChip(), text: pick(on, "a needs you", "ZZ needs you") })),
  "AttentionChip.icon": status((s, on) => (s.attention = { ...attentionChip(), icon: pick(on, null, "warning") })),
  "AttentionChip.ink": status((s, on) => (s.attention = { ...attentionChip(), ink: pick(on, "warn", "good") })),
  "AttentionChip.keys": status((s, on) => (s.attention = { ...attentionChip(), keys: pick(on, "`", "~") })),
  // HealthChip
  "HealthChip.label": status((s, on) => (s.health = [{ ...healthChip(), label: pick(on, "svc", "ZZSVC") }])),
  "HealthChip.glyph": status((s, on) => (s.health = [{ ...healthChip(), glyph: pick(on, "", "#") }])),
  "HealthChip.ink": status((s, on) => (s.health = [{ ...healthChip(), ink: pick(on, "good", "bad") }])),
  "HealthChip.count": status((s, on) => (s.health = [{ ...healthChip(), count: on ? "1/4" : null }])),
  // Hint
  "Hint.keys": hint((h, on) => (h.keys = pick(on, "q", "ZZK"))),
  "Hint.label": hint((h, on) => (h.label = pick(on, "quit", "ZZLABEL"))),
  "Hint.icon": hint((h, on) => (h.icon = on ? "settings" : null)),
  "Hint.mnemonic_at": hint((h, on) => { h.keys = "q"; h.label = "quit"; h.mnemonic_at = on ? 0 : null; }),
  "Hint.mnemonic_shift": hint((h, on) => { h.keys = "Q"; h.label = "quit"; h.mnemonic_at = 0; h.mnemonic_shift = on; }),
  // Message
  "Message.text": status((s, on) => (s.message = { text: pick(on, "one", "ZZTWO"), ink: "warn" })),
  "Message.icon": status((s, on) => (s.message = { text: "one", icon: on ? "settings" : null, ink: "warn" })),
  "Message.ink": status((s, on) => (s.message = { text: "one", ink: pick(on, "warn", "bad") })),

  // MenuView
  "MenuView.palette": menu((m, on) => (m.palette = on ? { matched: 2, total: 8, highlights: [[0], [1]] } : null)),
  "PaletteView.matched": menu((m, on) => (m.palette = { matched: on ? 1 : 2, total: 8, highlights: [[], []] })),
  "PaletteView.total": menu((m, on) => (m.palette = { matched: 2, total: on ? 12 : 8, highlights: [[], []] })),
  "PaletteView.highlights": menu((m, on) => (m.palette = { matched: 2, total: 8, highlights: on ? [[0], [1]] : [[], []] })),
  "PaletteView.hint_highlights": menu((m, on) => {
    m.items[0]!.hint = "watch";
    m.palette = { matched: 2, total: 8, highlights: [[], []], hint_highlights: on ? [[0, 1], []] : [] };
  }),
  "MenuView.title": menu((m, on) => (m.title = pick(on, "Pick one", "ZZTITLE"))),
  "MenuView.icon": menu((m, on) => (m.icon = on ? "settings" : null)),
  "MenuView.question": menu((m, on) => (m.question = on)),
  "MenuView.items": menu((m, on) => (m.items = on ? m.items : [])),
  "MenuView.selected": menu((m, on) => (m.selected = on ? 1 : 0)),
  "MenuView.typed": menu((m, on) => (m.typed = on ? "ZZTYPED" : null)),
  "MenuView.caret": menu((m, on) => ((m.typed = "abcdef"), (m.caret = on ? 2 : 6))),
  "MenuView.at": menu((m, on) => (m.at = on ? { column: 3, row: 4 } : null)),
  "MenuView.scroll": menu((m, on) => {
    m.items = ["a1", "b2", "c3", "d4"].map((l, i) => ({ value: l, label: `row${l}`, hint: "", key: l, target: `chrome:menu:${i}` }));
    m.scroll = on ? 2 : 0;
  }),
  "MenuView.lines": menu((m, on) => ((m.lines = on ? 30 : 2), (m.shown = 2))),
  "MenuView.shown": menu((m, on) => ((m.lines = 30), (m.shown = on ? 2 : 30))),
  "MenuView.usable": menu((m, on) => (m.usable = on)),
  "MenuView.refused": menu((m, on) => ((m.typed = "x"), (m.refused = on ? "ZZREFUSED" : null))),
  // Choice
  "Choice.label": choice((c, on) => (c.label = pick(on, "beta", "ZZLABEL"))),
  "Choice.hint": choice((c, on) => (c.hint = on ? "ZZHINT" : "")),
  "Choice.key": choice((c, on) => (c.key = on ? "z" : null)),
  "Choice.keys": choice((c, on) => ((c.key = null), (c.keys = on ? "ctrl-z" : ""))),
  "Choice.icon": choice((c, on) => (c.icon = on ? "settings" : null)),
  "Choice.heading": choice((c, on) => (c.heading = on)),
  "Choice.note": choice((c, on) => (c.note = on)),
  "Choice.sample": choice((c, on) => (c.sample = on ? "ZZSAMPLE" : null)),
  "Choice.shown_only": choice((c, on) => (c.shown_only = on)),
  "Choice.muted": choice((c, on) => (c.muted = on)),
  "Choice.mnemonic_at": choice((c, on) => (c.mnemonic_at = on ? 0 : null)),
  "Choice.mnemonic_shift": choice((c, on) => { c.key = "B"; c.mnemonic_at = 0; c.mnemonic_shift = on; }),

  // SettingsNav
  "SettingsNav.groups": nav((n, on) => (n.groups = on ? n.groups : [])),
  "SettingsNav.selected": nav((n, on) => (n.selected = on ? "chrome:section:1" : null)),
  "SettingsNav.section_title": (on) => {
    const s = nav((n, o) => (n.section_title = pick(o, "connection", "ZZSECTION")))(on);
    return s;
  },
  "SettingsNav.detail_title": nav((n, on) => (n.detail_title = pick(on, "detail", "ZZDETAIL"))),
  "SettingsNav.unsaved": nav((n, on) => (n.unsaved = on)),
  // NavGroup / NavSection
  "NavGroup.name": group((g, on) => (g.name = pick(on, "Slack", "ZZGROUP"))),
  "NavGroup.open": group((g, on) => (g.open = on)),
  "NavGroup.sections": group((g, on) => ((g.open = true), (g.sections = on ? g.sections : []))),
  "NavSection.name": navSection((s, on) => (s.name = pick(on, "views", "ZZSEC"))),
  "NavSection.icon": navSection((s, on) => (s.icon = on ? "settings" : null)),

  // SetupChrome
  "SetupChrome.title": setup((s, on) => (s.title = pick(on, "step title", "ZZTITLE"))),
  "SetupChrome.step": setup((s, on) => (s.step = on ? "ZZ step 9 of 9" : null)),
  "SetupChrome.heading": setup((s, on) => (s.heading = pick(on, "head", "ZZHEADING"))),
  "SetupChrome.lead": setup((s, on) => (s.lead = pick(on, "lead", "ZZLEAD"))),

  // TourCard
  "TourCard.stop": tour((t, on) => (t.stop = on ? 4 : 2)),
  "TourCard.of": tour((t, on) => (t.of = on ? 9 : 5)),
  "TourCard.title": tour((t, on) => (t.title = pick(on, "tour title", "ZZTITLE"))),
  "TourCard.lines": tour((t, on) => (t.lines = on ? t.lines : [])),
  "TourCard.try_it": tour((t, on) => (t.try_it = on ? "ZZ try it" : null)),
  "TourCard.keys": tour((t, on) => (t.keys = text(pick(on, "next key", "ZZKEYS")))),

  // HistoryView
  "HistoryView.title": history((h, on) => (h.title = pick(on, "History", "ZZTITLE"))),
  "HistoryView.boundary": history((h, on) => (h.boundary = pick(on, "snapshot", "ZZBOUND"))),
  "HistoryView.scope": history((h, on) => (h.scope = pick(on, "recent", "ZZSCOPE"))),
  "HistoryView.notes": history((h, on) => (h.notes = on ? [text("ZZNOTE")] : [])),
  "HistoryView.focus": history((h, on) => (h.focus = pick(on, "events", "detail"))),
  "HistoryView.help": history((h, on) => (h.help = on)),
  "HistoryView.split": history((h, on) => (h.split = on)),
  "HistoryView.events_title": history((h, on) => (h.events_title = pick(on, "events", "ZZEVENTS"))),
  "HistoryView.columns": history((h, on) => (h.columns = on ? h.columns : [])),
  "HistoryView.rows": history((h, on) => (h.rows = on ? h.rows : [])),
  "HistoryView.selected": history((h, on) => (h.selected = on ? 1 : 0)),
  "HistoryView.list_scroll": history((h, on) => {
    h.rows = Array.from({ length: 60 }, (_, i) => ({ observed: `t${i}`, action: `act.${i}`, subject: "s", outcome: "ok", outcome_ink: "muted", target: `chrome:history:row:${i}` }));
    // the reference's cursor is `selected.unwrap_or(0)`: with none selected the table snaps back to row 0.
    h.selected = 25;
    h.list_scroll = on ? 20 : 0;
  }),
  "HistoryView.error": history((h, on) => ((h.state = "ZZSTATE"), (h.error = on ? "unavailable" : null))),
  "HistoryView.state": history((h, on) => ((h.rows = []), (h.state = on ? "ZZSTATE" : ""))),
  "HistoryView.detail_title": history((h, on) => (h.detail_title = pick(on, "detail", "ZZDETAIL"))),
  "HistoryView.detail": history((h, on) => (h.detail = on ? h.detail : [])),
  "HistoryView.detail_scroll": history((h, on) => {
    // the reference clamps the scroll to the overflow: the detail must be taller than its pane for a scroll to show.
    h.detail = Array.from({ length: 60 }, (_, i) => text(`DETAIL LINE ${i}`));
    h.detail_scroll = on ? 20 : 0;
  }),
  "HistoryView.help_title": history((h, on) => ((h.help = true), (h.help_title = pick(on, "keys", "ZZHELP")))),
  "HistoryView.help_lines": history((h, on) => ((h.help = true), (h.help_lines = on ? h.help_lines : []))),
  "HistoryView.help_scroll": history((h, on) => {
    h.help = true;
    h.help_lines = Array.from({ length: 60 }, (_, i) => text(`HELP LINE ${i}`));
    h.help_scroll = on ? 20 : 0;
  }),
  "HistoryView.footer": history((h, on) => (h.footer = on ? h.footer : [])),
  // HistoryRow (the second row)
  "HistoryRow.observed": historyRow((r, on) => (r.observed = pick(on, "t1", "ZZOBS"))),
  "HistoryRow.action": historyRow((r, on) => (r.action = pick(on, "a1", "ZZACT"))),
  "HistoryRow.subject": historyRow((r, on) => (r.subject = pick(on, "s1", "ZZSUBJ"))),
  "HistoryRow.outcome": historyRow((r, on) => (r.outcome = pick(on, "o1", "ZZOUT"))),
  "HistoryRow.outcome_ink": historyRow((r, on) => (r.outcome_ink = pick(on, "muted", "bad"))),
};

/**
 * Fields that carry no drawing, each with its reason. A pointer target is what a click on the thing
 * sends back (the hit tests in test/chrome and test/input cover it); an id or a value is what the host
 * is answered with; a width or height is the room the host made the view for, while the renderer draws in
 * its own terminal's room.
 */
const INVISIBLE: Record<string, string> = {
  "FootAction.target": "opaque host-admitted pointer identity; exact complete control hit rectangles and disabled/noncontrol exclusion are checked in detail-native-cells and detail-foot",
  "ChromeView.version": "view contract version, checked at Hello/Welcome",
  "ChromeView.width": "the room the host made the view for; the renderer draws in its own terminal's room",
  "ChromeView.height": "the room the host made the view for; the renderer draws in its own terminal's room",
  "ChromeView.screen_keys": "the capitals the host takes, which the input controller leaves to it (input tests); never drawn",
  "MenuView.change": "the reference draws no change mark: a write question's title has no '(add)' (cc18832b); drawing one would break parity",
  "BarTab.target": "pointer target",
  "ModeChip.target": "pointer target",
  "SourceTab.target": "pointer target",
  "HealthChip.part": "which part's health menu the chip opens, for the host",
  "AttentionChip.target": "where a click on the chip goes (chrome:attention, the host opens the dock on that tab); test/dock/attention.test.ts checks the one target",
  "HealthChip.state": "what the ink says, for tests and the host; the ink is drawn",
  "HealthChip.target": "pointer target",
  "HintKey.label": "locates the key inside Hint.keys for its pointer target",
  "HintKey.chord": "the key the target presses; sent back, never drawn",
  "HintKey.action": "action id the key runs; sent back, never drawn",
  "HintKey.target": "pointer target",
  "Hint.keys_of": "only HintKey pointer targets",
  "MenuView.id": "which menu this is, for the host",
  "Choice.value": "what Invoke answers with",
  "Choice.target": "pointer target",
  "NavGroup.id": "identity for the host",
  "NavGroup.target": "pointer target",
  "NavSection.id": "identity for the host",
  "NavSection.target": "pointer target",
  "SetupChrome.rail": "the host always sends it empty (setup shows `step n of m` on the top bar); neither renderer draws a rail",
  "SetupChrome.current": "the index of a rail stop; there is no rail",
  "RailStop.id": "no rail is sent",
  "RailStop.label": "no rail is sent",
  "RailStop.icon": "no rail is sent",
  "RailStop.state": "no rail is sent",
  "RailStop.result": "no rail is sent",
  "RailStop.target": "no rail is sent",
  "HistoryView.version": "view contract version",
  "HistoryView.width": "the room the host made the view for",
  "HistoryView.height": "the room the host made the view for",
  "HistoryRow.target": "pointer target",
  "HistoryView.pending": "the reference paints `state` only when there are no rows or an error (history.rs render_events); pending alone draws nothing more",
  "HistoryView.detail_keys_line": "the keys are words of that detail line; they add pointer areas, not cells (test/chrome/history.test.ts)",
  "HistoryView.detail_keys": "the keys are words of that detail line; they add pointer areas, not cells (test/chrome/history.test.ts)",
};

/**
 * Fields the renderer does not draw yet: live defects the contract found. Each is asserted to stay
 * undrawn, so this list shrinks (the case fails with "now drawn") when the owning task fixes it. The
 * owner of the chrome is T10 (shell/chrome).
 */
const KNOWN_UNDRAWN: Record<string, string> = {
};

/** Compatibility fields that deliberately must not affect the frame, still covered by their paired cases. */
const INTENTIONALLY_UNDRAWN: Record<string, string> = {
  "TreeRow.stripe": "#45 retains this legacy wire field but removes root and descendant stripes in both renderers; semantic foregrounds and shared row-state grounds carry meaning instead",
};

// ---- the tests --------------------------------------------------------------------------------------------

// Every field of every object type reachable from what the host sends for a view, walked from the
// generated files' imports (support.tsx viewFields): nothing here is a list of types.
const allFields = viewFields();
const ALL_CASES: Record<string, Case> = { ...CASES, ...SCREEN_CASES, ...DOCK_CASES, ...NOTICE_CASES };
const ALL_KNOWN: Record<string, string> = { ...KNOWN_UNDRAWN, ...SCREEN_KNOWN_UNDRAWN, ...DOCK_KNOWN_UNDRAWN };
const ALL_INVISIBLE: Record<string, string> = { ...INVISIBLE, ...SCREEN_INVISIBLE, ...DOCK_INVISIBLE, ...NOTICE_INVISIBLE };

async function frameOf(scene: Scene): Promise<string> {
  const [w, h] = scene.size ?? [W, H];
  const m = mount(w, h, scene.plainText);
  m.snapshot(scene);
  for (const msg of scene.extra ?? []) m.conn.receive(msg);
  await m.settle();
  let f = m.out.last();
  // A local compositor avoids the process-global motion controller shared by concurrent contract cases.
  if (m.session.noticeStore.throughId > 0) {
    const layer = new Motion("off", () => 0);
    layer.setPresentation(noticePresentation(m.session, { columns: w, rows: h }, () => true));
    f = layer.apply(f, w, h); layer.reset();
  }
  m.stop();
  return f;
}

describe("the generated types are read", () => {
  it("finds ChromeView's fields in the generated file", () => {
    expect(fieldsOf("ChromeView")).toEqual(["version", "width", "height", "top_bar", "sources", "status", "menu", "settings", "keys_at", "history", "screen_keys", "setup", "tour", "notices", "notice_history", "notices_visible", "pane_jumps", "write_mode"]);
  });
});

describe("every field of what the host sends for the chrome is drawn", () => {
  it("every field has a case, or is invisible with a reason (a new field fails here until it has one)", () => {
    const missing = allFields.filter((f) => !(f in ALL_CASES) && !(f in ALL_INVISIBLE));
    expect(missing.join(", "), "add a case in CASES for each, or a reason in INVISIBLE").toBe("");
    const stale = [...Object.keys(ALL_CASES), ...Object.keys(ALL_INVISIBLE), ...Object.keys(ALL_KNOWN), ...Object.keys(INTENTIONALLY_UNDRAWN)].filter((f) => !allFields.includes(f));
    expect(stale, "no longer in the generated types").toEqual([]);
    const both = Object.keys(ALL_INVISIBLE).filter((f) => f in ALL_CASES);
    expect(both).toEqual([]);
    expect(Object.keys(INTENTIONALLY_UNDRAWN).filter((f) => !(f in ALL_CASES) || f in ALL_KNOWN || f in ALL_INVISIBLE), "intentional no-drawing invariants keep a paired case and are not live defects or skipped fields").toEqual([]);
  });

  // Each case mounts two apps and waits for each to settle on the clock; at a load of 50 and more (the gate, many seats)
  // that passes the default 5 s for a few of the 300 cases. Every wait is bounded, so the limit is only for load.
  it.concurrent.each(Object.keys(ALL_CASES))("%s", { timeout: 60_000 }, async (name) => {
    const offScene = ALL_CASES[name]!(false);
    const onScene = ALL_CASES[name]!(true);
    const [off, on] = await Promise.all([frameOf(offScene), frameOf(onScene)]);
    const known = name in ALL_KNOWN;
    if (name in INTENTIONALLY_UNDRAWN) {
      expect(on, `${name} must stay undrawn: ${INTENTIONALLY_UNDRAWN[name]}`).toBe(off);
    } else if (onScene.needle) {
      // The field's text is what must appear (a layout shift alone does not count as drawn).
      const shown = plain(on).includes(onScene.needle);
      if (known) expect(shown, `${name} is now drawn: remove it from the known-undrawn lists (${ALL_KNOWN[name]})`).toBe(false);
      else expect(shown, `${name}: "${onScene.needle}" is not on screen: the renderer does not draw it`).toBe(true);
    } else if (known) {
      expect(on, `${name} is now drawn: remove it from the known-undrawn lists (${ALL_KNOWN[name]})`).toBe(off);
    } else {
      expect(on, `${name} changes nothing on screen: the renderer does not draw it`).not.toBe(off);
    }
  });
});

describe("every kind of node and of key the host can send is drawn", () => {
  const present = (node: any) => ({ id: "n", scope: "n", panes: [{ pane: "middle", title: "KINDPANE", body: node }], actions: [] });
  const seg = (t: string) => [{ text: t }];
  const KINDS: Record<string, any> = {
    list: { node: "list", selected: 0, rows: [{ target: "r0", text: seg("ZZKIND") }] },
    tree: { node: "tree", columns: [], rows: [{ id: "a", name: seg("ZZKIND") }], name_width: 20 },
    document: { node: "document", lines: [seg("ZZKIND")] },
    rich: { node: "rich", blocks: [{ block: "line", text: seg("ZZKIND") }] },
    table: { node: "table", columns: [{ heading: "COL", width: 20, keep: 0 }], rows: [{ target: "t0", cells: [{ heading: "COL", text: seg("ZZKIND") }] }], selected: 0, empty: null },
    entries: { node: "entries", rows: [{ target: "e0", head: seg("ZZKIND") }], selected: 0, empty: "none" },
    log: { node: "log", lines: [{ text: seg("ZZKIND"), row: 0, selected: false }], top: 0, total: 1, empty: "none" },
    diff: { node: "diff", lines: [{ kind: "added", old: null, new: 1, text: "ZZKIND" }], under: [], selected: 0, top: 0, split: false, language: null, empty: "none" },
    state: { node: "state", state: "empty", text: "ZZKIND" },
    extension: { node: "extension", capability: "zz.unknown", version: 1, data: {}, fallback: { node: "document", lines: [seg("ZZKIND")] } },
  };

  it("every node kind in the generated Node type has a fixture", () => {
    expect(Object.keys(KINDS).sort()).toEqual([...nodeKinds()].sort());
  });

  it.each(Object.keys(KINDS))("a %s node draws its content", async (kind) => {
    const m = mount(W, H);
    m.snapshot({ chrome: chromeOf(W, H), screen: present(KINDS[kind]) });
    await m.settle();
    const shown = m.frame();
    m.stop();
    expect(shown, `the ${kind} node is not drawn`).toContain("ZZKIND");
  });

  it("every ink slot the host can send paints differently from plain (secondary may share plain's look)", async () => {
    const slots = unionOf("Ink");
    expect(slots.length).toBeGreaterThan(10);
    const line = async (ink: string) => {
      const f = await frameOf({ chrome: chromeOf(W, H, { status: { scope: "S", message: { text: "ZZINK", ink } } }), screen: docScreen() });
      return f.split("\n").find((l) => l.includes("ZZINK")) ?? "";
    };
    const plain = await line("plain");
    const same: string[] = [];
    for (const ink of slots) if (ink !== "plain" && ink !== "secondary" && ink !== "primary" /* the host sends Ink::Plain as Primary (ui/log.rs): the same look */ && (await line(ink)) === plain) same.push(ink);
    expect(same, "ink slots drawn exactly as plain text").toEqual([]);
  });
});
