import React from "react";
import { Box, Text } from "ink";
import { renderNode as drawNode, type NodeContext } from "../nodes/index.js";
import type { ActionSpec } from "./actions.js";
import { ActionList as ActionRows } from "../nodes/actions.js";

/** What the shell gives every node: the room, who has the keys, and the actions of the focused place. */
export type NodeCx = {
  width: number;
  height: number;
  actions: ActionSpec[];
  focused?: boolean;
  plainText?: boolean;
  /** A Document scrolls in this pane (the settings detail). */
  scrolls?: boolean;
  /** The pane is the sheet of settings or setup (NodeContext.sheet). */
  sheet?: boolean;
  onIntent?: NodeContext["onIntent"];
  registry?: NodeContext["registry"];
  /** The page's tab row has the keys (ChromeView.keys_at is page_tabs). */
  tabsFocused?: boolean;
  /** Host-owned active-surface digit routing, including logically hidden panes. */
  paneJumps?: readonly { pane: string; key: string }[];
  scrollIdentity?: string;
  /** NodeContext.pane and .scrollTo: a screen pane's body reports its rows and takes the part's scroll. */
  pane?: string;
  scrollTo?: number;
  bodyTrack?: boolean;
  writeMode?: import("../nodes/detail.js").WriteMode;
};

export type RenderNode = (node: any, cx: NodeCx) => React.ReactNode;

const KNOWN = new Set(["list", "tree", "document", "rich", "table", "entries", "log", "diff", "state", "extension"]);

/**
 * The shell's node entry: T11's `renderNode` for every node, and for a node type this renderer
 * does not know a visible box naming it (T11's) with the focused place's actions listed under it,
 * so what the part offers stays reachable (their keys still invoke, App declares them).
 */
export const shellRenderNode: RenderNode = (node, cx) => {
  const ctx: NodeContext = {
    width: cx.width,
    height: cx.height,
    focused: cx.focused ?? true,
    plainText: cx.plainText,
    scrolls: cx.scrolls,
    sheet: cx.sheet,
    onIntent: cx.onIntent,
    registry: cx.registry,
    scrollIdentity: cx.scrollIdentity,
    pane: cx.pane,
    scrollTo: cx.scrollTo,
    bodyTrack: cx.bodyTrack,
  };
  const body = drawNode(node, ctx);
  if (node && typeof node === "object" && KNOWN.has(String(node.node))) return body;
  return (
    <Box flexDirection="column">
      {body}
      <ActionList actions={cx.actions} plain={cx.plainText} />
    </Box>
  );
};

export function ActionList({ actions, plain }: { actions: ActionSpec[]; plain?: boolean }) {
  const runnable = actions.filter((a) => !a.heading);
  if (runnable.length === 0) return <Text dimColor>no actions here</Text>;
  return <ActionRows actions={runnable} plain={plain} />;
}
