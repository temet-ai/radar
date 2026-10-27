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

### Prerequisites

- Git and a Rust toolchain. The workspace declares Rust 1.90 as its minimum;
  CI uses the current stable toolchain.
- Node and npm. Node 26 from nodejs.org matches CI and can build the embedded
  runtime. Running the renderer bundle needs Node 22 or later;
  `scripts/build-runtime` needs Node 25.5 or later for `node --build-sea`, built
  with single executables in. Homebrew's node 26 is built without them, so when the
  node on PATH cannot build one, build-runtime uses the newest install on the
  machine that can (nvm, fnm, volta, asdf, mise, Homebrew's kegs, nvm-windows) and
  says so; `RADAR_NODE=/path/to/node` names one outright.
- Your platform's C/C++ build tools and linker. On macOS, install the Xcode
  Command Line Tools. On Linux x86_64, install `mold`: `.cargo/config.toml`
  selects it with `-fuse-ld=mold`.
- Optional: `cargo-nextest` for the test runner used in CI. `cargo test` works
  for focused local tests when nextest is unavailable.

### Build and run from a checkout

```sh
git clone https://github.com/temet-ai/radar.git
cd radar
(cd ink && npm ci && npm run bundle)
cargo build --locked
RADAR_SANDBOX= cargo run --locked
```

A plain `cargo build` builds the Rust host; it does not bundle the renderer.
The commands above leave the renderer at `ink/dist/radar-ink.js`, which the
host finds in this checkout, and require Node on PATH at runtime. There is no
fallback renderer. To build a standalone release with an embedded runtime and
install it to `~/.local/bin/radar`, use `scripts/install`. It replaces that
local binary; use `RADAR_INSTALL_ROOT` to choose another install prefix.

For development of the Claude provider, also run
`(cd sidecars/claude-agent && npm ci --omit=optional --no-audit --no-fund)`.
The provider CLI and its own sign-in are separate; see the [agent guide](docs/user-guide/agents.md).

### Check a change

Run commands from the repository root. Replace `radar-core` with the crate you
changed; use the renderer and sidecar commands for changes in those folders:

```sh
cargo nextest run -p radar-core --locked
# Without nextest: cargo test -p radar-core --locked
cargo clippy --workspace --all-features --all-targets --locked --keep-going -- -D warnings
scripts/check-mixes
(cd ink && npm ci && npm run typecheck && npm test)
(cd sidecars/claude-agent && npm ci --omit=optional --no-audit --no-fund && npm test)
python3 scripts/check-docs
```

Keep local checks focused on the changed crates. `scripts/check` runs the batch
checks to the end and summarizes failures; CI checks the workspace.
[Testing tiers](docs/test-tiers.md) explains the coverage, and
[AGENTS.md](AGENTS.md#build-and-test) lists visual and Storybook checks.

Cargo sets `RADAR_SANDBOX` for tests, so radar opens no browser, copies nothing,
shows no notification and starts no real agent CLI. Clear it only for a live
run, as above. Cargo also sets the time zone to UTC for reproducible recordings.

### Documentation changes

Keep the [documentation index](docs/README.md) and the relevant user guide in
step with behavior. Describe the user's task first, show commands they can run,
and state prerequisites and expected results. Check names, keys, defaults and
permission boundaries against the implementation. Distinguish development on
`main` from a published release; do not promise an archive merely because its
tag exists. Installation instructions belong in the
[installation guide](docs/user-guide/install.md), with a short path from the README.

Run `python3 scripts/check-docs` for local links, images and heading anchors, then
review changed Markdown on GitHub. The checker is offline; it does not prove
external links are reachable or that installation works on every platform.
When a behavior or screen changes, update its guide and screenshot together.

## How radar is put together

radar is a host (`crates/radar-core`) plus one crate per part (GitLab, GitHub, Jira, Slack,
local Git, workspaces, the work area and agent sessions). The host owns all state and sends
view data to the renderer, Ink (`ink/`).
[AGENTS.md](AGENTS.md) maps the repository; [docs/GUIDE.md](docs/GUIDE.md) explains how
radar is built and its UX rules; [docs/components.md](docs/components.md) lists every UI
element.

A few rules hold in every change:

- Parts draw with the shared UI kit and never depend on one another.
- Nothing visual is hard-coded: colours are theme slots, glyphs come from the icon pack,
  widths are display cells.
- Ink draws every picture. A change to how something looks is made in `ink/`.
- Test data uses made-up names, never a real organisation, person, project or ticket.
  New test data takes its names from Terry Pratchett's Discworld (`Sam Vimes`,
  `ankh-morpork/watch`, `WATCH-12`); older fixtures use `acme`, `alice` and `ABC-12`.
  Hosts are `example.com` or `.invalid`.

## Pull requests

Keep a pull request to one change, with tests for it. Say in the description what it does
for someone using radar and how you checked it. If a snapshot changes, read its diff before
accepting it.
