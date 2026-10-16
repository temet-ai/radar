# What radar stores and sends

radar runs on your machine. It keeps everything it stores in your own folders, it talks
only to the services you connect and to the places listed below, and it sends no
telemetry: no usage data, crash reports or analytics go anywhere.

## What it sends, and where

| To | What | When |
|---|---|---|
| Each service you connect (GitLab, GitHub, Jira, Slack), at the address you configured | API requests with that service's token, to read your items and, when you say yes to a write, to send it | while radar runs and syncs, and when you act |
| The same service | requests for pictures in descriptions, tickets and threads, with that service's token | as a picture scrolls into view |
| The npm registry (`registry.npmjs.org`) | one download of Anthropic's Claude Agent SDK, at the version radar was built with, checked against its pinned checksum | the first time you start a Claude session; never again for that version |
| Nothing else | | |

A token is sent only to the service it belongs to, at the address you configured for it.
radar refuses to follow a link from a service's reply to any other host. Tokens are never
written to the log.

Every write (a comment, a reply, an approval, a merge, a moved ticket) is shown to you in
full before it is sent and goes only when you say yes. In read-only mode radar sends no
writes at all.

## The coding agents

The agents in the dock run as your own `claude`, `codex` or `copilot` programs, with your
own logins. Each agent talks to its own vendor (Anthropic, OpenAI, GitHub) under that
vendor's terms, the same as when you use it in a terminal. radar does not read, store or
pass on an agent's login. What an agent sees of your work is what you send it: a message
you type, or an item you hand it with `A`, whose text radar shows you first.

radar also gives each agent session a small tool server (MCP) of its own. It runs on your
machine, on a socket only your user can open, answers only that session (which proves itself
with a one-time ticket), and is gone when the session ends.

## What it stores, and where

| What | Where |
|---|---|
| your config, and your icon packs in `icons/` | `~/.config/radar/` |
| what the services returned (a cache, so radar starts fast and works offline) | `~/.local/state/radar/cache.sqlite3` |
| your action history | `~/.local/state/radar/journal/actions.sqlite3` |
| the log | `~/.local/state/radar/radar.log` |
| pictures | `~/.cache/radar/images` on Linux, `~/Library/Caches/radar/images` on macOS (at most 200 MB) |
| radar's renderer, unpacked from the binary | `~/.cache/radar/runtime` (macOS: `~/Library/Caches/radar/runtime`) |
| the Claude Agent SDK, once downloaded, in a folder named for its version | `~/.cache/radar/claude-agent-sdk/<version>` (macOS: `~/Library/Caches/radar/claude-agent-sdk/<version>`) |
| local reviews, when no work item takes them (`[sessions] reviews_dir` moves it) | `~/.local/state/radar/reviews` |
| the sockets of terminal tabs in the dock | `~/.local/state/radar/terminals` |
| which start lines showed lately, by id, so they do not repeat | `~/.local/state/radar/start-lines.toml` |

`XDG_CONFIG_HOME`, `XDG_STATE_HOME` and `XDG_CACHE_HOME` move these folders when set.

The config file holds a token only if you typed one in; otherwise it holds where to find
it (`env:GITLAB_TOKEN`, or `glab` for the GitLab CLI's own login). The action history
records which operation ran on which item and how it ended. It never records text you
typed, the clipboard, drafts, credentials or what a service returned. The cache and the
history are created readable only by you.

All of it can be deleted. radar rebuilds the cache and the runtime on its next start, and
downloads the SDK again the next time you start a Claude session.
