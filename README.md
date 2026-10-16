# radar

radar is a terminal workspace for developers who spend their day across GitLab, GitHub,
Jira and Slack. It shows your merge requests, pull requests, tickets and the Slack threads
about them on one screen, joined to each other and to the code checked out on your machine,
and it runs coding agents (Claude, Codex, GitHub Copilot) beside that work.

![radar starting, then moving through merge requests, a merge request's page, Jira, Slack and an agent session](docs/user-guide/images/demo.gif)

## What it does

- **One list of what needs you.** Merge requests and pull requests across groups and
  organisations, with pipelines and checks; Jira tickets across sites; Slack mentions,
  threads and review asks. Each links to the others: a ticket shows its merge requests and
  the Slack threads that discuss it.
- **Act without switching tools.** Comment, reply, approve, rebase, merge, move a ticket or
  check out a branch from the keyboard. Every write is shown to you in full and sent only
  when you say yes, and in read-only mode (one click on the mode at the top right) radar
  sends nothing at all.
- **Your local work in the same place.** radar finds your checkouts of the repositories it
  shows, opens them in your editor or a terminal, and keeps a private local history of what
  you did.
- **Agents next to the work.** An agent dock runs Claude (through your own Claude Code),
  Codex or Copilot sessions, and you can hand an agent a merge request or ticket with its
  context already written down. Each agent runs as your own CLI (`claude`, `codex` or
  `copilot`, installed and signed in as usual) with your own login.

radar is one file: there is no Node, Python or other runtime to install, and it works in
any modern terminal, over SSH and inside tmux.

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

radar is in alpha (0.1.0-alpha.6). It is used every day by its maintainers, but screens,
keys and the config file can still change between releases, and some parts are newer than
others: GitHub is read-only for now, and the agent dock is the newest part.

## Privacy

radar keeps your config, caches and local history on your machine. Tokens go only to the
service they belong to, at the address you configured, and are never logged. radar sends no
telemetry. The first time you start a Claude session radar downloads Anthropic's Claude
Agent SDK (1.6 MB) from the npm registry itself, checks it against a pinned checksum and
keeps it in its cache; you do not need Node or npm for this.
[What radar stores and sends](docs/user-guide/privacy.md) has the full list, and
[SECURITY.md](SECURITY.md) says how to report a security problem.

## Contributing

Bug reports and small fixes are welcome; for anything larger, open an issue first.
[CONTRIBUTING.md](CONTRIBUTING.md) explains how to build radar and run its tests.

## Licence

MIT, see [LICENSE](LICENSE). `radar licenses` prints the licences of everything a release
carries ([THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md)).

radar is an independent project, not affiliated with or endorsed by Anthropic, OpenAI,
GitHub, GitLab, Atlassian or Slack. Their product names are their trademarks.
