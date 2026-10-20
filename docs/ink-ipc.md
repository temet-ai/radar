# The Ink renderer's IPC

Ink is the default renderer; `RADAR_RENDERER=ratatui` is the tested way back.

How radar's host (Rust) and its Ink renderer (`ink/`, Node) talk, who owns the
terminal when, what happens when the renderer is old, slow or dies, and how
to choose the renderer. The code: `crates/radar-core/src/ipc.rs` (the
messages and their bounds), `ipc/bridge.rs` (the host's side),
`ipc/supervisor.rs` (the child process) and `runner.rs` (the switch). The
TypeScript types are generated from the Rust ones into
`ink/src/protocol/generated/`.

## The split

The host keeps everything that decides: the parts, the config, the caches,
the action journal, the write mode and its checks, confirmations, work
started on a service. The renderer keeps only what draws: layout, focus
within a view, scroll positions, the text being edited before it is sent.
It learns everything from the host's messages and sends back only what the
user did, as `view::Intent`s naming targets and action ids the host sent it.
It opens no journal, no config, no state file and calls no provider; its
`package.json` has no SQLite, provider SDK or HTTP client.

## Version

`ipc::PROTOCOL` is **1**. The views it carries have their own version,
`view::VERSION` (**1**). Both are bumped on any change the other side could
misread; added fields default, so additive changes need no bump.

## Transport

- A Unix domain socket. The host makes it in radar's state folder
  (`$XDG_STATE_HOME/radar/ink-<pid>.sock`; the temporary folder when that path
  is too long for a socket) and removes it once the renderer has connected.
  Its path reaches the renderer in `RADAR_IPC_SOCKET`. The host listens; the
  renderer connects within 10 s or counts as crashed.
- One compact JSON object per line, UTF-8, ending in a single `\n`.
  Messages are tagged by `"type"` in snake case. A line is at most
  `ipc::MAX_LINE` = 16 MiB (without its newline); a longer line closes the
  connection.
- A sample of every message is the snapshot
  `crates/radar-core/src/ipc/snapshots/radar_core__ipc__tests__schema.snap`.

## Messages

Every message is one line, so every message is at most `MAX_LINE` = 16 MiB.
The tables give each message's own bound under that: numbers are fixed
size (`u16` columns and rows, `u32` versions, `u64` cursors and counts,
sent as JSON numbers below 2^53), and text has a constant in `ipc.rs` that
the code enforces (the test named beside it).

Renderer → host (`ToHost`):

| Message | Fields | Bound | Meaning |
|---|---|---|---|
| `hello` | `protocol`, `view`, `capabilities` {`columns`, `rows`, `mouse`, `true_colour`, `image`: `kitty`\|`sixel`\|`none`, `plain_text`} | fixed size: two `u32`, two `u16`, three booleans, one of three words | First message, once per connection. |
| `resize` | `columns`, `rows` | fixed size: two `u16` (0 is taken as 1) | The terminal's new size; the parts fit their words to it. |
| `intent` | `seq`, `cursor`, `intent` | two `u64` and one `view::Intent`, whose strings are targets and action ids the host sent; the line bound | What the user did: `seq` counts from 1 per renderer; `cursor` is the newest cursor it had seen. |
| `ack` | `cursor` | fixed size: one `u64` | The snapshot with this cursor is drawn. |
| `log` | `level`: `debug`\|`info`\|`warn`\|`error`, `text` | `MAX_LOG` = 2048 characters; the host cuts the rest (`a_log_line_is_cut_to_its_bound`) | A line for radar's log: what happened, never what was shown. |
| `bye` | | none: no fields | The user quit through the renderer. |
| `resync` | | none: no fields | The dock's transcript came as a change to a revision the renderer does not hold (see *The dock's transcript*): the next snapshot sends it whole. |

Host → renderer (`ToRenderer`):

| Message | Fields | Bound | Meaning |
|---|---|---|---|
| `welcome` | `protocol`, `view`, `cursor` | fixed size: two `u32`, one `u64` | The renderer is accepted; a snapshot follows. |
| `refuse` | `wanted`, `got`, `message` | two `u32`; `message` is the host's own sentence, under 200 characters | Another protocol or view version: the renderer prints `message` and exits with code 3. `wanted` and `got` are always the protocol versions (the host's, the renderer's); when it is the view version that differs they are equal, and `message` names both view versions. |
| `look` | `tier`: `truecolor`\|`256`\|`16`\|`mono`, `slots` {name → colour}, `modifiers` {name → words}, `icons` {name → {`glyph`, `style`}}, `icon_width`, `pack` | one entry per theme slot (the core slots and the ones the parts declare) and one per `ui::icon::Icon`; a colour is at most 13 characters, a glyph a few cells (`ui::icon::check`), a style a slot name and modifiers; `icon_width` 0–2 | The colours and icons to draw with, resolved by the host exactly as the Ratatui renderer draws them (`ipc::Look::of(&Theme)`, test `the_look_sent_is_what_ratatui_resolves_at_every_tier`). A slot's colour is written as Ink's `color` takes it: `#rrggbb` at truecolor, `ansi256(n)` at 256, a terminal colour's name (`red`, `redBright`, `blackBright` …) at 16, `""` (the terminal's own) at mono; a background slot (`bg.selection`) is drawn with the same string. `modifiers` lists only slots carrying their own (`text.emphasis` → `bold`). An icon's `glyph` is padded to the pack's width and empty when icons are off or the pack has none; its `style` is a spec (`status.error bold`). The renderer chooses no colour or glyph of its own once this arrives. |
| `snapshot` | `cursor`, `chrome` (`view::ChromeView`), `screen` (`ScreenView` or null), `section` (`SectionView` or null), `focus` (`PaneId`) | the line bound; the views are bounded by the parts' own view bounds (`view::chrome::MOST` and the room given); a snapshot that would not fit is sent without `screen` and `section`, and logged | Everything showing, whole. With `RADAR_TRACE_LATENCY=1` only, `trace` (`SnapshotTrace`: the cause it answers, the intent `seq`, the cause's epoch ms, the host's ms, the bytes) so the renderer can log the latency to its frame. |
| `question` | `id`, `cursor`, `menu` (`view::MenuView`) | two `u64`; the menu as `chrome.menu` carries it (at most `view::chrome::MOST` items) | A menu or a confirmation opened. Answered by `intent` `invoke` with a choice's value (Esc as a `key` closes it). |
| `input_request` | `id`, `cursor`, `title`, `value` | `title` at most `MAX_NOTICE` = 2048 characters, `value` at most `MAX_INPUT` = 65536 characters; the host cuts the rest | A menu field capturing text opened; keys go back as `intent` `key`. |
| `notice` | `cursor`, `text` | `text` at most `MAX_NOTICE` = 2048 characters; the host cuts the rest (`a_notice_is_cut_to_its_bound`) | The status line's words changed (`copied 4 characters`). |
| `clipboard` | `text` | `MAX_CLIPBOARD` = 1 MiB; longer is not sent and the copy is recorded as failed | Text for the terminal's clipboard: the renderer writes it as OSC 52, because it owns the terminal. |
| `shutdown` | `reason` | the host's own word (`quit`) | Leave now: the renderer exits 0. |

## Lifecycle

1. The host starts the renderer (see *The child*) and waits for its
   connection.
2. The renderer sends `hello`. If `protocol` or `view` differs from the
   host's, the host sends `refuse` with a message naming both versions and
   the way back (`RADAR_RENDERER=ratatui radar`), acts on nothing more from it,
   and the Ratatui renderer takes over with that message on stderr and in
   the status line. A refused renderer is not restarted.
3. Otherwise `welcome`, then `look`, then a `snapshot`, then whatever
   changes. `look` is sent again whenever the theme configuration changes
   (palette, colour tier, a slot's colour, the icon pack, an `[icons]`
   glyph), at once and before the next snapshot, whether or not a snapshot
   is in flight; the newest replaces the last. It was added within protocol
   1: a renderer that does not know it logs it and keeps its defaults.
4. The user quits (a `bye`, or an intent that quits the host): the host
   sends `shutdown`, waits up to 2 s for the renderer to exit (then ends
   it), and puts the terminal back.

## Cursors and snapshots

- `cursor` is a number the host raises by one for every state it tells the
  renderer: each snapshot, question, input request and notice gets the next
  one. It is never reused within a radar session (a restarted renderer
  continues from where the host is).
- A snapshot is whole state: chrome, screen or section, focus. The renderer
  replaces what it showed with it; it never merges, except the dock's
  transcript, which comes as what changed (*The dock's transcript*).
- The renderer acknowledges each snapshot (`ack` with its cursor) when it
  has drawn it.
- **A stale cursor.** An intent carries the newest cursor the renderer had
  seen, and that may be older than the host's newest (the user acted while
  a snapshot was on its way). The host handles it all the same, against
  the state it has now, as it handles a key in the Ratatui renderer that
  arrives after a redraw: targets and action ids are stable within their
  scope, so an intent about an older snapshot still names the same row, and
  a target that is gone does nothing. A part that must not act on data
  newer than the user saw checks its own revision (GitHub's revision lease
  does). An intent whose cursor is *ahead* of the host's is still handled,
  and logged as a renderer fault. The renderer draws whatever snapshot
  comes next, even when it acted after the one it showed; it never holds
  an intent back to wait for a cursor.

## The dock's transcript

A snapshot is whole state with one exception: the agent dock's transcript, the only part of
a snapshot that grows without bound (a session's whole history), goes as what changed.
The bridge (`Bridge::send_snapshot`) keeps the transcript it last sent and compares the new
one with it item by item; the dock carries `revision` (the transcript's, rising by one per
change) and `kept` {`revision`, `items`}: keep the first `items` items of the transcript
held at `kept.revision`, then append `transcript`. A word streamed into the last item sends
that item; nothing changed sends none. With no `kept` the transcript is whole (the first
snapshot, another tab, a renderer started again). A renderer holding another revision sends
`resync`, draws what it was sent, and the next snapshot sends the transcript whole. Ink
(`ink/src/dock/transcript.ts`) gives the dock its whole transcript back, keeping the item
objects it had, so their wrapped rows are reused. The rest of a snapshot is compared as
values (`same_state`), not written out, to decide whether to send.

## Backpressure

- **One snapshot in flight.** The host sends a snapshot only when the last
  one was acknowledged. A change while one is in flight is not queued: the
  next snapshot, sent on the acknowledgement, is the newest state and holds
  every change before it. Progress (spinners, ticks every 200 ms, counts)
  therefore coalesces to the renderer's pace, and the host never holds more
  than one snapshot for it. Nothing durable is lost because a snapshot is
  whole state.
- **Looks, questions, input requests, notices and clipboard text are never
  coalesced.** They go out at once, in order, whether or not a snapshot is
  in flight, so a slow renderer still sees every confirmation and every
  message. None is ever dropped.
- **The bound.** Lines for the renderer go through a queue of at most
  `supervisor::MAX_QUEUED` = 256 lines, written to the socket by a task of
  their own, so the host never waits on the renderer. Only one snapshot
  can be in it (the others wait for its ack); the rest are questions,
  notices and clipboard text, which a renderer that reads at all keeps
  near empty. A full queue alone ends nothing (the writing task may not
  have had its turn): the host waits for room, and only a queue that
  stays full for `supervisor::STALLED_AFTER` = 2 s means the renderer
  stopped reading. Then the host ends it as crashed, puts the terminal back and starts it again under the
  restart policy, and the new renderer is told everything again (a
  question still open, the newest state). So a renderer either sees every
  question and notice or is replaced; the host holds at most 256 lines,
  each at most 16 MiB, for it.
- The events the host makes itself (ticks, finished work) are handled
  only once a renderer has said hello, so none folds into the first
  snapshot unseen.

## Who owns the terminal

Raw mode, the alternate screen, mouse reporting, the cursor and resize
handling belong to one process at a time:

| Moment | Owner | What it does |
|---|---|---|
| Before the renderer starts | host | Saves the terminal's line settings (`stty -g`); writes nothing. |
| While the renderer runs | renderer | Turns on raw mode, the alternate screen and SGR mouse reporting, hides and moves the cursor, handles resizes (and sends `resize`), writes OSC 52 on `clipboard`. The host writes nothing to the terminal and reads nothing from it; stdin and stdout are the renderer's (inherited). |
| After the renderer exits, for any reason | host | Writes `ipc::supervisor::RESTORE`: mouse reporting off (1000, 1002, 1003, 1006), focus and paste reports off (1004, 2004), the alternate screen left (1049), the cursor shown (25); then puts the saved line settings back (raw mode off). |
| RADAR_RENDERER=ratatui, or after a fall back | host (Ratatui runner) | As before Ink: `runner::run_tui` turns them on and restores them on exit and on a panic. |

`rg -n 'enable_raw_mode|EnterAlternateScreen|EnableMouseCapture' crates src`
finds only the Ratatui runner (`runner.rs`); the Ink path's owner is the
renderer process, and the host's part of it is the restore.

## The child

- The renderer is found as: `node <RADAR_INK_BUNDLE>` (a path to the bundle
  file, which must exist; it says where the Ink renderer is, not whether to
  use it), else `<RADAR_RUNTIME> ink` (radar's runtime, see
  [RELEASING.md](RELEASING.md)), else `node` with
  `<folder of the radar executable>/../share/radar/ink/radar-ink.js`, else
  `ink/dist/radar-ink.js` in the checkout the executable was built in (the
  first folder above it holding `Cargo.toml` and `ink/`). The bundle is one
  file: Ink's Yoga layout is inlined. `RADAR_INK_BUNDLE` is a path, not a
  switch: it never turns Ink on or off. When it is set and the file is
  missing, the other places are not tried.
- It runs with stdin, stdout and stderr inherited and `RADAR_IPC_SOCKET` set.
  For a bundle, `node` must be a real binary on `PATH` (`find_node` looks for
  a file named `node` in each `PATH` folder). A mise shim with no global node
  version is not one that works: the lookup fails and radar falls back as
  below. Put a real node first (a mise global or `mise exec -- radar`).
- No renderer or no `node`: one line on stderr says which (`radar: the Ink
  renderer is not installed (…); using the Ratatui renderer` or `radar: the
  Ink renderer's bundle needs \`node\` on PATH; using the Ratatui
  renderer`), radar's log has the reason, and the Ratatui renderer runs.
  Never a silent fall back. When it does start, the log names the command.
- The renderer's process id is logged as `Ink renderer started, pid N` in
  radar's log, for tests that end it.

## Crash and restart

- The renderer exits non-zero, is ended by a signal, closes the socket
  without `bye`, does not connect within 10 s, or stops reading until 256
  lines wait for it: that is a crash.
- After every exit the host puts the terminal back (above). The host's
  state and the journal are untouched: an intent the host had already
  received was handled and recorded as it would be from any renderer; the
  renderer's exit itself records nothing and never marks an action as
  succeeded. A question open before the crash is still open, and the new
  renderer is sent it again.
- The host starts the renderer again, up to **3 restarts in any 60 s**. A
  fourth crash inside the window hands the host to the Ratatui renderer,
  with one line on stderr and in the status line:
  `the Ink renderer stopped 4 times in a minute (…); using the Ratatui renderer`.

## The switch and the way back

There is one switch: the environment variable `RADAR_RENDERER`.

| Value | What runs |
|---|---|
| `ink` | The Ink renderer in a supervised child, over this IPC. If it is not installed, `node` is missing for a bundle, or it crashes 4 times in a minute, the Ratatui renderer runs instead and says so. |
| `ratatui` | The Ratatui runner in the radar process (`runner::run_tui`). No child, no socket. |
| unset, or any other value | The build's default, `radar::DEFAULT_RENDERER` (`src/lib.rs`). |

The default is **Ink** since 2026-10-13. Ratatui stays in the tree as the
tested rollback (`tests/ink_pty.rs` runs the same journey with
`RADAR_RENDERER=ratatui`).

There is no second switch: no command-line flag, no config key, no cargo
feature, no second environment variable. `RADAR_INK_BUNDLE` only says where the
bundle is. The way back from Ink, at any time and without a rebuild:
`RADAR_RENDERER=ratatui radar`. To make it stay, export it in the shell's profile;
reverting the default is the one-line commit.

## Tests

- `ipc::tests`: one sample of every message (the schema snapshot), framing
  and the line bound, the log bound, the switch, and
  `the_committed_typescript_matches_the_rust_types` (fails when
  `ink/src/protocol/generated/` differs from a fresh ts-rs export;
  `RADAR_TS_UPDATE=1` writes it).
- `ipc::bridge_tests`: the same intents through Headless and the bridge
  record the same journal actions with the same causal parents and ask the
  same question; a renderer of another protocol or another view version is
  refused with words to show; a notice is cut to its bound; a slow renderer
  gets every question and notice while snapshots coalesce, then the newest
  state; a restarted renderer is told everything again.
- `ipc::supervisor_tests`: a fake renderer process (the test binary itself,
  run as the test `fake_renderer` with `RADAR_FAKE_RENDERER`) over the real
  socket: says goodbye; is refused for another protocol and for another
  view version, the refusal in the status line and on stderr
  (`say_fall_back`); acknowledges 150 ms late while the host handles 42
  events (21 notices, 20 ticks, one intent that asks a question) and still
  sees all 21 notices in order and the question, with fewer snapshots than
  changes; stops reading and is ended for it and started again;
  crashes once mid-run and is restarted with the terminal restored each
  time and the host's question and journal intact; keeps crashing and falls
  back after three restarts. Also the bundle lookup order, the restart
  window and the socket path.

Regenerate the TypeScript after changing a message or a view type:

```sh
RADAR_TS_UPDATE=1 cargo nextest run -p radar-core \
  -E 'test(the_committed_typescript_matches_the_rust_types)'
```

A new view type derives `ts_rs::TS`; a field kept by icon name
(`with = "icon_name"`) adds `#[ts(as = "Option<String>")]`. TypeScript
names that differ from Rust: `ui::rich::Block` is `RichBlock`,
`ui::rich::Kind` is `InlineKind`, `ui::diff::Kind` is `DiffKind`.
