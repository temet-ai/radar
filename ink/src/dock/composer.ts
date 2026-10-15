import type { Chord } from "../chrome/types.js";
import type { DockAsk, DockCommand, DockIntent, DockItem, DockView } from "./model.js";
import { dockScroll, pickKey } from "./rows.js";
import { transcriptUnits } from "./tools.js";

/**
 * The dock's keys as a pure function: the composer's text is Ink's editor state until it is sent (docs: DockComposer),
 * everything else is an intent for the host. `dockKey` takes the local state, the host's view and one chord and says
 * what the state is now and which intents to send. Nothing here touches a terminal.
 */

export type DockLocal = {
  text: string;
  /** Code points before the caret. */
  caret: number;
  /** Position in the sent history while walking it (null: writing a fresh message). */
  walk: number | null;
  /** What was typed before the walk began, put back when the walk returns to the foot. */
  stash: string;
  /** Items whose diff is open ("view diff" is Ink-side only). */
  diffs: string[];
  /** The open picker's selected choice. */
  pick: number;
  /** The slash menu's selected row, among the commands matching what is typed (it starts at the first again whenever the text changes). */
  cmd: number;
  /** Rows the transcript is scrolled up from its foot (0: following it), as it was when it had `anchor` rows. */
  scroll: number;
  /** The transcript's rows when `scroll` was set: what comes in under it after that keeps the same rows in view. */
  anchor: number;
  /** The choices picked so far of a question that takes several, by pickKey(request, question); sent with ⏎. */
  picks: Record<string, string[]>;
  /** Why the last ⏎ sent nothing (the session is disconnected, or not started yet): the dock's status line says so
   * until the next key, and the text stays in the composer. */
  refused?: string;
  /** The groups of calls shown open (reads in a row fold into one row, src/dock/tools.ts; the host knows only calls). */
  groups: string[];
};

export const emptyLocal = (text = ""): DockLocal => ({ text, caret: Array.from(text).length, walk: null, stash: "", diffs: [], pick: 0, cmd: 0, scroll: 0, anchor: 0, picks: {}, groups: [] });

const toggled = (xs: string[] | undefined, x: string) => ((xs ?? []).includes(x) ? (xs ?? []).filter((y) => y !== x) : [...(xs ?? []), x]);

export type DockKeyResult = { local: DockLocal; intents: DockIntent[]; handled: boolean };

/** What the last draw of the transcript said: how far it can scroll and how many rows it has. */
export type DockMetrics = { max: number; total: number };

/** Rows a page key scrolls. */
export const PAGE = 5;
/** Rows a wheel notch scrolls. */
export const WHEEL = 3;

export { dockScroll };

/** The transcript scrolled `rows` up (negative: down), within what the last draw said it can. */
export function scrollDock(local: DockLocal, rows: number, m: DockMetrics): DockLocal {
  const now = Math.min(m.max, dockScroll(local, m.total));
  return { ...local, scroll: Math.min(m.max, Math.max(0, now + rows)), anchor: m.total };
}
const chars = (s: string) => Array.from(s);
const put = (l: DockLocal, text: string, caret: number): DockLocal => ({ ...l, text, caret, walk: null, cmd: 0, refused: undefined });

/** The slash commands that match what is typed after the `/` (the first word only). */
export function matchingCommands(text: string, commands: DockCommand[] | undefined): DockCommand[] {
  if (!text.startsWith("/") || text.includes(" ") || text.includes("\n")) return [];
  const head = text.slice(1).toLowerCase();
  return (commands ?? []).filter((c) => c.name.toLowerCase().startsWith(head));
}

/** Rows the slash menu shows at most. */
export const MENU_ROWS = 6;

/** The menu's window onto `n` matches: the rows from `first` on that fit in `rows`, keeping `sel` in view at the foot as it moves down. */
export function menuWindow(n: number, sel: number, rows = MENU_ROWS): { first: number; count: number } {
  const count = Math.max(0, Math.min(n, rows));
  return { first: Math.max(0, Math.min(sel - (count - 1), n - count)), count };
}

type PermissionItem = Extract<DockItem, { item: "permission" }>;
type QuestionItem = Extract<DockItem, { item: "question" }>;

/** What waits for the user, the newest first: a permission not yet answered, or a question still waiting with the
 * question of it that is answered next. The keys and buttons answer this one. */
export function pendingAsk(view: DockView): { permission: PermissionItem } | { question: QuestionItem; ask: DockAsk } | undefined {
  const items = view.transcript ?? [];
  for (let i = items.length - 1; i >= 0; i--) {
    const it = items[i]!;
    if (it.item === "permission" && !it.answered) return { permission: it };
    if (it.item === "question" && it.state === "waiting") {
      const ask = it.questions.find((q) => !q.answered);
      if (ask) return { question: it, ask };
    }
  }
  return undefined;
}

/** The request waiting for an answer, if the newest thing waiting is a permission. */
export const pendingPermission = (view: DockView): PermissionItem | undefined => {
  const p = pendingAsk(view);
  return p && "permission" in p ? p.permission : undefined;
};

/** The question waiting for an answer, if the newest thing waiting is a question. */
export const pendingQuestion = (view: DockView) => {
  const p = pendingAsk(view);
  return p && "question" in p ? p : undefined;
};

const withoutPick = (picks: Record<string, string[]>, key: string) => {
  const { [key]: _gone, ...rest } = picks;
  return rest;
};

/** A choice of the question waiting: the answer when it takes one, else picked or unpicked (sent with ⏎). */
function choose(local: DockLocal, q: QuestionItem, ask: DockAsk, label: string): DockKeyResult {
  const out = (l: DockLocal, intents: DockIntent[] = []): DockKeyResult => ({ local: l, intents, handled: true });
  if (!ask.multiple) return out(local, [{ dock: "answer", id: q.id, question: ask.id, choices: [label] }]);
  const k = pickKey(q.id, ask.id);
  const picked = local.picks[k] ?? [];
  const next = picked.includes(label) ? picked.filter((p) => p !== label) : [...picked, label];
  return out({ ...local, picks: { ...local.picks, [k]: next } });
}

/** A click on one of a request's buttons (src/dock/rows.ts hitTarget): exactly what its key does. `undefined` for a
 * target that is not a button, or a button of a request that no longer waits. */
export function dockClick(local: DockLocal, view: DockView, target: string): DockKeyResult | undefined {
  const out = (l: DockLocal, intents: DockIntent[] = []): DockKeyResult => ({ local: l, intents, handled: true });
  const [dock, what, ...enc] = target.split(":");
  if (dock !== "dock" || !what) return undefined;
  const [id = "", question = "", label = ""] = enc.map(decodeURIComponent);
  const items = view.transcript ?? [];
  const perm = items.find((i): i is PermissionItem => i.item === "permission" && i.id === id && !i.answered);
  const q = items.find((i): i is QuestionItem => i.item === "question" && i.id === id && i.state === "waiting");
  const ask = q?.questions.find((a) => a.id === question);
  switch (what) {
    case "approve":
      return perm?.actions.includes("approve_once") ? out(local, [{ dock: "approve", id }]) : undefined;
    case "session":
      return perm?.actions.includes("approve_for_session") ? out(local, [{ dock: "approve_for_session", id }]) : undefined;
    case "deny":
      return perm?.actions.includes("deny") ? out(local, [{ dock: "deny", id }]) : undefined;
    case "diff": {
      if (!perm) return undefined;
      const open = local.diffs.includes(id);
      return out({ ...local, diffs: open ? local.diffs.filter((d) => d !== id) : [...local.diffs, id] });
    }
    case "choose":
      return q && ask && !ask.answered && (ask.choices ?? []).some((c) => c.label === label) ? choose(local, q, ask, label) : undefined;
    case "submit": {
      const picked = local.picks[pickKey(id, question)] ?? [];
      if (!q || !ask || picked.length === 0) return undefined;
      return out({ ...local, picks: withoutPick(local.picks, pickKey(id, question)) }, [{ dock: "answer", id, question, choices: picked }]);
    }
    case "decline":
      return q ? out(local, [{ dock: "decline", id }]) : undefined;
    case "group":
      return transcriptUnits(items).some((u) => u.u === "group" && u.id === id) ? out({ ...local, groups: toggled(local.groups, id) }) : undefined;
  }
  return undefined;
}

/** What the status line tells of the keys: leave a terminal tab; esc declines or stops what runs; alt-w closes the tab.
 * After a ⏎ that could send nothing, why, first: the text was kept, never dropped. */
export function dockHint(view: DockView, refused?: string): string | undefined {
  const tab = (view.tabs ?? []).find((t) => t.id === view.active);
  if (tab?.terminal) return view.focused ? "ctrl-] leave" : undefined;
  if (refused) return `not sent: ${refused}`;
  const parts = [
    pendingQuestion(view) ? "esc decline" : view.composer?.can_stop ? "esc stop" : undefined,
    // The dock over the whole middle column and back (host state, DockView.full).
    view.focused ? (view.full ? "alt-f split" : "alt-f full") : undefined,
    view.focused && tab ? "alt-w close" : undefined,
  ];
  return parts.filter(Boolean).join(" · ") || undefined;
}

/** Close tab `id` (its mark, a middle click, alt-w). The host asks first when the tab's turn runs; the draft goes with a
 * tab that closes at once and stays with one that is asked about. */
export function dockClose(local: DockLocal, view: DockView, id: string): DockKeyResult {
  const t = (view.tabs ?? []).find((x) => x.id === id);
  const asked = !!t && !t.terminal && (t.state === "streaming" || t.state === "approval");
  return { local: t && id === view.active && !asked ? emptyLocal() : local, intents: [{ dock: "close_tab", id }], handled: true };
}

/** alt-w: the chord that closes the tab showing (whichever way the terminal wrote it: `ESC w`, `CSI 119;3u`, `CSI 27;3;119~`). */
export function isCloseChord(chord: Chord): boolean {
  return !!chord.alt && !chord.ctrl && typeof chord.key === "object" && chord.key.char === "w";
}

/** alt-w with the dock shown: closes the active tab; with no tab left it hides the dock, so "close" never leaves an
 * empty window (the dock has the keys or not: the key works while the dock is on screen). */
export function dockCloseKey(local: DockLocal, view: DockView): DockKeyResult {
  const active = (view.tabs ?? []).find((t) => t.id === view.active);
  return active ? dockClose(local, view, active.id) : { local, intents: [{ dock: "hide" }], handled: true };
}

/** `metrics` is how far the transcript can scroll and its rows (the last draw said; a number is the furthest only); keys
 * past it stop there. */
export function dockKey(local: DockLocal, view: DockView, chord: Chord, metrics: number | DockMetrics = Infinity): DockKeyResult {
  const m: DockMetrics = typeof metrics === "number" ? { max: metrics, total: local.anchor } : metrics;
  const out = (l: DockLocal, intents: DockIntent[] = []): DockKeyResult => ({ local: l, intents, handled: true });
  const key = chord.key;
  const ch = typeof key === "object" ? key.char : undefined;
  const tabs = view.tabs ?? [];
  const at = Math.max(0, tabs.findIndex((t) => t.id === view.active));
  const active = tabs[at];

  // The terminal tab owns its keys (T7's surface); the dock keeps only the way out.
  if (active?.terminal) return { local, intents: [], handled: false };

  // A picker open over the transcript takes the arrows, enter and esc.
  if (view.picker) {
    const n = view.picker.choices.length;
    if (key === "up") return out({ ...local, pick: Math.max(0, local.pick - 1) });
    if (key === "down") return out({ ...local, pick: Math.min(Math.max(0, n - 1), local.pick + 1) });
    if (key === "esc") return out(local, [{ dock: "close_picker" }]);
    if (key === "enter") {
      const c = view.picker.choices[local.pick];
      if (!c) return out(local, [{ dock: "close_picker" }]);
      // A skill is asked for in the composer, where it can still be edited before it is sent.
      if (view.picker.kind === "skill") {
        const ask = `Use the ${c.id} skill. `;
        const text = local.text.slice(0, local.caret) + ask + local.text.slice(local.caret);
        return out({ ...local, text, caret: local.caret + ask.length }, [{ dock: "pick_skill", id: c.id }]);
      }
      const pick = { model: "pick_model", session: "pick_session", agent: "pick_agent" }[view.picker.kind as "model" | "session" | "agent"];
      return out(local, [{ dock: pick, id: c.id } as DockIntent]);
    }
    return out(local);
  }

  // A question waits: esc declines it (the agent hears so; a second esc stops the turn).
  const waiting = pendingQuestion(view);
  if (key === "esc" && waiting) return out(local, [{ dock: "decline", id: waiting.question.id }]);
  // While a turn runs, esc stops it (ctrl-c stays the host's quit); with none running it gives the keys back.
  if (key === "esc") return out(local, [view.composer?.can_stop ? { dock: "stop" } : { dock: "blur" }]);
  if (key === "page_up") return out(scrollDock(local, PAGE, m));
  if (key === "page_down") return out(scrollDock(local, -PAGE, m));
  // alt-o folds or opens the latest tool call, group of calls or reasoning (a click on its row does the same for any).
  if (chord.alt && ch === "o") {
    const u = [...transcriptUnits(view.transcript ?? [])].reverse().find((x) => x.u === "group" || x.item.item === "tool" || x.item.item === "reasoning");
    if (u?.u === "group") return out({ ...local, groups: toggled(local.groups, u.id) });
    const it = u?.item;
    return out(local, it && (it.item === "tool" || it.item === "reasoning") ? [{ dock: "fold", item: it.id, open: !it.open }] : []);
  }

  // Tabs and sessions: alt-← / alt-→ switch, ctrl-t starts one, alt-w closes the tab (ctrl-w too, with nothing written), alt-m and alt-s open the pickers.
  if (chord.alt && (key === "left" || key === "right") && tabs.length > 1) {
    const next = tabs[(at + (key === "right" ? 1 : tabs.length - 1)) % tabs.length]!;
    return out(emptyLocal(), [{ dock: "switch_tab", id: next.id }]);
  }
  // The host's dock key, pressed in an empty composer, goes to the tab that wants you out of sight (the status line's
  // chip names it), else hides the dock (the host does the same when the dock has no focus).
  if (ch === "`" && !chord.ctrl && !chord.alt && local.text === "")
    return view.attention && view.attention !== view.active ? out(emptyLocal(), [{ dock: "switch_tab", id: view.attention }]) : out(local, [{ dock: "hide" }]);
  // alt-f: the dock takes the whole middle column, and back.
  if (chord.alt && ch === "f") return out(local, [{ dock: "toggle_full" }]);
  if (chord.ctrl && ch === "t") return out(local, [{ dock: "new_session" }]);
  // alt-w closes the tab. ctrl-w does too while nothing is written (with text it deletes the word before the caret, as
  // a shell's does, below): a draft is not lost to a habit.
  if ((chord.alt && ch === "w") || (active && chord.ctrl && ch === "w" && local.text === "")) return dockCloseKey(local, view);
  if (chord.alt && ch === "m") return out({ ...local, pick: 0 }, [{ dock: "open_picker", kind: "model" }]);
  if (chord.alt && ch === "s") return out({ ...local, pick: 0 }, [{ dock: "open_picker", kind: "session" }]);
  if (chord.alt && ch === "a") return out({ ...local, pick: 0 }, [{ dock: "open_picker", kind: "agent" }]);
  if (chord.alt && ch === "k") return out({ ...local, pick: 0 }, [{ dock: "open_picker", kind: "skill" }]);
  if (chord.ctrl && ch === "c") return view.composer?.can_stop ? out(local, [{ dock: "stop" }]) : { local, intents: [], handled: false };

  // An approval waits and the composer is empty: its keys are letters inside their words (a̲pprove once, approve for
  // s̲ession, d̲eny, v̲iew diff).
  const ask = pendingPermission(view);
  if (ask && local.text === "" && ch && !chord.ctrl && !chord.alt) {
    if (ch === "a" && ask.actions.includes("approve_once")) return out(local, [{ dock: "approve", id: ask.id }]);
    if (ch === "s" && ask.actions.includes("approve_for_session")) return out(local, [{ dock: "approve_for_session", id: ask.id }]);
    if (ch === "d" && ask.actions.includes("deny")) return out(local, [{ dock: "deny", id: ask.id }]);
    if (ch === "v" && ask.actions.includes("view_diff")) {
      const open = local.diffs.includes(ask.id);
      return out({ ...local, diffs: open ? local.diffs.filter((d) => d !== ask.id) : [...local.diffs, ask.id] });
    }
  }

  // A question waits: in an empty composer its choices' numbers answer it (or pick, when it takes several); ⏎ sends
  // what is typed as the answer when it takes the user's own words, or the picks.
  if (waiting && !chord.ctrl && !chord.alt) {
    const { question: q, ask } = waiting;
    const choices = ask.choices ?? [];
    if (local.text === "" && ch && /^[1-9]$/.test(ch) && choices[Number(ch) - 1]) return choose(local, q, ask, choices[Number(ch) - 1]!.label);
    if (key === "enter") {
      const text = local.text.trim();
      const k = pickKey(q.id, ask.id);
      const picked = local.picks[k] ?? [];
      if (text && ask.free_text) return out({ ...emptyLocal(), diffs: local.diffs, groups: local.groups, picks: withoutPick(local.picks, k) }, [{ dock: "answer", id: q.id, question: ask.id, text }]);
      if (!text && ask.multiple && picked.length > 0) return out({ ...local, picks: withoutPick(local.picks, k) }, [{ dock: "answer", id: q.id, question: ask.id, choices: picked }]);
    }
  }

  const off = view.composer?.disabled;
  const cs = chars(local.text);
  // The slash menu, while the first word is a `/` and something matches: its arrows, tab and ⏎ choose a command.
  const menu = off ? [] : matchingCommands(local.text, view.composer?.commands);
  const sel = Math.min(Math.max(0, local.cmd), Math.max(0, menu.length - 1));
  const complete = (name: string) => out(put(local, `/${name} `, chars(`/${name} `).length));
  if (key === "enter") {
    if (chord.alt) return off ? out(local) : out(put(local, [...cs.slice(0, local.caret), "\n", ...cs.slice(local.caret)].join(""), local.caret + 1));
    // ⏎ on a half-typed command finishes the word (and its space for arguments); on the whole word it sends.
    const chosen = menu[sel];
    if (chosen && !off && chosen.name.toLowerCase() !== local.text.slice(1).toLowerCase()) return complete(chosen.name);
    const text = local.text.trim();
    if (text === "") return out(local);
    // A turn that runs never refuses a message (the host queues it); only a session that cannot take one does, and
    // then the text stays and the status line says why.
    if (off || !view.composer?.can_send) return out({ ...local, refused: off ?? "the session cannot take a message now" });
    // The groups shown open stay open, as the host keeps the calls' own folds.
    return out({ ...emptyLocal(), groups: local.groups }, [{ dock: "send", text }]);
  }
  if (off) return out(local);

  if (key === "backspace") return out(local.caret === 0 ? local : put(local, [...cs.slice(0, local.caret - 1), ...cs.slice(local.caret)].join(""), local.caret - 1));
  if (key === "delete") return out(put(local, [...cs.slice(0, local.caret), ...cs.slice(local.caret + 1)].join(""), local.caret));
  if (key === "left") return out({ ...local, caret: Math.max(0, local.caret - 1) });
  if (key === "right") return out({ ...local, caret: Math.min(cs.length, local.caret + 1) });
  if (key === "home" || (chord.ctrl && ch === "a")) return out({ ...local, caret: 0 });
  if (key === "end" || (chord.ctrl && ch === "e")) return out({ ...local, caret: cs.length });
  if (chord.ctrl && ch === "u") return out(put(local, cs.slice(local.caret).join(""), 0));
  if (chord.ctrl && ch === "k") return out(put(local, cs.slice(0, local.caret).join(""), local.caret));
  if (chord.ctrl && ch === "w") {
    // the word before the caret, and the spaces between it and the caret
    let from = local.caret;
    while (from > 0 && /\s/.test(cs[from - 1]!)) from--;
    while (from > 0 && !/\s/.test(cs[from - 1]!)) from--;
    return out(put(local, [...cs.slice(0, from), ...cs.slice(local.caret)].join(""), from));
  }

  if (menu.length > 0 && (key === "up" || key === "down")) return out({ ...local, cmd: key === "up" ? Math.max(0, sel - 1) : Math.min(menu.length - 1, sel + 1) });

  // History: up from a message with no line above the caret walks back through what was sent; down walks forward.
  const history = view.composer?.history ?? [];
  const firstLine = !cs.slice(0, local.caret).includes("\n");
  const lastLine = !cs.slice(local.caret).includes("\n");
  // In a message of several lines (a pasted or typed one), up and down move the caret by line; only the first and last lines hand over to the history.
  if ((key === "up" && !firstLine) || (key === "down" && !lastLine)) return out({ ...local, caret: moveLine(cs, local.caret, key === "up" ? -1 : 1) });
  if (key === "up" && firstLine && history.length > 0) {
    const walk = local.walk === null ? history.length - 1 : Math.max(0, local.walk - 1);
    const text = history[walk]!;
    return out({ ...local, text, caret: chars(text).length, walk, stash: local.walk === null ? local.text : local.stash });
  }
  if (key === "down" && lastLine && local.walk !== null) {
    const walk = local.walk + 1;
    const text = walk >= history.length ? local.stash : history[walk]!;
    return out({ ...local, text, caret: chars(text).length, walk: walk >= history.length ? null : walk });
  }
  // The dock has the keys: ↑ and ↓ with no history to walk move nothing, not the pane under the dock either.
  if ((key === "up" || key === "down") && !chord.ctrl && !chord.alt) return out(local);
  if (key === "tab") {
    // Complete the menu's selected slash command.
    const chosen = menu[sel];
    return chosen ? complete(chosen.name) : out(local);
  }

  if (ch && !chord.ctrl && !chord.alt) return out(put(local, [...cs.slice(0, local.caret), ch, ...cs.slice(local.caret)].join(""), local.caret + 1));
  return { local, intents: [], handled: false };
}

/** The caret one line up (-1) or down (1), in the same column or at the end of a shorter line. */
function moveLine(cs: string[], caret: number, dir: -1 | 1): number {
  const start = (at: number) => {
    let i = at;
    while (i > 0 && cs[i - 1] !== "\n") i--;
    return i;
  };
  const here = start(caret);
  const col = caret - here;
  if (dir < 0) {
    const prevEnd = here - 1; // the newline ending the line above
    const prevStart = start(prevEnd);
    return prevStart + Math.min(col, prevEnd - prevStart);
  }
  let nextStart = caret;
  while (nextStart < cs.length && cs[nextStart] !== "\n") nextStart++;
  nextStart++; // past the newline
  let nextEnd = nextStart;
  while (nextEnd < cs.length && cs[nextEnd] !== "\n") nextEnd++;
  return nextStart + Math.min(col, nextEnd - nextStart);
}

/** A paste: its text with line breaks kept, inserted at the caret. */
export function dockPaste(local: DockLocal, text: string): DockLocal {
  const cs = chars(local.text);
  const add = chars(text.replace(/\r\n?/g, "\n"));
  return put(local, [...cs.slice(0, local.caret), ...add, ...cs.slice(local.caret)].join(""), local.caret + add.length);
}
