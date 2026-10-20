---
title: radar — component registry
status: active
created: 2026-10-13
updated: 2026-10-13
summary: Every UI element radar draws, in one list. For each element, its kit or view type, its Ratatui and Ink drawers, states, slots, glyphs, keys and the parts that use it. Also the design rules that keep radar consistent, the one canonical form of each shared concept, and the known drift to fix.
---

# radar — component registry

**Read this before you draw anything.** If the element you need is here, use it. If it
is here with a different look than you want, change it here and in the kit, not in your
part. If it is not here, it goes into the kit first (GUIDE 1.15), then into this file, then
into the gallery (both at 80 and 160 wide).

**Component docs live in Storybook.** Storybook in `ink/` (`npm run storybook`, MCP at
`http://localhost:6016/mcp`) plus its committed manifest, `docs/storybook/components.json`, is now the
source of truth for each component's docs: what it is, when to use it, its states and rules, drawn by the
real Ink component from recorded view data beside Ratatui's picture of the same data. The registry tables
below point there: an element with a story is documented by its story (`Nodes/TreeNode`, `Chrome/MenuBox`,
...); this file keeps the inventory, the parity gaps, the canonical forms and the drift.

This file is the source of truth for what exists. The kit gallery (`cargo run -p radar-core
--example gallery`) shows the elements live. GUIDE.md still owns the rules, and where the
two disagree GUIDE wins until this file is fixed. Line numbers drift: search for the name.

Contents:
1. Rules every element follows
2. View types: what a part sends, and who draws it
3. Kit elements: primitives, composites, chrome
4. Shared concepts and their one canonical form
5. Keys by kind of place
6. Known drift to fix
7. Keeping this file true

The planned changes are tracked in issue #47 (#29–#46). Each entry below names the issue
that changes it.

---

## 1. Rules every element follows

From GUIDE.md and the operator's design decisions (October 2026).

| Rule | Detail | Source |
|---|---|---|
| Parts draw with the kit | No hand-rolled element the kit has. A missing element or variant goes into the kit first. | GUIDE 1.14, 1.15 |
| Nothing visual is hard-coded | Colours come from theme slots and look words. Tones go through `widgets::tone`. | GUIDE 1.8, 2.18 |
| Glyphs come from the icon pack | A part never writes a glyph literal. It names an `Icon`. Marks such as ✓ and ✗ also come from the kit. | GUIDE 1.18, 2.33 |
| Widths are display cells | No `chars().count()`, no `{:<N}` padding. Use `ui::width`, `pad`, `fit`. | GUIDE 1.18 |
| Both renderers, same picture | While Ratatui is the reference, every look change lands in both renderers. A difference between them is a defect. | operator |
| **No chevrons for folding** | No ▸ ▾ › ⌄, nerd chevrons, `[+]` or `[-]` as fold marks. The tree lines show open; a folded row ends in `⋯ n`. | GUIDE 2.12, operator |
| **No pill shapes** | Chips, counts, keys, buttons and tags are square: a ground and one space each side. No round caps. | operator |
| **No left-edge stripes** | No coloured bar down the left of rows or cards. Groups are marked by the heading shade. | operator |
| **Keys live inside their word** | Hint, button and tab keys are the word's letter, underlined (b̲ulk, q̲uit). A key with no letter (↑↓ ⏎ esc) is a glyph before the word. No "b bulk". | operator, #44 |
| **Breadcrumbs join with /** | `acme / api / !263`, not `›`. | operator, #46 |
| **Tabs show names, not counts** | One attention dot when something there waits for you. | operator, #33 |
| **Icons where they mean something** | Several icons to a row is fine when each says something. No rule of one icon per row. | operator |
| **Icons take a fixed slot** | Two cells whatever the pack, so changing pack never moves a column. | operator, #43 |
| **A shadow dims, never erases** | Anything under a layer's shadow keeps its glyphs and lines. | operator, #29 |
| Same key, same action, same kind of place | Keys are context-dependent across kinds of place, but consistent within one kind of place in every part. | operator, GUIDE 2.x |
| Mouse and keys both work | Every click target is padded and lit by the hover shade. | GUIDE 2.5 |
| Don't show the same thing twice | | GUIDE 2.14 |
| Lists, panels and pages look alike | List footers are titled "shows". Sections use `card::section`. | GUIDE 2.29, 2.29a, 2.30 |
| Motion explains a change | Motion never delays a key. Any key cancels it. It is off over SSH and on slow terminals. | GUIDE 1.17 |

---

## 2. View types: what a part sends, and who draws it

Each Ink drawer in this table and in 3.3 has its docs in Storybook under its own name (`Nodes/ListNode`,
`Text/DiffView`, `Chrome/StatusLine`, `Page/PageBody`, `Editor/EditorOverlay`, `Media/MediaBlock`, ...), with
Ratatui's cells beside it and the reference gaps named there.

Parts never draw with ratatui. They send `view::` data (`crates/radar-core/src/view.rs`,
mirrored to `ink/src/protocol/generated/` by ts-rs). Two renderers draw it:

- **Ratatui:** `crates/radar-core/src/view_tui.rs`, `shell.rs`, `shell/menu.rs`
- **Ink:** `ink/src/nodes`, `chrome`, `text`, `app`, `editor`, `media`

`ink/test/contract/every-field-is-drawn.test.tsx` walks every generated field. A field
either has a case, is marked `INVISIBLE` with a reason, or is listed in `SCREEN_KNOWN_UNDRAWN`
(`ink/test/contract/cases-screen.tsx`) or `DOCK_KNOWN_UNDRAWN` (`ink/test/contract/cases-dock.tsx`).
Those two lists are the live parity backlog.

| View type | Ratatui | Ink | Parity gaps (Ink unless said) |
|---|---|---|---|
| `Node::List`, `ListRow` | `view_tui::draw_node` → `ui::rows::RowList` | `nodes/lists.tsx` `ListNode` | `ListRow.heading`, `ListRow.link`, `Editing.at` not drawn |
| `Node::Tree`, `TreeRow`, `Chips`, `TreeLook`, `Note` | `view_tui::draw_tree` → `ui::tree::BandedTree` | `nodes/tree.tsx` `TreeNode`, `nodes/columns.ts`, `nodes/headrow.tsx` | `TreeLook.*`, row `chips`, `glow`, `heading`, `ordered_columns`, `name_width`, `TreeEditing.at` not drawn |
| `Node::Table`, `TableRow`, `ColumnSpec`, `CellView` | → `ui::table::Table` | `nodes/lists.tsx` `TableNode` | `ColumnSpec.extra` |
| `Node::Entries`, `EntryView` | → `ui::entries::Entries` | `nodes/lists.tsx` `EntriesNode` | none known |
| `Node::Log`, `LogPaintLine` | inline loop + `ui::scroll::scrollbar` | `nodes/lists.tsx` `LogNode` | `top`, `LogPaintLine.row` |
| `Node::Diff`, `DiffUnder`, `InlineInput` | → `ui::diff::DiffView` (highlights itself) | `text/DiffView.tsx` (uses host-filled `painted`) | `top`, `language`, `DiffUnder.input`, `InlineInput.*` |
| `Node::Document` | `widgets::paragraph` | `nodes/index.tsx` `DocumentNode` | Ratatui ignores `cut`; Ink honours it |
| `Node::Rich`, `Block` (line, doc, meter, wrapped) | `rich_lines` → `ui::rich`, `ui::image` | `nodes/index.tsx` `BlockView`, `text/RichDoc.tsx` | `pictures`, `doc.most`, `doc.urls`, `wrapped.ink`, link urls, image src |
| `Block::Meter`, `MeterStage` | `ui::meter::stages` | `nodes/index.tsx` `MeterLines` | `MeterStage.look` |
| `Node::State` (empty, loading, failed) | `ui::states::empty` | `text/StateView.tsx` | the three states look alike on both sides |
| `Node::Extension` | always the fallback text | `nodes/index.tsx` (`OWN_CAPABILITIES = ["editor"]`) | Ink-only in-pane editor; `work.*` and `media` drawers exist in Ink but the host sends neither |
| `Editor` overlay | `view_tui::draw_overlay` + `field` | `editor/EditorOverlay.tsx`, `EditorView.tsx` | Ratatui ignores `selection` |
| `ScreenView`, `PaneView` | `render_screen`, `render_panes`, `draw_pane` | `app/Panes.tsx` `ScreenPanes`, `Framed` | `PaneView.from`, `ScreenView.overlay` (an editor over the foot of the body) not drawn |
| `PageView`, `TabView`, `PageLayout` | `render_page` → `ui::page::PageFrame` | `app/Panes.tsx` `PageBody`, `app/TabRow.tsx`, `nodes/pagetitle.tsx` | none known |
| `FieldBar` | `draw_field_bar` → `input::TextInput` | `chrome/filter.tsx` | none (cell-compared) |
| `SectionView` (settings) | `render_section`, `shell::settings` | `app/App.tsx` `SectionBody` | `actions` drawn by neither (the menu owns them) |
| `ChromeView`, `TopBar`, `BarTab`, `Badge`, `ModeChip`, `SourceTab` | `shell::top_bar`, `tabs`, `source_tabs` | `chrome/layout.ts`, `chrome/Chrome.tsx` | none known |
| `StatusLine`, `Hint`, `HintKey` | `shell::status_line`, `key_hits` | `chrome/layout.ts` `layoutStatus` | Status hints remain visible while notices show |
| `NoticeView`, `NoticeStateView`, `NoticeHistoryView` | `ui/notices.rs`, `shell/notices.rs` | `notices/layout.ts`, `paint.ts`, `text.ts`, `History.tsx` | Actual native cells are compared at 80/160, narrow Unicode and all tiers |
| `MenuView`, `Choice` | `shell/menu.rs` | `chrome/layout.ts` `layoutMenu`, `Chrome.tsx` `MenuBox` | `change` deliberately undrawn |
| `SettingsNav` | `shell::settings` | `chrome/layout.ts` `layoutNav` | none known |
| `HistoryView` | `history.rs` | `chrome/history.ts` | `pending` deliberately undrawn |
| `Text`, `Seg`, `Ink`, `Tone` | `view_tui::span`, `words`, `slot` | `text/TextLine.tsx`, `text/cells.tsx` | `marked`, `link`, `look` listed undrawn; may be stale (text tests cover them) |
| `Media` | `ui::rich::media`, `ui::image` | `media/MediaBlock.tsx` (fallback line only; kitty drawer not wired) | pictures: #38 |

Ink draws a few things that have no view type behind them: scrollbars and scroll state,
hit regions, hover, pane frames and widths, the unknown-node fallback, motion, the work
area, and cell widths. Each mirrors a Ratatui behaviour, not a view type.

### 2a. Agent dock: who draws it

The agent dock sits under the middle pane. The host sends it as `DockView`
(`crates/radar-core/src/dock.rs`, on `ScreenView.dock`; while settings show, on `Snapshot.dock`,
and Ink draws it under the section in `SectionBody`, `ink/src/app/App.tsx`). Ink draws every
part below. On settings Ratatui draws no notice line.
Ratatui draws no dock: while the dock is shown, it draws one line in the dock's place,
`dock::NOTICE` ("the agent dock needs the Ink renderer"), in the middle column under the
middle pane (`notice_below` in `crates/radar-core/src/view_tui.rs`; the block in
`crates/radar-core/src/shell.rs`). With the dock hidden it draws no row, so the rows match Ink's.
That line is the one recorded exception to "both renderers, same picture"
(operator, 2026-10-13; [plans/agent-dock.md](plans/agent-dock.md)).

| Dock part | Ratatui | Ink (who draws it) | States (fixtures and stories) |
|---|---|---|---|
| Frame: under the middle pane, about 40% of its column, top edge drags to resize, full height over the pane below 100 columns; `alt-f` (hinted `alt-f full`, then `alt-f split`) makes it take the whole middle column at any width and back, host state (`DockView.full`, `DockIntent::ToggleFull`) kept for the run like the dragged height; a drag gives the column back | the notice line only | `ink/src/dock/Dock.tsx`, `ink/src/dock/layout.ts` (`splitDock(.., full)`), slot in `ink/src/app/Panes.tsx` (`ScreenPanes`) | shown and hidden, focused, resize floor, full height (`full_*`, `Dock/Dock` FullHeight) |
| Activity line: one row above the composer once the session has had a turn (`DockView.activity`, built by radar-sessions from the turn and the plugin's clock). While a turn runs: the kit's spinner (or the pack's `warning` while it waits for you), what it does (`thinking`, `writing`, `running <command>`, `waiting for your approval · <target>`, `waiting for your answer`), the turn's time (`1m 12s`) and tokens when the provider reports usage. Once it ended, muted: `done in 42s · 3.4k tokens`, `stopped after 31s`, `failed after 12s` with the pack's `run_passed`, `run_canceled`, `run_failed`. The row stays so the dock does not jump; the words are cut, never the time. A turn read back from history (resume) shows nothing. Messages sent while the turn runs and waiting for after it are counted (`2 queued`, `DockActivity.queued`). A session restored from the last run: `detached` (muted, the pack's `connection`, `not attached to <provider> · a message or alt-r resumes it`, no time) and `resuming` (the spinner, `resuming <provider>…`, then the seconds since the read-back began) | notice line only | `ink/src/dock/rows.ts` (`activityCells`, `duration`, `tokenWords`), `ink/src/dock/Dock.tsx` | every activity state at 40, 78 and 158 cells (`ink/test/dock/attention.test.ts`, `restored.test.ts`), `detached_*`, `Dock/Dock` Activity, ActivityDone, Detached |
| Tab state marks: after the name, in their own slot inside the tab: the kit's spinner while its turn runs (frame from `DockTab.running`, the turn's seconds), the pack's `warning` while it needs you, `run_failed` when it failed, `unread` when a turn ended while the tab was out of sight (`DockTab.unread`, cleared when the tab shows in a shown dock), `run_canceled` and `run_skipped` once seen. A session restored from the last run and not attached to its provider (`detached`) shows the pack's `connection` in `text.muted`; one being read back (`resuming`) shows the spinner in `status.info`; neither ever needs you (no chip, no desktop notification). Names, never counts; the name is cut before the mark | notice line only | `ink/src/dock/rows.ts` (`tabState`, `tabRowCells`, `tabSpans`) | `attention_*`, `restored_*`, `detached_*`, `Dock/Dock` TabStates, Restored, Detached |
| Text selection by mouse (one selection per pane): a drag, a double click (word, iTerm2 word characters, a path or URL whole) or a triple click (line) in text, shift+arrows/home/end to extend, ctrl+c to copy again, any other key or click to clear; painted on `bg.selection` (reversed in the mono tier); copied by the host (`Intent::Copy`, `HostEffect::CopySelection`, `copy_ways`), notice `copied n characters · way`; the status line hint names the terminal's bypass key (`shift`, `option`, `fn`) while a drag selects. The areas a drag can select in are the inside of each pane (`Frontend::mark_selectable`, `select::Area`); text panes (`Node::is_text`: the `node` tag `document`, `rich`, `log`, `state`, `extension`) take word and line clicks, row panes keep double-click | `crates/radar-core/src/select.rs` (`Gesture`, pure), `runner.rs` | `ink/src/input/selection.ts` | `select::tests` (two panes, edges, words, paths, URLs, keys), `ink/test/app/selection*.test.tsx` |
| Attention chip on the status line: while a session needs you or ended unseen and its tab is out of sight (dock hidden, or another tab showing), the dock key `` ` `` then a square chip after the scope: the pack's icon, the session's name (cut at 24 cells) and `needs you` / `failed` / `done`, `+n` for the others, bold base text on the state's ground (`Ink::Warn`, `Bad`, `Good`). It never gives way. Under the pointer its key is lit and the chip keeps its ground (`ui::hover::paint` and `ink/src/app/hover.ts` keep the state grounds as they keep the selection and the accent). The key (from a pane, or from an empty composer in the dock) or a click shows the dock on that tab (`StatusLine.attention`, `chrome:attention`, `DockView.attention`). Desktop notifications of the same, opt-in in Settings → Agents ("while you are away"), through `system::notify_ways` (notify-send on Linux, osascript on macOS, none over SSH) | the same chip, `crates/radar-core/src/shell.rs` `status_line` | `ink/src/chrome/layout.ts` (`layoutStatus`) | `status/attention_*` with Ratatui's row beside it, `Chrome/StatusLine` Attention, AttentionWide |
| Tabs: one per session (provider icon, name, state, then a close mark `×`, the pack's `close` icon); a PTY tab with the terminal icon. A click on the mark or a middle click on the tab closes it; `alt-w` closes the showing one (`ctrl-w` too from an empty composer) and the status line says so; `/close` is in the slash menu. A session whose turn runs is asked about first (stop it and close / keep it); the closed session stays in the provider's list for `alt-s`; the neighbour tab shows; the last tab closing hides the dock and gives the page its keys back (alt-w on a dock with no tab hides it too), and alt-w works while the dock is shown without the keys; the key is read as ESC w, kitty CSI-u (`CSI 119;3u`) or modifyOtherKeys (`CSI 27;3;119~`) | notice line only | `ink/src/dock/Dock.tsx`, `ink/src/dock/rows.ts` (`tabRowCells`, `tabSpans`), `ink/src/dock/composer.ts` (`dockClose`, `dockHint`) | streaming, stopped, error, PTY tab, two tabs, three tabs (`Dock/Dock` Tabs) |
| Transcript items: user message, streamed text, tool call (`⋯ n` folded, tree lines open), result, error, reasoning (folded); prose (user, answer) a row apart from what is next to it, calls and requests in a run together | notice line only | `ink/src/dock/rows.ts` (marks come from the icon pack) | empty, streaming, tool folded, tool open, error |
| Answer in markdown: the host reads the text (`DockItem::assistant`: `ui::markdown` into `doc`, the half-received last line with its open `**`, `` ` `` and `~~` closed, code blocks highlighted into `painted`); Ink lays the document out as `ui::rich::layout` does, line for line (held to `rich-layout.json`, written by radar-core's `rich_layout_reference`); link addresses follow their words, muted, as cells carry no OSC 8 | notice line only (Ratatui draws the same document this way in descriptions, `ui::rich`) | `ink/src/text/richRows.ts` (`docRows`), `ink/src/dock/rows.ts` | markdown, markdown streaming (`Dock/Dock` Markdown, MarkdownStreaming) |
| Tool call row: one row, `<kind icon> <verb> <target> [+a −r] [failed] ⋯ n`: the kind from the tool's own name (read, search, shell, edit, write, web, mcp, agent, todo), the target relative to the session's folder and cut in the middle by cells, a running call's and a failed call's marks; an approval already answered sits in its call's row (`DockPermission.tool`), one without a call in the transcript is one row (`✓ read approved  path`); two or more reads (or searches) in a row fold into `read 5 files ⋯ 5`, the target of the read still running beside it, opened (click, alt-o; Ink-side `DockLocal.groups`) each on a tree line | notice line only | `ink/src/dock/tools.ts` (`toolKind`, `transcriptUnits`, `shortPaths`, `middleFit`), `ink/src/dock/rows.ts` (`unitLayout`) | tools, both sizes (`Dock/Dock` ToolCalls, ToolCallsOpen, NarrowToolCalls) |
| Edit: `+a −r` on its row (`DockItem::Tool.diff`, from the provider's edit input: `radar_sessions::event::edit_diff`), opened its diff on tree lines in the diff view's look (tinted rows, changed words on the stronger tint) | notice line only | `ink/src/dock/rows.ts` (`diffRows`, `lookOf` and `changedWords` from `ink/src/text/DiffView.tsx`) | edit open (`Dock/Dock` EditOpen) |
| Reasoning while the session works: folded, `thinking · <its latest line> ⋯ n`; after, `reasoning ⋯ n` | notice line only | `ink/src/dock/rows.ts` | reasoning live (`Dock/Dock` ReasoningLive) |
| Approval box: exact command, path, diff or URL; approve once, approve for session (where offered), always allow (adds the request's rules to Settings → Agents, key l), deny, view diff; the buttons wrap to a second row when narrow. A request (or question) that waited when radar closed comes back lapsed (`DockPermission.lapsed`, `AskState` `lapsed`): the process that asked is gone, so it is muted, the pack's `run_skipped`, `<kind>: lapsed · radar closed before this was answered` and the target (a question: each row ends `→ lapsed`), with no buttons, no diff and no keys (a, s, l, d, v, the numbers, esc and clicks ignore it); a notice says the turn was ended and a message carries on. A request the provider asks again after the resume waits and is answered as any other | notice line only | `ink/src/dock/rows.ts` (`itemRows`, permission rows), `ink/src/dock/composer.ts` (`pendingAsk`, `dockClick`), `ink/src/dock/Dock.tsx` | approval pending, `restored_*`, `Dock/Dock` Approval, Restored (`ink/test/dock/restored.test.ts`) |
| Composer: the editor in `ink/src/editor`, multiline, history, the adapter's slash commands (a `/` opens their menu over the foot of the transcript: ↑↓ and tab choose, ⏎ finishes the word and then runs it, a click on a row runs that command at once, `dockCommand`). On a detached session (restored, not attached to its provider) `alt-r` attaches it and reads it back without sending anything (`DockIntent::Resume`; also on an errored tab, a resume that failed; the host ignores it when attached), `/resume` is in the slash menu (the host handles it) and a message typed there resumes it first; the status line leads with `⏎ or alt-r resume` (`resuming…` while it is read back) and an empty transcript says `not attached · a message or alt-r reads it back` (`reading the session back…`). A restored tab is read back when it first shows in a shown dock, never at start | notice line only | `ink/src/dock/composer.ts` (`dockKey`, `dockHint`), `ink/src/dock/Dock.tsx` (`emptyWords`) | the Dock stories (`ink/stories/dock`), `detached_*`, `Dock/Dock` Detached |
| Terminal tab: any CLI's own TUI in a PTY (host PTY in `crates/radar-sessions-pty`) | notice line only | `ink/src/terminal/TerminalView.tsx`, `ink/src/terminal/TerminalSurface.tsx` | running, exited, failed (`Dock/TerminalView` stories) |

Recorded states are the files in `ink/test/fixtures/dock/`, written from the host by
`dock_fixtures_are_what_the_host_sends` (`crates/radar-core/src/host_dock_tests.rs`) when
`RADAR_DOCK_FIXTURES_DIR` is set, and drawn at 80x24 and 160x40 by `ink/test/dock/recorded.test.tsx`;
`status/attention_*` (the status line with the attention chip, and the row Ratatui draws from it) by
`the_attention_chip_is_recorded_with_the_row_ratatui_draws` in `crates/radar-core/src/host_dock_tests.rs`, compared by `ink/test/dock/attention.test.ts`.
Stories: `Dock/Dock` (`ink/stories/dock/Dock.stories.tsx`) and `Dock/TerminalView` (`ink/stories/terminal/TerminalView.stories.tsx`).

---

## 3. Kit elements

All Rust paths are under `crates/radar-core/src/`. "Gallery" names the tab in the kit
gallery: the 9 element tabs (Tree, Table, Card, Meters, Rows & states, Entries & log,
Markdown, Diff, Motion) plus the stories (Colours, Tones, Row states, Grid states,
Waiting, Long text, Icons, Notices). The Motion tab (`ui/motion_demo.rs`, `MotionDemo`) starts each
effect of `motion` with a key (`s m g p n t l`; `f` steps full, subtle, off) and runs only in full colour.

### Session notices

The host records distinct occurrences for this run only. Info expires at 5 seconds;
errors expire at 10 seconds or on input. Escape clears active occurrences but keeps
recent history. `~` opens the explicit recent-notice overlay; `H` still opens the
separate durable action journal. History is inert: selecting a row never repeats
an action. Its newest-first list retains up to 256 summaries of 120 characters;
the selected detail carries the full message, bounded at 2,048 characters, wraps
by complete graphemes, and scrolls with PgUp/PgDn. Omitted older entries are counted.

Transient rows sit bottom-right inside the border of the panel that stands on the
status line at the right edge, read from the frame just drawn (`shell::notices::anchor`,
Ink `noticeAnchor`); with no such panel, inside the content area. Each row is as wide
as its words, at most 64 cells and the panel's inside; a longer one ends in `…` and
the full text is in `~`. Up to four are visible, newest lowest and older rows dimmed.
An additional summary shows the hidden count and opens history. Short panes reserve
a summary row; a one-row pane puts the count in the newest notice. Unless motion is
off, a row reads in from the right over its first 120 ms and goes back into the right
edge over its last 120 ms; a wide glyph the edge would cut stays hidden until whole.
Icons use the active pack, with severity words when icons are off. Truecolor
uses primary text and 18% severity tint over `bg.overlay`, channel-rounded
half-up; the tint's width is `ceil(width × remaining / lifetime)`, kept whole
at wide-grapheme boundaries. Motion off uses static tint and unchanged expiry.
Lower color tiers keep static overlay ground and severity text; mono is reversed.

Both renderers paint notices after fold/focus motion and exclude them from
retained clean frames. Menus, modals, editors, splash, setup, tour, either
history overlay and focused raw PTY input suppress transient rows while expiry
continues. Notice clicks consume their rectangles and dismiss by stable occurrence
ID. History rows and status commands also capture stable identities, so a stale
click cannot act on a replacement provider control. Ordinary dock composers
remain visible and hit-safe.

The Notices gallery draws timed, static and history states. The
`ui::notices::tests::captures::native_notice_cells` contract records the actual
production native buffer into `ink/test/fixtures/notices/`; both renderers compare
the same glyphs, wide tails, styles and rectangles. Capture generation is opt-in
with `RADAR_NOTICE_CELLS_DIR`; snapshots still require review before acceptance.

### 3.1 Primitives

| Element | Where | Does | States and variants | Slots | Glyphs | Gallery |
|---|---|---|---|---|---|---|
| `widgets::tone` | widgets.rs | Turns a tone into a style. The only place tones become slots. | Plain, Muted, Accent, Info, Good, Warn, Bad | TextPrimary, TextMuted, AccentPrimary b, StatusInfo, StatusSuccess, StatusWarning b, StatusError b | | Tones |
| `widgets::panel` | widgets.rs | Rounded, titled block | focused or not | AccentPrimary or AccentMuted border, TextEmphasis or TextSecondary title, BgSurface | `╭─╮│╰╯` | all |
| `widgets::heading` | widgets.rs | Muted bold capitals | | TextMuted b | | Meters, Card |
| `widgets::chip` | widgets.rs / `text/chips.ts` | A label on its tone's tint | by tone; the ground is the tone 16% over BgOverlay in truecolour (`chip_ground`, #30), the flat BgOverlay on 256, 16 and mono | tone + BgOverlay | ` text ` | Card, Tones |
| `widgets::key_hint` | widgets.rs / `text/mnemonic.ts` | Host-positioned underlined letter inside the action word | Unicode graphemes, explicit Shift for uppercase, named/glyph/numeric prefixes; invalid legacy metadata retains its key (#44) | TextMuted underline; AccentSecondary b for explicit prefixes | | gallery footer |
| `widgets::row`, `row_style` | widgets.rs | Selected row with a right-hand word | selected focused or unfocused, plain | see drift 6.2 | | |
| `widgets::Button`, `buttons` | widgets.rs | Three-row raised button row | focused, unfocused, muted, icon | AccentPrimary, BgSelection, BgBase | `▔ ▁` | **missing** |
| `widgets::kv`, `empty`, `paragraph`, `padded` | widgets.rs | Small helpers | | | | |
| `design::select` | design/select.rs | Row selection styles, no glyphs | focused: TextEmphasis on BgSelection, bold; unfocused: TextPrimary on BgOverlay; Mono adds underline | | none | |
| `design::theme`, `theme::Theme` | design/theme/, theme.rs | Slot palette, tiers, style specs | TrueColor, 256, Ansi16, Mono | all slots | | Colours |
| `design::help_bar::KeyHint` | design/help_bar.rs | (keys, action) data | | | | |
| `input::TextInput` | input.rs | One-row editable field | | BgOverlay, TextEmphasis | reversed block caret | Grid states |
| `Icon`, `icon::*` | ui/icon.rs | Semantic icons and packs | nerd, unicode, emoji, off | icon's own slot | from pack | Icons |
| `hover::paint` | ui/hover.rs | Lights what the pointer is over | BgHover, or underline on Ansi16 and Mono | BgHover | | **missing** |
| `scrim::scrim`, `scrim::shadow` | ui/scrim.rs / `app/scrim.ts` | Dims the screen behind a menu, question or help; a one-cell shadow right of and below the box, glyphs kept | truecolour: fg 55%, bg 43% toward BgBase, bold off; shadow bg 60% toward black, fg 45% toward BgBase. 256/16: SGR dim, shadow on black. Mono: dim | BgBase, TextPrimary | | none (Storybook Design/Surfaces, Overlays) |
| `link`, `pointer` | ui/link.rs, ui/pointer.rs | OSC 8 links, OSC 22 pointer shape | on, off, detect | | | **missing** |
| Text helpers `width`, `take`, `pad`, `fit`, `clip`, `wrapped`, `duration`, `short_name` | ui/mod.rs | Display-cell string handling | | | | Long text |
| `Mark` | ui/mod.rs | Run-state symbol and icon | Done, Failed, Running, Waiting, Manual, Stopped, Skipped | via Tone | `✓ ✗ ● ◷ ▷ ⊘ »` | Tones |

### 3.2 Composites

| Element | Where | Does | States and variants | Keys | Gallery |
|---|---|---|---|---|---|
| `BandedTree`, `Node`, `TreeLook` | ui/tree.rs | Grouped hierarchy with bands and inline input. **The one tree for every part (#45).** | selected, unfocused, folded (`⋯ n`), input row | `+` open selected, `=` open all held groups, `-` close group/parent, `←`/`→` navigate, `space` toggle, `↑` at top to the bar | Tree, Grid states |
| `Columns`, `Column` | ui/columns.rs | Aligned right-hand columns, dropped by rank as width shrinks | | | Tree, Table |
| `Table` | ui/table.rs | Flat list with headings and cursor | empty, cut with `…` | | Table |
| `RowList`, `ListRow` | ui/rows.rs | Selectable rows, headings stepped over | heading, detail dropped whole | `↓ ↑ Home End PgDn PgUp` | Rows & states |
| `rows::kv`, `labelled`, `add_row` | ui/rows.rs | Label/value rows; `+ add …` first row | | | Card, Rows & states |
| `Entries` | ui/entries.rs | Rows with lines beneath them | | | Entries & log |
| `Log` | ui/log.rs | Long output: fold, `/` search, ANSI to slots | folded, match | `/`, `n`/`N`, `+`/`-` | Entries & log |
| `Diff` | ui/diff.rs | Hunks, line numbers, tints, inline blocks | cursor row, added, removed; word tints planned | `DiffState::key` | Diff |
| `Highlight` | ui/highlight.rs | syntect code colours on slots (Ink: shiki, #36) | unknown language is plain | | in Diff, Markdown |
| `Rich`, `Markdown` | ui/rich.rs, ui/markdown.rs | Document layout and markdown | | | Markdown |
| `Image` | ui/image.rs | Inline pictures via ratatui-image (kitty, sixel, iTerm2). Ink port: #38 | on, off, detect | | **missing** |
| `PageFrame`, `crumbs`, `title_line`, `tab_row` | ui/page.rs | Item page head: crumbs, title, details, under line, tabs | focused tab row, tab window when narrow | `← → h l [ ]`, `esc` back | all tabs |
| `frame::{focus_border, wordmark}` | ui/frame.rs / text/frame.ts | Theme-resolved truecolor ramp from glow.purple through accent.tertiary to glow.cyan; clockwise frame perimeter and left-to-right wordmark | only the keyboard-owning pane; titles and scrollbars retain their styles; lower tiers unchanged | | Card |
| `card::{card, panel, section, footer, shows}` | ui/card.rs | Titled scrolling panel, section rule, "shows" footer | footer only with 3 rows to spare | `Scroll::key` | Card |
| `scroll::{Scroll, scrollbar}` | ui/scroll.rs | Scroll state and bar | no bar when nothing to scroll | PgUp/PgDn | Card |
| `chips::{chip, chips, after_name, words}` | ui/chips.rs | Contributed chips; chips after a name that gives way | | | Card |
| `status::strip` | ui/status.rs | Counts joined by muted ` · ` | | | Meters |
| `status::Check`, `as_you_type` | ui/status.rs | Check mark, line and verdict | NotYet, Running, Passed, Failed | | Rows & states |
| `meter::{stages, ratio, sparkline}` | ui/meter.rs | Stage bar, `━─` ratio, sparkline | | | Meters, Card |
| `states::{empty, set_up, loading, failed, busy, spinner}` | ui/states.rs | Empty, not set up, loading, failed, spinner | | | Rows & states, Waiting |
| `Composer` | ui/composer.rs | Multi-line write input | one line, multi-line | `⏎`, `ctrl-s`, `esc` | Long text |
| `motion_demo::MotionDemo` | ui/motion_demo.rs | The gallery's Motion tab: every effect of `motion` on sample rows, each started by a key | screen switch, and the other effects (`s m g p n t l`); levels full, subtle, off (`f`) | `s m g p n t l`, `f` | Motion |
| `connection::*` | ui/connection.rs | A part's connection section | token masked, env found | | Rows & states |
| `rail::{render, Stop}` | ui/rail.rs | Setup's step rail: the steps along one track, two rows (the stop, then its line: a result, `skipped`, `step n of m`); the track is lit (`status.success`) up to the current stop | done (`run_passed`), skipped (`run_skipped`), ahead (`todo`, muted), current (its own icon on `accent.primary`, a square chip); labels cut with `…` in display cells | a click on a stop reached goes back to it | snapshots `rail_80`, `rail_160` (no gallery tab yet) |

### 3.3 Chrome

| Element | Where (Ratatui / Ink) | Does | Notes |
|---|---|---|---|
| Top bar and tabs | `shell::top_bar`, `tabs` / `chrome/layout.ts` `layoutTopBar` | Brand, names-only part tabs with an underlined key and one warning-coloured attention dot when needed, spinner, words, mode | Truecolor brand ramp uses resolved glow.purple → accent.tertiary → glow.cyan; padding, hit geometry and lower tiers unchanged (#31) |
| Tab attention | `BarTab.attention`, `TabView.attention` | One dot after the name in `status.warning`; no count. The host derives top-bar attention from ready parts' warn/bad badges and unsaved settings. Item-page owners set attention from actionable state. Counts remain in their page/status content. | |
| Source tabs | `shell::source_tabs` / `chrome/layout.ts` | Second row: a part's sources | |
| Status line | `shell::status_line`, `key_hits` / `layoutStatus` | Mode chip, key hints or a message, counts at the right | notices planned (#41) |
| Menu and question | `shell/menu.rs` / `layoutMenu`, `MenuBox` | Menus with key, label, line; questions as dialogs with buttons | over a scrim with a one-cell shadow (`ui::scrim`, #29); nothing behind is lit; Ctrl-P palette with scalar-indexed match highlights, right-side keys and match/total count (`MenuView.palette`); cached targets, no query-time requests |
| Field bar | `view_tui::draw_field_bar` / `chrome/filter.tsx` | Filter (`/`) and go-to (`:`) at the top of a pane | |
| Panes and frames | `shell`, `view_tui::draw_pane` / `app/Panes.tsx` | Nav, middle, detail; focused frame | Truecolor ramp on the keyboard-owning pane, including active inline fields; background panes stay flat under menus, overlays and visible focused docks (#31). Host-owned jump numbers match the active logical pane mapping, including responsive layouts and setup exclusions (#46). The focus perimeter follows the shared motion policy (#32) |
| Settings | `shell::settings` / `layoutNav`, `SectionBody` | Settings nav and sections | Agents folder rows offer repository overrides (`p`): shared menus add/select a repository and edit checkout, working and review folders, or drop overrides. Git link menus offer a per-repository worktree template (`o`). Empty overrides inherit global settings. |
| Setup | `shell/setup.rs` / `app/SetupBody.tsx` | `radar  setup / <step>` with `step n of m` and the mode; at most 140×40, centred; the sheet ("about this step", the step's help) on top as high as its text, then the card (heading, lead, then the step's settings section or a host step's rows) under it, each a filled panel under its tab; no rail; the key bar is the status line scoped `SETUP` | from `ChromeView.setup` (`SetupChrome`) and `Snapshot.section`; no tabs, no settings nav |
| History | `history.rs` / `chrome/history.ts` | Action history overlay | |
| Editor overlay | `draw_overlay` / `editor/EditorOverlay.tsx` | Writing a description or comment | split preview planned (#37) |
| Startup splash | `splash::frame` + `splash::draw` (runner) / `app/Splash.tsx` from `Snapshot.splash` | A 64×24 braille radar scope with a contact for each part on the splash (`Plugin::on_splash`: mark, name, what it holds for you, or `no signal`). Under it: the readout, the `radar` mark in half blocks (`splash::BITMAP`, 29×4 cells), the parts' names and the start line (`start_lines`). All 34 rows are centred on `bg.base` | One layout for both: radar-core works out every cell and colour. The phosphor ramp is mixed from `glow.cyan`, `accent.muted` and `glow.purple`; the mark's ramp runs `glow.purple` → `accent.tertiary` → `glow.cyan` on full colour, `accent.primary` below, bold with none. Timing in `splash::Splash`: full hands off at 2300 ms with the screen fade (an event 1.5 s later); subtle at 1600 ms; motion off only when startup is slow. Any input ends it and acts. Below 64×34 it drops the scope, below 29×8 the mark, below 20 columns it shows nothing. `[theme] splash = auto \| off`. Skipped before 1.5 s, its start line shows on the status line for 5 s. Pixel tier: where the terminal takes kitty's graphics, the scope's dots are painted in pixels, each in its own colour (`splash::picture`, kitty image 7370 replaced each frame and deleted when the splash goes; `SplashView.picture` for Ink); braille everywhere else. Cells compared in `ink/test/splash` |
| Tour card | `shell/tour.rs` / Ink from `ChromeView.tour` | `tour · n of m` in a panel at the body's lower right, over the app: the stop's title (bold), its lines, `try it: press …` (muted), its keys; the kit's shadow | from `TourCard`; the app under it keeps its keys; `→ ← esc`, the key a stop names goes on |

---

## 4. Shared concepts and their one canonical form

Each concept below is drawn more than one way today. The canonical form is the target;
the "today" column lists what has to change. Paths are under `crates/`.

| Concept | Canonical form | Today | Issue |
|---|---|---|---|
| Breadcrumbs | Segments joined by a muted ` / `, last one bold. First crumb is the part name. | ` › ` (`radar-core/src/ui/page.rs`); first crumb is the screen name in GitLab, the part name elsewhere; GitHub's is Muted, the others Secondary; GitHub keeps `owner/repo` as one crumb | #46 |
| Nested data | The shared tree, in every part | Shared tree navigation and fold admission cover nested owner data, including GitHub repository settings and Workspace's Local repositories screen. Jira assembles arbitrary-depth ancestry from loaded tickets; the Workbench's genuinely flat view stays flat. | #45 |
| Fold signal | Tree lines; folded row ends `⋯ n` | Both renderers reserve the folded marker and count held descendants, including GitHub. This is a loaded-view count, not an unfetched provider total; very narrow rows retain the marker without inventing a shortened number. | #45 |
| Run and state marks | `Mark` and `Icon`, one meaning per glyph | `●` means running (GitLab), done (Jira), in use (Git), unread (Slack); GitLab `ready::blocks`, Jira category marks, GitHub plain words | Separate glyph-semantics follow-up; not the #45 row-state contract |
| State chips | `widgets::chip` by tone, tinted ground (the kit draws it since #30) | GitHub uses a bare word in Secondary; Slack builds its own | |
| People | person or `you` icon + short name; avatar when pictures work | GitLab `person_icon`, Jira `assignee_segs`, Slack `who()`, GitHub none | #38 |
| Ages | `time::ago` (`5m`, `3d`) in cells; `N ago` in details | local wrappers (Jira `ago`, Slack `age`, Slack `demo::ago`) beside core's `time::ago_str` (`crates/radar-core/src/time.rs`); only Jira tints by age | |
| Counts | `data.number` in cells; none on tabs | bare numbers, `"{n} NEW"`, `"{n} repos"`, `"{n} PRs"`, `"{n} MR(s)"`, `↑n` outside tabs | |
| Section headings | `widgets::heading` (muted bold capitals) | a detail pane's sections are `view::Block::section` (drawn as `ui::card::section`, muted bold capitals and a rule) in GitLab, Jira, GitHub, Slack and Git; Workbench uses Emphasis | #46 |
| Tree header strip | `widgets::heading` | Plain in GitLab and Jira, Muted bold in Git, Secondary in Slack | |
| Label/value facts | `rows::labelled` with label width = longest label + 2 | widths 14 (Jira), 10 (GitLab settings), computed (Git's local `kv`) | #46 |
| Empty, loading, error | `Node::State` in a pane; a tree `Note` when the rows must stay selectable | Workspace uses a plain line; GitHub errors are bare red text; Git uses `✗` + `icon_or` | #42 |
| Verdict ✗ | one helper and one colour | `✗` is Warn in `kit::as_you_type`, Workspace and Jira settings, but Bad in Git and Slack | |
| Unread | `Icon::Unread` | bare `"● "` in Slack | |
| Day divider | one kit line | hand-written `"── #chan · day ──"` and `"── … ──"` in Slack | |
| Colour swatch, tenancy bullet | one kit element | `"██ "` and `"● "` in GitLab and Jira settings | |
| Editor titles | `Icon::Comment` + words | literal `"✎ …"` in GitLab and Slack | |
| Site and tenancy colours | one shared slot table | copied three times in `lib.rs`: `TENANT_SLOTS` (GitLab), `SITE_SLOTS` (Jira), `CHANNEL_SLOTS` (Slack) | |
| Detail pane | Head on the heading shade, aligned facts, sections under named rules, pinned actions with keys in words | GitLab MR, Jira ticket and Slack message previews use `view::detail`: three-row shaded identity, nonempty aligned facts, existing named sections and a pinned foot. The typed extension includes a complete ordinary Rich fallback. Live single-press controls are stamped and revalidated by the displayed host; raw views contain no runnable foot targets. No section drowns the others: a detail pane is a summary and the item's page has everything. The last comment shows at most 12 lines with a link to it on its service; another part's section (`Parts::detail_sections`) is marked with its icon like the owner's and cut to `DETAIL_SECTION_MOST` lines, ending `⋯ n more lines on its page`. Contributed lines carry their own looks and links in place of spelled-out addresses. | #46 |

---

## 5. Keys by kind of place

Keys may mean different things in different kinds of place. Within one kind of place they
mean the same thing in every part.

| Place | Key | Means | Today's drift |
|---|---|---|---|
| Any tree | `space` | toggle the fold | consistent |
| Any tree | `+` / `=` / `-` | open / open every node / close | shared host routing and hints; existing owner Enter actions remain separate |
| Any tree | `→` / `←` | open or go in / close or go up | focused trees own unmodified horizontal arrows; h/l, Tab and digits retain pane movement |
| A group row (tenancy, site, status, heading) | `⏎` | **to decide**: open its page, or toggle the fold | GitLab opens a page; Jira and Slack fold; GitHub opens a page; Git means "use it" |
| An item row | `⏎` | open its page | consistent |
| Any row | `x` | actions menu | consistent |
| Any page | `[` `]` | previous / next tab | the Workbench pages through groups instead |
| Any screen | `/` filter, `:` go to, `r` refresh, `o` open in browser, `y` copy link | | Workbench `y` copies a batch |
| A discussion | `c` | comment or reply | GitLab job uses `c` for excerpt, Workbench for copy locator |

---

## 6. Known drift to fix

Found by inventory on 13 October 2026. Each item is a defect against the rules above.

1. **`HelpBar` is dead code** (`design/help_bar.rs`). Nothing calls it; the status line
   uses `widgets::key_hint`. It also breaks the width rule. Delete it, or make it the one
   key-hint drawer.
2. **Two unfocused-selection looks.** `design::select` gives TextPrimary on BgOverlay;
   `widgets::row_style` gives TextSecondary on BgSelection. Pick one.
3. **The active-tab style is copied three times**: `shell::tabs`, `source_tabs`, and the
   status-line chip. `ui::page::tab_row` has a fourth variant.
4. **Wrapper pairs that look alike**: `card::panel`/`widgets::panel`,
   `states::empty`/`widgets::empty`, `rows::kv`/`widgets::kv`,
   `scroll::paragraph`/`widgets::paragraph`, `look_heading`/`heading`,
   `status::strip` and the status line's own counts join.
5. **Two `padded` functions** with different meanings (`widgets.rs`, `view_tui.rs`).
6. **Width-rule violations**: `shell.rs` tab labels, `shell/menu.rs` caret, `meter::stages`,
   `input::render_field`, `help_bar.rs`.
7. **Glyph literals outside the pack**: `ui/status.rs` (`✓ ✗`), `view/kit.rs` (`✓ ✗ ⚠`), and the
   part literals in section 4.
8. **The spinner is drawn two ways**: the top bar uses `BRAILLE_SIX` directly; the kit
   uses `states::spinner`.
9. **Three field paths**: the field bar, the menu's typed field, the tree row input.
10. **Log fold glyphs duplicated** in two render paths of `ui/log.rs`.
11. **Missing from the gallery**: `Button`, `FieldBar`, `Link`, `Pointer`,
    `Image`, `Hover`, and all chrome (top bar, status line, menu, settings panes).
    Fixed for `TextInput` (Grid states story) and `Composer` (Long text story).
12. **Three hand-copied tone and slot maps in Ink**: `text/theme.ts` `SLOT_OF`,
    `chrome/layout.ts` `chromeInk`, and `text/generated/pack.ts`, which has no script
    to regenerate it.
13. **GUIDE names that no longer exist**: GUIDE 2.1 and 2.2 cite `shell::hints`;
    the work is in `status_line` and `key_hits`.
14. **Dead ratatui render in a part**: `radar-workspace/src/screen.rs` (`#[allow(dead_code)]`).
15. **`KNOWN_UNDRAWN` entries that may be stale**: `SCREEN_KNOWN_UNDRAWN` holds 27 fields
    today (state nodes, tree look and chips, `heading`, `Seg.marked`, `Seg.link`, `Seg.look`,
    `PaneView.from`, `ScreenView.overlay`, inline inputs under a diff, `TerminalState.exited.code`, ...);
    `DOCK_KNOWN_UNDRAWN` holds `DockTab.provider` and `TerminalView.cwd`. Each goes red with
    "now drawn" when fixed. Check each against its renderer and remove the ones already drawn.

---

## 7. Keeping this file true

- **Change it in the same commit** as the element: a new element, a new state, a new key
  in a kind of place, or a fix from section 6.
- **A new view field** shows up in `ink/test/contract/every-field-is-drawn.test.tsx`.
  If it can't be drawn yet, it goes into `SCREEN_KNOWN_UNDRAWN` or `DOCK_KNOWN_UNDRAWN` and into section 2 here.
- **The view-type and node-kind lists could be generated** from the `Node` and `Block`
  enums (ts-rs already exports them, and `ink/test/contract/support.tsx` already derives
  the kinds), with a check that each kind has a drawer on both sides. Until that exists,
  this file is kept by hand.
