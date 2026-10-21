# Alpha releases

## Version and artifacts

Use one workspace SemVer version: `0.1.0-alpha.5`, then increment the final
number for each alpha. Commit the version and lockfile together. An annotated
`v0.1.0-alpha.5` tag on `main` of `temet-ai/radar` triggers `.github/workflows/release-alpha.yml`.
The workflow rejects a tag that disagrees with Cargo, builds all default parts,
checks `radar --version` and `radar config path`, and publishes a GitHub prerelease
with SHA-256 checksums. Never move a released tag or replace its assets; publish
the next alpha instead. `radar --version` includes the Cargo version and commit.

The first matrix covers macOS Apple Silicon and Intel (built on Apple Silicon, with
an x64 node under Rosetta for the runtime), Linux arm64 and x64
(glibc, built on Ubuntu 22.04). The archives contain `radar`, LICENSE and
THIRD-PARTY-NOTICES.md; users need no
Rust toolchain, source checkout or Node, as that one file carries radar's runtime
(see Terminal renderer). There is no Windows build yet: the host's Unix sockets
and PTY code have no Windows side, so Windows users run the Linux build in WSL2.
Linux users on older glibc distributions and other architectures remain outside
this matrix.

## Distribution

radar is developed in [`temet-ai/radar`](https://github.com/temet-ai/radar), and its
releases are the single source for package managers.

The public `temet-ai/homebrew-radar` tap contains `Formula/radar.rb`. After each
prerelease, render it from that release's
`checksums.txt` and commit the new formula:

```sh
scripts/render-homebrew-formula 0.1.0-alpha.5 temet-ai/radar checksums.txt > Formula/radar.rb
brew install temet-ai/radar/radar
brew update && brew upgrade temet-ai/radar/radar
```

Install docs always give the full name `temet-ai/radar/radar`. Homebrew has an
unrelated cask called `radar`, and on Homebrew 7.0.9 `brew install radar` installs
that cask even with this tap tapped and trusted (checked 2026-10-15). The full name
also trusts the formula, which Homebrew now asks of third-party taps.

The formula selects the native binary, pins its SHA-256, and installs it into
Homebrew's `bin`, with LICENSE and THIRD-PARTY-NOTICES.md in its prefix. It does not write user configuration or state. Test it on a
macOS runner and Linuxbrew before sharing the installation command. An alpha
in a third-party tap avoids the stable-release and open-source requirements of
`homebrew/core`.

mise can use the same public release assets on macOS and Linux:

```toml
[tools]
"github:temet-ai/radar" = { version = "latest", prerelease = true, minimum_release_age = "0s" }
```

Run `mise install` to install and `mise upgrade` to follow the next alpha.
The `prerelease = true` option is necessary because GitHub marks these releases
as prereleases. The tool-specific `minimum_release_age = "0s"` allows immediate
alpha updates; mise otherwise holds new GitHub releases for 24 hours by default.
Pin `version = "0.1.0-alpha.5"` for reproducible projects.
mise chooses the platform archive and checks its release digest. For a private
release repository, each user needs repository access and an authenticated
GitHub token; the public distribution route is simpler.

## Terminal renderer

radar draws its TUI with one renderer, Ink/React: a child process that draws the
host's views over radar's IPC ([ink-ipc.md](ink-ipc.md)). There is no switch and no
fallback. When Ink cannot start (no `radar-js`, no bundle or node, a refused
protocol, a crash loop), radar restores the terminal, prints
`radar: the renderer did not start: <why>` and `run scripts/install to rebuild it`,
and exits 1.

Release notes carry one line for this: to go back to an earlier renderer, pin the
previous release.

### radar's runtime: one file, no node

Ink and the Claude Code sidecar are JavaScript. `scripts/build-runtime`
bundles both (`runtime/entry.mjs`) into one Node single executable, `radar-js`
(about 120 MB; `radar-js ink`, `radar-js claude-agent`), built with
`node --build-sea` from Node 25.5 or later. With `RADAR_RUNTIME_EMBED=<radar-js>`
set at build time, `build.rs` embeds it in the radar binary; on start radar writes it
once to `<cache>/radar/runtime/<version>-<hash>/radar-js` (`~/.cache` on Linux,
`~/Library/Caches` on macOS), removes the ones older builds wrote, and names it
in `RADAR_RUNTIME` for itself and the processes it starts. The release jobs for
macOS and Linux and `scripts/install` both build this way, so `radar` is about
180 MB and the archive about 60 MB.

Where radar starts the renderer from, in order: `node <RADAR_INK_BUNDLE>` when that
variable is set; `<RADAR_RUNTIME> ink`; `node` on `PATH` with
`<folder of radar>/../share/radar/ink/radar-ink.js` or `ink/dist/radar-ink.js` in the
checkout (a `cargo build` without the runtime). The Claude Code sidecar the
same way: `RADAR_CLAUDE_SIDECAR` with `node`, `<RADAR_RUNTIME> claude-agent`, then
`node` with the checkout's sidecar. Setting `RADAR_RUNTIME` yourself points radar at
a runtime built elsewhere. The user's own `claude`, `codex` or `copilot` CLI is
still theirs to install, only for the agent they want.

## Publishing checklist

1. On `main`: bump the version, run `scripts/notices`, `cargo nextest run --workspace --all-features`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo deny check`, and the Ink and sidecar tests. Commit and push.
2. Create and push an annotated tag `v<version>` on that commit.
3. Wait for the four build jobs and the prerelease publication to succeed.
4. With the `HOMEBREW_TAP_TOKEN` secret set on `temet-ai/radar` (a token that can write to `temet-ai/homebrew-radar`), the publish job also commits the rendered formula to the tap. Without it the job warns: download `checksums.txt` from the release, run `scripts/render-homebrew-formula <version> temet-ai/radar checksums.txt > Formula/radar.rb` in a clone of the tap, commit and push.
5. `brew upgrade temet-ai/radar/radar` on Linux and the Mac, and run radar with no `node` on `PATH`: Ink draws it, and Settings → Agents lists the CLIs.

The release workflow needs only the repository's `contents: write` permission
to publish there; `HOMEBREW_TAP_TOKEN` is the one other credential.
