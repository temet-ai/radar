import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { LogNode } from "../../src/nodes/lists";
import { regionOf } from "../support/Compare";
import { PaneFigure, paneOf, type Globals } from "../support/stories";
import type { GoldenScene } from "../support/golden";
import log80 from "../../test/fixtures/gitlab_repos-page2-log_80x24.json";
import log80Cells from "../../test/cells/gitlab_repos-page2-log-80x24.json";
import log160 from "../../test/fixtures/gitlab_repos-page2-log_160x40.json";
import log160Cells from "../../test/cells/gitlab_repos-page2-log-160x40.json";

type Args = React.ComponentProps<typeof LogNode>;
const argsOf = (pane: any): Args => ({ lines: pane.body.lines, top: pane.body.top, total: pane.body.total, empty: pane.body.empty, ctx: { width: 0, height: 0, focused: true } });

function InPane({ args, globals, pane, scene }: { args: Args; globals: Globals; pane: any; scene: GoldenScene }) {
  return (
    <PaneFigure
      globals={globals}
      pane={pane}
      focused={args.ctx.focused}
      region={regionOf(scene, { pane: pane.title })}
      body={(ctx) => <LogNode {...args} ctx={{ ...ctx, focused: args.ctx.focused }} />}
    />
  );
}

const pane80 = paneOf(log80, "log");
const pane160 = paneOf(log160, "log");

/**
 * A log (`Node::Log`, Ratatui inline loop plus `ui::scroll::scrollbar`): long output such as a job's trace, with a
 * terminal's colours mapped to theme slots (`LogPaintLine` runs), sections that fold under their titles, `/` matches
 * standing out, and a scrollbar on the frame's right border when it scrolls.
 *
 * **Use it** for any long machine output. Use entries when each item needs only a few lines.
 *
 * **States:** the tail shown (the host sends the last lines it holds), scrolled (`top`), empty.
 *
 * **Rules:** colours come from the slots the host mapped the ANSI colours to, never the raw ANSI; folded sections
 * read `─ … ⋯ n lines`, no chevrons. Keys: `/` search, `n`/`N` between matches, `+`/`-` fold.
 */
const meta = {
  title: "Nodes/LogNode",
  component: LogNode,
  args: argsOf(pane80),
  render: (args, { globals }) => <InPane args={args} globals={globals} pane={pane80} scene={log80Cells as GoldenScene} />,
} satisfies Meta<typeof LogNode>;

export default meta;
type Story = StoryObj<typeof meta>;

/** A failed job's log tail on its page at 80x24. */
export const Tail: Story = {};

/** The same at 160x40. */
export const Wide: Story = {
  args: argsOf(pane160),
  render: (args, { globals }) => <InPane args={args} globals={globals} pane={pane160} scene={log160Cells as GoldenScene} />,
};
