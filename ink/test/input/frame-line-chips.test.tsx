import React from "react";
import { describe, expect, it } from "vitest";
import { PageBody } from "../../src/app/Panes.js";
import { renderNode } from "../../src/nodes/index.js";
import { InputController, type InputView } from "../../src/input/controller.js";
import { UiState } from "../../src/state/ui-state.js";
import type { Intent } from "../../src/chrome/types.js";
import { frameAt } from "../nodes/helpers.js";

// A chip (Seg.target) on one of a page's frame lines (crumbs, the details beside the title, the line under, the notice)
// is pressed like a chip in a body row: Ink sends the chip's own target, and the words around it stay as they were
// (host.rs pane_of_target maps the frame to the middle pane).
const word = (text: string) => ({ text });
const chip = (text: string, target: string) => ({ text, chip: true, target });
const page = {
  crumbs: [word("Repositories"), chip("watch-core", "crumb:1"), word("!42")],
  title: [{ text: "Fix the night watch rota", strong: true }],
  details: [word("Sam Vimes"), word(" · "), chip("draft", "details:0")],
  under: [word("branch"), word(" "), chip("copy branch name", "under:3:0"), word(" and more words")],
  notice: [word("Stale for 3 days"), word("  "), chip("rebase", "notice:0")],
  tabs: [],
  panes: [],
};
const LINES: Array<{ name: string; target: string; chipText: string; context: string }> = [
  { name: "crumbs", target: "crumb:1", chipText: " watch-core ", context: "Repositories" },
  { name: "details", target: "details:0", chipText: " draft ", context: "Sam Vimes" },
  { name: "under", target: "under:3:0", chipText: " copy branch name ", context: "branch" },
  { name: "notice", target: "notice:0", chipText: " rebase ", context: "Stale for 3 days" },
];

type Hit = { x: number; y: number; w: number; h: number; target: string };
// frameAt unmounts the tree, which would remove what a line registered (HitRegistry.remove), so this keeps what was last
// registered under each owner.
class Registry {
  private owners = new Map<string, Hit[]>();
  add(hit: Hit) { this.owners.set(`add${this.owners.size}`, [hit]); }
  replace(owner: string, hits: Hit[]) { this.owners.set(owner, hits); }
  all(): Hit[] { return [...this.owners.values()].flat(); }
}

function drawn(width: number, plainText = false) {
  const registry = new Registry();
  const cx = { width, focused: true, registry, plainText } as never;
  const frame = frameAt(<PageBody page={page} width={width} height={10} cx={cx} renderNode={renderNode as never} />, width, 10);
  return { frame: frame.split("\n"), registry };
}

function press(registry: Registry, x: number, y: number, button = 0) {
  const sent: Intent[] = [];
  const ui = new UiState();
  const view: InputView = {
    menu: () => null,
    capturing: () => false,
    route: (chord) => ({ intent: "key", chord }),
    hits: () => registry.all(),
    selectAt: () => undefined,
    paneAt: () => "middle",
    frameRow: () => [],
  };
  const controller = new InputController(view, (i) => sent.push(i), ui, () => 1000);
  controller.mouse(`\x1b[<${button};${x + 1};${y + 1}M\x1b[<${button};${x + 1};${y + 1}m`);
  return sent;
}

for (const width of [80, 160]) {
  describe(`a chip on a page's frame line at ${width} columns`, () => {
    for (const line of LINES) {
      it(`${line.name}: registers the cells the chip is drawn in, and only those`, () => {
        const { frame, registry } = drawn(width);
        const hits = registry.all().filter((h) => h.target === line.target);
        expect(hits).toHaveLength(1);
        const hit = hits[0]!;
        expect(hit.h).toBe(1);
        // frameAt trims a line's trailing spaces, so a chip at the line's end is compared without its last one
        expect(frame[hit.y]!.slice(hit.x, hit.x + hit.w)).toBe(line.chipText.slice(0, frame[hit.y]!.length - hit.x));
        // the words around it register nothing
        const row = frame[hit.y]!;
        expect(registry.all().filter((h) => h.y === hit.y && h.target !== line.target)).toEqual([]);
        expect(row.indexOf(line.context)).toBeGreaterThanOrEqual(0);
      });

      it(`${line.name}: a press sends the chip's target alone, a right press its context`, () => {
        const { registry } = drawn(width);
        const hit = registry.all().find((h) => h.target === line.target)!;
        expect(press(registry, hit.x + 1, hit.y)).toEqual([{ intent: "select", target: line.target }]);
        expect(press(registry, hit.x + hit.w - 1, hit.y, 2)).toEqual([{ intent: "context", target: line.target }]);
      });

      it(`${line.name}: a press on the words beside the chip sends no chip target`, () => {
        const { frame, registry } = drawn(width);
        const hit = registry.all().find((h) => h.target === line.target)!;
        const at = frame[hit.y]!.indexOf(line.context);
        expect(registry.all().some((h) => h.y === hit.y && at >= h.x && at < h.x + h.w)).toBe(false);
        expect(press(registry, at, hit.y).some((i) => (i as { target?: string }).target === line.target)).toBe(false);
      });
    }
  });
}

describe("a chip on a page's frame line in plain text", () => {
  it("registers the bracketed chip's cells", () => {
    const { frame, registry } = drawn(80, true);
    for (const line of LINES) {
      const hit = registry.all().find((h) => h.target === line.target)!;
      expect(frame[hit.y]!.slice(hit.x, hit.x + hit.w)).toBe(`[${line.chipText.trim()}]`);
    }
  });
});
