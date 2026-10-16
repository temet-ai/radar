# When something looks wrong

Back to the [user guide](README.md).

## The log

radar writes what it did and why to a log. Tokens never reach it.

The log is `~/.local/state/radar/radar.log` on Linux and macOS (under `$XDG_STATE_HOME`
if set).

It rotates at 256 KB into `radar.log.1`. For more detail, start radar with `RADAR_DEBUG=1`:

```sh
RADAR_DEBUG=1 radar
tail -f ~/.local/state/radar/radar.log
```

The log has start-up, which config and cache were read, each sync and how long it took, checks,
links radar could not open, and crashes with where they happened.

## Common things

| You see | Try |
|---|---|
| "could not sync: ... is not set" | the variable named by `env:NAME` is not exported in the shell that started radar. Export it and restart |
| a part says "not set up" | press `s` on its screen to walk its setup step, or open Settings (`,`) and fill in its connection |
| a write says "radar is read only" | change the [mode](README.md#mode-what-radar-may-change) |
| right-click does nothing | your terminal does not pass right-clicks through; use `x` |
| the screen looks wrong or garbled | start with the other renderer to compare: `RADAR_RENDERER=ratatui radar`. If it looks right there, report the difference |
| empty boxes instead of icons | set the icon pack to `unicode`, or use a Nerd Font |
| Slack says it needs a user token | use an `xoxp-` token, not a bot token |
| nothing refreshes | radar paused after idle time; press any key, or `z` |
| no agent in the dock | the `claude`, `codex` or `copilot` program is not on your `PATH` or not signed in; see Settings, Agents |

## Where radar keeps things

| What | Where |
|---|---|
| config, and your icon packs in `icons/` | `~/.config/radar/` |
| log | `~/.local/state/radar/radar.log` |
| what the services returned (cache) | `~/.local/state/radar/cache.sqlite3` |
| action history | `~/.local/state/radar/journal/actions.sqlite3` |
| pictures | `~/.cache/radar/images` (up to 200 MB; on macOS `~/Library/Caches/radar/images`) |
| the Claude Agent SDK | `~/.cache/radar/claude-agent-sdk/<version>` (on macOS `~/Library/Caches/radar/claude-agent-sdk/<version>`) |
| local reviews (`[sessions] reviews_dir` moves it) | `~/.local/state/radar/reviews` |
| the sockets of terminal tabs | `~/.local/state/radar/terminals` |
| the start lines shown lately | `~/.local/state/radar/start-lines.toml` |

`XDG_CONFIG_HOME`, `XDG_STATE_HOME` and `XDG_CACHE_HOME` move these folders when set. If you delete
the cache, radar rebuilds it on its next sync. Keep the history file private.

## Report a problem

Open an issue on the [radar repository](https://github.com/temet-ai/radar/issues) and include:

* the output of `radar --version`
* your operating system and terminal (`radar terminal` prints what radar detected)
* what you pressed and what you expected
* the last lines of the log, taken with `RADAR_DEBUG=1` if you can

Read the log before you post it. It holds no tokens, but it names your repositories and sites.
