# Jira: tickets, discussion and linked work

Jira in radar keeps your saved queries, ticket details and related code together. You
can read a ticket, follow its discussion or linked change, then comment, assign it or
move it without losing your place in the list.

First [connect a Jira Cloud site](connect.md#jira), choose its watched projects and
configure the views you need. Press `J` to open Jira. Each view is a saved JQL query;
the same view can collect tickets from more than one configured site.

![Jira's views, site/status tree and ticket detail](images/jira.png)

The pictures use fictional demonstration data documented in the
[screenshot index](images/INDEX.md).

## Example: pick up a ticket and its change

Suppose the fictional `WATCH-12` describes a patrol scheduling fix. Follow its details
and related change before picking up the work:

1. Press `:` and find `WATCH-12`, or choose a saved view and select it in the tree.
   Press `⏎` to read **Overview**, including its description and acceptance criteria.
2. Select **Comments** to read the latest discussion. Older comments load as you scroll
   down. Check **Links** for its parent, subtasks or related tickets.
3. If **Code** is present, select the relevant change and press `⏎`. Radar opens the
   owning part's page where supported, or follows the web link. `esc` returns through
   nested radar pages.
4. Back on the ticket, use `i` to ask to assign it to yourself and `t` to choose a
   transition Jira permits. Both need **edit** mode and a confirmation.
5. Use `c` for a progress comment. `ctrl-s` reviews the text, then `y` sends it when
   [write or edit mode](README.md#the-mode) permits it. Read Jira's result before
   considering the action complete.

![A Jira ticket's Overview and detail pane](images/detail-jira-issue.png)

## Jira screen

The left pane holds saved views. The middle pane groups tickets by **site**, then
**status**. Rows show their key, type, status, summary, assignee and age; connected parts
can add related-code information. The right pane describes the selected ticket or
group and shows the relevant actions.

`/` filters the current view by words. `a` picks an assignee filter; `:` finds a ticket
by key or words. These narrow or navigate what radar has available; edit the saved
JQL under Settings, Jira, views when you want to change the query itself.

`space` folds a site or status group. `⏎` opens a ticket page; `[` and `]` move between
its tabs. Opening a related ticket from a page puts it above the current page, so
`esc` comes back to the previous ticket before returning to the list. See the shared
[navigation keys](keys.md#everywhere).

## Every page and tab

Jira's item page is a ticket. Site and status rows organize the screen and have details
in the right pane; they do not introduce another set of ticket tabs.

| Tab | What it contains | What you can do |
|---|---|---|
| **Overview** | Description, acceptance criteria and ticket details such as assignee, reporter, priority, labels, parent, components and versions when available. Other parts can add related information. | Read the ticket before acting; use `x` for its actions. |
| **Comments** | Comments newest first, with their authors and ages. The heading indicates how many of the total are loaded. | Scroll down to load older comments; `c` starts your comment. |
| **Links** | Parent, children, linked tickets and web links, with their relationship and available status. | `⏎` on a ticket opens its Jira page; on a web link it opens the browser. |
| **Code** | Linked changes supplied by other parts and by Jira's development panel, combined without repeated links. Jira-supplied rows say **from Jira**. | `⏎` follows the selected change. This tab appears when linked code is available. |
| **Slack** | Related threads radar has read from Slack. | `⏎` opens the selected discussion. It appears when relevant threads are known. |
| **Notes** | Local workspace notes associated with the ticket. | Select a note and press `⏎` to open it through the workspace part. It appears when a configured workspace has matching notes. |

On a ticket page, `o` opens that ticket in Jira, `y` copies its link and `Y` copies only
its key. On Links or a contributed tab, use `⏎` to follow the selected row. The extra
tabs depend on connected parts and known relationships, so not every ticket has them.

## Comment, create, assign and move

Comments and new tickets need **write** or **edit** mode; assignment and transitions
need **edit** mode. Jira permissions still apply. Each change presents a confirmation
before it is sent, and an API failure is shown rather than treated as a successful
change. Shared editor, confirmation and recovery keys are in
[Text and questions](keys.md#text-and-questions).

| Action | How it works |
|---|---|
| Comment (`c`) | Write the comment, review it with `ctrl-s`, then confirm. A declined or interrupted draft can be recovered with `ctrl-r`. |
| New ticket (`n`) | Radar uses the current site and offers a watched project when one must be chosen. It asks Jira which issue types you may create, then takes the summary and description before confirmation. Add a watched project in Settings if none is available. |
| Assign to me (`i`) | Radar identifies your account and shows the assignment it will send. This action assigns to you, rather than opening a picker for arbitrary assignees. |
| Move (`t`) | Choose from the transitions Jira currently offers for that ticket, then confirm. The list comes from its workflow and your permissions. |

When you start a ticket through a local workspace, radar can also offer to assign it
to you and move it into an in-progress status in one question. It shows the proposed
changes first; `[jira] on_start = "off"` turns this offer off, and `start_status`
selects the desired status. Local workspace setup and agent work are covered in the
[Workspaces](workspaces.md) and [Agents](agents.md).

## Loading, empty results and failures

With no configured site, the screen points to Settings, Jira, connection. If a view
uses watched projects but none are configured, add a project under Settings, Jira,
projects. If a configured query returns no tickets, check the chosen view, assignee
and word filter before changing the connection.

Site rows stay selectable during loading or a failed read, so their detail can explain
the problem. Existing ticket data stays visible when a refresh fails; its error is
shown alongside it. `r` retries the list, or refreshes the current ticket page. A site
that was removed from Settings cannot refresh an already-open page until it is
configured again.

**No comments yet** and **No parent, children or links** describe empty loaded sections.
A loading or failure message describes an unavailable read instead. Related Code rows
may remain visible if an optional development-panel read fails; the error does not
turn a previously observed change into a current result.

Next, follow the change through [GitLab](gitlab.md) or [GitHub](github.md), or start a
contextual [agent session](agents.md). [Jira keys](keys.md#jira) is the compact action
reference.
