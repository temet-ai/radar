import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { PaneFrame } from "../../src/app/Panes";
import { shellRenderNode } from "../../src/app/Body";
import { paintSelection } from "../../src/app/selected";
import type { Selection } from "../../src/input/selection";
import { Compare, regionOf, type Region } from "../support/Compare";
import { paneOf, type Globals } from "../support/stories";
import type { GoldenScene } from "../support/golden";
import selection80Cells from "../../test/cells/highlight_selection-80x24.json";
import selection160Cells from "../../test/cells/highlight_selection-160x40.json";

type Args = React.ComponentProps<typeof PaneFrame>;
type Pt = { x: number; y: number };
/** The `selection` a scene records beside its view (ink/test/cells): the press, the pointer and the pane's
 * inside the drag is bound to, in screen cells. */
type Recorded = { anchor: Pt; head: Pt; bound: Pt & { width: number; height: number } };

const sceneOf = (cells: unknown) => cells as GoldenScene;
const recordedOf = (scene: GoldenScene) => (scene.view as { selection: Recorded }).selection;

/** The scene's pane the drag was made in: the one whose inside is the recorded bound (the log on the item page at
 * 80x24, the detail pane at 160x40). */
const paneTitleOf = (scene: GoldenScene): string => {
  const { bound } = recordedOf(scene);
  const probe = (title: string) => regionOf(scene, { pane: title }).crop;
  const screen = scene.view!.screen as any;
  const titles: string[] = [...(screen.panes ?? []), ...(screen.page?.panes ?? [])].map((p: any) => p.title);
  const found = titles.find((t) => {
    const c = probe(t);
    return c && c.x + 1 === bound.x && c.y + 1 === bound.y && c.w - 2 === bound.width;
  });
  if (!found) throw new Error(`no pane of the ${scene.scene} scene has the selection's bound ${JSON.stringify(bound)}`);
  return found;
};

/** The scene's pane, and the selection in that crop's own cells. */
function pictureOf(scene: GoldenScene): { region: Region; selection: Selection; title: string } {
  const title = paneTitleOf(scene);
  const region = regionOf(scene, { pane: title });
  const crop = region.crop!;
  const r = recordedOf(scene);
  const local = (p: Pt): Pt => ({ x: p.x - crop.x, y: p.y - crop.y });
  return {
    title,
    region,
    selection: {
      area: { id: `pane:${title}`, kind: "text", x: r.bound.x - crop.x, y: r.bound.y - crop.y, w: r.bound.width, h: r.bound.height },
      anchor: local(r.anchor),
      head: local(r.head),
    },
  };
}

const argsOf = (scene: GoldenScene, focused: boolean): Args => {
  const { title, region } = pictureOf(scene);
  const { w, h } = region.crop!;
  const pane = paneOf(scene.view!.screen, title);
  return { pane, width: w, height: h, focused, cx: { width: w, height: h, actions: [], focused }, renderNode: shellRenderNode };
};

/** The pane drawn by Ink with the selection painted over its finished frame, as the renderer does (app/selected.ts),
 * (at the mono tier the selected cells are reversed, with the cells' own colours kept). */
function Draw({ args, globals, scene }: { args: Args; globals: Globals; scene: GoldenScene }) {
  const { selection } = pictureOf(scene);
  return (
    <Compare
      globals={globals}
      cols={args.width}
      rows={args.height}
      paint={(frame) => paintSelection(frame, args.width, selection)}
      ink={<PaneFrame {...args} />}
    />
  );
}

const scene80 = sceneOf(selection80Cells);
const scene160 = sceneOf(selection160Cells);

/**
 * Text selected with the mouse (issue #51; Ratatui `select::paint`, Ink `app/selected.ts` over `input/selection.ts`):
 * gg captures the mouse, so the terminal cannot select text and the renderer does. A drag starts in the text of a pane
 * and selects from the press to the pane's right edge, every row between from edge to edge, then from the left edge to
 * the pointer, as a terminal's selection runs. It belongs to the pane it started in: the border, the title row and the
 * scrollbar are never part of it, and a pointer dragged outside stops at the pane's edge.
 *
 * **Use it** through the host: a drag in a document, rich text, a log or the dock's transcript selects, a double-click
 * selects a word (a path or URL whole), a triple-click the line, shift+arrows extend, `ctrl+c` copies and any other key
 * or the wheel clears it. On rows (a list, tree, table) a double-click still opens, so a drag there does not select. A
 * part never draws a selection; it is a paint over the finished frame, like the hover.
 *
 * **States:** a drag across two rows (the press to the edge, then the left edge to the pointer), and the mono tier.
 *
 * **Rules:** the selected cells take `bg.selection` and keep their text colour and glyph; in the mono tier, where the
 * slot has no colour, they are reversed instead. The cells just outside the pane (its border and scrollbar, the rows
 * above and below the drag, the cells before the press and after the pointer) are never selected. While a drag selects,
 * the status line names the key for the terminal's own selection.
 *
 * **Data gap:** the scene records the drag (`view.selection`) but not which pane has the keys. The border colour
 * says: the log's is the accent (the keys), the detail pane's `accent.muted` (not), so the stories draw them so.
 * `test/app/selection-cells.test.tsx` makes the same drag in the whole App and compares every body cell's selection
 * background with the scene's cells.
 */
const meta = {
  title: "Chrome/TextSelection",
  component: PaneFrame,
  args: argsOf(scene80, true),
  render: (args, { globals }) => <Draw args={args} globals={globals} scene={scene80} />,
} satisfies Meta<typeof PaneFrame>;

export default meta;
type Story = StoryObj<typeof meta>;

/** A drag across two rows of a job's log at 80x24 (`highlight_selection`): from the press to the pane's right edge,
 * then from its left edge to the pointer. */
export const TwoRows: Story = {};

/** The same drag in the rich text of a detail pane at 160x40: the pane has no keys, so its border is the muted one. */
export const Wide: Story = {
  args: argsOf(scene160, false),
  render: (args, { globals }) => <Draw args={args} globals={globals} scene={scene160} />,
};

/** The mono tier, where `bg.selection` has no colour: the selected cells are reversed. */
export const Mono: Story = {
  globals: { tier: "mono" },
  render: (args, { globals }) => <Draw args={args} globals={{ ...globals, tier: "mono" }} scene={scene80} />,
};

/** The mono tier at 160x40. */
export const MonoWide: Story = {
  globals: { tier: "mono" },
  args: argsOf(scene160, false),
  render: (args, { globals }) => <Draw args={args} globals={{ ...globals, tier: "mono" }} scene={scene160} />,
};
