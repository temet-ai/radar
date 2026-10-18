import "./fast-output.js";
import { hover } from "./hover.js";
import { EditorOverlay, editorOverlayHeight } from "../editor/EditorOverlay.js";
import { isEditorData } from "../editor/EditorView.js";
import React, { useEffect, useLayoutEffect, useMemo, useRef, useSyncExternalStore } from "react";
import { Box, Text, useInput, useStdin, useStdout, type DOMElement } from "ink";
import { Session } from "./session.js";
import { dockOf, dockShown, resolveKeyOwner } from "./key-owner.js";
import { motion, isInput, afterMotionLayout } from "../motion/motion.js";
import { MotionSceneContext } from "../motion/scene.js";
import type { MotionScene } from "../motion/interaction.js";
import { shellRenderNode, type NodeCx, type RenderNode } from "./Body.js";
import type { ActionSpec } from "./actions.js";
import { afterIntent, desynced, screenKey, scrollKey, sectionKey, sheetKey, workingFrom } from "../input/route.js";
import { scrollerAt, scrollers } from "../nodes/index.js";
import { layoutNoticeHistory, NOTICE_OPEN } from "../notices/layout.js";
import { NoticeHistoryOverlay } from "../notices/History.js";
import { noticeHistoryEscapeTarget } from "../notices/input.js";
import { hitAt } from "../chrome/layout.js";
import { layoutHistory } from "../chrome/history.js";
import { HistoryOverlay, MenuBox, SettingsNavPane, SourcesRow, StatusRow, TopBarRow, TourCardBox, frameOf, withNotice } from "../chrome/Chrome.js";
import { useSplashPicture } from "./splashPicture.js";
import { SetupBody } from "./SetupBody.js";
import { InputController, menuShowing, type InputView } from "../input/controller.js";
import { WorkArea } from "../workarea/index.js";
import { FrameRampOwnership, Framed, PageBody, PaneArea, ScreenPanes, share, paneTitle, type DockSlot } from "./Panes.js";
import { splitDock } from "../dock/layout.js";
import { Dock } from "../dock/Dock.js";
import { TerminalSurface } from "../terminal/index.js";
import { dockClick, dockCommand, dockClose, dockCloseKey, dockKey, isCloseChord, emptyLocal, scrollDock, WHEEL, type DockLocal } from "../dock/composer.js";
import { Splash } from "./Splash.js";
import type { MouseBus } from "../input/stdin-tap.js";
import { HitRegistry, UiState } from "../state/ui-state.js";
import { tracer } from "./trace.js";
import type { ChromeView, Intent, MenuView } from "../chrome/types.js";
import { selected } from "./selected.js";
import { areaAt, insideOf, selectKindOf } from "../input/selection.js";

/** The key that lets the terminal select past radar's mouse capture: shift in most terminals, option in iTerm2, fn in
 * Terminal.app (issue #51). */
export function bypassKey(env: NodeJS.ProcessEnv = typeof process !== "undefined" ? process.env : {}): string {
  const term = env.TERM_PROGRAM ?? "";
  if (term === "iTerm.app") return "option";
  if (term === "Apple_Terminal") return "fn";
  return "shift";
}

/** While a drag selects text the status line's keys give way to one: how to get the terminal's own selection (host.rs
 * status_hints, while `selecting`). */
export function withDragHint(chrome: ChromeView, dragging: boolean, key = bypassKey()): ChromeView {
  if (!dragging) return chrome;
  return { ...chrome, status: { ...chrome.status, hints: [{ keys: key, label: "terminal's own selection" }], least_needed: [] } };
}

export type AppProps = {
  session: Session;
  ui?: UiState;
  registry?: HitRegistry;
  renderNode?: RenderNode;
  /** Mouse reports taken out of stdin (input/stdin-tap.ts). */
  mouse?: MouseBus;
  /** Called once when the session ends (refused, shutdown, connection lost). */
  onExit?: (code: number, message?: string) => void;
};

function routeBody(snap: any, bodyKeys: boolean, chord: import("../chrome/types.js").Chord): Intent {
  // Only while the body has the keys: with the bar, sources or page tabs focused the host moves.
  if (!snap || !bodyKeys) return { intent: "key", chord };
  // A settings section moves its own rows only while it has the focus; with the nav or the detail focused
  // the host moves (the nav's sections, the detail's scroll).
  if (snap.section) return snap.focus === "middle" ? sectionKey(snap.section, chord) : { intent: "key", chord };
  if (snap.screen) return screenKey(snap.screen, typeof snap.focus === "string" ? snap.focus : undefined, chord);
  return { intent: "key", chord };
}

function useSize() {
  const stdout = useStdout().stdout as unknown as import("node:events").EventEmitter & { columns?: number; rows?: number };
  const read = () => ({ columns: stdout.columns ?? 80, rows: stdout.rows ?? 24 });
  const [size, setSize] = React.useState(read);
  useEffect(() => {
    const on = () => setSize(read());
    stdout.on("resize", on);
    return () => void stdout.off("resize", on);
  }, [stdout]);
  return size;
}

const emptyChrome = (w: number, h: number): ChromeView => ({
  version: 1,
  width: w,
  height: h,
  write_mode: "read_only",
  top_bar: { brand: "radar", tabs: [], mode: { text: "", ink: "plain", target: "chrome:mode" } },
  status: { scope: "" },
  keys_at: "body",
});

export function App({ session, ui: uiProp, registry: regProp, renderNode = shellRenderNode, mouse, onExit }: AppProps) {
  const state = useSyncExternalStore(
    (fn) => session.subscribe(fn),
    () => session.state,
  );
  // RADAR_TRACE_LATENCY=1: what was marked before this render shows in the next frame written.
  tracer.rendered();
  const ui = useMemo(() => uiProp ?? new UiState(), [uiProp]);
  const registry = useMemo(() => regProp ?? new HitRegistry(), [regProp]);
  const size = useSize();
  // The dock's composer text and picker choice are Ink's editor state until sent (everything else is the host's).
  const [dockLocal, setDockLocal] = React.useState<DockLocal>(() => emptyLocal());
  const dockRef = useRef(dockLocal);
  // While the dock's top edge is dragged: the row the dock ends at.
  const dockDrag = useRef<number | null>(null);
  const dockMetrics = useRef({ max: 0, total: 0 });
  // A blur or focus of the dock Ink sent since snapshot `base`: the keys after it in the same read go where it sent them.
  const dockFocus = useRef<{ base: any; focused: boolean } | null>(null);
  const [, redrawDockFocus] = React.useReducer((n: number) => n + 1, 0);
  // While a drag selects text: the status line names the key that gives the terminal its own selection.
  const [dragging, setDragging] = React.useState(false);

  // Resize: tell the host and relayout; focus is never touched. A selection, in screen cells, is dropped.
  const first = useRef(true);
  useEffect(() => {
    if (first.current) {
      first.current = false;
      return;
    }
    controllerRef.current?.selector.resized();
    session.resize(size.columns, size.rows);
  }, [size.columns, size.rows, session]);
  const controllerRef = useRef<InputController | null>(null);

  useEffect(() => {
    if (state.phase === "refused" || state.phase === "closed") onExit?.(state.exitCode ?? 1, state.message);
  }, [state.phase]);

  const snap = state.snapshot;
  // A tab shown for the first time starts from the draft the host kept for it (restored after a restart).
  const dockTab = dockOf(snap)?.active ?? null;
  const dockDraft: string = dockOf(snap)?.composer?.draft ?? "";
  useEffect(() => {
    const next = emptyLocal(dockDraft);
    dockRef.current = next;
    setDockLocal(next);
  }, [dockTab]);
  const chrome = snap?.chrome ?? emptyChrome(size.columns, size.rows);
  // An input request is a typed field in a popup, as a typed menu is (shell/menu.rs draws both the same way).
  const requestMenu: MenuView | null = state.inputRequest
    ? ({ id: "input", title: state.inputRequest.title, question: false, items: [], typed: state.inputRequest.value, caret: [...state.inputRequest.value].length } as MenuView)
    : null;
  const menu: MenuView | null = (menuShowing(state.questions[0]?.menu, chrome.menu) ?? requestMenu);
  const notice = state.notices[0];
  const shownChrome: ChromeView = withDragHint(withNotice(chrome, notice), dragging);
  const actions: ActionSpec[] = (snap?.section?.actions ?? snap?.screen?.actions ?? []) as ActionSpec[];
  const baseFrame = frameOf(shownChrome, menu, size.columns, size.rows, session.plainText);
  // The history overlay replaces everything under the top bar (shell.rs): body, nav, menu and status line.
  const history = chrome.history ?? null;
  const noticeHistory = chrome.notice_history ?? null;
  const noticeHistoryLaid = noticeHistory ? layoutNoticeHistory(noticeHistory, size.columns, Math.max(0, size.rows - 2)) : null;
  const historyLaid = history ? layoutHistory(history, size.columns, Math.max(1, size.rows - 1), 1, session.plainText) : null;
  const frame = noticeHistoryLaid
    ? { ...baseFrame, menu: null, hits: [...baseFrame.top.hits, ...baseFrame.status.hits, ...noticeHistoryLaid.hits] }
    : historyLaid
    ? { ...baseFrame, menu: null, hits: [...baseFrame.top.hits, ...historyLaid.hits] }
    : baseFrame;
  // A capturing editor over the foot of the body (view_tui.rs draw_overlay).
  const overlayBody = snap?.screen?.overlay?.body;
  const editor = overlayBody?.node === "extension" && isEditorData(overlayBody.data) ? overlayBody.data : null;
  const capturing = !history && !noticeHistory && (!!state.inputRequest || menu?.typed != null || editor !== null);

  // Body hits are registered by node components while the frame renders; the controller reads
  // them live, so a click lands on what is on screen now.
  registry.clear();
  const bodyKeys = (chrome.keys_at ?? "body") === "body" && !history && !noticeHistory;
  const live = useRef({ frame, menu, capturing, actions, snap, bodyKeys, registry, history: historyLaid });
  live.current = { frame, menu, capturing, actions, snap, bodyKeys, registry, history: historyLaid };
  const pending = useRef<{ base: any; working: any } | undefined>(undefined);

  const view: InputView = useMemo(() => {
    // The dock as drawn now, and whether it has the keys (a blur or focus sent since the snapshot counts).
    const shown = () => live.current.snap?.chrome?.notice_history || live.current.snap?.chrome?.history ? null : dockShown(live.current.snap);
    const dockHasKeys = () => resolveKeyOwner({ snapshot: live.current.snap, pendingDock: dockFocus.current,
      menuOpen: !!live.current.menu, editorOpen: live.current.capturing, inputRequested: !!session.state.inputRequest }).dock;
    const sendDock = (i: any) => {
      session.sendIntent({ intent: "dock", ...i } as any);
      const focused = i.dock === "blur" || i.dock === "hide" ? false : i.dock === "focus" || i.dock === "show" ? true : null;
      if (focused !== null) {
        const before = dockFocus.current;
        dockFocus.current = { base: live.current.snap, focused };
        if (before?.base !== live.current.snap || before?.focused !== focused) redrawDockFocus();
      }
    };
    return {
      noticeInput: (escape) => session.noticeInput(escape),
      noticeAt: (x, y) => hitAt(session.noticeHits, x, y),
      noticeActivate: (target) => {
        if (target === NOTICE_OPEN) session.sendIntent({ intent: "activate", target });
        else if (/^chrome:notice:\d+$/.test(target)) session.dismissNoticeId(Number(target.slice("chrome:notice:".length)));
      },
      // The session's own state is read now, not the last frame: answering a question clears it at once, and
      // the keys after it in the same read are no longer the question's.
      // Under the history overlay its keys are the overlay's: a question or menu it covers waits (host.rs).
      menu: () => ((live.current.snap?.chrome?.history || live.current.snap?.chrome?.notice_history) ? null : menuShowing(session.state.questions[0]?.menu, live.current.snap?.chrome?.menu)),
      capturing: () => live.current.capturing,
      route: (chord) => {
        // Keys that arrive together are routed one from the other, not all from the newest snapshot.
        const base = live.current.snap;
        pending.current = workingFrom(pending.current, base);
        const intent = routeBody(pending.current?.working, live.current.bodyKeys, chord);
        if (pending.current) pending.current.working = afterIntent(pending.current.working, intent);
        return intent;
      },
      desync: () => {
        pending.current = workingFrom(pending.current, live.current.snap);
        if (pending.current) pending.current.working = desynced(pending.current.working);
      },
      screenKeys: () => live.current.snap?.chrome?.screen_keys,
      dockMouse: (ev, hit) => {
        const dock = shown();
        if (!dock) return false;
        const target = hit?.target ?? "";
        const send = sendDock;
        if (dockDrag.current !== null) {
          // The top edge follows the pointer: the dock is as tall as the rows from the pointer to its foot.
          if (ev.kind === "release") dockDrag.current = null;
          else if (ev.kind === "motion") send({ dock: "resize", rows: Math.max(3, dockDrag.current - ev.y) });
          return true;
        }
        if (ev.kind === "wheel_up" || ev.kind === "wheel_down") {
          // The wheel over the dock scrolls its transcript and nothing else; it moves no focus (keys stay where they were).
          if (!target.startsWith("dock:")) return false;
          const next = scrollDock(dockRef.current, ev.kind === "wheel_up" ? WHEEL : -WHEEL, dockMetrics.current);
          dockRef.current = next;
          setDockLocal(next);
          return true;
        }
        // A tab's close mark closes it on a click; a middle click anywhere on a tab does the same.
        if (ev.kind === "press" && (target.startsWith("dock:close:") ? ev.button === "left" || ev.button === "middle" : target.startsWith("dock:tab:") && ev.button === "middle")) {
          const r = dockClose(dockRef.current, dock, target.slice(target.indexOf(":", 5) + 1));
          dockRef.current = r.local;
          setDockLocal(r.local);
          for (const i of r.intents) send(i);
          if (!dock.focused) send({ dock: "focus" });
          return true;
        }
        // A press anywhere but the dock and the bars takes the keys from the dock (as a click takes them from the top bar).
        if (ev.kind === "press" && !target.startsWith("dock:") && !target.startsWith("chrome:") && dockHasKeys()) send({ dock: "blur" });
        if (ev.kind !== "press" || ev.button !== "left") return false;
        if (/^dock:(approve|session|always|deny|diff|choose|submit|decline|group):/.test(target)) {
          // A request's button: what its key does (approve once, approve for session, always allow, deny, view diff,
          // a question's choice, submit, decline).
          const r = dockClick(dockRef.current, dock, target);
          if (r) {
            dockRef.current = r.local;
            setDockLocal(r.local);
            for (const i of r.intents) send(i);
          }
          if (!dockHasKeys()) send({ dock: "focus" });
          return true;
        }
        if (target.startsWith("dock:cmd:")) {
          // A row of the slash menu: its command runs, as ⏎ on it does.
          const r = dockCommand(dockRef.current, dock, target);
          if (r) {
            dockRef.current = r.local;
            setDockLocal(r.local);
            for (const i of r.intents) send(i);
          }
          if (!dockHasKeys()) send({ dock: "focus" });
          return true;
        }
        if (target.startsWith("dock:fold:")) {
          const rest = target.slice("dock:fold:".length);
          const cut = rest.lastIndexOf(":");
          send({ dock: "fold", item: rest.slice(0, cut), open: rest.slice(cut + 1) === "1" });
          if (!dockHasKeys()) send({ dock: "focus" });
          return true;
        }
        if (target.startsWith("dock:edge:")) {
          dockDrag.current = Number(target.slice("dock:edge:".length));
          return true;
        }
        if (target.startsWith("dock:tab:")) {
          send({ dock: "switch_tab", id: target.slice("dock:tab:".length) });
          if (!dockHasKeys()) send({ dock: "focus" });
          return true;
        }
        if (target === "dock:body") {
          if (!dockHasKeys()) send({ dock: "focus" });
          return true;
        }
        return false;
      },
      dock: (chord) => {
        const dock = shown();
        if (!dock) return false;
        // alt-w closes the tab showing while the dock is on screen, whoever has the keys (a terminal tab that has them
        // is the exception: its program gets every key but ctrl-]).
        if (!dockHasKeys()) {
          if (!isCloseChord(chord)) return false;
          const r = dockCloseKey(dockRef.current, dock);
          dockRef.current = r.local;
          setDockLocal(r.local);
          for (const i of r.intents) sendDock(i);
          return true;
        }
        const r = dockKey(dockRef.current, dock, chord, dockMetrics.current);
        dockRef.current = r.local;
        setDockLocal(r.local);
        for (const i of r.intents) sendDock(i);
        return r.handled;
      },
      paneAt: (x, y) => live.current.registry.paneAt(x, y),
      // Text is selected in a pane, the dock's transcript or the editor; with a menu or a typed prompt up only inside
      // it (nothing behind the scrim, shell.rs); over the history in its panels; never over the splash.
      selectAt: (x, y) => {
        const l = live.current;
        if (l.snap?.splash || l.snap?.chrome?.notice_history) return undefined;
        if (l.menu) {
          const m = l.frame.menu;
          return m ? areaAt([{ ...insideOf(m), id: "menu", kind: "rows" }], x, y) : undefined;
        }
        if (l.history) return areaAt(l.history.selects, x, y);
        return l.registry.selectAt(x, y);
      },
      frameRow: (y) => selected.row(y),
      showSelection: (sel, dragging) => {
        selected.set(sel, session.plainText);
        setDragging(dragging);
      },
      wheel: (pane, up, x, y) => {
        // The history overlay is the host's: it takes the wheel as k/j (history.rs on_mouse).
        if (live.current.snap?.chrome?.history || live.current.snap?.chrome?.notice_history) return false;
        if (!pane) return true;
        const base = live.current.snap;
        pending.current = workingFrom(pending.current, base);
        if (!pending.current) return true;
        // host.rs on_mouse: the wheel focuses the pane under the pointer, then moves it as j/k would. The dock keeps the
        // keys if it had them: what is typed there goes on there.
        const focus = typeof pending.current.working.focus === "string" ? pending.current.working.focus : undefined;
        if (pane !== focus || !live.current.bodyKeys) {
          const sel = { intent: "select", target: `pane:${pane}` } as Intent;
          session.sendIntent(sel);
          pending.current.working = afterIntent(pending.current.working, sel);
        }
        const s = pending.current.working;
        // A Document or Rich pane Ink scrolls itself: the one under the pointer (view_tui::key, Scroll::key); at a Rich
        // pane's end the part hears of it. Past the top the wheel stays in the pane.
        const sc = s.screen && !s.screen.overlay ? scrollerAt(x, y) : undefined;
        if (sc) {
          const moved = sc.key(up ? "k" : "j");
          if (moved && sc.kind === "rich" && sc.at() >= sc.max()) session.sendIntent({ intent: "end", pane } as Intent);
          return true;
        }
        const intent = routeBody(s, true, { key: { char: up ? "k" : "j" } });
        pending.current.working = afterIntent(s, intent);
        session.sendIntent(intent);
        return true;
      },
      editor: () => {
        const body = live.current.snap?.screen?.overlay?.body;
        return body?.node === "extension" && isEditorData(body.data) ? body.data : null;
      },
      // Document and Rich panes scroll in the frontend (view_tui::key), only while the body has the keys.
      scroll: (chord) => {
        // As route: keys that arrive together go from the snapshot the previous one left, a newer snapshot replaces it
        // (a page just opened is not the screen the last key was routed on).
        const base = live.current.snap;
        pending.current = workingFrom(pending.current, base);
        const s = pending.current?.working;
        if (!live.current.bodyKeys) return undefined;
        if (s?.section) return sheetKey(s.section, chord, scrollers());
        return s?.screen ? scrollKey(s.screen, typeof s.focus === "string" ? s.focus : undefined, chord, scrollers()) : undefined;
      },
      // A menu is over the body: body rows under it are not pointable (its hits and the bars' are the frame's).
      hits: () => {
        const l = live.current;
        if (l.snap?.chrome?.notice_history || l.snap?.chrome?.history) return l.frame.hits;
        const now = menuShowing(session.state.questions[0]?.menu, l.snap?.chrome?.menu);
        if (l.menu && !now) {
          // A question the session just answered: the frame still has its popup, but the keys and clicks that
          // follow in the same read are the body's.
          return [...l.registry.all(), ...l.frame.hits.filter((h) => !h.target.startsWith("chrome:menu:"))];
        }
        // A menu is over the body: body rows under it are not pointable (its hits and the bars' are the frame's).
        return l.menu ? l.frame.hits : [...l.registry.all(), ...l.frame.hits];
      },
    };
  }, []);
  const controller = useMemo(
    () =>
      new InputController(
        view,
        (i) => {
          if (i.intent === "activate" && noticeHistoryEscapeTarget(live.current.snap?.chrome, i.target)) session.noticeInput(true);
          session.sendIntent(i);
          // A click selects: the keys after it in the same read go from where it left the selection.
          if (i.intent === "select") {
            const base = live.current.snap;
            pending.current = workingFrom(pending.current, base);
            if (pending.current) pending.current.working = afterIntent(pending.current.working, i);
          }
        },
        ui,
        undefined,
        // ui::hover::paint over the finished frame (app/hover.ts): a new area is one frame Ink takes from the last one,
        // no React render.
        (hit) => hover.set(hit, session.plainText),
        (column, row) => session.sendPointer(column, row),
      ),
    [view, session, ui],
  );
  controllerRef.current = controller;

  // shell.rs hover: a redraw can move a row under a still pointer; ask again once this frame's hits are registered (a
  // frame follows only when what is under the pointer changed).
  useLayoutEffect(() => {
    registry.settle();
    controller.refreshHover();
  });

  // The hover asks Ink for a frame through the root node, as a commit does; nothing is lit while a menu is up.
  const rootBox = useRef<DOMElement>(null);
  useLayoutEffect(() => {
    let root: DOMElement | undefined = rootBox.current ?? undefined;
    while (root?.parentNode) root = root.parentNode;
    const r = root as (DOMElement & { onRender?: () => void; onImmediateRender?: () => void }) | undefined;
    const restoreMotionLayout = r ? afterMotionLayout(r, () => motion.engine.level === "full") : undefined;
    const redraw = () => r?.onRender?.();
    // Nothing is lit over the startup splash either.
    hover.attach(redraw, () => live.current.menu !== null || !!live.current.snap?.splash);
    selected.attach(redraw);
    return () => {
      hover.detach(redraw);
      selected.detach(redraw);
      restoreMotionLayout?.();
      motion.reset();
    };
  }, []);

  // The set of targets showing, so focus survives a snapshot while its target exists. UiState is read by the
  // controller, never by a render, so nothing is drawn again for it (a second full render per snapshot was
  // the same frame computed twice: T21).
  useEffect(() => {
    if (snap) ui.applySnapshot(snap, frame.hits.map((h) => h.target));
  }, [snap?.cursor]);

  // One read of stdin is keys and mouse reports in the order they arrived; each is handled before the next.
  useEffect(
    () =>
      mouse?.onInput((events) => {
        for (const ev of events) {
          if (ev.kind === "key" || ev.kind === "paste" || ev.kind === "mouse" && isInput(ev.data)) session.input();
          if (ev.kind === "mouse") controller.mouse(ev.data);
          else if (ev.kind === "paste") controller.paste(ev.text);
          else if (ev.kind === "focus") {
            if (!ev.on) controller.blur();
          }
          else {
            controller.keyChord(ev.chord, ev.shift);
          }
        }
      }),
    [mouse, controller, session],
  );

  // Ink keeps stdin in raw mode while an input hook is active. Without a TTY on stdin (a pipe, a test)
  // raw mode is unsupported: the renderer still draws. With the tap, keys never reach this hook.
  const { isRawModeSupported } = useStdin();
  useInput((input, key) => {
    if (mouse) return;
    if (/^\[?<\d+;\d+;\d+[Mm]$/.test(input)) return;
    session.input();
    controller.key(input, key);
  }, { isActive: isRawModeSupported });

  const cx: NodeCx = {
    width: size.columns,
    height: frame.bodyHeight,
    actions,
    focused: menu === null,
    plainText: session.plainText,
    onIntent: (i) => session.sendIntent(i as unknown as Intent),
    registry,
    tabsFocused: chrome.keys_at === "page_tabs",
    paneJumps: chrome.pane_jumps,
    writeMode: chrome.write_mode ?? "read_only",
  };
  // shell.rs: a pane draws as focused only while the body has the keys (not the bar, the sources or the page tabs).
  // While the dock has the keys no pane draws focused either: one frame says where the keys go.
  const keysInBody = (chrome.keys_at ?? "body") === "body" && !dockShown(snap)?.focused;
  const focusPane = keysInBody ? (typeof snap?.focus === "string" ? snap.focus : undefined) : "none";
  const shownDock = dockShown(snap);
  const dockOverride = dockFocus.current;
  const pendingDockFocus = !!shownDock && dockOverride !== null && dockOverride.base === snap && dockOverride.focused !== !!shownDock.focused;
  const rampOwners = resolveKeyOwner({ snapshot: snap, pendingDock: dockOverride, menuOpen: menu !== null, editorOpen: editor !== null, inputRequested: !!state.inputRequest });
  const motionScene: MotionScene = {
    cursor: snap?.cursor ?? 0,
    context: snap?.motion?.interaction?.context ?? null,
    interaction: snap?.motion?.interaction ?? undefined,
    blocked: rampOwners.blocked || state.phase === "closed" || state.phase === "refused",
    local: pendingDockFocus,
  };
  motion.prepareScene(motionScene);
  useLayoutEffect(() => { motion.commitScene(motionScene); });
  const bodyW = size.columns;
  const bodyH = frame.bodyHeight;
  const raw = useStdout().stdout;
  useSplashPicture(snap?.splash, (data) => void raw.write(data));
  // The startup splash, while the host sends it, in place of everything (radar_core::splash). The screen under it is in
  // the same snapshot: keys and clicks are routed to it as usual, and the host ends the splash on the first one.
  if (snap?.splash) {
    return (
      <Box ref={rootBox} flexDirection="column" width={size.columns} height={size.rows}>
        <Splash view={snap.splash} columns={size.columns} rows={size.rows} />
      </Box>
    );
  }
  let body: React.ReactNode = null;
  if (state.phase === "connecting" && !snap) body = <Text dimColor>radar ink renderer: waiting for the host</Text>;
  else if (snap?.section && shownChrome.setup) {
    body = (
      <SetupBody
        section={snap.section}
        width={bodyW}
        height={bodyH}
        cx={cx}
        renderNode={renderNode}
        title={shownChrome.setup.title}
        heading={shownChrome.setup.heading}
        lead={shownChrome.setup.lead}
        detailTitle={snap.section.detail_title}
      />
    );
  } else if (snap?.section) {
    body = (
      <SectionBody
        section={snap.section}
        nav={shownChrome.settings ? <SettingsNavPane chrome={shownChrome} frame={frame} focused={focusPane === "nav"} /> : null}
        navWidth={frame.navWidth}
        navBar={frame.navTotal > Math.max(1, frame.bodyHeight - 2)}
        focus={focusPane ?? "middle"}
        width={bodyW}
        height={bodyH}
        cx={cx}
        renderNode={renderNode}
        title={shownChrome.settings?.section_title ?? snap.section.scope}
        detailTitle={shownChrome.settings?.detail_title ?? snap.section.detail_title}
        dock={dockSlot(snap.dock, { dockLocal, bus: mouse, metrics: dockMetrics, menuOpen: !!menu, leave: () => session.sendIntent({ intent: "dock", dock: "blur" } as any), cx })}
      />
    );
  } else if (snap?.screen) body = <ScreenBody screen={snap.screen} dockLocal={dockLocal} bus={mouse} metrics={dockMetrics} menuOpen={!!menu} leave={() => session.sendIntent({ intent: "dock", dock: "blur" } as any)} cx={cx} renderNode={renderNode} focus={focusPane} width={bodyW} height={bodyH} rows={size.rows} />;
  if (noticeHistoryLaid) {
    return (
      <MotionSceneContext.Provider value={motionScene}>
      <Box ref={rootBox} flexDirection="column" width={size.columns} height={size.rows}>
        <TopBarRow frame={frame} />
        <Box height={Math.max(0, size.rows - 2)} width={size.columns} overflow="hidden" flexDirection="column">
          <NoticeHistoryOverlay layout={noticeHistoryLaid} />
        </Box>
        <StatusRow frame={frame} />
      </Box>
      </MotionSceneContext.Provider>
    );
  }
  if (historyLaid) {
    return (
      <MotionSceneContext.Provider value={motionScene}>
      <Box ref={rootBox} flexDirection="column" width={size.columns} height={size.rows}>
        <TopBarRow frame={frame} />
        <Box height={size.rows - 1} width={size.columns} overflow="hidden" flexDirection="column">
          <HistoryOverlay layout={historyLaid} />
        </Box>
      </Box>
      </MotionSceneContext.Provider>
    );
  }
  return (
    <MotionSceneContext.Provider value={motionScene}>
    <FrameRampOwnership.Provider value={rampOwners}>
    <Box ref={rootBox} flexDirection="column" width={size.columns} height={size.rows}>
      <TopBarRow frame={frame} />
      <SourcesRow frame={frame} />
      <Box height={bodyH} width={size.columns} overflow="hidden" flexDirection="column">
        <Box flexDirection="column" flexShrink={0} width={size.columns}>
          {body}
        </Box>
        {editor ? (
          <Box position="absolute" marginTop={Math.max(0, bodyH - editorOverlayHeight(editor.lines?.length ?? 1, bodyH))} width={size.columns}>
            <EditorOverlay data={editor} width={size.columns} areaHeight={bodyH} registry={registry} />
          </Box>
        ) : null}
        {shownChrome.tour ? <TourCardBox card={shownChrome.tour} width={size.columns} height={bodyH} plain={session.plainText} /> : null}
        {menu ? <MenuBox menu={menu} frame={frame} /> : null}
      </Box>
      <StatusRow frame={frame} />
    </Box>
    </FrameRampOwnership.Provider>
    </MotionSceneContext.Provider>
  );
}

type Metrics = { current: { max: number; total: number } };
type DockOpts = { dockLocal: DockLocal; bus?: MouseBus; metrics: Metrics; menuOpen: boolean; leave: () => void; cx: NodeCx };

/** The dock drawn into the column it sits under, while the host shows it. */
function dockSlot(d: any, o: DockOpts): DockSlot | undefined {
  const { dockLocal, bus, metrics, menuOpen, leave, cx } = o;
  return d?.visible ? { rows: d.rows, full: !!d.full, render: (w, h) => <Dock data={d} width={w} height={h} plain={cx.plainText} draft={dockLocal.text} caret={dockLocal.caret} registry={cx.registry as any} scroll={dockLocal.scroll} anchor={dockLocal.anchor} metrics={metrics} pick={dockLocal.pick} cmd={dockLocal.cmd} diffs={dockLocal.diffs} picks={dockLocal.picks} refused={dockLocal.refused} groups={dockLocal.groups} terminal={(columns, rows, origin) => { const t = d.tabs?.find((x: any) => x.id === d.active)?.terminal; return t && t.socket ? <TerminalSurface socket={t.socket} columns={columns} rows={rows} focused={!!d.focused && !menuOpen} bus={bus} origin={origin} onLeave={() => leave()} /> : null; }} /> } : undefined;
}

const hasWorkSections = (screen: any): boolean =>
  (screen.panes ?? []).some((p: any) => p?.body?.node === "extension" && String(p.body.capability).startsWith("work."));

/** A `work.*` extension is the work area's own section: T12's layout draws it, from the content the part gave. */
const unwrapWork = (node: any) => (node?.node === "extension" && String(node.capability).startsWith("work.") && node.fallback ? node.fallback : node);

function ScreenBody({ screen, dockLocal, bus, metrics, menuOpen, leave, cx, renderNode, focus, width, height, rows }: { screen: any; dockLocal: DockLocal; bus?: MouseBus; metrics: Metrics; menuOpen: boolean; leave: () => void; cx: NodeCx; renderNode: RenderNode; focus?: string; width: number; height: number; rows: number }) {
  if (screen.page) return <PageBody page={screen.page} focus={focus} width={width} height={height} cx={cx} renderNode={renderNode} />;
  if (hasWorkSections(screen)) {
    return (
      <WorkArea
        screen={screen}
        capabilities={{ columns: width, rows, plain_text: !!cx.plainText }}
        height={height}
        panels="pending"
        paneJumps={cx.paneJumps}
        focus={focus}
        registry={cx.registry}
        renderNode={(node, c) => renderNode(unwrapWork(node), { ...cx, width: c.columns, focused: c.focused })}
      />
    );
  }
  const d = screen.dock;
  const dock = dockSlot(d, { dockLocal, bus, metrics, menuOpen, leave, cx });
  return <ScreenPanes screen={screen} focus={focus} width={width} height={height} cx={cx} renderNode={renderNode} dock={dock} />;
}

/** One framed pane of a settings section or of setup: the section's rows (`middle`) or its detail (`detail`). */
function sectionPane(cx: NodeCx, renderNode: RenderNode, focused: boolean, title: string, node: any, w: number, h: number, pane: string) {
  return (
    <Framed title={paneTitle({ pane, title }, cx.paneJumps)} width={w} height={h} focused={focused} pane={pane} registry={cx.registry} select={selectKindOf(node)}>
      {renderNode(node, { ...cx, focused, scrolls: node.node === "document", sheet: pane === "detail", width: Math.max(0, w - 2), height: Math.max(1, h - 2) })}
    </Framed>
  );
}

/** A settings section: the nav beside the section's rows and its detail (a stack when narrow). */
function SectionBody(p: { section: any; nav: React.ReactNode; navWidth: number; navBar?: boolean; focus: string; width: number; height: number; cx: NodeCx; renderNode: RenderNode; title: string; detailTitle: string; dock?: DockSlot }) {
  const { section, nav, navWidth, width, height, cx, renderNode } = p;
  const rest = Math.max(0, width - navWidth);
  // shell.rs settings_areas: from 120 cells the section and its detail sit side by side (3:2), below that the detail is
  // under the section, the two halves equal (Fill(1), Fill(1)).
  const wide = width >= 120;
  const detailNode = section.detail_blocks?.length ? { node: "rich", blocks: section.detail_blocks } : { node: "document", lines: section.detail ?? [] };
  const mk = (title: string, node: any, w: number, h: number, pane: string) => sectionPane(cx, renderNode, p.focus === pane, title, node, w, h, pane);
  // Ratatui's Fill constraints round the first share to the nearest cell (136 cells at 3:2 are 82 and 54).
  // The dock sits under the section (under the section and its detail when they stack); narrow, it covers them.
  const split = p.dock ? splitDock(height, width, true, p.dock.rows, p.dock.full) : null;
  const docked = split && split.dock > 0 ? split : null;
  const above = docked ? (docked.full ? 0 : docked.above) : height;
  const under = (w: number) => (docked ? p.dock!.render(w, docked.dock) : null);
  const total = wide ? rest : above;
  const first = Math.round((total * (wide ? 3 : 1)) / (wide ? 5 : 2));
  const [a, b] = [first, total - first];
  return (
    <Box flexDirection="row" width={width} height={height} overflow="hidden">
      {nav ? (
        <PaneArea pane="nav" registry={cx.registry} width={navWidth} height={height} select="rows" selectRight={p.navBar ? 1 : 0}>
          {nav}
        </PaneArea>
      ) : null}
      {wide ? (
        <>
          <Box flexDirection="column" width={a!} height={height} flexShrink={0}>
            {above > 0 ? mk(p.title, section.body, a!, above, "middle") : null}
            {under(a!)}
          </Box>
          {mk(p.detailTitle, detailNode, b!, height, "detail")}
        </>
      ) : (
        <Box flexDirection="column" width={rest} height={height}>
          {above > 0 ? mk(p.title, section.body, rest, a!, "middle") : null}
          {above > 0 ? mk(p.detailTitle, detailNode, rest, b!, "detail") : null}
          {under(rest)}
        </Box>
      )}
    </Box>
  );
}
