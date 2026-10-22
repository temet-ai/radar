# Keys

Back to the [user guide](README.md).

This page collects the shared keys and the main keys for each part. In radar, `x` (or `?`) lists the keys that work where you are,
and the status line shows the main ones. How keys are chosen is in
[the key model](README.md#the-key-model).

## Everywhere

| Key | Does |
|---|---|
| `↑` `↓` `←` `→` or `k` `j` `h` `l` | move |
| `tab` / `shift-tab` | next / previous pane |
| `1` `2` `3` | the pane at that position |
| `⏎` | go in; open the row's page |
| `esc` | back one level; close a menu; stop typing; clear a filter |
| `PgUp` `PgDn` | scroll the detail pane |
| `x` or `?` | the menu for where you are |
| `ctrl-p` | command palette: find a screen, setting, action or cached item |
| `~` | recent notices from this session |
| `b` | bulk actions on everything shown |
| `z` | pause or resume refreshing |
| `q` | quit (asks first when a setting is not saved) |
| `ctrl-c` | quit, including while typing; with a mouse text selection showing, copy it instead |

Global navigation shortcuts apply while navigating; editors, menus and a focused agent
dock keep their own keys. The [page directory](pages.md) links each detailed guide.

In the parts' lists and pages:

| Key | Does |
|---|---|
| `/` | filter the list by words (GitLab, Jira, Slack, Work) |
| `r` | refresh a list or an open page |
| `o` | open in the service |
| `y` | copy the link |
| `A` | ask an agent about the row (GitLab, Jira, Slack, Branches) |

## Screens

| Key | Screen |
|---|---|
| `R` | Repositories; preserves the selected source when already on this screen |
| `G` | GitHub source of Repositories |
| `W` | Local workspaces source of Repositories |
| `J` | Jira |
| `S` | Slack |
| `B` | Branches |
| `I` | Work |
| `H` | History |
| `,` | Settings |

`↑` from the top of a pane reaches the top bar. There `←` `→` switch screens, and `↓`, `⏎` or
`esc` go back in.

## Trees and groups

| Key | Does |
|---|---|
| `space` | fold or unfold the group |
| `+` or `=` | expand the group |
| `-` | collapse the group (on a row inside one, collapses that group) |

## Pages

| Key | Does |
|---|---|
| `[` `]` | previous / next tab |
| `↑` at the top | go to the tab row; `←` `→` move along it, `↓` goes back in |
| `esc` | back to the list, with your place kept |

## Text and questions

These controls apply to comment/reply composers and their confirmation questions.
Single-line fields have the separate editing controls below.

| Key | Does |
|---|---|
| `⏎` | new line in a multiline composer; review/advance in a one-line composer |
| `ctrl-s` | review and send |
| `esc` | stop writing (asks first when you have typed something) |
| `y` | on a question: do it |
| `e` | on a question about text: go back to editing |
| `n` | on a question: drop it |
| `ctrl-r` | in GitLab, Jira and Slack: the text you declined to send, to copy back or drop |

### Single-line fields

Settings values, searches and other one-line inputs use these editing controls:

| Key | Does |
|---|---|
| `⏎` / `esc` | submit the value / cancel editing |
| `←` `→` | move the caret |
| `Home` / `ctrl-a`, `End` / `ctrl-e` | start / end of the field |
| `ctrl-←` `ctrl-→` or `alt-←` `alt-→` | move by word |
| `alt-b` `alt-f` | previous / next word |
| `shift` with arrows, `Home` or `End` | extend a selection; typing replaces it |
| `Backspace` / `Delete` | delete before / after the caret, or the selection |
| `ctrl-u` / `ctrl-k` | delete to the start / end |
| `ctrl-w` or `alt-Backspace` | delete the previous word |
| `alt-d` | delete the next word |
| `ctrl-z` | undo the last edit |
| `ctrl-shift-z` or `ctrl-r` | redo |

Pasting into a one-line field turns line breaks into spaces. Multiline comment and
reply composers preserve pasted lines. While editing a field, `ctrl-r` means redo;
leave the editor first to use a part's retained-text recovery shortcut. Terminal
support determines which modified key combinations reach radar.

## Setup

| Key | Does |
|---|---|
| `⏎` | continue; in the list of what to set up, go to that row's step |
| `space` | tick a row, or choose it |
| `a` `n` | in the list of what to set up: tick all, tick none |
| `esc` | skip this step |
| `←` | back a step |
| `f` | finish later: keeps your answers and leaves setup |
| `s` | on the set-up page of a part not set up yet (reached from another part's link): walk its setup step; `⏎` opens its settings section |

## GitLab

On a merge request:

| Key | Does |
|---|---|
| `u` | rebase onto the target |
| `m` | merge |
| `e` | approve, or take your approval back (`a` on the merge request's page) |
| `p` | open the pipeline |
| `i` | review it locally with an agent |
| `c` | on a page's Changes or Threads tab: write a comment, or reply in the thread |
| `d` | on a thread: resolve it, or open it again |

In the list:

| Key | Does |
|---|---|
| `a` | show one author's merge requests |
| `:` | go to a group, repository or merge request by part of its name or number |
| `.` | in a view: show only the repositories that have merge requests in it, and back |

Where the repository or merge request is checked out on this machine:

| Key | Does |
|---|---|
| `c` | check out the merge request's branch |
| `e` | open an editor there (on a merge request's row, `e` approves instead) |
| `t` | open a terminal there |
| `s` | look for checkouts again |
| `n` | new branch |

In a diff:

| Key | Does |
|---|---|
| `v` | side by side |
| `f` | collapse the files beside it |
| `n` `N` | next / previous file |
| `}` `{` | next / previous hunk |

On a job's log page:

| Key | Does |
|---|---|
| `/` | search the log |
| `n` `N` | next / previous line with the search words |
| `space` | fold or open the section the cursor is in |
| `g` | go to the start, loading the part before it |
| `c` | copy the lines around the first error (or the last lines) |

## Jira

| Key | Does |
|---|---|
| `n` | new ticket |
| `t` | move the ticket to another status |
| `i` | assign it to me |
| `c` | comment |
| `a` | show one assignee's tickets |
| `:` | go to a ticket by its key or words |
| `Y` | copy the ticket's key |

## Slack

| Key | Does |
|---|---|
| `c` | reply or write |
| `m` | mark read up to the selected message (radar's own mark; Slack is not told) |
| `e` | react to a message |
| `v` | on a review ask: review its linked merge request locally |

On a review ask you are drafting:

| Key | Does |
|---|---|
| `p` | post it |
| `e` | edit the words for the selected item |
| `d` | take the selected item out of it |
| `c` | post it in another channel |
| `y` | copy its text |
| `n` | drop the draft |

## Branches

| Key | Does |
|---|---|
| `d` | delete the branch or worktree (asks first) |
| `r` | read the clones again |
| `b` | bulk menu, with "prune merged and gone" |

## Work

| Key | Does |
|---|---|
| `n` | new task |
| `t` | set a local group label |
| `[` `]` | previous / next group or item page, depending on the focused pane |
| `v` | read original evidence |
| `c` | in source detail: copy the scoped source locator |
| `r` | refresh, or review a retained pending change |

## History

| Key | Does |
|---|---|
| `n` `p` | next / previous page |
| `r` | fresh snapshot |
| `a` | all recorded activity |
| `t` | recent activity, the last 24 hours |
| `c` | events sharing the selected event's causal root |
| `i` | events for the selected exact action ID |
| `j` `k` | choose a row |
| `tab` | switch between events and detail |
| `esc` | back |

## Recent notices

| Key | Does |
|---|---|
| `~` or `esc` | close recent notices |
| `↑` `↓` or `k` `j` | select a notice |
| `Home` / `g`, `End` / `G` | first / last notice |
| `PgUp` `PgDn` | scroll the selected notice's detail |

Notices last only for the current session; [History](history.md) is the persistent record.

## Settings

| Key | Does |
|---|---|
| `⏎` | change the row; on a check row, run the check |
| `<` `>` | move a row up or down, where order matters (tenancies, views) |
| `PgUp` `PgDn` | scroll the explanation |
| `p` | on an agent provider: paths and folders; on working/review folders: repository overrides |
| `o` | on a Git link: its worktree-folder template |

## Agent dock

| Key | Does |
|---|---|
| `` ` `` (backtick) | show the dock, or go to a session waiting for you; in an empty composer, hide it. Work and item pages do not draw the dock; on an item page this changes its visibility for supported screens while the keys stay with the page |
| `/` in the composer | the session's commands: `↑` `↓` choose, `tab` completes the name, `⏎` runs it, a click on a row runs it |
| `ctrl-t` | new session |
| `alt-f` | the dock takes the whole middle column, and back |
| `alt-w` | close the session tab (`ctrl-w` too, while the composer is empty) |
| `alt-←` `alt-→` | previous / next session tab |
| `alt-a` | choose the provider's named agent for new sessions |
| `alt-m` | choose the model for new sessions |
| `alt-s` | choose a session to open |
| `alt-k` | put a skill's request in the composer |
| `alt-o` | fold or open the latest tool call |
| `alt-r` | on a tab that is not attached: read the session back from its agent |
| `esc` | with a question waiting, decline it; while a turn runs, stop it; otherwise give the keys back to the lists |
| `a` `s` `l` `d` `v` | on a request, with the composer empty: the [approval](agents.md#approvals) choices |

## Mouse

| Action | Does |
|---|---|
| click | select; on a tab, a menu row or a status line key, act |
| click twice | open (same as `⏎`) |
| right-click a row | open its `x` menu; right-click another row to move it, or empty space to close it |
| wheel | scroll the pane under the pointer |
| drag | select text inside the pane the drag started in; letting go copies it |
| double-click in text | select the word under the pointer (a path or a link whole) and copy it |
| triple-click in text | select the line and copy it |
| drag the dock's top edge | resize the dock |

### Selecting text

| Key | Does |
|---|---|
| `shift`+`←` `→` `↑` `↓` | with a selection showing: move its end a cell or a row, inside the pane |
| `shift`+`Home` `End` | the selection's end goes to the left or right edge of its row |
| `ctrl+c` | with a selection showing: copy it again (otherwise it quits) |
| any other key, a click, the wheel | clear the selection; the key does what it always does |

While a drag selects, the status line names the key that makes your terminal select its own
text instead of radar: `shift`, or `option` in iTerm2, `fn` in Terminal.app.
