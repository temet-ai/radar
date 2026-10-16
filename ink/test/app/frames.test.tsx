import { afterEach, describe, expect, it } from "vitest";
import { mount, tick, until } from "./harness.js";
import { RENDER_OPTIONS } from "../../src/app/render-options.js";

let live: Array<() => void> = [];
afterEach(() => {
  live.forEach((f) => f());
  live = [];
});

const rows = Array.from({ length: 200 }, (_, i) => ({
  target: `row:${i}`,
  text: [{ ink: "accent_secondary", text: `!${1000 + i} ` }, { text: `Fix the flaky thing number ${i} in the pipeline runner` }],
}));
const screenAt = (selected: number) => ({
  id: "mrs",
  title: "mrs",
  panes: [{ pane: "middle", title: "merge requests", body: { node: "list", selected, rows } }],
  actions: [],
});

/** How the renderer is run (app/run.tsx): throttled, writing only what changed. */
const RUN_OPTIONS = { ...RENDER_OPTIONS, alternateScreen: false, debug: false };

describe("what one key writes to the terminal", () => {
  for (const [cols, height] of [[160, 40], [80, 24]] as const) {
    it(`Down on a 200-row list at ${cols}x${height} writes only the changed lines (under 3 KB), once`, async () => {
      const m = mount(cols, height, {}, RUN_OPTIONS);
      live.push(m.stop);
      m.welcome();
      m.snapshot({ screen: screenAt(0) }, 1);
      // The first paint is counted out by what it wrote, not by a wait: on a busy machine it can take longer than any
      // fixed wait, and what it wrote after one would be counted as the key's.
      await until(() => m.out.frames.some((f) => f.includes("number 0 in the pipeline")));
      const base = m.out.frames.join("").length;
      const n = m.out.frames.length;
      m.type("\x1b[B");
      await until(() => m.intents().length >= 1);
      // The host's answer: the selection moved.
      m.snapshot({ screen: screenAt(1) }, 2);
      await until(() => m.out.frames.length > n);
      // Then room for a second write that should not come: a slow machine only makes it later, never a false failure.
      await tick(200);
      const written = m.out.frames.join("").length - base;
      expect(written, `${written} bytes in ${m.out.frames.length - n} writes`).toBeLessThan(3000);
      expect(m.out.frames.length - n).toBeLessThanOrEqual(2);
    });
  }
});
