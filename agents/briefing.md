# How radar briefs an agent

Version 2 (template v2; the code is `crates/radar-sessions/src/brief/`, keep the two in step and bump the
version here and `TEMPLATE_VERSION` there together).

When you send an item from radar to an agent (a Slack message, a merge request, a ticket, a branch, a file
and line), radar starts a session and sends it one opening prompt, the **brief**, at once. The transcript shows
exactly what was sent. The brief reads like a prompt a person would type.

## The shape

1. **The task**, in plain words: your own words for this send if there are any, then the recipe's
   instructions (`agents/recipes/<name>.md`). A recipe says what is to be done, and states a limit only when
   the task needs one (a review is not posted anywhere).
2. **The information to act on**, one headed section per item you sent: its kind and title, its link, then
   each field the owning part contributed (`Source branch: fix/retry`), long or multi-line text as a block.

Nothing else: no commentary about radar, about where the text came from or who wrote it, and no standing
rules. What the session may do is enforced by the host through the adapter (each command approved in the
dock, writes denied for a review), not argued in the prompt.

## Rules

- **Plain context.** The information is given as it is. A block of text sits in a code fence longer than any
  run of backticks inside it, so a description that contains a fence cannot break the layout. The fence has
  no label.
- **Bounded.** One field is cut at 8 KiB (marked "(cut short)"), a context at 32 KiB and 32 fields, a task at
  16 KiB, a brief at 96 KiB and eight contexts. An item left out is named. The builder applies the caps
  again; it does not take a part's word for them. No part sends a whole workspace: a part contributes the
  item you pointed at and what is needed to understand it, and nothing is attached that you did not choose.
- **No credentials.** Parts never contribute them. As a net, the builder replaces token-shaped words (Slack,
  GitHub, GitLab, cloud and API keys, bearer tokens, `password=` values) with `[redacted]` and drops private
  key blocks. Control and invisible characters (including bidirectional overrides) are removed.
- **Same text, same brief.** Building twice from the same inputs gives the same text (no clock, no random
  fence), so reviews and tests can compare.

## Recipes

A recipe is a versioned Markdown file in `agents/recipes/` (format in `agents/recipes/README.md`). Its
front matter (inputs, scope, permissions, expected output, stopping point) is shown in Settings and says what
the host enforces; its body is the task. Recipes: `ask` (any item, a question about it), `review-mr` (a local
review of a merge request).

## For a part that contributes context

Fill `radar_core::agent_context::AgentContext` for the subject you own: a kind word, the scoped `SubjectKey`,
a title, a link, and named fields (`ContextField::new`). Contribute what the agent needs for that item (an MR:
source and target branch, head sha, diff stats, description, open threads; a Slack message: text, author,
channel, thread link; a ticket: key, summary, status, description) and no more. Do not repeat the title as a
field. Never read another part's data, and never add secrets, tokens or whole files.

## Changing this

A change to the layout, a cap or the redaction list is a change to the template: bump the version in this
file and in `TEMPLATE_VERSION`, update the golden file
(`RADAR_WRITE_GOLDEN=1 cargo nextest run -p radar-sessions -E 'test(/brief::/)'`) and read its diff.

## Connected context for reviews

The review recipe also receives a task-context snapshot reference. The host assembles
linked owner-provided contexts and the brief keeps its existing size limits. MCP `context`
and `read` expose further sources and full cached sections with the same scoped identities.
This affects review task content, not template v2's layout. Automatic Slack intake uses the
same review journey; [setup and limits](../docs/user-guide/mcp-and-reviews.md) describe it.
