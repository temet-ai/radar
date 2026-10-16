# What radar stores and sends

Back to the [user guide](README.md).

## Network

| To | What | When |
|---|---|---|
| Each service you connect (GitLab, GitHub, Jira, Slack), at the address you configured | API requests with that service's token: reads, and the writes you confirm | while radar syncs, and when you act |
| The same service | pictures in descriptions, tickets and threads, with that service's token | when a picture scrolls into view |
| The npm registry (`registry.npmjs.org`) | Anthropic's Claude Agent SDK (about 1.6 MB), at the version radar was built with, checked against its pinned checksum | the first time you start a Claude session with that version |

radar makes no other network requests. A token is sent to the address configured for its
service. When a service redirects a request to another host, the request goes on without
the token. Tokens are not written to the log.

Which writes radar may send depends on its [mode](README.md#the-mode).

## Coding agents

The agents run as your own `claude`, `codex` or `copilot` programs, with their own logins,
and talk to their vendor (Anthropic, OpenAI, GitHub) as they do in a terminal. radar does
not read or store those logins. An agent receives what you type to it, or an item you hand
it with `A`; radar shows that text before it is sent.

Each agent session also gets an MCP tool server from radar. It listens on a local socket
that your user alone can open, answers that one session (by a one-time ticket), and stops
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

The config file holds a token if you typed one in. Otherwise it holds where the token
is: `env:GITLAB_TOKEN`, or `glab` for the GitLab CLI's login.

The action history records which operation ran on which item and how it ended. It does not
record typed text, the clipboard, drafts, credentials or service responses. Paths and item
names in it still say something about your work. The cache and the history are created
readable by your user alone.

Deleting any of these is safe. radar recreates the cache and the runtime on its next start,
and downloads the SDK again the next time a Claude session starts.
