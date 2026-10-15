import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Box } from "ink";
import { Board } from "./Board";
import { TargetLabel } from "./parts";
import { Compare, fromScene } from "../support/Compare";
import { ScreenPanes } from "../../src/app/Panes";
import { shellRenderNode } from "../../src/app/Body";
import type { ChromeView } from "../../src/chrome/types";
import { StatusLine, TopRows, bodyArea } from "../support/frames";
import type { RatatuiScene } from "../support/ratatui";
import repos160 from "../../test/fixtures/gitlab_repos_160x40.json";
import repos160Cells from "../../test/cells/gitlab_repos-160x40.json";
import chrome160 from "../../test/fixtures/host_chrome_160x40.json";

/** The Repositories screen as Ink draws it today: the host chrome's rows, the screen's panes, the status line. */
function TodayScreen() {
  const chrome = chrome160 as ChromeView;
  const body = bodyArea(chrome, 160, 40);
  return (
    <Box flexDirection="column" width={160} height={40}>
      <TopRows chrome={chrome} width={160} height={40} />
      <ScreenPanes screen={repos160} focus="middle" width={160} height={body.height} cx={{ width: 160, height: body.height, actions: [], focused: true }} renderNode={shellRenderNode} />
      <StatusLine chrome={chrome} width={160} height={40} />
    </Box>
  );
}

/**
 * The whole Repositories screen at 160x40 (board 08 · the screen): every sheet at once, with the redesigned detail
 * pane (a head on the heading shade, aligned facts, sections under named rules, actions with keys in their words),
 * pane jump numbers and `/` breadcrumbs (#46), the focus ramp (#31), tabs without counts (#33), keys in words (#44),
 * square tinted chips (#30) and one tree with `⋯ n` folds (#45).
 *
 * `Target` is the canvas; `Today` is the same screen drawn by today's Ink components from the recorded view data, with
 * Ratatui's cells for it beside them.
 */
const meta = {
  title: "Design/Screen",
  component: Board,
  parameters: { layout: "padded" },
  args: { name: "Screen" },
} satisfies Meta<typeof Board>;

export default meta;
type Story = StoryObj<typeof meta>;

/** The target screen, drawn by the canvas's own grid builder. */
export const Target: Story = {
  render: () => (
    <div>
      <TargetLabel issues={[46, 31, 33, 44, 30, 45]} note="#46 the detail pane, jump numbers and / breadcrumbs; the rest as listed above" />
      <Board name="Screen" />
    </div>
  ),
};

/** Today: Ink's Repositories screen at 160x40 from gitlab_repos_160x40 under the recorded host chrome, beside
 * Ratatui's cells for the same surface (the chrome rows differ: the host_chrome fixture has no sources row). */
export const Today: Story = {
  render: (_args, { globals }) => (
    <Compare globals={globals} cols={160} rows={40} reference={fromScene(repos160Cells as RatatuiScene)} ink={<TodayScreen />} />
  ),
};
