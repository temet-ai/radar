import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { TabRow } from "../../src/app/TabRow";
import { bgOf } from "../../src/text/look";
import type { ChromeView } from "../../src/chrome/types";
import { Compare, fromScene } from "../support/Compare";
import { bodyTopOf } from "../support/scene";
import { sceneCells } from "../support/ratatui";
import { type Globals } from "../support/stories";
import type { RatatuiScene } from "../support/ratatui";
import page80 from "../../test/fixtures/gitlab_repos-page1-overview-changes-threads-pipeline-commits-ticket_80x24.json";
import page80Cells from "../../test/cells/gitlab_repos-page1-overview-changes-threads-pipeline-commits-ticket-80x24.json";
import page160 from "../../test/fixtures/gitlab_repos-page1-overview-changes-threads-pipeline-commits-ticket_160x40.json";
import page160Cells from "../../test/cells/gitlab_repos-page1-overview-changes-threads-pipeline-commits-ticket-160x40.json";
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

/** The tab row's line on the page: after the crumbs, the title and the line under it (PageBody's head). */
const tabLine = (view: any, scene: RatatuiScene) => bodyTopOf(sceneCells(scene)) + 2 + (view.page.under ? 1 : 0);

function Draw({ args, globals, view, c, scene, missing }: { args: Args; globals: Globals; view: any; c: unknown; scene: RatatuiScene | undefined; missing?: string }) {
  const chrome = c as ChromeView;
  return (
    <Compare
      globals={globals}
      cols={chrome.width}
      rows={1}
      reference={scene ? fromScene(scene, { crop: { x: 0, y: tabLine(view, scene), w: chrome.width, h: 1 } }) : undefined}
      missing={missing}
      ink={<InLook view={view} args={args} />}
    />
  );
}

/**
 * A page's tab row (`PageView.tabs`, Ratatui `ui::page::tab_row`; Ink app/TabRow.tsx): the tabs' names, the chosen
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
  render: (args, { globals }) => <Draw args={args} globals={globals} view={page80} c={chrome80} scene={page80Cells as RatatuiScene} />,
} satisfies Meta<typeof TabRow>;

export default meta;
type Story = StoryObj<typeof meta>;

/** A merge request's tabs at 80 wide, Overview chosen. */
export const Tabs: Story = {};

/** At 160 wide. */
export const Wide: Story = {
  args: argsOf(page160, chrome160),
  render: (args, { globals }) => <Draw args={args} globals={globals} view={page160} c={chrome160} scene={page160Cells as RatatuiScene} />,
};

/** The tab row has the keys. */
export const Focused: Story = {
  args: argsOf(page80, chrome80, true),
  render: (args, { globals }) => <Draw args={args} globals={globals} view={page80} c={chrome80} scene={undefined} missing={"tests/ratatui_cells.rs records no page with its tab row focused."} />,
};
