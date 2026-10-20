---
title: radar — builder's guide
status: active
created: 2026-10-06
summary: How radar is built (host, plugins, pages), the UX contract distilled from the user's corrections, how to test, and a Jira walkthrough, so the next plugin is right the first time.
---

# radar — builder's guide

How to add to radar without a round of corrections. Read it before writing a part. It has
five sections:

1. [How radar is built](#1-how-radar-is-built): the host, the `Plugin` trait, the building blocks.
2. [The UX contract](#2-the-ux-contract): checkable rules, each with the reason behind it.
3. [Testing](#3-testing): unit tests, snapshots, the actions-menu test, the live check.
4. [Adding a part: a Jira walkthrough](#4-adding-a-part-a-jira-walkthrough), and the hand-off checklist.
5. [Known issues](#5-known-issues).

Paths are relative to the repo root. Claims here were last checked against the code on
2026-10-16.

---

## 1. How radar is built

### 1.1 Host plus plugins

```
crates/radar-core          the host: Plugin trait, reducer, chrome, settings, setup, theme, config,
                           runner, CLI, and compose (subjects, Show, contributions, the parts section)
crates/radar-gitlab        GitLab: merge requests, pipelines, the Repositories screen (the reference part)
crates/radar-github        GitHub: pull requests and checks (read-only for now)
crates/radar-jira          Jira: tickets across sites
crates/radar-slack         Slack: mentions, threads, channels
crates/radar-git           local Git: checkouts, branches, worktrees
crates/radar-workspace     local workspaces and their clones
crates/radar-workbench     Work: local work items
crates/radar-sessions*     agent sessions and the dock; one crate per provider
src/lib.rs                 the parts compiled in, each behind its cargo feature (`radar::parts()`)
src/main.rs                runner::main(parts, radar::DEFAULT_RENDERER); also `radar mcp`, the
                           relay an agent CLI starts as radar's MCP server
```

`radar-core` is a clean interface that knows no tool or service; the parts integrate the
tools. A part (a plugin crate, `radar-<id>`) depends on `radar-core` only (never on another
part), implements `radar_core::Plugin`, and is registered in `src/lib.rs` behind its feature.
Parts meet only through radar-core: subjects, `Show` and contributions (1.13) and the bus
(1.6). `Host::new`
(`crates/radar-core/src/host.rs`) refuses, at start-up, two plugins with the same id, two
screens with the same key, a screen key that is not an ASCII capital or is one the host
keeps (`H`, `,`), an id that is the host's own (`theme`, `terminal`, `setup`, and its
general sections), two settings sections with the same name under one group (the host's
`colours`, `parts`, `mode` and `icons` are under `general`; every part has its own
`connection` under its own heading), and two theme slots with the same name.

Dependencies you are expected to use (declared in the root `Cargo.toml`, all from
crates.io; never a path outside the repo): `opaline` (palettes, mapped onto
radar's slots by `radar_core::design`, which also has `KeyHint`). The host and the parts
name no terminal library: colours, styles and keys are radar's own types
(`design::theme::{Color, Modifier, Style}`, `view::Chord`), and Ink draws every view.

### 1.2 The reducer and effects model

Nothing does I/O inside a handler.

- The host's `Host::update(Event) -> Vec<HostEffect>` is pure. `Event` is `Key`, `Tick`,
  `Wake`, `Msg` (a spawned task's result), `Saved`, `Notice`, `ErrorNotice`, `ToClipboard`, `Paste`,
  `Mouse`, `Intent` (from Ink), `History` and `Tracked` (the action journal),
  `TerminalRoute` and `Scanned` (setup's look for keys).
- A plugin's handlers (`intent`, `on_msg`, `on_bus`, `on_menu`, `tick`, `paste`)
  change the plugin's own state and return `Effect`s: `Spawn(Task)`, `Open(url)`,
  `OpenApp{link,web}`,
  `Copy(text)`, `Notify(msg)`, `Emit{topic,payload}`, `PersistCache`, `Focus(Pane)`,
  `Menu(Menu)`, `Show(Subject)`, `Changed(Changes)` (rows a sync changed, for motion,
  1.17), `Press(char)` (a key pressed where the focus is: a row an owner drew for another
  part's action does what its key does, 1.13), `Act` (run another part's action on a
  subject), `NotifyWith(icon, msg)`, `Notice(Message)`; for the agent dock `StartReview`, `SendToAgent`,
  `ShowDock`, `HideDock`; for the action journal and the work map `Journal`, `Report`,
  `SpawnAction`, `Observe`, `Work`, `ResolveSources`, `ShowSource`
  (`crates/radar-core/src/plugin.rs`).
- `Host::route` turns plugin effects into `HostEffect`s and delivers bus events.
  The runner's `perform` (`crates/radar-core/src/runner.rs`, private) does the real work: spawns the
  task on tokio and sends the result back as `Event::Msg`, opens links, copies, writes
  `config.toml`, and queues caches for `plugin_cache`'s writer thread.
- Each notice enqueue gets a unique occurrence ID, including repeated words.
  `Effect::notify` is informational; use `Effect::error` for a semantic failure,
  without guessing severity from its text or icon. Host producers use `Host::notify`
  or `notify_error`. The bounded session ledger retains the latest 256 notices and
  an omitted count, never persisting their text. Information expires after five
  seconds; errors after ten seconds or a key; Escape clears the active set.
  The runner and IPC bridge inject monotonic UI milliseconds separately from
  provider/journal epoch time. Snapshots carry active occurrence IDs and bounded
  history only while open; immediate IPC delivery uses the same IDs.
- Nothing a part or the host shows is drawn by it: both describe it as view data
  (`radar_core::view`) and Ink paints it. A part gives `Plugin::view` and
  `section_view` (its screens and sections); the host gives the chrome,
  `Host::chrome_view(width, height) -> view::ChromeView` (the top bar's tabs with their
  mnemonics, attention flags and targets, the part's words and the mode on its right, the source
  tabs, the status line's keys each with its chord and target, the open menu or question
  as a `MenuView`, the settings navigation). Ink draws the chrome from that data and
  records each pointer area from a target in it. A click on a chrome target reaches the host as
  `Intent::Activate { target }` (`Host::chrome_target_of`), the same intent
  `Headless` and Ink send, so both press a tab, the mode, a settings
  row, a menu row or a status-line key the same way. State only known while drawing (how
  far a pane can scroll) lives in Ink, or in the host's `Cell`/`RefCell` fields, never in a part.
- The runner reads keys on a thread of its own, enables bracketed paste and mouse
  capture, and sends `Event::Tick` every 200 ms (and on resize). A part that woke the host
  (`radar_core::wake`) gets `Event::Wake` at once, then at most one per 40 ms however often it
  wakes. It draws after each event, and on a 30 ms frame tick only while a motion effect
  runs (1.17) or a visible notice countdown needs it. Notices are composited after
  motion, outside retained frames; motion-off keeps their expiry and static ground.

### 1.3 The `Plugin` trait, member by member

Every member except `id`, `configure` and `config` has a default. The "called" column says
when the host calls it. The trait names no terminal type (no terminal library, `Buffer`,
`Rect` or `KeyEvent`; a ban test over radar-core and every part crate keeps it so):
a part describes its screens and sections as views and gets what the user did as
`Intent`s and keys as `view::Chord`s.

| Member | What it is for | Called |
|---|---|---|
| `id() -> &'static str` | Config table name `[<id>]`, cache rows `plugin = <id>`, CLI subcommand `radar <id>`, owner in slot names | always |
| `configure(Option<&toml::Value>)` | Read your `[<id>]` table; `None` when the file has none. Return `Err` to abort start-up with the file path | `Host::load_config`, once at start |
| `config() -> Option<toml::Value>` | The table to write | **every frame** and after every event (the host compares it with how the last event left it, and saves a completed change, 1.7). Keep it cheap and deterministic |
| `first_run() -> Option<String>` | Called when there is **no config file at all**; the host then opens setup (`Host::open_setup`) and does not show the note | only with no file; a file without your table gets `configure(None)` |
| `slots()` | Your domain theme slots and defaults (1.8) | `Host::new` |
| `demo()` | Replace what you hold with your sample data (the stand-ins your tests use), so the offline demo and `ink-live` show your screens with no network or config. Default: nothing. Implement behind your `demo` feature | the demo host and `ink-live`, after `load_config`; never at a normal start |
| `restore(Value)` / `snapshot()` | Cache read at start, so the first frame is never empty / the value to write | `restore` after `configure`; `snapshot` when you return `Effect::PersistCache` |
| `tick(now)` | Time passing. Start refreshes here | every 200 ms and on resize; throttle yourself (`GitlabPlugin::tick` uses `refresh_secs`) |
| `set_waker(Waker)` / `woken(now)` | Background work of your own that has something to show now (an agent's streamed text): keep the `Waker` and call `wake()` from any thread; take the news in `woken`. Wakes close together are one call (at most one per 40 ms). `SessionsPlugin` wakes on every push into its inbox | `set_waker` once, in `Host::new`; `woken` soon after a wake (not while paused; the next tick takes it in) |
| `input(now)` | Someone is there: a key, click or paste reached radar (any screen), after it was handled. Resume what you paused while nobody was looking (`GitlabPlugin` pauses refreshing after `idle_minutes`) | after every `Key`, `Mouse` and `Paste` event, for every plugin |
| `on_msg(Msg)` | A task you spawned finished. `Msg = Box<dyn Any + Send>`; downcast to your own enum | after `Effect::Spawn` |
| `on_bus(topic, &payload)` | Another plugin emitted a topic | `Effect::Emit` from any other plugin |
| `capturing()` | True while you take text input. Then **every** key reaches you, including `q`, `x`, `?`, `w`, screen keys; mouse clicks are ignored | before every key and click |
| `paste(&str)` | Text pasted while capturing | `Event::Paste` |
| `screens() -> Vec<ScreenSpec>` | `{id, name, key}`: a screen, a tab, a Shift+letter | start-up |
| `badge(screen)` | An optional state/count with a tone. The host shows only one warning dot on its tab for `Warn` or `Bad`; other tones show no tab indicator. Keep counts in page/status content. | each frame |
| `top_right(screen)` | Right of the top bar, e.g. "synced 12s ago · every 1m" | each frame |
| `busy(screen)` | True while loading: the top bar shows a spinner | each frame |
| `status(screen) -> Vec<(String, Tone)>` | Counts for the right of the status line, e.g. "3 to review" | each frame |
| `screen_group(screen)` / `icon(screen)` | Screens of several parts under one top-bar tab (each a source tab under it) / the tab's icon | each frame |
| `view(screen, cx) -> Option<ScreenView>` | The screen as renderer-neutral data (`radar_core::view`): panes with lists, trees, documents or states, stable targets, and the actions where the focus is, each with the part's own stable id (`ActionSpec::named("gitlab.mr.approve", &binding)`; ids are written out, never made from a label or a key). The Ink renderer over IPC draws it and `Headless` turns it into text for tests; a screen without one shows "no view for this screen" | each frame, and before routing input |
| `section_view(section, cx) -> Option<SectionView>` | As `view`, for a settings section: rows, the field being typed into, the detail and the row's actions | each frame |
| `intent(scope, pane, &Intent) -> Outcome` | What the user did on a screen or section, from Ink: `Select`, `Activate`, `Context`, `Fold` on a target, `Invoke` an action by its id, a `Key`. Movement and scrolling are the frontend's; you get the selection. Match your action ids in `Invoke`; an action you do not match runs by the key your view lists for it (the host's transitional fallback, M11 A0b), so an action without a key must be matched by id | input |
| `pictures(source)` | How the pictures in your rich text are fetched, with your own account | while drawing |
| `panes(screen)` | Panes `tab`, `h`/`l` and `1`..`9` move through. Default `Nav, Middle, Detail` | on those keys |
| `hints(screen, pane)` | Status-line keys **only while `capturing()`**, e.g. `⏎ keep  esc cancel` | each frame, only while capturing |
| `row_hints(screen, pane)` | The few keys that apply to what is selected now (`u rebase`, `m merge`), put **first** on the fixed status line when not capturing; none by default | each frame |
| `bulk(scope) -> Vec<MenuItem>` | Things done to everything shown at once; opens with `b`; the choice returns as `on_menu("bulk", value)` | on `b` and each frame (to decide whether the bar shows `b bulk`) |
| `keys(screen, pane) -> Vec<Binding>` | The `x` menu for the focused pane (2.7) | on `x` / `?` |
| `menu_key_context(scope, pane, key) -> Option<String>` | Optional opaque identity for a target-sensitive own key; changed or missing identity blocks stale `x` replay | when opening `x` and choosing its guarded row |
| `sections() -> Vec<SectionSpec>` | `{id, name, group}`: settings sections, listed under `group` | start-up |
| `section_icon(section)` | The icon before the section's name in the settings navigation | each frame |
| `section_hints(section)` | Like `hints`, while capturing in a section | while capturing |
| `section_keys(section)` | Like `keys`, for the selected row | on `x` in the section |
| `setup_step() -> Option<SetupStep>` | Its step in setup (`radar setup`, and the first run): `{id, label, icon, section, about, summary, group}`. The card is the named section of its own, so setup and Settings edit the same fields; `summary` is the one-line result on the rail and the ready screen; `group` lists it under this machine, services (the default) or agents | setup's chooser, rail and ready screen, each frame |
| `setup_finds() -> SetupFinds` | Where setup may look for its keys once the user said yes: variable names (exact, or a prefix ending in `_`) and sign-in files under the home folder. Only names and existence are read (`setup::scan`) | when the user says yes to the look |
| `start_facts(now) -> Vec<(&str, Value)>` | Facts from your cache for the start line (`radar_core::start_lines`), each by the name a line's `{placeholder}` or `when` uses in the shipped dictionary (`crates/radar-core/assets/start-lines.toml`: `mine.open`, `reviews.count`). Never fetch for a line | once at start, as the splash rolls its line (`Host::start_facts`) |
| `typing(menu, text)` | What you say of the text typed into a menu's field: a verdict and offers | as the user types |
| `on_menu(menu, value)` | The user chose `value` in the menu you opened as `menu` | after `Effect::Menu` |
| `cli()` / `run_cli(&ArgMatches)` | Subcommands under `radar <id>` (1.10) | `radar <id> ...` |
| `name()` | The part's name for people: the parts section, the heading over its actions in others' menus. Default: the id | each frame |
| `set_up() -> Setup` | `Ready`, or `Needed("what is missing")`: then its screen is the host's set-up page and it adds nothing anywhere (1.13) | each frame |
| `owns() -> Vec<Owned>` | The subject kinds it owns, each with a plural name and whether it has a page for one (`Owned::page("ticket", "tickets")`, `Owned::link(..)`) | when routing, drawing, the parts section |
| `recognise(text)` | Subjects of its kinds in text, as byte ranges (a link, a key) | `cx.parts.recognise`, and on `Show` of a bare link |
| `show(&Subject) -> Showing` | Open one of its subjects: `Page { screen, effects }`, `Done(effects)` (e.g. opened its link), or `Cannot` | `Effect::Show` from any part |
| `depth(screen)` | How many pages deep its screen is (0: its list); the cross-part back stack uses it | after a show, on `esc` |
| `subject(screen, pane)` | What the focused pane is about, for the actions others add to its `x` menu | on `x`, and on a key it did not use |
| `offers()` / `adds()` | In words, for the parts section: the places it offers others, and what it adds to them | the parts section |
| `contribute(point, &Subject) -> Option<Contribution>` | Fill another part's place on a subject, from your own cache. Never fetch here | while the owner draws, and on `x` |
| `act(&Subject, id)` | One of the actions you contributed was chosen | from `x`, or its key on the owner's screen |

The trait has more members with defaults, each documented in `plugin.rs`: folding
(`folds`, `fold_keys`, `section_folds`), `health`, `on_splash` (what the splash shows for
the part), `refresh_now`, item-page tabs (`tabs`, `show_tab`, 1.5), `related` and
`act_with_related` (1.13), the action journal (`journal_subject`, `subject_from_journal`,
`subject_menu_context`, `run_cli_observed`), and the agent dock (`dock`, `dock_tabs`,
`dock_visible`, `dock_intent`, `agent_context`, `agent_brief`, `review_*`, `save_note`,
`resolve_change`, `ensure_worktree`), which only the sessions part fills.

Minimal plugin (compiles against `radar_core`):

```rust
use radar_core::plugin::{Outcome, Pane, ScreenSpec};
use radar_core::view::{Intent, Node, PaneId, PaneView, ScreenView, State, ViewCx};
use radar_core::Plugin;

#[derive(Default)]
pub struct HelloPlugin { greeting: String }

impl Plugin for HelloPlugin {
    fn id(&self) -> &'static str { "hello" }
    fn configure(&mut self, t: Option<&toml::Value>) -> anyhow::Result<()> {
        self.greeting = t.and_then(|t| t.get("greeting")?.as_str()).unwrap_or("hello").into();
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        let mut t = toml::Table::new();
        t.insert("greeting".into(), self.greeting.clone().into());
        Some(t.into())
    }
    fn screens(&self) -> Vec<ScreenSpec> {
        vec![ScreenSpec { id: "main", name: "hello", key: 'E' }]
    }
    fn view(&self, scope: &str, _: &ViewCx<'_>) -> Option<ScreenView> {
        let body = Node::State { state: State::Empty, text: self.greeting.clone() };
        Some(ScreenView::new(scope, vec![PaneView::new(PaneId::Middle, "greeting", body)]))
    }
    fn intent(&mut self, _: &str, _: Pane, _: &Intent) -> Outcome { Outcome::Ignored }
}
```

Register it in `src/lib.rs` behind its feature (1.14).

### 1.4 Keys: who gets what

Before ordinary key routing, notice dismissal observes the physical key without
consuming it. `~` opens the session-only recent-notices view when no menu, modal,
editor, setup, tour or focused dock owns input. That inert view uses `j`/`k` for
selection, PageUp/PageDown for full-detail scrolling and Escape or `~` to return;
`H` still opens the durable action journal. Background confirmations and editors
take ownership immediately. A notice click dismisses its captured occurrence ID
and cannot activate the covered control.

`Host::on_key` (`host.rs`) decides, in this order:

1. `ctrl-c` quits from anywhere, even while typing or with a menu open.
2. A menu is open: the menu takes the key.
3. The colours section is capturing, or the target plugin `capturing()`: the key goes to
   it, nothing else sees it.
4. `ctrl-p` opens the cross-part command palette when no editor or question owns input,
   including from History and Settings. Any other ctrl key goes to the plugin.
5. **Host keys**, never seen by a plugin: `q` quit (settings save themselves, so it asks
   only after a save failed: try saving again and quit, quit without saving, or stay;
   save-and-quit quits only after the save succeeded: `QUIT_MENU`, `quit_after_save` in
   `host.rs`); `x` and `?` the actions menu; `b` the
   bulk menu; `w` write the config now (harmless; it is saved after each change anyway); `z`
   pause or resume every part's refreshing; `` ` `` the agent dock (`dock::KEY`), when a part
   has one; `H` read-only activity history (`HISTORY_KEY`); `~` session notices; `,` settings (`SETTINGS_KEY`); a Shift+letter that is a screen's key;
   `tab`/`shift-tab` next/previous pane; `h`/`l` previous/next pane; `1`..`9` the
   pane at that position. Unmodified `←`/`→` navigate a focused tree; outside trees
   they move panes. Past a screen's outer pane `h l` go to the screen beside;
   past an item page's, to the tab beside (`Plugin::tabs`, 1.5), as `tab` does on a page
   with one pane.
6. Settings with the nav focused: `j`/`k`/`↑`/`↓` move between sections.
7. The key goes to the plugin as `intent(scope, pane, &Intent::Key { .. })`. If it answers `Ignored`: `⏎` on
   `Nav` focuses the next pane, `esc` focuses the previous, and `↑` (or `k`) past the top
   gives the keys to the top bar on a screen's panes (2.2) or to the page's tab row on an
   item page (1.5).

So a plugin cannot bind `h l q x ? b w z H ~ ,` or `` ` ``, a screen's capital, the arrows
left/right, `tab` or digits, and every other plain key is free on its own screen (a
contributed action has a shorter list, 1.13). `Effect::Focus(pane)` moves focus from a handler.

Capture hints follow the same ready-target boundary as key delivery. An unready
setup screen shows its normal controls even when its part holds an unfinished editor;
it must not advertise hidden editor actions that the host cannot deliver there.
Settings sections retain their own capture hints, including Slack kept-reply recovery
without credentials, and ready-screen editors keep their existing hints and routing.

The command palette searches cached destinations across configured parts, with the
selected item's actions first on an empty query, then session-frequent places.
Typing and arrow selection never navigate; Enter or clicking a result executes it
through normal action/confirmation or Show routing. Escape returns unchanged,
including to suspended History. Existing draft fields, questions and agent terminal
input keep their ownership; finish or cancel them first. Search highlights matching
letters, places each shortcut on the right and reports matching versus total entries.
Discovery shows a short list; a nonempty search carries up to 256 results and reports
the full match count. Frequency is in memory for the session, not stored on disk.
Parts implement `Plugin::palette_targets` using local cache only, with exact scoped
subjects and safe search words. The host rechecks readiness and the complete scoped
subject before showing it; removed targets never fall back to opening a browser.

### 1.5 Panes, focus, mouse

The universal layout is three panes, `Pane::{Nav, Middle, Detail}`: say **nav**, **middle**
and **detail** in code, comments, docs and on screen, not rail, tree, preview, side panel or
left/right pane (a *tree* is the widget that may fill the middle pane, a *preview* the card
that may fill the detail pane). A screen may draw them
however it likes (GitLab's Repositories is nav │ middle │ detail; at narrow widths the
renderer shows only the focused one). `Host.focus` is the focused pane; start-up focus is
`Middle`. Settings is nav │ section │ detail at 120 columns and wider; below that, the
section and detail stack beside the nav. All three are in `panes()`. Detail takes
`↑↓ j k Home End`; `PgUp/PgDn` scroll it from any settings pane. Changing the
selected field or section resets it; changing focus or receiving a check update does not.

**An item page's tab row is a bar of its own, the host's.** A part with a page open
returns its tabs from `Plugin::tabs` (`PageTabs { names, on }`, the names without their
icons or counts) and shows one on `Plugin::show_tab`; while a page is open `panes()` is the page's
own panes (one, or the Changes tab's files beside its diff). `← → h l` past the page's
outer pane show the tab beside (landing on the new tab's first pane going right, its last
going left), `tab`/`shift-tab` do the same where the page has one pane, and `↑` (or `k`)
past the top of the body gives the row the keys (`Host.tabs`, `Cx::tabs`): the part draws
the row focused (`PageFrame::focused`, the tab showing as the top bar's does), no pane
draws focused, `← → h l [ ] tab` move along the row and `↓ j ⏎ esc` go back into the
body; the status line says `←→ tabs · ↓ enter` and the `x` menu names the tabs either
side. A click anywhere takes the keys back. A page's own `[`/`]` keep working from the
body. A page with one tab has no row to move across. For this to work a part reports `↑`
not handled at the top of a page's list or text (`move_in`, `Scroll::key`, the kit's
lists and trees do). `h l tab` remain host pane keys; a focused tree takes `← →`
for its own hierarchy, while other panes retain arrow pane navigation. *Why:* a page's tabs are reached by the same movement keys as a screen's
panes.

Mouse: what can be pointed at is a `Target` in the view data, a part's (rows, cells,
tabs, action lines) and the host's (`chrome:*`: tabs, the mode, source tabs, settings
rows, menu rows, status-line keys). Ink lays the panes out, records the area of each
target it draws (the sizes are in 2.5; later areas win, so the pane first and its rows
after) and finds the target under the pointer itself. What is under the pointer is lit and
the pointer is a hand there (2.5). A press reaches the part as `Intent::Select`/`Activate`
on its target and the chrome as `Intent::Activate` on its `chrome:*` target. A click
on a row focuses its pane and selects the row at once; a click on a pane's empty space or
border **only focuses it** (its cursor stays). Two presses on the same area within 400 ms
behave as `⏎`. The wheel focuses the pane under the pointer and sends it `j`/`k`; over
nothing scrollable it does nothing. Ink sends
`Intent::Select` on `pane:<id>` for a press on a pane's empty space and before the wheel's
`j`/`k`; the host focuses that pane and no part sees it. The agent dock: the wheel
over it scrolls its transcript and moves no focus, the wheel over a pane never types into
the composer, the dock's keys (`↑ ↓ PgUp PgDn Home End` included) never reach the pane
under it, a press outside it takes the keys from it, and no pane draws focused while it has
them. Its transcript follows new output only while it is at its foot. Tabs
and settings sections act on one click. Right-click sends `Intent::Context` on the
target and opens its `x` actions at the pointer: a cell that opens a link, starts a
comment or dispatches an action must not do that on `Context`. Return `Ignored` for
cells without row-specific actions; the host does not offer another row's actions
there. The terminal must pass right-button events through.
The runner detects its terminal stack (`radar terminal`). In a Herdr pane, it asks
Herdr to forward right-clicks to that pane after enabling mouse reporting; the
request runs asynchronously so startup does not wait for Herdr. A failed request
appears on the status line. The top bar shows the host or Herdr routing state;
General → parts shows the full stack and routing result. tmux forwards the right button while radar requests
mouse reporting. Foot also forwards it; radar identifies Foot by its default `TERM=foot`
or `foot-direct`, since Foot clears `TERM_PROGRAM` and provides no separate identity
variable. A custom `TERM` can make Foot appear unidentified. Other terminal hosts
decide whether to pass the button through.
Another right-click replaces the open `x` menu with the clicked row's actions (or
dismisses it over empty space). Other menus remain modal. With a menu open the left
mouse button belongs to the menu. Ink keeps the left button: a press
reaches the host as an intent when it is let go without moving; a drag selects text as a
terminal would (the cells shown reversed) and letting go copies it, through
the same ways as `Effect::Copy`. Parts do nothing for this. Pointer moves (no button down)
stay in Ink, which lights the target under the pointer without a round trip to the host. In
tests send the intent a click would send (`Intent::Select`, `Activate` or `Context` on a
target, as `host_ink_intent_tests` in radar-core does); live, send SGR mouse sequences
through tmux (`ink-live click`, 3.4).

### 1.6 The bus

`Effect::Emit { topic: &'static str, payload: serde_json::Value }` is delivered to
`on_bus` of **every other** plugin (never back to the sender), depth-limited to 8
(`BUS_DEPTH`). Name topics `<plugin>.<event>`, with a JSON object payload that stays
stable, because other plugins parse it.

| Topic | Emitted when | Payload |
|---|---|---|
| `gitlab.selected` | the tree selection moves onto a merge request (`screen::moved`, built by `GitlabPlugin::selected_payload`, also after go to); nothing is emitted when a repo or tenancy is selected | `{repo, iid, title, branch, url, state}`; `state` is the role word (`review`, …) of an open one, or `merged` / `closed` |
| `workspace.start` | radar-workspace → radar-git, when `s` on a ticket has its repos picked (radar-git hears it and asks once) | `{ticket: {key, title, type}, workspace, repos: [{dir, repo, how}]}`; `how` is `checkout` or `worktree`; only the picked repos are listed |
| `git.started` | radar-git → all, when the run of the one start-work question ends | `{ticket, results: [{dir, repo, branch, base, worktree, stash, ok, why}]}`; `worktree` and `stash` are empty when unused; a repo after a refusal has `ok: false` and `why: "not run: stopped at <repo>"` |
| `workspace.started` | radar-workspace → all, after `git.started` has filled the README's `## Branches` table and the editor or terminal is opened | `{ticket, workspace, notes, repos}`; `notes` is the README's path and `repos` is the `git.started` results |

Add every new topic to this table.

### 1.6a Action history

The runner records a local SQLite action journal; the host only collects pure records.
Plugins never open the database. Use `Effect::Journal` with a stable semantic
`ActionDescriptor` before work, and `Effect::Report` when an outcome is actually known.
For an independently observed background read, use `Effect::SpawnAction { action, task }`:
only that task and its reply inherit the semantic action, not later sibling effects.
Use `Effect::Observe { action, outcome }` for a scoped, synchronous observation such as
a suppressed duplicate or count-only result; it does not complete the enclosing action.
Do not turn an opaque callback or an error string into assumed remote success/failure.
Use scoped, validated subject identities; never record input text, payloads, config,
clipboard contents, tokens or raw errors. Test hosts leave recording off by default.
A new event type keeps the journal's schema and query versions (`crates/radar-core/src/journal.rs`).

The work map (`work_api.rs`) adds explicit durable items, typed item and action tags,
and relations backed by evidence. Its storage changes are validated and idempotent and
run only at the I/O boundary; no reducer or plugin opens SQLite. A subject's locator is
not a permanent identity, and a tag implies no cause or permission. Nothing registers
items or tags them on its own. The `radar work` CLI (`work_cli.rs`) runs commands the
runner owns and acknowledges; even its queries record that they ran and may create or
migrate the local journal, so none of them is a read that writes nothing.
`radar work events --stdin` discovers bounded original source evidence by exact
scoped filters, including unbound history. It preserves the original request
sequence and explicit outcomes; discovery never creates an identity or binding.

### 1.6b The history overlay

The host owns `H` (recent activity, the last 24 hours) and the History entries in `x`,
all recorded activity included. Text being typed comes first, so `H` in an editor never
opens history. Parts give the subject of what is selected through their `subject` and
`journal_subject` hooks; a menu captures that subject when it opens, and `esc` gives back
the view and focus under it.

History's reducer and renderer do no I/O: the journal's worker answers each request with
a query bounded by a snapshot, one page of events at a time, and a refresh starts a new
snapshot. The reader records nothing about itself and infers no outcome. While it is
open, tasks still report to their parts. Details show only the journal fields on its
allowlist.

### 1.7 Cache, config, settings

- **Config file**: `$XDG_CONFIG_HOME/radar/config.toml`, else `~/.config/radar/config.toml` on every platform
  (`runner::config_home`; an existing file in the old macOS folder is still read). One table per
  plugin by `id()` plus `[theme]`. **Saved by the host after each completed change**
  (`Host::autosave`): when an event changed the config (it is compared before and after
  each event) and nothing is being typed (no plugin `capturing()`, no colour being
  typed), the host returns `HostEffect::Save`; a change made while typing is saved when
  the typing ends. So a field kept with `⏎`, a choice in a
  menu, a row added, removed or toggled is written at once; a field being typed is not,
  until it is kept. A part does nothing for this: change `self.cfg` in a handler and it is
  saved; do not say "w saves". The runner writes it atomically (a temp file beside it,
  mode 0600 on Unix, then renamed over it; a symlinked config stays a link) through
  `runner::write_atomic`; a typed-in token lives in it, so never log it. A failed save
  says so on the status line and keeps the "unsaved" marker (`Host::unsaved()`) until a
  save succeeds; `w` writes now and tries again.
- **Cache**: rows of `<state>/radar/cache.sqlite3` (`crates/radar-core/src/plugin_cache.rs`;
  not the journal, which never holds cache blobs). Return `Effect::PersistCache` when fresh
  data is worth keeping; `snapshot()` supplies it, and the cache's own thread stores it, off
  the UI thread. An object snapshot is stored one row per top-level member and only changed
  members are rewritten, so put parts that refresh on their own (a site, an account) at the
  top level. An old `<id>.json` is imported once and renamed `<id>.json.imported`; without
  the database the JSON file is read and written as before. `XDG_CONFIG_HOME` and
  `XDG_STATE_HOME` redirect both on every platform (`runner::config_home`,
  `system::state_home`); use that in live checks.
- **Settings sections**: `sections()` lists `SectionSpec{id, name, group}`; the host shows a
  tree of groups (uppercase headings, no fold markers; `ChromeView::settings`, the
  `SettingsNav` Ink paints) │ your `section_view`'s body │ its detail. `name` is unique under its group and is the nav row and panel
  title; a name two groups share is titled with the group's name in front ("Jira
  connection", `Host::section_title`). A part's first section is its `connection`
  (2.11a). Keep the selected row of each section in the plugin
  (`GitlabPlugin::section_rows`). The host adds its own `colours`, `parts`, `mode` and
  `icons` sections under `general`.
- Every key of your config table must be editable in a section, and every section row
  must map to a config key or a visible check, so nothing needs editing by hand.
- **Diagnostic log** (`crates/radar-core/src/log.rs`): a TUI cannot print to the terminal it
  draws on, so radar writes what it did and why to `<state dir>/radar/radar.log`
  (`~/.local/state/radar/radar.log` on Linux; `XDG_STATE_HOME` moves it). Past 256 KB the file
  moves to `radar.log.1` and a new one starts. The runner logs start-up, the config and cache
  it read, saves, opens and copies that failed, and panics (its panic hook gives the
  terminal back first, then logs the message and a backtrace). Plugins call
  `radar_core::log::{info, warn, error}` and `debug(|| ..)` (kept only with `RADAR_DEBUG=1`);
  log from inside a spawned task, where the I/O already is, not from a handler. Every line
  goes through `log::redact`: token shapes (`glpat-…`, `Bearer …`, `token=…`) and anything
  registered with `log::secret(value)` become `[redacted]`. Register every token you
  resolve (`auth::Configured::token` does). Nothing is written before `log::init`, so tests
  stay quiet.

### 1.8 Theme slots

Nothing visual is hard-coded. The theme (`crates/radar-core/src/theme.rs`) is `[theme]`:
`palette` (any Opaline palette, default `silkcircuit-neon`), `colors`
(`auto|truecolor|256|16|none`), `hyperlinks` and `images` (`auto|on|off`, 1.16; written
only when not `auto`), `pointer` (`auto|on|off`, the mouse pointer's shape, 2.5; written
only when not `auto`), `motion` (1.17), `icons` (the icon pack, 1.18; written only when
not `auto`) and `slots` (`name = "#rrggbb"` overrides of any slot). The host's colours
section edits all of them (links and pictures under `terminal`). Icon overrides are a
table of their own, `[icons]`, edited in `General › icons`.

- A plugin **declares** domain slots in `slots()`:
  `("jira.blue".into(), SlotValue::new((0x82,0xaa,0xff), 111, Color::Blue))`: truecolor,
  xterm-256 index, ANSI-16 colour. This is the only place a plugin writes colour values
  (`TENANT_SLOTS` in `crates/radar-gitlab/src/lib.rs`). Prefix names with your plugin id;
  `slot_info` groups them under "plugins" and labels `owner rest`. The host does not
  enforce the prefix, only uniqueness.
- A plugin **uses** a slot through `Theme`: `theme.s(Slot::TextMuted)` for core slots,
  `theme.style("jira.blue")` for a domain slot, `theme.spec("jira.blue bold")` for a style
  spec (slot name then `bold|italic|underline|dim|reversed|strike`), and
  `widgets::tone(theme, Tone::Warn)` for the feeling of a count.
- A plugin **lets the user choose** which slot a word uses through its own config table:
  `[gitlab.style] review = "status.warning bold"` (`config::default_styles`,
  `Settings::style_for`), edited in its `look` section; the host's `colours` section edits
  the theme itself. Do not mix the two: "theme" is the host's palette and colours; "look"
  is a plugin's choice of slot per word.
- `Theme::new(Tier::Mono)` is what snapshots render with: no colour, so a snapshot cannot
  prove a colour. Check colour live (3.4).
- Gap: a plugin cannot give its slots a plain-words label or meaning; `slot_info` shows
  "declared by a plugin".

### 1.9 Menus, confirm dialogs, text input, the system module

- **Menu** (`Effect::Menu(Menu::new(id, title, items))`): lazydocker's context menu, with
  a key column, a label and a line saying what it does. `MenuItem::new(key, label, hint,
  value)` is chosen by its key; `MenuItem::pick(..)` by moving; `MenuItem::heading(..)`
  groups; `MenuItem::note(..)` is a line drawn as written and never chosen (a confirm
  shows with them exactly what it will send); `.sample(spec)` draws a swatch. `⏎` or the item key chooses, `j`/`k`/arrows move,
  `esc` or `q` closes. Do not give an item the key `q` (it closes the menu) or `j`/`k`
  (they would shadow movement). Slack kept replies uses `s` for "keep current draft",
  leaving `j`/`k` navigation unable to move text out of the composer. The choice comes
  back as `on_menu(menu_id, value)`; a
  menu taller than the screen scrolls. Long labels, explanations, headings and refusal
  reasons wrap to the available width. A scrollbar shows when more remains;
  `PgUp`/`PgDn` and the wheel read overflowing content without choosing an action.
  An action list opens with its selected action visible, so paging up may reveal an
  earlier long title. Typed menus keep their field visible above their offers.
  A part can opt a target-sensitive own key into `Plugin::menu_key_context`: return
  a cheap opaque token for its exact target and connection revision. The host checks
  its original route and token before replay; a changed or missing context asks the
  user to reopen actions. Avoid general refresh/loading state so unchanged targets
  remain usable. Tokens stay in memory, are not displayed or journalled, and must
  not contain credentials. Direct keys and plugin-owned confirmations/editors keep
  their existing behavior.
- **Confirm**: a question is a dialog, not a list. The question is the title (after the
  warning icon), what it will do or send are the lines under it, then a blank line and
  its choices as raised buttons in a row against the right, each the key and what choosing
  it does (`y post it   e keep writing   n drop it`). A button is three lines tall: a
  light edge, the face, a dark edge (a bevel, `widgets::buttons`); the highlighted one is
  filled with the accent, the others with the selection's grey, and without colour the
  edges alone draw it. `y` is highlighted first. The key answers; `←`/`→` (`h`/`l`, `tab`, `j`/`k`
  too) move along the buttons, `⏎` chooses the highlighted one, `esc` closes, a click
  on a button chooses it; a button that does not fit beside the others starts a new row.
  A question starts at its first words, with choices pinned below while the text
  scrolls; reading never changes the selected answer. Long button captions wrap;
  short terminals can omit the bevel, and terminals too small for usable controls
  show a resize message rather than accepting an invisible choice.
  `Menu::confirm(id, question, yes, no)`: `y` answers `"yes"`, `n` answers `"no"`; `yes`
  and `no` are the buttons' words, a verb phrase saying what the choice does ("delete
  it", "keep it", "check it out"), never a sentence; what will happen is a line before
  them (`.saying("it leaves the nav pane")`). Use it before anything destructive on this
  machine and, after a setting the service can confirm, to ask "check ... now?"
  (each part has its own small `settings::ask_to_check`). A write to a service asks with the write confirms instead
  (2.23): `Menu::write_confirm(id, question, text, yes, no)` shows the whole text as
  lines (cut at 72 cells, never reworded) and offers `y` (`"yes"`), `e` keep writing
  (`"edit"`) and `n` (`"no"`); `Menu::write_plain(id, question, notes, yes, no)` is the
  one without text. A write confirm adds (`Change::Add`) and a plain one edits
  (`Change::Edit`) unless it says otherwise (`.adds()`, `.edits()`); the host adds the
  mode's note as the last line before the buttons when the mode refuses it
  (`.noting(note)`, `radar_core::writes`, 2.23). All three set `Menu::question`, which is
  what draws a menu as a dialog.
- **As view data**: the open menu or question reaches Ink as
  `view::MenuView` (in `Host::chrome_view`, or `Host::menu_view`): its id, title (the
  question, which may run to several lines), icon, whether it is a question, the write it
  is before, each row as a `Choice` (value, label, hint, key and the key column's words,
  icon, heading or note, muted, a sample's look, and a target `chrome:menu:<i>` for the
  rows that can be chosen), the highlighted row, the field's text and caret, why the part
  refuses the text, where a context click opened it and how far it is scrolled. The
  Ink lays it out and paints it from that alone; the host's
  viewport only takes what Ink measured, for the paging keys. `Intent::Invoke`
  with a choice's value answers it, `Intent::Activate`/`Select` on a row's target chooses
  or highlights it.
- **Text input**: always `radar_core::input::TextInput`. `TextInput::new(value)`,
  `key(KeyEvent) -> Edit::{Submit(text), Cancel, Typing}`, `paste(&str)`,
  `value()`, and `render(theme, row_rect, buf, cx.cursor)`, which draws one row scrolled to
  keep the cursor visible and sets the terminal cursor. Keys: ← → home end, word jumps,
  backspace/delete, `ctrl-u` clear to start, `ctrl-k` to end, `ctrl-w` word, `ctrl-a`/
  `ctrl-e`, `ctrl-z` undo, `ctrl-r` redo. Return `true` from `capturing()` while one is
  open and implement `paste`.
- **widgets** (`crates/radar-core/src/widgets.rs`): `panel` (rounded, titled, accent border
  when focused), `heading`, `chip`, `row`/`row_style` (selected rows; an unfocused pane
  keeps a visible cursor, underlined in mono), `kv`, `empty`, `paragraph`, `padded`,
  `scroll_offset`, `step`, `vertical`, `tone`. These are the small pieces; whole elements
  (a banded tree, a table, a page frame, a card) are the kit's (1.15).
- **System** (`crates/radar-core/src/system.rs`): never start a browser or clipboard program
  yourself. Return `Effect::Open(url)` or `Effect::Copy(text)`. The runner calls
  `Env::detect` (ssh, macOS, Windows, WSL, Wayland, X11, tmux, `$BROWSER`), tries
  `open_ways`/`copy_ways` in order and falls back to the user's own clipboard through the
  terminal (OSC 52, wrapped for tmux), then says what happened on the status line. Also
  return `Effect::Notify("link copied")` after `Copy`, because the runner stays silent when
  a program did it.
- **An app's own link** (`Effect::OpenApp { link, web }`): `link` (`app://…`) goes only to
  the desktop's opener (`system::app_ways`: `xdg-open`/`gio`, `open`, `start`, `wslview`),
  never to a browser or the clipboard; with no desktop, over SSH, or when nothing takes the
  scheme, `web` opens as `Open` would. Whether the app is worth trying (is it running?) is
  the part's to find out, in a task.
- **A folder** (`system::folder_ways(env, dir, InFolder::Terminal | InFolder::Editor(cmd),
  terminal, name)`): how to open a terminal in a folder or an editor on it, best first,
  plus `by_hand` (the command line that does it, e.g. `cd '/x/api' && nvim .`) and
  `why_not` when nothing here can reach the user. `start_in(starts, dir)` tries them off
  the event loop (inside an `Effect::Spawn`, through `tokio::task::spawn_blocking`); when
  none can or none works, return `Effect::Copy(by_hand)` and say why. `installed_editors`,
  `on_path`, `runs_in_terminal` and `split_command` are there too. What each environment
  gets is the table in 2.22.

### 1.10 The CLI hook

`cli()` returns a `clap::Command` (the runner renames it to your id); `run_cli(&ArgMatches)`
returns a `CliTask` resolving to the text to print. Mirror what the TUI shows and offer
`--json`: `radar gitlab list [--view NAME] [--filter TEXT] [--json]`. The runner's
own commands are `radar config path|show`, `radar icons list|export|path`, `radar setup
[STEP]` (the TUI, opened on setup), `radar terminal`, `radar licenses`, the audited `radar work` commands (1.6a), and `radar mcp`, the relay an agent CLI
starts (`src/main.rs`); a part's id may not be one of them.
`run_cli` takes `&self`: copy what you need out
of `self` before the `async` block, as `GitlabPlugin::run_cli` does.

### 1.11 What the GitLab plugin is, for reference

`crates/radar-gitlab/src`: `lib.rs` (the plugin, bus, pause, author pick, tests), `config.rs`
(the `[gitlab]` table: `url`, `token` as `env:`/`glab`/literal, `base`, `refresh_secs`,
`idle_minutes`, `history_days`, `style`, `source` tenancies, `view`s, `side_by_side`,
`hide_files`), `client.rs`
(GraphQL: a tenancy's merge requests and projects, a path's check; REST for jobs, logs and
file diffs; `graphql` and `write` (sent once) for the requests that change something;
every request goes through a `Transport`, so tests play recorded replies, `client::recorded`), `auth.rs` (a token per request, renewed once from
glab on a 401), `sync.rs` (what each sync asks for, and folding the answer in), `model.rs`
(`Mr`, `MrState`: open, merged, closed; `Role`: review, fix, ready, waiting, draft, team,
bot), `ready.rs` (what stands between a merge request and merging, its STATE: GitLab's
`detailedMergeStatus` leads, its `mergeabilityChecks` give the rest, weighed by the
repo's settings; the role follows it: `fix` when what is in the way is the author's to
do), `filter.rs` (the `/` grammar and its plain-words description), `screen.rs` (nav │
middle │ detail), `goto.rs` (go to), `settings.rs`
(sections `connection`, `tenancies`, `views`, `look`, each with detail pane, live
validation and checks), `pages/` (item pages, 1.12; `pages/act.rs` their actions;
`pages/job.rs` a job's log), `why.rs` (why a pipeline is red: failed jobs and their logs,
fetched and drawn), `joblog.rs` (a job's log read: the runner's timestamps and section
markers, the error patterns, the excerpt, `failure_reason` in words; plain functions),
`mr_actions.rs` (rebase and merge: requests built and read as plain functions),
`list_actions.rs` (the same actions and approve from a merge request's row in the middle pane: keys,
`x` rows, the detail pane's clickable list (with the actions other parts add, a click
pressing their key), STATE clicks, status-line keys; it asks through `pages/act.rs` and
`pages/review.rs`, so the questions, the mode's word and the progress are the page's),
`write.rs` (comments, replies, resolving, approving: each a `Change` with its REST
request and whether it adds or edits, asked first with the kit's write confirm, sent once),
`pages/changes.rs` (the Changes tab: files, diffs fetched as they open, threads under
lines), `pages/review.rs` (the Threads tab's list and the `a`/`c`/`d` keys). Copy its
shape, not its details.

Local checkouts are not GitLab's: they are the git part's (`crates/radar-git`, 1.11a).

### 1.11a The git part

`crates/radar-git/src`: `local.rs` (checkouts: the `[git]` table, the scan, the index,
`git` checkout), `settings.rs` (sections `checkouts`, `editors`, `links` and `branches`
under the group `git`) and `join.rs` (what it adds to `repo` and `merge-request` subjects, whoever hosts
them: the line under the title, `checkout  ~/code/api · java · on main` or why there is
none; and the actions `c` check out the merge request's branch (asked first), `e` an
editor, `t` a terminal, `s` look again). It is registered last, so a key another part
already gives the same subject stays that part's.

It also has (`crates/radar-git/src`):

- **Links** (`[[git.link]]`, `repo`, `dir`, `branch`, `base`): a link wins over a clone
  found by origin. Set from the repo's `link to a local folder…` (an `x` row: a field,
  `Menu::typed`, to type or paste the folder in, offering this repo's clones found and the
  folders the typed path goes on to, each said for what it is, and refusing a folder that
  is not a clone), or in Settings › git › links. Worktrees are grouped under their
  clone (`branch.rs`, `local.rs`); the normal checkout is the one acted on, and
  `use another worktree…` picks another (`Enter` on a worktree in the Branches screen
  does the same).
- **Branch conventions** (`[git] me`, `branch`, `worktree`; `[[git.convention]]` with
  `under`, `branch`, `base`; `[git.types]`): rendered by `branch::render`, with the
  refusals of `branch::problem` and `valid_ref`. The base is the link's, else the
  convention's, else origin/HEAD, else `main`. Settings › git › branches shows a live
  sample. Its templates take the variables `{me}`, `{key}`, `{project}`, `{num}`,
  `{slug}` and `{type}`; a template without `{key}` is refused.
- **Actions** on a repo and a merge request:
  - `n` new branch…: the name comes from the convention and the ticket key found on the
    page; when `me` is not set, it says so and names nothing;
  - `u` rebase on the base, on a repo page (on a merge request page `u` is GitLab's
    rebase, so git's is the keyless `rebase on <base>…`);
  - x rows, keyless: `push <branch>…` (a forced push after a rebase says so; push is a
    write and is refused in read-only mode), `park on a worktree…` (beside its clone),
    `link to a local folder…` (`link to another local folder…` once one is), `use
    another worktree…`.

  Every question is asked first and says what would be lost. A dirty checkout offers
  `s stash it as KEY-local-<date>` (the default), `w park on a worktree instead`, or
  `n leave it`. A branch that exists defaults to `s switch to it`; `r recreate it off
  fresh <base>` says how many commits on no remote are lost. Local git (stash, branch,
  rebase, park) runs in every mode; only push is a service-facing write and asks the mode.
  Answering `n` runs nothing.
- **Start work** (`workspace.start` → `git.started`, see 1.6): one question lists every
  repo's steps, including the `after_worktree` command a parked repo runs. A refusal stops
  the repos after it, and they are reported as not run.
- **The Branches screen** (`B`, `screen.rs`): a nav pane (All, Dirty, Ahead of remote,
  Stashes), a middle pane (a tree: clone → its worktrees → its local branches, with ahead and behind,
  dirty, stashes, and the ticket a branch carries as a `checkout` column for other parts),
  and a detail pane. `d` deletes a branch or worktree: it asks and says what is lost, and it
  refuses the current branch. Prune is in the screen's bulk list as `p prune merged and
  gone`: merged or gone clean ones only, asked once, then `after_worktree` runs. `r`
  refreshes it. A failed or interrupted status read shows unknown or explicitly
  last-known counts, never a fresh clean result; the Dirty view retains unread
  worktrees. Removal refuses unread status, including a question already open when
  the read failed, and prune reports how many unread worktrees it skips. Only a
  complete read advances the whole-screen sync time. The sample for the snapshots
  is in `demo.rs` (two clones, a worktree). Worktree removal questions retain their
  original target and observed risk. A changed status, dirty-file count, branch,
  short HEAD or clone scope requires reviewing removal again; an old clean approval
  cannot become a forced removal. Each question can dispatch at most once. These
  checks validate observed counts and the existing short HEAD, not dirty-content
  identity or execution-time filesystem races.

The part owns the `checkout` kind (its folder as the id, with `repo` and `branch`).

`[git]` is `roots` (folders to look in, `~` allowed), `depth` (folders down, default 4),
`terminal` (empty: found; else a command line with `{dir}` and `{cmd}`), the branch
conventions above (`me`, `branch`, `worktree`, `after_worktree`, `[git.types]`) and
`[[git.type]]` (`name`, `markers`, `editor`): java, python, node, rust and go by
default; a checkout's type is the first whose marker file is at its top. The index
(every checkout's folder, `origin`, repo path, branch and top-level names) is in its
cache under `index`; a scan runs once at start-up and on `s`.

How it syncs, as a pattern for any service with a list that changes:

- **A tenancy is a group or one repo.** The config does not say which; the first sync
  asks GitLab (`group(fullPath)` and `project(fullPath)` in one query) and keeps the
  answer in the cache (`GroupData.repo`); the check in Settings, tenancies says which it
  found. `Settings::source_for` gives a repo tenancy its own repo, over the group above it.
- **First full, then changes** (`sync::plan`, `sync::merge`). The first sync of a tenancy
  in a session fetches every open merge request and the projects. Later ones ask only for
  merge requests updated since the newest one held, in any state (`updatedAfter`, state
  null), so one that was merged or closed is seen leaving; open ones with a running
  pipeline are asked for again by iid (a pipeline changes without touching its merge
  request); projects come again every 5 minutes. Tenancies sync most recently active
  first. `r` makes the next sync full (it also drops merge requests that were deleted).
- **Merged and closed only when asked.** They are fetched only while a view or the `/`
  filter could show them (`Filter::wants_ended`: `state:merged`, `state:all`,
  `-state:open`), back `history_days` (default 14), at most 4 pages per state. A filter
  with no `state:` term shows open ones only (`filter::state_default`).
- **Pause when nobody is there** is the host's, for every part: after `[general]
  idle_minutes` (default 30, 0 never; a part's old `idle_minutes` is read as a fallback)
  without input the host stops calling `tick` and the top bar says so; `z` pauses and
  resumes every part by hand (`paused · z resumes`). A part has no `z` handler, pause row
  or idle timer.
- **A glab token that expired** (`auth::with_token`): on a 401, when the token came from
  glab, radar runs `glab auth status` once (glab renews its login when it talks to the
  server), reads the token again and retries once. A second 401, or any 401 with an
  `env:` or typed token, is shown as it is: never a loop.
- **Main's newest commit** comes with the projects (`repository.tree.lastCommit`), shown
  in a repo's detail pane and its page's title line. It costs GitLab a lookup per project, so
  projects are fetched 50 a page and only every few minutes.

GitLab gotcha: its GraphQL returns some text HTML-escaped (a commit's title). Pass such
fields through `client::text_of()`, which unescapes them, before showing them.

### 1.11b The workspace part

`crates/radar-workspace/src`: `config.rs` (the `[workspace]` table and its
`[[workspace.space]]` entries), `find.rs` (clones and workspaces on this machine, read
without running git), `okf.rs` (the workspace's `.okf.yaml`), `notes.rs` (a ticket's
notes), `readme.rs` (the README a start makes), `start.rs` (start work), `settings.rs`
(the `workspaces` section, under the group `workspace`), `lib.rs` (the part's
contributions). It imports no other part; it meets the others only through the bus
(1.6) and subjects (1.13).

- **A workspace** is a top folder of clones:

  ```toml
  [[workspace.space]]
  dir = "~/work/acme"
  name = "acme"
  tickets = ["ABC"]          # Jira projects worked here
  repos = "repositories"     # where its clones are
  open_at = "top"            # e and t open here, so every repo is in view
  after_worktree = "python3 scripts/sync-vcs-mappings.py"
  notes = "okf"              # read .okf.yaml; or a path template
  ```

  A folder that holds other clones is offered as a found workspace, as a question in
  settings; it is never added by itself. A bare `[[workspace]]` table is refused: the
  entries are `[[workspace.space]]`. The part owns the `workspace` kind (its folder),
  which it shows by opening it in an editor.
- **A ticket's notes** are the `.md` files whose frontmatter `ticket` is the key, and
  every file in a folder named `<KEY>` or `<KEY>-…` under the knowledge paths. Where the
  workspace's `.okf.yaml` says notes go is where they are found and where new ones are
  made (`create` location); with no such file, a
  folder named by the key is the fallback. `done/` and `archive/` are included; clones
  are excluded. Each note is a `note` subject: its path is its id, its `file://` URL its
  link, and its fields are `ticket`, `type`, `status`, `title`.
- **On a ticket** (contributed to the `ticket` subject): a **Notes** tab, the README
  first and then the rest by `type`, `status` and name (`readme · README.md`, then
  `spec · draft · spec-backend.md`); a Notes section with the README's opening lines;
  `e` opens the workspace in an editor and an `x` row opens a terminal at its top.
  `look for notes again` re-reads them. Opening a note's row opens its file in the editor.
- **`s` start work on a ticket** (contributed to `ticket` subjects whose project is in a
  workspace's `tickets`): a multi-pick of the workspace's clones, with the ones ticked
  when they carry a branch with the key as a whole word, when the README or notes link
  to them, or when their frontmatter `repos:` names them. Then `how`: the normal checkout
  (the default) or a parked worktree. The pick sends `workspace.start` (1.6). When
  `git.started` comes back, the README is made from `artifacts/.template.md` or the
  plan's template (or the existing folder of the ticket), and only its `## Branches`
  table is filled, asked first when it already has rows. A start where nothing succeeded
  fills nothing, opens nothing and says the ticket was not started, with why.
  Without radar-git present, nothing answers `workspace.start`: the status line says it asked
  git, and the README is still made.

  The `workspace.started` that follows is what Jira hears (1.6, and radar-jira's start
  question, asked once per its `[jira] on_start` setting: `"ask"` (the default) asks
  "start KEY on Jira?" with the lines the change would make, such as `assignee: Bob → you`
  and `status: To Do → In Progress`; `"off"` never asks).

### 1.11c The GitHub part

GitHub is one source of the Repositories tab: with several sources the source row
switches between GitLab, GitHub and local workspaces (radar-workspace: the configured
workspaces and the clones under each, `⏎` opens a folder). `R` opens Repositories; `G`
and `W` go straight to GitHub and local workspaces. Branches and worktrees stay on the
Branches screen (`B`).

`crates/radar-github` reads only: GitHub.com over HTTPS, the repositories the config
names, with no token (public reads), a typed token or an `env:NAME` reference.

- **Local first.** What it read (repositories, open pull requests, pull request pages
  with their checks, each read's ETag and "what moved" marker) is kept in the `github`
  cache and drawn at start.
- **Each minute asks only what moved.** With a token, one GraphQL query for every
  repository's open pull requests, in batches (50 repositories of markers, 25 asked in
  full, four batches in flight at most), then a conditional REST read for each one that
  moved (a 304 does not count against the rate limit). A repository with nothing held is
  asked in full in that query (its description and up to 30 open pull requests); what the
  query cannot give exactly is read over REST, six repositories at a time. Without a
  token, one conditional read per repository.
- **Actions.** The Actions tab asks one page of the newest runs (sized to the view; on a
  pull request page, of its source branch), conditionally, and keeps them until the
  repository moves, a run is still going (a minute) or a quarter hour has passed. A run's
  page pins one run and attempt. A failed Actions read never shows as no activity, and
  the latest workflow seen is not a claim about the whole repository's health.
- **Limits.** An open pull request page is asked again only while its checks run. A
  rate-limit refusal waits for `Retry-After` or the bucket's reset; a bucket under a tenth
  left waits for its reset; 5xx and network failures back off 1, 2, 4 … 15 minutes.
- **Settings.** The connection check calls GitHub whatever repositories are configured.
  With a token, discovery offers up to five pages of the repositories it can reach,
  grouped by owner, to choose from (200 at most); a saved group may hold repositories of
  several owners. The token is masked; a typed one is saved in `config.toml`.
- **Subjects.** `repository`, `pull-request` and `workflow-run` (1.13) carry canonical
  URLs as ids, so they never collapse with another provider's. It offers `Line`,
  `Section` and `Actions`; its actions only navigate. The local parts match a clone by
  `repository_url` (and `source_repository_url` for a pull request), host and path in
  full; a pull request with no source repository known is not matched to its base.

### 1.12 Detail pages: the pattern for item views

Use this for any "open this thing" view (a Jira ticket page, an environment, a Slack
thread). The GitLab implementation is `crates/radar-gitlab/src/pages/` (`mod.rs`, `mr.rs`,
`repo.rs`, `tenancy.rs`, `tests.rs`).

- **Opening.** `⏎` on a tree row opens a page for the tenancy, repo or merge request
  (`pages::open_from_tree`); `space` alone folds. Rows on a page open the next page on `⏎`
  (tenancy → repo → merge request) or a link (a pipeline). Pages form a stack
  (`GitlabPlugin::pages: Vec<Page>`, `Kind::{Tenancy, Repo, Mr}`).
- **Layout** (`pages/neutral.rs`, the page as view data; `changes_neutral.rs` for the
  Changes tab): the page takes over the whole body: a breadcrumb line
  (`Repositories › a › api › !263`), a title line, a tab row, then the tab's
  content. Tabs are clickable.
- **Keys** (`pages::key`, given the focused pane): `[`/`]` switch tabs (wrapping), as the
  host's `← → h l` do past the page's panes, `tab` on a tab with one pane, and the tab row
  itself after `↑` (1.5); `↑↓ j k PgUp PgDn g G`
  move the cursor on a list tab and scroll a text tab (`move_in`, which reports `↑` at
  the top not handled); `⏎` opens the row (on
  a tab another part added, it shows the row's subject; on the Overview it offers the
  page's links, below);
  `o`/`p`/`y` open, open the pipeline, copy the link; `r` refreshes; `esc` pops one page
  (repo → merge request → `esc` → repo) and finally returns to the tree with the cursor
  where it was. On a merge request: `u` rebase, `m` merge (`pages/act.rs`). With the git
  part here, on a repo or merge request: `c` check out the merge request's branch, `e`
  editor, `t` terminal, `s` look for checkouts again (1.11a). The review (M2): `a`
  approves or takes it back; on the Changes tab `n`/`N` next and previous file in the
  order the tree beside the diff shows them (folders skipped); that tree is the page's
  first pane (`Nav`, the diff is `Middle`: `h l tab 1 2` move between them as
  everywhere, `panes()` says so while the tree shows; in it `j k g G` move over folders
  and files, `space` or `⏎` folds a folder, `⏎` on a file opens it, `+`/`-` fold
  (`folds`), `↑` at its top goes up to the tab row, a click on a row selects it and opens
  a file, and the `x` menu lists the files' keys there and the diff's on the diff), `}`/`{`
  next and previous hunk, `space` folds the thread on the line, `c` or `⏎` comments on the
  line or replies in its thread (a click on a line's numbers does the same, as GitLab's
  gutter does; a click elsewhere on it only moves the cursor, two are `⏎`), `v` shows the
  diff side by side or in one column (`[gitlab] side_by_side`, kept; the old file on the
  left, a removed line facing the line that replaced it, `DiffView::split` and
  `DiffState::key_split`, the cursor moving by rows and keeping its side; a panel under
  90 columns stays one column), `f` from either pane collapses the files and the diff
  takes the width, or shows them again (`[gitlab] hide_files`, kept), `d` resolves a thread or opens it again; on the Threads tab `c`
  comments on the merge request (first row) or replies, `d` resolves, `⏎` opens a thread
  at its line. On those two tabs `c` writes, so checking out is on the others.
- **Links work.** On the Overview `⏎` (the `x` menu's `follow a link on this page…`)
  offers everything the page's text links to, gathered while drawing into `Page::links`
  (`join::links_in`): the description's markdown links (named by their words), what the
  parts `recognise` in the description, in the sections other parts added and in the
  latest comment (a ticket's key, a merge request's or thread's link), and any address
  written out, once each. Choosing one is `Effect::Show`: a ticket's, merge request's or
  thread's page opens here (`esc` comes back), a plain link opens the radar way (2.34).
- **The line under the title.** A repo or merge request page draws the first line
  another part puts there (`Point::Line`, `ui::chips::words`): with the git part,
  `checkout  <folder> · <type> · on <branch>` (`✓ on <branch>` when it is the merge
  request's), or why there is none and where to set the folders.
- **Panes.** While a page is open `panes()` is `[Middle]`, or `[Nav, Middle]` on the
  Changes tab while its files show, so `tab`, `h`/`l` and the digits move within the page
  and, past its edge, across its tabs (1.5). Hook it in `Plugin` by delegation, as
  `lib.rs` does: `view`, `intent`, `keys`, `panes`, `tabs`, `show_tab`, `folds`, `bulk`
  and `busy` each check `self.pages.is_empty()` and hand over to the page code.
  The tree's bulk commands are empty while a page is open.
- **`x` menu.** `pages::keys` returns the page's own actions, titled with the item
  (`!263`, the repo, the tenancy): open, copy, refresh, scroll only if there is more, `esc`
  "back to ..."; each row must work (2.7). Repo and merge request pages group them under
  headings: `on GitLab` (rebase, merge), `page` (tabs, back); the host adds other parts'
  rows under their names (`Git`: checkout, editor, terminal, look again).
- **Data.** What the list query lacks is fetched lazily when the page opens
  (`pages::fetch`, GraphQL: a merge request's description, discussions, jobs, commits,
  files; a repo's pipelines on its default branch). Each fetch is a `Fetched<T>`
  `{value, loading, error, at}` kept in memory on the plugin (`mr_pages`, `runs`) and
  refetched once older than `refresh_secs`; a fetch already running is not repeated.
  `busy()` includes `pages::loading` so the top-bar spinner runs; the old value stays
  while a refresh runs; an error shows in the page. A merge request page's fresh copy also
  replaces the list's (`on_mr_page`).
  A refresh that fails keeps what was held: the error shows as a line above the cached
  content (even a cached empty result), is not a selectable row, moves no row's index,
  and goes on the next successful read. Each notice names the read it is about (the
  merge request's details, a repo's runs, a failed-job diagnosis), so a fetch that did
  not fail keeps its own status.
- **Tabs.** Merge request: Overview (how it ended if merged or closed, its labels, the
  description, and a "can it merge?" panel), Changes (the files as a folder tree, GitLab's order:
  folders first, a folder with one folder in it joined to it, `+added −removed` summed on
  folders, beside the open file's diff, threads under their lines; a file's diff is fetched when it opens, 20 files a
  request (`client::DIFF_PAGE`), and kept until the head commit moves), Threads (a list: comment on the merge
  request, then each thread), Pipeline, Commits.
  Repo (its title line has main's newest commit: sha, author, when): Merge requests,
  Pipelines on main (named after the default branch). Tenancy: a
  table of its repos. Descriptions are markdown laid out by the kit (`ui::markdown`):
  tables as columns, `<details>` open, comments gone; `⏎` lists the page's links to follow.
- **Why it is red.** When a merge request's pipeline failed, its Pipeline tab (`Pipeline ·
  1 failed`) leads with `WHY IT IS RED`: each failed job (the ones not allowed to fail
  first) with its stage, how long it ran, GitLab's `failure_reason` in words, and the lines
  of its log around the first error (`joblog::is_error`: what compilers, test runners,
  shells and runtimes write; else its last lines, `no error recognised`); then `ALL JOBS`
  by stage. A repo whose main pipeline failed leads its Pipelines tab with `why main is
  red` above the runs. Every job is a row; `⏎` opens its page. The failed jobs come from
  REST (`/pipelines/:id/jobs?scope[]=failed`), with the last part of the first five logs;
  the detail pane of a red merge request or repo says `✗ unit-test and 1 more failed: the
  script failed` once known, and moving onto one asks for its failed jobs (no logs).
- **A job's page** (`Kind::Job`, `pages/job.rs`): its log in the kit's log view
  (`ui::log`), opened at the first error. The runner's timestamps go, its sections fold
  under their titles (folded when GitLab says `collapsed=true` and the runner's own
  set-up and clean-up), colours become slots. Only the last 256 KB is fetched (`Range`);
  the tab says `Log · last 256 KB of 1.2 MB` and reaching the top fetches the part before.
  `/` searches (matches stand out, `n`/`N` between them, `esc` clears it before it goes
  back), `space` folds the section at the cursor, `g`/`G` start and end; `o` opens the
  job, `p` its pipeline, `y` copies its link, `c` copies the excerpt, `r` loads it again.
  A fresh tail whose text changed resets the position on every page of that job (the
  search is kept and its matches found again); the same text or a failed read keeps the
  position and folds.
- **Testing** (`pages/tests.rs`): open each page kind, every tab, `esc` back to the tree
  cursor, repo → merge request → `esc`, an error in the page, refresh refetches, tab click,
  and snapshots at 160x40 and 80x24. The row check in `lib.rs`
  (`every_row_and_target_is_taken`, 3.2) visits each tree row's page
  (`.down(n).enter()`), every tab (`.tab(t)`) and the pages it opens (`.enter()` again);
  add a page kind by adding its place there.

### 1.13 How parts join: subjects, Show and contributions

Someone may run radar with GitLab and no Jira, Jira and no Slack, or all of them. Every
mix must work, look finished, and gain from each part that is there. Parts join each
other only through radar-core (`crates/radar-core/src/compose.rs`).

**The rules.**

1. **No part depends on another.** No part crate imports another part crate, and none
   names another part's id or crate in code. radar-core names no tool at all, not even in
   comments or test samples. `tests/parts.rs` fails the build otherwise.
2. **Absent means absent.** A part that is not compiled in, or not set up, leaves no
   empty column, no dead key, no greyed-out action, no "install X to see this". Its
   contributions simply do not exist.
3. **Each part works alone**, and each one present makes the others richer.
4. **Present but not set up** (`set_up()` returns `Setup::Needed(why)`): its screen tab
   shows the host's set-up page (the sentence, and `⏎` goes to its first settings
   section), its settings sections are there, its badge and status are not, and it adds
   nothing to other parts and owns nothing for `Show`.

**Subjects** are references to things: `Subject { kind, id, label, url, fields, related }`. radar-core
never looks inside a kind; the kind names and their fields are the parts' shared
vocabulary, written down here (not in a crate of constants). Two subjects are the same
thing when kind and id are. Build one with `Subject::new(kind, id, label).with_url(..)
.with(field, value)`; `Subject::link(url)` is the one kind core knows (`link`).

Before an action runs, the host gathers `Plugin::related(subject)` from the other ready
parts into `subject.related` and passes the same references to `act_with_related`,
one level deep. Existing `subject.related` entries remain unchanged; newly gathered
entries are deduplicated by kind and id, with the first entry winning. These are
cached references, with no fetch. Jira supplies the same tickets it shows on an MR
(title, branch, or development panel); Slack uses their URLs in review asks, linking
keys in the title and appending linked keys when absent from the title. Without a
resolved ticket URL, the words stay plain.

| Kind | Owner | id | Fields | Page |
|---|---|---|---|---|
| `repository` | radar-github | canonical `https://github.com/owner/repo` | `provider`, `instance`, `repository`, `repository_url` | yes |
| `pull-request` | radar-github | canonical `https://github.com/owner/repo/pull/number` | `provider`, `instance`, `repository`, `number`, `repository_url`; held `title`, `branch`, `head_sha`, `source_repository_url` | yes |
| `workflow-run` | radar-github | canonical `https://github.com/owner/repo/actions/runs/id/attempts/attempt` | `provider`, `instance`, `repository`, `repository_url`, `run_id`, `attempt` | yes |
| `merge-request` | radar-gitlab | `<project>!<iid>` | `project`, `iid`, `title`, `branch`, `target`, `author`, `state` | yes |
| `repo` | radar-gitlab | full path | `project` | yes |
| `pipeline` | radar-gitlab | `<project>#<id>` | `project`, `id` | no (its link) |
| `job` | radar-gitlab | `<project>#<id>` (the job's id) | `project`, `id`; `name`, `stage`, `status`, `pipeline` (the pipeline's id) once known | yes (its log) |
| `ticket` | radar-jira | the key, `ABC-12` | `title` (summary), `status`, `project`, `site` (Jira's site name), `type` (issue type), `assignee` (display name, empty when none), `parent` (the parent's key, empty when none) | yes |
| `checkout` | radar-git | the clone's folder | `repo` (the repo path, empty when none is linked), `branch` (its current branch) | yes (its row on the Branches screen) |
| `workspace` | radar-workspace | the workspace's folder | — | no (opened in an editor) |
| `note` | radar-workspace | the note's path | `ticket`, `type`, `status`, `title`; its link is the `file://` URL | no (opened in an editor) |
| `work-item` | radar-workbench | the item's id | — | yes |
| `thread` | radar-slack | `<channel>/<ts>` (the thread's first message) | `channel`, `ts`; optional `team_id` (workspace known when the reference was made) | yes |
| `review-ask` | radar-slack | `draft`: the review ask being written | — | yes |
| `link` | radar-core | the URL | — | no |

**A link opens a page here only when it is exactly one of yours.** Every owner's `show`
and `recognise` hold to these rules (GitLab's, Jira's and Slack's tests cover each):

- The subject's id is the identity. Its fields (`project`, `iid`, `id`, `site`,
  `channel`, `ts`, `team_id`) must agree with the id, and a supplied URL must name the
  same target on a configured origin (scheme, host, effective port, and for GitLab the
  install path; for Jira the exact `/browse/<KEY>`; for Slack the workspace held in
  `Me.url`). A Jira `site` given is a constraint: an unknown one refuses rather than
  trying another site.
- An unsafe spelling never routes locally: user info or credentials in the URL,
  backslashes, spaces or control characters, dot segments a parser would repair,
  encoded path separators, extra path segments.
- A refusal changes nothing (pages, caches, the back stack) and leaves the original URL,
  query and fragment included, to the normal fallback (the browser, or the clipboard
  over SSH).
- Recognising works on whole URL tokens, as byte ranges: trailing prose punctuation and
  unmatched wrappers are trimmed, and a ticket key inside a URL that was refused is not
  picked out as a bare key. Cached data may add display words, never change the target.
- These checks prove the link is consistent with what radar holds, not that the remote
  thing exists or that the account owns it.

radar-slack matches the subjects it is asked about by their `url` (a thread that links to it
or under it) and, for a `ticket`, by its id as a word in the text; it groups its own
discussions by what the owners here `recognise` in a thread's links and text.

Add a kind or a field here before using it. A contributor matches on fields (a ticket
part looks for its keys in a merge request's `title` and `branch`), never on who owns
the kind.

**Recognising.** A part recognises its own kinds in text: `recognise(text)` returns byte
ranges and subjects (radar-gitlab: links to its server's merge requests, pipelines and the
repos it knows). Others call `cx.parts.recognise(text)`, which merges every owner's finds
in order. So a chat part learns `ABC-12` is a ticket only if a ticket part is here.

**Showing.** Any part returns `Effect::Show(subject)`. The host sends it to the set-up
part that owns the kind, whose `show` opens its page (`Showing::Page`); the host
switches to its screen and remembers where you came from. `esc` backs out of the owner's
pages as usual, and once it backs out past the page that was shown (the owner's
`depth` drops below what it was), you are back where the show came from, focus and all.
The `x` menu's `esc` row says so ("back to tickets"). Switching screens by hand forgets
the way back. A `link` is first offered to the owners' `recognise` (a link to a GitLab
merge request opens its page); with no owner, the host opens `subject.url` the radar way
(browser, or clipboard over SSH) and says so, or says it cannot.

**Contributions.** A part that draws a subject offers places on it; any other set-up part
may fill them from its own cache (its syncs, the bus). `contribute` is called while
drawing: never fetch, never block. The owner decides where; the contributor decides what;
the host asks in plugin order (the order in `src/lib.rs`) and never asks the drawer
itself.

| Point | `Contribution` | Drawn by the owner as | radar-gitlab offers it on |
|---|---|---|---|
| `Chips` | `Chips(vec![Chip::new(text, tone)])` | chips after the row's name (the title gives way while it keeps 12 cells) | merge request rows (tree, repo page), repo rows, the merge request's detail pane |
| `Column` | `Cell(Cell::new(heading, text).tone(..).show(subject))` | a column before `for you`, only when some row has a value and the names keep 30 cells, cut to its width and dropped whole when there is no room; a click on the cell shows its `show` subject | the repositories tree, a repo page's merge requests |
| `Section` | `Section { title, lines }` | a titled block | the merge request page's overview (after the description) and detail pane |
| `Tab` | `Tab { title, rows }` | a tab after the page's own, with its keys; `⏎` on a row shows its `subject` | the merge request page, the repo page |
| `Actions` | `Actions(vec![Action::new(key, what, id).icon(..)])` | rows in the subject's `x` menu under the part's `name()`; an owner may also list them where it draws the subject (`cx.parts.actions(subject, own_keys)` gives the keys the menu keeps), a click returning `Effect::Press(key)` | merge requests and repos (tree, preview, pages; the tree's preview lists them); jobs (the job page) |
| `Line` | `Line(vec![Chip::new(words, tone).icon(..)])` | one line under the subject's title, as running words (`ui::chips::words`), the first part's only | the repo and merge request pages |
| `Mark` | `Mark { icon, words }` | the row's icon in place of the kind's, and the words in place of the owner's detail (a repo cloned here, on its branch) | repo rows |

A badge on the top bar is still `badge()`, by the part's own tab, and hidden while it is
not set up.

radar-slack adds to repos (from the alerts its rules place on them): a `Chips` marker
(`firing · 2`, `job failed · 1`) and an `Alerts` tab whose rows show the alert's `thread`;
radar-gitlab draws the chips on repo rows and on the repo page's title line.

radar-jira offers on tickets: `Column` (its middle pane), `Section` (the ticket's detail pane and page
overview), `Tab` (the ticket page; a contributed tab titled like one of its own, Code,
joins it) and `Actions`. It adds a ticket column (a click opens the ticket), a section, a
Ticket tab (`⏎` on its row opens the ticket's page) and actions to merge requests; radar-gitlab
adds a Code tab, a `code` column and a section to tickets (`crates/radar-gitlab/src/tickets.rs`).

radar-git owns `checkout`; it adds to `repo` (its `project`, else its id) and
`merge-request` (`project`, `branch`) a `Line` (where it is checked out here) and
`Actions` (`c` check out a merge request's branch, `e` editor, `t` terminal, `s` look
again), for any host that draws them (1.11a).

**Keys of contributed actions.** The host drops a contributed key that the owner's `x`
rows use, that the host or moving about uses (`q x ? b w z h l j k g G [ ] / : + = -`
space, digits, capitals; `host_key` in `host.rs`), or that an earlier part already took:
the row stays, chosen from the menu only. A kept key works on the owner's screen too, but
only when the owner did not use it (its `intent` returned `Ignored`): no contribution can take a key the owner uses.
Choosing one calls `act(subject, id)` on the contributor.

An owner may give its contributed `x` rows a context (`Plugin::subject_menu_context`), as
`menu_key_context` does for its own keys (1.9): the host keeps the token, the route and
the subject when the menu opens, and checks them and the contributor's action again
before it runs one. The token lives in memory only and is never journalled; `None`, the
default, runs on the subject captured.

**Related subjects for actions.** A part may implement `related(subject)` to return
associated subjects from its cache, without fetching or blocking. For a ticket, the
code-host part returns its linked merge requests' `repo` subjects, one per exact project
path. The repository subject's id is its full project path; no label or folder-name
matching is implied.

`Parts::related(subject)` gathers only set-up providers, excluding the receiving part,
and keeps the first subject for each `(kind, id)`. Before dispatching a contributed
action, by its key or its menu row, the host gathers these subjects and calls
`act_with_related(subject, id, related)`. The default forwards to `act`, so existing
parts keep their behavior. This is a one-hop, read-only lookup, not a recursive graph
walk. A missing provider contributes nothing. Start work uses related repositories to
suggest its initial clone selections; the user can still change every tick before the
existing start-work confirmation.

The host gathers this input once and both records and dispatches it: the
`plugin.action` journal record keeps the subject and its related subjects (top level
only, at most 32, those it cannot record counted). A related subject recorded says only
that it was passed, not that the receiver used it.

**Knowing what is there.** `cx.has(kind)` (a set-up part owns it) and `cx.can_show(kind)`
(and opens a page for it), for choices that are not contributions.

**The parts section** (settings, `general`, host-owned): every part compiled in, set up or
not; for the selected one what it has, what it opens, the places it offers others and
what it adds to the parts that are here (`offers()`, `adds()`).

A part adding a chip and an action to merge requests, from its own cache:

```rust
use radar_core::{Action, Adds, Chip, Contribution, Owned, Point, Subject, Tone};

impl Plugin for TicketsPlugin {
    fn owns(&self) -> Vec<Owned> {
        vec![Owned::page("ticket", "tickets")]
    }
    fn adds(&self) -> Vec<Adds> {
        vec![Adds::new("merge-request", "ticket chips and start work")]
    }
    fn contribute(&self, point: Point, s: &Subject) -> Option<Contribution> {
        if !s.is("merge-request") {
            return None;
        }
        let title = s.field("title")?;
        let t = self.cache.iter().find(|t| title.contains(&t.key))?; // never fetch here
        match point {
            Point::Chips => Some(Contribution::Chips(vec![Chip::new(
                format!("{} · {}", t.key, t.status),
                Tone::Warn,
            )])),
            Point::Actions => Some(Contribution::Actions(vec![Action::new(
                's',
                format!("start work on {}…", t.key),
                "start",
            )])),
            _ => None,
        }
    }
    fn act(&mut self, s: &Subject, action: &str) -> Vec<Effect> {
        match action {
            "start" => self.start_work(s), // asks first (2.23), then Effect::Spawn
            _ => vec![],
        }
    }
}
```

Testing a part next to another: write a stand-in part in your tests that knows only the
vocabulary (`crates/radar-gitlab/src/stand_in.rs`: a tracker that owns `ticket`s and fills
every point on merge requests), and run a few targeted host tests with it (what it adds
shows, a contributed row runs). What your part adds to others' `x` menus is checked on
your plugin alone with `Check::actions(&subject)` (3.2). radar-core's own routing, ordering, key
clash, back stack and absent-means-absent tests use neutral stand-ins (shelf, notes, bell)
in `crates/radar-core/src/compose/tests.rs`.

### 1.14 Building each part on its own

Each part is developed, tested and run on its own; radar-core is the only thing they share.

- **Features.** The root `Cargo.toml` has a feature per part (`gitlab`, ...), all on by
  default. Adding a part is a feature line (`jira = ["dep:radar-jira"]`), an optional
  dependency (`radar-jira = { workspace = true, optional = true }`) and one `#[cfg(feature =
  "jira")]` line in each list in `src/lib.rs`. Run a part alone with
  `cargo run --no-default-features --features jira`; no parts at all with
  `--no-default-features` (settings with the parts section saying none are compiled in).
  A provider feature turns on the part it serves (`sessions-claude = ["sessions", …]`),
  and `shots` brings `demo` and `png`.
- **The guard** (`tests/parts.rs`): fails if a part depends on another (from
  `cargo metadata`), if a part's source names another part's crate or `"id"`, if radar-core
  names any part or tool word (gitlab, jira, slack, merge request, ticket, ...), or if a
  part is not behind its feature.
- **The mix build** (`scripts/check-mixes`, portable bash, nextest when installed):
  checks none, each part alone, all parts, each adapter with its part and the shipped mix,
  and runs `tests/mix.rs` with `demo` in each: the smoke (each screen draws, a click and
  a key act), the first screen's recording (`tests/snapshots/mix__*.snap`) and, with
  all parts, each other screen once and one screen per icon pack. A mix whose check
  fails skips its tests. No crate's own suite (its session runs it, CI runs the
  workspace once) and no pairs or other combinations: the guard keeps parts from
  depending on each other, and what two parts do together runs in the all-parts mix.
  `--quick` is the same run.

**Starting work on a part.** Its own branch, worktree and session. Its crate depends on
radar-core and crates.io only; its tests never need another part (use a stand-in, 1.13). Run
it alone with its feature. Learn the vocabulary above; add the kinds and fields you own
or match on to the table first.

**A radar-core change** (a new point, a trait member, a field in the contract) gets its own
small branch that lands first, with its tests in radar-core using neutral stand-ins, and no
tool named. Parts then build on it.

**Merging a part into `main`** happens only when its own checks pass (`cargo test -p
radar-<id>`, clippy, its live check) and CI passes (which runs `scripts/check-mixes`).

### 1.15 The kit: `radar_core::ui`

**The registry is [`docs/components.md`](components.md):** every element, which view type
carries it, how Ink draws it, its states, slots, glyphs and keys, the one
canonical form of each shared concept, and the drift still to fix. Look there before you
draw anything, and update it in the same commit as the element.

**Parts describe with the kit; they do not hand-roll an element the kit has.** The kit is
the set of view types and helpers in radar-core (`view`, the rich document model, rows,
scroll state, widths and icons) plus the Ink components that draw them. If a part needs
one the kit lacks, or a variant of one, it goes into the kit first (a small radar-core
change, 1.14, and an Ink component with a story), and then the part uses it. A part keeps
only what is its own: which rows, which words, which look words, which slots.

Every element is generic (no tool words; the guard in `tests/parts.rs` checks), draws only
with theme slots, and takes styles a part resolved from its look words where the user can
choose them. Each Ink component has stories at 80 and 160 wide (`ink/stories`).

**See them live**, without any part: Storybook is the gallery.

```bash
cd ink && npm run storybook      # http://localhost:6016, MCP at /mcp
```

Each story draws a component from recorded view data in each of its states, side by side:
selected, without focus, empty, loading, failed, too long. **To show a new element or
state:** add a story to the component's file (or a new file next to it), recorded from view
data a Rust test produced, title each panel with the state it shows, and run
`npm run manifest` so `docs/storybook/components.json` follows. Look at it once in
Storybook, and for the whole screen with `scripts/shots --compare main` (3.1a).

Every element, its API, file, states and keys is in
[`docs/components.md`](components.md) section 3; this guide does not repeat the list.

When to use which:

- **Tree or table?** A tree when rows group under others and folding helps (a group, its
  sources, their items); a table when the rows are one kind (a page's list of items, a
  history of runs). Both take the same `Columns`, so a column means the same thing in each.
- **Columns** get a `keep` rank: the lowest goes first as the width shrinks, and the order
  never changes. Another part's columns go in with `with_extra`, before the last, only while
  the names keep their room; never push out your own.
- **Card, scrolled body or table?** Text goes in a `card`; a body that stacks widgets (a
  gauge over text over a table) goes through `scrolled_with`; a list with a cursor is a
  `Table` or a `RowList`, which keep the cursor in view themselves. Keep one `Scroll` per
  scrolling panel on the plugin (it holds `Cell`s, so drawing through `&self` can clamp it),
  reset it with `top()` when the selection changes, and hand it keys with `key()` (2.4).
- **Clicks**: a part gives each clickable thing a `Target` in its view data (tab labels,
  rows); Ink records the area of each target it draws (2.5). Whatever has a target is lit
  under the pointer and must act when clicked: `radar_core::check` checks it in the part's
  tests (3.2).
- **Look words**: a tree's lines, band and heading shade come from the part's look words
  through `TreeLook::from_specs`; a stage's style from the part's word for that state;
  everything else uses core slots or a `Tone`.
- **Tree lines**: a row at depth `d` starts `"{d-1 spaces}├─ "`, so each glyph sits under
  its parent's name; top-level names have no left-edge stripe. Legacy stripe metadata
  is retained for compatibility but draws no rail. `TreeLook::band_level` says whose
  blocks alternate bands (1: a group's
  children; 2: one level deeper). A row with no cells (a group's heading) runs across
  the columns.

### 1.16 Rich text, links and images

A service's formatted text (a description, a comment, a message) is drawn through one
model, `ui::rich`, so every part's text wraps, quotes, lists and links the same way. A part
**converts** its format into a `Doc` and **lays it out** with the kit; it keeps only what is
its own (resolving names, its statuses and panels, its look words).

**The model.** `Doc { blocks }`. A `Block` is `Para`, `Heading { level }` (level 1 is
underlined), `List { items }` (each `Item` has a `Marker`: `Bullet`, `Number(n)`, `Todo`,
`Done`, `Decision`, and blocks of its own, so lists nest), `Quote`, `Callout { tone,
symbol }` (a bar in the tone's style: a panel, a note), `Code { lang, text }`, `Table {
aligns, head, rows }` (columns, or a block per row when they cannot fit), `Rule`,
`Details { summary }` (shown open, no chevron) and `Media` (`image`, `name`, `url` where it
opens, `src` what draws it, `size` in pixels when the service says). Runs are `Inline {
text, kind, marks }`: `Kind::Text`, `Link { url }`, `Mention { me }` (`me` styles it as the
reader's), `Emoji` (its glyph, or `:name:`), `Badge { tone }` (a state, drawn as a chip),
`Image { src }` (an image among words: its words), `Break`; `Marks` are bold, italic,
strike, underline, code. Build with `Inline::text/link/mention/emoji/badge/line_break`,
`.bold()`, `.code()`, `.with(marks)`, `Block::para`. A `Doc` serialises, so a part can
keep it in its cache (radar-jira does, and reads an older shape leniently: `lenient` in its
`model.rs`).

**Converting.** Markdown: `ui::markdown::parse(text)`. Jira: `adf::doc(&value)` (statuses
become badges by colour, panels call-outs by type, dates text, media found among the
ticket's attachments by name with `adf::attach`). Slack: `view::doc` turns
`mrkdwn::parse`'s lines into blocks (a run of `>` lines a quote, a ``` block code), names
people, groups and channels from the cache, marks you and your groups as `me`. Emoji
short names go through `rich::emoji(name)`: the glyph only when it is one wide character
(every terminal draws it two cells wide); joined sequences, flags and skin tones keep
their `:name:`, so columns never move. Glyphs may go anywhere, tree rows included: the kit
counts display cells (1.18, the width rule). Slack's rows show a message's emoji as glyphs
(`view::first_line_spans`); a custom emoji with no glyph stays `:name:` in the `emoji`
look.

**Laying out.** `rich::layout(&doc, &look, width) -> Laid { lines, links, media }`: lines
at most `width` cells, the links in order once per url (for a links list or `o`), and
where images sit. `Look::new(theme)` is the kit's look; set its fields from your look
words (`text`, `heading`, `quote`, `bar`, `code`, `link`, `mention`, `me`, `emoji`,
`muted`, `marker`, `rule`; Slack sets `me`, `mention`, `link`, `emoji`, `code`, `quote`).
`look.urls = true` shows a link's address after its words (Jira does when the terminal
cannot open the words). `rich::plain(&doc, width)` is the same words with no style, for a
command line or a test. Lines put after others (a header, a reply's indent) move their
images with `Placed::moved(lines_before, cells_in)`.

**Links.** Every link in rich text is a terminal hyperlink when the terminal takes them
(`[theme] hyperlinks = auto|on|off`; auto: Ghostty, kitty, WezTerm, iTerm2, VTE, Windows
Terminal and the like, never inside tmux or over ssh by guess). A part puts a link in its
view data (an inline link or a `Chip` carrying `.link(url)`); Ink underlines the words and
wraps each run of cells in OSC 8. Only `http(s)` and `mailto` urls without control
characters are handed to the terminal. `o`, `y` and the Links
tabs still open links the radar way (`Effect::Open`).

**Images.** Where the terminal answers for kitty's, sixel or iTerm2's graphics
(`[theme] images = auto|on|off`, asked once at start, not inside tmux unless `on`), rich
text keeps rows for each `Media` with `image` and a `src` (its size fitted to the width,
at most 20 rows; one row until its size is known) and `Laid::media` says where; elsewhere
(and in every snapshot) the image is the line `▣ image <name> <url>`. Draw the lines with
`ui::image::card(...)` or `ui::image::scrolled(...)` instead of `card`/`scroll::scrolled`,
passing the placed media and your `Fetch`. The kit asks only for images in view, three at
a time, keeps 32 in memory and the bytes on disk under the cache folder (200 MB, the
oldest let go); a failed one falls back to its line. A part supplies only the fetch, and
keeps its credentials in it:

```rust
let token = self.cfg.resolved_token().ok();
let fetch = radar_core::ui::image::fetch(move |src: String| {
    let token = token.clone();
    async move { client::picture(&src, token.as_deref()).await } // your client's fetch: bytes, or an error
});
radar_core::ui::image::card(t, area, buf, "thread", true, &self.scroll, lines, &media, &fetch);
```

Send a token only to your own service's hosts (Slack's `client::slack_host`, Jira's
`client::on_site`, GitLab's `pictures::on_server`), never log it, and refuse a sign-in page as not
a picture. Slack draws a file from its largest thumbnail up to 1024 wide (Bearer token);
GitLab makes a description's `/uploads/...` whole and asks its API first (Bearer token);
Jira draws an attachment's content (basic auth, the redirect to its media store followed
without it).

### 1.17 Motion: `radar_core::motion`

Short effects that show what changed, run by the host and the runner over a frame
already drawn (`crates/radar-core/src/motion.rs`, built on `tachyonfx`). **Parts never
depend on `tachyonfx`** and never animate anything themselves; they say what changed and
where it is, and the host decides what moves.

How it works: the host compares what is on screen before and after each event and
queues a `Cue` (`Host::take_cues`) and sends it to Ink with the view data; Ink turns each
cue into an effect on the region or row it marks, and redraws on a frame tick
only while one runs. Idle, radar draws on events alone.

| Effect | Cue (who) | Where | Time | `subtle` | `full` |
|---|---|---|---|---|---|
| Body fades in | another screen: a tab's key, `,`, a show across parts (host) | the body | 100 ms | yes | |
| Screen assembles | the same cue at `full` (`motion::assemble`): panels found by their corners, left to right 30 ms apart; each frame draws down row by row, each row inside reads in from the left 40 ms after its frame row; anything outside a panel is there at once | the body | 450 ms | | yes |
| Body sweeps in from the left | an item's page opened: the screen's `depth` grew (host) | the body | 300 ms | | yes |
| Popup sweeps open | a menu or confirm appeared (host); closing it is instant | the popup | 180 ms | yes | yes |
| Row glows (accent background that fades) | a sync changed the row (part: `Effect::Changed`) | the row | 450 ms | yes | yes |
| New row sweeps in | a sync added the row (part) | the row | 320 ms | glows | yes |
| Message sweeps in | a marked message appeared (status line) | the message | 250 ms | | yes |
| Message fades out | the marked message left (status line) | where it was | 200 ms | | yes |
| Loading outlines, repeating | a panel waits for its first data (part: `motion::loading`): its blank rows show the shape rows will have (icon slot, name, state, age, `motion::outline`), and a light wave runs down them | the panel | 1.4 s a pass | | yes |

Host notices are composited after motion, outside retained frames. A notice reads
in from the right edge over its first 120 ms and goes back into it over its last
120 ms, whenever motion is not `off`; both are worked out
from its age, so nothing is kept after it expires.

**What a part does.**

1. After a sync, say which rows changed: compare what was held with what is held now,
   and return `Effect::Changed(changes)` when it is not empty.
   `motion::Changes::between(before, after)` takes `(key, value)` pairs (`value:
   PartialEq`) and reports `changed` (there before, different now) and `added`; with
   nothing before (the first data) it reports nothing, so a list that fills does not
   flash. The key is the row's subject, `motion::key(kind, id)` (`ticket:ABC-12`), so two
   parts' keys never meet. radar-gitlab does it in `on_msg` for merge requests
   (`join::motion_key`), radar-jira per answer for tickets, radar-slack per read for threads
   (`SlackPlugin::threads`: each thread's first message, whose reply count and latest
   reply say when it moved).
2. Give each tree row in your view its key: `TreeRow::glow(key)` (`view.rs`). The
   renderer marks the rows it draws; a part marks nothing
   itself. A row not drawn in the frame after the cue is let go; a row that moves (the
   sync re-sorted the list) glows where it is now.
3. Where a panel waits for its first data (`Loading from …`, a page's `loading…`), call
   `motion::loading(inner)` while it waits. Stop when the data is there (do not mark a
   panel that is refreshing what it already shows).

**Rules.**

- Input wins: a key, click or paste drops what runs before the host sees it, and the
  key acts at once (`Engine::input`); cues from that key start after. Loading outlines go on.
- Every effect is time-based and short; `FRAME`, the durations and the slow-terminal
  rule are constants in `motion.rs`. Colours come from the theme: the body and popup fade
  from `bg.base`, the message from `bg.overlay`, the glow is `accent.primary` over the
  row's own background, outlines sit 12% from `bg.surface` toward `text.primary` and
  the wave lifts them to 45%.
- One plain frame follows the last effect, so what stays on screen is exactly what radar
  drew.
- `off` applies nothing at all. Motion is also off, whatever the setting, over SSH,
  below full colour (the effects blend colours) and, for the rest of the session, after
  three frames in a row take over 60 ms to draw while effects run (`motion::held_back`,
  `Engine::drew_in`); the colours section's detail says so.
- The setting: `[theme] motion = "full" | "subtle" | "off"`, `subtle` by default, chosen
  in Settings › General › colours (`motion` row).
- **The startup splash** (`radar_core::splash`) is a radar sweep that follows the level. At
  `full`, a 64 × 24 braille scope opens. One turn of the beam (200–1400 ms) locks a contact on
  each part at its bearing, and the readout counts them. The mark paints in, then the parts'
  names. The start line (`radar_core::start_lines`, a new one each start) types in, and the
  first screen fades in at 2300 ms (the screen cue). At `subtle` there is no beam: everything
  fades in, and it hands off at 1600 ms. At `off` (or held back) the last frame shows only when
  the first screen is not ready after 150 ms. A rare echo (1 in 50) or the fire event (motion
  full, held 1.5 s longer) comes with the start. Too small for the scope, it shows the mark,
  the names and the line. In a terminal with kitty's graphics the scope is drawn in pixels;
  braille elsewhere. Skipped before 1.5 s (`splash::EARLY_MS`), its line moves to the status
  line for 5 s. Any key, click or
  paste ends it and acts on the screen under it. radar-core works out every cell
  (`splash::frame`); the bridge sends it as `Snapshot.splash` and Ink paints it. `[theme] splash = "auto" | "off"` (Settings › General ›
  colours, `splash` row).
- Rust tests never run effects: the view-data text form carries no motion, so a
  snapshot cannot change because of it. Test the facility with the injected clock
  (`motion::ManualClock`, `crates/radar-core/src/motion/tests.rs`), the cues through
  `Host::take_cues` (`what_changed_on_screen_is_cued_for_motion`), and a part's change
  and marks with `motion::take()` after the part's view call (e.g.
  `a_sync_says_which_merge_requests_changed_and_the_tree_marks_their_rows`). How an
  effect looks is Ink's to test (`ink/test`, `ink/stories`).
- See an effect live with `ink-live` (3.4): capture a few frames
  within the effect with `capture-pane -e` to see the colours move and settle.

### 1.18 Icons: `radar_core::ui::icon`

Icons go wherever they make sense: on anything that reads faster with one. **The kit hands back icons from one place; a part
never writes a glyph.** A part names what it means and the kit draws it from the pack the
user chose, in the icon's colour.

**The set.** `Icon` is one semantic enum (`crates/radar-core/src/ui/icon.rs`, the `icons!`
table): code (`Group`, `Repo`, `Branch`, `Commit`, `Change` (open, asked to be merged),
`Draft`, `Merged`, `Closed`, `Approved`, `NeedsApproval`, `Thread`, `Comment`,
`Resolved`, `Conflict`, `Rebase`, `Label`), runs (`Pipeline`, `RunPassed`, `RunFailed`,
`RunRunning`, `RunPending`, `RunSkipped`, `RunManual`, `RunCanceled`, `Job`, `Log`),
work (`Issue`, `Epic`, `Story`, `Task`, `Bug`, `Subtask`, five priorities, `Todo`,
`InProgress`, `Done`, `Site`, `Board`), people (`Person`, `You`, `Bot`, `Team`), chat
(`Chat`, `Channel`, `PrivateChannel`, `Direct`, `Reply`, `Mention`, `Reaction`, `Unread`,
`Alert`, `AlertFiring`, `AlertResolved`), the app (`Settings`, `Link`, `Search`, `Time`,
`Updated`, `Calendar`, `Info`, `Warning`, `Error`, `Empty`, `Menu`, `Keys`, `Quit`,
`Navigate`, `Scroll`, `Bulk`, `Colours`, `Icons`, `Parts`, `Connection`, `Views`, `Look`,
`Local`, `Editor`, `Terminal`) and files (`File`, `Folder`, and a type per language:
`Icon::for_file(path)` picks one by extension or name). Each has a name (`run_failed`, the
key in packs and `[icons]`), a meaning (shown in `General › icons`) and a default style (a
slot: a failed run `status.error`, a bug `status.error`, a story `status.success`, a
direct message `accent.primary`). Add an icon by adding a line to the table, with its
glyph in each built-in pack; the tests check every glyph's width.

**Packs.** A pack maps names to glyphs (`name = "glyph"` or `name = { glyph = "…", style =
"status.error bold" }`) and declares `width`, the cells every glyph takes (1, or 2 for
emoji; narrower glyphs are padded, so every icon of a pack is as wide as the others).
Built in: `nerd` (Nerd Font symbols, one cell: Octicons, Font Awesome, Devicons), `unicode`
(plain symbols every font has, one cell, never emoji: `✓ ✗ ● ◷ ⇄ ▤ ⎇`), `emoji` (two cells)
and `off` (no icon and no gap). A user's pack is `<config dir>/radar/icons/<name>.toml`
(`radar icons path`): `name`, `description`, `extends` (another pack), `width`, `[icons]`.
Names it leaves out come from the pack it extends, and in the end from `unicode`, so a
partial pack always works. Overrides in config.toml's `[icons]` go on top of the chosen
pack (`off` stays off). A glyph wider than the pack's width, or one terminals draw at a
width of their own choosing (joined emoji, skin tones, flags), is refused with a reason
(`icon::check`), and the next pack's glyph is used; an unknown name is a warning in the
log, not an error. `docs/icons/example.toml` is a commented example; `radar icons export
<pack>` writes a whole pack to start from; `radar icons list` shows every name and its glyph
in the pack in use.

**Choosing.** `[theme] icons = "auto" | <pack>`. `auto` (`icon::detect`) is `nerd` where the
terminal is known to ship or commonly use Nerd Font symbols (TERM_PROGRAM `ghostty`,
`WezTerm`, `kitty`; TERM with kitty, ghostty or wezterm; and, inside a multiplexer,
`GHOSTTY_RESOURCES_DIR`, `KITTY_WINDOW_ID` or `WEZTERM_PANE` still set; herdr passes
TERM_PROGRAM=ghostty through), else `unicode`; over SSH `unicode` (the font is at the other
end). The runner says what `auto` is (`icon::set_auto`) and loads the user's packs
(`icon::set_packs`) when it starts; until then `auto` is `off`, which is what every test
and snapshot gets. Settings: `General › colours`, row `icons` (a menu of `auto`, the
built-in packs and the user's, each with a preview line), and `General › icons` (every
icon with its glyph; `⏎` types your own into `[icons]`, refused with the reason when it
does not fit; `r` gives it back to the pack).

**How a part picks an icon.**

- `icon::lead(theme, Icon::Repo)` before a name: the glyph and a space, styled; an empty
  span when icons are off. `icon::span` is the glyph alone.
- `icon::text` / `icon::with(theme, Icon::Bug, "words")` for words that are a `String`
  (a tab label, a menu row, a chip, a heading).
- `icon::or(theme, Icon::RunFailed, "✗", style)` where the screen already showed a symbol:
  the icon when icons are on, else the old symbol, so `off` looks as it did.
  `Mark::span(theme, style)` is the same for a run's mark.
- Words a part hands the host as strings carry their icon by name, and the host draws it
  from the pack in use, in the icon's own colour, when it draws them: `Menu::icon` (before
  the title, after the host's own: a confirm's warning, the keys menu's), `MenuItem::icon`
  (before a heading's, an item's or a note's label), `Binding::icon` (a keys menu heading
  first in the list gives the menu's title its icon; `Binding::heading("!263").icon(Icon::Change)`),
  `Chip::icon` (drawn by `ui::chips::chip_spans` on the chip's shade; `chip` alone puts it
  in the chip's colour), `Cell::icon` (a contributed column's value; the owner draws
  `Cell::spans` into its tree cell) and `Effect::NotifyWith(icon, words)` for a status-line message
  (`Host::message` is a `Message { icon, text }` that reads as its words). With icons off
  none of them takes room. Where the words showed a symbol before icons (`✗ could not …`),
  `let (icon, mark) = icon::plain_or(Icon::Error, "✗")` gives the icon to name with icons
  on, else the symbol to write: `Effect::notify(icon, format!("{mark}could not …"))`,
  `MenuItem::heading(format!("{mark}{status}")).icon(icon)` (the builders take an
  `Option<Icon>` too). `icon::plain` is only for words that truly have no style (a
  menu item's hint, a line copied out).
- A tree cell is a span or several: `Node::cell(h, vec![icon::lead(t, i), words])` keeps the
  icon's colour beside the cell's words.
- `Column::icon(Icon::Person)` puts an icon before a column heading (the heading still
  names the column for its cells).
- `Plugin::icon(screen)` puts one on the screen's tab; `Plugin::section_icon(section)` on
  each settings section in the nav. The host's own chrome has its icons already: the
  Settings tab, its sections, the status line's keys, menu titles (a confirm carries the
  warning), the empty, set-up and failed states.
- One icon says one thing: a row can carry several (its kind, its priority, its state)
  when each means something; never two side by side that say the same.

**The width rule.** Every fit, cut and pad in the kit counts display cells
(`ui::width`, `ui::take`, `ui::pad`, `ui::fit`, `ui::clip`; `Span::width`), never
characters: an emoji or a CJK character is two cells, a combining mark none, a wide
character is never cut in half. Do the same in a part's own drawing code: no
`.chars().count()` and no `format!("{:<N}")` for anything drawn in columns. Text
snapshots (the view-data text form) show a wide character once, as the
terminal does, so columns in a snapshot read as on screen. Ink's tests put emoji and
CJK in tree rows, tables, columns and cards.

**Testing.** Tests run with icons off (auto is off there), so existing snapshots never
change because of icons; a part's icon snapshots load a config with `[theme] icons =
"unicode"` or `"nerd"` (Private Use Area glyphs are fine in a snapshot), at 80x24 and
160x40. Each pack's glyph table is a unit test in radar-core (`ui/icon/tests.rs`); the mix
check keeps one screen per pack. Storybook's Icons story shows every pack, and
`scripts/shots` draws with the pack in the fixture, its SVG fonts putting Nerd Fonts first.

---

## 2. The UX contract

A new plugin meets every rule below. Each has its reason. "Check" says how to tell.

### Keys and bars

**2.1 The status line is lazydocker's fixed bar.** `PgUp/PgDn scroll · bulk · +/-
expand/collapse · quit · extra actions · ←→↑↓ navigate`, drawn by the host
(`Host::status_hints`), with b, q and x underlined inside their words. The bulk
action appears only where there is a bulk list, `+/-` only on a tree
that folds, `PgUp/PgDn` on every screen (and in settings when the detail scrolls). With
the top bar or a page's tab row holding the keys it says `←→ screens · ↓ enter` (or
`←→ tabs`, `←→ sources`) instead. A plugin adds nothing to it except while capturing
(`hints`, `section_hints`) and, in front of the fixed keys, the one or two main actions
of the selected row that apply right now (`row_hints`: u in “update by rebasing”, m in “merge”; none when
nothing applies). *Why:* one bar with the same keys everywhere is learned once. Check:
no `hints()` output outside capturing; `PgUp/PgDn` really scrolls something on your
screen.

**2.2 No shortcut footers or repeated letters.** No key hints inside a panel, no
separate letter before or after a visual action word. Tabs, hints, menus and
buttons underline the key inside a meaningful word. The host supplies
`mnemonic_at` (a Unicode scalar position) and `mnemonic_shift` on `Hint`, `Choice`,
`ActionSpec` and keyed rich lines; Ink underlines the complete grapheme
at that position, without searching for another matching letter. Uppercase
bindings retain their dispatch and show `shift` explicitly. Named and glyph
keys stay before the word; numeric custom-provider choices also keep their
visible number. Missing or invalid legacy metadata keeps the original key
visible. Plain-text output retains explicit shortcuts because underline alone
cannot convey them to a screen reader. *Why:* visual repetition is noise, but a
shortcut must remain discoverable in every output mode.

**The top bar takes the keys too.** `↑` (or `k`) on the first row of any pane of a screen
(not inside an item page, where `↑` at the top does nothing) gives the keys to the top
bar (`Host::bar`): its tab draws focused and no pane does (the view's focus),
`← →` / `h l` switch screens as they go, and `↓`, `j`, `⏎` or `esc` go back into the
screen, to the pane it had focused last (the first pane the first time). The letters,
`,` and clicks work as anywhere; the status line shows `←→ screens · ↓ enter` and the `x`
menu the same. Settings is on the bar too: `↑` from the nav's first row. **A part's key
handler reports `↑` not handled when it cannot move any further** (the kit's
`tree::key`, `Scroll::key` and `rows::select` do; a part's own list returns
`Outcome::Ignored`), and the host takes it up. An item page's tab row takes the keys the
same way: `↑` past the top of the body, `← →` along the row, `↓` back in; in the body
`h l` past the page's panes and `tab` on a one-pane tab move across the tabs; arrows
retain that behavior outside focused trees, so no
movement key is dead on a page (1.5). *Why:* moving to and from the top bar and a page's
tabs must work the same on every screen and page.

**2.3 Pane movement is the host's.** `tab`, `h l`, `1`..`N`, `⏎` in, `esc` out (1.4).
Do not rebind them. Outside trees, `← →` also move panes. Inside a focused tree,
`space` toggles the selected group; `+` opens it; `-` closes an open group or
closes/selects its parent; `→` opens a group or enters its first selectable child;
`←` closes an open group or selects its parent; `=` opens all groups already held
in the current tree. It never fetches missing children. `⏎` retains the owner's
original action. Up/k at the first selectable row still exits to the top bar.
Editors, menus, dock capture and modified keys keep their prior ownership.
The host consumes `Intent::OpenAll` and applies validated parent-first folds
without changing the selected subject; owner actions and confirmations remain
separate. Where a focused panel is a tree (`Plugin::folds`,
`Plugin::section_folds`), shared hints/menu entries describe the same operations.

**2.4 `↑`/`↓` in a focused detail pane move through it.** `PgUp`/`PgDn` scroll it from
any pane; a new selection resets it to the top; a scrollbar shows when there is more.
*Why:* `↓` in the detail pane is expected to read further, not jump to the next row. Test: `down_in_the_preview_moves_through_it_not_to_the_next_merge_request`.

**2.5 The mouse and the keys both work, everywhere, and every part proves it.** *Why:*
people use both; what the pointer is over should answer, targets must be big enough to
hit, and clicking in and out of a pane keeps its cursor.

- **Clicks.** Tabs (top bar, page tabs), nav rows, middle-pane rows, list and table rows,
  settings rows, column headings that pick something, other parts' cells, the detail pane's
  action lines, menu items and every key on the status line are clickable. A click on a
  row selects it at once, even in an unfocused pane; a click on a pane's empty space only
  focuses it and keeps its cursor; the cursor of a pane you left stays visible
  (`row_style`); two clicks within 400 ms are `⏎`. A click on the status line's `q quit`,
  `x menu`, `b bulk`, `+/-` (each half), `u rebase` … presses that key, with a menu open and
  while typing too (the status line's key targets). A click on the screen showing gives
  its tab the keys, as `↑` does. While something is typed, clicks elsewhere wait for `⏎`
  or `esc`.
- **Hover.** What a click would act on is lit under the pointer (`bg.hover`, quieter
  than the selection; underlined with 16 colours or none; the cursor row keeps its own
  look): the line of the target under the pointer. Over a menu the menu's highlight moves to
  the item under the pointer. Nothing is lit over a pane's background, a menu's frame or
  heading, empty space, or when the pointer leaves the window (the terminal lost focus).
  Ink does all of it from the targets in the view data; motion settings do not apply.
- **Pointer.** Where the terminal takes OSC 22 (Ghostty, kitty, foot, WezTerm; never
  inside tmux or screen unless `on`) the pointer is a hand over what a click acts on, a
  text cursor while a drag selects, the terminal's own elsewhere and after radar stops
  (`[theme] pointer = auto|on|off`, Settings › colours › terminal).
- **Target sizes.** A part gives a target to the thing, never only a row's text; Ink
  sizes the area: a row's whole width of the panel, a tree's or list's drawn rows (not the
  empty space under them), a tab, chip, heading or mark with a cell either side (never under
  3 cells), a pane's background (focuses, never lit), a page's tabs with no dead cell
  between them (the tab showing is not a target) and its body. The kit's row
  elements (`RowList`, `Table`, `Entries`, `LogView`, `DiffView`) are full-width rows.
- **Every target acts, every key shown acts.** Each click target a part gives must do
  something when clicked, each `x` row's key must reach the part and be taken, an `x`
  row's key must not be one the host keeps (`q x ? b w z h l j k g G [ ] / : + = -`,
  space, `tab`, `←` `→`, digits, the settings key `,`, history's `H` and any screen's
  capital: `check::host_keeps`), a click that opens a menu
  or a link has a key in that place's `x` menu (or `⏎`), and an action a part adds to
  another's menu returns effects. A key that would do nothing says why on the status line
  instead (`nothing to scroll here`, `nothing to move to here`, `nothing to fold`): report
  `PgUp`/`PgDn`/`↑` not handled when nothing moved (`Scroll::page` says whether it did).
  **Each part checks this in its own crate, at unit speed** (3.2): a table test over its
  places with `radar_core::check`, on the plugin alone, no host. The host's own targets,
  status-line keys and the routing of contributed rows are radar-core's unit tests; a mix
  checks only its smoke (`tests/mix.rs`: each screen draws, a click and a key act, no `x`
  row is on another screen's capital).

**2.6 `⏎` and `esc`.** `⏎` goes in or does the row's main thing; `esc` backs out one
level (closes a menu, cancels an input, clears a filter, returns to the previous pane).
Return `Ignored` for `esc` and `⏎` you do not need so the host's fallback works. A page
(1.12) pops one level per `esc` and the last `esc` returns to the list with its cursor kept.

**2.7 The `x` menu is about where you are.** Only the actions for the focused pane, to
what is selected, first row a heading naming the selection (`!263`, the repo, the group).
No movement keys. Every row works when chosen: the host presses the row's key, and drops
rows whose `keys` string is not a single key `parse_key` knows (`a`, `space`, `⏎`, `esc`,
`tab`, arrows, `pgup`, `pgdn`, and explicit ctrl combinations such as `ctrl-r`). Say what the key will do now ("hide
its repos" vs "show its repos"). *Why:* a menu of every option, or of options that do
nothing, cannot be trusted. Check: the part's row check (3.2).

**2.8 `b` is for bulk.** `bulk(scope)` lists what can be done to everything shown (open
all, close all, copy every link, refresh). Anything destructive asks first. If you have
none, return an empty list and the bar omits `b bulk`.

**2.9 Menus are lazydocker's; questions are dialogs.** A menu: key, label, a line saying
what it does. A question (`Menu::confirm`, the write confirms): what will happen, a blank
line, then its choices as raised buttons (a light edge, a filled face, a dark edge) in a row
against the right, each the key and what it does (`y post it   n drop it`), not `yes` and
`no` under each other (1.9). Never explain a
choice in a separate panel when the menu can carry the line. *Why:* lazydocker's context
menu is the model, and a question reads as a dialog with its answers side by side.

### Text and settings

**2.10 Editing is normal text editing.** Use `TextInput` for every field: visible cursor
where you type, readline keys, paste, nothing that eats characters. Implement `capturing()`
and `paste`. Pasted text is cleaned (links, partial paths) before it is validated. *Why:*
every field must edit like any normal text field, with the cursor where you type. Test by
typing through `Host::update`, not by setting fields.

**2.11 Settings rows explain themselves, validate as you type, and offer a check.** The
detail pane shows the live verdict while typing ("as you type": `✓ ...` or
`✗ reason`), then the latest check and current value before longer explanations,
accepted values and the config key (`section_detail_lines`). The host wraps and
scrolls this text inside the actual viewport. While capturing, the verdict stays
visible at the top without hiding the input or its cursor. `⏎` on a bad value is refused
with a reason ("not yet: ..."). After a change the service can confirm (token, server,
group), ask "check ... now?" with `Menu::confirm`; a check runs as `Effect::Spawn` and
reports ✓/✗ on the row and in the status line. Accept what people paste (a full URL as
well as a path). An "add" row (`+ add a tenancy`, hint of what it takes) is always the
first row, so it is visible that more can be added; removing asks first. *Why:* a
setting should say what it does, accept what people paste, and offer to check itself.

**2.11a Every part's first section is `connection`, laid out the same way.** Built with
`ui::connection`, in this order:

1. What was found (environment variables, or a file such as `.mcp.json`), each as a
   question: `? Found in your environment: a Jira site (JIRA_X_BASE_URL, JIRA_X_USERNAME,
   JIRA_X_API_TOKEN). Use it?` (`Found::row`). Its detail pane (`Found::detail`) says
   what was found, by name only, that radar stores only the names (`env:NAME`) and reads the
   values each time, never showing or saving them, and what choosing it does. `⏎` (or
   `y`) uses it. A part with several connections (Jira's sites) puts its `+ add …` row
   right under these.
2. Where: `server` (GitLab), `site` (Jira, one block per site), `workspace` (Slack).
3. `account` where the service asks for one, then `token` (`masked`: `env:NAME` by name,
   a typed-in token never whole).
4. `signed in as`, as the last check found.
5. `check`: what the last check said; `⏎` asks now.
6. `refresh`, shown as "every 1m" as the top bar says it (`connection::every`). Pausing
   when idle is the host's, under General (`pause if idle`), not a part row.
7. Then the part's own rows (GitLab's history, Slack's read back and permissions).

Every field's menu has `e change it` and, where the check confirms it, `c check the
connection`; `⏎` on a field goes straight to changing it (on a row that only checks, to
the check), as in Slack's other sections; the detail pane is `connection::Detail` (about, accepts, now, checked, as
you type, config key). *Why:* every part's settings read in one style, connection first,
and what radar found is a question, never a change made for you.

### Look and wording

**2.12 No chevrons, no `[+]`/`[-]`, no useless counts.** Folding is shown by tree lines and
indentation, not `▸▾` or `[+]`/`[-]`. Things that belong together share a band; colours
stay configurable. A part sends its tree as view data and Ink draws it: each row's own
tree line (`├─`/`└─`) comes before its text, with its room taken before the name is
fitted so columns stay aligned; there is no left-edge stripe. A `│` line runs past
a block that is not the last, with
background bands on group headings and on every other block (a repo with its merge
requests). Selection preserves semantic foregrounds and modifiers, using
`bg.selection` when focused and `bg.overlay` when unfocused; mono adds an accessible
selection marker through text modifiers. A folded node reserves a dim `⋯ n`, where
`n` counts held descendants only, not unfetched server totals. If the exact count
cannot fit, show only `⋯`, never a truncated number. The band, heading and line colours are look words (GitLab's `band`, `heading`,
`lines` in `config::default_styles`, group "repositories tree" in Settings, look); `band`
and `heading` are backgrounds, so only the slot's colour is used and the menu offers
colour and reset only. Panels are drawn on `bg.surface`, so a band must be a different
slot. A settings tree (GitLab's tenancies) follows the same pattern. A count appears
only where it changes a decision. Panels have titles that say what they are, and columns
have headings. *Why:* tree lines and bands show what belongs together better than
markers do, and a number nobody can act on is noise.

**2.13 Plain words, not filter jargon; views explained.** Say what a view or filter does
in a sentence (the SHOWS line at the foot of the list, `filter::describe`). Build conditions from menus; the
grammar is an advanced "edit as text" option. One word per state everywhere, with a symbol
plus the word so it reads without colour. *Why:* a filter written as grammar, or a view
with only a name, does not say what it shows.

**2.14 Do not show the same thing twice, and do not mix concepts.** No middle panel that
repeats the right one. Themes (palette and colours, the host) and looks (which slot a
word uses, the plugin) are separate sections with separate names. *Why:* repeated panels
waste the screen, and two ideas under one name confuse both.

**2.15 Group and keep information.** Hierarchies group and collapse (tenancy → repo → merge
request); a restyle never drops information the screen had. *Why:* a flat list hides
the structure, and a new look is no reason to lose facts.

**2.16 Say what an action applies to.** The selected row is visibly selected, and menus are
titled with it. *Why:* before acting you can see what the action is for.

### Architecture and repo hygiene

**2.17 Plugin-shaped.** Everything service-specific lives in the plugin crate; `radar-core`
changes only for something every plugin gains. *Why:* each new ability plugs in without
touching the others.

**2.18 Nothing visual is hard-coded.** No `Color::`, hex or style literals in plugin code
except slot defaults in `slots()`; words are styled by config spec; tones go through
`widgets::tone`. Check: `rg "Color::|Rgb\(|#[0-9a-f]{6}" crates/<plugin>/src` finds only
the slot table. *Why:* colour and style are the user's to configure.

**2.19 Succession, not reuse.** Do not port code, config keys or file formats from the
Python/Textual app radar replaced. *Why:* radar succeeds it; it does not carry it inside.

**2.20 No employer specifics anywhere in the repo**, including docs, tests, snapshots and
comments: tenancy paths, group names, people, ticket keys. New test data takes its names
from Terry Pratchett's Discworld: people (`Sam Vimes`, `vimes`), groups (`ankh-morpork/watch`),
ticket keys (`WATCH-12`). Older fixtures use `acme/team-a`, `alice` and `ABC-12`. The real
values live in `~/.config/radar/config.toml`. The hooks in `scripts/hooks` refuse a commit
that adds a word from the private list kept outside the repository.

**2.21 Use what the kit has before drawing by hand.** Trees, tables, inputs, menus,
spinners and scrolled bodies are view types the kit (1.15) draws once in radar's look for
every part; inputs are `TextInput`. Do not say a library is in use unless it is. *Why:* one
drawing of each element looks and behaves the same in every part.

**2.22 Works in every environment.** macOS, Linux (Wayland and X11), Windows, WSL, SSH,
tmux, no display. Open links and copy only through `Effect::Open`/`Effect::Copy`; never
assume a browser, a clipboard tool, a mouse or truecolor; say on the status line what
happened. Run in `NO_COLOR=1` and at 80x24. *Why:* people run radar on different systems,
and it should just work on each.

Opening a folder (`system::folder_ways`, tested per environment in `system.rs`). A
"terminal editor" is one that runs in a terminal (`nvim`, `hx`, `emacs -nw`, …); a
"window editor" has its own window (`code`, `idea`, `open -a "…"`).

| Where radar runs | Terminal (`t`) | Terminal editor | Window editor |
|---|---|---|---|
| inside tmux (also over SSH) | new tmux window in the folder | new tmux window running it | desktop: started on the folder; SSH: clipboard |
| Linux desktop (Wayland, X11) | your `terminal` setting, `$TERMINAL`, `xdg-terminal-exec`, `x-terminal-emulator` | the same, running it (`-e` or `{cmd}`) | started on the folder |
| macOS | `open -a Terminal <dir>` | Terminal.app via `osascript`, `cd … && editor .` | started on the folder (`open -a App` for apps without a command) |
| Windows | `wt -d <dir>`, then a `cmd` window | `wt -d <dir> editor .` | `cmd /C editor <dir>` |
| WSL | `wt.exe wsl.exe --cd <dir>` | the same with `--exec editor .` | started on the folder (the editor's WSL shim) |
| SSH without tmux, or no display | clipboard: `cd <dir>` | clipboard: `cd <dir> && editor .` | clipboard: `cd <dir>` |

"Clipboard" is `Effect::Copy` (OSC 52 over SSH) plus a status line saying why and what
to paste. A way that fails falls through to the next, and the last resort is always the
clipboard. A terminal editor is never run in radar's own terminal.

**2.23 Acting on a service or on this machine.** The service (or git) is the authority; radar
asks, then reports what it said. Writes behave one way in every part, through the kit
(`radar_core::writes`, `Menu::write_confirm`, `ui::composer`):
- **Every write is asked first and shown exactly**: `{verb} this on {item}?` ("post this
  on !263?", "post this on ABC-12?", "post this in #team, in Bob's thread?"). A write
  that sends text (a comment, a reply, a new ticket, a review ask) uses
  `Menu::write_confirm`: the whole text as lines, then the buttons `y {verb} it`,
  `e keep writing`, `n drop it` in a row (a question is a dialog, 1.9). One without
  text (approve, resolve, move, assign, react, rebase, merge) uses `Menu::write_plain`,
  its lines saying what it does. Nothing is sent before `y`;
  it is sent once and never retried.
- **Text is typed in the kit's composer** (`ui::composer::Composer`): `⏎` a new line,
  `ctrl-s` review and send, `esc` stop (asking first when there is text).
- **Text is never lost.** Declined (`n`) or refused text is kept as a draft that the key
  that wrote it brings back, and the notice says so (`· c has your text`).
- **One mode, decided in one place.** radar is in a mode (`writes::Mode`): **read only**
  (it changes nothing on any service), **write** (it may add: a comment or line comment,
  a reply, a reaction, a review ask or asking again, a new ticket) or **edit** (it may
  also change what is there: approve and unapprove, resolve and reopen, rebase, merge,
  move a ticket, assign). Each write says which it is (`writes::Change::Add` or `Edit`):
  on its question (`Menu::write_confirm` adds, `write_plain` edits, `.adds()`/`.edits()`
  to say otherwise), on its `x` row (`Binding::adds()`/`edits()`, `Action::adds()`) and
  where it sends (`writes::not_sent(change)`). `Mode::allows` decides. The host holds
  the mode and tells the thread (`writes::set_mode`): `[general] mode = "read-only" |
  "write" | "edit"` (default `edit`; settings, general, mode), or for one run `RADAR_MODE`,
  or the old switches `RADAR_NO_WRITES` and `RADAR_{PART}_NO_WRITES` (any value but empty or
  `0`), which mean read only. While the mode refuses a write, the host adds a note to
  its question before `y` ("radar is read only: nothing will be sent", "radar is in write
  mode: this needs edit mode, nothing will be sent"), its `x` row stays listed, muted,
  with `· radar is read only` or `· needs edit mode` (its key still opens the question),
  and `y` sends nothing: the part reports "not sent: radar is read only" as a refusal and
  keeps the text. The top bar shows the mode at all times (`◉ read only`, `✎ write`,
  `✎ edit`, the pack's icons); a click on it or the settings row opens its menu.
  Local actions (check out, an editor, a terminal) and
  settings are not service writes and work in every mode.
- **The words around it** are the kit's: `asking {Service} to {what}…` while it is sent,
  then `✓ {what was done}` or `✗ {Service} did not {what}: {why}` (`Service::asking`,
  `writes::done`, `Service::did_not`, `writes::kept`), with the pack's icons.
- Anything else irreversible on this machine (checking out a branch) asks first with
  `Menu::confirm`, naming the item and what will happen.
- An action that cannot be done now stays in the `x` menu, disabled in words ("merge ·
  needs 1 more approval"); choosing it says why on the status line and does nothing else.
- Re-check with the service just before an irreversible change (the merge queries the
  merge request again and merges only the commit it checked); show its refusal as given
  (`✗ merge refused: …` on the status line and in the page).
- Long or blocking work (git, scans, starting programs) runs in `Effect::Spawn` through
  `spawn_blocking`; the page shows it running (spinner, `● … asked of GitLab…`), and
  afterwards the page and the tree refresh.
- A local change refuses rather than risks work: a checkout refuses while tracked files
  have uncommitted changes and says so. git runs with prompts off
  (`GIT_TERMINAL_PROMPT=0`, `SSH_ASKPASS_REQUIRE=force`): it must never read radar's terminal.

- The same action reached from another place is the same code: the Repositories tree row
  (`list_actions.rs`) calls `act::ask_rebase`, `ask_merge` and `review::ask_approve`,
  which the page's `u m a` call (and the git part's `ask_checkout` for `c`, from either
  place), so the question, the mode's note, the one send and the refresh after cannot
  differ. On a
  tree row the keys are `u` rebase (when GitLab says it is behind), `m` merge, `e`
  approve, `c` check out; `a` is the middle pane's "show one author's merge requests", and
  `o p y` open, pipeline, copy. A row the detail pane lists is clickable: one click does what
  its key does (it is registered under the middle pane, or the detail pane when that has
  the focus, so it never costs a focusing click). A STATE cell is clickable only where
  it has an action (`needs rebase`, `ready to merge`, a failed pipeline).

**2.24 The agreed design wins.** Build what the agreed design shows; do not improvise a
different layout.

**2.25 Process.**
- Never commit or push unless asked.
- The workspace must build when you hand over, in debug and release.
- Rebuild before a live check: `cargo test` and `cargo clippy` do not rebuild
  `target/*/radar`.
- When you hand over, say where to run it and what to press.

**2.26 Keys are context dependent: the same action uses the same key in the same kind of
place, in every part.** Not one meaning per letter: a letter may mean different things
in different kinds of place, but never two things in one kind of place, and never two
letters for one thing.
- `r` refreshes: a list and an open page, in every part. (`R` is the Repositories
  screen.)
- Where there is a discussion (a Slack thread or message, a Jira ticket, a merge
  request's Changes and Threads tabs) `c` starts a comment or reply. On a merge request
  row or overview `c` still checks it out. The `x` menu and `?` say what `c`
  does in that place (the help bar holds navigation keys only, at most five hints).
- `o` opens in the service, `y` copies the link, `p` opens the pipeline. `Y` copies the
  ticket key (Jira).
- `a` picks a person. `/` filters the list by words, `:` goes to an item by typing part
  of its name. `z` pauses refreshing (host, 2.33). `space` folds and `⏎` goes in, from a
  focused detail pane too.
- A part's own actions are lower case (Jira: `t` move, `c` comment, `n` new ticket, `i` assign to me);
  **capitals are screens** (`R` Repositories, `G` GitHub, `J` Jira, `S` Slack, `B`
  Branches, `W` workspaces, `I` Work; the host's `H` history): the host takes a screen's capital first, so no
  part binds one of those (a settings list that reorders moves a row with `<` (up) and
  `>` (down), keys the host never takes, not `J`/`K`). Another capital is free, and a part uses it only as the shifted partner of
  a lower-case key that does a nearby thing (Jira's `Y` copies the ticket key beside `y`,
  which copies the link), handled in the part's own key handler; an action a part
  contributes to another's `x` menu or bulk list is never a capital (`host_key`).
  Digits, `q x ? b w z h l tab`, `` ` ``, arrows and the settings key `,` are the
  host's too (1.4, 2.5).
- Move a row up or down with the kit's `Move::from_key` (`j k`, arrows, `g G`, `Home`,
  `End`, in a part's settings and nav pane as in a list).
- A part that cannot do one of these says why on the status line instead of ignoring
  the key (Slack's `:` says go-to is not on Slack yet).
- A key that only exists in one part's kind of place (Slack's `m` mark read on a channel
  list, `m` merge on a merge request) is fine; the same letter on the same kind of
  place is not.
- The git part (1.11a) keeps to this. Its `n` new branch and `u` rebase (on a repo page)
  are lower case, and `d` deletes a branch or worktree in the Branches screen. `B` is the
  Branches screen, a screen's capital. Push has no key: it is the `x` row `push <branch>…`,
  because an action a part adds to another part's menu is never a capital. Prune is in
  the Branches screen's bulk list, not a key.

**2.27 Writes are the same in every part.** 2.23 is the rule. A part adds no switch or
wording of its own. Live checks run with `RADAR_MODE=read-only`.

**2.28 Times and people.**
- `time::ago` for "how long ago" (with " ago" in sentences only, and weeks:
  "2w", not "14d"), `ui::duration` for "how long". An absolute time comes from a kit
  helper (`time::when`) and is in local time, never UTC.
- Rows use `short_name` (it keeps a surname's particles); pages and detail panes use full
  names. The reader is "you" in rows, not their own name.
- The JSON of `radar <part> list` carries `updated` as RFC 3339, people as `{name, handle}`
  and a `url`.

**2.29 Lists look alike.**
- The nav pane has an icon per row. A count appears only where it decides something (GitLab:
  only on "Needs my action"). Group headings carry no counts unless one decides something.
- The nav pane's footer is titled "shows", in lowercase words, and a cut footer ends with "…".
- The middle pane's title is the nav row's name. The strip under it states the filter or
  scope, never the title again (not in capitals either). Strips such as "opened by" and
  "assigned to" are drawn with `widgets::heading`.
- The detail pane is titled by the item (GitLab "repo !iid", Jira the ticket key, Slack
  "author's message", no id); its relative time is in the body. Nav and detail widths
  come from one fit in the kit (`ui::detail_width`: 30%, 38..60).
- A tab with a number carries it as a badge only where it decides something, and hides it
  at 0.

**2.29a Panels and detail panes use one layout.** The host owns the pane positions and focus;
the part sends one panel for a list or one card for a
scrolling detail pane, and Ink draws the frame. Use the panel's inner width for content. The panel title names the
selected item or current view. Within a detail pane, start with the item's identity and
state, then group related details in sections: a part's view puts them in a
`view::Block::section` (or `section_of` for lines), which Ink draws as
a section with each row inset, and an action's line with its
key at the edge is `Block::keyed`. A section has a short noun title, an optional icon and a quiet rule;
put counts or recent state in its optional detail. Leave one blank row between the
identity and the first section, and between sections. Wrap rich content to the width
left after the two-cell inset. Keep section text on the panel surface and use the theme's
text slots; do not add a second bordered panel or a new colour just for a section.
At narrow widths, omit optional detail before clipping the title or content. A
contributed section keeps the provider's own line layout when it is already wrapped.
Lists, settings, and pages still use their own shared row, table, connection, and page helpers
inside the same panel frame. Actions live in the contextual `x` menu; a detail pane may
also show clickable actions with the same key and availability as that menu.

**2.30 Pages look alike.**
- The breadcrumb ends with the item. The title line carries full names and relative
  times. Body panels are titled by their tab, not by the key.
- Tab counts carry a unit where it helps ("· 2 open").
- One word and one icon for an open thread, everywhere.
- The `x` menu has the page's `]` and `[` tabs, `esc back to {where}`, and a group of
  write rows headed "on {Service}". `⏎` on a list's site or status row in the menu folds.
- An open page refreshes with its part's cycle.
- No key hints inside a panel (2.2): not in the detail pane, a page, a section's text or a
  settings detail.

**2.31 Contributions are drawn.** What a part offers other parts (`offers()`: chips,
columns, rows) the owner draws, in the middle and detail panes: chips after the name,
other parts' cells clickable and showing their subject. A contributor adds only to
points the owner offers; a part's join text is checked against `offers()` (1.13).

**2.32 Settings sections are named alike**, with the id equal to the name: `connection`,
then the part's own groupings (GitLab `tenancies`, Jira `projects`, Slack `channels`,
`review`, `alerts`), `views` where the part has views (Slack has none), and `look`;
The git part's `checkouts`, `editors`, `links` and `branches` are under its own group, `git`. An old id is still read from an existing
config. Colours live with their grouping (a site's colour with its projects). A look
section lists its looks under one heading style. "Check … now?" follows every change the
service can confirm. An ordered list (views, tenancies, alert rules) moves with `<`
(move up) and `>` (move down), offered in the row's menu only where the move is possible. The connection section shows `refresh`
as "every 1m", as the top bar does, and `signed in as` as the name with the handle muted
after it. A minimum with a reason says the reason ("10s: GitLab rate limits").

**2.33 Status, pause and logs.**
- A status-line count covers the whole part ("1 ticket"; singular and plural right).
- The top bar says one thing per part: "syncing {Part}", "synced {ago} ago" or "not
  synced yet". A failed source is a nav pane line, "✗ {source} could not sync: {why}", and
  "failed" is in the error colour in every part.
- Pause is the host's: `z` pauses every part with one notice and the top bar says so
  once; one "pause if idle" under General replaces per-part idle settings (an old
  `idle_minutes` is still read).
- Every log line starts with the part id (`gitlab `, `jira `, `slack `).
- Notices and check lines get their ✓ and ✗ through the kit (1.18), never as literals.
- The config lives in `$XDG_CONFIG_HOME/radar`, else `~/.config/radar`, on every platform.

**2.34 Links work.** Anything a page's text refers to is a link you can follow from the
keyboard, not just words: a ticket key, a merge request's or thread's link, a markdown
link, an address written out. On a text page `⏎` offers them (the merge request page:
1.12 "Links work"); on a list, `⏎` on the row. Following one shows the thing here when a
part owns it (its page opens, `esc` comes back) and opens the link the radar way when none
does. A part that draws text other parts know things in gathers them with
`cx.parts.recognise` while drawing and keeps them for the key. *Why:* text that names
something should take you to it.

---

## 3. Testing

The classified test inventory and replayable journeys are documented in
[`test-tiers.md`](test-tiers.md) and [`scenarios.md`](scenarios.md).

**Respect the testing pyramid.** Most tests are unit tests: fast, many, on one function or
one plugin. A few integration tests per part go through the host for the important paths.
A very small end-to-end smoke runs per mix. Never write a loop that rebuilds a host and
draws full frames for every row of every place: that is what made the suite take 25
minutes on a day-old project, and the same guarantee holds at unit speed.

| Level | What goes there | Where | Speed |
|---|---|---|---|
| Unit | logic: parsing, validation, config round trip, filter grammar; keys to actions; each `x` row's key is the part's and is taken, each click target it draws is taken, each action it adds to another part acts (`radar_core::check`, 3.2); hit helpers (full rows, padded labels), hover style, pointer sequences | `#[cfg(test)]` next to the code; no host, no full frame | ms each |
| Integration | a few per part, through the host: open a page, a write's confirm, a click on a row, the top bar, typing into an input; snapshots of each screen and section | the part's `tests.rs` (`hosted()`), radar-core's host tests | well under 1s each |
| End to end | per mix: start, each screen draws, a click and a key act, joined parts show each other's contributions; recordings only for the first screen of none, each alone and all; each other screen once, with all parts | `tests/mix.rs` only | under 1s per mix |

**Budgets, enforced.** Each crate's suite runs in under ~10s wall with `cargo nextest run
-p <crate>`. `scripts/check-mixes` is 15 feature builds (none, 8 parts alone, all, 4
adapters, shipped), each a check and one test binary; most of its time is compiling, so
it is not run while working on one part. `.config/nextest.toml` flags any
test over 2s as SLOW (and kills one over 20s): make it smaller or move it down the pyramid
(rebuild nothing per row, draw only the part's area, no real sleeps). Only the
whole-app sweep in `tests/` gets longer (`key_intent_parity`, which presses every key and
action on every surface: flagged at 10s, killed at 60s). The rest of `tests/` holds the
mix smoke (`mix`), the guard (`parts`, `plugin_boundary`, 1.14), recorded views
(`view_fixtures`, `reference_views`), the Ink renderer in a real PTY (`ink_pty`) and CLI tests (`work_cli`,
`slack_cli_history`, `mcp_relay`).

**A session working on one part tests only that crate** (`cargo nextest run -p radar-<part>`,
clippy on it). After a merge, test only the crates the merged branches changed; CI runs
the workspace suite and the mix check on the pull request. Before handing over:

```bash
cargo nextest run -p radar-<part>               # your crate, seconds (cargo test -p if no nextest)
cargo clippy -p radar-<part> --all-features --all-targets --keep-going -- -D warnings
cargo build --release                        # only when a live check needs the binary
scripts/check-mixes                          # when Cargo features, src/lib.rs or tests/mix.rs changed (1.14)
scripts/shots --compare main                 # when a colour, style or layout changed (3.1a)
```

A failing mix test is usually a recording that needs regenerating or a test that
looks a row up the old way; read it, fix the one test, and rerun that one mix
(`cargo nextest run -p radar --no-default-features --features <mix>,demo,test-support
--test mix`), not the whole check.

### 3.1 Unit tests and snapshots

- Plain unit tests next to the code (`#[cfg(test)] mod tests`): filter grammar, validation,
  classification, config round trip, system `Env` cases.
- Screens: build the real host and read the view data it returns as text.
  `radar_core::headless::text_of(&host, w, h)` (or `Headless::text()`) gives per pane a rule
  naming it and its title, then one line per row (`>` on the selected row, cells two spaces
  apart, chips in `[ ]`, the row's target in `{ }`), then the open menu and the status line's
  keys. It names no colour, width or cell, so a look change in Ink moves no part's snapshot.
  What a screen looks like is Ink's to test, from view fixtures, in vitest. The helpers in
  `crates/radar-gitlab/src/demo.rs` (`sample()`) and `lib.rs` (`hosted()`) are the pattern: a sample plugin
  with fixture data (`acme/team-a`, `alice`), wrapped in `radar_core::Host::new(..)`, then
  `host.load_config(Some(""))` and the sample put back (loading a config reconfigures the
  plugin). `radar_core::host::tests::Probe` is crate-private; write your own fixtures.
- Snapshot with `insta::assert_snapshot!(text_of(&host, 160, 40))` for
  the screen and for each settings section (`merge_requests`, `settings_tenancies`, ...),
  plus the `x` menu (`keys_menu`). A snapshot proves content and order, not layout or colour:
  for those, look at a picture (3.1a).
- **Read the diff before accepting a snapshot.** `INSTA_UPDATE=always cargo test` rewrites
  blindly. Run `cargo test`, read the old/new lines it prints, decide each change is
  intended, then accept (`cargo insta review` if installed, or `INSTA_UPDATE=always`
  for that crate) and look at `git diff` of the `.snap` files. A failed run leaves
  `*.snap.new` files (gitignored): delete the ones you do not accept.
- Read the view before pressing keys that depend on what is visible: the host knows a
  tree's rows only after the screen's view was built (`let _ = text_of(&host, 160, 40);`
  before each key, as the tests do).
- **A test that crosses crates is a smell.** A crate's tests test that crate, with its own
  fixtures. When a change in one crate makes another crate's tests fail, that failure is a
  coupling defect: the test leans on another crate's rendering, text or internals. Fix the
  test so it no longer depends across the boundary; do not re-record the other crate's
  snapshot. Only the mix tests in `tests/` and contract tests at a published boundary
  (view data, the IPC protocol) span crates on purpose.

### 3.1a Shots: seeing what was drawn

A text snapshot cannot show a wrong colour, a lost highlight or a missing bold. Shots can:
Ink draws a screen and every cell is kept at its colour.

- A place is a recorded view fixture (`ink/test/fixtures/*.json`, made from the mix
  recordings). `scripts/shots` has Ink draw each one through the whole App to styled
  cells (`ink/tools/shots/draw.test.tsx`), turns the cells into SVG (`ink/tools/shots/svg.ts`:
  full colour, bold, italic, underline, reversed and dim honoured, wide characters on two
  cells; fonts 0xProto Nerd Font Mono, Symbols Nerd Font Mono, JetBrainsMono Nerd Font,
  JetBrains Mono, then monospace, so icons draw) and then PNG with `rsvg-convert`. The
  same screen gives the same bytes, so two SVGs diff. Shots never touch a live service or
  your config.
- `scripts/shots` draws every place at 80x24 and 160x40 into `shots/` and writes
  `shots/index.html`: a grid with titles, a dark or light page behind it (the button at
  the top), SVGs one click away. `scripts/shots --compare <git-ref>` also draws the places
  from that ref's fixtures and Ink source (in a temporary worktree) and shows each place
  that changed as before | after, with unchanged ones folded away, new ones marked.
- **When to use them.** Changing a colour, a style, a highlight, a border or a layout in
  the kit or a part: run `scripts/shots --compare main` and look at what changed before you
  hand over. Reviewing a branch: open the compare page. Adding a kit element or a state:
  show it in a story (1.15). Ink's own tests (`ink/test`) fail when a colour moves; a
  part's screens keep text snapshots of their view data.

### 3.2 The row check: every `x` row and target, at unit speed

Each part has a table test over its places with `radar_core::check`
(`every_row_and_target_is_taken` in `crates/radar-gitlab/src/lib.rs`, and in the
`tests.rs` of radar-jira, radar-slack, radar-git and radar-workspace). It works on the plugin
alone, with no host: for each place it makes a fresh plugin, presses the keys that lead
there, builds the part's view, then

- for every row `keys()` (or `section_keys()`) offers: the key must not be one the host
  keeps (`check::host_keeps`), and pressed on a fresh plugin at that place it must be
  taken (`Outcome::Handled`), unless the host acts on it when the part does not
  (`check::host_falls_back`: `esc`, arrows, `PgUp`/`PgDn`, `+`/`-`, `⏎` on the nav pane);
- for every click target the view carries (not a pane's background): selected on a fresh
  plugin, it must be taken.

For target-sensitive popup keys, also exercise the real host: open `x`, deliver a
refresh or account/navigation change while it stays open, then choose by key,
Enter and mouse before/after redraw. Changed targets must not receive the old
action; unchanged targets and a deliberately reopened menu must still work.

`Check::actions(&subject)` does the same for what the part adds to another part's `x`
menu: each action must return effects. A place is built from a scope and keys:

```rust
use radar_core::check::{Check, Place};
let tree = || Place::screen("repos", Pane::Middle);
let places = vec![
    Place::screen("repos", Pane::Nav).down(2),     // a nav row
    tree().down(3).focus(Pane::Detail),             // a middle row's detail pane
    tree().down(3).enter().tab(2),                  // its page, third tab
    Place::section("views").down(1),                // a settings row
];
let make = || Box::new(sample()) as Box<dyn Plugin>;
let mut check = Check::new(&make);
let gaps = check.places(&places);
assert!(gaps.is_empty(), "{}", gaps.join("\n"));
```

When you add a screen, pane, tree node kind, page tab or settings section, add its place,
and keep the `checked > N` guard honest. The whole check stays under 2s; if it grows,
pick one place per kind of row rather than every row. The git part's sample has a
checkout of `acme/team-a/api` at `/work/acme/api`, so its rows are covered; `Effect::Spawn`
tasks are returned, never run, so no row reaches GitLab, git or a program. What it does
not see: a capital that another part's screen takes (the mix smoke checks that), and what
a row does once taken (the part's targeted tests).

### 3.2a Actions that change things

- Writes are tested at the request and answer level only, against recorded replies
  (GitLab's `client::recorded::Replay` in `mr_actions.rs` and `write.rs`, Jira's
  `client::recorded::Replay`, Slack's `client::Recorded`): the path and body, reading
  each answer and refusal, sent once. Each part also tests its writes' questions (the
  whole text, the notes, `e` keep writing), declined and refused text kept, the notices,
  and each mode against each write: allowed (sent once) or refused before sending (its
  question's note, its muted `x` row, "not sent"). Set the mode on the test's thread
  (`writes::set_mode`, or `host.set_env_mode(..)` on a hosted part); a test never reads
  the process's environment. radar-core tests the mode itself: `RADAR_MODE`, the old
  switches with `1`, `0`, empty and unset, `[general] mode`, the settings row and the
  top bar at 80 and 160 wide.
- Never send a write to a real server from a test. Run live checks with
  `RADAR_MODE=read-only` (or the old `RADAR_NO_WRITES=1`), which turns every write in every
  part into "not sent": open each write's question, see the mode's note, answer `y`,
  read "not sent". Never run a live check in write or edit mode.
- git is tested only on throwaway repositories the test makes under the system's temp
  folder (radar-git's `local.rs`: a bare "origin", a clone, a dirty tree), with
  `GIT_CONFIG_GLOBAL=/dev/null` for the setup commands.
- The tree row's actions are tested the same way (`list_actions/tests.rs`): a question is
  answered with the menu's value, the task a yes returns is dropped without being run,
  and each mode is set with `writes::set_mode`. Their keys must stay clear of the
  tree's own (`a o p y / . r : z`), the host's and every part's contributed rows
  (`v > s t n` today); `tests/mix.rs` shows the row in every mix.
- Live checks open the pages and menus and stop at the confirm with `n` (or, with
  `RADAR_MODE=read-only`, answer `y` to see "not sent"). `t` and `e` inside the check's own
  tmux server open windows in that server, which is safe.

### 3.3 Other tests every plugin needs

- Typing into every input through `host.update(Event::Key(..))`, including paste and a
  rejected value.
- Each validation rule, both outcomes.
- Mouse: the intents a click sends (`Intent::Select` on a row's target selects it,
  `Select` on `pane:<id>` only focuses, `Activate` acts, `Context` opens the row's `x`
  actions).
- The row check (2.5, 3.2) over a place of every kind your sample data has (give it a
  row of every kind: every page, tab and state you draw differently); a gap names the
  place and the row or target.
- A bus topic you emit and one you consume (`Effect::Emit` payload fields).
- `config()`/`configure()` round trip.
- `config()` must be cheap and deterministic, because the host calls it every frame.

### 3.4 The live check, against the real service

Do it at **160x40 and 80x24**, with your own tmux server, and kill the session afterwards.

```bash
S=$SCRATCH                                   # your scratch directory
mkdir -p $S/cfg $S/state
tmux -L radarcheck new-session -d -s radar -x 160 -y 40 \
  "XDG_CONFIG_HOME=$S/cfg XDG_STATE_HOME=$S/state COLORTERM=truecolor target/release/radar"
sleep 4
tmux -L radarcheck send-keys -t radar Down Down Enter x   # drive it
tmux -L radarcheck capture-pane -t radar -p               # read the screen
tmux -L radarcheck resize-window -t radar -x 80 -y 24     # then look again
tmux -L radarcheck kill-session -t radar                  # always
```

- A dedicated socket (`-L radarcheck`) means `kill-session`/`kill-server` cannot touch the
  user's tmux. Never run bare `tmux kill-server` or `tmux set -g` in his server.
- Use a scratch config and state directory so the user's `config.toml` and cache are not
  read or overwritten. Put a real token in the scratch config through `env:`; never print it.
- Also run once with `NO_COLOR=1` (everything must read without colour) and, for opening
  and copying, with `SSH_CONNECTION='1.2.3.4 1 5.6.7.8 22'` (the link must reach the
  clipboard and the status line must say so; `tmux set-clipboard on` is needed inside that
  server).
- Press each action in the `x` menu on a real item and see that it did what it says.
- `scripts/ink-live/ink-live` does the same for Ink and proves Ink drew the screen.
  `tests/ink_pty.rs` is the automated form.
- To prove an idle pause without waiting half an hour, set `idle_minutes = 1` under `[general]`
  in the scratch config and wait for `refreshing paused` in the scratch `radar.log`
  (`until grep -q "refreshing paused" $S/state/radar/radar.log; do sleep 2; done`).
- **The stand-in glab trick** (to prove a refused glab token is renewed once and then
  shown, never looped): put an executable `glab` first in `PATH` that records its
  arguments and prints a token GitLab refuses for `config get token`; run with
  `token = "glab"`; the record shows `config get token`, `auth status`, `config get token`
  once per tenancy, and the detail pane shows the 401.
- **The stand-in opener trick** (to prove that `o`/`p` open the right URL without a
  browser): put an executable called `xdg-open` first in `PATH` that records its
  arguments, unset `BROWSER` (it is tried first), run the session, press the key, read the
  file.

```bash
mkdir -p $S/bin
printf '#!/usr/bin/env bash\necho "$@" >> %s/opened.txt\n' "$S" > $S/bin/xdg-open
chmod +x $S/bin/xdg-open; rm -f $S/opened.txt
tmux -L radarcheck new-session -d -s radar -x 160 -y 40 "env -u BROWSER PATH=$S/bin:\$PATH target/release/radar"
# ... press the key ...; then:
cat $S/opened.txt
```

- The mouse is driven through tmux with SGR sequences sent as literal text (cells
  1-based): a move `tmux -L radarcheck send-keys -t radar -l $'\e[<35;X;YM'`, a click the press
  `$'\e[<0;X;YM'` then the release `$'\e[<0;X;Ym'`. `capture-pane -p -e` shows the colours
  (the hover background on the line under the pointer), and `RADAR_DEBUG=1` logs each press
  and release. tmux keeps OSC 22 to itself: to see the pointer's shapes, set `pointer =
  "on"` in the scratch config and record what radar writes with `tmux pipe-pane -o 'cat >>
  out.raw'`, then `grep -ao $'\e]22;[a-z]*' out.raw`.
- `send-keys … Escape 1` arrives as Alt-1 (tmux's escape-time): send `Escape` on its own,
  then the next key.
- Over SSH without tmux: start radar with `env -u TMUX SSH_CONNECTION='1.2.3.4 1 5.6.7.8 22'`
  and `tmux set -g set-clipboard on`; press `t` on a page with a checkout and read
  `tmux show-buffer`: it holds the `cd` line.

---

## 4. Adding a part: a Jira walkthrough

How `crates/radar-jira` is put together, as the pattern for the next part. Read its code
beside each step; keep names from real work out of everything you write (2.20).

### 4.1 Steps and files

1. **Crate.** `crates/radar-<id>/` with its `Cargo.toml` on the workspace's version fields
   and dependencies (`radar-core`, `anyhow`, `clap`, `reqwest`, `serde`, `serde_json`,
   `toml`; `insta` for tests). The root `Cargo.toml`'s `members = ["crates/*"]` takes it
   in; add it to `[workspace.dependencies]`.
2. **Register.** Behind a feature (1.14): `jira = ["dep:radar-jira"]`, added to `default`
   and to the root `demo` feature (`"radar-jira?/demo"`), `radar-jira = { workspace = true,
   optional = true }`, and `#[cfg(feature = "jira")]` lines in `src/lib.rs`. Start-up fails
   with a message if an id, screen key, section name or slot clashes.
3. **Module layout.** Jira's: `lib.rs` (the struct, `impl Plugin`, sync state), `config.rs`,
   `client.rs` (every request through a `Transport`, so tests replay recorded answers),
   `model.rs`, `screen.rs` and `view.rs` (the screen as view data), `page.rs` (the ticket
   page), `settings.rs`, `join.rs` (what it offers and adds, 1.13), `write.rs` and
   `write_dialog.rs` (writes, 2.23), `start.rs` (start work), `demo.rs` (the sample), tests
   in `tests.rs` and beside each module.
4. **Config table** `[jira]`, every field with `serde(default)` so an old file still loads:
   `refresh_secs`, `idle_minutes`, `on_start` (`ask` or `off`), `start_status`, `style`
   (word to slot spec), `[[jira.site]]` (`name`, `url`, `email`, `token` as `env:NAME` or a
   literal, masked when shown, `color`, `[[jira.site.project]]` with `key` and `ac_field`,
   and `from`, the variables a found site came from) and `[[jira.view]]` (`name`, `when`
   conditions or `jql`, and `site`). `configure(None)` gives a working empty part, and its
   `set_up()` says what is missing, so the screen shows the host's set-up page. Name the
   settings key through `radar_core::host::SETTINGS_KEY` in tests, never a literal.
5. **Screen.** `{id: "tickets", name: "jira", key: 'J'}`; the key is unique and capital.
   Same shape as Repositories: nav pane of views │ middle pane (a banded tree: site,
   status group, ticket) │ detail pane, `x` menu and `bulk` per 2.7 and 2.8. Every state
   word is a word with an icon and a style spec from `[jira.style]`.
6. **Settings sections** (group `jira`): `connection` (one block per site: url, email,
   token, signed in as, check, refresh; what was found in the environment as a question,
   2.11a), `projects` (each site's projects, with their colour), `views` and `look`.
7. **Slots.** `jira.<colour>` per site colour (seven, `SITE_SLOTS`), defaults in `slots()`;
   words use core slots (`status.warning`, …) through `[jira.style]`.
8. **Joining others.** Jira owns `ticket` (1.13) and adds to merge requests and pull
   requests what it finds by key in their title and branch: a column, a section, a Ticket
   tab and actions (`join.rs`), and a ticket column on a checkout's branch. It offers
   chips, a column, a section, a tab and actions on tickets, which GitLab, Slack, Git and
   the workspace part fill. On the bus it hears
   `workspace.started` (1.6) and asks once to start the ticket on Jira. No part imports
   another.
9. **CLI.** `radar jira list [--view NAME] [--filter TEXT] [--json]` and `radar jira show
   KEY`, the same data as the screen.
10. **Cache.** `snapshot()`/`restore()` the tickets, pages read and the signed-in account,
    so the first frame is full and nothing held is asked for again.
11. **Tests.** Fixtures with Discworld names (`WATCH-12`, `vimes`; older ones use `ABC-12`
    and `acme`); `sample()` and `hosted()`; snapshots at 80x24 and 160x40 for the screen and
    each section; the row check (`radar_core::check`, 3.2) with your places; typing,
    validation, mouse, writes against recorded replies, and each mode (3.2a); then the live
    check against the real service in a scratch config (3.4).
12. **Docs.** Add your topics to 1.6 and your kinds to 1.13 of this guide, the part's
    keys and settings to the user guide, and any new element to `docs/components.md`.

### 4.2 Pre-handoff checklist

- [ ] `cargo nextest run -p radar-<part>` (in its budget, nothing SLOW) and `cargo clippy
      -p radar-<part> --all-features --all-targets -- -D warnings` pass; CI (workspace
      suite, workspace clippy, `scripts/check-mixes`) passes on the pull request (3).
- [ ] It joins the others only through subjects, `Show` and contributions (1.13): no
      import or name of another part; nothing shown for a part that is absent; set up or
      not, it looks finished alone.
- [ ] No colour or hex literal outside `slots()` (2.18); no employer specifics (2.20); no
      old-app code or formats (2.19).
- [ ] The status line shows only the fixed bar (2.1); no footers or repeated letters (2.2).
- [ ] Panels have titles, columns have headings, no chevrons, no `[+]`/`[-]`, no useless
      counts (2.12); words are plain and views explain themselves (2.13).
- [ ] `x` is titled with the selection and lists only what works here; the row check
      covers every kind of place (2.7, 3.2). `b` only if there are bulk commands (2.8).
- [ ] `↑`/`↓` move through a focused detail pane; `PgUp`/`PgDn` scroll it (2.4).
- [ ] Every input is a `TextInput`, validates as you type, refuses with a reason, and the
      service-checkable ones ask to be checked (2.10, 2.11). Every config key has a row.
- [ ] Mouse: every row has a target, focus memory kept, `Activate` acts; the row check finds no gap (2.5, 3.2); no test is flagged SLOW (3).
- [ ] Links and copies go through `Effect::Open`/`Copy`; run with `NO_COLOR=1`, over
      `SSH_CONNECTION`, and with the stand-in opener (3.4).
- [ ] Anything that changes a service or this machine asks first, is disabled in words
      when it cannot be done, runs off the event loop and reports the answer (2.23);
      tested on requests and throwaway repositories only (3.2a).
- [ ] Snapshot diffs were read, not blind-accepted; no stray `*.snap.new`.
- [ ] Live check at 160x40 and 80x24 against the real service; session killed.
- [ ] Nothing committed or pushed; `git status` shows only what you meant to change.
- [ ] The hand-off says where to run it and what to press, what was verified live, and what
      was not.

---

## 5. Known issues

Found by reading the code, not yet fixed. Do not copy these patterns; fix them when you
touch the area.

- The status line shows `PgUp/PgDn scroll` on every screen, whether or not the plugin
  scrolls anything (`Host::status_hints`).
- Bulk menus share one id, `bulk`, across screens and sections; `GitlabPlugin::on_menu`
  tells them apart by value strings, so values must never overlap.
- Images cannot be seen through tmux (radar does not ask there unless `images = "on"`, and a
  live check's own tmux server has no terminal to answer): check them in Ghostty or kitty
  directly, or through herdr. Their protocol output is covered by `ui::image`'s tests.
- Incremental syncs cannot see a merge request that was deleted; `r` catches it.
- Go to (`:`) is each part's own; Slack answers it with "not on Slack yet".
- Counts to review against 2.12: the tree title ("N repos[ with merge requests]"), the
  tenancy rows ("N repos · M MRs") and the nav pane's per-view counts.
- A plugin cannot give its theme slots a plain-words label (`slot_info` says "declared by a
  plugin").
- A `Binding` whose key `parse_key` cannot parse is dropped from the `x` menu without a
  warning. Reserved `MenuItem` shortcuts are not rejected: `q` is intercepted by dismissal
  and `j`/`k` would shadow movement, although Enter or mouse can still select the row.
  Shipped menu items avoid these keys; future plugin rows need the same check.
