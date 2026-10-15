import type { Meta, StoryObj } from "@storybook/react-vite";
import { Board } from "./Board";

/**
 * The design canvas, board by board, as it was agreed (saved from the canvas on 13 October 2026 into
 * `stories/design/boards`). Each board is drawn by its own markup and its own grid builders.
 *
 * **Most of what the boards show is target, not shipped.** A board marks each part `shipped` or `new in ink`;
 * the Design stories split them up, put today's real Ink component beside each target, and label every target
 * "Target · not built yet · issue #N".
 */
const meta = {
  title: "Design/Canvas",
  component: Board,
  parameters: { layout: "fullscreen" },
  argTypes: {
    name: { control: "select", options: ["Main", "Colour", "Type", "Icons", "Components", "Data", "Surfaces", "Motion", "Screen", "Research"] },
  },
  args: { name: "Main" },
} satisfies Meta<typeof Board>;

export default meta;
type Story = StoryObj<typeof meta>;

/** 00 · radar in Ink: what Ink opens up, and the five rules. Pick another board with the `name` control. */
export const Main: Story = {};
