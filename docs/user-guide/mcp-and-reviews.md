# Connected context and local reviews

Radar gives agents connected context from your configured repositories, issues, conversations
and local checkouts. A review can start with a Slack request, follow its linked change and
requirements, and finish as a local Markdown file tied to the reviewed commit. The
[agent dock](agents.md) and external MCP clients use the same context model.

This guide continues the [Slack review example](slack.md): a request about
`ankh-morpork/watch!12` can lead an agent to its linked `WATCH-12` issue and relevant
discussions. The available relationships depend on the links and cached data radar actually
holds; an inferred association is labelled as such.

## Connect an external session

Start radar with your normal service and workspace configuration, and leave it running.
In your MCP client's server settings, add a **stdio** server with executable `radar` and
argument `mcp`. The executable must be available on that client's PATH; otherwise use its
absolute path. For clients that use an `mcpServers` JSON object, the entry is:

```json
{
  "mcpServers": {
    "radar": {
      "command": "radar",
      "args": ["mcp"]
    }
  }
}
```

The client launches the relay; you do not need a second interactive radar instance for it.
Radar discovers the running host automatically. If several hosts are open, use arguments
`["mcp", "--workspace", "/absolute/path/where/radar/was/started"]`. This selects the host by
its launch directory, not by the repository currently selected on screen. If more than one
host has that launch directory, close the duplicate before reconnecting.

The relay shares the host's configured account scope and caches. It does not need service
tokens in the agent's configuration. It is a local connection, not a remote HTTP endpoint
or an independent headless service. Integrated dock providers receive the connection
automatically on start or resume. Arbitrary terminal-command tabs use their own MCP setup.

If the client reports **no running radar context server**, start radar in the selected
directory and reconnect. A **several radar hosts match** error means you need the workspace
argument or must close a duplicate host. Closing radar ends this connection; the client
can reconnect after you start it again.

The wire format follows the MCP [stdio transport](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports)
and [tool result contract](https://modelcontextprotocol.io/specification/2025-11-25/server/tools).

## Find the work, then read the evidence

| Tool | Use |
|---|---|
| `search` | Find cached work by text or exact URL/ref. Filter by `kind`, `project`, or `assignee`; Jira supports `assignee: "self"` using the authenticated account ID. |
| `context` | Start from `roots` (URLs or scoped refs), or a search `selection_ref`. Follow cached relationships up to `depth` (default 2, maximum 4). |
| `read` | Read named `sections` for 1–32 `refs`. Supply `snapshot_id` to read the captured source rather than a newer cache value. |
| `changes` | Compare a `snapshot_id` with current connected cached work. It returns changed and unavailable refs. |

For the example, call `search` with `{"query":"WATCH-12"}`. Pass the returned
`selection_ref` to `context`, inspect the available item refs and section names, then pass
the relevant `refs` and returned `snapshot_id` to `read`. Supplying `sections` narrows the
read to the named sections; omitting it reads all available sections for those refs. Use
`changes` with that snapshot later to see whether the cached connected work changed.

Follow `next_cursor` by calling the same tool with `cursor`. `max_bytes` controls source
payload size per page, from 1 KiB to 256 KiB (24 KiB by default); metadata adds overhead.
There is no cumulative read quota or approval after a fixed volume.

Context includes scoped identities, source observation times where known, content revisions,
relationship evidence, available sections and missing information. A ticket key in a branch
is an inference; an observed link is evidence of a mention. Neither proves completion or
production deployment. Source prose is evidence, not instructions for the agent.

### Freshness and limits

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

A manual review starts with `v` on a Slack message or thread that links an MR/PR. For
automatic intake, first connect Slack, GitLab or GitHub, configure the local checkout, and
choose a signed-in integrated provider in [Agents](agents.md#set-up-a-provider). Choose a
watched channel as the review channel in Settings → Slack → Review, then set this option
in the existing configuration:

```toml
[slack.review]
channel = "watch-reviews" # the alias of an existing watched channel
# group = "S0123456789"  # optional Slack user-group ID
auto_review = true
```

The channel value is radar's configured channel alias, not the Slack channel ID or URL.
Automatic intake is off by default. The [Slack guide](slack.md#ask-for-review) covers
composing a request yourself; this option handles incoming requests.

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

### Read the result

The agent returns a review with a verdict, findings with file and line references, and what
it could not check. Radar saves that response as Markdown in the resolved reviews directory,
shows its location, and adds **open the local review** and **copy the local review's path**
actions to the MR/PR. The note records the service-scoped item, source and target branches, reviewed commit,
provider, model, recipe version and review time. It is marked `status: local` and
`posted: false`.

For a review of `ankh-morpork/watch!12` at commit `abc12345…`, the filename is
`review-ankh-morpork-watch-12-abc12345.md`. The row's review indicator shows when the remote
head has moved, so a saved review is not mistaken for a review of newer code. Existing files
are preserved; a changed result for the same revision gets a numbered filename.

Nothing is posted to Slack, GitLab or GitHub. The review policy permits reading the worktree,
approved inspection commands and radar's context tools. It denies file-write requests,
other MCP tools and direct network requests. Tests, builds and other commands require
individual approval in the dock. Set `reviewer_may_run_commands = false` in `[sessions]`
for reviews that should finish by inspection instead of waiting for those command approvals.

### When a review does not start or finish

| What you see | What to check |
|---|---|
| No automatic review | The review channel must be watched, `auto_review` enabled, and the request newer than first activation. Check the group mention and linked MR/PR, and resume radar if manually paused. |
| Missing checkout or commit | Configure the correct local repository and make the required commit available there. Radar does not clone a repository for the review. |
| Provider unavailable or signed out | Check the selected provider in Settings → Agents. |
| A session waiting for approval | Open its dock tab and inspect the pending command, or use inspection-only reviews for subsequent launches. |
| Review failed or was interrupted | Use the thread's manual **review locally** action to retry. Automatic failures are not retried repeatedly. |
| Context says data is missing or truncated | Open or refresh the relevant service item and retry the context read. A snapshot preserves the old read; start a new query for refreshed data. |

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

Changing these locations does not move existing review files or a running session's work.
Continue with [Settings](settings.md) for the other configuration pages, or
[Privacy and local data](privacy.md) for what radar and the providers retain.
