import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { EditorOverlay, editorOverlayHeight } from "../../src/editor/EditorOverlay";
import type { EditorData } from "../../src/nodes/view";
import type { ChromeView } from "../../src/chrome/types";
import { Compare, fromText } from "../support/Compare";
import { bodyArea } from "../support/frames";
import { type Globals } from "../support/stories";
import compose80 from "../../test/fixtures/diff/editor_compose_80x24.json";
import compose80Text from "../../test/fixtures/diff/editor_compose_80x24.txt?raw";
import compose160 from "../../test/fixtures/diff/editor_compose_160x40.json";
import compose160Text from "../../test/fixtures/diff/editor_compose_160x40.txt?raw";

type Args = React.ComponentProps<typeof EditorOverlay>;

const argsOf = (view: any): Args => {
  const chrome = view.chrome as ChromeView;
  return { data: view.screen.overlay.body.data as EditorData, width: chrome.width, areaHeight: bodyArea(chrome, chrome.width, chrome.height).height };
};

function Draw({ args, globals, text, file }: { args: Args; globals: Globals; text: string; file: string }) {
  const h = editorOverlayHeight(args.data.lines.length, args.areaHeight);
  return (
    <Compare
      globals={globals}
      cols={args.width}
      rows={h}
      reference={fromText(text, `ink/test/fixtures/diff/${file}`, { pane: args.data.title })}
      ink={<EditorOverlay {...args} />}
    />
  );
}

/**
 * The editor overlay (`Node::Extension` with the `editor` capability in `ScreenView.overlay`; Ratatui
 * `view_tui::draw_overlay` and `field`; Ink editor/EditorOverlay.tsx): writing a comment or a description. A focused
 * panel over the foot of the body, titled with what is written, the last lines that fit, then the line being typed
 * as a field with the caret.
 *
 * **Use it** through the host's Composer: `⏎` a new line, `ctrl-s` review and send, `esc` stop (asking first when
 * there is text).
 *
 * **States:** one line, several lines (the panel grows up to two thirds of the body), a selection (Ink draws it;
 * Ratatui ignores `selection`), wide characters in the line.
 *
 * **Rules:** the caret is a reversed cell; the title's icon comes from the pack (`Icon::Comment`), not a typed `✎`
 * (the recorded title still carries one, docs/components.md section 4). The host owns the text; the overlay only draws
 * it.
 *
 * The references are Ratatui's screens as text (no colours).
 */
const meta = {
  title: "Editor/EditorOverlay",
  component: EditorOverlay,
  args: argsOf(compose80),
  render: (args, { globals }) => <Draw args={args} globals={globals} text={compose80Text} file={"editor_compose_80x24.txt"} />,
} satisfies Meta<typeof EditorOverlay>;

export default meta;
type Story = StoryObj<typeof meta>;

/** A comment on !263 being written at 80x24: two lines kept, the third being typed with wide characters. */
export const Compose: Story = {};

/** The same at 160x40. */
export const Wide: Story = {
  args: argsOf(compose160),
  render: (args, { globals }) => <Draw args={args} globals={globals} text={compose160Text} file={"editor_compose_160x40.txt"} />,
};
