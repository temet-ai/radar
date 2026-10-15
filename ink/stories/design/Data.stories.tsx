import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Board, section } from "./Board";
import { Row, Shipped, Target, Today } from "./parts";
import { TreeNode } from "../../src/nodes/tree";
import { DiffView } from "../../src/text/DiffView";
import { EditorOverlay, editorOverlayHeight } from "../../src/editor/EditorOverlay";
import type { Tree } from "../../src/protocol/generated/Tree";
import type { EditorData } from "../../src/nodes/view";
import { InPane } from "../support/frames";
import { paneOf } from "../support/stories";
import repos80 from "../../test/fixtures/gitlab_repos_80x24.json";
import tickets80 from "../../test/fixtures/jira_tickets_80x24.json";
import unified80 from "../../test/fixtures/diff/diff_unified_80x24.json";
import compose80 from "../../test/fixtures/diff/editor_compose_80x24.json";

const treePane = (view: unknown, title: string, globals: any, caption: string, rows = 16) => {
  const pane = paneOf(view, title);
  return (
    <Today globals={globals} cols={56} rows={rows} caption={caption}>
      <InPane pane={pane} width={56} height={rows} focused body={(ctx) => <TreeNode tree={pane.body as Tree} ctx={ctx} />} />
    </Today>
  );
};

/**
 * Data on screen (board 05 · data): trees and row states, one tree in every part, meters, charts, the pipeline graph,
 * diagrams, the editor with markdown, and diff tints. Where radar draws the thing today, the real Ink component sits
 * beside the canvas. Every proposal is labelled "Target · not built yet" with its issue: one tree #45, charts #39,
 * the pipeline graph #34, mermaid #40, the editor and markdown #37, gradient meters #31; diff tints have no issue of
 * their own yet (#47).
 *
 * Folding is tree lines and `⋯ n`, never a chevron; selection is the background across the row.
 */
const meta = {
  title: "Design/Data",
  component: Board,
  parameters: { layout: "padded" },
  args: { name: "Data" },
} satisfies Meta<typeof Board>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Trees and the five row states, each shown by its ground alone, beside today's tree. */
export const RowStates: Story = {
  render: (_args, { globals }) => (
    <Row>
      {treePane(repos80, "Everything", globals, "TreeNode, GitLab Repositories (gitlab_repos_80x24)")}
      <Shipped board="Data" picks={[section("Trees & row states")]} what="the canvas's drawing of the tree and its row states" />
    </Row>
  ),
};

/** One tree in every part: GitLab, Jira, Slack, Git all nest with the same tree, the same keys and the same marks. */
export const OneTreeEveryPart: Story = {
  render: (_args, { globals }) => (
    <Row>
      <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
        {treePane(repos80, "Everything", globals, "TreeNode in GitLab (gitlab_repos_80x24)", 14)}
        {treePane(tickets80, "My tickets", globals, "TreeNode in Jira (jira_tickets_80x24)", 14)}
      </div>
      <Target board="Data" picks={[section("One tree, every part")]} issues={[45]} />
    </Row>
  ),
};

/** Meters: the stage bar and a ratio, drawn to the eighth of a cell (the panel marked new). Today's stage bar is
 * Nodes/MeterLines (no recorded stages yet). */
export const Meters: Story = {
  render: () => <Target board="Data" picks={[section("Meters")]} issues={[31]} note="the panel marked new; the stage bar itself is shipped" />,
};

/** Charts: bars, braille lines and heat for what changes over time, numbers kept beside them in text. */
export const Charts: Story = {
  render: () => <Target board="Data" picks={[section("Charts")]} issues={[39]} />,
};

/** A GitLab pipeline as a graph of jobs: stages are columns, jobs are boxes bordered in their state, needs are lines
 * (laid out with dagre, drawn in cells). */
export const PipelineGraph: Story = {
  render: () => <Target board="Data" picks={[section("Pipelines as graphs")]} issues={[34]} />,
};

/** A mermaid block drawn as a diagram in cells, not shown as source. */
export const Mermaid: Story = {
  render: () => <Target board="Data" picks={[section("Diagrams")]} issues={[40]} />,
};

/** The editor workspace: the markdown source on the left, the live preview on the right. Today: the overlay. */
export const EditorAndMarkdown: Story = {
  render: (_args, { globals }) => {
    const data = (compose80 as any).screen.overlay.body.data as EditorData;
    const h = editorOverlayHeight(data.lines.length, 21);
    return (
      <Row>
        <Today globals={globals} cols={80} rows={h} caption="EditorOverlay (diff/editor_compose_80x24)">
          <EditorOverlay data={data} width={80} areaHeight={21} />
        </Today>
        <Target board="Data" picks={[section("The editor & markdown")]} issues={[37]} />
      </Row>
    );
  },
};

/** Diff: today coloured text on a tint; the target adds a brighter tint under the words that changed. */
export const DiffTints: Story = {
  render: (_args, { globals }) => {
    const pane = ((unified80 as any).screen.page.panes as any[]).find((p) => p.body?.node === "diff");
    const b = pane.body;
    return (
      <Row>
        <Today globals={globals} cols={80} rows={14} caption="DiffView (diff/diff_unified_80x24)">
          <InPane
            pane={pane}
            width={80}
            height={14}
            focused
            body={(ctx) => <DiffView lines={b.lines} under={b.under} selected={b.selected} top={b.top} split={b.split} painted={b.painted} empty={b.empty} width={ctx.width} height={ctx.height} focused />}
          />
        </Today>
        <Target board="Data" picks={[section("Diff")]} issues={[47]} note="the panel marked new; no issue of its own yet" />
      </Row>
    );
  },
};
