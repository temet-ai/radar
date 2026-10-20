import type { Connection } from "../transport/connection.js";
import type { Capabilities } from "../protocol/generated/Capabilities.js";
import type { MotionView } from "../protocol/generated/MotionView.js";
import type { SplashView } from "../protocol/generated/SplashView.js";
import { NoticeStore } from "../notices/store.js";
import type { Hit } from "../chrome/types.js";
import type { ChromeView, Intent, MenuView } from "../chrome/types.js";
import { applyHostLook, type HostLook } from "../text/look.js";
import { tracer } from "./trace.js";
import { takeTranscript, type HeldTranscript } from "../dock/transcript.js";

/** The protocol and view contract this renderer speaks (crates/radar-core/src/ipc.rs `PROTOCOL`, view::VERSION). */
export const PROTOCOL = 1;
export const VIEW_VERSION = 1;

export const EXIT_OK = 0;
export const EXIT_ERROR = 1;
export const EXIT_NO_SOCKET = 2;
export const EXIT_REFUSED = 3;
/** How long a question's drawing waits for the snapshot the host sends after it (Session.set). */
export const QUESTION_WAIT_MS = 30;

/** The PaneView.scroll each drawn screen pane (`scope/pane`) was last given to take: a Rows report waits for it. */
export const appliedScroll = new Map<string, number | null | undefined>();

export type SnapshotState = {
  cursor: number;
  chrome: ChromeView;
  screen: any | null;
  section: any | null;
  /** The agent dock while settings show (Snapshot.dock); a screen carries its own. */
  dock?: any | null;
  focus: unknown;
  /** Host-owned motion cues and interaction provenance, when supported by the snapshot. */
  motion?: MotionView;
  /** The startup splash while it shows (Snapshot.splash), drawn in place of everything. */
  splash?: SplashView;
};

export type QuestionState = { id: number; cursor: number; menu: MenuView };
export type InputRequestState = { id: number; cursor: number; title: string; value: string };

export type SessionState = {
  phase: "connecting" | "ready" | "refused" | "closed";
  snapshot?: SnapshotState;
  /** Questions in the order asked; none is lost while the user is busy. */
  questions: QuestionState[];
  /** Counts the host's look messages, so a new theme redraws. */
  look?: number;
  notices: string[];
  inputRequest?: InputRequestState;
  /** The refusal or close message to print before exiting. */
  message?: string;
  exitCode?: number;
};

export function capabilitiesFrom(
  env: NodeJS.ProcessEnv,
  stdout: { columns?: number; rows?: number },
): Capabilities {
  const kitty = env.KITTY_WINDOW_ID !== undefined || env.TERM_PROGRAM === "ghostty";
  const colour = env.COLORTERM === "truecolor" || env.COLORTERM === "24bit";
  return {
    columns: stdout.columns ?? 80,
    rows: stdout.rows ?? 24,
    mouse: true,
    true_colour: colour,
    image: kitty ? "kitty" : "none",
    plain_text: env.RADAR_PLAIN_TEXT === "1",
  };
}

/**
 * The renderer's end of the protocol: Hello, Welcome or Refuse, the newest Snapshot (acknowledged
 * once it is held), questions, notices and input requests that are never dropped, and the
 * Intents the user's input becomes, each stamped with the cursor of the snapshot it was seen at.
 */
export class Session {
  state: SessionState = { phase: "connecting", questions: [], notices: [] };
  private listeners = new Set<() => void>();
  private inputListeners = new Set<() => void>();
  private intentListeners = new Set<(seq: number, cursor: number) => void>();
  private seq = 0;
  /** The dock's transcript as last taken in; the host sends only what changed after it. */
  private transcript: HeldTranscript | undefined;
  private resync = false;
  private noticeProtocol = false;
  readonly noticeStore: NoticeStore;
  /** Hits from the last actually composited frame, never an unpainted snapshot. */
  noticeHits: Hit[] = [];

  constructor(
    private readonly conn: Connection,
    private readonly caps: Capabilities,
    private readonly onClipboard: (text: string) => void = () => {},
    /** Tell listeners once per burst of messages (the real renderer); tests default to one call per change. */
    private readonly batch = false,
    noticeClock?: () => number,
  ) {
    this.noticeStore = new NoticeStore(noticeClock);
    conn.onMessage((m) => this.receive(m as any));
    conn.onClose((reason) => {
      if (this.state.phase === "closed" || this.state.phase === "refused") return;
      this.set({ phase: "closed", message: `host connection closed (${reason})`, exitCode: EXIT_ERROR });
    });
  }

  get plainText(): boolean {
    return this.caps.plain_text;
  }

  start() {
    this.conn.send({ type: "hello", protocol: PROTOCOL, view: VIEW_VERSION, capabilities: this.caps });
  }

  subscribe(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  /** Local actionable input, including keys handled without a host intent. Live only: nothing is queued or replayed. */
  onInput(fn: () => void): () => void {
    this.inputListeners.add(fn);
    return () => void this.inputListeners.delete(fn);
  }

  /** Called before an event is handled, so local edits and scrolling can cancel an older visual transition too. */
  input(): void {
    for (const fn of [...this.inputListeners]) fn();
  }

  /** The exact wire identity of an intent, observed once before the connection can answer it. */
  onIntent(fn: (seq: number, cursor: number) => void): () => void {
    this.intentListeners.add(fn);
    return () => void this.intentListeners.delete(fn);
  }

  /** The state changes at once; with `batch`, listeners hear of a burst of changes (messages read together) once, so a
   * burst is one React commit and one Ink frame, not one each. */
  private set(patch: Partial<SessionState>) {
    // A question the host asked is followed by the snapshot that carries its menu (bridge.rs ask_and_tell) a few ms
    // later: drawing the question alone and then the snapshot was two full frames for one right-click. Its drawing waits
    // for that snapshot, or QUESTION_WAIT_MS when none comes; anything else that changes draws at once, the question
    // with it. The state itself changes now (keys after it are the question's).
    const asked = patch.questions !== undefined && patch.questions.length > this.state.questions.length && Object.keys(patch).length === 1;
    this.state = { ...this.state, ...patch };
    if (!this.batch) {
      for (const l of [...this.listeners]) l();
      return;
    }
    if (asked && !this.notifyQueued) {
      this.held ??= setTimeout(() => this.notify(), QUESTION_WAIT_MS);
      return;
    }
    if (this.notifyQueued) return;
    this.notifyQueued = true;
    queueMicrotask(() => this.notify());
  }
  private notify() {
    if (this.held !== undefined) clearTimeout(this.held);
    this.held = undefined;
    this.notifyQueued = false;
    for (const l of [...this.listeners]) l();
  }
  private notifyQueued = false;
  private held: ReturnType<typeof setTimeout> | undefined;

  private receive(m: any) {
    switch (m?.type) {
      case "welcome":
        this.set({ phase: "ready" });
        break;
      case "refuse":
        this.set({
          phase: "refused",
          message: String(m.message ?? `host refused protocol ${m.got} (wants ${m.wanted})`),
          exitCode: EXIT_REFUSED,
        });
        break;
      case "snapshot": {
        const { type: _t, ...snap } = m;
        tracer.snapshot(m.cursor, m.trace);
        this.noticeProtocol ||= !!snap.chrome && (snap.chrome.notices !== undefined || snap.chrome.notices_visible !== undefined || snap.chrome.notice_history !== undefined);
        if (snap.chrome?.notices) this.noticeStore.reconcile(snap.chrome.notices);
        // The dock's transcript comes as what changed since the last one (dock/transcript.ts): made whole here.
        const dock = snap.screen?.dock ?? snap.dock;
        if (dock) {
          const took = takeTranscript(this.transcript, dock);
          this.transcript = took.held;
          if (!took.ok) {
            this.transcript = undefined;
            this.resync = true;
          }
        }
        // The snapshot is the truth for which menu is open (the host sends every menu it opens as a question, and a
        // click on a menu row closes it without an intent that names it). A question asked at or before this
        // snapshot that is not the chrome's menu now is closed; later ones are not in this snapshot yet.
        const open = (snap as SnapshotState).chrome?.menu?.id;
        const questions = this.state.questions.filter((q) => q.cursor > m.cursor || q.menu.id === open);
        // An input request is closed the same way: a snapshot at or after it with no typed menu open. Kept, it was
        // drawn over the body and took every key as typing after ⏎ or esc had closed it in the host.
        const req = this.state.inputRequest;
        const closed = req !== undefined && req.cursor <= m.cursor && (snap as SnapshotState).chrome?.menu?.typed == null;
        this.set({
          snapshot: snap as SnapshotState,
          ...(questions.length === this.state.questions.length ? {} : { questions }),
          ...(closed ? { inputRequest: undefined } : {}),
        });
        this.conn.send({ type: "ack", cursor: m.cursor });
        if (this.resync) {
          this.resync = false;
          this.conn.send({ type: "resync" });
        }
        break;
      }
      case "look":
        // The host's resolved theme and icon pack: every slot's colour, every icon's glyph. Drawing re-reads them.
        applyHostLook(m as unknown as HostLook);
        this.set({ look: (this.state.look ?? 0) + 1 });
        break;
      case "question":
        this.set({ questions: [...this.state.questions, { id: m.id, cursor: m.cursor, menu: m.menu }] });
        break;
      case "input_request":
        this.set({ inputRequest: { id: m.id, cursor: m.cursor, title: m.title, value: m.value } });
        break;
      case "notice":
        if (m.occurrence) {
          this.noticeProtocol = true;
          this.noticeStore.immediate(m.occurrence, m.revision ?? 0, m.now_ms ?? 0);
          this.set({});
        } else this.set({ notices: [...this.state.notices, String(m.text)].slice(-256) });
        break;
      case "clipboard":
        this.onClipboard(String(m.text));
        break;
      case "shutdown":
        this.set({ phase: "closed", exitCode: EXIT_OK, message: undefined });
        break;
      default:
        this.conn.send({ type: "log", level: "warn", text: `unknown message ${String(m?.type)}` });
    }
  }

  get cursor(): number {
    return this.state.snapshot?.cursor ?? 0;
  }

  sendIntent(intent: Intent) {
    const seq = ++this.seq;
    const cursor = this.cursor;
    // Answering a question clears it here; the host's next snapshot is the truth.
    if (intent.intent === "invoke" && this.state.questions[0]?.menu.items.some((c) => c.value === intent.action)) {
      this.set({ questions: this.state.questions.slice(1) });
    } else if (intent.intent === "key" && intent.chord.key === "esc" && this.state.questions.length > 0) {
      this.set({ questions: this.state.questions.slice(1) });
    } else if (intent.intent === "activate" && intent.target.startsWith("chrome:menu:") && this.state.questions.length > 0) {
      // A click on a menu row chooses it and closes the menu.
      this.set({ questions: this.state.questions.slice(1) });
    }
    tracer.intent(seq);
    for (const fn of [...this.intentListeners]) fn(seq, cursor);
    this.conn.send({ type: "intent", seq, cursor, intent });
  }

  /** The cell a right-click pressed, for the Context intent that follows (ToHost::Pointer). */
  sendPointer(column: number, row: number) {
    this.conn.send({ type: "pointer", column, row });
  }

  resize(columns: number, rows: number) {
    this.conn.send({ type: "resize", columns, rows });
  }

  private rowsSent = "";
  /** What the scrolled panes of screen `scope` laid out (ToHost::Rows): sent only when it changed. Nothing is sent
   * while a drawn pane has not yet taken the PaneView.scroll the latest snapshot gives it (the report would put the
   * part's scroll back where it was), and a pane the snapshot has that is not drawn (detail hidden on a narrow screen)
   * is reported with rows 0, so the part does not page a pane nobody sees. */
  sendRows(scope: string, drawn: Array<{ pane: "nav" | "middle" | "detail"; rows: number; content: number; at: number }>) {
    const snapPanes: any[] = this.state.snapshot?.screen?.panes ?? [];
    const behind = drawn.some((d) => {
      const want = snapPanes.find((p) => p.pane === d.pane)?.scroll;
      return want !== undefined && want !== null && appliedScroll.get(`${scope}/${d.pane}`) !== want;
    });
    if (behind) return;
    const panes = [...drawn];
    for (const p of snapPanes) {
      const scrolls = p.body?.node === "document" || (p.body?.node === "extension" && p.body?.capability === "detail");
      if (scrolls && !drawn.some((d) => d.pane === p.pane)) panes.push({ pane: p.pane, rows: 0, content: 0, at: 0 });
    }
    panes.sort((a, b) => a.pane.localeCompare(b.pane));
    const key = JSON.stringify([scope, panes]);
    if (key === this.rowsSent) return;
    this.rowsSent = key;
    this.conn.send({ type: "rows", scope, panes });
  }

  /** Presentation-only input also accompanies keys consumed by a local editor or scroll. */
  noticeInput(escape: boolean) {
    if (this.noticeStore.input(escape)) this.set({});
    if (this.noticeProtocol) this.sendIntent({ intent: "notice_input", escape, through_id: this.noticeStore.throughId });
    this.dismissNotices();
  }

  dismissNoticeId(id: number) {
    if (!this.noticeStore.dismiss(id)) return;
    this.set({});
    this.sendIntent({ intent: "dismiss_notice", id });
  }

  dismissNotices() {
    if (this.state.notices.length > 0) this.set({ notices: [] });
  }

  dismissNotice() {
    if (this.state.notices.length > 0) this.set({ notices: this.state.notices.slice(1) });
  }

  log(level: "debug" | "info" | "warn" | "error", text: string) {
    this.conn.send({ type: "log", level, text: text.slice(0, 1000) });
  }

  bye() {
    this.conn.send({ type: "bye" });
  }
}
