import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { DiffView } from "../../src/text/DiffView";
import { fromText } from "../support/Compare";
import { PaneFigure, type Globals } from "../support/stories";
import unified80 from "../../test/fixtures/diff/diff_unified_80x24.json";
import unified80Text from "../../test/fixtures/diff/diff_unified_80x24.txt?raw";
import cursor80 from "../../test/fixtures/diff/diff_cursor_down_80x24.json";
import cursor80Text from "../../test/fixtures/diff/diff_cursor_down_80x24.txt?raw";
import comment80 from "../../test/fixtures/diff/diff_inline_comment_80x24.json";
import comment80Text from "../../test/fixtures/diff/diff_inline_comment_80x24.txt?raw";
import split160 from "../../test/fixtures/diff/diff_split_160x40.json";
import split160Text from "../../test/fixtures/diff/diff_split_160x40.txt?raw";

type Args = React.ComponentProps<typeof DiffView>;
/** The page's diff pane (its title differs between recordings: `4 of 5 · docs/README.md`, `docs/README.md`). */
const paneIn = (view: any) => (view.screen.page.panes as any[]).find((p) => p.body?.node === "diff");
/** The diff pane's title on Ratatui's screen: the frame that names README.md. */
const titleIn = (text: string) => /╭ ([^─]*README\.md) ─/.exec(text)?.[1] ?? "docs/README.md";
const argsOf = (view: any): Args => {
  const b = paneIn(view).body;
  return { lines: b.lines, under: b.under, selected: b.selected, top: b.top, split: b.split, painted: b.painted, empty: b.empty, width: 0, focused: true };
};

function InPane({ args, globals, view, text, file }: { args: Args; globals: Globals; view: any; text: string; file: string }) {
  return (
    <PaneFigure
      globals={globals}
      pane={paneIn(view)}
      focused={args.focused}
      reference={fromText(text, `ink/test/fixtures/diff/${file}`, { pane: titleIn(text) })}
      body={(ctx) => <DiffView {...args} width={ctx.width} height={ctx.height} focused={args.focused} />}
    />
  );
}

/**
 * One file's changes (`Node::Diff`, Ratatui `ui::diff::DiffView`): hunks with both sides' line numbers, added and
 * removed lines on a tint, the cursor row marked `▎`, code coloured by the host (`painted` runs; Ratatui highlights
 * itself with syntect), and blocks under a line (`under`: a thread, a comment being written).
 *
 * **Use it** for a merge request's Changes tab, one file at a time; the files are a tree in the nav pane.
 *
 * **States:** unified, split (two sides, 160 wide), cursor moved, a comment being written under a line.
 *
 * **Rules:** the cursor mark `▎` is in the gutter cell, not a stripe down the rows; tints and token colours are theme
 * slots (`diff.added`, `diff.removed`). Ink draws only rows in view.
 *
 * The references are Ratatui's screens as text (tests/view_fixtures.rs writes the `.txt` beside each fixture), so they
 * show no colours.
 */
const meta = {
  title: "Text/DiffView",
  component: DiffView,
  args: argsOf(unified80),
  render: (args, { globals }) => <InPane args={args} globals={globals} view={unified80} text={unified80Text} file={"diff_unified_80x24.txt"} />,
} satisfies Meta<typeof DiffView>;

export default meta;
type Story = StoryObj<typeof meta>;

/** docs/README.md in unified form, the cursor on the first hunk. */
export const Unified: Story = {};

/** The cursor moved down: the `▎` mark follows it. */
export const CursorMoved: Story = {
  args: argsOf(cursor80),
  render: (args, { globals }) => <InPane args={args} globals={globals} view={cursor80} text={cursor80Text} file={"diff_cursor_down_80x24.txt"} />,
};

/** A comment being written under line 2 (`under` with an input). */
export const InlineComment: Story = {
  args: argsOf(comment80),
  render: (args, { globals }) => <InPane args={args} globals={globals} view={comment80} text={comment80Text} file={"diff_inline_comment_80x24.txt"} />,
};

/** Split form at 160x40: old on the left, new on the right. */
export const Split: Story = {
  args: argsOf(split160),
  render: (args, { globals }) => <InPane args={args} globals={globals} view={split160} text={split160Text} file={"diff_split_160x40.txt"} />,
};
