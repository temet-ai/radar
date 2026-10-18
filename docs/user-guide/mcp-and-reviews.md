# Connected context and local reviews

Radar exposes the same connected work to its dock agents and external MCP clients.
Keep radar running with your normal service and workspace configuration. The MCP client
starts `radar mcp` over stdio; radar discovers the running host automatically. If several
hosts are open, use `radar mcp --workspace /absolute/path/where/radar/was/started`.
This relay shares the host's configured account scope and caches. It does not need service
tokens in the agent's configuration. It is a local connection, not a remote HTTP endpoint.

The wire format follows the MCP [stdio transport](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports)
and [tool result contract](https://modelcontextprotocol.io/specification/2025-11-25/server/tools).
Dock sessions receive this connection automatically when they start or resume.

| Tool | Use |
|---|---|
| `search` | Find cached work by text or exact URL/ref. Filter by `kind`, `project`, or `assignee`; Jira supports `assignee: "self"` using the authenticated account ID. |
| `context` | Start from `roots` (URLs or scoped refs), or a search `selection_ref`. Follow cached relationships up to `depth` (default 2, maximum 4). |
| `read` | Read named `sections` for 1–32 `refs`. Supply `snapshot_id` to read the captured source rather than a newer cache value. |
| `changes` | Compare a `snapshot_id` with current connected cached work. It returns changed and unavailable refs. |

For example, search for `WATCH-12`, pass its `selection_ref` to `context`, then pass relevant
item refs and the returned `snapshot_id` to `read`. Follow `next_cursor` by calling the same
tool with `cursor`. `max_bytes` controls source payload size per page, from 1 KiB to 256 KiB;
metadata adds overhead. There is no cumulative read quota or approval after a fixed volume.

Context includes scoped identities, source observation times where known, content revisions,
relationship evidence, available sections and missing information. A ticket key in a branch
is an inference; an observed link is evidence of a mention. Neither proves completion or
production deployment. Source prose is evidence, not instructions for the agent.

Snapshots last up to an hour, subject to memory limits: 256 items and 8 MiB per snapshot,
32 snapshots and 32 MiB of retained source content per host. Oversized selections report
omissions; query fewer roots or sections. Expired or invalidated cursors return an error so
the agent can repeat the query. Changing configuration or account scope invalidates old
snapshots. Credentials are redacted before source text is paginated.

This version reads the host's caches. Search does not claim to cover every remote item.
Review preparation separately fetches the change and linked source details through their
owning parts. GitLab discussion pages, Slack replies and Jira comments can still be incomplete;
the response reports this. GitHub PR descriptions and checks are exposed; GitHub review
comments, remote operational logs, and a standalone headless host are not yet included.

## Start reviews from Slack

Configure a watched channel in Radar's Slack settings, choose it as the review channel, and
set this option in the existing configuration:

```toml
[slack.review]
channel = "watch-reviews" # the alias of an existing watched channel
# group = "S0123456789"  # optional Slack user-group ID
auto_review = true
```

When a group is configured, a request must mention that exact group and its thread must
link a GitLab MR or GitHub PR. Without a group, a message that links a change, or a request
containing “review” or “ptal” in a thread linking a change, qualifies. Only the configured
watched review channel is scanned. First activation starts with new messages, not old
channel history. Requests received while Radar was closed can be processed from its next
channel read, within the configured history window (at most 30 days).

Requests are queued locally and processed one at a time. The queue and handled-message
checkpoints survive restart. Repeated refreshes do not enqueue the same message again;
a revision with an existing local review is skipped. A new request after the head changes
can produce a new review. Failed or interrupted reviews can be retried with the existing
“review locally” action (`v` on the Slack thread); they are not retried in a loop.

The review uses Radar's selected workspace/default agent provider, model and versioned
`review-mr` recipe. Configure the provider's login in Agents first. Radar resolves the exact
head commit and uses a clean worktree in a known local repository. A missing checkout,
missing commit or unavailable provider produces a visible error. No repository is cloned
or change merged automatically.

Before launch, Radar gathers linked tickets, related changes, Slack discussions and local
checkout information. Extra source loading gets up to ten seconds after the change resolves;
remaining gaps are named in the brief, and the reviewer can read newer context through MCP.
A background launch preserves the selected agent tab and does not open or focus the dock.
Enabled automatic intake keeps refreshes running while idle; manually pausing Radar still
pauses intake and review preparation. MCP reads of existing caches remain available.

The agent reads the exact revision and writes a Markdown review with a verdict, findings
with file and line references, and what it could not check. Radar saves the review in its
configured reviews directory, shows its location, and offers the local review from the
MR/PR's actions. Existing review indicators show when the head has moved. Nothing is posted
to Slack, GitLab or GitHub. File edits are denied; tests and builds use the existing review
command policy. An operator can set `reviewer_may_run_commands = false` in `[sessions]` for
reviews that must finish by inspection without waiting for command approval.

## Where reviews and work are saved

Set global locations in `~/.config/radar/config.toml` (or `$XDG_CONFIG_HOME/radar/config.toml`),
then override only the fields a repository needs. These settings apply to manual reviews
and automatic Slack reviews alike. Merge these examples into existing tables:

```toml
[sessions]
reviews_dir = "~/radar/reviews"
work_dir = "~/work"

[sessions.repositories."ankh-morpork/watch"]
checkout = "~/work/watch"
reviews_dir = "~/work/watch-notes/reviews"
work_dir = "~/work/watch"

[git]
worktree = "{clone}-{topic}"

[[git.link]]
repo = "ankh-morpork/watch"
dir = "~/work/watch"
worktree = "~/worktrees/watch/{topic}"
```

- **Review files:** repository `reviews_dir`, then global `sessions.reviews_dir`, then
  automatic workspace note routing, then `$XDG_STATE_HOME/radar/reviews`
  (`~/.local/state/radar/reviews` by default). An explicit folder wins over workspace
  notes. Radar creates it when saving and shows the resulting file path. Changed reviews
  of the same revision get numbered filenames; existing files are preserved.
- **Ordinary sessions:** repository `work_dir`, then the existing workspace `work_dir`
  if set, then global `sessions.work_dir`, then Radar's launch directory. Sending an MR/PR
  to an agent identifies the repository from the selected item. A plain dock session or a
  local branch/file uses `checkout` to match the launch/subject directory; the most specific
  matching checkout wins. `checkout` is optional when only remote items need the override.
  Working folders must already exist; code changes remain in the session's working folder.
- **New worktrees:** the matching `git.link.worktree`, then global `git.worktree`, whose
  default is `{clone}-{topic}`. This applies to both ordinary worktree actions and reviews.
  `{clone}` is the main checkout's path; `{topic}` is the last branch component, with the
  short commit appended for reviews. A clean existing worktree at the required commit can
  still be reused. Review agents always run in that exact-commit worktree, independently of
  `sessions.work_dir`.

Repository keys are exact project paths. To distinguish the same path on different services,
use a service URL, for example `[sessions.repositories."https://gitlab.example.test/ankh-morpork/watch"]`;
that entry wins over the unqualified path. Omitted fields inherit independently. Session
paths must be absolute or start with `~/`. Locations are fixed for a running session/review;
settings changes apply to subsequent launches. Global review and working folders can also
be edited in Settings → Agents. Select either folder row and press `p` for **repository
overrides**: add a repository, set its checkout/working/reviews folders, or drop its overrides.
An empty field inherits the default. Settings → Git → Links offers **worktree folder** (`o`)
on each repository link; clearing it restores the global template from Git → Branches.
