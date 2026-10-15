import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { card } from "./Board";
import { Row, ShippedLabel, Target } from "./parts";
import { MotionReference, MotionReplay } from "./MotionReplay";
import type { Cue } from "../../src/motion/engine";

type Args = { name: string; speed: "real" | "slow" };

/** changed-full's recorded cue holds one changed row and one added row; each story runs one of them. */
const GLOW_ONLY: Cue = { kind: "changed", changed: ["merge-request:acme/team-b/worker!342"], added: [] };
const ADDED_ONLY: Cue = { kind: "changed", changed: [], added: ["merge-request:acme/team-a/api!263"] };

function Shipped({ args, base, cue, what }: { args: Args; base: string; cue?: Cue; what: string }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
      <ShippedLabel what={what} />
      <Row>
        <MotionReplay name={`${base}-start`} speed={args.speed} cue={cue} />
        <MotionReference base={base} />
      </Row>
    </div>
  );
}

/**
 * Motion (board 07). **Shipped effects run for real:** each story drives Ink's own Engine (src/motion/engine.ts, the
 * effects in effects.ts) over the real shell frame, marks and cue of a Rust motion fixture
 * (ink/test/fixtures/motion, written by tests/motion_cells.rs), frame by frame every 30 ms, then pauses and runs again;
 * Ratatui's cells at the start, middle and end of the same effect sit beside it.
 *
 * **Next effects** are the canvas's own CSS animations: fold unroll and the rainbow focus lap (#32), assemble and
 * loading outlines (#42), the notice drain (#41). Each says "Target · not built yet".
 *
 * `speed`: real time, or four times slower to watch an effect. Motion never delays a key; any key cancels it; it is
 * off over SSH and on slow terminals.
 */
const meta = {
  title: "Design/Motion",
  component: MotionReplay,
  parameters: { layout: "padded" },
  argTypes: { speed: { control: "inline-radio", options: ["real", "slow"] }, name: { control: false } },
  args: { name: "screen-full-start", speed: "real" },
} satisfies Meta<typeof MotionReplay>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Screen fade: a new screen fades in from its text colour over 200 ms. */
export const ScreenFade: Story = { render: (args) => <Shipped args={args} base="screen-full" what="screen fade, Ink's Engine" /> };

/** Page sweep: an item page sweeps in from the left over 300 ms. */
export const PageSweep: Story = { render: (args) => <Shipped args={args} base="page-full" what="page sweep, Ink's Engine" /> };

/** Popup sweep: a menu sweeps down over 180 ms. */
export const PopupSweep: Story = { render: (args) => <Shipped args={args} base="popup-full" what="popup sweep, Ink's Engine" /> };

/** Message: a status message sweeps in over 250 ms. */
export const Message: Story = { render: (args) => <Shipped args={args} base="message-full" what="message in, Ink's Engine" /> };

/** Message gone: the message fades out over 200 ms. */
export const MessageGone: Story = { render: (args) => <Shipped args={args} base="message_gone-full" what="message out, Ink's Engine" /> };

/** Changed row glow: a row whose data changed glows in the accent and settles over 450 ms. */
export const ChangedRowGlow: Story = {
  render: (args) => <Shipped args={args} base="changed-full" cue={GLOW_ONLY} what="changed-row glow, Ink's Engine (the recorded cue's changed row)" />,
};

/** Added row: a new row sweeps in from the left over 320 ms. */
export const AddedRow: Story = {
  render: (args) => <Shipped args={args} base="changed-full" cue={ADDED_ONLY} what="added row, Ink's Engine (the recorded cue's added row)" />,
};

/** Shimmer: a panel waiting on its data shimmers (1.4 s a pass) until the rows come. */
export const Shimmer: Story = { render: (args) => <Shipped args={args} base="loading-full" what="loading shimmer, Ink's Engine" /> };

const boardSpeed = (a: Args) => ({ speed: a.speed === "slow" ? "slow" : "real" });

/** Fold unroll: a folded group's rows unroll from its line. */
export const FoldUnroll: Story = { render: (args) => <Target board="Motion" picks={[card("Fold")]} issues={[32]} props={boardSpeed(args)} width={720} /> };

/** A rainbow lap: the ramp runs once around the panel that takes the keys. */
export const RainbowFocusLap: Story = {
  render: (args) => <Target board="Motion" picks={[card("Focus · a rainbow lap")]} issues={[32]} props={boardSpeed(args)} width={900} />,
};

/** Assemble: a screen builds from its parts in layout order (replaces the screen fade at full). */
export const Assemble: Story = { render: (args) => <Target board="Motion" picks={[card("Assemble")]} issues={[42]} props={boardSpeed(args)} width={900} /> };

/** Loading outlines: rows take shape as outlines with a wave, then the real rows read in (replaces the shimmer). */
export const LoadingOutlines: Story = {
  render: (args) => <Target board="Motion" picks={[card("Loading · rows take shape")]} issues={[42]} props={boardSpeed(args)} width={900} />,
};

/** Notice drain: a notice's tinted ground drains as its time runs out. */
export const NoticeDrain: Story = { render: (args) => <Target board="Motion" picks={[card("Notice")]} issues={[41]} props={boardSpeed(args)} width={720} /> };
