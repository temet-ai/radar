// Joins the motion engine to the app: the host's cues and level (Snapshot.motion), the user's input, the places the
// effects run in (body, popup, status message: worked out from the same view data the app draws from), the frame tick,
// and the hook on Ink's output. Called once from the renderer's entry; nothing else in the app knows of motion.
import { RawNoticeInput } from "../notices/raw-input.js";
import { noticePresentation } from "../notices/paint.js";
import type { MotionCue } from "../protocol/generated/MotionCue.js";
import type { MotionView } from "../protocol/generated/MotionView.js";
import { cellWidth } from "../chrome/width.js";
import { frameOf, withNotice } from "../chrome/Chrome.js";
import { statusRight } from "../chrome/layout.js";
import { menuShowing } from "../input/controller.js";
import type { MouseBus } from "../input/stdin-tap.js";
import type { Session } from "../app/session.js";
import type { MenuView } from "../chrome/types.js";
import type { Cue, Marks } from "./engine.js";
import {
  installMotionOutput,
  isInput,
  motion as theMotion,
  type Motion,
} from "./motion.js";

/** The engine's cue for the host's. */
export function cueOf(c: MotionCue): Cue {
  return c.type === "changed"
    ? { kind: "changed", changed: c.changed, added: c.added }
    : { kind: c.type };
}

/**
 * Where the shell drew its places in the frame the session shows now (shell.rs marks the same three): the body
 * between the top bar and the status line, the menu or confirm, and the message on the status line. Rows and
 * waiting panels are marked by the nodes that draw them (`motion.markRow`, `motion.markLoading`).
 */
export function shellMarks(
  session: Session,
  columns: number,
  rows: number,
): Marks {
  const state = session.state;
  const snap = state.snapshot;
  if (!snap || columns <= 0 || rows < 3) return {};
  const chrome = withNotice(snap.chrome, state.notices[0]);
  const requestMenu: MenuView | null = state.inputRequest
    ? ({
        id: "input",
        title: state.inputRequest.title,
        question: false,
        items: [],
        typed: state.inputRequest.value,
        caret: [...state.inputRequest.value].length,
      } as MenuView)
    : null;
  const menu =
    menuShowing(state.questions[0]?.menu, snap.chrome.menu) ?? requestMenu;
  const history = snap.chrome.history != null || snap.chrome.notice_history != null;
  const frame = frameOf(chrome, history ? null : menu, columns, rows);
  const marks: Marks = {
    body: { x: 0, y: 1, w: columns, h: Math.max(0, rows - 2) },
  };
  if (frame.menu)
    marks.popup = {
      x: frame.menu.x,
      y: frame.menu.y,
      w: frame.menu.w,
      h: frame.menu.h,
    };
  const message = chrome.status.message;
  if (message) {
    // shell.rs status: the scope chip and two cells, then the message (its icon, if any, and its words), clipped
    // to what the counts and the services' chips on the right leave.
    const x = cellWidth(` ${chrome.status.scope} `) + 2;
    const { rightW } = statusRight(chrome, columns);
    const lead = message.icon ? 2 : 0;
    const w = Math.min(
      lead + cellWidth(message.text),
      columns - (rightW + 1) - x,
    );
    if (w > 0) marks.message = { x, y: rows - 1, w, h: 1 };
  }
  return marks;
}

/**
 * Wire motion into a running renderer. Returns a function that takes it out again.
 * - every new snapshot carries the host's level and the cues raised since the last one;
 * - a key, click, wheel turn or paste drops what runs (pointer moves do not);
 * - while an effect runs the session is poked every 30 ms so Ink draws a frame, and no other time.
 */
export function attachMotion(
  session: Session,
  mouse: MouseBus,
  stdout: { columns?: number; rows?: number },
  m: Motion = theMotion,
): () => void {
  installMotionOutput(m);
  m.setPresentation(noticePresentation(session, stdout, () => m.engine.level !== "off"));
  let seen: unknown;
  let seenLook = session.state.look;
  const take = () => {
    m.refreshPresentation();
    const state = session.state;
    if (state.look !== seenLook || state.phase === "closed" || state.phase === "refused" || (!state.snapshot?.chrome.history && (state.inputRequest || state.questions.length))) {
      m.invalidate();
      seenLook = state.look;
    }
    const snap = state.snapshot;
    if (!snap || snap === seen) return;
    seen = snap;
    const mv = (snap as { motion?: MotionView }).motion;
    if (!mv) { m.setLevel("off"); return; }
    m.setSlots(mv.slots);
    m.setLevel(mv.level);
    for (const c of mv.cues ?? []) m.cue(cueOf(c));
  };
  const offSession = session.subscribe(take);
  const offInput = session.onInput(() => m.input());
  const offIntent = session.onIntent((seq, cursor) => m.intent(seq, cursor));
  const rawNotices = new RawNoticeInput((escape) => session.noticeInput(escape));
  const offRaw = mouse.onRawInput((text) => { if (isInput(text)) m.input(); rawNotices.take(text); });
  m.marksFrom(() =>
    shellMarks(session, stdout.columns ?? 80, stdout.rows ?? 24),
  );
  // A frame tick pokes the session (a new state object): the app draws, Ink calls Output.get, and `apply` runs.
  const offTick = m.subscribe(() =>
    (session as unknown as { set(p: object): void }).set({}),
  );
  take();
  return () => {
    offSession();
    offInput();
    offIntent();
    offRaw();
    rawNotices.reset();
    offTick();
    m.reset();
  };
}
