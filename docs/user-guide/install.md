# Install and upgrade radar

Back to the [user guide](README.md). After installation, follow the
[local Git walkthrough](quickstart.md) or [connect your services](connect.md).

## Before you start

The release is a single binary with its terminal renderer built in. You do not
need Rust, Node or Python to run it. Git is needed for local checkout operations.
Agent CLIs are optional and are [installed and signed in separately](agents.md).

Current release archives cover:

| Your machine | Archive target |
|---|---|
| macOS, Apple Silicon | `aarch64-apple-darwin` |
| Linux, x86_64 (x64) | `x86_64-unknown-linux-gnu` |
| Linux, aarch64 (ARM64) | `aarch64-unknown-linux-gnu` |

Linux builds require glibc 2.35 or later. Alpine/musl, older glibc distributions
and Intel Macs are outside the current release matrix. Windows has no native
build; use a supported Linux distribution in WSL2. Run `uname -s` and `uname -m`
inside the environment where radar will run to check its OS and architecture.

Radar is in alpha. Use the [releases page](https://github.com/temet-ai/radar/releases)
to choose a **published release with assets**, and read its notes before upgrading.
The source and documentation on `main` can be ahead of downloadable packages.
`radar --version` prints the installed version and commit; read the guides at
that tag when you need instructions for that version.

## Homebrew

With Homebrew already installed, run:

```sh
brew install temet-ai/radar/radar
radar --version
```

Always use the full name. Homebrew has an unrelated app named `radar`, and the
short name can install that one. The formula selects your platform's archive
and verifies its checksum. The tap may lag a newly published release until its
formula is updated.

To upgrade:

```sh
brew update
brew upgrade temet-ai/radar/radar
radar --version
```

## mise

With [mise](https://mise.jdx.dev/dev-tools/backends/github.html) installed and activated in your shell, add this
to your project's `mise.toml`:

```toml
[tools]
"github:temet-ai/radar" = { version = "latest", prerelease = true, minimum_release_age = "0s" }
```

Then run:

```sh
mise install
mise exec -- radar --version
```

`prerelease = true` includes alpha releases. `minimum_release_age = "0s"` opts
into newly published releases immediately. Remove that option if you prefer
mise's usual waiting period. To keep a project reproducible, replace `latest`
with a published version such as `0.1.0-alpha.13`.

Run `mise upgrade github:temet-ai/radar` to update a `latest` entry. For a pinned
entry, change the version intentionally and run `mise install` again. If `radar`
is not found directly, check mise's shell activation or use `mise exec -- radar`.

## Download and verify an archive

1. Open the [releases page](https://github.com/temet-ai/radar/releases). Download
   the archive for your machine and `checksums.txt` from the **same release**.
   Work in a new empty folder so older files cannot be mistaken for this download.
2. Set `archive` to the exact filename you downloaded. This example is a published
   Linux x64 release; replace its version and target when choosing another:

   ```sh
   archive=radar-v0.1.0-alpha.13-x86_64-unknown-linux-gnu.tar.gz
   ```

3. Select its checksum entry. The manifest may prefix filenames with `./`:

   ```sh
   awk -v file="$archive" '$2 == file || $2 == "./" file' checksums.txt > selected-checksum.txt
   test "$(wc -l < selected-checksum.txt | tr -d ' ')" = 1
   ```

   If the `test` command fails, stop: the archive and manifest do not match.
   On Linux, verify with:

   ```sh
   sha256sum -c selected-checksum.txt
   ```

   On macOS, verify with:

   ```sh
   shasum -a 256 -c selected-checksum.txt
   ```

   Continue only if the verifier reports `OK`. If it fails, download both files
   again from the same release. Checksums detect damaged or mismatched downloads;
   they do not replace trust in the release publisher.
4. Extract and install into your own bin directory, without `sudo`:

   ```sh
   tar -xzf "$archive"
   mkdir -p "$HOME/.local/bin"
   install -m 755 radar "$HOME/.local/bin/radar"
   export PATH="$HOME/.local/bin:$PATH"
   radar --version
   ```

   This replaces an existing `~/.local/bin/radar`. Keep the archive, LICENSE and
   THIRD-PARTY-NOTICES.md if you want a record of this installation. Add the PATH
   line to your shell's startup file to make it persistent, then open a new shell.
   Use one installation method where possible; `command -v radar` shows which
   binary your shell will run.

Run `radar` to start setup. You can use local Git first without a service token.

## Upgrade safely or return to an earlier release

Quit radar before replacing its binary. Back up your config and local data before
an alpha upgrade: `radar config path` locates the config, and
[What radar stores and sends](privacy.md#files) lists the cache and journal.
These may contain credentials and private work, so keep backups private.

Use the same installation method to upgrade. For a manual install, repeat the
archive verification and installation steps with the new release. Check
`command -v radar` and `radar --version` afterward, especially if you have used
both a package manager and a manual install.

For rollback, pin an earlier published version in mise or verify and install its
archive. Homebrew's tap follows its current formula; `brew pin` prevents future
upgrades but does not reinstall an older version. Alpha data compatibility is
not guaranteed: an older binary may not understand newer config or state. Keep
the original backup and consult the release notes before restoring it. Do not
delete the cache or journal as a routine fix.

## Installation problems

- **Command not found:** check `command -v radar`, your PATH, and your package
  manager's shell setup. A manual binary can be checked directly with
  `"$HOME/.local/bin/radar" --version`.
- **Wrong version or wrong app:** use the full Homebrew formula name and check
  `command -v radar`. Your shell may be finding a different installation first.
- **Exec format / wrong architecture:** choose the target matching `uname -m`.
- **GLIBC version not found:** use a supported Linux distribution; these archives
  are not musl builds and do not support glibc older than 2.35.
- **Renderer did not start:** reinstall the verified release; source checkouts
  need a renderer bundle or embedded runtime. See [building from source](../../CONTRIBUTING.md#building-and-testing).
- **macOS blocks the download:** verify its source and checksum, then follow
  macOS's security guidance. Do not disable system-wide security protections.

For connection, display and agent problems, use
[When something looks wrong](troubleshooting.md).
