# Find a page

Use this directory to find the guide for the screen in front of you. Each guide
explains how to open the page, what its panes or tabs contain, the actions you
can take and what to check when it is empty or unavailable.

New to radar? Start with the [local Git walkthrough](quickstart.md), then learn
the [shared navigation](README.md#2-getting-around). `x` shows the actions at your
current position; `ctrl-p` searches screens, Settings, actions and cached items.

## Startup and shared controls

| Page or control | How to open it | Guide |
|---|---|---|
| Splash | Start radar; any key skips it | [First run](README.md#1-first-run) |
| Setup: Welcome, scan, chooser, Local code, service connections, Agents, Look & terminal, Mode, Ready | First run, or `radar setup`; a named step opens directly | [Setup walkthrough](README.md#setup), [connections](connect.md) |
| Tour | Choose the tour at the end of setup, or `radar setup tour` | [First run](README.md#setup) |
| Action and bulk menus | `x` or `?` on an item; `b` where bulk actions are available | [Menus](README.md#menus), [keys](keys.md) |
| Command palette | `ctrl-p` while navigating | [Finding things](README.md#finding-things) |
| Editors, confirmations and retained text | Start a comment, reply or other action from its menu | [Text and questions](keys.md#text-and-questions), the relevant service guide below |
| Recent notices | `~` while navigating | [Recent notices](README.md#recent-notices) |
| Agent dock, conversation pickers and approvals | Backtick opens the dock on supported screens; `A` on a supported item sends its context | [Agents](agents.md) |

## Services and their detail pages

`R` opens Repositories. Its sources are GitLab, GitHub (`G`) and Local workspaces
(`W`). Other screens have their own shortcuts. Use `⏎` on a row with a detail
page, `[` / `]` to change tabs and `esc` to return. Some rows open a browser or
editor instead; the guides identify those cases.

| Screen or page | How to reach it | Tabs or views documented |
|---|---|---|
| [GitLab Repositories](gitlab.md#repositories-screen) | `R`, GitLab source | Saved views, groups, repositories, merge requests |
| [GitLab tenancy](gitlab.md#every-page-and-tab) | `⏎` on a followed group | Repos |
| [GitLab repository](gitlab.md#every-page-and-tab) | `⏎` on a repository | Merge requests, Pipelines on the default branch; linked Alerts |
| [GitLab merge request](gitlab.md#merge-request-tabs) | `⏎` on a merge request | Overview, Changes, Threads, Pipeline, Commits; linked Ticket/Tickets and Slack |
| [GitLab job](gitlab.md#job-log) | `⏎` on a job in Pipeline, or a failed job on the repository page | Log, search, folded sections and earlier output |
| [GitHub Repositories](github.md#repositories-screen) | `G`, or select GitHub under Repositories | Everything and configured repository groups |
| [GitHub repository](github.md#every-page-and-tab) | `⏎` on a repository | Overview, Pull requests, Actions |
| [GitHub pull request](github.md#every-page-and-tab) | `⏎` on a pull request | Overview, Checks, Actions |
| [GitHub run attempt](github.md#every-page-and-tab) | `⏎` on an Actions run | Overview, Jobs; attempt chooser with `a`. Job logs open on GitHub |
| [Jira](jira.md#jira-screen) | `J` | Saved views, site and status groups, ticket list |
| [Jira ticket](jira.md#every-page-and-tab) | `⏎` on a ticket | Overview, Comments, Links; linked Code, Slack and Notes |
| [Slack](slack.md#choose-a-feed) | `S` | For you, Discussions, Alerts and watched channels |
| [Slack thread](slack.md#read-a-thread) | `⏎` on a message | Thread, Links; replies and reactions |
| [Slack review draft](slack.md#ask-for-review) | The review ask before it posts, when posting is set to draft first; add more requests to the draft individually | Items, per-item text, channel choice and posting confirmation |

Linked tabs appear only when the relevant parts are connected and radar knows
the relationship. Their absence can mean there is no linked information in the
current data. Each service guide covers loading, refresh failures and empty
results.

## Local work and activity

| Screen or view | How to reach it | Guide |
|---|---|---|
| Branches | `B` | [Clones, worktrees, branches and stashes](git.md) |
| Local workspaces | `W`, or select Local workspaces under Repositories | [Workspace and repository folders](workspaces.md#open-a-workspace-or-repository) |
| Ticket notes and start-work chooser | A linked ticket's Notes tab or workspace action | [Notes](workspaces.md#read-a-tickets-notes), [starting work across repositories](workspaces.md#start-a-ticket-in-several-repositories) |
| Work | `I` | [Tasks, groups, filters and focus](work.md#pages-filters-and-focus) |
| Work sources and evidence | Select a task and use its action menu | [Bindings, related sources and original evidence](work.md#inspect-sources-and-original-evidence) |
| History | `H`, or **history for selected item** in an item's menu | [Event list and detail](history.md#event-list-and-detail), [scopes and pagination](history.md#scopes-and-pages) |
| Saved local review | **open the local review** or **copy the local review's path** on a reviewed change | [Results](mcp-and-reviews.md#read-the-result), [save locations](mcp-and-reviews.md#where-reviews-and-work-are-saved) |

Workspaces open folders in your editor. Work and History show their details in
panes; they do not add a separate service-style item page. A saved review is a
local Markdown report. [MCP and automatic reviews](mcp-and-reviews.md) explains
how external sessions and Slack-triggered reviews use the connected context.

## Settings

Press `,` for Settings. The [Settings guide](settings.md) covers every section,
validation, saving and repository-specific overrides.

| Group | Sections |
|---|---|
| [General](settings.md#general) | Parts, Mode, Appearance, Colours, Icons |
| [GitLab](settings.md#gitlab) | Connection, Tenancies, Views, Look |
| [GitHub](settings.md#github) | Connection, Repositories |
| [Jira](settings.md#jira) | Connection, Projects, Views, Look |
| [Slack](settings.md#slack) | Connection, Channels, Review, Alerts, Look |
| [Git](settings.md#git) | Checkouts, Terminal, Editors, Links, Branches |
| [Workspace](settings.md#workspace) | Workspaces |
| [Agents](settings.md#agents) | Tools and defaults, including provider paths and repository folder overrides |

If the page still does not behave as expected, use
[troubleshooting](troubleshooting.md), [the key reference](keys.md) or
[what radar stores and sends](privacy.md).
