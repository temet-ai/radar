import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { MenuBox, frameOf } from "../../src/chrome/Chrome";
import type { ChromeView, MenuView } from "../../src/chrome/types";
import { Compare } from "../support/Compare";
import { BodyWithMenu } from "../support/frames";
import { bodyTopOf } from "../support/scene";
import { sceneCells } from "../support/golden";
import { type Globals } from "../support/stories";
import type { GoldenScene } from "../support/golden";
import chrome80 from "../../test/fixtures/host_chrome_80x24.json";
import chrome160 from "../../test/fixtures/host_chrome_160x40.json";
import menu80Cells from "../../test/cells/highlight_menu-80x24.json";
import menu160Cells from "../../test/cells/highlight_menu-160x40.json";
import { confirm } from "../../test/chrome/fixture";

type Args = React.ComponentProps<typeof MenuBox>;
const scene80 = menu80Cells as GoldenScene;
const scene160 = menu160Cells as GoldenScene;

const argsOf = (c: unknown, menu: unknown): Args => {
  const chrome = c as ChromeView;
  return { menu: menu as MenuView, frame: frameOf(chrome, menu as MenuView, chrome.width, chrome.height) };
};

/** `geometry` is the recorded screen the body is measured on (its body rows under the top bar and sources row). */
function Draw({ args, globals, c, screen, geometry }: { args: Args; globals: Globals; c: unknown; screen: unknown; geometry: GoldenScene }) {
  const chrome = c as ChromeView;
  const top = bodyTopOf(sceneCells(geometry));
  const height = geometry.height - top - 1;
  return (
    <Compare
      globals={globals}
      cols={chrome.width}
      rows={height}
      ink={<BodyWithMenu chrome={chrome} screen={screen} menu={args.menu} width={chrome.width} bodyHeight={height} />}
    />
  );
}

/**
 * A menu or a question (`MenuView`, the earlier `shell/menu.rs`), over the body: a framed box with a title, then each
 * choice with its key, label and line; headings (HISTORY) are never chosen; a question (`question: true`) is a
 * dialog whose choices are buttons. It sits where a context click opened it (`at`) or centred.
 *
 * **Use it** through the host: `x` opens the place's actions menu, a write asks a question before it is sent. A part
 * contributes choices; it never draws a menu.
 *
 * **States:** a choice highlighted (the selection background across the row), a heading, a choice with no key, a
 * question with buttons, a typed field, scrolled (`PgUp/PgDn`), too small to use.
 *
 * **Command palette:** Ctrl-P searches commands. `palette` adds the matched/total count, highlights the host's
 * matched label graphemes in bold accent, and puts shortcuts against the right edge. Typing, scrolling, pointer
 * targets, scrim, and shadow use the same menu. Long labels wrap without losing a command's target.
 *
 * **Rules:** the box clears what is under it (no list showing through); keys stand before the label in their own
 * column (`⏎`, `/`, `ctrl-r`); a shadow may dim what is under it but never erase it (#29). No chevrons at the
 * choices, no pill buttons.
 */
const meta = {
  title: "Chrome/MenuBox",
  component: MenuBox,
  args: argsOf(chrome80, scene80.view!.menu),
  render: (args, { globals }) => <Draw args={args} globals={globals} c={chrome80} screen={scene80.view!.screen} geometry={scene80} />,
} satisfies Meta<typeof MenuBox>;

export default meta;
type Story = StoryObj<typeof meta>;

/** The Repositories screen's actions menu (`x`) at 80x24 (`highlight_menu`).
 *
 * **Data gap:** the scene's recorded `view.menu` is not the whole MenuView the host sent: it has no `selected` (Ink
 * highlights the first row) and no keys for "go to its repositories" (`⏎`) and "kept comments" (`ctrl-r`), and the
 * focused pane is not recorded. */
export const Menu: Story = {};

/** The command-search variant at 160x40, using the existing wide-menu story. The view is an explicit illustrative
 * sample, not a recorded host frame; the matched offsets are scalar positions supplied by the host. */
export const Wide: Story = {
  args: argsOf(chrome160, {
    id: "palette", title: "commands", question: false, typed: "re", caret: 2, selected: 0,
    palette: { matched: 3, total: 12, highlights: [[0, 1], [0, 1], [0, 1]] },
    items: [
      { value: "refresh", label: "Refresh", hint: "current view", keys: "ctrl-r", target: "chrome:menu:0" },
      { value: "review", label: "Review Watch records", hint: "selected record", keys: "r", target: "chrome:menu:1" },
      { value: "restore", label: "Restore session", hint: "last session", keys: "", target: "chrome:menu:2" },
    ],
  } satisfies MenuView),
  render: (args, { globals }) => <Draw args={args} globals={globals} c={chrome160} screen={scene160.view!.screen} geometry={scene160} />,
};

/** A question before a write: a note, then two buttons. The data is Ink's hand-made test sample
 * (ink/test/chrome/fixture.ts `confirm`): no recorded view holds a question. */
export const Question: Story = {
  args: argsOf(chrome80, confirm),
  render: (args, { globals }) => <Draw args={args} globals={globals} c={chrome80} screen={scene80.view!.screen} geometry={scene80} />,
};

const rowItem = (i: number, label: string, key: string, hint = "", extra: Partial<MenuView["items"][number]> = {}): MenuView["items"][number] => ({
  value: label, label, hint, key, keys: key, mnemonic_at: null, mnemonic_shift: false, target: `chrome:menu:${i}`, ...extra,
});
const heading = (label: string): MenuView["items"][number] => ({ value: "", label, hint: "", heading: true, shown_only: true });

/** A row menu as the host sends it: every key in the key column (shift keys as the capital), no heading over the first
 * rows, facts about the row in `hint` at the right edge. Hand-made sample (Ink's own test data): no recorded view holds
 * a grouped row menu. */
const rowMenu = (over: Partial<MenuView> = {}): MenuView => ({
  id: "actions",
  title: "watch-rota vimes/night-shift",
  question: false,
  gear: true,
  selected: 1,
  items: [
    rowItem(0, "open in the browser", "o"),
    rowItem(1, "pull", "p", "not behind main"),
    heading("change it"),
    rowItem(2, "rename the branch", "r"),
    rowItem(3, "merge into main", "m", "radar is read only", { muted: true }),
    heading("work on it here"),
    rowItem(4, "open a terminal", "t"),
    rowItem(5, "start a session", "s", "running 4m"),
    heading("ask someone"),
    rowItem(6, "ask Sam Vimes to review", "A"),
    rowItem(7, "ask Carrot about it", "H"),
  ],
  ...over,
});

/** The row menu (`x` or a right click on a row) with its groups, hints at the right and the gear (key `,`) at the
 * right end of the title edge. 80 wide. */
export const RowMenu: Story = {
  args: argsOf(chrome80, rowMenu()),
  render: (args, { globals }) => <Draw args={args} globals={globals} c={chrome80} screen={scene80.view!.screen} geometry={scene80} />,
};

/** The same row menu at 160 wide. */
export const RowMenuWide: Story = {
  args: argsOf(chrome160, rowMenu()),
  render: (args, { globals }) => <Draw args={args} globals={globals} c={chrome160} screen={scene160.view!.screen} geometry={scene160} />,
};

/** The gear is on (`customising`): its cells sit on the selection background. 80 wide. */
export const RowMenuCustomising: Story = {
  args: argsOf(chrome80, rowMenu({ customising: true })),
  render: (args, { globals }) => <Draw args={args} globals={globals} c={chrome80} screen={scene80.view!.screen} geometry={scene80} />,
};

/** The gear is on, 160 wide. */
export const RowMenuCustomisingWide: Story = {
  args: argsOf(chrome160, rowMenu({ customising: true })),
  render: (args, { globals }) => <Draw args={args} globals={globals} c={chrome160} screen={scene160.view!.screen} geometry={scene160} />,
};

const paneMenu = (): MenuView => ({
  id: "pane",
  title: "watch-rota",
  question: false,
  selected: 0,
  items: [
    rowItem(0, "refresh", "r", "synced 2m ago"),
    rowItem(1, "new branch", "n"),
    heading("work on it here"),
    rowItem(2, "start a session", "s", "running 4m"),
    heading("ask someone"),
    rowItem(3, "ask Sam Vimes", "A"),
  ],
});

/** A pane's menu (the actions on the pane, not on a row): no gear in the title edge. 80 wide. */
export const PaneMenu: Story = {
  args: argsOf(chrome80, paneMenu()),
  render: (args, { globals }) => <Draw args={args} globals={globals} c={chrome80} screen={scene80.view!.screen} geometry={scene80} />,
};

/** A pane's menu at 160 wide. */
export const PaneMenuWide: Story = {
  args: argsOf(chrome160, paneMenu()),
  render: (args, { globals }) => <Draw args={args} globals={globals} c={chrome160} screen={scene160.view!.screen} geometry={scene160} />,
};
