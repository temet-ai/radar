import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { RailRows } from "../../src/chrome/Chrome";
import { layoutRail } from "../../src/chrome/rail";
import type { RailStop } from "../../src/protocol/generated/RailStop";
import { Compare } from "../support/Compare";
import { type Globals } from "../support/stories";

type Args = React.ComponentProps<typeof RailRows>;

const stop = (i: number, label: string, state: RailStop["state"], result: string | null): RailStop => ({
  id: label,
  label,
  icon: "board",
  state,
  result,
  target: `chrome:setup:${i}`,
});

/** The run the kit's test draws: a step in every state, and one with a long name. */
const STOPS: RailStop[] = [
  stop(0, "look & terminal", "done", "neon · nerd icons"),
  stop(1, "local code", "skipped", "skipped"),
  stop(2, "Shelves", "current", "step 3 of 5"),
  stop(3, "the Unseen University archive", "upcoming", null),
  stop(4, "Ledger", "upcoming", null),
  stop(5, "tour?", "upcoming", "asked at the end"),
];

function Draw({ globals, width }: { globals: Globals; width: number }) {
  return (
    <Compare
      globals={globals}
      cols={width}
      rows={2}
      ink={<RailRows rail={layoutRail(STOPS, width)} />}
    />
  );
}

/**
 * The step rail of setup (`ChromeView.setup.rail`, Ratatui `ui::rail`, the kit's element for the steps of a run along
 * one line). Each stop sits on a track at its own place; under it a line says how it went. It is two rows tall, one
 * cell in from each side of setup's body, and a click on a stop (its slot, both rows) goes back to it.
 *
 * **Use it** for the steps of a guided run: the host fills `RailStop` (id, label, icon, state, result, target); a part
 * offers a step and the host puts it on the rail.
 *
 * **States:** `done` (a check, the label in primary text, its result under it), `skipped` (`»`, muted, `skipped`),
 * `upcoming` (a hollow dot, muted), `current` (a square chip on the accent with the step's own icon, `step n of m`
 * under it). The track is lit up to the current stop, or the last one done or skipped when none is current.
 *
 * **Rules:** the chip is square, never a pill; a cell of ground on each side of a label breaks the track around it;
 * a label is cut to its stop's room in display cells with an ellipsis, never padded; a stop with no room for its words
 * draws nothing and is not a target. No chevrons, no left-edge stripes.
 *
 * The story draws `layoutRail` (chrome/rail.ts) through `RailRows` (chrome/Chrome.tsx); the colours are asserted in
 * test/chrome/rail.test.ts.
 */
const meta = {
  title: "Chrome/StepRail",
  component: RailRows,
  args: { rail: layoutRail(STOPS, 80) },
  render: (_args, { globals }) => <Draw globals={globals} width={80} />,
} satisfies Meta<typeof RailRows>;

export default meta;
type Story = StoryObj<typeof meta>;

/** At 80 wide the labels are cut to their stops' room. */
export const Narrow: Story = {};

/** At 160 wide each stop has room for its words; the long name is cut once. */
export const Wide: Story = {
  args: { rail: layoutRail(STOPS, 160) },
  render: (_args, { globals }) => <Draw globals={globals} width={160} />,
};
