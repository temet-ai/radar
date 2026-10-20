import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { StateView } from "../../src/text/StateView";
import { PaneFigure, paneOf, type Globals } from "../support/stories";
import start80 from "../../test/fixtures/reference/mix__git_start_80x24.json";

type Args = React.ComponentProps<typeof StateView>;
const pane = paneOf(start80.screen, "all");

/** The recorded pane with its state set by the story: no recorded view has a pane loading or failed. */
function InPane({ args, globals }: { args: Args; globals: Globals }) {
  return (
    <PaneFigure
      globals={globals}
      pane={pane}
      focused
      size={{ w: 80, h: 24 }}
      body={(ctx) => <StateView {...args} width={ctx.width} />}
    />
  );
}

/**
 * A pane's state (`Node::State`, the earlier `ui::states::empty`): what a pane shows in place of rows when there is
 * nothing yet, a fetch is running, or it failed. The text says what happened and what to do.
 *
 * **Use it** when a whole pane has nothing to show. When the rows must stay selectable (a group with nothing under
 * it), use a tree `Note` instead.
 *
 * **States:** `empty`, `loading`, `failed`. Today the three look alike on both renderers (docs/components.md
 * section 2), so the words carry the state.
 *
 * **Rules:** say what to do next in words ("Press , for settings"); no spinner glyph typed in by a part (the host's
 * spinner is the kit's); paths in the text join with `/` (the recorded text still says `git › checkouts`, #46).
 */
const meta = {
  title: "Text/StateView",
  component: StateView,
  args: { state: pane.body.state, text: pane.body.text, width: 78 },
  render: (args, { globals }) => <InPane args={args} globals={globals} />,
} satisfies Meta<typeof StateView>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Git with no clones known: the recorded empty state. */
export const Empty: Story = {};

/** The same node with `state: "loading"` (the recorded words kept). */
export const Loading: Story = { args: { state: "loading" }, render: (args, { globals }) => <InPane args={args} globals={globals} /> };

/** The same node with `state: "failed"` (the recorded words kept). */
export const Failed: Story = { args: { state: "failed" }, render: (args, { globals }) => <InPane args={args} globals={globals} /> };
