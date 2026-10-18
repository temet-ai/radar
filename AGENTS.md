# radar

A Rust terminal workspace over GitLab, GitHub, Jira,
Slack and local Git, built as a host (`crates/radar-core`) plus one crate per part. The host
owns all state and sends view data; two renderers draw it: Ratatui (in radar-core) and
Ink/React (`ink/`, the default). Read this file first; it points to the rest.

## Where things are

| What | Where |
|---|---|
| How radar is built, the UX contract, testing | `docs/GUIDE.md` (sections 1, 2, 3) |
| Every UI element, who draws it, canonical forms, known drift | `docs/components.md` |
| Host, plugin contract, theme, UI kit, Ratatui renderer | `crates/radar-core` (`ui/`, `design/`, `view.rs`, `view_tui.rs`, `shell.rs`) |
| Parts | `crates/radar-gitlab`, `radar-github`, `radar-jira`, `radar-slack`, `radar-git`, `radar-workspace`, `radar-workbench` |
| Ink renderer | `ink/src` (`app`, `chrome`, `dock`, `editor`, `input`, `media`, `motion`, `nodes`, `state`, `terminal`, `text`, `transport`, `workarea`); protocol types are generated into `ink/src/protocol/generated/` from the Rust types, never edited by hand |
| Agent sessions: the adapter contract, event envelope, cursor store, fake provider, briefs, and the state the agent dock draws | `crates/radar-sessions` |
| Agent providers: Copilot (Rust SDK), Codex (App Server), Claude Agent (Agent SDK sidecar in `sidecars/claude-agent`; the SDK is fetched on first use), any CLI in a PTY tab | `crates/radar-sessions-copilot`, `radar-sessions-codex`, `radar-sessions-claude`, `radar-sessions-pty` |
| How radar frames context for an agent, and the recipes it sends with it | `agents/briefing.md` (instructions), `agents/recipes/` (one file per recipe, versioned; `README.md` lists them) |
| The Claude Agent SDK sidecar: its Node source, tests and README | `sidecars/claude-agent` (`src`, `test`, `README.md`) |
| Component docs (source of truth): each Ink component drawn from recorded view data beside Ratatui's picture, its states and rules | Storybook in `ink/` (`ink/stories`, `ink/.storybook`); MCP at `http://localhost:6016/mcp` while it runs; the manifest committed at `docs/storybook/components.json` |

## Build and test

```bash
cargo nextest run -p radar-<crate>     # test only the crate you changed (cargo test -p if no nextest)
cargo clippy --workspace --all-features --all-targets -- -D warnings
scripts/check-mixes                 # none, each part alone, all; when features, src/lib.rs or tests/mix.rs change (CI runs it)
scripts/shots --compare main        # when a colour, style or layout changed
cargo run -p radar-core --example gallery   # every kit element live
cd ink && npm run typecheck && npm test  # Ink
cd ink && npm run storybook         # component docs on http://localhost:6016, MCP at /mcp
cd ink && npm run manifest          # after adding or removing a story: rewrites docs/storybook/components.json
cd ink && npm run check-static      # after build-storybook: one story must render from storybook-static (headless Chrome)
scripts/install                     # build release and install as ~/.local/bin/radar
scripts/build-runtime               # Ink + the Claude sidecar as one Node executable (radar-js); install and release embed it
RADAR_RENDERER=ratatui radar              # the Ratatui renderer, for side-by-side comparison
```

- **Testing pyramid:** unit-heavy, a few integration tests per part, a tiny end-to-end per
  mix. No loops that draw a full frame per row, no runs over every combination of parts.
  A crate's suite stays under ~10 s. A session tests the crates it changed, not the
  workspace. Details: GUIDE section 3.
- **Read a snapshot diff before accepting it.** `tests/mix.rs` keeps `INSTA_UPDATE`, so
  `INSTA_UPDATE=always` rewrites a mix's recordings in one run; read the `git diff` after.
- **Build folders:** use the repo's `target/`, never a per-task target dir.
- **Tests are sandboxed:** cargo sets `RADAR_SANDBOX`, so no test opens a browser, the
  clipboard, a notification or a real agent CLI. `RADAR_SANDBOX= cargo run` for a live run.
- **Storybook's MCP for agents:** start `npm run storybook` in `ink/`, then ask the `storybook` MCP server for
  component docs. Claude Code finds it in the project `.mcp.json`. Codex needs it in `~/.codex/config.toml`:

  ```toml
  [mcp_servers.storybook]
  url = "http://localhost:6016/mcp"
  ```

  With no Storybook running, read `docs/storybook/components.json` (the same manifest). Ink's `npm test` fails when
  a story was added or removed without `npm run manifest`. The manifest command streams build errors and
  exits nonzero on failure, preserving the committed manifest (which may still be stale). If Storybook reports
  `uv_interface_addresses` with a permission error, rerun `cd ink && npm run manifest` from the repository root
  in an authorized environment that permits network-interface enumeration; do not disable sandbox protections.

## UI rules (hold in every change)

The full set is in `docs/components.md` section 1 and GUIDE sections 1.15, 1.18 and 2.

- **Parts draw with the kit.** A missing element goes into the kit first, with gallery
  snapshots at 80 and 160 wide, then into `docs/components.md`.
- **Nothing visual is hard-coded:** colours are theme slots and look words; glyphs come
  from the icon pack (`ui::icon`); widths are display cells (`ui::width`), never
  `chars().count()` or `{:<N}`.
- **Both renderers draw the same picture.** While Ratatui is the reference, a look change
  lands in both; a difference is a defect.
- **No chevrons** (▸ ▾ › ⌄, `[+]`, `[-]`) for folding: tree lines show open, a folded row
  ends in `⋯ n`.
- **No pill shapes, no left-edge stripes.** Chips, counts, keys and buttons are square.
- **Keys sit inside their word**, underlined (b̲ulk); glyph keys (↑↓ ⏎ esc) go before the word.
- **Breadcrumbs join with `/`.** Tabs show names, not counts.
- **Same key, same action, in the same kind of place** across parts; keys may differ
  between kinds of place.
- **Icons wherever they mean something**, several to a row if each says something.

## Working here

- `scripts/ink-live/ink-live` runs the real radar in tmux with the Ink renderer (or Ratatui)
  and reads its screen as text or colour: the live check for anything visible.
- A recording of a real agent CLI for the adapter tests is scrubbed with
  `scripts/scan-secrets` before it is committed.
- **No real names in the repo:** no organisation, person, project or ticket key from real
  work, in code, tests, docs or commit messages. New test data uses Terry Pratchett names:
  people (`Sam Vimes`, `vimes`), organisations and groups (`ankh-morpork/watch`), places,
  ticket keys (`WATCH-12`). Older fixtures still use `acme`, `alice` and `ABC-12`.
