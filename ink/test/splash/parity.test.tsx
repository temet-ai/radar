import { vi } from "vitest";
vi.hoisted(() => {
  process.env.FORCE_COLOR = "3"; // Ink's colour library reads this when it loads: keep the cells' styles
});
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { mount, tick, until } from "../app/harness.js";
import { parseAnsi } from "../nodes/ansi.js";
import type { SplashView } from "../../src/protocol/generated/SplashView.js";

/**
 * The startup splash (crates/radar-core/src/splash.rs), cell for cell. Each file in ink/test/fixtures/splash holds a
 * frame's SplashView as the bridge sends it and the cells the Ratatui runner painted from the same frame (written by
 * the radar-core test `the_cells_ink_compares_against_are_the_committed_fixtures`). The whole App draws the snapshot that
 * carries it; every cell's character, background, and for a drawn character its colour and bold, must match.
 */
const here = path.dirname(fileURLToPath(import.meta.url));
const dir = path.resolve(here, "../fixtures/splash");

type Fixture = { name: string; width: number; height: number; view: SplashView; cells: Array<Array<[string, string, string, boolean]>> };

let live: Array<() => void> = [];
afterEach(() => {
  live.forEach((f) => f());
  live = [];
});

const files = fs.readdirSync(dir).filter((f) => f.endsWith(".json")).sort();

describe("the startup splash", () => {
  it("has frames at 80x24 and 160x40 to compare", () => {
    expect(files.some((f) => f.endsWith("80x24.json"))).toBe(true);
    expect(files.some((f) => f.endsWith("160x40.json"))).toBe(true);
  });

  for (const file of files) {
    it(`Ink draws Ratatui's cells: ${file}`, async () => {
      const fx = JSON.parse(fs.readFileSync(path.join(dir, file), "utf8")) as Fixture;
      const m = mount(fx.width, fx.height);
      live.push(m.stop);
      m.welcome();
      m.snapshot({ splash: fx.view }, 1);
      await tick(40);
      const ink = parseAnsi(m.frame());
      const bad: string[] = [];
      let drawn = 0;
      for (let y = 0; y < fx.height; y++) {
        for (let x = 0; x < fx.width; x++) {
          const [ch, fg, bg, bold] = fx.cells[y]![x]!;
          const c = ink[y]?.[x];
          const got = c?.ch ?? " ";
          const blank = ch.trim() === "";
          const why: string[] = [];
          if (got !== ch && !(blank && got.trim() === "")) why.push(`char ${JSON.stringify(got)} for ${JSON.stringify(ch)}`);
          if ((c?.bg ?? "").toLowerCase() !== bg.toLowerCase()) why.push(`bg ${c?.bg} for ${bg}`);
          if (!blank) {
            drawn++;
            if ((c?.fg ?? "").toLowerCase() !== fg.toLowerCase()) why.push(`fg ${c?.fg} for ${fg}`);
            if (!!c?.bold !== bold) why.push(`bold ${!!c?.bold} for ${bold}`);
          }
          if (why.length) bad.push(`row ${y} col ${x}: ${why.join(", ")}`);
        }
      }
      expect(bad.slice(0, 20)).toEqual([]);
      expect(drawn, "the frame has marks and words to compare").toBeGreaterThan(20);
    });
  }

  it("a key while it shows is not swallowed: it goes to the host as the screen's key", async () => {
    const fx = JSON.parse(fs.readFileSync(path.join(dir, files[0]!), "utf8")) as Fixture;
    const m = mount(fx.width, fx.height);
    live.push(m.stop);
    m.welcome();
    m.snapshot({ splash: fx.view }, 1);
    await tick(40);
    expect(m.frame()).toContain("▀");
    m.type(",");
    await until(() => m.intents().length > 0);
    expect(m.intents().map((i: any) => i.intent)).toEqual([{ intent: "key", chord: { key: { char: "," } } }]);
    // The host's next snapshot has no splash: the screen draws.
    m.snapshot({}, 2);
    await tick(40);
    expect(m.frame()).not.toContain("Sweeping the horizon for anything new.");
  });
});
