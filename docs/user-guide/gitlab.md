# GitLab: merge requests, pipelines and job logs

GitLab in radar brings the repositories you follow, their merge requests and failing
jobs into one place. Open a change to read its description, review its diff and follow
the job that explains a failed pipeline.

First [connect GitLab](connect.md#gitlab) and follow at least one group or repository.
Press `R` and select the **GitLab** source of Repositories. The shared
[navigation keys](keys.md#everywhere) apply throughout this guide.

![GitLab's views, repository tree and selected item's detail](images/repositories.png)

The pictures use fictional demonstration data; their capture details are in the
[screenshot index](images/INDEX.md).

## Example: investigate a failed merge request

Suppose the fictional `ankh-morpork/watch!12` has a failed pipeline. Follow the failure
from the merge request to its job log:

1. Select its repository and merge request in the tree, then press `⏎`. You can also use
   `:` to find a merge request by its number or words.
2. Read **Overview** for the description and merge blockers. Use `]` to reach
   **Pipeline**, where failed jobs and an excerpt explaining the failure lead the list.
3. Select a failed job and press `⏎` for its **Log**. Search with `/`, move between
   matches with `n` and `N`, and use `c` to copy the excerpt around the first error.
4. Press `esc` to return to the merge request, then select **Changes** to inspect the
   relevant file. On a code line, `c` starts a comment; on an existing discussion it
   starts a reply. Review the text with `ctrl-s`, then confirm with `y` if you want to
   send it and your [mode](README.md#the-mode) permits comments.

![A merge request's Overview, including merge readiness](images/detail-merge-request.png)

## Repositories screen

The left pane holds saved views such as **Everything** and **Needs my action**. The
middle pane is a tree of followed groups, repositories and merge requests. The right
pane describes the selected row, including state, pipeline, approvals and available
actions. A followed group is called a **tenancy** in Settings.

Use `/` to narrow the current view and `a` to choose an author. Plain words match titles,
branches and repository paths; useful filters include `author:@me`, `is:review`,
`label:bug` and `repo:watch`. A leading `-` excludes a term. Views show open merge
requests by default. `state:merged`, `state:closed` or `state:all` include recent finished
changes; **merged, closed** in the connection settings sets the lookback, initially
14 days. `.` switches between all repositories in the view and those with matching
merge requests.

`space` folds a group or repository. `⏎` opens its page. `esc` returns to the page you
came from, or to the tree with your selection kept. Use `[` and `]` for page tabs and
`x` for the actions available on the current row.

## Every page and tab

| Page | How to open it | Tabs and what they contain |
|---|---|---|
| Tenancy | `⏎` on a followed group | **Repos** lists repositories, their default-branch pipeline, merge requests and last activity. It also reports hidden repositories. `⏎` on a repository opens its page. |
| Repository | `⏎` on a repository in the tree or tenancy page | **Merge requests** lists changes for that repository, with those needing attention first. **Pipelines on main** (or the repository's default-branch name) lists recent runs and leads with failed jobs when that branch is red. |
| Merge request | `⏎` on a merge request in the tree or repository page | **Overview**, **Changes**, **Threads**, **Pipeline**, **Commits**, plus linked tabs when other parts have relevant information. |
| Job | `⏎` on a job in a merge request's Pipeline tab, or a failed job on a repository's pipeline tab | **Log** shows the job output, with search and foldable sections. |

On a repository's pipeline tab, `⏎` on a **pipeline run** opens GitLab in the browser;
on a **failed job** it opens the log inside radar. A repository whose default branch
is red opens directly on its pipeline tab.

### Merge request tabs

| Tab | What you can do |
|---|---|
| **Overview** | Read the description, labels, latest activity and merge checks. `⏎` offers links found on the page, opening a linked radar item where available. |
| **Changes** | Select a file in the tree and read its diff. `v` requests side-by-side display when there is room; `f` hides or shows the file tree. `n` / `N` move between files and `}` / `{` between hunks. Comment on a code line with `c`. |
| **Threads** | Read open discussions first, then ordinary comments and resolved discussions. The first row lets `c` comment on the merge request; on a thread it replies. `d` resolves or reopens a resolvable thread. `⏎` on a discussion attached to a loaded diff jumps to its line in Changes. |
| **Pipeline** | Inspect failed jobs, error excerpts, then all jobs by stage. `⏎` opens a job's Log. `p` opens the pipeline in GitLab. |
| **Commits** | Read the commits supplied for this change. |
| **Ticket** or **Tickets** | Appears when Jira recognizes a linked ticket. Select a ticket and press `⏎` to open its [Jira page](jira.md#every-page-and-tab). |
| **Slack** | Appears when radar has related Slack discussion or a review ask. `⏎` follows the selected thread. |

A repository may also have an **Alerts** tab when Slack has alerts mapped to it.
Linked tabs depend on the other parts being connected and having relevant data; they
are not an additional fetch of everything from those services.

### Job Log

Large logs load the last part first. Reaching the top, or pressing `g`, asks for the
earlier part. The range shown in the log tells you when only part is loaded. `/` searches
the loaded text; `n` and `N` move between matching lines. `space` folds or opens the
section under the cursor. `c` copies lines around the first detected error, or the last
lines when there is no detected error. `esc` clears an active search before leaving
the page. Use `r` to read the log again and `o` to open the job in GitLab.

## Review and change actions

Comments and replies need **write** or **edit** mode. Approving, withdrawing approval,
resolving discussions, rebasing and merging need **edit** mode and a GitLab token that
allows the operation. Each write shows its confirmation before sending; a declined
comment can be recovered with `ctrl-r`. See [text and questions](keys.md#text-and-questions).

On a merge request page, `a` approves or withdraws your approval; on the tree row that
action is `e`. `u` asks to rebase onto the target branch, and `m` asks to merge. The
action menu explains known blockers, and GitLab remains responsible for accepting the
write. After a request, wait for the status to reflect GitLab's answer.

Other parts add local actions when they can identify the checkout or linked work.
Use the row's `x` menu to open an editor or terminal, check out a branch, add it to a
work item, or start an agent. The `c` key checks out a branch in contexts where it is
not the Changes/Threads comment action. For a local review, use `i` on the merge request
row and follow [MCP and automatic local reviews](mcp-and-reviews.md).

## Loading, empty results and failures

Radar keeps the last fetched data visible while refreshing. An error beside cached
data means the refresh failed; it does not mean those rows are current. `r` retries
the current page, or refreshes the repository list when you are back in the tree.
If the screen is not configured, use `s` for setup or `⏎` for its Settings section.

An empty view may be a filter or hidden repository: clear `/`, check the selected view
and check Settings, GitLab, tenancies. Pages distinguish **No description**, no threads,
no commits, no pipeline and an empty job log from a read that is still loading or
failed. An MR retained on a page may no longer be in the latest open list because it
was merged or closed; refresh before acting on it.

Next, follow a linked [Jira ticket](jira.md), or compare the read-only
[GitHub pages](github.md). [GitLab keys](keys.md#gitlab) is the compact action reference.
