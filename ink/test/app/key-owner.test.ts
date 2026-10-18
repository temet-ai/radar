import { describe, expect, it } from "vitest";
import { dockOf, dockShown, resolveKeyOwner, type KeyOwnerSnapshot } from "../../src/app/key-owner.js";

// Pure view values: ownership needs no host, clock, environment, global look or rendered frame.
const screen = () => ({ panes: ["nav", "middle", "detail"].map((pane) => ({ pane, title: pane, body: { node: "document" } })) });
const snapshot = (): KeyOwnerSnapshot => ({ chrome: { keys_at: "body" }, screen: screen(), focus: "middle" });
const focusedDock = () => ({ visible: true, focused: true });
const body = (pane = "middle") => ({ body: true, dock: false, pane, blocked: false });
const none = (blocked = false) => ({ body: false, dock: false, blocked });

describe("live key ownership", () => {
  it("uses the actual focused pane and leaves chrome keys without a body owner", () => {
    for (const pane of ["nav", "middle", "detail"]) {
      expect(resolveKeyOwner({ snapshot: { ...snapshot(), focus: pane } })).toEqual(body(pane));
    }
    for (const keys_at of ["bar", "sources", "page_tabs"]) {
      expect(resolveKeyOwner({ snapshot: { ...snapshot(), chrome: { keys_at } } })).toEqual(none());
    }
    expect(resolveKeyOwner({})).toEqual(none(true));
    expect(resolveKeyOwner({ snapshot: { ...snapshot(), screen: null } })).toEqual(none());
  });

  it("uses caret zero and the first active field, including bottom fields", () => {
    const snap = snapshot();
    snap.focus = "nav";
    snap.screen.panes[0].fields = [{ caret: null }, {}];
    snap.screen.panes[1].bottom_field = { caret: 0 };
    snap.screen.panes[2].fields = [{ caret: 4 }];
    expect(resolveKeyOwner({ snapshot: snap })).toEqual(body("middle"));
    snap.screen.panes[1].bottom_field.caret = null;
    expect(resolveKeyOwner({ snapshot: snap })).toEqual(body("detail"));
  });

  it("resolves page fields instead of a hidden screen's caret or dock", () => {
    const snap = snapshot();
    snap.screen.panes[0].fields = [{ caret: 0 }];
    snap.screen.dock = focusedDock();
    snap.screen.page = { panes: [{ pane: "detail", bottom_field: { caret: 0 } }] };
    expect(dockOf(snap)).toBe(snap.screen.dock);
    expect(dockShown(snap)).toBeNull();
    expect(resolveKeyOwner({ snapshot: snap, pendingDock: { base: snap, focused: true } })).toEqual(body("detail"));
    snap.screen.page.panes = [];
    expect(resolveKeyOwner({ snapshot: snap })).toEqual(body());
  });

  it("keeps ordinary settings focus, while setup's card owns its body and hides the dock", () => {
    const snap: KeyOwnerSnapshot = { chrome: { keys_at: "body" }, section: { body: { node: "list" } }, focus: "detail" };
    expect(resolveKeyOwner({ snapshot: snap })).toEqual(body("detail"));
    snap.dock = focusedDock();
    expect(dockShown(snap)).toBe(snap.dock);
    expect(resolveKeyOwner({ snapshot: snap })).toEqual({ body: false, dock: true, blocked: false });
    snap.chrome!.setup = { title: "set up Watch" };
    expect(dockShown(snap)).toBeNull();
    expect(resolveKeyOwner({ snapshot: snap })).toEqual(body());
  });

  it("honors the forced-focused connection setup pane without giving it chrome keys", () => {
    const snap = { ...snapshot(), focus: "nav", screen: { panes: [{ pane: "middle", title: "set up Watch" }] } };
    expect(resolveKeyOwner({ snapshot: snap })).toEqual(body());
    expect(resolveKeyOwner({ snapshot: { ...snap, chrome: { keys_at: "bar" } } })).toEqual(none());
  });

  it("uses pending dock focus and blur immediately, but only on their exact visible base", () => {
    const snap = snapshot();
    snap.screen.dock = { visible: true, focused: false };
    const pendingDock = { base: snap, focused: true };
    expect(resolveKeyOwner({ snapshot: snap, pendingDock })).toEqual({ body: false, dock: true, blocked: false });
    expect(resolveKeyOwner({ snapshot: { ...snap }, pendingDock })).toEqual(body());
    snap.screen.dock.focused = true;
    expect(resolveKeyOwner({ snapshot: snap, pendingDock: { base: snap, focused: false } })).toEqual(body());
    snap.screen.dock.visible = false;
    expect(resolveKeyOwner({ snapshot: snap, pendingDock })).toEqual(body());
  });

  it("gives the shown dock keys before the bar, while work areas ignore its stale focus", () => {
    const snap = snapshot();
    snap.screen.dock = focusedDock();
    snap.chrome!.keys_at = "bar";
    expect(resolveKeyOwner({ snapshot: snap })).toEqual({ body: false, dock: true, blocked: false });
    snap.chrome!.keys_at = "body";
    snap.screen.panes[1].body = { node: "extension", capability: "work.watch" };
    expect(dockShown(snap)).toBeNull();
    expect(resolveKeyOwner({ snapshot: snap })).toEqual(body());
  });

  it.each(["menuOpen", "editorOpen", "inputRequested"] as const)("blocks body and dock for %s", (flag) => {
    const snap = snapshot();
    snap.screen.dock = focusedDock();
    expect(resolveKeyOwner({ snapshot: snap, [flag]: true })).toEqual(none(true));
  });

  it("blocks unknown screen overlays, tours and splash without inspecting their content", () => {
    const base = snapshot();
    for (const snap of [
      { ...base, screen: { ...base.screen, overlay: { body: { node: "extension" } } } },
      { ...base, chrome: { ...base.chrome, tour: {} } },
      { ...base, splash: {} },
    ]) expect(resolveKeyOwner({ snapshot: snap })).toEqual(none(true));
  });

  it("lets history own its panels ahead of hidden menus, requests, editors and docks", () => {
    for (const focus of ["events", "detail"]) {
      const snap = snapshot();
      snap.screen.dock = focusedDock();
      snap.screen.overlay = {};
      snap.chrome!.history = { focus };
      expect(resolveKeyOwner({ snapshot: snap, menuOpen: true, editorOpen: true, inputRequested: true })).toEqual({ body: false, dock: false, pane: focus, blocked: false });
      snap.chrome!.history.help = true;
      expect(resolveKeyOwner({ snapshot: snap })).toEqual({ ...none(true), pane: undefined });
    }
  });

  it("preserves unknown optimistic focus instead of inventing a middle-pane owner", () => {
    expect(resolveKeyOwner({ snapshot: { ...snapshot(), focus: null } })).toEqual({ ...body(), pane: undefined });
  });
});
