import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Box } from "ink";
import { MediaBlock } from "../../src/media/MediaBlock";
import { NodeView } from "../../src/nodes/index";
import type { Media } from "../../src/protocol/generated/Media";
import { Compare } from "../support/Compare";
import { PaneFigure, paneOf, type Globals } from "../support/stories";
import media80 from "../../test/fixtures/media/jira_ticket-media_80x24.json";

type Args = React.ComponentProps<typeof MediaBlock>;
const pane = paneOf(media80, "overview");
const docAt = pane.body.blocks.findIndex((b: any) => b.block === "doc" && b.doc.some((d: any) => d.t === "media"));
const doc: any[] = pane.body.blocks[docAt].doc;
const firstMedia = doc.findIndex((d) => d.t === "media");
const allMedia = doc.filter((d) => d.t === "media") as Media[];
const MISSING =
  "ink/test/fixtures/media is not drawn by tests/ratatui_cells.rs (no media scene in ink/test/cells). Adding the jira_ticket-media surface there would give Ratatui's fallback lines.";

/**
 * A picture or a file in a document (`RichBlock` `t: "media"`, a `Media`; Ratatui `ui::rich::media` and `ui::image`;
 * Ink media/MediaBlock.tsx). Where no picture is placed it is one fallback line: an icon, the name, the size in pixels
 * when known. Pictures in the terminal (kitty, sixel) are #38; the Ink kitty drawer is not wired yet.
 *
 * **Use it** inside rich text; parts send media blocks in a document, never draw them.
 *
 * **States:** an image with a size, an image with an address, a file; with icons on or off.
 *
 * **Rules:** the fallback says what it is in words, so it reads without colour or pictures; the icon comes from the
 * pack. The story's `media` replaces the first media block of the recorded ticket description.
 */
const meta = {
  title: "Media/MediaBlock",
  component: MediaBlock,
  args: { media: allMedia[0]! },
  render: function Render(args: Args, { globals }: { globals: Globals }) {
    const blocks = pane.body.blocks.map((b: any, j: number) => (j === docAt ? { ...b, doc: doc.map((d, k) => (k === firstMedia ? { t: "media", ...args.media } : d)) } : b));
    return (
      <PaneFigure
        globals={globals}
        pane={pane}
        focused
        size={{ w: 80, h: 22 }}
        missing={MISSING}
        body={(ctx) => <NodeView node={{ ...pane.body, blocks }} ctx={ctx} />}
      />
    );
  },
} satisfies Meta<typeof MediaBlock>;

export default meta;
type Story = StoryObj<typeof meta>;

/** The fallback lines in a Jira ticket's description (dashboard.png at 640x480, a design image, two files). */
export const InDescription: Story = {};

/** Each media block of that description on its own line, as MediaBlock draws it. */
export const EachKind: Story = {
  render: function Render(_args, { globals }) {
    return (
      <Compare
        globals={globals}
        cols={60}
        rows={allMedia.length}
        missing={MISSING}
        ink={
          <Box flexDirection="column" width={60}>
            {allMedia.map((m, i) => (
              <MediaBlock key={i} media={m} />
            ))}
          </Box>
        }
      />
    );
  },
};
