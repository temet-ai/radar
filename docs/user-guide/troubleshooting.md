# When something looks wrong

Back to the [user guide](README.md).

## The log

radar writes what it did and why to `~/.local/state/radar/radar.log` (under
`$XDG_STATE_HOME` if set). It rotates at 256 KB into `radar.log.1`. For more detail, start
radar with `RADAR_DEBUG=1`:

```sh
RADAR_DEBUG=1 radar
tail -f ~/.local/state/radar/radar.log
```

The log has start-up, which config and cache were read, each sync and how long it took, checks,
links radar could not open, and crashes with where they happened.
[What radar stores and sends](privacy.md#files) lists every file radar keeps.

## Common problems

| You see | Try |
|---|---|
| "could not sync: ... is not set" | the variable named by `env:NAME` is not exported in the shell that started radar. Export it and restart |
| a part says "not set up" | press `s` on its screen to walk its setup step, or `⏎` to open its section in Settings |
| a write says "radar is read only" | change the [mode](README.md#the-mode) |
| right-click does nothing | your terminal does not pass right-clicks through (`radar terminal` says what radar detected); use `x` |
| radar exits with `radar: the renderer did not start: <why>` | the Ink renderer is missing or crashed; the reason follows the colon. Run `scripts/install` to rebuild it (a release install: reinstall the release) |
| the screen looks wrong or garbled | check your terminal's truecolor and font support, then report it with the terminal name |
| empty boxes instead of icons | set the icon pack to `unicode`, or use a Nerd Font |
| Slack says it needs a user token | use an `xoxp-` token; a bot token does not work |
| nothing refreshes | input resumes an idle pause; an explicit pause lasts until you press `z` again |
| no agent in the dock | check provider status in Settings, Agents and follow [provider setup](agents.md#set-up-a-provider); item pages do not draw the dock |
| stale or unexpected rows | press `r`, check the view/filter and connection result; cached rows remain visible after a failed refresh |
| a status message disappeared | press `~` for [recent notices](README.md#recent-notices), or `H` for recorded operations |
| an MCP client cannot find radar | leave radar running, then follow [host selection and reconnecting](mcp-and-reviews.md#connect-an-external-session) |
| a Slack review did not start or finish | check the [review intake and result states](mcp-and-reviews.md#when-a-review-does-not-start-or-finish) |
| a session or report uses the wrong folder | check [global and repository location precedence](mcp-and-reviews.md#where-reviews-and-work-are-saved); changes apply to new work |

Do not delete the cache as a routine refresh fix. It also holds local read marks, review
queues and context, session resume pointers and pending recovery data. The journal holds
Work tasks and relationships. Quit radar and preserve the relevant files before any
storage repair; [What radar stores and sends](privacy.md#files) explains what is recoverable.

## Report a problem

Open an issue on the [radar repository](https://github.com/temet-ai/radar/issues) and include:

* the output of `radar --version`
* your operating system and terminal (`radar terminal` prints what radar detected)
* what you pressed and what you expected
* the last lines of the log, taken with `RADAR_DEBUG=1` if you can

Read and redact the log before you post it. Radar redacts credentials, but the log can
name your repositories, sites and local folders.
