import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import React from "react";
import { describe, expect, it } from "vitest";
import { PageTitleLine } from "../../src/nodes/pagetitle.js";
import { textWords } from "../../src/text/TextLine.js";
import { frameAt } from "./helpers.js";

// Defect 6 of the live survey: the MR page's title line lost its "→ main" target and the title ran into the author.
// ui::page::title_line: the details are right-aligned and kept to half the room, dropping their first runs (and the
// "·" after them) before their last; the title is cut with … to what is left, with two cells between.
const t = (text: string, ink?: string) => ({ text, ...(ink ? { ink } : {}) });
const draw = (title: any[], details: any[], width: number) =>
  frameAt(
    <PageTitleLine title={title} details={details} width={width} />,
    width,
  );

describe("the page title line", () => {
  const title = [
    t(
      "⇄ feat: ABC-1594 Rename FAILED status to SETTLED across the whole billing pipeline",
    ),
  ];
  const details = [
    t("Alice Example", "secondary"),
    t(" · ", "muted"),
    t("alice/ABC-1594-rename-… → main", "accent"),
  ];

  it("keeps the target (the last run of the details) whatever the width", () => {
    for (const w of [70, 80, 100, 160])
      expect(draw(title, details, w), `at ${w}`).toMatch(/→ main$/);
  });

  it("never lets the title touch the details: the title is cut with … and two cells stay between", () => {
    const line = draw(title, details, 80);
    expect(line).toMatch(/…\s{2,}\S/);
    expect(line.length).toBeLessThanOrEqual(80);
  });

  it("drops the leading run and its dot first, not the end", () => {
    const line = draw([t("short")], details, 60);
    expect(line).not.toMatch(/Alice/);
    expect(line).not.toMatch(/^\s*short\s+·/);
  });

  it("shows title and every detail whole when both fit", () => {
    const line = draw(
      [t("change 198")],
      [t("alice"), t(" · "), t("feature/x → main")],
      80,
    );
    expect(line).toMatch(/^ change 198\s+alice · feature\/x → main$/);
  });
});

// The class: the details' last run is on the title line of every recorded item page, at both sizes.
const dir = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../fixtures",
);
describe("every recorded item page", () => {
  const pages = fs
    .readdirSync(dir)
    .filter((f) => f.endsWith(".json") && !f.startsWith("_"))
    .map((f) => ({
      f,
      page: JSON.parse(fs.readFileSync(path.join(dir, f), "utf8")).page,
    }))
    .filter((x) => x.page && (x.page.details ?? []).length > 0);
  it("has pages", () => expect(pages.length).toBeGreaterThan(5));
  it("draws the last run of its details on the title line", () => {
    const bad: string[] = [];
    for (const { f, page } of pages) {
      const m = /_(\d+)x\d+\.json$/.exec(f)!;
      const last = textWords([page.details[page.details.length - 1]]).trim();
      const line = draw(page.title, page.details, Number(m[1]));
      if (!line.includes(last.length > 38 ? last.slice(0, 20) : last))
        bad.push(`${f}: "${last}" missing in "${line}"`);
    }
    expect(bad).toEqual([]);
  });
});
