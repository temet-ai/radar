import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Dock } from "../../src/dock/Dock";
import { Compare } from "../support/Compare";
import type { Globals } from "../support/stories";
import type { DockView } from "../../src/dock/model";
import empty80 from "../../test/fixtures/dock/empty_80x24.json";
import empty160 from "../../test/fixtures/dock/empty_160x40.json";
import streaming160 from "../../test/fixtures/dock/streaming_160x40.json";
import toolFolded160 from "../../test/fixtures/dock/tool-folded_160x40.json";
import toolOpen160 from "../../test/fixtures/dock/tool-open_160x40.json";
import approval160 from "../../test/fixtures/dock/approval_160x40.json";
import error160 from "../../test/fixtures/dock/error_160x40.json";
import stopped160 from "../../test/fixtures/dock/stopped_160x40.json";
import pty160 from "../../test/fixtures/dock/pty_160x40.json";
import tabs160 from "../../test/fixtures/dock/tabs_160x40.json";
import approval80 from "../../test/fixtures/dock/approval_80x24.json";
import question160 from "../../test/fixtures/dock/question_160x40.json";
import attention160 from "../../test/fixtures/dock/attention_160x40.json";
import full160 from "../../test/fixtures/dock/full_160x40.json";
import restored160 from "../../test/fixtures/dock/restored_160x40.json";
import detached160 from "../../test/fixtures/dock/detached_160x40.json";
import markdown160 from "../../test/fixtures/dock/markdown_160x40.json";
import markdownStreaming160 from "../../test/fixtures/dock/markdown-streaming_160x40.json";
import tools160 from "../../test/fixtures/dock/tools_160x40.json";
import tools80 from "../../test/fixtures/dock/tools_80x24.json";
import editOpen160 from "../../test/fixtures/dock/edit-open_160x40.json";
import reasoningLive160 from "../../test/fixtures/dock/reasoning-live_160x40.json";

type Recorded = { dock: DockView };
type Args = { recorded: Recorded; cols: number; rows: number; groups?: string[] };

/** The dock as the host sent it (a recorded ScreenView's `dock`), drawn in the room Ink gives it: the 40% of a 160x40
 * body's middle column, or the whole pane at 80x24. */
function Draw({ args, globals }: { args: Args; globals: Globals }) {
  return (
    <Compare
      globals={globals}
      cols={args.cols}
      rows={args.rows}
      ink={<Dock data={args.recorded.dock} width={args.cols} height={args.rows} groups={args.groups} />}
      missing="Ratatui does not draw the dock: it draws one line in its place, 'the agent dock needs the Ink renderer' (the plan's recorded exception)."
    />
  );
}

/**
 * The agent dock under the middle pane (host `DockView`, crates/radar-core/src/dock.rs; Ink only, `src/dock/Dock.tsx`;
 * Ratatui draws the one-line notice in its place). A frame holding the tab row (one tab per session: pack icon, name,
 * state; the one showing in brackets and bold accent, as the page tab row's), the transcript, the composer and the
 * status line. Every story draws a state the host recorded (ink/test/fixtures/dock, written by radar-core).
 *
 * **Use it** through the host: a part supplies the dock's content, the host adds shown/focused/height and sends it on
 * every screen (backtick shows and focuses it, again hides it). Ink keeps no state of record; every key, click and
 * drag is an `Intent::Dock`.
 *
 * **States:** empty, streaming (text marked by the running icon until saved), an answer in markdown (laid out as
 * ui::rich lays out a description, src/text/richRows.ts) and the same half received, tool calls one row each (kind
 * icon, verb, target relative to the session's folder, `+n −m` for an edit) folded to `⋯ n` and open with tree lines,
 * reads in a row folded into one row, an edit's diff opened, reasoning with its latest line while the session works,
 * an approval waiting, a question waiting, an error, stopped, a terminal tab (T7's surface); the activity line above
 * the composer (thinking, writing, running a command, waiting for your approval or answer, with the turn's time and
 * tokens; done, stopped or failed after it); each tab's state mark (spinner, needs you, failed, unread, not attached,
 * being read back); full height (`alt-f`); a session restored from the last run (lapsed requests and questions,
 * detached and resuming, `alt-r` and `/resume`).
 *
 * **Rules:** no chevrons or `[+]`/`[-]` for folding; every mark is a pack icon; keys sit inside their word, underlined
 * (`a̲pprove once`, `d̲eny`, `v̲iew diff`), and a click on the word does what the key does; nothing is approved or
 * answered but by the user, and an approval shows the exact command, path or URL. A question's choices are numbered
 * (the number is the key, before the word), the composer's words answer one that takes them, esc declines.
 */
const meta = {
  title: "Dock/Dock",
  render: (args, { globals }) => <Draw args={args} globals={globals} />,
  args: { recorded: empty160 as unknown as Recorded, cols: 100, rows: 16 },
} satisfies Meta<Args>;

export default meta;
type Story = StoryObj<typeof meta>;

const at = (recorded: unknown, cols = 100, rows = 16): Story => ({ args: { recorded: recorded as Recorded, cols, rows } });

/** A session with nothing sent (empty_160x40). */
export const Empty = at(empty160);
/** A turn running: streamed text, a tool call running, sending off and stop on (streaming_160x40). */
export const Streaming = at(streaming160);
/** Tool calls folded to one line ending in `⋯ n` (tool-folded_160x40). */
export const ToolFolded = at(toolFolded160);
/** The same open: tree lines for the result (tool-open_160x40). */
export const ToolOpen = at(toolOpen160);
/** A write request waits: the exact path and its keys (approval_160x40). */
export const Approval = at(approval160);
/** The agent asks a question: its choices numbered (the number is the key, a click on a choice answers), the
 * composer's words answer it too, esc declines (question_160x40). */
export const Question = at(question160);
/** The connection was lost (error_160x40). */
export const Failed = at(error160);
/** The user stopped the turn (stopped_160x40). */
export const Stopped = at(stopped160);
/** A terminal tab: T7's surface fills the tab (pty_160x40). */
export const Pty = at(pty160);
/** Three sessions, the middle one showing (streaming): each tab ends in the close mark, the pack's `close` icon. A
 * click on a mark or a middle click on a tab closes it, `alt-w` closes the showing one (the status line says so); a
 * session whose turn runs is asked about first (stop it and close / keep it), an idle one closes at once and the
 * neighbour shows (tabs_160x40). */
export const Tabs = at(tabs160);
/** A tool runs: the activity line above the composer says what the agent does now (`running cargo test -p radar-core`),
 * for how long the turn has run, and its tokens when the provider reports usage; the tab spins (tool-folded_160x40). */
export const Activity = at(toolFolded160);
/** The last turn ended: the activity line stays, muted, with how it ended and how long it took, so the dock does not
 * jump when the next turn starts (tool-open_160x40). */
export const ActivityDone = at(toolOpen160);
/** Every tab state at once: the one showing thinks (the spinner), the others need you (the attention mark), failed,
 * or ended out of sight (the unread mark), each mark in its own slot inside the tab; names, never counts. The dock key
 * goes to the one that wants you most (attention_160x40). */
export const TabStates = at(attention160);
/** The dock over the whole middle column (`alt-f`, host state kept for the run; `alt-f split` gives the column back,
 * as does a drag of its top edge), here at the height a 160x40 body gives it (full_160x40). */
export const FullHeight = at(full160, 132, 37);
/** Sessions restored from the last radar run. The showing one is attached; the request and the question that waited when
 * radar closed come back lapsed (the process that asked is gone): muted, the pack's skipped mark, `lapsed · radar closed
 * before this was answered` and `→ lapsed`, no buttons and no keys. A notice says the turn was ended and a message
 * carries on. The other tabs are not attached (the pack's connection mark) or being read back (the spinner)
 * (restored_160x40). */
export const Restored = at(restored160);
/** A restored session not attached to its provider: the body says so, the activity line says `not attached to copilot ·
 * a message or alt-r resumes it`, the status line leads with `⏎ or alt-r resume`, and `/resume` is in the slash menu.
 * A message typed here resumes it first; alt-r resumes without sending anything. A second tab is being read back
 * (detached_160x40). */
export const Detached = at(detached160);
/** The 80x24 terminal: the dock covers the middle pane (empty_80x24). */
export const Narrow = at(empty80, 56, 20);
/** An approval at 80x24 (approval_80x24). */
export const NarrowApproval = at(approval80, 56, 20);
/** An answer in markdown, laid out as ui::rich lays out a description: headings and bold in text.emphasis, inline code
 * in accent.secondary, nested lists with hanging indents, a quote behind its bar, fenced code coloured by the host's
 * highlighter, a table, a link with its address after it (cells carry no OSC 8). Prose stands a row apart
 * (markdown_160x40). */
export const Markdown = at(markdown160, 100, 34);
/** The same answer half received: the open `**` and code span read as if closed, so no stars flash while it streams
 * (markdown-streaming_160x40). */
export const MarkdownStreaming = at(markdownStreaming160);
/** A run of tool calls, one row each: the kind's pack icon, what it does, the target relative to the session's folder
 * (cut in the middle by cells), `+n −m` for an edit, `failed` in the error slot, a running call's mark; five reads fold
 * into `read 5 files ⋯ 5`; each approval already answered sits in its call's row; the one waiting stays below in full
 * (tools_160x40). */
export const ToolCalls = at(tools160, 100, 18);
/** The reads opened (a click on the row, or alt-o): each on a tree line, each opening on its own (tools_160x40,
 * DockLocal.groups). */
export const ToolCallsOpen = { args: { recorded: tools160 as unknown as Recorded, cols: 100, rows: 22, groups: ["group:t0"] } } satisfies Story;
/** An edit opened: its diff under the row in the diff view's look, the changed words on the stronger tint
 * (edit-open_160x40). */
export const EditOpen = at(editOpen160);
/** Reasoning while the session works: folded, its latest line beside the fold (reasoning-live_160x40). */
export const ReasoningLive = at(reasoningLive160);
/** The tool calls at 80x24 (tools_80x24). */
export const NarrowToolCalls = at(tools80, 56, 20);
