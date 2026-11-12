# Changelog

What changed in each radar release, for people who use it. Every release has a
section here before it is tagged; the release workflow publishes that section as the
release's notes and refuses a tag without one. Commit-level detail is in the
comparison link at the end of each release's notes.

Sections use these headings, leaving out any with nothing in them: **New**,
**Changed**, **Fixed**, **Security**, **Upgrading** (anything to do by hand).

## Unreleased

### Changed

- **Automatic reviews are an Agents setting.** Whether a review request found in
  Slack's review channel starts a local review is now **review on request** in
  Settings → Agents (`[sessions] auto_review`). Slack only finds the requests, and
  looks for them only while that is on.

### Fixed

- **Review worktrees no longer pile up.** A merge request gets one review worktree
  (`<topic>-review`), moved to each new commit, instead of one per commit. Prune
  now offers clean worktrees that are on no branch and says what commits only
  they hold.

### Upgrading

- `[slack.review] auto_review` is no longer read. To keep automatic reviews, set
  `auto_review = true` under `[sessions]`, or turn on review on request in
  Settings → Agents.

## 0.1.0-alpha.20 (2026-11-09)

### Fixed

- **Mouse moves no longer print over the screen.** When radar read a token again
  from your shell (new in alpha.18), it started the shell interactively, and that
  shell took over the terminal for a moment. Mouse moves made then were drawn as
  text across the screen, such as `35;63;20M35;64;21M…`, most often on the start
  screen. radar now starts that shell detached from the terminal. alpha.18 and
  alpha.19 have this bug.

## 0.1.0-alpha.19 (2026-11-09)

### Changed

- **Release notes say what changed.** Each release's page lists what is new,
  changed, fixed and security-related, from this file, with how to install or
  upgrade, which archive suits which machine, and every commit since the release
  before.

### Security

- **SQLite 3.54.1**, for CVE-2026-119375 (RUSTSEC-2026-0459). radar keeps its
  journal and caches in SQLite, built into the binary; it now bundles the fixed
  version (rusqlite 0.41). Existing journals and caches open as before.

## 0.1.0-alpha.18 (2026-11-09)

### New

- **A changed token works without restarting radar.** A setting written as
  `env:NAME` (a token, an email, an address) is read again from your login shell
  when you check it under Settings, when you choose `refresh now` on a service's
  chip, and about once a minute while a service is signed out. Export the new
  token in your shell and radar uses it. Before, radar kept the value it started
  with until it was restarted from a new shell. This covers GitLab, GitHub, Jira
  and Slack.
- **GitLab retries a refused token.** When GitLab answers 401 to an `env:` token,
  radar reads the variable again and retries once if the value changed.

### Changed

- **When a service last synced is a clock time.** The top bar and each service's
  health menu say `synced 10:44` instead of `synced 4s ago`, so the bar stays still
  between syncs.

### Fixed

- **Right-clicks in Herdr.** In the Herdr terminal app, radar asks Herdr to pass
  right-clicks through, so a row's menu opens on right-click.

### Security

- RUSTSEC-2026-0429 (the `zip` crate) is listed as ignored with the earlier zip
  advisories. It reaches radar through the Copilot SDK, which never calls the
  affected function; the SDK release that drops it changes radar's JSON key order,
  so it waits for a later SDK.

## 0.1.0-alpha.17 (2026-10-31)

### New

- **Drag to resize.** Drag column headings and column edges to resize or reorder
  columns, and pane borders to resize panes; a double click on a column edge fits
  the column. radar keeps the layout in the `[layout]` section of its config.
- **Row menus.** `x` or a right-click on a row opens a menu of what acts on that
  row, grouped by what it is for; filter, refresh, folds and other pane actions
  moved to the pane's menu (right-click on empty space, or `x` again). A gear in
  the menu customises it.
- **Slack review asks post at once.** "Ask for review in Slack" on a merge request
  posts to the review channel when chosen. The new `posting` row under Slack's
  review settings (`draft_first`) brings back the draft page and confirm.
  Review asks name the merge request by repository, number, title, author,
  branches and tickets; the template's help lists every placeholder with an example.

### Fixed

- **GitLab merges are followed to the end.** After GitLab accepts a merge, the
  merge request's state reads `merging · checking` until GitLab says it merged,
  refused it (with GitLab's reason), or stopped (`not merging · merge again`).
  Before, a merge GitLab dropped looked ready to merge for good.
- **Installing from source** stops with the error when the runtime build fails,
  and finds a Node that can build radar's single executable when the one on
  `PATH` cannot.

### Security

- tokio 1.54.1 and pulldown-cmark 0.13.6, for RUSTSEC-2026-0386 and
  RUSTSEC-2026-0414.

## 0.1.0-alpha.16 (2026-10-22)

### Changed

- **Documentation.** The install guide, user guides and renderer notes were
  checked against the code and corrected, including the Intel Mac install steps
  that alpha.15 brought back. radar itself is the same as alpha.15.

## 0.1.0-alpha.15 (2026-10-21)

### Fixed

- **Intel Macs are back.** The release builds for Intel Macs again (they were
  dropped in alpha.11), and the Homebrew formula loads on them; before, Homebrew
  on an Intel Mac stopped with "formula requires at least a URL".

## 0.1.0-alpha.14 (2026-10-21)

### Changed

- **One renderer.** Ink draws every screen; the older built-in terminal renderer
  is gone. If Ink cannot start, radar restores the terminal, prints why and exits.
  Release builds carry Ink inside the binary, so nothing changes for you.
- **Single-line fields edit like the rest.** One-line inputs have selection,
  word movement and undo/redo; a pasted line break becomes a space.

### Fixed

- Clickable chips on page frames, navigation and layout fixes across screens.

### Security

- rustls-webpki 0.103.16 and other dependency updates.

The full list of 124 changes is in
[docs/development-changes.md](docs/development-changes.md).

## 0.1.0-alpha.13 (2026-10-19)

### Changed

- **Local reviews ask less.** An agent reviewing a merge request may run more
  read-only commands without asking (output redirects, `jq`, `cut`, `tr`, line
  printing with `sed`, reads in the repository around the worktree), and reads
  only the threads or sections it was briefed on. Context snapshots last through
  a settings save; a merge request's context lists its repository without walking
  through it.
- **Every page and workflow is in the user guide.**

## 0.1.0-alpha.12 (2026-10-18)

alpha.11 was tagged but not published; its changes are here.

### New

- **Agents share radar's context.** Agents started from radar's dock and agents
  you run yourself read the same context over radar's MCP server: what radar
  holds about a merge request, ticket or thread, within the scope you configured.
- **Slack review asks start local reviews.** A review ask in Slack can queue a
  review of the merge request on your machine.
- **Where reviews and work go.** Review folders, working folders and worktree
  locations have defaults under `[sessions]` and overrides per repository; under
  Settings → Agents, `p` on a folder row sets one for a repository.

### Upgrading

- No Intel Mac build in this release (restored in alpha.15). On an Intel Mac,
  stay on alpha.10.

## 0.1.0-alpha.10 (2026-10-18)

### New

- **Notices last and have a history.** Information notices stay five seconds,
  errors ten seconds or until a key; Escape clears them. The session keeps the
  last 256 so you can read back what was said.

### Changed

- **Keys are underlined in their word** in action labels.
- **Detail panes, their footer actions and breadcrumbs** look and behave the same
  in every part; actions that cannot run now say why.
- **Trees** move the same way in every part.
- Coloured chips keep their background in every theme; dimmed backgrounds behind
  menus fade in and out cleanly; the loading shimmer stops when loading stops.

## 0.1.0-alpha.9 (2026-10-18)

### New

- **Command palette.** `ctrl-p` searches merge requests, tickets, threads and
  places across every part from what radar already holds, and runs or opens the
  one you choose. Your most used places come first.

### Changed

- **Tabs show names only**, with a dot when something needs your attention.
- **Motion:** the focused frame and the radar wordmark fade in, and folded rows
  unroll when opened.

## 0.1.0-alpha.8 (2026-10-17)

No changes you will notice: test and tooling work, and a fix for a newer Rust
compiler.

## 0.1.0-alpha.7 (2026-10-16)

### Changed

- **Faster.** The Work screen and History answer in milliseconds; diffs draw only
  the lines on screen; switching screens fades in over 100 ms with the words
  readable from the first frame.
- **The README and privacy notes** say what radar is for, part by part, and in
  plain terms what it stores and what it talks to.

## 0.1.0-alpha.6 (2026-10-16)

### New

- **First-run setup.** A welcome, then setup in its own panes: services, agents,
  where your code is, your terminal, and last whether radar may write (edit mode)
  or only read. The radar splash shows at each start.
- **Select and copy with the mouse** inside panes.
- **Settings → Look** chooses the icon pack, theme and motion, with a preview.
- The terminal list shows only terminals installed here, the one radar runs in
  first; Ghostty on a Mac opens the folder in the running Ghostty.

### Fixed

- **radar starts every time.** It failed to start about four times in ten.
- **The agent dock:** "always allow" works by click as by key and answers the
  agent's question; a session restored after a restart is read back from its
  provider when you first open it; a new session's brief is never lost before the
  provider listens; the dock takes keys only where it is drawn; a streamed word
  redraws what changed, not the whole session.

## 0.1.0-alpha.5 (2026-10-15)

The first public release. radar is a terminal workspace over GitLab, GitHub, Jira
and Slack: merge requests, pull requests, tickets and the Slack threads about
them on one screen, linked to each other and to the code checked out on your
machine.

### New

- **One list of what needs you**, across GitLab groups, GitHub organisations,
  Jira sites and Slack channels, with pipelines and checks.
- **Act from the keyboard:** comment, reply, approve, rebase, merge, move a
  ticket or check out a branch. Every write is shown in full and sent only when you
  say yes; in read-only mode radar sends nothing.
- **Your local work in the same place:** checkouts of the repositories shown,
  opened in your editor or terminal, and a private local history of what you did.
- **Agents beside the work:** a dock that runs Claude Code, Codex or GitHub
  Copilot with your own CLI and login, and hands an agent a merge request or
  ticket with its context written down.
- One file to install, through Homebrew or mise; no Node, Python or other
  runtime. Works in any modern terminal, over SSH and in tmux.

## Earlier releases

Alpha.1 to alpha.4 were private and are not published.
