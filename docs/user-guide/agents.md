# Agents

Back to the [user guide](README.md).

radar can run coding agent sessions in a dock under the middle pane, on every screen. A session is
one conversation with one agent. radar can start one with an item already described to it.

![The agent dock](images/agent-dock.png)

| Agent | Needs |
|---|---|
| Claude Code | the `claude` program, signed in |
| Codex | the `codex` program, signed in |
| GitHub Copilot | the `copilot` program, signed in |

radar drives each program itself. The agent's own program owns the transcript; radar reads it back
when you reopen a session.

## Using the dock

| Key | Does |
|---|---|
| `` ` `` (backtick) | show the dock; with a session waiting for you out of sight, go to that session |
| `ctrl-t` | start a new session |
| `alt-f` | the dock takes the whole middle column, and back |
| `alt-w` | close the session tab showing |
| `alt-a` | choose the agent new sessions start as |
| `alt-k` | put a skill's request in the composer |

Each session is a tab. Type your message in the box at the bottom and press `⏎`. A message sent
while a turn is running is queued and sent afterwards. Drag the dock's top edge to resize it.

An **activity line** above the box says what the agent is doing (thinking, writing, running a
command, waiting for you) and for how long, and afterwards how the turn ended (for example
`done in 42s · 3.4k tokens`). A tab shows a spinner while it works, a warning mark when it needs
you, and a mark when a turn ended while you were looking elsewhere. If a session needs you out
of sight, the status line shows a chip; press the dock key or click the chip to go there.

## Approvals

When an agent wants to run a command, change a file or fetch a page, the dock shows the exact
command, path, diff or address and waits. Nothing is approved for you.

| Choice | Does |
|---|---|
| a̲pprove | allow it once |
| s̲ession | allow this kind of request for the rest of the session (when the agent offers it) |
| d̲eny | refuse; the agent is told |
| v̲iew diff | see the change before choosing |

A request waits as long as it needs to. Rules you already set in an agent's own configuration
still apply.

## Sending an item to an agent

On a row for a merge request, ticket, Slack message or branch, the `x` menu has a **With an
agent** group:

* `A` **ask an agent about it…** starts a session with the item described, and your question.
* `i` **review it locally…** (on a merge request) makes a fresh worktree and starts a review
  session in it. The review is written in radar. Nothing is posted to GitLab.

## Settings, Agents

Settings, Agents shows each program: whether it is installed, its version and who you are
signed in as. It also holds the default agent, the folders an agent may use, shared skill
folders, and whether an agent's own configuration is read. An option for desktop notifications
"while you are away" is off by default.

![Settings, Agents](images/settings.sessions.agents.png)

