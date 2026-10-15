import { afterEach, describe, expect, it } from "vitest";
import { mount, tick } from "./harness.js";
import { shellRenderNode, type NodeCx } from "../../src/app/Body.js";
import { RENDER_OPTIONS } from "../../src/app/render-options.js";

let live: Array<() => void> = [];
afterEach(() => {
  live.forEach((f) => f());
  live = [];
});

const rows = Array.from({ length: 30 }, (_, i) => ({ target: `row:${i}`, text: [{ text: `row ${i}` }] }));
const screenAt = (selected: number) => ({
  id: "mrs",
  title: "mrs",
  panes: [{ pane: "middle", title: "merge requests", body: { node: "list", selected, rows } }],
  actions: [],
});

describe("what one snapshot costs the renderer (T21)", () => {
  it("a snapshot is drawn by one pass over the body, not two (the effect after it asked for a second)", async () => {
    let passes = 0;
    const renderNode = (node: any, cx: NodeCx) => {
      if (node?.node === "list") passes++;
      return shellRenderNode(node, cx);
    };
    const m = mount(80, 24, { renderNode });
    live.push(m.stop);
    m.welcome();
    m.snapshot({ screen: screenAt(0) }, 1);
    await tick(80);
    const before = passes;
    m.snapshot({ screen: screenAt(1) }, 2);
    await tick(80);
    expect(passes - before, "list renders for one snapshot").toBe(1);
  });

  it("Ink is let draw 120 frames a second: a key's local change and the host's snapshot are not 32 ms apart", () => {
    expect(RENDER_OPTIONS.maxFps).toBeGreaterThanOrEqual(120);
  });
});
