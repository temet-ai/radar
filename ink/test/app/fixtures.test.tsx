import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { freezeClock, mount, tick } from "./harness.js";

let unfreeze = () => {};
beforeAll(() => {
  unfreeze = freezeClock();
});
afterAll(() => unfreeze());
import { booksChrome, settingsNav } from "../chrome/fixture.js";

/**
 * The shell side of Q12: every recorded view fixture (ink/test/fixtures/<part>_<surface>_<w>x<h>.json,
 * written by tests/view_fixtures.rs) is drawn by the whole App (chrome + body) at its own size and
 * compared with a committed snapshot. A fixture is a ScreenView, a SectionView (has `detail_title`) or a
 * ChromeView (`host_chrome_*`, has `top_bar`); a screen or section is shown under that size's recorded
 * host chrome when there is one, else under the hand-made Books chrome. T11's harness
 * (test/nodes/fixtures.test.tsx) draws the pane bodies alone.
 */
const dir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../fixtures");
const files = fs.existsSync(dir) ? fs.readdirSync(dir).filter((f) => f.endsWith(".json")).sort() : [];
const load = (f: string) => JSON.parse(fs.readFileSync(path.join(dir, f), "utf8"));

const sizeOf = (f: string) => {
  const m = /_(\d+)x(\d+)\.json$/.exec(f);
  return m ? { cols: Number(m[1]), rows: Number(m[2]) } : undefined;
};
const kind = (v: any): "chrome" | "section" | "screen" => (v.top_bar ? "chrome" : v.detail_title !== undefined ? "section" : "screen");

let live: Array<() => void> = [];
afterEach(() => {
  live.forEach((f) => f());
  live = [];
});

describe("recorded view fixtures through the whole App", () => {
  it("there are fixtures to draw", () => {
    expect(files.length).toBeGreaterThan(0);
  });

  // tests/view_fixtures.rs writes _surfaces.json: every surface Headless enumerates (the file stems without the
  // size). Each must have a fixture at both sizes, and every fixture must be one Headless enumerates.
  it("the fixtures are exactly the surfaces Headless enumerates, at both sizes", () => {
    const manifest = path.join(dir, "_surfaces.json");
    expect(fs.existsSync(manifest), "ink/test/fixtures/_surfaces.json is written by tests/view_fixtures.rs").toBe(true);
    const listed: string[] = JSON.parse(fs.readFileSync(manifest, "utf8")).surfaces;
    expect(listed.length).toBeGreaterThan(0);
    const have = (name: string, size: string) => files.includes(`${name}_${size}.json`);
    const missing = listed.flatMap((n) => ["80x24", "160x40"].filter((z) => !have(n, z)).map((z) => `${n}_${z}`));
    expect(missing, "surfaces Headless enumerates with no fixture").toEqual([]);
    const stems = files.filter((f) => !f.startsWith("_")).map((f) => f.replace(/_\d+x\d+\.json$/, ""));
    expect([...new Set(stems)].filter((n) => !listed.includes(n)), "fixtures Headless does not enumerate").toEqual([]);
  });

  for (const f of files) {
    const size = sizeOf(f);
    if (!size) continue;
    it(`${f} at ${size.cols}x${size.rows}`, async () => {
      const view = load(f);
      const m = mount(size.cols, size.rows);
      live.push(m.stop);
      m.welcome();
      const k = kind(view);
      const chromeFile = files.find((c) => c.startsWith("host_chrome") && sizeOf(c)?.cols === size.cols && sizeOf(c)?.rows === size.rows);
      const chrome = k === "chrome" ? view : chromeFile ? load(chromeFile) : booksChrome({ width: size.cols, height: size.rows });
      // Each surface under its own status scope; a settings section under the settings navigation.
      const shown =
        k === "chrome"
          ? chrome
          : { ...chrome, status: { ...chrome.status, scope: String(view.scope ?? "").toUpperCase() }, ...(k === "section" ? { settings: { ...settingsNav, section_title: view.scope, detail_title: view.detail_title } } : {}) };
      m.snapshot({ chrome: shown, screen: k === "screen" ? view : null, section: k === "section" ? view : null }, 1);
      await tick(40);
      const frame = m.frame();
      expect(frame.split("\n").length).toBeLessThanOrEqual(size.rows);
      expect(frame).toMatchSnapshot();
    });
  }
});
