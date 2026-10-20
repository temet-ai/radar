import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { freezeClock, mount, tick } from "../app/harness.js";
import { plain } from "../contract/support.js";
import { applyLook, defaultLook } from "../../src/text/look.js";

/**
 * C1: the whole Ink App (chrome + screen/section) draws each per-commit recording's view data
 * (ink/test/fixtures/reference/<name>.json, written by tests/reference_views.rs) at the recorded size, and the text
 * must equal the recording tests/snapshots/<name>.snap (insta header, up to the second `---` line, taken off;
 * trailing blanks per line are not drawn by either side). One test per recording, so a run lists every mismatch.
 */
const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../../..",
);
const refDir = path.join(root, "ink/test/fixtures/reference");
const snapDir = path.join(root, "tests/snapshots");

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

export const recordingBody = (snap: string): string[] => {
  const lines = snap.split("\n");
  let seen = 0;
  let i = 0;
  for (; i < lines.length && seen < 2; i++) if (lines[i] === "---") seen++;
  const body = lines.slice(i).map((l) => l.replace(/\s+$/, ""));
  while (body.length && body[body.length - 1] === "") body.pop();
  return body;
};

/** tests/mix.rs masks an alert's age (`firing 3h 07m`) as `#h ##m`, digits only, so the width stays. */
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

const names = fs.existsSync(refDir)
  ? fs
      .readdirSync(refDir)
      .filter((f) => f.endsWith(".json"))
      .map((f) => f.slice(0, -5))
      .sort()
  : [];

describe("Ink draws the recordings", () => {
  // T1 writes one view-data file per recording (tests/reference_views.rs); none may be missing.
  it("every recording has reference view data", () => {
    const recs = fs
      .readdirSync(snapDir)
      .filter((f) => f.startsWith("mix__") && f.endsWith(".snap"))
      .map((f) => f.slice(0, -5));
    expect(recs.filter((r) => !names.includes(r))).toEqual([]);
  });

  it("there is reference view data to draw", () => {
    expect(names.length).toBeGreaterThan(0);
  });

  const run = (name: string) => async () => {
    const view = JSON.parse(
      fs.readFileSync(path.join(refDir, `${name}.json`), "utf8"),
    );
    const snapFile = path.join(snapDir, `${name}.snap`);
    expect(fs.existsSync(snapFile), `recording ${name}.snap`).toBe(true);
    const want = recordingBody(fs.readFileSync(snapFile, "utf8")).map(aged);
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
    expect(got.join("\n")).toBe(want.join("\n"));
  };
  for (const name of names) it(name, run(name));
});
