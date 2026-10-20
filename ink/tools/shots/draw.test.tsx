// scripts/shots draws every recorded view fixture through the whole App into styled cells and writes each as an SVG.
// Not part of `npm test` (its config does not include this folder): scripts/shots runs it with
//   SHOTS_OUT=DIR [SHOTS_SIZES=80x24,160x40] npx vitest run --config tools/shots/vitest.config.ts
// DIR gets <place>_<size>.svg and places.tsv ("place<TAB>title" a line). Fixtures are ink/test/fixtures/*.json
// of the tree it runs in; a fixture is composed with the recorded host chrome of its size, as test/app/fixtures.test.tsx does.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { expect, it } from "vitest";
import { freezeClock, mount, tick } from "../../test/app/harness.js";
import { booksChrome, settingsNav } from "../../test/chrome/fixture.js";
import { parseAnsi } from "../../test/nodes/ansi.js";
import { currentLook } from "../../src/text/look.js";
import { cellsToSvg, parseHex, type Rgb } from "./svg.js";

const outDir = process.env.SHOTS_OUT;
const sizes = (process.env.SHOTS_SIZES ?? "80x24,160x40").split(",");
const dir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../test/fixtures");
const load = (f: string) => JSON.parse(fs.readFileSync(path.join(dir, f), "utf8"));
const kind = (v: any): "chrome" | "section" | "screen" => (v.top_bar ? "chrome" : v.detail_title !== undefined ? "section" : "screen");

it.skipIf(!outDir)("draws the fixtures as SVG", async () => {
  const unfreeze = freezeClock();
  const slots = currentLook().slots;
  const dfg = parseHex(slots["text.primary"] ?? "#c0c0c0", [0xc0, 0xc0, 0xc0]) as Rgb;
  const dbg = parseHex(slots["bg.base"] ?? "#000000", [0, 0, 0]) as Rgb;
  fs.mkdirSync(outDir!, { recursive: true });
  const files = fs.readdirSync(dir).filter((f) => f.endsWith(".json") && !f.startsWith("_")).sort();
  const places = new Set<string>();
  for (const f of files) {
    const m = /^(.*)_(\d+)x(\d+)\.json$/.exec(f);
    if (!m || !sizes.includes(`${m[2]}x${m[3]}`)) continue;
    const [, name, c, r] = m;
    const cols = Number(c);
    const rows = Number(r);
    const view = load(f);
    const k = kind(view);
    const chromeFile = files.find((x) => x.startsWith("host_chrome") && x.endsWith(`_${c}x${r}.json`));
    const chrome = k === "chrome" ? view : chromeFile ? load(chromeFile) : booksChrome({ width: cols, height: rows });
    const { pane_jumps: _p, ...plain } = chrome;
    const shown =
      k === "chrome"
        ? chrome
        : { ...plain, status: { ...plain.status, scope: String(view.scope ?? "").toUpperCase() }, ...(k === "section" ? { settings: { ...settingsNav, section_title: view.scope, detail_title: view.detail_title } } : {}) };
    const app = mount(cols, rows);
    app.welcome();
    app.snapshot({ chrome: shown, screen: k === "screen" ? view : null, section: k === "section" ? view : null }, 1);
    await tick(40);
    const cells = parseAnsi(app.frame());
    app.stop();
    fs.writeFileSync(path.join(outDir!, `${name}_${c}x${r}.svg`), cellsToSvg(cells, cols, rows, dfg, dbg));
    places.add(name!);
  }
  fs.writeFileSync(path.join(outDir!, "places.tsv"), [...places].map((n) => `${n}\t${n.replace(/_/g, " ")}`).join("\n") + "\n");
  unfreeze();
  expect(places.size).toBeGreaterThan(0);
});
