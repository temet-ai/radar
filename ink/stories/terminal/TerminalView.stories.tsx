import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { TerminalView } from "../../src/terminal/TerminalView";
import type { Frame, TerminalState } from "../../src/terminal/frames";
import type { TerminalLink } from "../../src/terminal/link";
import { Compare } from "../support/Compare";
import type { Globals } from "../support/stories";
import less80 from "../../test/terminal/recorded/less-80x24.json";
import less160 from "../../test/terminal/recorded/less-160x40.json";

interface Recording {
  columns: number;
  rows: number;
  state: TerminalState;
  base64: string;
}

const bytesOf = (r: Recording) => Uint8Array.from(atob(r.base64), (c) => c.charCodeAt(0));

/** Replays a recording as the host does on attach: the state, then the ring; `end` then says how the program ended. */
const replay = (r: Recording, end?: Partial<TerminalState>): TerminalLink => ({
  onFrame(fn: (f: Frame) => void) {
    fn({ kind: "state", state: r.state });
    fn({ kind: "output", bytes: bytesOf(r) });
    if (end) fn({ kind: "state", state: { ...r.state, ...end } });
  },
  onClose() {},
  input() {},
  resize() {},
  restart() {},
  kill() {},
  close() {},
});

type Args = { rec: Recording; end?: Partial<TerminalState>; focused: boolean };

function Draw({ rec, end, focused, globals }: Args & { globals: Globals }) {
  return (
    <Compare
      globals={globals}
      cols={rec.columns}
      rows={rec.rows}      ink={<TerminalView open={() => replay(rec, end)} openKey={`${rec.columns}-${String(!!end)}`} columns={rec.columns} rows={rec.rows} focused={focused} />}
    />
  );
}

/**
 * A program's own screen in a dock tab (`ink/src/terminal`, host side `crates/radar-sessions-pty`). The host runs the
 * program in a PTY and keeps its output; Ink parses the bytes (xterm's engine) and draws the cells. The pictures are
 * recordings of `less -R` made by the host's PTY, replayed as the host replays them to a renderer that attaches.
 *
 * **Use it** in a dock tab whose `terminal` is set; the dock gives it the tab's content area and the keys while focused.
 *
 * **States:** following a running program; the program ended (how it ended, `r̲estart`); at 80x24 and 160x40.
 *
 * **Rules:** the program's colours are its own; the surface adds nothing but the ended row, whose key sits inside its
 * word. Ctrl+] gives the keys back. Ink only: the host has no terminal tab in its own picture.
 */
const meta = {
  title: "Dock/TerminalView",
  component: TerminalView,
  args: { open: () => replay(less80 as unknown as Recording), openKey: "story", columns: 80, rows: 24, focused: true },
  render: (_args, { globals }) => <Draw rec={less80 as unknown as Recording} focused={true} globals={globals} />,
} satisfies Meta<typeof TerminalView>;

export default meta;
type Story = StoryObj<typeof meta>;

/** `less -R` at 80x24, following the program. */
export const Running: Story = {};

/** The same program at 160x40, recorded at that size. */
export const Wide: Story = {
  render: (_args, { globals }) => <Draw rec={less160 as unknown as Recording} focused={true} globals={globals} />,
};

/** The program ended: its last lines stay, the row below says how and offers r̲estart. */
export const Exited: Story = {
  render: (_args, { globals }) => <Draw rec={less80 as unknown as Recording} end={{ state: "exited", code: 2 }} focused={true} globals={globals} />,
};

/** The program could not start: nothing to show but why. */
export const Failed: Story = {
  render: (_args, { globals }) => (
    <Draw rec={{ ...(less80 as unknown as Recording), base64: "" }} end={{ state: "failed", message: "could not start nope: No such file or directory" }} focused={true} globals={globals} />
  ),
};
