import React, { useId, useLayoutEffect, useRef } from "react";
import { Box, type DOMElement } from "ink";
import { Scrolled, screenAt } from "./index.js";
import { detailBodyRows, type DetailRows } from "./detailRows.js";
import { CellLine, padCells } from "../text/cells.js";
import type { DetailContent, NodeContext } from "./view.js";

function DetailWindow({ laid, at, height, ctx }: { laid: DetailRows; at: number; height: number; ctx: NodeContext }) {
  const ref = useRef<DOMElement>(null);
  const owner = useId();
  useLayoutEffect(() => {
    const pos = screenAt(ref.current);
    if (!pos || !ctx.registry) return;
    const hits = laid.hits.filter((hit) => hit.dy >= at && hit.dy + hit.h <= at + height && hit.dx >= 0 && hit.dx + hit.w <= ctx.width)
      .map((hit) => ({ x: pos.x + hit.dx, y: pos.y + hit.dy - at, w: hit.w, h: hit.h, target: hit.target }));
    if (ctx.registry.replace) ctx.registry.replace(owner, hits);
    else hits.forEach((hit) => ctx.registry!.add(hit));
  });
  useLayoutEffect(() => () => ctx.registry?.remove?.(owner), [ctx.registry, owner]);
  return <Box ref={ref} flexDirection="column" width={ctx.width} height={height} overflow="hidden">
    {laid.rows.slice(at, at + height).map((row, i) => <CellLine key={i} cells={padCells(row, ctx.width)} plain={ctx.plainText} />)}
  </Box>;
}

export function DetailNode({ data, ctx }: { data: DetailContent; ctx: NodeContext }) {
  const laid = detailBodyRows(data, ctx.width, ctx.plainText);
  const height = Math.max(0, ctx.height ?? laid.rows.length);
  if (!height || !ctx.width) return null;
  if (ctx.height === undefined) return <DetailWindow laid={laid} at={0} height={height} ctx={ctx} />;
  return <Scrolled kind="rich" id={ctx.scrollIdentity ?? JSON.stringify([data.header.path, data.header.title])} total={laid.rows.length} room={height} width={ctx.width} focused={ctx.focused} plain={ctx.plainText} pane={ctx.pane} scrollTo={ctx.scrollTo} fullWidth>
    {(at) => <DetailWindow laid={laid} at={at} height={height} ctx={ctx} />}
  </Scrolled>;
}
