import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { ListNode } from "../../src/nodes/lists";
import { fromScene, sizeOf, Compare } from "../support/Compare";
import { InPanel } from "../support/frames";
import { PaneFigure, paneOf, type Globals } from "../support/stories";
import type { RatatuiScene } from "../support/ratatui";
import repos80 from "../../test/fixtures/gitlab_repos_80x24.json";
import repos80Cells from "../../test/cells/gitlab_repos-80x24.json";
import navFocusCells from "../../test/cells/highlight_focused-pane-nav-80x24.json";
import connection80 from "../../test/fixtures/gitlab_settings-connection_80x24.json";
import connection80Cells from "../../test/cells/gitlab_settings-connection-80x24.json";

type Args = React.ComponentProps<typeof ListNode>;
const views = paneOf(repos80, "views");
const navFocused = paneOf((navFocusCells as RatatuiScene).view!.screen, "views");

function InPane({ args, globals, pane, scene }: { args: Args; globals: Globals; pane: any; scene: RatatuiScene }) {
  return (
    <PaneFigure
      globals={globals}
      pane={pane}
      focused={args.ctx.focused}
      reference={fromScene(scene, { pane: pane.title })}
      body={(ctx) => <ListNode {...args} ctx={{ ...ctx, focused: args.ctx.focused }} />}
    />
  );
}

/**
 * A row list (`Node::List`, Ratatui `ui::rows::RowList`): selectable rows, each with words at the left, an optional
 * word at the right (`right`) and a `detail` dropped whole when it does not fit. A heading row (`ListRow.heading`)
 * is never selected; the cursor steps over it.
 *
 * **Use it** for a nav pane's views, a settings section's rows and any flat choice of places. Use a table for items
 * with columns, a tree for nested items.
 *
 * **States:** selected with the keys (bold emphasis on the selection background across the row), selected without
 * the keys (the quieter selection), plain rows, a heading, a row being edited in place (`editing`).
 *
 * **Rules:** the selection is the background across the whole row, never a marker glyph (no `›`, `▶`). Counts at
 * the right are `data.number`, plain words. Keys `↓ ↑ Home End PgDn PgUp` move over headings.
 */
const meta = {
  title: "Nodes/ListNode",
  component: ListNode,
  args: { rows: views.body.rows, selected: views.body.selected, ctx: { width: 22, height: 19, focused: false } },
  render: (args, { globals }) => <InPane args={args} globals={globals} pane={views} scene={repos80Cells as RatatuiScene} />,
} satisfies Meta<typeof ListNode>;

export default meta;
type Story = StoryObj<typeof meta>;

/** GitLab's views in the nav while the middle pane has the keys: the selected row on the quieter selection. */
export const Unfocused: Story = {};

/** The nav pane has the keys (`highlight_focused-pane-nav`): the selected row bold on the selection background. */
export const Focused: Story = {
  args: { rows: navFocused.body.rows, selected: navFocused.body.selected, ctx: { width: 22, height: 19, focused: true } },
  render: (args, { globals }) => <InPane args={args} globals={globals} pane={navFocused} scene={navFocusCells as RatatuiScene} />,
};

/** A settings section's rows (GitLab connection): label and value rows in a plain panel, as App's SectionBody
 * draws a section. */
export const SettingsRows: Story = {
  args: { rows: connection80.body.rows as Args["rows"], selected: connection80.body.selected, ctx: { width: 0, height: 0, focused: true } },
  render: function Render(args, { globals }) {
    const ref = fromScene(connection80Cells as RatatuiScene, { pane: "GitLab connection" });
    const { w, h } = sizeOf(ref, { w: 56, h: 10 });
    return (
      <Compare
        globals={globals}
        cols={w}
        rows={h}
        reference={ref}
        ink={
          <InPanel title="GitLab connection" width={w} height={h} focused={args.ctx.focused}>
            <ListNode {...args} ctx={{ width: w - 2, height: h - 2, focused: args.ctx.focused }} />
          </InPanel>
        }
      />
    );
  },
};
