import React from "react";
import { describe, expect, it } from "vitest";
import { PageBody } from "../../src/app/Panes.js";
import { renderNode } from "../../src/nodes/index.js";
import { frameAt } from "./helpers.js";

// Defect 6 through what the user sees: the page head of an MR page keeps "→ main" at 70, 80 and 160 columns.
const t = (text: string, ink?: string) => ({ text, ...(ink ? { ink } : {}) });
const page = {
  crumbs: [t("Repositories"), t("billing-service"), t("!42")],
  title: [
    t(
      "⇄ feat: ABC-1594 Rename FAILED status to SETTLED across the whole billing pipeline",
      "emphasis",
    ),
  ],
  details: [
    t("Alice Example", "secondary"),
    t(" · ", "muted"),
    t("alice/ABC-1594-rename-… → main", "accent"),
  ],
  tabs: [{ label: "Overview" }],
  tab: 0,
  panes: [],
};

describe("the item page head", () => {
  for (const w of [70, 80, 160]) {
    it(`keeps '→ main' on the title line at ${w} columns and does not run the title into the details`, () => {
      const out = frameAt(
        <PageBody
          page={page}
          width={w}
          height={10}
          cx={{ width: w, focused: true } as never}
          renderNode={renderNode as never}
        />,
        w,
        10,
      );
      const title = out.split("\n").find((l) => l.includes("feat:"))!;
      expect(title).toMatch(/→ main$/);
      if (w < 160) expect(title).toMatch(/…\s{2,}\S/); // at 160 the whole title fits
    });
  }
});
