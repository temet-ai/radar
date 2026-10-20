import React from "react";
import { EventEmitter } from "node:events";
import { render } from "ink";
import { describe, expect, it } from "vitest";
import { focusedScroller, renderNode, scrollers, thumbOf, type Node } from "../../src/nodes/index.js";

// The frontend scrolls a focused Document or Rich pane (view/scroll.rs Scroll::key): ↓ j ↑ k PgUp PgDn
// g G Home End, ↑ at the top is the host's, and a scrollbar runs down the right edge. The node keeps the offset.

class Out extends EventEmitter {
  frames: string[] = [];
  constructor(public columns: number, public rows: number) {
    super();
  }
  write = (s: string) => {
    this.frames.push(s);
    return true;
  };
  get isTTY() {
    return false;
  }
}
class In extends EventEmitter {
  isTTY = false;
  setRawMode() {}
  setEncoding() {}
  resume() {}
  pause() {}
  ref() {}
  unref() {}
  read() {
    return null;
  }
}

const tick = () => new Promise((r) => setTimeout(r, 30));
// eslint-disable-next-line no-control-regex
const plain = (f: string) => f.replace(/\u001b\[[0-9;?]*[A-Za-z]/g, "").replace(/\s+$/gm, "");

function mount(node: Node | React.ReactElement, width: number, height: number, focused = true) {
  const out = new Out(width, height);
  const app = render(React.isValidElement(node) ? node : renderNode(node as Node, { width, height, focused }), {
    stdout: out as unknown as NodeJS.WriteStream,
    stdin: new In() as unknown as NodeJS.ReadStream,
    debug: true,
    patchConsole: false,
    exitOnCtrlC: false,
  });
  const frame = () => plain(out.frames.filter((f) => f.trim().length > 0).at(-1) ?? "");
  return { frame, stop: () => app.unmount() };
}

const lines = (n: number) => Array.from({ length: n }, (_, i) => [{ text: `line ${i + 1}` }]) as never;
const doc = (n: number): Node => ({ node: "document", lines: lines(n) }) as Node;

describe("a Document pane is cut at its height and never scrolls, as view_tui draws it (widgets::paragraph, Scroll at max 0)", () => {
  it("shows the first rows, no scrollbar; Down j g G Home End are its own, Up k PgUp PgDn go on to the host", async () => {
    const m = mount(doc(30), 40, 10);
    await tick();
    const rows = m.frame().split("\n");
    expect(rows[0]).toMatch(/^ line 1$/);
    expect(rows[9]).toMatch(/^ line 10$/);
    expect(m.frame()).not.toMatch(/[█║]/);
    const s = focusedScroller()!;
    expect([s.at(), s.max(), s.height()]).toEqual([0, 0, 10]);
    expect(["down", "j", "g", "G", "home", "end"].map((k) => s.key(k))).toEqual([true, true, true, true, true, true]);
    expect(["up", "k", "pageUp", "pageDown"].map((k) => s.key(k))).toEqual([false, false, false, false]);
    await tick();
    expect(m.frame().split("\n")[0]).toMatch(/^ line 1$/);
    m.stop();
  });

  it("a document that fits is the same: no bar, nothing to scroll", async () => {
    const m = mount(doc(3), 40, 10);
    await tick();
    expect(m.frame()).not.toMatch(/[█║]/);
    expect(focusedScroller()!.max()).toBe(0);
    m.stop();
  });

  it("only the pane with the keys is the focused scroller", async () => {
    const m = mount(doc(30), 40, 10, false);
    await tick();
    expect(focusedScroller()).toBeUndefined();
    expect(scrollers()).toHaveLength(1);
    m.stop();
    expect(scrollers()).toHaveLength(0);
  });
});

describe("a Rich pane scrolls the same way", () => {
  const rich = (n: number): Node =>
    ({ node: "rich", blocks: Array.from({ length: n }, (_, i) => ({ block: "line", text: [{ text: `row ${i + 1}` }] })) }) as unknown as Node;

  it("clips at the pane height, draws the bar and scrolls", async () => {
    const m = mount(rich(25), 40, 8);
    await tick();
    await tick();
    const s = focusedScroller()!;
    expect(s.kind).toBe("rich");
    expect([s.at(), s.max(), s.height()]).toEqual([0, 17, 8]);
    expect(m.frame().split("\n")[0]).toMatch(/^ row 1\s+█/);
    expect(m.frame()).not.toMatch(/row 9\b/);
    s.key("End");
    s.key("end");
    s.key("G");
    await tick();
    await tick();
    expect(s.at()).toBe(17);
    expect(m.frame().split("\n")[0]).toMatch(/^ row 18\b/);
    expect(m.frame().split("\n")[7]).toMatch(/^ row 25\s+█/);
    m.stop();
  });
});

describe("the thumb follows the host's Scroll", () => {
  it("fills the track when nothing scrolls far, and sits at the end when scrolled to it", () => {
    expect(thumbOf(10, 20, 0)).toEqual({ start: 0, len: 3 });
    const end = thumbOf(10, 20, 20);
    expect(end.start + end.len).toBe(10);
  });
});

describe("inside a pane frame the bar is on the right border, as scroll::scrollbar draws it", () => {
  it("takes the node's report: ║/█ replace the border column between the corners, none in the padding cell", async () => {
    const { Framed } = await import("../../src/app/Panes.js");
    const node = { node: "rich", blocks: Array.from({ length: 30 }, (_, i) => ({ block: "line", text: [{ text: `line ${i + 1}` }] })) } as unknown as Node;
    const m = mount(
      <Framed title="detail" width={40} height={12} focused>
        {renderNode(node, { width: 38, height: 10, focused: true })}
      </Framed> as never,
      40,
      12,
    );
    await tick();
    await tick();
    const rows = m.frame().split("\n");
    expect(rows[0]).toMatch(/^╭ detail .*╮$/); // corners stay
    expect(rows[1]).toMatch(/^│ line 1\s+█$/);
    expect(rows[1]!.length).toBe(40); // the bar is the border column, not a cell inside it
    expect(rows[10]).toMatch(/║$/);
    expect(rows[11]).toMatch(/^╰.*╯$/);
    expect(rows[1]).not.toMatch(/█│$/);
    focusedScroller()!.key("G");
    await tick();
    await tick();
    expect(m.frame().split("\n")[10]).toMatch(/█$/);
    m.stop();
  });
});

describe("a Rich line block wraps at the padded width (view_tui Node::Rich: ui::scroll::scrolled wraps at padded(inner).width)", () => {
  // The node's width W: the recorded layout pads one cell each side and wraps a line block at W - 2. rich_lines' W - 2 - 1 lays out
  // doc blocks only; a line block is pushed whole (live at 160x40: the api detail's 44-cell !263 line on one row).
  const line = (n: number): Node =>
    ({ node: "rich", blocks: [{ block: "line", text: [{ text: `${"word ".repeat(Math.ceil(n / 5)).slice(0, n - 1)}x` }] }] }) as unknown as Node;
  const rows = async (n: number) => {
    const m = mount(line(n), 78, 6);
    await tick();
    await tick();
    const out = m.frame().split("\n").filter((r) => r.trim() !== "").length;
    m.stop();
    return out;
  };
  it("76 cells stay on one row in a 78-cell node, 77 wrap", async () => {
    expect(await rows(76)).toBe(1);
    expect(await rows(77)).toBe(2);
  });
});

describe("PgUp/PgDn on the detail Document of a screen without a page scroll", () => {
  it("a long detail Document does not scroll, so the router leaves the key to the host (which says 'nothing to scroll here')", async () => {
    const { scrollKey } = await import("../../src/input/route.js");
    const key = (name: string) => ({ key: name, ctrl: false, alt: false, shift: false }) as never;
    const m = mount(doc(40), 40, 10, false);
    await tick();
    const screen = {
      panes: [
        { pane: "middle", body: { node: "list", rows: [] } },
        { pane: "detail", body: { node: "document" } },
      ],
    } as never;
    expect(scrollKey(screen, "middle", key("page_down"), scrollers())).toBeUndefined();
    expect(scrollKey(screen, "middle", key("page_up"), scrollers())).toBeUndefined();
    await tick();
    expect(m.frame().split("\n")[0]).toMatch(/^ line 1$/);
    m.stop();
  });
});
