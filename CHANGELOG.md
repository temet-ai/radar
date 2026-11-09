# Changelog

What changed in each radar release, for people who use it. Every release has a
section here before it is tagged; the release workflow publishes that section as the
release's notes and refuses a tag without one. Commit-level detail is in the
comparison link at the end of each release's notes.

Sections use these headings, leaving out any with nothing in them: **New**,
**Changed**, **Fixed**, **Security**, **Upgrading** (anything to do by hand).

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

## Earlier releases

Alpha.16 and before have only their [release pages](https://github.com/temet-ai/radar/releases)
and, for alpha.13 to alpha.14, [docs/development-changes.md](docs/development-changes.md).
