import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { mount, tick } from "../app/harness.js";
import { applyLook, currentLook, defaultLook } from "../../src/text/look.js";

/**
 * The diff and the editor over view data the real host made on demo data (tests/diff_editor_views.rs writes
 * ink/test/fixtures/diff/<case>_<w>x<h>.json, the snapshot the bridge sends, and <case>_<w>x<h>.txt, the text the reference
 * draws for the same host). Each case is drawn by the whole App at its size and its text compared with the reference's, row
 * for row (a row's trailing blanks are not cells that differ).
 */
const dir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../fixtures/diff");
const cases = fs.existsSync(dir)
  ? fs.readdirSync(dir).filter((f) => f.endsWith(".json")).map((f) => f.replace(/\.json$/, "")).sort()
  : [];

// shell::to_text draws with the icon pack off; so does Ink here, or every icon would differ by itself.
const before = currentLook();
beforeAll(() => applyLook(defaultLook("off")));
afterAll(() => applyLook(before));

let live: Array<() => void> = [];
afterEach(() => {
  live.forEach((f) => f());
  live = [];
});

/** The rows from the panes' top edge down (the top bar, tabs and the MR header above them carry the chrome's icon pack,
 * which the host's text draws without; they are the chrome tests'). */
const rows = (text: string) => {
  const all = text.split("\n").map((l) => l.trimEnd());
  const top = all.findIndex((l) => l.startsWith("╭"));
  return top < 0 ? all : all.slice(top);
};

describe("diff and editor view data from the host", () => {
  it("there are cases, at both sizes, for the unified diff, the cursor, a comment, the compose editor", () => {
    for (const c of ["diff_unified", "diff_cursor_down", "diff_inline_comment", "editor_compose"])
      for (const s of ["80x24", "160x40"]) expect(cases, `${c}_${s}`).toContain(`${c}_${s}`);
    expect(cases).toContain("diff_split_160x40");
  });

  for (const name of cases) {
    it(`${name}: Ink draws the text the reference draws`, async () => {
      const view = JSON.parse(fs.readFileSync(path.join(dir, `${name}.json`), "utf8"));
      const want = fs.readFileSync(path.join(dir, `${name}.txt`), "utf8");
      const m = mount(view.width, view.height);
      live.push(m.stop);
      m.welcome();
      m.snapshot({ chrome: view.chrome, screen: view.screen, section: view.section, focus: view.focus }, 1);
      await tick(60);
      const got = rows(m.frame());
      const exp = rows(want);
      // Compare row by row so a failure names the row.
      for (let y = 0; y < Math.max(got.length, exp.length); y++)
        if ((got[y] ?? "") !== (exp[y] ?? "")) expect({ row: y, text: got[y] ?? "" }).toEqual({ row: y, text: exp[y] ?? "" });
    });
  }
});
