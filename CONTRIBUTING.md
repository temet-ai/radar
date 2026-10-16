# Contributing to radar

The most useful contributions now are bug reports from real use and small fixes. For
anything larger, open an issue first and say what you have in mind, so we can agree on the
shape before you spend time on it.
[When something looks wrong](docs/user-guide/troubleshooting.md#report-a-problem) says
what a bug report needs.

## Licence

radar is MIT licensed. By sending a pull request you agree that your contribution is
licensed under the MIT licence too. There is no contributor licence agreement to sign.

## Building and testing

You need Rust 1.90 or later and Node. The Ink renderer's bundle targets Node 22 and the
Claude Agent sidecar needs Node 20 or later. Node 25.5 or later is needed only for
`scripts/build-runtime` (`node --build-sea`), which `scripts/install` runs to embed the
runtime. From the repository root:

```bash
cargo build                                   # the radar binary, at target/debug/radar
cargo nextest run -p radar-<crate>            # the tests of the crate you changed
cargo clippy --workspace --all-features --all-targets -- -D warnings
scripts/check-mixes --quick                   # each part alone, none, and all together
cd ink && npm ci && npm run typecheck && npm test
cd sidecars/claude-agent && npm ci && npm test
```

cargo sets `RADAR_SANDBOX` for tests, so during a test radar opens no browser, copies
nothing, shows no notification and starts no real agent CLI.
To see links open from a build you run by hand, use `RADAR_SANDBOX= cargo run`.

## How radar is put together

radar is a host (`crates/radar-core`) plus one crate per part (GitLab, GitHub, Jira, Slack,
local Git, workspaces, the work area and agent sessions). The host owns all state and sends
view data to a renderer: Ink (`ink/`, the default) or Ratatui (inside radar-core).
[AGENTS.md](AGENTS.md) maps the repository; [docs/GUIDE.md](docs/GUIDE.md) explains how
radar is built and its UX rules; [docs/components.md](docs/components.md) lists every UI
element.

A few rules hold in every change:

- Parts draw with the shared UI kit and never depend on one another.
- Nothing visual is hard-coded: colours are theme slots, glyphs come from the icon pack,
  widths are display cells.
- Both renderers draw the same picture. A change to how something looks is made in both.
- Test data uses made-up names, never a real organisation, person, project or ticket.
  New test data takes its names from Terry Pratchett's Discworld (`Sam Vimes`,
  `ankh-morpork/watch`, `WATCH-12`); older fixtures use `acme`, `alice` and `ABC-12`.
  Hosts are `example.com` or `.invalid`.

## Pull requests

Keep a pull request to one change, with tests for it. Say in the description what it does
for someone using radar and how you checked it. If a snapshot changes, read its diff before
accepting it.
