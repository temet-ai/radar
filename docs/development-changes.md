# Development changes since alpha.13

This page summarizes user-facing and contributor-facing changes between
[`v0.1.0-alpha.13`](https://github.com/temet-ai/radar/tree/v0.1.0-alpha.13) and
[source commit `b58a98f`](https://github.com/temet-ai/radar/commit/b58a98f01cd478a66af756d2dae83038992b7518)
(version `0.1.0-alpha.14`). The range contains 124 commits, including merges,
fixture updates and test repairs. See the
[complete comparison](https://github.com/temet-ai/radar/compare/v0.1.0-alpha.13...b58a98f01cd478a66af756d2dae83038992b7518)
for every commit and file change.

These are **source changes**, not a promise that alpha.14 packages are available.
As checked on 2026-10-21, alpha.13 was the newest published release with archives.
Check the [releases page](https://github.com/temet-ai/radar/releases) before
installing, and match the documentation tag to `radar --version`.

## Using radar

- **One terminal renderer.** Ink is the only renderer; the old Rust terminal
  rendering path is removed. If Ink cannot start, radar restores the terminal,
  prints the cause and exits. Release binaries embed the runtime. A source build
  needs a bundle or embedded runtime; plain `cargo build` alone does not provide
  it. See [source builds](../CONTRIBUTING.md#build-and-run-from-a-checkout).
  ([Change](https://github.com/temet-ai/radar/commit/61eced5))
- **Single-line editing and paste.** One-line inputs now use radar's own editor
  state, including selection, word movement and undo/redo. Newlines pasted into
  one-line fields become spaces; restored values are kept verbatim. Ink sends a
  paste as one message to the host while a field captures text. Multiline
  comments retain their separate composer controls. See [editing keys](user-guide/keys.md#single-line-fields).
  ([Editor](https://github.com/temet-ai/radar/commit/eacf474),
  [paste](https://github.com/temet-ai/radar/commit/d76fccc),
  [restored values](https://github.com/temet-ai/radar/commit/9ef045c))
- **Navigation and layout fixes.** Clickable chips on page frames have targets;
  contributed cells can open their subject. Detail scrolling uses the space Ink
  actually measured, including undrawn panes, and rich settings sections can
  scroll to their final row. Mode chips use the selected icon pack. Slack's
  discussed-in context names the author and links the message.
  ([Chip targets](https://github.com/temet-ai/radar/commit/57ddee6),
  [subjects](https://github.com/temet-ai/radar/commit/b3573f3),
  [viewport](https://github.com/temet-ai/radar/commit/0d4388a),
  [scrolling](https://github.com/temet-ai/radar/commit/9f55a3b),
  [Slack context](https://github.com/temet-ai/radar/commit/a4bfc8c))
- **Image limitation.** Current service descriptions, tickets and threads show
  image names and links, rather than inline image previews. The Kitty splash is
  separate. Terminal capability or the images preference does not imply that
  service-image rendering is implemented. See [links and pictures](user-guide/README.md#links-and-pictures).

## Building and contributing

- **Renderer-neutral host and parts.** View data, intents and radar's own
  `Chord`, `Color`, `Modifier` and `Style` types replace terminal-library types.
  Removed painters, `Event::Key` and `Event::Mouse` are not extension points.
  Parts describe views; Ink draws them. See the [builder's guide](GUIDE.md) and
  [IPC protocol](ink-ipc.md).
  ([Types](https://github.com/temet-ai/radar/commit/16287a2),
  [removed events and painters](https://github.com/temet-ai/radar/commit/f7d882c))
- **Checks follow that boundary.** Host and part tests inspect view data and
  route intents; Ink owns visual fixtures and goldens. `scripts/shots` draws
  Ink cells, and the scenario/check tooling reads views rather than a removed
  renderer. `scripts/check` runs checks to the end and summarizes failures.
  The range also updates fixtures, protocol types and test-tier mappings to
  match these changes. See [testing tiers](test-tiers.md).
  ([Shots](https://github.com/temet-ai/radar/commit/e32614f),
  [batch checks](https://github.com/temet-ai/radar/commit/0110744),
  [host/bridge parity](https://github.com/temet-ai/radar/commit/20c2ffe))
- **Dependency updates.** The lockfile updates rustls-webpki to 0.103.16 and
  webpki-roots/webpki-root-certs to 1.0.10; removed renderer dependencies no longer
  need their old advisory exceptions.
  ([Dependency change](https://github.com/temet-ai/radar/commit/8e255b8))

Use the [installation and upgrade guide](user-guide/install.md) for backup,
version verification and rollback. This summary groups related changes; the
comparison above remains the full record, including test-only fixes.
