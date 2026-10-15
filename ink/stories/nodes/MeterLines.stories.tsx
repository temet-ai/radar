import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { NodeView } from "../../src/nodes/index";
import type { Node } from "../../src/nodes/view";
import { fromText } from "../support/Compare";
import { PaneFigure, paneOf, type Globals } from "../support/stories";
import actions160 from "../../test/fixtures/reference/mix__gitlab+github+jira+slack+git+workspace+workbench+sessions_row_actions_160x40.json";
import actions160Snap from "../../../tests/snapshots/mix__gitlab+github+jira+slack+git+workspace+workbench+sessions_row_actions_160x40.snap?raw";

type Args = React.ComponentProps<typeof NodeView>;
const pane = paneOf(actions160.screen, "api !263");

/**
 * The stage bar (`Block::Meter` in a `Node::Rich`, Ratatui `ui::meter::stages`; Ink nodes/index.tsx MeterLines,
 * reached through NodeView): the steps of a run across the width, a bar of `▀` over each stage's mark and name, each
 * in the stage's look (its run state's tone). It sits under a PIPELINE section (`Block::Section`).
 *
 * **Use it** under a PIPELINE heading in a detail pane or an item page, for a pipeline's or a workflow's stages.
 *
 * **States:** each stage by its `Mark` (done ✓, failed ✗, running ●, waiting ◷, manual ▷, stopped ⊘, skipped »);
 * names cut to the stage's share of the width; no stages draws nothing.
 *
 * **Rules:** the marks read without colour; the colour is the stage's look, a theme slot. No pill or round caps on
 * the bars.
 *
 * **Data gap:** every recorded view (ink/test/fixtures) carries `"stages": []`, so this story shows the real detail
 * pane where the meter sits, with an empty bar. A surface with stages (a merge request page's Pipeline tab) recorded by
 * tests/view_fixtures.rs and tests/ratatui_cells.rs would give both renderers' pictures of a filled bar.
 *
 * The story's `node` is the whole rich node of the pane, as the host sent it.
 */
const meta = {
  title: "Nodes/MeterLines",
  component: NodeView,
  args: { node: pane.body as Node, ctx: { width: 0, focused: false } },
  render: function Render(args: Args, { globals }: { globals: Globals }) {
    return (
      <PaneFigure
        globals={globals}
        pane={pane}
        focused={args.ctx.focused}
        reference={fromText(actions160Snap, "tests/snapshots/mix__gitlab+github+jira+slack+git+workspace+workbench_row_actions_160x40.snap", { snap: true, pane: "api !263" })}
        body={(ctx) => <NodeView node={args.node} ctx={{ ...ctx, focused: args.ctx.focused }} />}
      />
    );
  },
} satisfies Meta<typeof NodeView>;

export default meta;
type Story = StoryObj<typeof meta>;

/** A merge request's detail pane at 160x40: the PIPELINE section, passed, and the meter the host sent (no stages
 * recorded). */
export const RecordedDetail: Story = {};
