import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Box } from "ink";
import { TourCardBox } from "../../src/chrome/Chrome";
import type { TourCard } from "../../src/protocol/generated/TourCard";
import { Compare } from "../support/Compare";
import { InPanel } from "../support/frames";
import { type Globals } from "../support/stories";

type Args = React.ComponentProps<typeof TourCardBox>;

const text = (words: string) => [{ text: words }];
const padded = (s: string, n: number) => s + " ".repeat(Math.max(0, n - [...s].length));

/** Stop 1 of the tour as the host sends it (`radar setup tour`): a title, lines, a line to try and the keys. */
const FIRST: TourCard = {
  stop: 1, of: 5, title: "Every screen has a letter.",
  lines: [
    text("underlined in its name; press it from anywhere"),
    [],
    text(padded("R repositories  J jira          S slack         B branches", 64)),
    text(padded("I work          H history       , settings", 64)),
    [],
    text("↑ at the top of a pane reaches the bar, then ← →"),
  ],
  try_it: "try it: press J",
  keys: text("→ next   ← back   esc end the tour"),
};

/** The last stop: no `try_it`, its keys say how to leave. */
const LAST: TourCard = {
  stop: 5, of: 5, title: "That's the tour.",
  lines: [text("radar is read only for now: top right, always"), [], text("x  the menu of every key, wherever you are"), text(",  settings: everything from setup")],
  try_it: null,
  keys: text("⏎ start working   ← back"),
};

function Draw({ args, globals, width, height }: { args: Args; globals: Globals; width: number; height: number }) {
  return (
    <Compare
      globals={globals}
      cols={width}
      rows={height}      ink={
        <Box width={width} height={height}>
          <InPanel title="Repositories" width={width} height={height} focused>
            <Box />
          </InPanel>
          <TourCardBox card={args.card} width={args.width} height={args.height} />
        </Box>
      }
    />
  );
}

/**
 * The tour's card (`ChromeView.tour`, Ratatui `shell/tour.rs`): a panel titled `tour · N of M` over the app at the body's
 * lower right, one cell in from the right and from the bottom, with the kit's shadow. Inside, with a cell of padding:
 * the title in bold emphasis, a blank line, the stop's lines, a blank line, what to try in muted words (when there is
 * one), then its keys. It is as wide as its widest line plus 4 (at least 40, at most the body less 2) and as tall as its
 * lines plus 2.
 *
 * **Use it** through the host: `radar setup tour`, or the tour at the end of setup, sends the card for the stop that
 * shows and takes it away when the tour ends. Nothing but the shadow dims: the app under it stays lit and keeps its
 * click targets. A menu draws over it and dims it with the rest.
 *
 * **States:** a stop with something to try, the last stop without, narrow (80x24), a body too small for it (nothing
 * drawn under 10 wide or 4 tall).
 *
 * **Rules:** the panel is on bg.surface with the accent border (focused); square keys, no chevrons, no pill shapes.
 */
const meta = {
  title: "Chrome/TourCard",
  component: TourCardBox,
  args: { card: FIRST, width: 160, height: 37 },
  render: (args, { globals }) => <Draw args={args} globals={globals} width={160} height={37} />,
} satisfies Meta<typeof TourCardBox>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Stop 1 of 5 at 160x40: the body is 37 rows under the sources row. */
export const First: Story = {};

/** The last stop: nothing to try, so the keys follow the lines directly. */
export const Last: Story = {
  args: { card: LAST },
};

/** At 80x24 the card keeps its width (the widest line plus 4) and sits in the lower right. */
export const Narrow: Story = {
  args: { card: { ...FIRST, lines: [text("press the letter of a screen")] }, width: 80, height: 21 },
  render: (args, { globals }) => <Draw args={args} globals={globals} width={80} height={21} />,
};
