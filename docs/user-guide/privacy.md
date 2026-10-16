# What radar stores and sends

## Network

| To | What | When |
|---|---|---|
| Each service you connect (GitLab, GitHub, Jira, Slack), at the address you configured | API requests with that service's token: reads, and the writes you confirm | while radar syncs, and when you act |
| The same service | pictures in descriptions, tickets and threads, with that service's token | when a picture scrolls into view |
| The npm registry (`registry.npmjs.org`) | Anthropic's Claude Agent SDK, at the version radar was built with, checked against its pinned checksum | the first time you start a Claude session with that version |

radar makes no other network requests. A token goes only to the address configured for its
service, and radar does not follow a redirect to another host. Tokens are not written to the
log.

A write (a comment, a reply, an approval, a merge, a moved ticket) is shown in full and sent
when you confirm it. In read-only mode writes are shown and not sent.

## Coding agents

The agents run as your own `claude`, `codex` or `copilot` programs, with their own logins,
and talk to their vendor (Anthropic, OpenAI, GitHub) as they do in a terminal. radar does
not handle those logins. An agent receives what you type to it, or an item you hand it with
`A`; radar shows that text before it is sent.

Each agent session also gets an MCP tool server from radar. It listens on a local socket
that only your user can open, answers only that session (by a one-time ticket), and stops
when the session ends.

## Files

| What | Where |
|---|---|
| config, and icon packs in `icons/` | `~/.config/radar/` |
| cache of what the services returned | `~/.local/state/radar/cache.sqlite3` |
| action history | `~/.local/state/radar/journal/actions.sqlite3` |
| log | `~/.local/state/radar/radar.log` |
| pictures, at most 200 MB | `~/.cache/radar/images` (macOS: `~/Library/Caches/radar/images`) |
| the renderer, unpacked from the binary | `~/.cache/radar/runtime` (macOS: `~/Library/Caches/radar/runtime`) |
| the Claude Agent SDK, per version | `~/.cache/radar/claude-agent-sdk/<version>` (macOS: `~/Library/Caches/radar/claude-agent-sdk/<version>`) |
| local reviews (`[sessions] reviews_dir` moves them) | `~/.local/state/radar/reviews` |
| sockets of terminal tabs in the dock | `~/.local/state/radar/terminals` |
| which start lines showed recently, by id | `~/.local/state/radar/start-lines.toml` |

`XDG_CONFIG_HOME`, `XDG_STATE_HOME` and `XDG_CACHE_HOME` move these folders when set.

The config file holds a token only if you typed one in. Otherwise it holds where the token
is: `env:GITLAB_TOKEN`, or `glab` for the GitLab CLI's login.

The action history records which operation ran on which item and how it ended. It does not
record typed text, the clipboard, drafts, credentials or service responses. The cache and
the history are created readable only by your user.

Deleting any of these is safe. radar recreates the cache and the runtime on its next start,
and downloads the SDK again the next time a Claude session starts.
