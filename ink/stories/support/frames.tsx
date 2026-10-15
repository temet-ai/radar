// The places a component sits in on radar's screen, built from Ink's own shell pieces (app/Panes.tsx, chrome/Chrome.tsx),
// so a story draws its component where and as radar does: a node inside its pane's frame, a pane inside an item page,
// the chrome rows from the host's ChromeView.
import React from "react";
import { Box } from "ink";
import { Framed, PageBody, PaneFrame, ScreenPanes } from "../../src/app/Panes";
import { shellRenderNode, type NodeCx, type RenderNode } from "../../src/app/Body";
import type { NodeContext } from "../../src/nodes/view";
import { MenuBox, SourcesRow, StatusRow, TopBarRow, frameOf } from "../../src/chrome/Chrome";
import type { ChromeView, MenuView } from "../../src/chrome/types";

/** What a node is given by the shell (app/Body.tsx shellRenderNode does the same). */
export const nodeCtx = (cx: NodeCx): NodeContext => ({
  width: cx.width,
  height: cx.height,
  focused: cx.focused ?? true,
  plainText: cx.plainText,
  scrolls: cx.scrolls,
});

/** A renderNode that draws `body` for the node `target` (the story's component with its args) and every other node
 * as the shell does. */
export function drawing(target: unknown, body: (ctx: NodeContext) => React.ReactElement): RenderNode {
  return (node, cx) => (node === target ? body(nodeCtx(cx)) : shellRenderNode(node, cx));
}

const cxOf = (width: number, height: number, focused: boolean): NodeCx => ({ width, height, actions: [], focused });

/** A screen pane (`PaneView`: title, notes, fields, body) framed as view_tui draw_pane frames it. */
export function InPane({ pane, width, height, focused, body }: { pane: any; width: number; height: number; focused: boolean; body: (ctx: NodeContext) => React.ReactElement }) {
  return <PaneFrame pane={pane} width={width} height={height} focused={focused} cx={cxOf(width, height, focused)} renderNode={drawing(pane.body, body)} />;
}

/** A plain titled panel (widgets::panel), as a settings section's rows and detail are drawn (App.tsx SectionBody). */
export function InPanel({ title, width, height, focused, children }: { title: string; width: number; height: number; focused: boolean; children: React.ReactNode }) {
  return (
    <Framed title={title} width={width} height={height} focused={focused}>
      {children}
    </Framed>
  );
}

/** An item page: crumbs, title and details, the line under it, the tab row and its panes (ui::page::PageFrame). */
export function InPage({ page, width, height, focus, tabsFocused, target, body }: { page: any; width: number; height: number; focus?: string; tabsFocused?: boolean; target?: unknown; body?: (ctx: NodeContext) => React.ReactElement }) {
  const renderNode = target && body ? drawing(target, body) : shellRenderNode;
  return <PageBody page={page} focus={focus} width={width} height={height} cx={{ ...cxOf(width, height, true), tabsFocused }} renderNode={renderNode} />;
}

/** The chrome's rows at the top (top bar, then the sources row when there is one). */
export function TopRows({ chrome, width, height }: { chrome: ChromeView; width: number; height: number }) {
  const frame = frameOf(chrome, null, width, height);
  return (
    <Box flexDirection="column" width={width}>
      <TopBarRow frame={frame} />
      <SourcesRow frame={frame} />
    </Box>
  );
}

/** The status line (the chrome's last row). */
export function StatusLine({ chrome, width, height }: { chrome: ChromeView; width: number; height: number }) {
  const frame = frameOf(chrome, null, width, height);
  return (
    <Box width={width}>
      <StatusRow frame={frame} />
    </Box>
  );
}

/** The body with a menu or question over it, as App draws it: the screen's panes, then MenuBox where layoutMenu puts it. */
export function BodyWithMenu({ chrome, screen, menu, width, bodyHeight, focus }: { chrome: ChromeView; screen: any; menu: MenuView; width: number; bodyHeight: number; focus?: string }) {
  // The frame for a screen whose body is `bodyHeight` rows: layoutMenu places the box inside the body by its height.
  const top = frameOf(chrome, null, width, 24).bodyTop;
  const frame = frameOf(chrome, menu, width, bodyHeight + top + 1);
  return (
    <Box height={frame.bodyHeight} width={width} overflow="hidden" flexDirection="column">
      <Box flexDirection="column" flexShrink={0} width={width}>
        <ScreenPanes screen={screen} focus={focus} width={width} height={frame.bodyHeight} cx={cxOf(width, frame.bodyHeight, true)} renderNode={shellRenderNode} />
      </Box>
      <MenuBox menu={menu} frame={frame} />
    </Box>
  );
}

/** Where the body sits under a chrome: its first row and height. */
export function bodyArea(chrome: ChromeView, width: number, height: number) {
  const f = frameOf(chrome, null, width, height);
  return { top: f.bodyTop, height: f.bodyHeight };
}
