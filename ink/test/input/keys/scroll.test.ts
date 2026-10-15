import fs from "node:fs";
import { describe, expect, it } from "vitest";
import { scrollKey, workingFrom, type ScrollerLike } from "../../../src/input/route.js";
import { InputController, type InputView } from "../../../src/input/controller.js";
import { UiState } from "../../../src/state/ui-state.js";
import type { Chord, Intent } from "../../../src/chrome/types.js";

/**
 * Class: panes the frontend scrolls itself (view_tui::key → ui::scroll::Scroll::key). A focused Document or Rich pane
 * takes ↓ j ↑ k PgUp PgDn g G Home End; ↑/k at the top is the host's (the top bar); PgUp/PgDn on a screen without a
 * page scroll the detail Document; a Rich pane scrolled to its end sends Intent::End.
 */
function scroller(kind: "document" | "rich", o: { at?: number; max?: number; focused?: boolean } = {}): ScrollerLike & { log: string[] } {
  let at = o.at ?? 0;
  const max = o.max ?? 10;
  const log: string[] = [];
  return {
    kind,
    focused: o.focused ?? true,
    log,
    at: () => at,
    max: () => max,
    key(name) {
      log.push(name);
      if ((name === "up" || name === "k") && at === 0) return false;
      if (name === "down" || name === "j") at = Math.min(max, at + 1);
      else if (name === "up" || name === "k") at -= 1;
      else if (name === "end" || name === "G") at = max;
      else if (name === "home" || name === "g") at = 0;
      else if (name === "pageDown") at = Math.min(max, at + 5);
      else if (name === "pageUp") at = Math.max(0, at - 5);
      else return false;
      return true;
    },
    page(down) {
      log.push(down ? "page:down" : "page:up");
      const to = down ? Math.min(max, at + 5) : Math.max(0, at - 5);
      const moved = to !== at;
      at = to;
      return moved;
    },
  };
}

const screen = (body: string, extra: object = {}) => ({ panes: [{ pane: "middle", body: { node: body } }], ...extra });
const c = (k: Chord["key"], over: Partial<Chord> = {}): Chord => ({ key: k, ...over });
const ch = (x: string): Chord => c({ char: x });

describe("a focused Document scrolls in the frontend", () => {
  it.each([
    [ch("j"), "j"], [c("down"), "down"], [ch("k"), "k"], [c("up"), "up"], [c("page_down"), "pageDown"], [c("page_up"), "pageUp"],
    [c("home"), "home"], [ch("g"), "g"], [c("end"), "end"], [ch("G"), "G"],
  ])("%j is Scroll::key(%s) and sends nothing", (chord, name) => {
    const s = scroller("document", { at: 3 });
    expect(scrollKey(screen("document"), "middle", chord, [s])).toBe("done");
    expect(s.log).toEqual([name]);
  });

  it("↑ and k at the top are the host's (the top bar)", () => {
    for (const chord of [c("up"), ch("k")]) {
      expect(scrollKey(screen("document"), "middle", chord, [scroller("document", { at: 0 })])).toBeUndefined();
    }
  });

  it("other keys, modified keys, an overlay and a typed field are not the scroll's", () => {
    const s = scroller("document", { at: 3 });
    expect(scrollKey(screen("document"), "middle", ch("x"), [s])).toBeUndefined();
    expect(scrollKey(screen("document"), "middle", c("down", { ctrl: true }), [s])).toBeUndefined();
    expect(scrollKey(screen("document", { overlay: {} }), "middle", c("down"), [s])).toBeUndefined();
    const typing = { panes: [{ pane: "middle", body: { node: "document" }, fields: [{ caret: 2 }] }] };
    expect(scrollKey(typing, "middle", c("down"), [s])).toBeUndefined();
    expect(s.log).toEqual([]);
  });

  it("a pane that is not focused does not scroll", () => {
    const s = scroller("document", { focused: false });
    expect(scrollKey(screen("document"), "middle", c("down"), [s])).toBeUndefined();
  });
});

describe("a Rich pane", () => {
  it("scrolls, and at its end the part may bring more (Intent::End)", () => {
    const s = scroller("rich", { at: 9, max: 10 });
    expect(scrollKey(screen("rich"), "middle", c("down"), [s])).toEqual({ intent: "end", pane: "middle" });
    const mid = scroller("rich", { at: 2, max: 10 });
    expect(scrollKey(screen("rich"), "middle", c("down"), [mid])).toBe("done");
  });
});

describe("PgUp/PgDn on a screen without a page scroll the detail Document", () => {
  const withDetail = (extra: object = {}) => ({
    panes: [{ pane: "middle", body: { node: "list", rows: [] } }, { pane: "detail", body: { node: "document" } }],
    ...extra,
  });
  it("whichever pane has the focus", () => {
    const d = scroller("document", { focused: false });
    expect(scrollKey(withDetail(), "middle", c("page_down"), [d])).toBe("done");
    expect(d.log).toEqual(["page:down"]);
  });
  it("not when the screen has a page", () => {
    const d = scroller("document", { focused: false });
    expect(scrollKey(withDetail({ page: {} }), "middle", c("page_down"), [d])).toBeUndefined();
  });
  it("a detail that cannot move leaves the key to the host", () => {
    const d = scroller("document", { focused: false, at: 10, max: 10 });
    expect(scrollKey(withDetail(), "middle", c("page_down"), [d])).toBeUndefined();
  });
});

describe("the controller", () => {
  const ctl = (scroll: InputView["scroll"], sent: Intent[]) =>
    new InputController(
      { menu: () => null, capturing: () => false, route: (chord) => ({ intent: "key", chord }), hits: () => [], scroll },
      (i) => sent.push(i),
      new UiState(),
      () => 0,
    );
  it("sends nothing for a key the frontend scrolled, the End intent, and routes the rest", () => {
    const sent: Intent[] = [];
    const answers: Array<Intent | "done" | undefined> = ["done", { intent: "end", pane: "middle" }, undefined];
    const ctrl = ctl(() => answers.shift(), sent);
    ctrl.keyChord(c("down"));
    ctrl.keyChord(c("down"));
    ctrl.keyChord(c("down"));
    expect(sent).toEqual([{ intent: "end", pane: "middle" }, { intent: "key", chord: c("down") }]);
  });
  it("a menu or a typed field keeps its keys", () => {
    const sent: Intent[] = [];
    const ctrl = new InputController(
      { menu: () => null, capturing: () => true, route: (x) => ({ intent: "key", chord: x }), hits: () => [], scroll: () => "done" },
      (i) => sent.push(i),
      new UiState(),
      () => 0,
    );
    ctrl.keyChord(c("down"));
    expect(sent).toEqual([{ intent: "key", chord: c("down") }]);
  });
});

describe("an item page's panes (screen.page.panes, view_tui::panes)", () => {
  const file = new URL("../../fixtures/gitlab_repos-page1-overview-changes-threads-pipeline-commits-ticket_160x40.json", import.meta.url);
  const page = JSON.parse(fs.readFileSync(file, "utf8"));
  it("the fixture's screen has its panes under page, none at the top", () => {
    expect(page.panes ?? []).toEqual([]);
    expect(page.page.panes.map((p: any) => p.pane)).toEqual(["middle", "detail"]);
  });
  it("the focused Rich pane of the page takes j and PageDown", () => {
    for (const chord of [ch("j"), c("down"), c("page_down")]) {
      const s = scroller("rich", { at: 2, max: 10 });
      expect(scrollKey(page, "middle", chord, [s])).toBe("done");
      expect(s.log).toHaveLength(1);
    }
  });
  it("the page's detail Document takes keys when it has the focus", () => {
    const s = scroller("document", { at: 2, max: 10 });
    expect(scrollKey(page, "detail", c("down"), [s])).toBe("done");
  });
});

describe("keys that arrive together start from the newest snapshot (workingFrom)", () => {
  const screenBefore = { screen: { panes: [{ pane: "middle", body: { node: "tree" } }] } };
  const pageOpened = { screen: { panes: [], page: { panes: [{ pane: "middle", body: { node: "rich" } }] } } };
  it("a newer snapshot replaces the working copy, so the first key after a page opens is routed on the page", () => {
    // the keys before: working was advanced from the old snapshot
    const old = workingFrom(undefined, screenBefore)!;
    old.working = { ...screenBefore, advanced: true } as any;
    const now = workingFrom(old, pageOpened)!;
    expect(now.working).toBe(pageOpened);
    const s = scroller("rich", { at: 2 });
    expect(scrollKey((now.working as any).screen, "middle", c("page_down"), [s])).toBe("done");
  });
  it("the same snapshot keeps what the previous key left", () => {
    const p = workingFrom(undefined, screenBefore)!;
    p.working = { ...screenBefore, advanced: true } as any;
    expect(workingFrom(p, screenBefore)!.working).toEqual({ ...screenBefore, advanced: true });
  });
  it("no snapshot yet: nothing to route", () => {
    expect(workingFrom(undefined, undefined)).toBeUndefined();
  });
});
