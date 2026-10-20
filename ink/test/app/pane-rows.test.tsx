import { afterEach, expect, it } from "vitest";
import { mount, until } from "./harness.js";
import { scrollers } from "../../src/nodes/index.js";
import { booksChrome } from "../chrome/fixture.js";

// #32: Ink reports what each scrolled screen pane laid out (ToHost::Rows), so the part's own scroll knows how far
// there is to go; and goes where the part moved it (PaneView.scroll), once, without undoing its own scrolling.

let stop: (() => void) | undefined;
afterEach(() => { stop?.(); stop = undefined; });

const detail = (scroll?: number) => ({
  pane: "detail", title: "WATCH-12",
  ...(scroll === undefined ? {} : { scroll }),
  body: { node: "extension", capability: "detail", version: 1,
    data: { header: { path: [{ text: "ankh-morpork/watch" }], title: [{ text: "WATCH-12" }] },
      sections: [{ block: "section", title: "REPORT", blocks: Array.from({ length: 40 }, (_, i) => ({ block: "line", text: [{ text: `Sam Vimes, line ${i}` }] })) }] },
    fallback: { node: "document", lines: [] } },
});
const screen = (scroll?: number, page?: unknown) => ({
  scope: "watch",
  panes: [{ pane: "middle", title: "reports", body: { node: "list", selected: 0, rows: [{ target: "report:12", text: [{ text: "WATCH-12" }] }] } }, detail(scroll)],
  ...(page ? { page } : {}),
});
const chrome = (w: number, h: number) => ({ ...booksChrome({ width: w, height: h }), sources: [] });

it("sends the rows each scrolled screen pane laid out, when they change, and goes where the part moved it once", async () => {
  const m = mount(130, 16); stop = m.stop;
  m.welcome();
  m.snapshot({ chrome: chrome(130, 16), screen: screen(), focus: "middle" }, 1);
  const shown = () => scrollers().find((s) => s.pane === "detail");
  await until(() => !!shown()?.max() && m.sent("rows").length > 0);
  const s = shown()!;
  const last = () => m.sent("rows").at(-1);
  // The detail's own numbers: the rows of its body that show and the rows its content took at the pane's width.
  expect(s.height()).toBeGreaterThan(0);
  expect(s.content()).toBeGreaterThan(s.height());
  expect(last()).toEqual({ type: "rows", scope: "watch", panes: [{ pane: "detail", rows: s.height(), content: s.content(), at: 0 }] });
  expect(s.content() - s.height()).toBe(s.max());

  // Ink scrolls it itself (the focused detail's End): the host hears where it is.
  s.key("end");
  await until(() => last()?.panes[0]?.at === s.max());
  expect(last()?.panes[0]?.at).toBe(s.max());
  const sent = m.sent("rows").length;
  // The same snapshot again: nothing changed, nothing is sent.
  m.snapshot({ chrome: chrome(130, 16), screen: screen(), focus: "middle" }, 2);
  await until(() => m.sent("ack").some((a: any) => a.cursor === 2));
  expect(m.sent("ack").some((a: any) => a.cursor === 2)).toBe(true);
  expect(m.sent("rows").length).toBe(sent);

  // The part paged it (PgDn from the tree): the view says where; Ink goes there and says so.
  m.snapshot({ chrome: chrome(130, 16), screen: screen(5), focus: "middle" }, 3);
  await until(() => s.at() === 5 && last()?.panes[0]?.at === 5);
  expect(s.at()).toBe(5);
  expect(last()?.panes[0]).toEqual({ pane: "detail", rows: s.height(), content: s.content(), at: 5 });
  // Ink scrolls back to the top itself; a snapshot still naming 5 does not undo it.
  s.key("home");
  m.snapshot({ chrome: chrome(130, 16), screen: screen(5), focus: "middle" }, 4);
  await until(() => m.sent("ack").some((a: any) => a.cursor === 4) && last()?.panes[0]?.at === 0);
  expect(m.sent("ack").some((a: any) => a.cursor === 4)).toBe(true);
  expect(last()?.panes[0]?.at).toBe(0);
  expect(s.at()).toBe(0);

  // A second part move arrives and a report from before Ink drew it goes out meanwhile: it is held, not sent.
  m.snapshot({ chrome: chrome(130, 16), screen: screen(5), focus: "middle" }, 40);
  await until(() => s.at() === 5 && last()?.panes[0]?.at === 5);
  const held = m.sent("rows").length;
  m.snapshot({ chrome: chrome(130, 16), screen: screen(9), focus: "middle" }, 41);
  m.session.sendRows("watch", [{ pane: "detail", rows: s.height(), content: s.content(), at: 5 }]);
  expect(m.sent("rows").length).toBe(held);
  await until(() => s.at() === 9 && last()?.panes[0]?.at === 9);
  expect(last()?.panes[0]?.at).toBe(9);
  m.snapshot({ chrome: chrome(130, 16), screen: screen(0), focus: "middle" }, 42);
  await until(() => s.at() === 0 && last()?.panes[0]?.at === 0);

  // A page over the screen: its panes are not the screen's, nothing is reported for them.
  const before = m.sent("rows").length;
  m.snapshot({ chrome: chrome(130, 16), screen: screen(undefined, { title: "WATCH-12", panes: [detail()], tabs: [], selected_tab: 0 }), focus: "detail" }, 5);
  await until(() => m.sent("ack").some((a: any) => a.cursor === 5));
  expect(m.sent("ack").some((a: any) => a.cursor === 5)).toBe(true);
  await new Promise((r) => setTimeout(r, 20));
  expect(m.sent("rows").slice(before).every((r: any) => r.panes.length === 0)).toBe(true);
});

it("reports rows 0 for a detail pane it does not draw (a narrow screen shows only the focused pane)", async () => {
  const m = mount(60, 16); stop = m.stop;
  m.welcome();
  m.snapshot({ chrome: chrome(60, 16), screen: screen(), focus: "middle" }, 1);
  await until(() => m.sent("rows").length > 0);
  expect(scrollers().find((s) => s.pane === "detail")).toBeUndefined();
  expect(m.sent("rows").at(-1)).toEqual({ type: "rows", scope: "watch", panes: [{ pane: "detail", rows: 0, content: 0, at: 0 }] });
});
