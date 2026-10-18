import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { TopBarRow, frameOf } from "../../src/chrome/Chrome";
import type { ChromeView } from "../../src/chrome/types";
import { Compare, fromScene } from "../support/Compare";
import { TopRows } from "../support/frames";
import { type Globals } from "../support/stories";
import type { RatatuiScene } from "../support/ratatui";
import chrome80 from "../../test/fixtures/host_chrome_80x24.json";
import chrome80Cells from "../../test/cells/host_chrome-80x24.json";
import chrome160 from "../../test/fixtures/host_chrome_160x40.json";
import chrome160Cells from "../../test/cells/host_chrome-160x40.json";

type Args = React.ComponentProps<typeof TopBarRow>;
const frame = (c: unknown) => {
  const chrome = c as ChromeView;
  return frameOf(chrome, null, chrome.width, chrome.height);
};

function Draw({ args: _args, globals, c, scene }: { args: Args; globals: Globals; c: unknown; scene: RatatuiScene }) {
  const chrome = c as ChromeView;
  const rows = chrome.sources?.length ? 2 : 1;
  return (
    <Compare
      globals={globals}
      cols={chrome.width}
      rows={rows}
      reference={fromScene(scene, { crop: { x: 0, y: 0, w: chrome.width, h: rows } })}
      ink={<TopRows chrome={chrome} width={chrome.width} height={chrome.height} />}
    />
  );
}

/**
 * The top bar (`ChromeView.top_bar`, Ratatui `shell::top_bar` and `tabs`) and, under it, the sources row
 * (`ChromeView.sources`, `shell::source_tabs`). The bar holds the brand, a tab per part with its key underlined
 * inside the word and an optional attention dot, History and Settings, the sync words and the mode.
 *
 * **Use it** as is: the host fills it; parts add a tab through the plugin contract, never draw their own.
 *
 * **States:** the active tab on the selection shade, a tab needing attention (one warning-colored dot), the bar focused (`keys_at: bar`), a source tab active.
 *
 * **Rules:** keys live inside their word, underlined (R̲epositories, S̲ettings); a key with no letter goes before
 * the word. Tabs show names, not counts. No pill shapes on tabs.
 *
 * The story's `frame` is `frameOf(chrome)` (chrome/Chrome.tsx), from the recorded host chrome.
 */
const meta = {
  title: "Chrome/TopBar",
  component: TopBarRow,
  args: { frame: frame(chrome80) },
  render: (args, { globals }) => <Draw args={args} globals={globals} c={chrome80} scene={chrome80Cells as RatatuiScene} />,
} satisfies Meta<typeof TopBarRow>;

export default meta;
type Story = StoryObj<typeof meta>;

/** The host's chrome at 80 wide: the tabs narrow to their keys and attention dots. */
export const Narrow: Story = {};

/** At 160 wide every tab shows its name. */
export const Wide: Story = {
  args: { frame: frame(chrome160) },
  render: (args, { globals }) => <Draw args={args} globals={globals} c={chrome160} scene={chrome160Cells as RatatuiScene} />,
};
