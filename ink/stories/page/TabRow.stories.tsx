import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { TabRow } from "../../src/app/TabRow";
import { bgOf } from "../../src/text/look";
import type { ChromeView } from "../../src/chrome/types";
import { Compare } from "../support/Compare";
import { type Globals } from "../support/stories";
import page80 from "../../test/fixtures/gitlab_repos-page1-overview-changes-threads-pipeline-commits-ticket_80x24.json";
import page160 from "../../test/fixtures/gitlab_repos-page1-overview-changes-threads-pipeline-commits-ticket_160x40.json";
import chrome80 from "../../test/fixtures/host_chrome_80x24.json";
import chrome160 from "../../test/fixtures/host_chrome_160x40.json";

type Args = React.ComponentProps<typeof TabRow>;

/** PageBody's tab data: the tab's name, icon and explicit attention flag. */
const argsOf = (view: any, c: unknown, focused = false): Args => {
  const chrome = c as ChromeView;
  const page = view.page;
  return {
    tabs: page.tabs,
    on: page.tab,
    focused,
    width: chrome.width - 2,
  };
};

/** The labels are laid out again when Ink draws, in the look the toolbar chose (the icon pack decides the glyphs). */
function InLook({ view, args }: { view: any; args: Args }) {
  const fresh = argsOf(view, { width: args.width + 2 }, args.focused);
  return <TabRow {...args} tabs={fresh.tabs} base={bgOf("bg.base")} />;
}

function Draw({ args, globals, view, c }: { args: Args; globals: Globals; view: any; c: unknown }) {
  const chrome = c as ChromeView;
  return (
    <Compare
      globals={globals}
      cols={chrome.width}
      rows={1}
      ink={<InLook view={view} args={args} />}
    />
  );
}

/**
 * A page's tab row (`PageView.tabs`, the earlier `ui::page::tab_row`; Ink app/TabRow.tsx): the tabs' names, the chosen
 * one on the selection shade, a window of tabs with the chosen one in view when they do not all fit.
 *
 * **Use it** inside a page (PageBody draws it); the top bar's part tabs are the TopBar.
 *
 * **States:** a tab chosen, the row focused (the chosen tab on the focused shade), windowed when narrow.
 *
 * **Rules:** tabs show names, not counts. One warning-colored attention dot follows the name when the host
 * marks the tab as needing attention. No pill tabs; square shades only.
 */
const meta = {
  title: "Page/TabRow",
  component: TabRow,
  args: argsOf(page80, chrome80),
  render: (args, { globals }) => <Draw args={args} globals={globals} view={page80} c={chrome80} />,
} satisfies Meta<typeof TabRow>;

export default meta;
type Story = StoryObj<typeof meta>;

/** A merge request's tabs at 80 wide, Overview chosen. */
export const Tabs: Story = {};

/** At 160 wide. */
export const Wide: Story = {
  args: argsOf(page160, chrome160),
  render: (args, { globals }) => <Draw args={args} globals={globals} view={page160} c={chrome160} />,
};

/** The tab row has the keys. */
export const Focused: Story = {
  args: argsOf(page80, chrome80, true),
  render: (args, { globals }) => <Draw args={args} globals={globals} view={page80} c={chrome80} />,
};
