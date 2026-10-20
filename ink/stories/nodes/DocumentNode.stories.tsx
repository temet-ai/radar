import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { NodeView } from "../../src/nodes/index";
import type { Node } from "../../src/nodes/view";
import { Compare, regionOf, sizeOf } from "../support/Compare";
import { InPanel } from "../support/frames";
import { PaneFigure, paneOf, type Globals } from "../support/stories";
import type { GoldenScene } from "../support/golden";
import connection80 from "../../test/fixtures/gitlab_settings-connection_80x24.json";
import connection80Cells from "../../test/cells/gitlab_settings-connection-80x24.json";
import overview160 from "../../test/fixtures/gitlab_repos-page1-overview-changes-threads-pipeline-commits-ticket_160x40.json";
import overview160Cells from "../../test/cells/gitlab_repos-page1-overview-changes-threads-pipeline-commits-ticket-160x40.json";

type Args = React.ComponentProps<typeof NodeView>;
const merge = paneOf(overview160, "can it merge?");

/**
 * A document (`Node::Document`, the earlier `widgets::paragraph`; Ink nodes/index.tsx DocumentNode, reached through
 * NodeView as the shell reaches it): lines of styled text, wrapped between words to the pane, or cut at the edge when
 * `cut` is set (the earlier renderer ignored `cut`; Ink honours it). In a settings detail it scrolls in its pane
 * (`scrolls`), with the scrollbar on the frame's border.
 *
 * **Use it** for prose and facts with no rows to choose: a settings field's explanation, "can it merge?", a set-up
 * page.
 *
 * **States:** wrapped, cut, scrolling (settings detail), empty.
 *
 * **Rules:** section headings inside it are muted bold capitals (`widgets::heading`); label/value facts align at the
 * longest label plus two.
 *
 * The story's `node` is the document the host sent; width and height in `ctx` come from the pane.
 */
const meta = {
  title: "Nodes/DocumentNode",
  component: NodeView,
  args: { node: merge.body as Node, ctx: { width: 0, height: 0, focused: false } },
  render: function Render(args: Args, { globals }: { globals: Globals }) {
    return (
      <PaneFigure
        globals={globals}
        pane={merge}
        focused={args.ctx.focused}
        region={regionOf(overview160Cells as GoldenScene, { pane: "can it merge?" })}
        body={(ctx) => <NodeView node={args.node} ctx={{ ...ctx, focused: args.ctx.focused }} />}
      />
    );
  },
} satisfies Meta<typeof NodeView>;

export default meta;
type Story = StoryObj<typeof meta>;

/** "Can it merge?" beside a merge request's overview at 160x40. */
export const PagePane: Story = {};

/** A settings section's detail (GitLab connection, the server field): it scrolls in its panel. */
export const SettingsDetail: Story = {
  args: { node: { node: "document", lines: connection80.detail } as Node, ctx: { width: 0, height: 0, focused: false, scrolls: true } },
  render: function Render(args, { globals }) {
    const ref = regionOf(connection80Cells as GoldenScene, { pane: connection80.detail_title });
    const { w, h } = sizeOf(ref, { w: 56, h: 11 });
    return (
      <Compare
        globals={globals}
        cols={w}
        rows={h}
        ink={
          <InPanel title={connection80.detail_title} width={w} height={h} focused={args.ctx.focused}>
            <NodeView node={args.node} ctx={{ width: w - 2, height: h - 2, focused: args.ctx.focused, scrolls: true }} />
          </InPanel>
        }
      />
    );
  },
};
