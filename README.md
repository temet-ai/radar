# radar

radar is a terminal workspace for developers who spend their day across GitLab, GitHub,
Jira and Slack. It shows your merge requests, pull requests, tickets and the Slack threads
about them on one screen, joined to each other and to the code checked out on your machine,
and it runs coding agents (Claude, Codex, GitHub Copilot) beside that work.

![radar starting, then moving through merge requests, a merge request's page, Jira, Slack and an agent session](docs/user-guide/images/demo.gif)

## What it is for

You open radar in a terminal and keep it open. Each screen is one part of your day, and
each part works the same way: views on the left, items in the middle, the selected item's
detail on the right, and a page for each item. The bottom line shows the keys that work
where you are; `x` lists them all.

### Merge requests (GitLab)

Your groups and repositories as a tree, with their open merge requests: which need your
review, which are yours and need fixing, which are ready to merge, and whose pipeline
failed. A merge request's page has its description, changes, threads and pipeline. From
there you can comment, approve, rebase, merge or check out its branch.

![Merge requests on the Repositories screen](docs/user-guide/images/repositories.png)

### Pull requests (GitHub)

Repositories, open pull requests, the checks on each pull request's head commit, and
Actions runs. GitHub is read-only for now.

![GitHub](docs/user-guide/images/github.png)

### Tickets (Jira)

Your saved queries as views, tickets grouped by site and status, each with the merge
request linked to it. You can open, create, move, assign and comment on tickets.

![Jira](docs/user-guide/images/jira.png)

### Chat (Slack)

Threads in the channels you watch, mentions of you and review asks, newest first, one line
a message. You can reply, react and mark threads read.

![Slack](docs/user-guide/images/slack.png)

### Your local code (Git)

The clones on this machine, their worktrees and branches: which are dirty, ahead or behind
their remote, and your stashes. On a merge request or repository radar shows where it is
checked out, and opens it in your editor or a terminal.

![Branches](docs/user-guide/images/branches.png)

### Work and history

Work is a list of your own tasks and of items you add from any screen. History is a record
on your machine of what radar did: syncs, items opened, and each write you confirmed or
declined.

![Work](docs/user-guide/images/work.png)

### Agents

A dock under the middle pane runs Claude (through your own Claude Code), Codex or Copilot
sessions. `A` on a row starts a session with that item already described to the agent, and
`i` on a merge request asks an agent to review it locally. Each agent runs as your own CLI,
with your own login, and waits for you before it runs a command or changes a file.

![The agent dock](docs/user-guide/images/agent-dock.png)

### The parts work together

A ticket shows its merge requests and the Slack threads that discuss it; a merge request
shows its ticket and where it is checked out. Every write (a comment, an approval, a merge)
is shown in full and sent when you confirm it, and the mode at the top right (read only,
write or edit) sets which writes radar may send at all.

radar is one file. It needs no Node, Python or other runtime, and it works in any modern
terminal, over SSH and inside tmux.

## Install

On macOS or Linux with Homebrew:

```sh
brew install temet-ai/radar/radar
```

Use the full name: Homebrew has an unrelated app called `radar`, and the short name
installs that one. Later releases come with `brew upgrade temet-ai/radar/radar`.

With [mise](https://mise.jdx.dev), add this to `mise.toml` and run `mise install`:

```toml
[tools]
"github:temet-ai/radar" = { version = "latest", prerelease = true, minimum_release_age = "0s" }
```

Or download the archive for your platform from the
[releases page](https://github.com/temet-ai/radar/releases), check it against
`checksums.txt`, and put `radar` on your `PATH`. Builds exist for macOS (Apple Silicon and
Intel) and Linux (x64 and arm64, glibc 2.35 or later). There is no Windows build yet; on
Windows, run the Linux build in WSL2.

## First run

Run `radar`. The first time, it walks you through setup: it asks before it looks for
keys you already have, then sets up what you choose: icons, theme and motion, your local
code, each service, and your agents. `radar setup` runs it again. [Connect your services](docs/user-guide/connect.md) says what each service needs,
and the [user guide](docs/user-guide/README.md) covers the screens, keys, agents and
settings. Press `x` anywhere to see every key that works there.

## Status

Version 0.1.0-alpha.6. Screens, keys and the config file can still change between
releases. GitHub is read-only for now, and the agent dock is the newest part.

## Privacy

radar's config, cache and history are files under your home folder. It talks to the
services you connect, and to the npm registry once, to fetch Anthropic's Claude Agent SDK
(1.6 MB) the first time you start a Claude session.
[What radar stores and sends](docs/user-guide/privacy.md) lists each file and request, and
[SECURITY.md](SECURITY.md) says how to report a security problem.

## Contributing

Bug reports and small fixes are welcome; for anything larger, open an issue first.
[CONTRIBUTING.md](CONTRIBUTING.md) explains how to build radar and run its tests.

## Licence

MIT, see [LICENSE](LICENSE). `radar licenses` prints the licences of everything a release
carries ([THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md)).

radar is an independent project, not affiliated with or endorsed by Anthropic, OpenAI,
GitHub, GitLab, Atlassian or Slack. Their product names are their trademarks.
