# User guide screenshots

All drawn by the Ink renderer (`ink-live` proves it), 160x40 unless the name says otherwise. In the original capture set, GitLab, Jira,
GitHub and Slack are one local stand-in (`tools/fake-services.py`, an invented `acme` organisation with
alice, bob and carol), so every screen shows a normal, successful sync; the agent is the fake provider;
there are three small git checkouts and a few local work items. Keys are typed from the first screen after
start; `R J S B I H ,` are Shift-letters.

- splash.png — the start splash: the radar sweep with a contact for each part, the mark, the parts' names and that start's line (a new one each start, so a regenerated picture has another) — none; it shows for the first moments after `radar` starts
- setup-scan.png, setup-choose.png, setup-look.png, setup-ready.png — setup's first question, the chooser (the look said yes; nothing was found, so nothing is ticked but look), the look & terminal step and ready — `RADAR_LIVE_FRESH=1 ink-live start NAME ink 160x40` (no config, so setup opens), then `⏎` to each; taken by hand, not yet in `capture-all.sh`
- setup-slack.png — setup's Slack step on its own, no config: the card (the same section as Settings › Slack › connection) and the help beside it, with a "Found in your environment" question from a dummy `SLACK_TOKEN` (a fake value, never used) — `RADAR_LIVE_FRESH=1 RADAR_ARGS='setup slack' ink-live start NAME ink 160x40`. Take it at 160x40: at 80x24 Ink cuts the found row. The other services' steps are their Settings connection pictures below
- demo.gif — the README's clip, 120x36, about 23 seconds: the splash, Repositories, !263's page and its Changes tab, Jira, Slack, then an agent asked about !263 — `tools/clip.sh demo`
- repositories.png — GitLab source of Repositories: views, tree of team-a/api/web with branches, detail pane — `R`
- repositories-100x30.png — the same screen at 100x30 (the top bar shrinks to letters) — `R`, terminal resized to 100x30
- menu-row-actions.png — the actions menu of merge request !263 (open, GitLab, agent, Jira, Git, Work, History groups) — `R`, right-click on the `!263` row (`x` opens the same menu from the keyboard)
- detail-merge-request.png — the page of !263 (Overview tab, "can it merge?") — `R`, click on `!263`, `⏎`
- agent-dock.png — the agent dock under the middle pane with a fake agent session: brief, a typed message and the reply — `R`, click on `!263`, `A`, type a message, `⏎`
- github.png — GitHub source: a group (`acme-labs`), two repositories, their pull requests and the open one — `R`, click the `GitHub` source tab
- local-workspaces.png — Local workspaces source: the workspace `code` with its clones `api` and `web` — `R`, click the `Local workspaces` source tab, `↓`, `r`, `↓`
- jira.png — Jira: site, status group and ABC-12 — `J`
- detail-jira-issue.png — the page of ABC-12 (Overview tab, details pane) — `J`, click on `ABC-12`, `⏎`
- slack.png — Slack, For you: mentions of you, of your group, direct messages and threads you are in — `S`
- slack-channel.png — Slack, the `#team` channel: messages by day, a thread with replies, one detail — `S`, click `#team` in the left list
- branches.png — Git: clones api and web, their branches, dirty and stash columns, detail of api — `B`
- work.png — Work: three local tasks and the group `This week` — `I`, `r`. The tasks are written during the capture with `radar work apply` (the screen's own `n` cannot save under the read-only mode `ink-live` runs radar in)
- history.png — History: the read-only activity list and one event's detail — `H`
- settings.general.parts.png — Settings, General › parts — `,`, click `parts` in the nav (or `↓` to it)
- settings.gitlab.connection.png — Settings, GitLab › connection — `,`, click `connection` under GitLab
- settings.github.connection.png — Settings, GitHub › connection — `,`, click `connection` under GitHub
- settings.jira.connection.png — Settings, Jira › connection — `,`, click `connection` under Jira
- settings.slack.connection.png — Settings, Slack › connection (no token set) — `,`, click `connection` under Slack
- settings.sessions.agents.png — Settings, Agents › Tools and defaults (provider `fake`) — `,`, click `Tools and defaults` under Agents

## Additional page-guide captures

Captured on 2026-10-19 with radar `0.1.0-alpha.12` (`2722ec0`), Ink at 160×40.
These use `ink-live`'s bundled offline cache and a scratch configuration, without
the fake-service server. The unset-token notice is expected; no service sync is
attempted with real credentials. New folder examples use `ankh-morpork/watch`.

- settings.repository-locations.png — Agents / Tools and defaults, **working folder**, `p` for repository overrides; add `ankh-morpork/watch`, set working folder `~/work/watch` and reviews folder `~/reviews/watch`, then reopen its menu. The checkout folder inherits its default. Only the scratch config is changed; the folders are not created.
- command-palette.png — close the settings menu, `R`, `ctrl-p`, type `repo`; shows matching screens, settings, actions and cached repositories. The cache contains the older fictional `acme` fixtures.
- notice-history.png — close the palette and press `~`; the two retained notices show the saved working/review folder settings from this session.

To capture these, start with:

```sh
RADAR_BIN="$HOME/.local/bin/radar" scripts/ink-live/ink-live start docs-pages ink 160x40
```

Drive the keys and menus above, capture with `scripts/ink-live/ink-live colour docs-pages`,
and render the saved ANSI with `docs/user-guide/tools/render.sh capture.ans out.png 160 40`.
Finish with `scripts/ink-live/ink-live stop docs-pages`. These captures are not part of
`capture-all.sh`.

## Things to know when using them

- Nothing leaves the machine: the stand-in answers on `127.0.0.1` and the tokens in `tools/guide-config.toml`
  are dummy words. The address `127.0.0.1:38080` shows on the Jira, GitLab, Slack and Git screens; it is the
  stand-in's fixed port (`GUIDE_FAKE_PORT` changes it; a busy port falls back to any free one).
- Times are relative to the capture ("25m", "yesterday"), so a regenerated picture differs in its ages and clock
  times; the data does not.
- A red "pipeline failed" (!264) and "1 broken" are sample data, not errors.
- Paths in the dock and the Local workspaces detail read `/tmp/radar-live/gd/home/...`; the CLI list in
  settings.sessions.agents reads the Copilot, Codex and Claude Code versions installed on the machine that ran it.
- On Jira, the summary column is narrow (radar's layout at 160 columns), so long summaries are cut.
- settings.github.connection.png shows the stand-in's address as the server (the `[github] api` key, for tests
  and guides); with no such key it reads `https://api.github.com`.

## Regenerate them all

```bash
docs/user-guide/tools/capture-all.sh            # about two minutes; writes docs/user-guide/images/*.png
KEEP_ANS=/tmp/ans docs/user-guide/tools/capture-all.sh   # also keep each ANSI capture
```

`capture-all.sh` starts radar through `scripts/ink-live/ink-live` (so a fall back to Ratatui
fails the run), drives it by keys and clicks, and turns each `ink-live colour` capture into a PNG with
`tools/render.sh` (`ansi2html.py` puts every cell on a fixed grid, headless Chrome draws it in 0xProto Nerd
Font Mono, ImageMagick crops it). One picture alone: `tools/render.sh capture.ans out.png 160 40`
(`SCALE=2` for a sharper image). Config: `tools/guide-config.toml` (`@FAKE@` becomes the stand-in's address);
the four services: `tools/fake-services.py` (Python 3, standard library; run it alone with
`python3 tools/fake-services.py` to point a browser or `curl` at it; requests it does not know are logged to
stderr as `UNKNOWN ...`, and `capture-all.sh` prints them at the end, or keep them with `KEEP_FAKE_LOG=file`);
clones: `tools/make-fixtures.sh`. It needs a radar build that has the `[github] api` and `[slack] api` keys.
Needs `google-chrome-stable` (or Chrome/Chromium), `magick` or `convert`, `python3`, `tmux`, `node`, and a
Nerd Font named `0xProto Nerd Font Mono` (change `FONT` in `ansi2html.py` for another).

## Clips

```bash
docs/user-guide/tools/clip.sh demo              # about three minutes; writes images/demo.gif and demo.mp4
```

`clip.sh` uses the same stand-in, start and helpers as `capture-all.sh` (`tools/lib.sh`), plays one story
(a function in `clip.sh`), and while it plays `tools/frames.py` grabs the screen ten times a second, keeping
each one that changed with the time it appeared. Each is drawn as a still is (`render.sh`) and shown for as
long as it was on screen; ffmpeg joins them into a GIF (for the README, which GitHub plays inline) and an
MP4. Add a story as a function and a line in its `case`. Keep a clip short (under 30 seconds) and about one
thing; a picture is enough where nothing moves. Needs ffmpeg as well.
