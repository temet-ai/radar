import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { TableNode } from "../../src/nodes/lists";
import { regionOf } from "../support/Compare";
import { PaneFigure, paneOf, type Globals } from "../support/stories";
import type { GoldenScene } from "../support/golden";
import mrs80 from "../../test/fixtures/gitlab_repos-page2-merge-requests-pipelines-on-main_80x24.json";
import mrs80Cells from "../../test/cells/gitlab_repos-page2-merge-requests-pipelines-on-main-80x24.json";
import mrs160 from "../../test/fixtures/gitlab_repos-page2-merge-requests-pipelines-on-main_160x40.json";
import mrs160Cells from "../../test/cells/gitlab_repos-page2-merge-requests-pipelines-on-main-160x40.json";
import repos80 from "../../test/fixtures/gitlab_repos-page1-repos_80x24.json";
import repos80Cells from "../../test/cells/gitlab_repos-page1-repos-80x24.json";

type Args = React.ComponentProps<typeof TableNode>;
const argsOf = (pane: any, focused = true): Args => ({ columns: pane.body.columns, rows: pane.body.rows, selected: pane.body.selected, empty: pane.body.empty, ctx: { width: 0, height: 0, focused } });

function InPane({ args, globals, pane, scene }: { args: Args; globals: Globals; pane: any; scene: GoldenScene }) {
  return (
    <PaneFigure
      globals={globals}
      pane={pane}
      focused={args.ctx.focused}
      region={regionOf(scene, { pane: pane.title })}
      body={(ctx) => <TableNode {...args} ctx={{ ...ctx, focused: args.ctx.focused }} />}
    />
  );
}

const mrs80Pane = paneOf(mrs80, "merge requests");
const mrs160Pane = paneOf(mrs160, "merge requests");
const reposPane = paneOf(repos80, "repos");

/**
 * A table (`Node::Table`, the earlier `ui::table::Table`): a flat list of one kind of row with column headings and a
 * cursor. Columns come from `ColumnSpec` (the same `Columns` a tree uses, so a column means the same in both), are
 * dropped by `keep` rank as the width shrinks, and values that do not fit are cut with `…`.
 *
 * **Use it** for a page's list of items or a history of runs, where every row is the same kind. Use a tree when rows
 * group under others.
 *
 * **States:** rows with the cursor row selected, empty (the `empty` words in place of rows), columns dropped when
 * narrow.
 *
 * **Rules:** headings are muted bold capitals; the selection is the background across the row; no counts in tab
 * labels above it (the tab says "Merge requests", not "Merge requests · 1", #33).
 */
const meta = {
  title: "Nodes/TableNode",
  component: TableNode,
  args: argsOf(mrs80Pane),
  render: (args, { globals }) => <InPane args={args} globals={globals} pane={mrs80Pane} scene={mrs80Cells as GoldenScene} />,
} satisfies Meta<typeof TableNode>;

export default meta;
type Story = StoryObj<typeof meta>;

/** A repository's merge requests on its page at 80x24, the first row selected. */
export const Rows: Story = {};

/** The same table at 160x40: the columns dropped at 80 come back. */
export const Wide: Story = {
  args: argsOf(mrs160Pane),
  render: (args, { globals }) => <InPane args={args} globals={globals} pane={mrs160Pane} scene={mrs160Cells as GoldenScene} />,
};

/** A table with no rows: the host's `empty` words where the rows go. */
export const Empty: Story = {
  args: argsOf(reposPane),
  render: (args, { globals }) => <InPane args={args} globals={globals} pane={reposPane} scene={repos80Cells as GoldenScene} />,
};
