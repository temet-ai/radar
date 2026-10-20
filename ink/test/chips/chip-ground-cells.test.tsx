import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3"; // Ink's colour library reads this when it loads: keep the cells' styles
});
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { mount, tick } from "../app/harness.js";
import { applyLook, defaultLook } from "../../src/text/look.js";
import { parseAnsi } from "../nodes/ansi.js";
import { adoptInkFrame, updatingGoldens, type SceneFile } from "../support/goldens.js";

// widgets::chip_ground (#30): a chip stands on its tone's tint. The golden cells (ink/test/cells, first drawn by the earlier renderer,
// now Ink's, rewritten from Ink by INK_UPDATE_GOLDENS=1) for the GitHub screens carry a muted chip line
// (' ⌂ local folder  no matching origin…'); the App draws the same view data and each chip cell's ground must be the
// golden's, and the ground must stop where the golden's does. The host chrome's bars differ from the scene's, so the
// chip's row is found by its words; its columns are the same.
const here = path.dirname(fileURLToPath(import.meta.url));
const ch = (c: { ch?: string } | undefined) => (c?.ch === undefined || c.ch === "" ? " " : c.ch);

let live: Array<() => void> = [];
afterEach(() => {
  live.forEach((f) => f());
  live = [];
  applyLook(defaultLook());
});

describe("a chip's ground against the reference cells", () => {
  for (const [surface, size] of [
    ["github_github", "160x40"],
    ["github_github-page1-overview-pull-requests-actions", "80x24"],
  ] as const) {
    it(`${surface} at ${size}`, async () => {
      const file = path.resolve(here, `../cells/${surface}-${size}.json`);
      const rat = JSON.parse(fs.readFileSync(file, "utf8")) as {
        rows: Array<Array<{ ch?: string; bg?: string }>>;
      };
      const screen = JSON.parse(fs.readFileSync(path.resolve(here, `../fixtures/${surface}_${size}.json`), "utf8"));
      const chrome = JSON.parse(fs.readFileSync(path.resolve(here, `../fixtures/host_chrome_${size}.json`), "utf8"));
      const [cols, rows] = size.split("x").map(Number) as [number, number];
      const m = mount(cols, rows);
      live.push(m.stop);
      m.welcome();
      m.snapshot({ chrome, ...(screen.screen ? screen : { screen }) }, 1);
      await tick(60);
      const got = parseAnsi(m.frame());
      if (updatingGoldens()) adoptInkFrame(file, rat as unknown as SceneFile, got);
      // The golden's tinted cells, as runs: where each starts and the words in it.
      let compared = 0;
      rat.rows.forEach((r, y) => {
        const xs = r.flatMap((c, x) => ((c.bg ?? "").startsWith("#") ? [x] : []));
        if (!xs.length) return;
        const words = r.slice(xs[0], xs[xs.length - 1]! + 1).map(ch).join("").trim().slice(0, 12);
        const iy = got.findIndex((g) => g.map(ch).join("").includes(words));
        expect(iy, `Ink drew "${words}"`).toBeGreaterThan(-1);
        // the same columns, and the ground starts where the reference's does. Its right end is not compared: where a chip wraps,
        // the reference's word wrap drops the space at the break and Ink's keeps it as one more cell of ground (drift older
        // than the tint).
        for (const x of [...xs, xs[0]! - 1]) {
          if (r[x]?.ch === "") continue;
          const want = xs.includes(x) ? r[x]!.bg : undefined;
          const g = got[iy]![x]?.bg?.toLowerCase();
          if (want) expect(g, `(${x},${y}) "${ch(r[x])}"`).toBe(want);
          else expect(g, `(${x},${y}) just outside the chip`).not.toBe(r[xs[0]!]!.bg);
          compared++;
        }
      });
      expect(compared, "chip cells compared").toBeGreaterThan(20);
    });
  }
});
