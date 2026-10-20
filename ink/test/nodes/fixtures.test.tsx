import React from "react";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { Box, Text } from "ink";
import { describe, expect, it } from "vitest";
import { renderNode, type Node } from "../../src/nodes/index.js";
import { frameAt } from "./helpers.js";

// Every recorded view fixture (tests/view_fixtures.rs writes `<part>_<surface>_<w>x<h>.json`) is rendered at its own
// size, pane by pane through renderNode, and compared with a committed snapshot. A fixture is a ScreenView, a
// SectionView or a ChromeView; chrome is T10's, so only the node-bearing parts are drawn here: every pane body of a
// screen or page, a section's body and its detail, and a screen's overlay. Files starting with `_` are hand-made
// samples of the same shape and are not part of the Headless surface list.
const dir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../fixtures");
const files = fs.existsSync(dir) ? fs.readdirSync(dir).filter((f) => f.endsWith(".json")).sort() : [];

interface Pane {
  title: string;
  body: Node;
  head?: unknown;
  notes?: unknown[];
  /** The settings sheet's detail (NodeContext.sheet): a link's address is not written after its words there. */
  sheet?: boolean;
}

/** Every pane (title + body) a view holds, in reading order. */
export function panesOf(view: any): Pane[] {
  const out: Pane[] = [];
  const add = (p: any) => p?.body && out.push({ title: String(p.title ?? ""), body: p.body });
  if (!view || typeof view !== "object") return out;
  if (view.page) (view.page.panes ?? []).forEach(add);
  (view.panes ?? []).forEach(add);
  if (view.overlay) add(view.overlay);
  if (view.node && typeof view.node === "string") out.push({ title: "", body: view }); // a bare node
  else if (view.body && view.detail_title !== undefined) {
    out.push({ title: "settings", body: view.body });
    const detail = (view.detail ?? []) as unknown[];
    out.push({ title: String(view.detail_title), body: { node: "document", lines: detail } as Node });
    if ((view.detail_blocks ?? []).length) out.push({ title: "detail", body: { node: "rich", blocks: view.detail_blocks } as Node, sheet: true });
  }
  return out;
}

function Fixture({ view, width, height }: { view: unknown; width: number; height: number }) {
  return (
    <Box flexDirection="column">
      {panesOf(view).map((p, i) => (
        <Box key={i} flexDirection="column" marginBottom={1}>
          <Text bold>{`── ${p.title} ──`}</Text>
          {renderNode(p.body, { width, height, focused: true, sheet: p.sheet })}
        </Box>
      ))}
    </Box>
  );
}

describe("recorded view fixtures", () => {
  it.skipIf(files.length === 0)("has fixtures at both sizes for the same surfaces", () => {
    const sizes = new Map<string, Set<string>>();
    for (const f of files.filter((f) => !f.startsWith("_") && f !== "_surfaces.json")) {
      const m = /^(.*)_(\d+x\d+)\.json$/.exec(f);
      expect(m, `${f} is named <part>_<surface>_<w>x<h>.json`).not.toBeNull();
      sizes.set(m![1], (sizes.get(m![1]) ?? new Set()).add(m![2]));
    }
    for (const [surface, have] of sizes) expect([...have].sort(), surface).toEqual(["160x40", "80x24"]);
  });

  // _surfaces.json ({surfaces:[...]}) is written by tests/view_fixtures.rs: the sorted `<part>_<surface>` names Headless enumerates. A
  // surface in it with no fixture at a size, or a fixture for a surface Headless no longer lists, fails here.
  it("covers exactly the surfaces Headless enumerates, at both sizes", () => {
    const recorded = files.filter((f) => !f.startsWith("_") && f !== "_surfaces.json");
    const manifest = path.join(dir, "_surfaces.json");
    if (recorded.length === 0 && !fs.existsSync(manifest)) return; // nothing generated yet (see the report)
    expect(fs.existsSync(manifest), "_surfaces.json is missing: regenerate with tests/view_fixtures.rs").toBe(true);
    const listed: string[] = JSON.parse(fs.readFileSync(manifest, "utf8")).surfaces;
    const have = new Set(recorded.map((f) => f.replace(/_\d+x\d+\.json$/, "")));
    expect([...have].sort()).toEqual([...listed].sort());
    for (const name of listed) {
      for (const size of ["80x24", "160x40"]) expect(recorded, `${name} at ${size}`).toContain(`${name}_${size}.json`);
    }
  });

  for (const f of files.filter((f) => f !== "_surfaces.json")) {
    const m = /^(.*)_(\d+)x(\d+)\.json$/.exec(f);
    if (!m) continue;
    const [, name, w, h] = m;
    if (name === "host_chrome") continue; // a ChromeView holds no pane nodes: T10's App-level snapshots draw it
    it(`${name} at ${w}x${h}`, () => {
      const view = JSON.parse(fs.readFileSync(path.join(dir, f), "utf8"));
      const out = frameAt(<Fixture view={view} width={Number(w)} height={Number(h)} />, Number(w), Number(h));
      expect(out).not.toMatch(/does not draw/); // no node the renderer lacks
      expect(out).toMatchSnapshot();
    });
  }
});
