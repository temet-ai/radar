import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { TreeNode } from "../../src/nodes/tree";
import type { Tree } from "../../src/protocol/generated/Tree";
import { fromScene } from "../support/Compare";
import { PaneFigure, paneOf, type Globals } from "../support/stories";
import type { RatatuiScene } from "../support/ratatui";
import repos80 from "../../test/fixtures/gitlab_repos_80x24.json";
import repos160 from "../../test/fixtures/gitlab_repos_160x40.json";
import repos80Cells from "../../test/cells/gitlab_repos-80x24.json";
import repos160Cells from "../../test/cells/gitlab_repos-160x40.json";
import navFocusCells from "../../test/cells/highlight_focused-pane-nav-80x24.json";

const pane80 = paneOf(repos80, "Everything");
const pane160 = paneOf(repos160, "Everything");
const tree80 = pane80.body as Tree;

/** The host's tree after `-` on team-a: the same rows with that group left out of `open`. */
const foldedTree: Tree = { ...tree80, open: (tree80.open ?? []).filter((p) => p.join("/") !== "t:team-a") };

type Args = React.ComponentProps<typeof TreeNode>;

function Figure({ args, globals, pane, scene, missing }: { args: Args; globals: Globals; pane: any; scene: RatatuiScene | undefined; missing?: string }) {
  return (
    <PaneFigure
      globals={globals}
      pane={pane}
      focused={args.ctx.focused}
      reference={scene ? fromScene(scene, { pane: pane.title }) : undefined}
      size={{ w: 56, h: 21 }}
      missing={missing}
      body={(ctx) => <TreeNode tree={args.tree} ctx={{ ...ctx, focused: args.ctx.focused }} />}
    />
  );
}

/**
 * The banded tree (`Node::Tree`, Ratatui `ui::tree::BandedTree`): the one tree every part uses for nested data
 * (groups, then sources, then items). Each row has a name, cells in the tree's columns (aligned at every depth and
 * dropped by `keep` rank as the width shrinks), optional chips after the name, and a `detail` shown whole or not at
 * all.
 *
 * **Use it** when rows group under others and folding helps (a tenancy, its repositories, their merge requests). Use
 * a table when the rows are one kind.
 *
 * **States:** open (tree lines `├─ └─ │` show what is open), folded (the row ends in `⋯ n`, the count of rows it
 * holds), selected (the selection background across the whole row, bold emphasis text), selected without focus (a
 * quieter background when another pane has the keys), group rows on the heading band.
 *
 * **Rules:** no chevrons (▸ ▾ › ⌄) and no `[+]`/`[-]` for folding: the tree lines and `⋯ n` say it. No left-edge
 * stripes for new work (the group stripe `▌` is on its way out, #45). Selection is the background across the row,
 * never a marker glyph. Keys: `space` toggles a fold, `+`/`=` open, `-` closes or goes to the parent.
 *
 * Width and height in `ctx` come from the pane the story draws it in.
 */
const meta = {
  title: "Nodes/TreeNode",
  component: TreeNode,
  args: { tree: tree80, ctx: { width: 54, height: 19, focused: true } },
  render: (args, { globals }) => <Figure args={args} globals={globals} pane={pane80} scene={repos80Cells as RatatuiScene} />,
} satisfies Meta<typeof TreeNode>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Every group open, the first group row selected with the keys in this pane (GitLab Repositories at 80x24). */
export const Open: Story = {};

/** The same tree at 160x40: the columns the 80-cell pane dropped come back, aligned at every depth. */
export const Wide: Story = {
  args: { tree: pane160.body as Tree, ctx: { width: 0, height: 0, focused: true } },
  render: (args, { globals }) => <Figure args={args} globals={globals} pane={pane160} scene={repos160Cells as RatatuiScene} />,
};

/** team-a folded: its row keeps its cells and ends in `⋯ n`; no chevron, no `[+]`. */
export const Folded: Story = {
  args: { tree: foldedTree },
  render: (args, { globals }) => <Figure args={args} globals={globals} pane={pane80} scene={undefined} missing={"tests/ratatui_cells.rs records no folded tree. A `highlight_folded-row` scene there would give one."} />,
};

/** The nav pane has the keys: the tree's selected row keeps a quieter selection background, the frame is muted. */
export const Unfocused: Story = {
  args: { ctx: { width: 54, height: 19, focused: false } },
  render: (args, { globals }) => <Figure args={args} globals={globals} pane={paneOf((navFocusCells as RatatuiScene).view!.screen, "Everything")} scene={navFocusCells as RatatuiScene} />,
};
