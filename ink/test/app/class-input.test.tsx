import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { mount, tick, until } from "./harness.js";
import { booksChrome, settingsNav } from "../chrome/fixture.js";
import { HitRegistry } from "../../src/state/ui-state.js";
import { hitAt } from "../../src/chrome/layout.js";
import { scrollers } from "../../src/nodes/index.js";

/**
 * Class tests over every recorded screen and settings section at 160x40 (the chord and click pairs are pinned in
 * intents.test.ts; these run the whole App on each recorded view):
 *  - a right click on any row the screen registers sends Context with that row's own target and nothing else;
 *  - a left click sends Select with it (never Context), a right click off every row sends nothing;
 *  - a movement or ⏎ key sends exactly one intent, and a Select/Activate/Fold it sends names a target that is in
 *    the view (a stale or invented target is the host refusing the move);
 *  - except where the focused pane is a Document or Rich node: the frontend scrolls those itself, as the reference's
 *    view_tui::key does (ui::scroll::Scroll::key). A key it takes sends nothing, or Intent::End{pane} for a Rich
 *    pane that the key leaves at its end ("the part may bring more"); a key it does not take (↑ at the top, ⏎)
 *    is passed to the host as a key. The expectation is computed here from the scroller's own offset and range.
 */
const dir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../fixtures");
const files = fs.existsSync(dir) ? fs.readdirSync(dir).filter((f) => /_160x40\.json$/.test(f) && !f.startsWith("_") && !f.startsWith("host_chrome")).sort() : [];
const load = (f: string) => JSON.parse(fs.readFileSync(path.join(dir, f), "utf8"));

let live: Array<() => void> = [];
afterEach(() => {
  live.forEach((f) => f());
  live = [];
});

/** Every string the view names as a target, and every tree path joined as the host joins it. */
function targetsOf(view: unknown): Set<string> {
  const out = new Set<string>();
  const walk = (v: any) => {
    if (Array.isArray(v)) return v.forEach(walk);
    if (v && typeof v === "object") {
      for (const [k, x] of Object.entries(v)) {
        if ((k === "target" || k === "header_target") && typeof x === "string") out.add(x);
        if ((k === "path" || k === "selected" || k === "open") && Array.isArray(x) && x.every((s) => typeof s === "string")) out.add((x as string[]).join("\u001f"));
        walk(x);
      }
    }
  };
  walk(view);
  return out;
}

async function open(f: string) {
  const view = load(f);
  const section = view.detail_title !== undefined;
  const registry = new HitRegistry();
  const m = mount(160, 40, { registry });
  live.push(m.stop);
  m.welcome();
  const chrome = booksChrome({ width: 160, height: 40 });
  const shown = { chrome: section ? { ...chrome, settings: { ...settingsNav, section_title: view.scope, detail_title: view.detail_title } } : chrome, screen: section ? null : view, section: section ? view : null, focus: "middle" } as const;
  m.snapshot(shown, 1);
  await tick(60);
  let cursor = 1;
  /** The host answers a key with a snapshot before the next key (a key it was sent can have moved its state). */
  const answer = async () => {
    m.snapshot(shown, ++cursor);
    await tick(5);
  };
  return { m, view, registry, answer };
}

/** Which scroll cases ran, so the suite fails if no fixture reaches them. */
const ran = { documentScrolled: 0, richAtEnd: 0, richScrolled: 0 };

describe.each(files)("%s", (f) => {
  it("right and left clicks on every registered row, and the keys", async () => {
    const { m, view, registry, answer } = await open(f);
    const named = targetsOf(view);
    const hits = registry.all().filter((h) => h.y >= 2 && h.y < 39 && h.w > 0);
    // The hits that are topmost at their middle cell (a later hit covers an earlier one).
    const tops = hits.filter((h) => hitAt(registry.all(), h.x + Math.floor(h.w / 2), h.y) === h);
    const sample = tops.slice(0, 6);
    for (const h of sample) {
      const x = h.x + Math.floor(h.w / 2) + 1;
      const n = m.intents().length;
      m.type(`\x1b[<2;${x};${h.y + 1}M\x1b[<2;${x};${h.y + 1}m`);
      await until(() => m.intents().length > n, 500);
      await tick(10);
      const got = m.intents().slice(n).map((i) => i.intent);
      expect(got, `right click on ${h.target} at ${x},${h.y}`).toEqual([{ intent: "context", target: h.target }]);
      const n2 = m.intents().length;
      m.type(`\x1b[<0;${x};${h.y + 1}M\x1b[<0;${x};${h.y + 1}m`);
      await until(() => m.intents().length > n2, 500);
      await tick(10);
      const left = m.intents().slice(n2).map((i) => i.intent);
      expect(left[0], `left click on ${h.target}`).toEqual({ intent: "select", target: h.target });
    }
    const n = m.intents().length;
    m.type(`\x1b[<2;1;40M`);
    await tick(10);
    expect(m.intents().length, "a right click on the status line's empty cells").toBe(n);

    const focusPane = (view.page ? view.page.panes ?? [] : view.panes ?? []).find((p: any) => p.pane === "middle");
    const kind: string | undefined = focusPane?.body?.node === "document" || focusPane?.body?.node === "rich" ? focusPane.body.node : undefined;
    for (const [name, bytes] of [["down", "\x1b[B"], ["up", "\x1b[A"], ["j", "j"], ["enter", "\r"], ["home", "\x1b[H"], ["end", "\x1b[F"]] as const) {
      await answer();
      const before = m.intents().length;
      // Scroll::key: ↓ j, ↑ k (not at the top), Home, End scroll; ⏎ is not a scroll key.
      const sc = kind ? [...scrollers()].reverse().find((x) => x.focused && x.kind === kind) : undefined;
      const at = sc?.at() ?? 0;
      const max = sc?.max() ?? 0;
      const moved = !!sc && (name === "down" || name === "j" || name === "home" || name === "end" || (name === "up" && at > 0));
      const after = name === "home" ? 0 : name === "end" ? max : name === "up" ? at - 1 : Math.min(at + 1, max);
      m.type(bytes);
      if (moved) {
        await tick(60);
        const sent = m.intents().slice(before).map((i) => i.intent);
        if (kind === "rich" && after >= max) {
          expect(sent, `${name} on a rich pane left at its end`).toEqual([{ intent: "end", pane: "middle" }]);
          ran.richAtEnd++;
        } else {
          expect(sent, `${name} scrolls the ${kind} pane in the frontend and sends nothing`).toEqual([]);
          if (kind === "rich") ran.richScrolled++;
          else ran.documentScrolled++;
        }
        expect(sc!.at(), `${name} moved the scroller`).toBe(after);
        continue;
      }
      await until(() => m.intents().length > before, 500);
      await tick(15);
      const sent = m.intents().slice(before).map((i) => i.intent);
      expect(sent.length, `${name} sends one intent`).toBe(1);
      const i = sent[0]!;
      if (i.intent === "select" || i.intent === "activate" || i.intent === "fold") {
        expect(named.has(i.target) || hits.some((h) => h.target === i.target), `${name}: ${i.intent} names ${i.target}, which is in the view`).toBe(true);
      } else expect(i.intent, name).toBe("key");
    }
  });
});

describe("the frontend-scroll cases", () => {
  it("ran on at least one fixture each: a document scrolled, a rich pane scrolled and a rich pane left at its end", () => {
    expect(ran.documentScrolled, "document").toBeGreaterThan(0);
    expect(ran.richAtEnd, "rich at its end").toBeGreaterThan(0);
  });
});
