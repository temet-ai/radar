# radar user guide

This guide shows how to set radar up and work in it. What radar is for, and how to install
it, are in the [README](../../README.md).

New to radar? Start with [Get started with radar](quickstart.md), a short walkthrough
using a local Git checkout.

**On this page**

1. [First run](#1-first-run)
2. [Getting around](#2-getting-around)
3. [The parts](#3-the-parts)
4. [Agents](#4-agents)
5. [Settings and the config file](#5-settings-and-the-config-file)
6. [The command line](#6-the-command-line)

More pages: [Keys](keys.md) (every key on one page), [Connect your services](connect.md),
[Agents](agents.md), [MCP and automatic local reviews](mcp-and-reviews.md), [What radar stores and sends](privacy.md) and
[When something looks wrong](troubleshooting.md).

---

## 1. First run

Start radar:

```sh
radar
```

radar opens full screen in your terminal. For a moment a radar sweep finds each part it has,
with what each holds for you, and a new line greets you each start.

![radar's start screen](images/splash.png)

Any key skips it, and the screen under it takes that key. Skipped in the first second and a
half, its line shows on the status line for a few seconds instead. Terminals that can draw
pictures (kitty's graphics) get a sharper, pixel-drawn scope. `splash` under Settings,
General, colours turns it off.

### Setup

The first time (when there is no config file), radar walks you through setup. Each step is
one page: what you choose on the left, help for that step on the right, and `step n of m`
at the top. Nothing is sent to a service until you connect it.

1. **Welcome.** What radar is and what setup will do. Choose `yes, let's set it up`, or
   `not now` to go straight to radar.
2. **Look for keys?** radar asks whether it may look for keys you already have. It reads the
   *names* of variables such as `GITLAB_TOKEN` and `JIRA_…`, and whether the sign-in files of
   `glab`, `gh`, `claude` and `codex` exist, not their contents. Choose `yes, look` or
   `no thanks`. It asks every time setup runs.

   ![Setup's first question: may radar look for keys, and what it reads](images/setup-scan.png)

3. **Choose.** A list of what can be set up, under this machine, services, agents and
   optional. What the look found is ticked already. `space` ticks a row, `a` ticks all, `n`
   none; `⏎` starts at the row you are on, one step for each tick.

   ![Setup: choosing what to set up](images/setup-choose.png)

4. **One step per tick**, in the list's order:
   * **Local code**: the folders radar looks in for your Git checkouts. It offers folders it
     finds, such as `~/projects`.
   * **Each service** you ticked: its connection section from Settings, the same fields,
     with help beside it on how to get a token. [Connect your services](connect.md) has
     each one in full.

     ![Setup's Slack step: a token found in the environment, the connection, and how to make a token](images/setup-slack.png)

   * **Agents**: which of Claude Code, Codex and Copilot are installed and signed in, and
     which one new sessions use ([Agents](agents.md)).
   * **Look & terminal**: what radar found about your terminal, then the icons (`nerd` needs
     a Nerd Font; `plain` works in any font), the theme, and how much motion. `space`
     applies a choice at once.

     ![Setup: look and terminal](images/setup-look.png)

5. **Mode.** Which writes radar may send: read only, write or edit
   ([the mode](#the-mode)). Every setup ends with this step, and it cannot be skipped.
6. **Ready.** What was set up, where the config is saved, and the mode. Ready offers a short
   tour of the screens; `radar setup tour` shows it again.

   ![Setup: ready](images/setup-ready.png)

`esc` skips a step, `←` goes back one, and `f` leaves setup and keeps your answers; the
status line then says where you stopped. The rest of setup's keys are in
[Keys](keys.md#setup).

A part you have not set up says so on its screen; `s` there walks its setup step, and `⏎`
opens its section in Settings. Run setup again whenever you like, or open one step on its
own:

```sh
radar setup            # from the start
radar setup gitlab     # one step: welcome, scan, choose, look, local, gitlab, github,
                       # jira, slack, agents, mode, ready, tour
```

Everything setup sets is also in Settings (`,`).

---

## 2. Getting around

### The screen

* **Top bar.** One tab per screen: Repositories, Jira, Slack, Branches, Work, History,
  Settings. A screen's key is the letter underlined in its name. On the right radar shows its
  [mode](#the-mode) and when it last synced.
* **Three panes.** Most screens have views on the left, the items in the middle, and the
  detail of the selected item on the right. `⏎` on an item opens its page.
* **Status line.** The bottom line shows the keys that work where you are. Every key on it
  can be clicked.
* **Agent dock.** When you start an agent, it appears under the middle pane (see
  [Agents](#4-agents)).

### The key model

radar follows three rules, so you can guess keys.

1. **A key is underlined inside its word**: the `b` in b̲ulk, the `q` in q̲uit. A key that is a
   symbol (`↑` `↓` `⏎` `esc`) comes before its word.
2. **The same key does the same thing in the same kind of place.** `r` refreshes a list or an
   open page in every part. `o` opens the item in its service. `y` copies its link. `c`
   writes a comment where there is a discussion. Keys can differ between *kinds* of place.
3. **Everything can be reached from the menu.** `x` (or `?`) lists every key for where you
   are. Choose a row and radar presses its key.

Every key is listed in [Keys](keys.md).

### Menus

Press `x` and a menu opens beside the selection. It lists what you can do *here*, with the
key beside each row. A row that cannot be done now stays in the list, muted, with the reason
(for example "radar is read only" or "needs 1 approval").

![A row's action menu](images/menu-row-actions.png)

### Pages

`⏎` on a row opens its **page**. A page has tabs: a merge request has Overview, Changes,
Threads, Pipeline, Commits and Ticket. `[` and `]` switch tabs, and `esc` goes back to the
list with your place kept.

![A merge request page](images/detail-merge-request.png)

![A Jira ticket page](images/detail-jira-issue.png)

### Finding things

* `/` filters the list by words. In GitLab you can also write terms such as `state:merged`
  or `label:bug`.
* `:` goes to a group, repository or merge request in GitLab, or a ticket in Jira, by name,
  key or number.
* `a` shows one author's merge requests in GitLab, or one assignee's tickets in Jira.

### The mouse

The mouse does what the keys do. What a click would act on is lit under the pointer. A click
selects, two clicks open, right-click on a row opens its `x` menu, and the wheel scrolls the
pane under the pointer. Right-click needs a terminal that passes right-clicks through;
`radar terminal`, and Settings, General, parts, show what radar detected.

Dragging selects text inside the pane it started in, and letting go copies it. The status
line says how it was copied (`copied 42 characters · wl-copy`, or `· terminal` when your
terminal's clipboard took it). In text, a double-click selects a word, a path or a link
whole, and a triple-click a line. [Keys](keys.md#mouse) has the rest.

### Links and pictures

Links in descriptions, tickets and messages are terminal hyperlinks: Ctrl-click (Cmd-click on
a Mac) opens one, in terminals that support them. Pictures in Slack threads, merge request
descriptions and tickets are drawn in the terminal where it supports kitty, sixel or iTerm2
graphics; elsewhere a line names the picture and its link. Both can be set under Settings,
General, colours.

Opening links and copying work on macOS, Linux (Wayland and X11), Windows under WSL2, inside
tmux and over SSH. Where radar cannot reach a browser itself (over SSH, say), it puts the
link on your clipboard through the terminal and says so on the status line. `$BROWSER` is
used first when set.

### The mode

The top bar shows radar's mode on the right. A click on it, or Settings, General, mode,
changes it.

| Mode | What radar may send |
|---|---|
| read only | nothing. A write still shows its question, and `y` sends nothing |
| write | additions: comments, replies, reactions, review asks, new tickets |
| edit (the default) | also changes: approve, resolve, rebase, merge, move or assign tickets |

`RADAR_MODE=read-only radar` (or `write`, `edit`) sets the mode for one run.

Every write is shown in full first, and is sent when you press `y`; `e` goes back to
editing. Text is typed in the same small editor everywhere ([keys](keys.md#text-and-questions)).
Text you decline is kept, and the key that wrote it brings it back.

---

## 3. The parts

Each service, and your local Git, is a part of radar with a screen of its own. The keys for
each are in [Keys](keys.md).

### GitLab

Press `R`. This is the merge request screen.

![GitLab on the Repositories screen](images/repositories.png)

* **Left:** views, such as "Everything", "Needs my action", "Opened by me".
* **Middle:** a tree of your groups, repositories and open merge requests, with state (needs
  approval, ready to merge, pipeline failed), age and what is waiting for you.
* **Right:** detail of the selected group, repository or merge request, and its actions.

Views show open merge requests unless a filter says otherwise. `state:merged`,
`state:closed` or `state:all` add merged and closed ones from the last 14 days (the
**merged, closed** row in the GitLab connection changes the number of days).

On a merge request you can rebase, merge, approve, open the pipeline, check out its branch,
comment on the Changes and Threads tabs, and resolve threads. Pipelines and job logs have
their own pages. After the first sync, radar asks GitLab only for what changed; `r` fetches
everything again.

### GitHub

Press `G` (or `R` and pick GitHub). Repositories, open pull requests, the checks on a pull
request's head commit, and Actions runs.

![GitHub](images/github.png)

GitHub is read only: radar does not rerun, cancel, review or merge anything there. "No
checks" is not shown as "passed", and a list radar could not read in full says so.

### Jira

Press `J`. Views are your saved queries. The middle pane is a tree: site, then status, then
tickets. A ticket row shows its type, status, the merge request linked to it and its age.

![Jira](images/jira.png)

You can create tickets, comment, move a ticket to another status and assign it to yourself.
Comments and new tickets need write mode; moves and assignments need edit mode.

### Slack

Press `S`. Slack shows the threads in the channels you watch, mentions of you, and review
asks, newest first. Messages read like a compact chat: one line a message, with the author
left blank when the same person writes again within five minutes.

![Slack, For you](images/slack.png)

![Slack, the #team channel](images/slack-channel.png)

You can reply, react, and mark a thread read up to a message (a mark radar keeps; Slack is
not told). A review ask can be drafted in radar and posted to a channel. `:name:` emoji are
shown as emoji. For a roomier layout, set `density` to `cozy` in Settings, Slack, look.

### Git (Branches)

Press `B`. This is your local clones, their worktrees and their branches, with how far each
is ahead of or behind its remote, which are dirty, and your stashes.

![Branches](images/branches.png)

* **Left:** all, dirty, ahead of remote, stashes.
* **Middle:** a tree of clone, then worktrees, then branches.

`d` deletes a branch or worktree; radar asks first, says what would be lost, and will not
delete the current branch. The bulk menu (`b`) has "prune merged and gone".

Tell radar where your code is in Settings, Git, **checkouts**. radar then shows, on a merge
request or repository, where it is checked out, and from there checks out the branch, opens
an editor or opens a terminal. Editors per project type are under Settings, Git,
**editors**. A terminal opens as a new tmux window when radar runs inside tmux; otherwise it
opens in the terminal set in Settings, Git, **terminal**, or one radar finds on this
machine.

### Workspaces

Press `R`, then `W` (the "Local workspaces" source). A workspace is a top folder that holds
your clones, for example `~/work/acme`. Add them in Settings, Workspace, **workspaces**.
radar also offers folders it finds, as a question.

![Local workspaces](images/local-workspaces.png)

A workspace lets you start work on a ticket: you choose the ticket's repositories, and radar
makes the branches (or parked worktrees) and a README for the ticket, asking first.

### Work

Press `I`. Work is a list of your own local work items: tasks you create (`n`), and things
you add from a service's `x` menu (**add to work item…**). Labels and links appear only when
you add them.

![Work](images/work.png)

### History

Press `H`. radar keeps a journal on your machine of what it did and what happened: syncs,
checks, opens, and every write you confirmed or declined. History shows it, oldest first,
50 events a page, opening on the last 24 hours. From any item's `x` menu, **history for
selected item** shows just that item. [What radar stores and sends](privacy.md#files) says
what the journal records.

![History](images/history.png)

---

## 4. Agents

radar runs Claude Agent, Codex and Copilot sessions in a dock under the middle pane, on
every screen. `A` on a row starts a session with that item described to the agent, and `i`
on a merge request starts a local review. Each agent needs its own program installed and
signed in. [Agents](agents.md) covers what each needs, the dock, approvals and the agent
settings.

---

## 5. Settings and the config file

### Settings

Press `,`. Settings is a tree on the left, the section in the middle and an explanation of
the selected row on the right. Everything saves as you go.

![Settings, parts](images/settings.general.parts.png)

| Group | Sections |
|---|---|
| General | parts, mode, appearance, colours, icons |
| GitLab | connection, tenancies, views, look |
| GitHub | connection, repositories |
| Jira | connection, projects, views, look |
| Slack | connection, channels, review, alerts, look |
| Git | checkouts, terminal, editors, links, branches |
| Workspace | workspaces |
| Agents | tools and defaults |

`parts` shows which parts this radar has, whether each is set up, and the terminal radar
detected. `mode` sets the [mode](#the-mode) and the pause when idle. `appearance` is setup's
look step: icons, theme and motion. `colours` and `icons` are below.

The right pane says what a row does, what it accepts and which config key it writes. A bad
value is refused with a reason.

### The config file

```sh
radar config path     # where it is
radar config show     # what radar would write, in full
```

It is a TOML file at `~/.config/radar/config.toml` (or `$XDG_CONFIG_HOME/radar/config.toml`),
on macOS too. radar writes it after each completed change in Settings and reads it when it
starts, so you can also edit it by hand. The tables are named after the parts: `[gitlab]`,
`[github]`, `[jira]`, `[slack]`, `[git]`, `[workspace]`, `[sessions]`, `[theme]`,
`[general]` and `[icons]`. Every row in Settings names its key in the right pane (for
example `config: [gitlab] url`).

A small example:

```toml
[gitlab]
url = "https://gitlab.com"
token = "env:GITLAB_TOKEN"

[general]
mode = "read-only"     # or "write" or "edit"
idle_minutes = 30      # 0 never pauses

[theme]
palette = "silkcircuit-neon"
motion = "subtle"
```

### Colours

Settings, General, **colours** has a palette picker (39 palettes) and every colour slot as a
hex value, such as `#e135ff`. Colours you set stay set when you change the palette.

| Setting (`[theme]`) | Values |
|---|---|
| `palette` | a palette name |
| `motion` | `subtle` (the default), `full`, `off` |
| `splash` | `auto`, `off` |
| `hyperlinks`, `images`, `pointer` | `auto`, `on`, `off` |
| `icons` | `auto`, `nerd`, `unicode`, `emoji`, `off`, or your own pack |
| `colors` | `auto` (detect), `truecolor`, `256`, `16`, `none`: how many colours radar uses |

**Motion.** `subtle` fades a screen in, sweeps menus open and makes rows that a sync changed
glow for a moment. `full` adds sliding pages, rows sweeping in and a shimmer on panels
waiting for data. `off` has none. A key always acts at once. Motion is off over SSH, below
full colour, and on terminals too slow to draw it.

### Icons

radar draws icons beside rows, states, tabs and menus. `auto` picks `nerd` (Nerd Font
symbols) in terminals that bring them (Ghostty, kitty, WezTerm) and `unicode` elsewhere and
over SSH. If you see empty boxes, set the pack to `unicode`, or install a Nerd Font.

Settings, General, **icons** lists every icon: `⏎` types your own, `r` gives it back to the
pack. You can also make a whole pack:

```sh
radar icons list                    # every icon and its glyph
radar icons export nerd > mine.toml # a pack as a start
radar icons path                    # the folder your packs go in
```

A pack can `extends` another and set only the icons it wants. Icons you change in Settings
are saved in the config file's own `[icons]` table (an icon's name and your glyph); they go
on top of the chosen pack.

### The start line

Each start shows a new line under the mark: a joke, a tip, or something radar already holds
for you ("3 merge requests wait for your review"). radar does not ask a service for a line.

```toml
[start_lines]
show = true                       # false: no line at all
tips = true                       # false: no tips
off = ["time"]                    # groups not shown
mine = ["Coffee first."]          # your own lines
weights = { mine = 30 }           # how often each group is drawn
```

The lines and their groups are listed in the file radar ships
([`start-lines.toml`](../../crates/radar-core/assets/start-lines.toml)). Your own file,
`~/.config/radar/start-lines.toml`, may add lines in the same form.

### Refresh and idle

Each part has a refresh time in its connection section. After 30 idle minutes radar stops
refreshing; any key resumes it. Change the time in Settings, General, mode (`pause if
idle`). `z` pauses or resumes by hand.

---

## 6. The command line

Several parts print their lists without opening the screen, for scripts or a quick look.
The part commands (`gitlab`, `github`, `jira`, `slack`) send no writes, and use the same
config and tokens as the screen.

```sh
radar gitlab list [--view NAME] [--filter TEXT] [--json]
radar gitlab show '!263' [--json]        # or repo!263 when several repositories have one
radar github list [--json]
radar jira list [--view NAME] [--site NAME] [--filter TEXT] [--json]
radar jira show ABC-12 [--site NAME] [--json]
radar slack list [--view CHANNEL] [--filter TEXT] [--json]
radar slack catchup [CHANNEL] [--json]
radar config path | show
radar setup [STEP]                       # opens radar on setup, from the start or one step
radar icons list | export PACK | path
radar terminal                           # the terminal layers radar detected
radar licenses                           # radar's licence and the licences of what it carries
radar work id | apply | handoff | items | events | tag | change | resolve
```

`--filter` takes the same words as `/` in the screen. `radar work` is for tools: each
subcommand except `id` takes `--stdin` and reads one JSON object on standard input. `id`
prints a new UUID; `apply` records local work annotations; `handoff`, `items`, `events`,
`tag`, `change` and `resolve` look up what was recorded. `radar work --help` describes them.
