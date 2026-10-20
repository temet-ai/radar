import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { PaneFrame } from "../../src/app/Panes";
import { shellRenderNode } from "../../src/app/Body";
import { Compare, regionOf, sizeOf } from "../support/Compare";
import { paneOf, type Globals } from "../support/stories";
import type { GoldenScene } from "../support/golden";
import typing80Cells from "../../test/cells/highlight_typing-filter-80x24.json";
import typing160Cells from "../../test/cells/highlight_typing-filter-160x40.json";

type Args = React.ComponentProps<typeof PaneFrame>;

const argsOf = (scene: GoldenScene): Args => {
  const pane = paneOf(scene.view!.screen, "Everything");
  const ref = regionOf(scene, { pane: "Everything" });
  const { w, h } = sizeOf(ref, { w: 56, h: 21 });
  return { pane, width: w, height: h, focused: true, cx: { width: w, height: h, actions: [], focused: true }, renderNode: shellRenderNode };
};

function Draw({ args, globals }: { args: Args; globals: Globals }) {
  return (
    <Compare
      globals={globals}
      cols={args.width}
      rows={args.height}
      ink={<PaneFrame {...args} />}
    />
  );
}

/**
 * The field bar (`PaneView.fields` and `bottom_field`, a `FieldBar`; Ratatui `view_tui::draw_field_bar` with
 * `input::TextInput`; Ink chrome/filter.tsx, drawn by PaneFrame): the `/` filter and the `:` go-to at the top of a
 * pane. Three rows: the mark ` / `, the typed text on the overlay shade with the caret cell reversed, a hint row and a
 * blank row; the rows under it narrow as you type.
 *
 * **Use it** through the host: `/` filters any screen, `:` goes to an item by its id. Parts never draw their own
 * filter box.
 *
 * **States:** typing (caret at the end or inside), empty, a hint, the bottom field under the body.
 *
 * **Rules:** the caret is a reversed cell, never a typed `|` or `_`; the field's ground is `bg.overlay`; the hint uses
 * glyph keys before the word (`⏎ keep`, `esc clear`).
 */
const meta = {
  title: "Chrome/FieldBar",
  component: PaneFrame,
  args: argsOf(typing80Cells as GoldenScene),
  render: (args, { globals }) => <Draw args={args} globals={globals} />,
} satisfies Meta<typeof PaneFrame>;

export default meta;
type Story = StoryObj<typeof meta>;

/** `/` then `g` `g` on the Repositories screen at 80x24 (`highlight_typing-filter`): the tree narrows to matches. */
export const Typing: Story = {};

/** The same at 160x40. */
export const Wide: Story = {
  args: argsOf(typing160Cells as GoldenScene),
  render: (args, { globals }) => <Draw args={args} globals={globals} />,
};
