import { typingPane } from "../input/route.js";

/** The renderer state ownership needs, without clocks, frame geometry or a retained snapshot. */
export type KeyOwnerSnapshot = {
  chrome?: {
    keys_at?: string;
    history?: { focus?: string; help?: boolean } | null;
    notice_history?: unknown;
    setup?: unknown;
    tour?: unknown;
  };
  screen?: any;
  section?: any;
  dock?: any;
  focus?: unknown;
  splash?: unknown;
};

export type PendingDockFocus = { base: KeyOwnerSnapshot | undefined; focused: boolean };
export type KeyOwner = { body: boolean; dock: boolean; pane?: string; blocked: boolean };
export type KeyOwnerInput = {
  snapshot?: KeyOwnerSnapshot;
  pendingDock?: PendingDockFocus | null;
  menuOpen?: boolean;
  editorOpen?: boolean;
  inputRequested?: boolean;
};

/** A screen carries its dock; settings carry theirs beside the section. */
export const dockOf = (snapshot: KeyOwnerSnapshot | undefined): any => snapshot?.screen?.dock ?? snapshot?.dock ?? null;

/** Only a dock with a place in the current body can own keys. Setup, item pages and work areas do not draw it. */
export function dockShown(snapshot: KeyOwnerSnapshot | undefined): any {
  const dock = dockOf(snapshot);
  if (!dock?.visible) return null;
  if (snapshot?.section) return snapshot.chrome?.setup ? null : dock;
  const screen = snapshot?.screen;
  const work = (screen?.panes ?? []).some((pane: any) => pane?.body?.node === "extension" && String(pane.body.capability).startsWith("work."));
  return screen && !screen.page && !work ? dock : null;
}

/**
 * One live answer for routing and frame ownership. Pending dock focus belongs only to the exact snapshot it changed;
 * a newer snapshot wins. A caret owns its pane even while the legacy focus highlight still names another pane.
 */
export function resolveKeyOwner({ snapshot, pendingDock, menuOpen = false, editorOpen = false, inputRequested = false }: KeyOwnerInput): KeyOwner {
  const none = (blocked: boolean): KeyOwner => ({ body: false, dock: false, blocked });
  if (!snapshot || snapshot.splash || snapshot.chrome?.tour) return none(true);
  if (snapshot.chrome?.notice_history) return none(true);
  const history = snapshot.chrome?.history;
  // History replaces the entire body, including menus and requests waiting underneath it.
  if (history) return { body: false, dock: false, pane: history.help ? undefined : history.focus, blocked: !!history.help };
  if (menuOpen || editorOpen || inputRequested || snapshot.screen?.overlay) return none(true);
  const shown = dockShown(snapshot);
  const dock = !!shown && (pendingDock?.base === snapshot ? pendingDock.focused : !!shown.focused);
  if (dock) return { body: false, dock: true, blocked: false };
  if ((snapshot.chrome?.keys_at ?? "body") !== "body" || (!snapshot.section && !snapshot.screen)) return none(false);
  const focus = typeof snapshot.focus === "string" ? snapshot.focus : undefined;
  if (snapshot.section) return { body: true, dock: false, pane: snapshot.chrome?.setup ? "middle" : focus, blocked: false };
  const screen = snapshot.screen;
  const panes = screen.page ? (screen.page.panes ?? []) : (screen.panes ?? []);
  // The connection setup screen renders its single pane focused even when legacy focus remains on nav.
  const setup = !screen.page && panes.length === 1 && String(panes[0]?.title).startsWith("set up ");
  return { body: true, dock: false, pane: typingPane(screen) ?? (setup ? panes[0].pane : focus), blocked: false };
}
