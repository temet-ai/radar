// Small helpers the story files share.
import React from "react";
import type { NodeContext } from "../../src/nodes/view";
import { Compare, sizeOf, type Region } from "./Compare";
import { InPane } from "./frames";
import type { Pack, Tier } from "./look";

export type Globals = { tier?: Tier; pack?: Pack };

/** The pane titled `title` in a recorded view (a screen's panes or an item page's panes). */
export function paneOf(view: any, title: string): any {
  const panes: any[] = [...(view?.panes ?? []), ...(view?.page?.panes ?? [])];
  const p = panes.find((x) => x.title === title);
  if (!p) throw new Error(`no pane titled ${title} in this view (${panes.map((x) => x.title).join(", ")})`);
  return p;
}

/** The first node of `kind` anywhere in a recorded view. */
export function firstNode(view: unknown, kind: string): any {
  let found: any;
  const walk = (x: any) => {
    if (found || !x || typeof x !== "object") return;
    if (x.node === kind) {
      found = x;
      return;
    }
    for (const v of Array.isArray(x) ? x : Object.values(x)) walk(v);
  };
  walk(view);
  if (!found) throw new Error(`no ${kind} node in this view`);
  return found;
}

/** A node drawn in its own pane, at the size `region` gives it on the recorded screen (else `size`). */
export function PaneFigure({
  globals,
  pane,
  focused,
  region,
  size,
  body,
}: {
  globals: Globals;
  pane: any;
  focused: boolean;
  region?: Region;
  size?: { w: number; h: number };
  body: (ctx: NodeContext) => React.ReactElement;
}) {
  const { w, h } = sizeOf(region, size ?? { w: 80, h: 21 });
  return (
    <Compare
      globals={globals}
      cols={w}
      rows={h}
      ink={<InPane pane={pane} width={w} height={h} focused={focused} body={body} />}
    />
  );
}
