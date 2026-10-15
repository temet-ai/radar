import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { PageBody } from "../../src/app/Panes";
import { shellRenderNode } from "../../src/app/Body";
import { Compare, fromScene } from "../support/Compare";
import { type Globals } from "../support/stories";
import { sceneCells, type RatatuiScene } from "../support/ratatui";
import { bodyTopOf } from "../support/scene";
import page80 from "../../test/fixtures/gitlab_repos-page1-overview-changes-threads-pipeline-commits-ticket_80x24.json";
import page80Cells from "../../test/cells/gitlab_repos-page1-overview-changes-threads-pipeline-commits-ticket-80x24.json";
import page160 from "../../test/fixtures/gitlab_repos-page1-overview-changes-threads-pipeline-commits-ticket_160x40.json";
import page160Cells from "../../test/cells/gitlab_repos-page1-overview-changes-threads-pipeline-commits-ticket-160x40.json";

type Args = React.ComponentProps<typeof PageBody>;

/** The page's body on Ratatui's screen: under the top bar and the sources row, above the status line. */
const pageBodyOf = (scene: RatatuiScene) => {
  const top = bodyTopOf(sceneCells(scene));
  return { top, width: scene.width, height: scene.height - top - 1 };
};

const argsOf = (view: any, scene: RatatuiScene, tabsFocused = false): Args => {
  const body = pageBodyOf(scene);
  return {
    page: view.page,
    focus: "middle",
    width: body.width,
    height: body.height,
    cx: { width: body.width, height: body.height, actions: [], focused: true, tabsFocused },
    renderNode: shellRenderNode,
  };
};

function Draw({ args, globals, scene, reference, missing }: { args: Args; globals: Globals; scene: RatatuiScene; reference: boolean; missing?: string }) {
  const body = pageBodyOf(scene);
  return (
    <Compare
      globals={globals}
      cols={args.width}
      rows={args.height}
      reference={reference ? fromScene(scene, { crop: { x: 0, y: body.top, w: body.width, h: body.height } }) : undefined}
      missing={missing}
      ink={<PageBody {...args} />}
    />
  );
}

const s80 = page80Cells as RatatuiScene;
const s160 = page160Cells as RatatuiScene;

/**
 * An item's page (`PageView`, Ratatui `ui::page::PageFrame` via `render_page`; Ink app/Panes.tsx `PageBody`,
 * nodes/pagetitle.tsx, app/TabRow.tsx): the breadcrumbs, the title with its details at the right, the line under it,
 * the tab row, then the tab's panes in the page's layout (`PageLayout`: a pane, a split, or responsive by width).
 *
 * **Use it** for any one item (a merge request, a ticket, a job): the host builds it from the part's `PageView`.
 *
 * **States:** a tab chosen, the tab row focused (`keys_at: page_tabs`), the tab window when the tabs do not fit, a
 * notice under the tabs, responsive panes (side by side when wide, stacked when narrow).
 *
 * **Rules:** breadcrumbs join with a muted ` / ` and the last is bold (both renderers still draw ` › `, #46). Tabs
 * show names, not counts (`Changes · 1 file` and `Threads · 1 open` are still sent, #33). The first crumb is the part
 * name. Keys: `[` `]` or `← →` between tabs, `esc` back.
 */
const meta = {
  title: "Page/PageBody",
  component: PageBody,
  args: argsOf(page80, s80),
  render: (args, { globals }) => <Draw args={args} globals={globals} scene={s80} reference={true} />,
} satisfies Meta<typeof PageBody>;

export default meta;
type Story = StoryObj<typeof meta>;

/** A merge request's Overview tab at 80x24: "can it merge?" stacked above the overview (responsive layout). */
export const Overview: Story = {};

/** The same page at 160x40: "can it merge?" beside the overview. */
export const Wide: Story = {
  args: argsOf(page160, s160),
  render: (args, { globals }) => <Draw args={args} globals={globals} scene={s160} reference={true} />,
};

/** The tab row has the keys (`keys_at: page_tabs`): the chosen tab on the focused shade. */
export const TabsFocused: Story = {
  args: argsOf(page80, s80, true),
  render: (args, { globals }) => <Draw args={args} globals={globals} scene={s80} reference={false} missing={"tests/ratatui_cells.rs records no page with its tab row focused."} />,
};
