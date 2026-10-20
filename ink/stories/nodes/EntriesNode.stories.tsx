import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { EntriesNode } from "../../src/nodes/lists";
import { regionOf } from "../support/Compare";
import { PaneFigure, paneOf, type Globals } from "../support/stories";
import type { GoldenScene } from "../support/golden";
import red80 from "../../test/fixtures/gitlab_repos-page1-merge-requests-pipelines-on-main_80x24.json";
import red80Cells from "../../test/cells/gitlab_repos-page1-merge-requests-pipelines-on-main-80x24.json";
import red160 from "../../test/fixtures/gitlab_repos-page1-merge-requests-pipelines-on-main_160x40.json";
import red160Cells from "../../test/cells/gitlab_repos-page1-merge-requests-pipelines-on-main-160x40.json";

type Args = React.ComponentProps<typeof EntriesNode>;
const argsOf = (pane: any): Args => ({ rows: pane.body.rows, selected: pane.body.selected, empty: pane.body.empty, ctx: { width: 0, height: 0, focused: true } });

function InPane({ args, globals, pane, scene }: { args: Args; globals: Globals; pane: any; scene: GoldenScene }) {
  return (
    <PaneFigure
      globals={globals}
      pane={pane}
      focused={args.ctx.focused}
      region={regionOf(scene, { pane: pane.title })}
      body={(ctx) => <EntriesNode {...args} ctx={{ ...ctx, focused: args.ctx.focused }} />}
    />
  );
}

const pane80 = paneOf(red80, "why main is red");
const pane160 = paneOf(red160, "why main is red");

/**
 * Entries (`Node::Entries`, the earlier `ui::entries::Entries`): selectable rows with lines under them (a failure and the
 * lines that say why) and lines before them that are never chosen (a heading). The selected entry's lines are kept in
 * view.
 *
 * **Use it** where each item needs a few lines of its own: failed jobs and their log lines, threads and their
 * comments.
 *
 * **States:** an entry selected with its lines, the others plain, empty (the `empty` words).
 *
 * **Rules:** the selection is the background across the entry's row; marks (✓ ✗ ●) come from the host's `Mark` and
 * the icon pack, never typed in by a part.
 */
const meta = {
  title: "Nodes/EntriesNode",
  component: EntriesNode,
  args: argsOf(pane80),
  render: (args, { globals }) => <InPane args={args} globals={globals} pane={pane80} scene={red80Cells as GoldenScene} />,
} satisfies Meta<typeof EntriesNode>;

export default meta;
type Story = StoryObj<typeof meta>;

/** "Why main is red" on a repository's Pipelines on main tab at 80x24: the failed jobs with their lines. */
export const FailedJobs: Story = {};

/** The same at 160x40. */
export const Wide: Story = {
  args: argsOf(pane160),
  render: (args, { globals }) => <InPane args={args} globals={globals} pane={pane160} scene={red160Cells as GoldenScene} />,
};
