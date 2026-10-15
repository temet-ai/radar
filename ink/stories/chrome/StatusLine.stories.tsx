import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { StatusRow, frameOf } from "../../src/chrome/Chrome";
import type { ChromeView } from "../../src/chrome/types";
import { Compare, fromScene } from "../support/Compare";
import { StatusLine } from "../support/frames";
import { type Globals } from "../support/stories";
import type { RatatuiScene } from "../support/ratatui";
import chrome80 from "../../test/fixtures/host_chrome_80x24.json";
import chrome80Cells from "../../test/cells/host_chrome-80x24.json";
import chrome160 from "../../test/fixtures/host_chrome_160x40.json";
import chrome160Cells from "../../test/cells/host_chrome-160x40.json";
import attention80 from "../../test/fixtures/dock/status/attention_80x24.json";
import attention160 from "../../test/fixtures/dock/status/attention_160x40.json";

type Args = React.ComponentProps<typeof StatusRow>;
const frame = (c: unknown) => {
  const chrome = c as ChromeView;
  return frameOf(chrome, null, chrome.width, chrome.height);
};

function Draw({ args: _args, globals, c, scene }: { args: Args; globals: Globals; c: unknown; scene: RatatuiScene }) {
  const chrome = c as ChromeView;
  return (
    <Compare
      globals={globals}
      cols={chrome.width}
      rows={1}
      reference={fromScene(scene, { crop: { x: 0, y: chrome.height - 1, w: chrome.width, h: 1 } })}
      ink={<StatusLine chrome={chrome} width={chrome.width} height={chrome.height} />}
    />
  );
}

/**
 * The status line (`ChromeView.status`, Ratatui `shell::status_line` and `key_hits`): the mode chip with the scope
 * (the place the keys act on), then the key hints for that place or a message, and counts at the right. Hints that do
 * not fit are dropped, the `least_needed` first.
 *
 * **Use it** as is: the host builds the hints from the focused place's bindings; a part names keys, never draws them.
 *
 * **States:** hints, a message in place of the hints (info, warn, bad), counts at the right, a hint pressed by the
 * pointer.
 *
 * **Rules:** a hint's key sits inside its word, underlined (b̲ulk, q̲uit), not "b bulk" (#44); glyph keys (↑↓ ⏎ esc)
 * go before the word. Same key, same action, in the same kind of place across parts. No pill around the scope.
 *
 * The story's `frame` is `frameOf(chrome)` (chrome/Chrome.tsx), from the recorded host chrome.
 */
const meta = {
  title: "Chrome/StatusLine",
  component: StatusRow,
  args: { frame: frame(chrome80) },
  render: (args, { globals }) => <Draw args={args} globals={globals} c={chrome80} scene={chrome80Cells as RatatuiScene} />,
} satisfies Meta<typeof StatusRow>;

export default meta;
type Story = StoryObj<typeof meta>;

/** The host's status line at 80 wide: the scope and the hints that fit. */
export const Hints: Story = {};

/** At 160 wide more hints fit. */
export const Wide: Story = {
  args: { frame: frame(chrome160) },
  render: (args, { globals }) => <Draw args={args} globals={globals} c={chrome160} scene={chrome160Cells as RatatuiScene} />,
};

function DrawAttention({ globals, c }: { globals: Globals; c: unknown }) {
  const { chrome, ratatui } = c as { chrome: ChromeView; ratatui: string };
  return (
    <Compare
      globals={globals}
      cols={chrome.width}
      rows={1}
      ink={<StatusLine chrome={chrome} width={chrome.width} height={chrome.height} />}
      missing={`No cells are recorded for it; Ratatui's row from the same host data is recorded beside the chrome (ink/test/fixtures/dock/status) and ink/test/dock/attention.test.ts compares the two: "${ratatui.trimEnd()}"`}
    />
  );
}

/** An agent session wants you and its tab is out of sight (the dock hidden, or another tab showing): after the scope,
 * the dock key, then a square chip on its state's ground (warning for needs you, error for failed, success for done),
 * the session's name and what it wants, `+n` for the others. It never gives way to the keys. The key or a click opens
 * the dock on that tab (host `StatusLine.attention`, recorded at 80x24: status/attention_80x24). */
export const Attention: Story = {
  args: { frame: frame((attention80 as { chrome: unknown }).chrome) },
  render: (_args, { globals }) => <DrawAttention globals={globals} c={attention80} />,
};

/** The same at 160 wide (status/attention_160x40). */
export const AttentionWide: Story = {
  args: { frame: frame((attention160 as { chrome: unknown }).chrome) },
  render: (_args, { globals }) => <DrawAttention globals={globals} c={attention160} />,
};
