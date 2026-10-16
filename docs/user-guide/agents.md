# Agents

Back to the [user guide](README.md).

radar runs coding agent sessions in a dock under the middle pane, on every screen. A session
is one conversation with one agent. radar can start one with an item already described to it.

![The agent dock](images/agent-dock.png)

## What each agent needs

| Agent | Needs |
|---|---|
| Claude Agent | the `claude` program (Claude Code), signed in; radar runs it through your own Claude Code login |
| Codex | the `codex` program, signed in |
| Copilot | the `copilot` program (GitHub Copilot), signed in |

Install the program and sign in to it as you normally do, with a subscription or an API key.
That vendor's terms apply to its use; for heavy or unattended use of Claude, Anthropic
recommends an API key. radar drives each program itself. The program keeps the transcript,
and radar reads it back when you reopen a session.

Setup's agents step shows which programs are installed and signed in, and which one new
sessions use; `y` copies the command that installs or signs in one.

## Using the dock

`ctrl-t` starts a session, and `` ` `` shows the dock. Each session is a tab. Type your
message in the box at the bottom and press `⏎`. A message sent while a turn is running is
queued and sent afterwards. Drag the dock's top edge to resize it. Every dock key is in
[Keys](keys.md#agent-dock).

An **activity line** above the box says what the agent is doing (thinking, writing, running a
command, waiting for you) and for how long, and afterwards how the turn ended (for example
`done in 42s · 3.4k tokens`). A tab shows a spinner while it works, a warning mark when it needs
you, and a mark when a turn ended while you were looking elsewhere. If a session needs you out
of sight, the status line shows a chip; press `` ` `` or click the chip to go there.

## Approvals

When an agent wants to run a command, change a file or fetch a page, the dock shows the exact
command, path, diff or address and waits until you choose:

| Choice | Does |
|---|---|
| a̲pprove once | allow it once |
| approve for s̲ession | allow this kind of request for the rest of the session (when the agent offers it) |
| al̲ways allow | allow this kind of request from now on, as a standing rule (when the agent offers it) |
| d̲eny | refuse; the agent is told |
| v̲iew diff | see the change before choosing |

Rules you already set in an agent's own configuration still apply. Settings, Agents has
**always allow** and **always deny**, which list the requests your agents made so you can
turn one into a standing rule.

## After you quit radar

Your session tabs come back when you start radar again, but the agent programs that ran them
ended with radar. A restored tab shows a connection mark and says it is not attached. radar
reads the session back from the agent when you first look at its tab, type a message in it,
or press `alt-r` (or type `/resume`).

A request or question that was waiting when radar closed comes back muted, marked "radar
closed before this was answered", with no buttons, and it does not count as needing you. A
line says the turn was ended. Send a message to carry on; if the agent asks again, the new
request waits for you as usual.

## Sending an item to an agent

On a row for a merge request, ticket, Slack message or branch, the `x` menu has a **With an
agent** group:

* `A` **ask an agent about it…** starts a session with the item described, and your question.
* `i` **review it locally…** (on an open merge request) makes a fresh worktree and starts a
  review session in it. The review is written in radar and is not posted to GitLab. On a
  Slack review ask, `v` does the same for its linked merge request.

## Settings, Agents

Settings, Agents shows each program: whether it is installed, its version and who you are
signed in as. It also holds the default agent, the folders an agent may use, shared skill
folders, and whether an agent's own configuration is read. Desktop notifications when a
session needs you or ends out of sight are off by default.

![Settings, Agents](images/settings.sessions.agents.png)
