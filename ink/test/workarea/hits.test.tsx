// Clicking what the work area draws sends the reference's intent. In the reference the work.* sections are their
// fallback documents, so the only targets there are the ones the part gives each row (the map's items,
// the same `item` targets the list panes use); a press selects the target (the reference's click).
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import {
  chromeOf,
  mount,
  tick,
  until,
  unmountAll,
  type Mounted,
} from "../contract/support.js";

afterEach(unmountAll);

const read = (name: string) =>
  JSON.parse(
    readFileSync(
      fileURLToPath(
        new URL(
          `../../../crates/radar-workbench/tests/fixtures/${name}`,
          import.meta.url,
        ),
      ),
      "utf8",
    ),
  );
const FIXTURES = ["work_view.json"]; // the pending fixture has no map items and the reference draws no target on its question

async function press(m: Mounted, x: number, y: number) {
  const before = m.intents().length;
  // a press and its release, as a terminal sends them: a click in a section's text acts when let go (runner.rs)
  m.type(`\x1b[<0;${x + 1};${y + 1}M\x1b[<0;${x + 1};${y + 1}m`);
  await until(() => m.intents().length > before, 150);
  await tick(10);
  return m.intents().slice(before);
}

const mapItems = (screen: any): Array<{ target: string; title: string }> =>
  screen.panes.find((p: any) => p.body?.capability === "work.map")?.body.data
    .items ?? [];

for (const [w, h] of [
  [160, 40],
  [100, 30],
] as const) {
  describe.each(FIXTURES)(`${w}x${h} %s`, (name) => {
    it("the app draws no map items while the map holds no question (the reference draws only nav/middle/detail)", async () => {
      const screen = read(name);
      const items = mapItems(screen);
      expect(items.length).toBeGreaterThan(0);
      const m = mount(w, h);
      m.snapshot({ chrome: chromeOf(w, h), screen, focus: "middle" });
      await m.settle();
      for (const item of items)
        expect(m.lines().some((l) => l.includes(`${item.title} · `))).toBe(
          false,
        );
    });

    it("a press on each map item sends select with the item's target while the map holds a question", async () => {
      const screen = read(name);
      screen.panes.find(
        (p: any) => p.body?.capability === "work.map",
      ).body.data.pending = true;
      const items = mapItems(screen);
      const m = mount(w, h);
      m.snapshot({ chrome: chromeOf(w, h), screen, focus: "middle" });
      await m.settle();
      const seen: string[] = [];
      for (const item of items) {
        const lines = m.lines();
        const y = lines.findIndex((l) => l.includes(`${item.title} · `));
        if (y < 0) continue; // clipped by the screen height: the host's own scroll, not this test
        const x = lines[y]!.indexOf(item.title);
        expect(await press(m, x, y)).toEqual([
          { intent: "select", target: item.target },
        ]);
        seen.push(item.target);
      }
      expect(seen.length).toBeGreaterThan(0);
    });
  });
}

// #25: the panes beside the work area's sections are resized by their borders, as on every screen.
it("a drag of the work screen's nav border sends the split", async () => {
  const m = mount(160, 40);
  m.snapshot({ chrome: chromeOf(160, 40), screen: read("work_view.json"), focus: "middle" });
  await m.settle();
  const y = m.lines().findIndex((l) => l.includes("╮╭"));
  const x = m.lines()[y]!.indexOf("╮╭");
  const before = m.intents().length;
  m.type(`\x1b[<0;${x + 1};${y + 3}M\x1b[<32;${x + 11};${y + 3}M\x1b[<0;${x + 11};${y + 3}m`);
  await until(() => m.intents().length > before, 500);
  expect(m.intents().slice(before)).toEqual([{ intent: "layout", layout: "split", edge: "nav", share: Math.round(((x + 11) * 1000) / 160) }]);
});

// GUIDE 2.12: a chevron is never a focus, selection or fold marker, in any section, in either mode.
describe("no chevron marks focus, selection or folds in the work area", () => {
  const CHEVRON = /(^|\s)[›▸▶❯>»](\s|$)/m;
  it.each(["work_view.json", "work_view_pending.json"])(
    "%s at 80x24 and 160x40, plain or not, focused on each section",
    async (name) => {
      const screen = read(name);
      const targets = screen.panes.map(
        (p: any, i: number) =>
          p.body?.data?.target ?? (i === 0 ? "pane:nav" : undefined),
      );
      const { render } = await import("./render.js");
      const React = (await import("react")).default;
      const { WorkArea } = await import("../../src/workarea/WorkArea.js");
      for (const [w, h] of [
        [80, 24],
        [160, 40],
      ] as const) {
        for (const plain_text of [false, true]) {
          for (const focus of targets) {
            const r = render(
              React.createElement(WorkArea, {
                screen,
                capabilities: { columns: w, rows: h, plain_text },
                focus,
              }),
            );
            const f = r.lastFrame() ?? "";
            r.unmount();
            expect(
              f,
              `${name} ${w}x${h} plain=${plain_text} focus=${focus}`,
            ).not.toMatch(CHEVRON);
          }
        }
      }
    },
  );
});
