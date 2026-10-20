import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Box } from "ink";
import { Board, childOf, section } from "./Board";
import { Row, Shipped, Target, Today } from "./parts";
import { TextLine } from "../../src/text/TextLine";
import type { Seg } from "../../src/nodes/view";
import type { ChromeView, MenuView } from "../../src/chrome/types";
import type { Tree } from "../../src/protocol/generated/Tree";
import { TreeNode } from "../../src/nodes/tree";
import { TabRow } from "../../src/app/TabRow";
import { bgOf } from "../../src/text/look";
import { BodyWithMenu, InPane, StatusLine, TopRows } from "../support/frames";
import { paneOf } from "../support/stories";
import type { GoldenScene } from "../support/golden";
import tickets160 from "../../test/fixtures/jira_tickets_160x40.json";
import chrome80 from "../../test/fixtures/host_chrome_80x24.json";
import chrome160 from "../../test/fixtures/host_chrome_160x40.json";
import page80 from "../../test/fixtures/gitlab_repos-page1-overview-changes-threads-pipeline-commits-ticket_80x24.json";
import repos80 from "../../test/fixtures/gitlab_repos_80x24.json";
import typing80Cells from "../../test/cells/highlight_typing-filter-80x24.json";
import menu80Cells from "../../test/cells/highlight_menu-80x24.json";
import { confirm } from "../../test/chrome/fixture";

/** The canvas's target for a component: the third column of its row on board 04 ("in ink"), with the row's notes. */
const targetOf = (heading: string) => [childOf(section(heading), 0), childOf(section(heading), 2)];

/** Every chip seg in the recorded Jira tickets view. */
function recordedChips(): Seg[] {
  const out: Seg[] = [];
  const walk = (x: any) => {
    if (!x || typeof x !== "object") return;
    if (x.chip === true) out.push(x);
    for (const v of Array.isArray(x) ? x : Object.values(x)) walk(v);
  };
  walk(tickets160);
  return out;
}

/** The board's state chips, one per tone, as a part sends them (view `Seg`s with `chip`). */
const stateChips: Seg[] = [
  { chip: true, icon: "run_passed", ink: "good", text: "passed" },
  { text: " " },
  { chip: true, ink: "warn", text: "needs review" },
  { text: " " },
  { chip: true, icon: "run_failed", ink: "bad", text: "failed" },
  { text: " " },
  { chip: true, icon: "run_running", ink: "info", text: "running" },
  { text: " " },
  { chip: true, icon: "merged", ink: "accent", text: "merged" },
  { text: " " },
  { chip: true, ink: "muted", text: "draft" },
];

/**
 * The kit's components, today and next (board 04 · components). Each story puts **today's real Ink component**, drawn
 * from recorded view data, beside the **canvas's target** for it. Every target is labelled "Target · not built yet"
 * with the issue that builds it: chips #30, keys #44, tabs #33, people #38, buttons #44; fields have no issue of their
 * own (#47 orders them).
 *
 * **Rules both sides keep:** square shapes (a ground, a space each side, no round caps), no left stripes, keys
 * underlined inside their word, no counts on tabs.
 */
const meta = {
  title: "Design/Components",
  component: Board,
  parameters: { layout: "padded" },
  args: { name: "Components" },
} satisfies Meta<typeof Board>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Chips: a tone's text on its tone's tint, 16% over the overlay shade (#30; Jira's recorded chip), as the target
 * draws them. */
export const Chips: Story = {
  render: (_args, { globals }) => (
    <Row>
      <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
        <Today globals={globals} cols={40} rows={1} caption="TextLine, the chip on Jira ABC-12 (jira_tickets_160x40)">
          <Box width={40}>
            <TextLine text={recordedChips()} />
          </Box>
        </Today>
        <Today globals={globals} cols={64} rows={1} caption="TextLine, one chip per tone (the board's states)">
          <Box width={64}>
            <TextLine text={stateChips} />
          </Box>
        </Today>
      </div>
      <Shipped board="Components" picks={targetOf("Chips")} what="the canvas's tinted chips (#30)" width={760} />
    </Row>
  ),
};

/** Keys: today the status line writes the key, then the word; the target underlines the key inside its word. */
export const Keys: Story = {
  render: (_args, { globals }) => (
    <Row>
      <Today globals={globals} cols={80} rows={1} caption="The status line (host_chrome_80x24)">
        <StatusLine chrome={chrome80 as ChromeView} width={80} height={24} />
      </Today>
      <Target board="Components" picks={targetOf("Keys")} issues={[44]} width={760} />
    </Row>
  ),
};

/** Tabs: names with one warning-colored dot when the host reports attention, in the top bar and item pages. */
export const Tabs: Story = {
  render: (_args, { globals }) => (
    <Row>
      <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
        <Today globals={globals} cols={160} rows={1} caption="The top bar (host_chrome_160x40)">
          <TopRows chrome={chrome160 as ChromeView} width={160} height={40} />
        </Today>
        <Today globals={globals} cols={80} rows={1} caption="A merge request page's tab row (gitlab_repos-page1-overview…_80x24)">
          <PageTabs />
        </Today>
      </div>
      <Target board="Components" picks={targetOf("Tabs")} issues={[33]} width={760} />
    </Row>
  ),
};

function PageTabs() {
  const page = (page80 as any).page;
  return <TabRow tabs={page.tabs} on={page.tab} focused={false} width={78} base={bgOf("bg.base")} />;
}

/** Fields: today the `/` filter at the top of a pane (highlight_typing-filter); the target says what it acts on and
 * how many match. */
export const Fields: Story = {
  render: (_args, { globals }) => {
    const pane = paneOf((typing80Cells as GoldenScene).view!.screen, "Everything");
    return (
      <Row>
        <Today globals={globals} cols={56} rows={8} caption="The field bar in its pane (highlight_typing-filter-80x24)">
          <InPane pane={pane} width={56} height={8} focused body={(ctx) => <TreeNode tree={pane.body as Tree} ctx={ctx} />} />
        </Today>
        <Target board="Components" picks={targetOf("Fields")} issues={[47]} note="no issue of its own yet" width={760} />
      </Row>
    );
  },
};

/** People: today a person is a word (`you`, `group_1_bot`); the target adds the person mark and avatars. */
export const People: Story = {
  render: (_args, { globals }) => {
    const pane = paneOf(repos80, "Everything");
    return (
      <Row>
        <Today globals={globals} cols={56} rows={14} caption="Tree rows with their people (gitlab_repos_80x24)">
          <InPane pane={pane} width={56} height={14} focused body={(ctx) => <TreeNode tree={pane.body as Tree} ctx={ctx} />} />
        </Today>
        <Target board="Components" picks={targetOf("People")} issues={[38]} note="avatars come with pictures" width={760} />
      </Row>
    );
  },
};

/** Buttons: today a question's buttons (Ink's hand-made confirm sample over the Repositories screen; no recorded
 * question exists); the target underlines each button's key in its label, one primary per box. */
export const Buttons: Story = {
  render: (_args, { globals }) => {
    const scene = menu80Cells as GoldenScene;
    return (
      <Row>
        <Today globals={globals} cols={80} rows={21} caption="MenuBox with a question (ink/test/chrome/fixture.ts confirm)">
          <BodyWithMenu chrome={chrome80 as ChromeView} screen={scene.view!.screen} menu={confirm as unknown as MenuView} width={80} bodyHeight={21} />
        </Today>
        <Target board="Components" picks={targetOf("Buttons")} issues={[44]} width={760} />
      </Row>
    );
  },
};
