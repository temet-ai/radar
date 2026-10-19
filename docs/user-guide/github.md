# GitHub: pull requests, checks and Actions

GitHub in radar lets you browse repositories, read pull requests and trace their checks
to an Actions run. It brings the observed state into your workspace while keeping
GitHub operations read only: review submissions, merging, rerunning and cancelling jobs
are done on GitHub.

First [connect GitHub](connect.md#github) and choose the repositories to follow. Public
repositories can be read without a token; private repositories need a token or `gh`'s
sign-in. Press `R`, then select the **GitHub** source (`G`).

![GitHub's groups, repositories and pull requests](images/github.png)

This picture uses fictional demonstration data recorded in the
[screenshot index](images/INDEX.md).

## Example: follow a pull request's failing check

Suppose the fictional `ankh-morpork/watch#12` needs investigation. Follow its checks
to the run and job that reported them:

1. Select its group, repository and pull request, then press `⏎` for **Overview**.
2. Press `]` for **Checks**. Read the check runs and commit statuses for the pull
   request's head commit. Missing results mean the check state is unknown.
3. Move to **Actions**, select the relevant workflow run, and press `⏎` to open that
   run attempt. Read **Overview**, then **Jobs** for its jobs and steps.
4. Select the failing job and press `⏎` to open it on GitHub, where you can read its
   full log. `y` copies the selected Actions link if you want to keep or share it.
5. Use `esc` to return through the run and pull-request pages. After a new commit or
   a rerun, use `r` and check the displayed commit and attempt again.

## Repositories screen

The left pane shows **Everything** and the groups configured under Settings, GitHub,
repositories. The middle pane is a group → repository → open pull request tree.
The right pane shows the selected item's description and metadata. Groups can be an
owner's repositories or a collection you named yourself.

Select a group to narrow the tree. `space` folds groups and repositories; `⏎` opens a
repository or pull request. A group is a navigation choice, not a separate item page.
Use the shared [tree and page keys](keys.md#trees-and-groups), including `[` / `]` for
tabs and `esc` to return with your position kept. `x` shows the current actions.

## Every page and tab

| Page | Tab | What it shows and where it leads |
|---|---|---|
| Repository | **Overview** | Visibility, default branch, archived state, description and the latest observed default-branch Actions summary, once Actions have been loaded. `⏎` offers links found on the page. |
| Repository | **Pull requests** | The repository's open pull requests. Select one and press `⏎` for its page. |
| Repository | **Actions** | Recently observed workflow runs. Select a run and press `⏎` to open its particular attempt. |
| Pull request | **Overview** | Description, author, state, source and target branches, and head commit. Related information from other parts may appear as sections or actions. `⏎` offers links from the page. |
| Pull request | **Checks** | Check runs and commit statuses for the exact head commit, from the base repository and the source repository when accessible. |
| Pull request | **Actions** | Observed base-repository runs associated with the pull request or matching its head commit. Select a run to open it. |
| Run attempt | **Overview** | The workflow run's identity, attempt, state and available metadata. |
| Run attempt | **Jobs** | Jobs and their steps for that attempt. `⏎` on a job opens it on GitHub; radar does not have a separate GitHub log page. |

On a run page, `a` opens the attempt chooser. It offers the first, current, adjacent
and latest known attempts, so you can distinguish a failed attempt from its rerun.
`o` opens and `y` copies the selected Actions run or job; elsewhere they open or copy
the current repository or pull request.

## Reading checks correctly

**Checks** and **Actions** answer different questions. Checks describes the head commit.
An Actions run may instead test a generated merge commit or another event. Neither the
absence of check runs nor the absence of statuses means that checks passed. Radar does
not establish whether every required check was run or whether GitHub will allow a merge.

The repository Overview summarizes only workflows radar has observed on the default
branch. A green run there is not a statement that every workflow or branch is healthy.
Similarly, an empty pull request Actions result is not proof that there has been no
workflow activity, particularly when results are partial or source-repository access
is unavailable.

## Refresh, loading and access failures

Radar draws its saved data first, then checks for changes about once a minute while
refreshing is active. It refreshes an open pull request page while its checks are
running. Previously loaded repository Actions are checked when the repository changes
and at least every 15 minutes during normal syncing. `r` requests a refresh of the
current page or Actions view; from the tree it refreshes configured repositories.
Pausing or idle suspension stops automatic refreshes.

Loading labels describe reads still in flight. Failed reads keep the last available
data and show an error. A **partial** result means there may be more repositories,
pull requests, runs or jobs than radar could read; an empty partial result is not a
complete empty list. GitHub rate limits and retry delays are shown, and radar waits
before trying again.

If a group is empty, add or select its repositories in Settings. If private data is
unavailable, check the token, repository access and connection check described in
[Connect your services](connect.md#github). Changing radar to write or edit mode does
not add GitHub write actions.

Next, use [GitLab](gitlab.md) for its in-terminal diff and review workflow, or open a
linked [Jira ticket](jira.md). The shared [navigation keys](keys.md) cover returning to
the other screens.
