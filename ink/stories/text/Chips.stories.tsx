import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Box } from "ink";
import { TextLine } from "../../src/text/TextLine";
import type { Seg } from "../../src/nodes/view";
import { Compare } from "../support/Compare";
import { type Globals } from "../support/stories";
import tickets160 from "../../test/fixtures/jira_tickets_160x40.json";
import discussed from "../../test/fixtures/chips/jira-discussed.json";

type Args = React.ComponentProps<typeof TextLine>;

/** ABC-12's chips as the host sent them (Jira My tickets, 160x40). */
function recordedChips(): Seg[] {
  let found: Seg[] | undefined;
  const walk = (x: any) => {
    if (found || !x || typeof x !== "object") return;
    if (x.chips?.text) found = x.chips.text;
    else for (const v of Array.isArray(x) ? x : Object.values(x)) walk(v);
  };
  walk(tickets160);
  return found!;
}
const recorded = recordedChips();
const chip = recorded.find((s) => s.chip)!;

/** The recorded chip in every tone the host has (`Ink` names widgets::tone maps): the same seg, its `ink` changed. */
const TONES = ["plain", "muted", "accent", "info", "good", "warn", "bad"] as const;
const everyTone: Seg[] = TONES.flatMap((ink) => [{ ...chip, ink, text: ink, link: null }, { text: " " }]);

/** The width of the recorded `discussed · 1` chip in cells (ink/test/fixtures/chips/jira-discussed.json). */
const discussedWidth = discussed.cells.length;

/**
 * A chip (`Seg.chip`, `widgets::chip` and `ui::chips::chip_spans` in the earlier renderer): a short label after a row's name, on the
 * overlay shade, one space each side: ` ` + the icon and its gap in the icon's own colour + `words ` in the tone's
 * colour. A row's `Chips.gives` names the run of the name that gives way to them while it keeps a dozen cells.
 *
 * **Use it** for a state or a fact worth seeing in the list (discussed in Slack, a local folder, needs you).
 *
 * **States:** each tone (plain, muted, accent, info, good, warn, bad; accent, warn and bad are bold), with or without
 * an icon, given way (not drawn) when the name needs the room.
 *
 * **Rules:** square, never a pill: a ground and one space each side, no round caps (powerline half circles). No left stripe. The
 * tone is a theme slot through `widgets::tone`; a part never picks the colour.
 */
const meta = {
  title: "Text/Chips",
  component: TextLine,
  args: { text: recorded },
  render: function Render(args: Args, { globals }: { globals: Globals }) {
    return (
      <Compare
        globals={globals}
        cols={discussedWidth}
        rows={1}
        ink={
          <Box width={discussedWidth}>
            <TextLine text={args.text.filter((s) => s.chip)} />
          </Box>
        }
      />
    );
  },
} satisfies Meta<typeof TextLine>;

export default meta;
type Story = StoryObj<typeof meta>;

/** The chip on Jira ABC-12 (`discussed · 1`, muted, the thread icon). Turn the icons on in the toolbar to see the icon
 * slot. */
export const Recorded: Story = {};

/** The recorded chip in each of the seven tones, with its words replaced by the tone's name (the recorded views carry
 * only the muted chip). */
export const EveryTone: Story = {
  args: { text: everyTone },
  render: function Render(args, { globals }) {
    return (
      <Compare
        globals={globals}
        cols={110}
        rows={1}        ink={
          <Box width={110}>
            <TextLine text={args.text} />
          </Box>
        }
      />
    );
  },
};
