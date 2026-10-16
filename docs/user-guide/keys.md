# Keys

Back to the [user guide](README.md).

Press `x` (or `?`) anywhere in radar for the keys that work where you are. Choosing a row in
that menu presses its key. The status line at the bottom shows the main ones, and each can be
clicked.

A key is underlined inside its word on screen; a symbol key (`↑` `⏎` `esc`) comes before its
word. The same key does the same thing in the same kind of place in every part.

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
| `b` | bulk actions on everything shown |
| `/` | filter by words |
| `:` | go to a group, repository or merge request by part of its name or number |
| `r` | refresh a list or an open page |
| `o` | open in the service |
| `y` | copy the link |
| `z` | pause or resume refreshing |
| `H` | History: recent activity |
| `,` | Settings |
| `q` | quit |
| `ctrl-c` | quit from anywhere, even while typing |

## Screens

| Key | Screen |
|---|---|
| `R` | Repositories (then `G` GitHub, `W` local workspaces) |
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

| Key | Does |
|---|---|
| `⏎` | new line |
| `ctrl-s` | review and send |
| `esc` | stop writing |
| `y` | on a question: do it |
| `e` | on a question: go back to editing |
| `n` | on a question: drop it |

Nothing is sent until you press `y`. Text you decline is kept. In GitLab, Jira and Slack,
`ctrl-r` lists the kept text so you can copy it back or drop it.

## GitLab merge requests

| Key | Does |
|---|---|
| `u` | rebase onto the target |
| `m` | merge |
| `e` | approve, or take your approval back (`a` on the merge request's page) |
| `c` | check out its branch; on a page's Changes or Threads, write a comment |
| `p` | open the pipeline |
| `a` | show one author's merge requests |
| `.` | in a view: show only the repositories that have merge requests in it, and back |
| `e` (on a page with a local checkout) | open an editor there |
| `t` | open a terminal in the checkout |
| `s` | look for checkouts again |
| `v` | on a diff: side by side |
| `f` | on a diff: collapse the files beside it |
| `n` `N` | on a diff: next / previous file |
| `}` `{` | on a diff: next / previous hunk |

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
| `t` | move the ticket |
| `i` | assign it to me |
| `c` | comment |
| `a` | show one assignee's tickets |
| `:` | go to a ticket by its key or words |
| `A` | ask an agent about the ticket |
| `Y` | copy the ticket's key |

## Slack

| Key | Does |
|---|---|
| `c` | reply or write |
| `m` | mark read up to the selected message (radar's own mark) |
| `e` | react to a message |
| `v` | on a review ask: review its linked merge request locally |
| `A` | ask an agent about the message |

On a review ask you are drafting:

| Key | Does |
|---|---|
| `p` | post it |
| `e` | edit the words for the selected item |
| `d` | take the selected item out of it |
| `n` | drop the draft |

## Branches

| Key | Does |
|---|---|
| `d` | delete the branch or worktree (asks first) |
| `r` | refresh |
| `b` | bulk menu, with "prune merged and gone" |

## History

| Key | Does |
|---|---|
| `n` `p` | next / previous page |
| `r` | fresh snapshot |
| `a` | all recorded activity (when a filter is set) |
| `j` `k` | choose a row |
| `tab` | move to the detail |
| `esc` | back |

## Settings

| Key | Does |
|---|---|
| `⏎` | change the row; on a check row, run the check |
| `<` `>` | move a row up or down |
| `PgUp` `PgDn` | scroll the explanation |

## Agent dock

| Key | Does |
|---|---|
| `` ` `` (backtick) | show the dock; go to a session waiting for you. On an item page, where the dock is not drawn, it shows or hides the dock for the lists and the keys stay with the page |
| `/` in the composer | the session's commands: ↑↓ and tab choose, `⏎` runs, a click on a row runs it |
| `ctrl-t` | new session |
| `alt-f` | whole column, and back |
| `alt-w` | close the session tab (`ctrl-w` does too while the composer is empty) |
| `alt-←` `alt-→` | previous / next session tab |
| `alt-a` | choose the agent for new sessions |
| `alt-m` | choose the model for the session |
| `alt-s` | choose a session to open |
| `alt-k` | put a skill's request in the composer |
| `alt-o` | fold or open the latest tool call |
| `alt-r` | read a restored session back from its agent |
| `esc` | while a turn runs, stop it; otherwise give the keys back to the lists |
| `A` | on a row: ask an agent about it |
| `i` | on a merge request: review it locally |

## Mouse

| Action | Does |
|---|---|
| click | select; on a tab, a menu row or a status line key, act |
| click twice | open (same as `⏎`) |
| right-click a row | open its `x` menu |
| wheel | scroll the pane under the pointer |
| drag | select text inside the pane the drag started in; letting go copies it |
| double-click in text | select the word under the pointer (a path or a link whole) and copy it |
| triple-click in text | select the line and copy it |

### Selecting text

| Key | Does |
|---|---|
| `shift`+`←` `→` `↑` `↓` | with a selection showing: move its end a cell or a row, inside the pane (in the editor they select in the editor) |
| `shift`+`Home` `End` | the selection's end goes to the left or right edge of its row |
| `ctrl+c` | with a selection showing: copy it again (otherwise it quits as always) |
| any other key, a click, the wheel | clear the selection; the key does what it always does |

While a drag selects, the status line names the key that makes your terminal select its own
text instead: `shift`, or `option` in iTerm2, `fn` in Terminal.app.
