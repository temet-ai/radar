import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Board, section } from "./Board";
import { Row, Target, Today } from "./parts";
import { TreeNode } from "../../src/nodes/tree";
import { StateView } from "../../src/text/StateView";
import { TableNode } from "../../src/nodes/lists";
import type { Tree } from "../../src/protocol/generated/Tree";
import type { ChromeView } from "../../src/chrome/types";
import { BodyWithMenu, InPane } from "../support/frames";
import { paneOf } from "../support/stories";
import type { RatatuiScene } from "../support/ratatui";
import { NoticeSurfaces } from "./NoticeReplay";
import repos80 from "../../test/fixtures/gitlab_repos_80x24.json";
import menu80Cells from "../../test/cells/highlight_menu-80x24.json";
import chrome80 from "../../test/fixtures/host_chrome_80x24.json";
import start80 from "../../test/fixtures/reference/mix__git_start_80x24.json";
import reposPage80 from "../../test/fixtures/gitlab_repos-page1-repos_80x24.json";

/**
 * Surfaces and layers (board 06): the focused panel, overlays over a scrim, notices, and the panel states. Today's
 * real component sits beside each canvas proposal. Targets: the focus ramp #31, the scrim and shadow #29 with the
 * command palette #35; notices #41 and the panel states are shipped and redrawn on the canvas with loading outlines
 * (#42).
 *
 * A shadow dims what is under it and never erases it; no left stripes; no pill shapes.
 */
const meta = {
  title: "Design/Surfaces",
  component: Board,
  parameters: { layout: "padded" },
  args: { name: "Surfaces" },
} satisfies Meta<typeof Board>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Focus: today the focused panel has the accent border, the others the quiet one; the target runs the glow ramp
 * around the focused edge. */
export const Focus: Story = {
  render: (_args, { globals }) => {
    const pane = paneOf(repos80, "Everything");
    return (
      <Row>
        <Today globals={globals} cols={56} rows={14} caption="A focused pane (gitlab_repos_80x24)">
          <InPane pane={pane} width={56} height={14} focused body={(ctx) => <TreeNode tree={pane.body as Tree} ctx={ctx} />} />
        </Today>
        <Target board="Surfaces" picks={[section("Focus")]} issues={[31]} />
      </Row>
    );
  },
};

/** Overlays: today a menu over the body (highlight_menu), the body dimmed behind it and a one-cell shadow under its
 * right and bottom edges (#29); the target is a command palette over that scrim. */
export const PaletteOverScrim: Story = {
  render: (_args, { globals }) => {
    const scene = menu80Cells as RatatuiScene;
    return (
      <Row>
        <Today globals={globals} cols={80} rows={21} caption="MenuBox over the Repositories screen (highlight_menu-80x24)">
          <BodyWithMenu chrome={chrome80 as ChromeView} screen={scene.view!.screen} menu={scene.view!.menu as any} width={80} bodyHeight={21} />
        </Today>
        <Target board="Surfaces" picks={[section("Overlays")]} issues={[29, 35]} note="#29 the scrim and shadow (shipped), #35 the palette" />
      </Row>
    );
  },
};

/** Production stacked notices and the separate session-only history surface. */
export const Notices: Story = { render: (_args, { globals }) => <NoticeSurfaces globals={globals} /> };

/** Panel states: loading, empty and broken, each said in the panel's own space. Today: the recorded empty state and an
 * empty table. */
export const PanelStates: Story = {
  render: (_args, { globals }) => {
    const empty = paneOf((start80 as any).screen, "all");
    const table = paneOf(reposPage80, "repos");
    return (
      <Row>
        <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
          <Today globals={globals} cols={80} rows={6} caption="StateView, empty (mix__git_start_80x24)">
            <InPane pane={empty} width={80} height={6} focused body={(ctx) => <StateView state={empty.body.state} text={empty.body.text} width={ctx.width} />} />
          </Today>
          <Today globals={globals} cols={80} rows={6} caption="TableNode, empty (gitlab_repos-page1-repos_80x24)">
            <InPane pane={table} width={80} height={6} focused body={(ctx) => <TableNode columns={table.body.columns} rows={table.body.rows} selected={table.body.selected} empty={table.body.empty} ctx={ctx} />} />
          </Today>
        </div>
        <Target board="Surfaces" picks={[section("Panel states")]} issues={[42]} note="shipped states, redrawn with loading outlines" />
      </Row>
    );
  },
};
