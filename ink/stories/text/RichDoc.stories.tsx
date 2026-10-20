import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { RichBlocks } from "../../src/text/RichDoc";
import { NodeView } from "../../src/nodes/index";
import type { RichBlock } from "../../src/protocol/generated/RichBlock";
import { regionOf } from "../support/Compare";
import { PaneFigure, paneOf, type Globals } from "../support/stories";
import type { GoldenScene } from "../support/golden";
import ticket80 from "../../test/fixtures/jira_tickets-page1-overview-comments-links_80x24.json";
import ticket80Cells from "../../test/cells/jira_tickets-page1-overview-comments-links-80x24.json";
import ticket160 from "../../test/fixtures/jira_tickets-page1-overview-comments-links_160x40.json";
import ticket160Cells from "../../test/cells/jira_tickets-page1-overview-comments-links-160x40.json";

type Args = React.ComponentProps<typeof RichBlocks>;
const docIndex = (pane: any) => pane.body.blocks.findIndex((b: any) => b.block === "doc");
const argsOf = (pane: any): Args => ({ blocks: pane.body.blocks[docIndex(pane)].doc as RichBlock[] });

/** The pane's whole rich node, its doc block holding the story's blocks: the blocks go the way radar draws them
 * (NodeView, then BlockView, then RichBlocks). */
function InPane({ args, globals, pane, scene }: { args: Args; globals: Globals; pane: any; scene: GoldenScene }) {
  const i = docIndex(pane);
  const node = { ...pane.body, blocks: pane.body.blocks.map((b: any, j: number) => (j === i ? { ...b, doc: args.blocks } : b)) };
  return (
    <PaneFigure
      globals={globals}
      pane={pane}
      focused
      region={regionOf(scene, { pane: pane.title })}
      body={(ctx) => <NodeView node={node} ctx={ctx} />}
    />
  );
}

const pane80 = paneOf(ticket80, "overview");
const pane160 = paneOf(ticket160, "overview");

/**
 * Rich text (`Block::Doc` in a `Node::Rich`, the earlier `ui::rich`): any service's formatted text (markdown, Jira's
 * ADF, Slack's mrkdwn) as one document laid out to the width: paragraphs, headings, lists that hang, quotes,
 * call-outs, code, tables as columns, rules, media; runs with marks, links, mentions and emoji.
 *
 * **Use it** for a ticket's description, a merge request's description, a comment, a Slack message. Plain facts go
 * in line blocks around it.
 *
 * **States:** laid out at the pane's width (wrapped between words), scrolled when taller than the pane.
 *
 * **Rules:** a blank row between blocks; links are words (OSC 8 where the terminal can), not raw URLs; list markers
 * come from the document, never chevrons. The story's `blocks` are the ticket's description; the lines around it are
 * the rest of the same recorded pane.
 */
const meta = {
  title: "Text/RichDoc",
  component: RichBlocks,
  args: argsOf(pane80),
  render: (args, { globals }) => <InPane args={args} globals={globals} pane={pane80} scene={ticket80Cells as GoldenScene} />,
} satisfies Meta<typeof RichBlocks>;

export default meta;
type Story = StoryObj<typeof meta>;

/** A Jira ticket's overview at 80x24: facts, then the description. */
export const TicketDescription: Story = {};

/** The same at 160x40. */
export const Wide: Story = {
  args: argsOf(pane160),
  render: (args, { globals }) => <InPane args={args} globals={globals} pane={pane160} scene={ticket160Cells as GoldenScene} />,
};
