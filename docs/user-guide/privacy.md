# What radar stores and sends

Radar keeps service data, local work records and session pointers on your machine.
Connected services and agent providers receive requests when their features run.
This page explains that boundary and which local files you need to preserve.
Back to the [user guide](README.md).

## Network

| To | What | When |
|---|---|---|
| Each service you connect (GitLab, GitHub, Jira, Slack), at the address you configured | API requests with that service's token: reads, and the writes you confirm | while radar syncs, and when you act |
| The npm registry (`registry.npmjs.org`) | Anthropic's Claude Agent SDK at radar's pinned version, checked against its pinned checksum | first use when that SDK is not already installed or cached |
| The selected agent provider | Prompts, supplied work context, tool results and the provider's own requests | when you use an agent or an enabled automatic review starts one |
| Git remotes and programs you launch | Fetches/pushes, opened URLs, or requests made by an editor, terminal command, worktree hook or agent tool | when the corresponding operation runs |

The service connection determines where its API credentials are sent. The current
renderer shows images in service descriptions, tickets and threads as labels and
links, rather than downloading and drawing inline previews. Opening a link delegates
to your browser or the terminal's link handler. Processes started by radar can make
their own network requests. The Kitty splash picture is generated locally.

Confirmed service writes follow radar's [mode](README.md#the-mode). That mode is not
a network or filesystem sandbox for local Git operations, terminal programs or agents.
Agent approvals are described in [Agents](agents.md#approvals).

## Coding agents

Claude Agent, Codex and Copilot use their provider's authentication and session storage.
Radar checks sign-in status and can start the provider's sign-in flow; session transcripts
remain provider-owned. Radar receives the live events needed to draw the dock and can
read the provider's saved transcript when you resume a session.

The agent receives your messages and the context included in its brief. `A` supplies the
selected item and connected work; a manual or automatic review can supply the change,
linked tickets, discussions and local checkout information. Enabling
[automatic Slack reviews](mcp-and-reviews.md#start-reviews-from-slack) allows qualifying
requests to start this process without pressing `A` for each request. Reviews are saved
as local Markdown and are not automatically posted to Slack or the code host.

### Shared MCP context

Radar's integrated provider sessions and external clients using `radar mcp` can read the
same host context. They can query the host's configured caches, not just the item used
to start a particular session. Arbitrary programs in a PTY tab do not receive an injected
MCP connection.

One context server belongs to the running radar host. It uses a local Unix socket in a
private directory, checks the connecting user ID and requires the host's discovery
ticket. The directory is mode `0700`; its socket and endpoint file are `0600`. The
ticket is shared by clients of that host, not a one-time ticket for one session. Closing
one agent session does not stop the host's context server; closing the host does.

The tools read configured cached sources. Captured context snapshots stay in host memory
for up to an hour, subject to capacity eviction, and are invalidated when their
configuration/account scope changes. Credentials are redacted from returned source
text, but the returned work content can still be sensitive and becomes available to
the receiving client/provider. See [Connected context](mcp-and-reviews.md) for the tools,
coverage and snapshot limits.

## Files

| What | Where |
|---|---|
| Config, custom icon packs and recipe overrides | `~/.config/radar/` |
| Service caches and persisted part state, including local read marks, automatic-review intake/queue state, session pointers and pending Work recovery data | `~/.local/state/radar/cache.sqlite3` |
| Action history **and Work tasks, authored labels, source bindings and relations** | `~/.local/state/radar/journal/actions.sqlite3` |
| Diagnostic log and its previous rotation | `~/.local/state/radar/radar.log`, `radar.log.1` |
| Image cache retained from earlier versions (200 MB limit when populated) | `~/.cache/radar/images` (macOS: `~/Library/Caches/radar/images`) |
| the renderer, unpacked from the binary | `~/.cache/radar/runtime` (macOS: `~/Library/Caches/radar/runtime`) |
| the Claude Agent SDK, per version | `~/.cache/radar/claude-agent-sdk/<version>` (macOS: `~/Library/Caches/radar/claude-agent-sdk/<version>`) |
| Local review Markdown | Repository review folder → global review folder → workspace note routing → `~/.local/state/radar/reviews` |
| Workspace notes, generated ticket READMEs and Git worktrees | The configured workspace and Git destinations, which may be outside radar's state directory |
| sockets of terminal tabs in the dock | `~/.local/state/radar/terminals` |
| Running host's MCP socket and discovery ticket | `$XDG_RUNTIME_DIR/radar-mcp/…`, or `~/.config/radar/run/radar-mcp/…` when no runtime directory is set |
| which start lines showed recently, by id | `~/.local/state/radar/start-lines.toml` |

`XDG_CONFIG_HOME`, `XDG_STATE_HOME` and `XDG_CACHE_HOME` select the corresponding roots;
use absolute paths. `XDG_RUNTIME_DIR` selects the MCP runtime location. Review files,
agent working folders and worktrees have their own
[global and per-repository location settings](mcp-and-reviews.md#where-reviews-and-work-are-saved).
`RADAR_CLAUDE_SDK_DIR` can also override the SDK cache destination.

The config file holds a token if you typed one in. Otherwise it holds where the token
is: `env:GITLAB_TOKEN`, or `glab` for the GitLab CLI's login.

Service caches contain descriptions, comments and message text returned by their
services. Slack's account partitions may also store a noncryptographic token
fingerprint to select a cached preview before fresh authentication. This does not
authenticate the account; see [Slack cache and recovery](../slack-cache.md). They also hold local state that a fresh remote sync cannot reconstruct:
Slack read marks and review checkpoints, queued automatic reviews and their source
context, session IDs/cursors, review records, and pending Work changes. The session
metadata records working folder, title, provider, model and resume cursor; radar does
not persist the dock's full conversation timeline in that session metadata.

Ordinary action events store bounded operation metadata and scoped subjects rather than
full service responses or conversation transcripts. **Work's authored titles and group
labels are stored text in the same journal database**, together with source bindings
and relations. Paths, identifiers and labels still describe your work. The cache and
journal databases are private files (`0600`); they are not encrypted. The diagnostic
logger redacts registered secrets and recognized credential patterns before writing.

Older installations may also have per-part JSON cache files and `.json.imported`
backups. Radar imports old caches into SQLite; if SQLite cannot be opened, it can fall
back to the JSON files. Keep these in mind when backing up or removing local data.

### What can be regenerated, and what cannot

Close all radar hosts before backing up or removing their state. Preserve SQLite
databases together with any `-wal` and `-shm` companion files still present; do not
remove an active process's socket or database files.

| Data | Effect of deleting it |
|---|---|
| Images, unpacked runtime, cached Claude SDK | Recreated or downloaded when next needed; an offline launch may need the SDK cache retained |
| Diagnostic logs or start-line history | Removes diagnostics or recent-line preferences; new files can be created |
| `cache.sqlite3` and legacy part caches | Remote data can reload, but local read marks, automation checkpoints/queues, session pointers and pending recovery state are lost or reset |
| `journal/actions.sqlite3` | Permanently removes recorded History and local Work annotations; syncing services cannot rebuild your tasks, groups and relations |
| Config and custom packs/recipes | Removes your connection references, preferences and authored customizations; radar starts without them |
| Review Markdown, workspace notes and worktrees | Removes local work products or working files; radar cannot reconstruct their contents merely by syncing |

Back up the journal, caches, configuration and the actual configured notes/review folders
if you want to preserve your workspace. Deleting radar's session pointers does not delete
the provider's own transcripts. Deleting local files also does not undo comments, merges
or other writes already made to a service.
