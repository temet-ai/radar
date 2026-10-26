import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { freezeClock, mount, tick } from "../app/harness.js";
import { plain } from "../contract/support.js";
import { applyLook, defaultLook } from "../../src/text/look.js";

/**
 * C1: the whole Ink App (chrome + screen/section) draws each mix recording's view data
 * (ink/test/fixtures/reference/<name>.json, written by tests/reference_views.rs) at the recorded size, and the text
 * must equal Ink's golden ink/test/reference/goldens/<name>.txt (trailing blanks per line are not drawn).
 * One test per recording, so a run lists every mismatch.
 *
 * Ink owns its picture: the goldens are its own recordings
 * (tests/snapshots/mix__<name>.snap bodies) as they stood when tests/mix.rs moved to the view-data text form,
 * frozen here; tests/mix.rs no longer records a painted frame. A golden moves only when Ink's own drawing does:
 * read the diff before rewriting one (`RADAR_GOLDEN_UPDATE=1 npx vitest run test/reference/parity.test.tsx`).
 */
const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "../../..");
const refDir = path.join(root, "ink/test/fixtures/reference");
const goldenDir = path.join(here, "goldens");

let unfreeze = () => {};
beforeAll(() => {
  unfreeze = freezeClock();
});
afterAll(() => unfreeze());

let live: Array<() => void> = [];
afterEach(() => {
  applyLook(defaultLook());
  live.forEach((f) => f());
  live = [];
});

/** A golden's lines: trailing blanks per line and blank lines at the end are not drawn. */
const goldenLines = (text: string): string[] => {
  const body = text.split("\n").map((l) => l.replace(/\s+$/, ""));
  while (body.length && body[body.length - 1] === "") body.pop();
  return body;
};

/** An alert's age (`firing 3h 07m`) is masked as `#h ##m`, digits only, so the width stays; the goldens hold it masked. */
const aged = (l: string) =>
  l.replace(
    /firing (\d[\ddhms ]*)/g,
    (_m, age: string) => "firing " + age.replace(/\d/g, "#"),
  ).replace(/(^|[^A-Za-z0-9_])v\d+(\.\d+)*(?![A-Za-z0-9_])/g, "$1v#");

// A shipped version (`recipe ask  v2 · built-in`) is content, not layout: masked on both sides, so raising one fails nothing here.

/** The icon pack a recording was made with: tests/mix.rs names the packed ones `icons_<pack>`; the rest have icons off. */
const packOf = (name: string) =>
  (/_icons_(nerd|unicode|emoji)_/.exec(name)?.[1] ?? "off") as
    "nerd" | "unicode" | "emoji" | "off";

const listed = (dir: string, ext: string) =>
  fs.existsSync(dir)
    ? fs
        .readdirSync(dir)
        .filter((f) => f.endsWith(ext))
        .map((f) => f.slice(0, -ext.length))
        .sort()
    : [];

const names = listed(refDir, ".json");

describe("Ink draws the recordings", () => {
  // Every golden has its view data and every view data file its golden.
  it("every golden has reference view data, and the other way round", () => {
    const goldens = listed(goldenDir, ".txt");
    expect(goldens.length).toBeGreaterThan(0);
    expect({
      goldensWithoutViews: goldens.filter((g) => !names.includes(g)),
      viewsWithoutGoldens: names.filter((n) => !goldens.includes(n)),
    }).toEqual({ goldensWithoutViews: [], viewsWithoutGoldens: [] });
  });

  it("there is reference view data to draw", () => {
    expect(names.length).toBeGreaterThan(0);
  });

  const run = (name: string) => async () => {
    const view = JSON.parse(
      fs.readFileSync(path.join(refDir, `${name}.json`), "utf8"),
    );
    const goldenFile = path.join(goldenDir, `${name}.txt`);
    expect(fs.existsSync(goldenFile), `golden ${name}.txt`).toBe(true);
    const want = goldenLines(fs.readFileSync(goldenFile, "utf8")).map(aged);
    applyLook(defaultLook(packOf(name)));
    const m = mount(view.width, view.height);
    live.push(m.stop);
    m.welcome();
    m.snapshot(
      {
        chrome: view.chrome,
        screen: view.screen ?? null,
        section: view.section ?? null,
        focus: view.focus ?? "middle",
      },
      1,
    );
    await tick(60);
    const got = plain(m.frame())
      .split("\n")
      .map((l) => aged(l.replace(/\s+$/, "")));
    while (got.length && got[got.length - 1] === "") got.pop();
    // RADAR_GOLDEN_UPDATE=1 rewrites the golden from what Ink draws now; read the diff before keeping it.
    if (process.env.RADAR_GOLDEN_UPDATE === "1") fs.writeFileSync(goldenFile, got.join("\n") + "\n");
    else expect(got.join("\n")).toBe(want.join("\n"));
  };
  for (const name of names) it(name, run(name));
});
