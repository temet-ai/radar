---
name: radar-ink-live
description: Run the real radar binary with the Ink renderer (RADAR_RENDERER=ink) or Ratatui beside it, drive it by keys and mouse, read its screen as text or colour, and prove Ink, not a silent Ratatui fallback, drew it. Use whenever a radar task says "live", "exercise", "run radar with RADAR_RENDERER=ink", "compare with Ratatui side by side", "right-click a row", "the status line shows", or needs to see what Ink draws in a real terminal rather than in ink-testing-library; also to check mouse modes and terminal restore after exit.
---

# Running radar live with Ink

`scripts/ink-live/ink-live` (in the repository) does the setup that went wrong by hand:
it copies the binary, rebuilds the bundle when `ink/src` is newer, gives radar scratch
XDG folders with offline fixture data, starts it in a private tmux server per
checkout, TMPDIR and name (parallel seats do not meet), and refuses a run where `RADAR_RENDERER=ink` fell back to Ratatui.

```bash
L=scripts/ink-live/ink-live     # from the repository root
$L start ink1 ink 160x40        # prints "drawn by Ink: Ink renderer started, pid N"
$L start rat1 ratatui 160x40    # the reference, same data, same size
diff <($L text ink1) <($L text rat1)
$L keys ink1 Down Down Enter    # tmux key names: Up Down Left Right Enter Escape Tab BTab PageDown x 1
$L click ink1 30 6 right        # 1-based cell; left | right | double | hover
$L wheel ink1 30 20 up 3        # three wheel notches at a 1-based cell; up | down (tests: scripts/ink-live/test-ink-live)
$L colour ink1                  # SGR colours: the selection and focus backgrounds
$L modes ink1                   # alternate screen and mouse modes 1000/1002/1003/1006
$L stop ink1; $L stop rat1      # quits with q, prints the modes left behind, cleans up
```

Name runs after your task (`t5ink`, `t5rat`): seats that share a checkout and TMPDIR
share the names too.

## Before the first start

- The binary is the default-features release build in the repo's own `target/`
  (`target/release/radar`). With `RADAR_BIN` unset, `ink-live start` builds it
  (`cargo build --release --bin radar`, `CARGO_TARGET_DIR` unset, as `scripts/install` does)
  when it is missing or older than HEAD, then copies it to TMPDIR. `RADAR_BIN=<path>` uses
  another binary, but `start` asks it for its parts (`radar shot --list`) and refuses, non-zero,
  when any default part (GitLab, GitHub, Jira, Slack, git, workspace, workbench) or the demo data is missing.
- **Never use `target/fleet/debug/radar`.** It is whatever feature mix the gate built last
  (jira+workspace on 2026-10-13: no GitLab repositories), so a live check on it proves nothing.
- Ink changes need no cargo build: the script bundles `ink/src` itself.

## The data

- Default: the offline fixtures of `tests/ink_pty.rs` (GitLab group acme/team-a with
  !263 and !264, Jira ABC-12, tokens unset, so nothing reaches a service). It shows the
  Repositories tree, an MR page, menus and Jira; it has no git checkouts or Slack.
- `RADAR_LIVE_CONFIG=<copy of a config.toml>` runs on that config's real services, read
  only (`RADAR_MODE=read-only`, `RADAR_NO_WRITES=1` are always set). Answer no to every
  confirm. Never point it at `~/.config/radar/config.toml` itself; copy it first.

## Reading what you see

- Compare Ink with Ratatui at the same size and data, never with what Ink drew before.
  Text equal is not enough for highlight and styling: compare `colour` output on the
  selected row and the focused pane's border.
- Mouse: `modes` on both renderers must match (Ratatui sets 1003 and 1006). If Ink does
  not take the mouse, herdr and tmux keep right-click for their own menus.
- The script removes the `HERDR_*` variables: radar in its tmux is not in your herdr pane,
  and with them it would reroute that pane's right-click and show `mouse …` or
  `mouse ✓` from it. The herdr right-click route itself is seen only in a real herdr pane.
- After `stop`, every mode must read 0 and `alternate=0`: the terminal was restored.
- Send `Escape` on its own (the script does); tmux otherwise joins it with the next key
  into Alt-<key>.
- `log NAME` shows the tail of radar.log ("Ink renderer stopped: <why>", restarts) and
  stderr.

## Failure messages

- "fell back to Ratatui": the bundle or `node` was missing, or the renderer failed to
  start four times in a minute; the reason is printed above it. One known cause is a
  socket path over 100 bytes; the script keeps its run folder short for that reason.
- "nothing drawn after 15 s": read the printed stderr and radar.log tail.

## When the check must be scripted

For a check that runs again (a test, not a look), use `tests/ink_pty.rs`:
`cd ink && npm run bundle`, then
`CARGO_TARGET_DIR=target/fleet cargo nextest run -p radar --test ink_pty --run-ignored all`,
with `RADAR_PTY_OUT=<dir>` to keep the screen at each step. Its
`live_read_only_through_ink_at_160x40` walks `RADAR_PTY_SCRIPT='keys=>words | ...'` over
`RADAR_PTY_CONFIG=<config.toml>`.
