# Slack: messages, discussions and review asks

Slack brings the conversations around your work into radar. You can catch up on mentions,
open the issue or change a thread discusses, and send that context to an agent without
copying messages between windows. Connect your account and choose watched channels using
[Connect your services](connect.md#slack), then press `S`.

For example, Sam Vimes posts a link to `ankh-morpork/watch!12` in your review channel and
mentions your group. Find it in **For you**, open its thread, and use **Links** to inspect
the merge request. From the message, **review locally** starts a review of that change;
[Connected context and local reviews](mcp-and-reviews.md#start-reviews-from-slack) explains
the worktree, the saved result and how to automate this step.

## Choose a feed

The left pane chooses the feed, the middle pane lists messages and threads, and the right
pane previews the selection. Replies fold under their parent message. Use `/` to filter
the loaded list by words in the channel, author or message; this is not a remote Slack
search. The [shared navigation keys](keys.md#trees-and-groups) apply to these trees.

| Feed | What it shows | Where to go next |
|---|---|---|
| **For you** | Mentions of you, mentions of your user groups, direct messages, and new replies in threads you are in. The search window is the last seven days. | Open a message to read its thread or reply. |
| **Discussions** | Threads from watched team and service channels, grouped by the linked work radar recognises. Threads without a recognised item go under **general**. Test and alerts channels are excluded. | Open a work-item heading to inspect the item, or a message to inspect the discussion. |
| **Alerts** | Firing alerts, failed jobs and recently resolved alerts, grouped by service and environment. This feed appears when a watched channel has kind **alerts**. | Open the alert's source thread or console link. |
| **#channel** | The watched channel's threads grouped by day, with recently active threads first. | Open a thread or expand its replies in place. |

Watched channels are the channels radar refreshes and lists in the left pane. A channel
can stay in the settings registry without being watched. **For you** can also find messages
outside the channel registry; those do not have the channel catch-up action.

## Read a thread

Press `⏎` on a message to open its thread page. Radar loads replies when its copy is
incomplete. The page has two tabs; `[` and `]` switch between them, and `esc` returns to the
previous list or page.

| Tab | What it contains | Actions |
|---|---|---|
| **Thread** | The opening message and replies, with authors, times, formatted text, shared messages and attachments. | Reply, react to the first message, open the conversation in Slack, or copy its link. |
| **Links** | Work items radar recognises in the thread and other links it contains. | Select a work item and press `⏎` to open it in radar. An ordinary link opens in your browser. `o` opens a recognised item's web page and `y` copies its URL. |

Loading or failed reads are shown on the page. Refresh the page to retry; a cached thread
can remain visible while a read fails. A recognised link depends on the relevant service
being configured in radar. Links to work on another service or account may remain ordinary
browser links.

Use `c` on a message or thread to write a reply. `⏎` adds a line, `ctrl-s` opens the send
confirmation, and the confirmation lets you send, return to editing or decline. `e` on a
message opens the reaction choices. On the Thread page, the reaction applies to the root
message; in a feed, it applies to the selected message. These actions change Slack and
follow radar's [write mode](README.md#the-mode).

Text retained after a declined or failed send is available through `ctrl-r` during the
current radar session. The retained-replies menu shows where each reply belongs and lets
you copy or drop it. If the connection changed, radar keeps the text associated with its
original workspace. If a send's outcome is uncertain, check the thread in Slack before
trying again. Retained reply text is not saved across radar restarts.

## Read marks and refreshing

Unread marks belong to radar. Opening a thread marks its loaded messages read here; `m`
on a feed message marks the channel read up to that message, and `m` on the thread page
marks through its newest held reply. Slack's own read state is unchanged.

The `b` bulk menu can mark the selected channel caught up, or every watched non-alerts
channel with new messages. It also opens or folds threads, copies or opens linked work,
and refreshes all watched channels and **For you**. On an alert row, `m` means **map this
alert**, as described below; use `x` to see the actions for the selected kind of row.

By default radar reads Slack every two minutes, starts with seven days of channel history,
and pauses reads after thirty minutes without input. Settings → Slack → Connection changes
those values. `r` refreshes, and `z` manually pauses or resumes refreshing. Automatic review
intake keeps reads running through ordinary idle time; manual pause still stops intake.

## Follow alerts

An alerts channel needs a reader selected in Settings → Slack → Channels. The readers
understand Alertmanager messages, failed-job messages, or bot messages with status words.
Messages the chosen reader cannot recognise are reported; a person's message is not turned
into an alert.

Firing and resolved messages for the same alert and environment are paired. Firing alerts
remain visible; failed jobs and resolved alerts stay for the configured recent window
(24 hours by default). An alert that has no matching mapping appears under **unmapped**.

On an alert, `⏎` opens its Slack thread and `o` opens its console link when supplied. For an
unmapped alert, `m` accepts a repository path such as `ankh-morpork/watch`, a repository URL,
or a service name such as `patrol`. That adds a local mapping rule; it does not acknowledge
or resolve the alert in the source system. Review the rule and retention window in
[Settings](settings.md).

## Draft a review ask

A review ask is a Slack message asking people to review a GitLab merge request. It is
separate from starting an agent's local review. From a merge request's `x` menu, choose
**ask for review in Slack…**. Radar opens the draft page, showing the items, destination
and rendered message. Asking from another merge request adds it to the same draft.

Use the draft's menu to edit the selected item's words, remove it, choose another channel,
copy the text or cancel the draft. `esc` goes back while keeping the draft. **Post** (`p`)
opens a final confirmation for the displayed destination and text. A blocked draft explains
which connection, channel or group setting needs attention. Nothing is sent just by opening
or editing the draft.

Settings → Slack → Review holds the normal channel, the user group to mention, a test
channel, the message template and when an unanswered request can be asked again. Sending
to the test channel leaves out the review-group mention.

The template puts the merge requests at `{mrs}`; **each request** sets how each one shows
there (`{link} {title}` by default). Both can name a merge request by `{link}`, `{url}`,
`{repo}`, `{project}`, `{iid}`, `{title}`, `{branch}`, `{target}`, `{author}` and
`{tickets}`. The template also takes `{count}`, `{group}` and `{@handle}`; with several
merge requests, a placeholder in the template joins their different values with commas.
For example, `{group} :gitlab: {repo} !{iid} by {author}: {link} {title}` names the repo,
number and author of one merge request. The settings page previews one and two merge
requests as you type. Once posted, the merge request
offers its review-ask thread and, when due, **ask again in the thread…**. A re-ask is an
explicit action, not an automatic reminder.

GitHub pull requests can show and open existing exact-link Slack discussions. The review-ask
draft action is currently for GitLab merge requests. Local agent reviews from Slack support
both GitLab and GitHub links.

## Give an agent the conversation

On a held message or open thread, `A` starts an agent with the message, author, thread context
and linked changes. If the message is not held yet, open its thread first. `v` starts a local
review of its linked MR/PR; a thread can supply several changes.

Linked Slack discussions also appear on relevant work-item pages, so the relationship works
in both directions. Integrated agent sessions receive the same connected context through
radar's MCP tools as an external agent client. Continue with [Agents](agents.md) for the dock,
then [Connected context and local reviews](mcp-and-reviews.md) for automatic intake and saved
reviews. The complete key list is in [Slack keys](keys.md#slack).

## Account changes and cache recovery

Slack data and local read marks are partitioned by workspace and user. A cached
preview may appear before sign-in finishes when the current token fingerprint
matches a previously authenticated account. It is marked cached; it does not
establish a fresh login or authorize writes. Changing accounts requires fresh
authentication and activates that account's separate data.

Older unverified or unsupported cache data is preserved without being adopted.
`radar slack cache-recovery` prints the quarantined JSON values without contacting
Slack or importing them. Its output can contain private messages and work context;
keep it private. An empty array means there is nothing quarantined. See
[Slack cache and recovery](../slack-cache.md) for a local export example, account
switching and retained draft behavior. Do not delete the cache to fix a failed
sign-in: it also holds local read marks, review state and other unrecoverable data.
