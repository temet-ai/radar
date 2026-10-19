# Local Git: checkouts, branches and worktrees

Branches shows the Git work already on your machine: which checkout is dirty, which
branch has commits to push, and which worktree you want radar to use. It reads the
checkouts configured under **Settings → Git → checkouts**. Service accounts are optional;
start with the [local checkout walkthrough](quickstart.md) if you have not added one.

## Inspect a checkout

For this example, suppose `~/work/watch` is a clone of the fictional
`ankh-morpork/watch` repository and its branch is `vimes/WATCH-12`.

1. Press `B`. Select **all** on the left, then move to the checkout tree with `tab`.
2. Select `watch`. Its right pane shows the folder, origin, repository, current branch,
   base branch and uncommitted changes. Existing worktrees and stashes appear below.
3. Expand the checkout with `space` or `+`, and select `vimes/WATCH-12`. The right pane
   now shows that branch's upstream, ahead/behind counts, commits on no remote, whether
   it is merged into the base, and where it is checked out.
4. Press `r` after changing something in your terminal to read the clones again.

![Branches with a checkout selected, branch rows and stash details](images/branches.png)

This is a [captured example](images/INDEX.md), with fictional repositories. Your paths,
branch names and counts come from your own checkouts.

### Views and selection

| Left-hand view | What it shows |
|---|---|
| all | Every known clone, its worktrees and local branches |
| dirty | Clones and worktrees with uncommitted changes; an unreadable status stays visible |
| ahead of remote | Branches with commits ahead of their recorded upstream |
| stashes | Clones with stashed changes; select a clone to read its stash list |

The middle columns show ahead, behind, dirty, stash count and latest commit age.
`gone` means the recorded upstream is gone. A dash or unavailable status is not a
claim that a checkout is clean. Radar refreshes these local reads while you use the
screen; `r` requests a fresh read. This is not a fetch of every remote.

The right pane changes with the selected clone, worktree or branch. There is no
separate tabbed branch page. `PgUp` and `PgDn` scroll the detail, and `x` shows the
actions available for the current row. `A` sends a selected checkout or branch to
an [agent](agents.md).

### Choose the working checkout

Select a clone or worktree and press `⏎` to make it the folder radar acts on for that
repository. The **in use** mark identifies the selected worktree. This changes radar's
local preference; it does not switch the Git branch. Pressing `⏎` on the main clone
restores the clone as the working folder. A branch row itself has no **use it** action.

If radar cannot match the checkout to a repository, check its origin or set an explicit
link in **Settings → Git → links**. On a repository's actions menu, **look for checkouts
again** refreshes that repository's candidates; **use another worktree…** chooses among
them.

## Work on a repository or change

Open a repository or merge request's `x` menu in [Repositories](README.md#3-the-parts).
The Git actions use its chosen local checkout. If none is known, the menu offers
**scan for a checkout** and **link to a local folder…**. Linking selects an existing
folder; it does not clone the repository.

For the `WATCH-12` example, open the merge request's menu, choose **open in an editor**,
and read the folder/branch choice if the checkout is on another branch. Select the
existing checkout or a new worktree as offered. Radar opens the resulting folder in
the configured editor; the status line reports the launch or a command copied for
you to run. Terminal and editor defaults are in **Settings → Git → terminal/editors**.

| Action or dialog | What happens |
|---|---|
| Check out the change's branch | Review the branch switch/update, then confirm; radar uses the existing clone |
| Open in an editor / open a terminal | Open the chosen folder; when a change's branch differs, choose how to open on that branch first |
| New branch | Radar derives the name from the ticket key and configured branch template; review the plan to track an existing remote branch or start a new branch from the fetched base |
| Dirty-checkout choice | Stash the changes, park the branch in a worktree, or leave the checkout alone |
| Existing-branch choice | Switch to the branch, recreate it from the fresh base, or cancel; the question names commits recreation would lose |
| Park on a worktree | Review the destination and branch, then create a separate working folder; an existing destination is refused |
| Update by rebasing | Fetch the base and rebase the current branch; commit or stash dirty files first |
| Push | Review the remote branch and commits; a rewritten branch uses `--force-with-lease` and the question describes remote commits it would replace |
| Link to another local folder | Choose the repository's local folder explicitly |

These branch-changing actions are offered on supported GitLab repository/change
subjects. A page with no ticket key cannot supply the name for **new branch** or
**park on a worktree**; start from the ticket or check the branch settings. GitHub's
scoped repository and PR folder actions open the current local folder without switching
its branch; use the exact wording in that source's menu.

If a rebase conflicts, radar leaves Git's rebase state available for you to resolve in
the checkout. `git rebase --abort` cancels that rebase. A failed operation is reported
on the status line; refresh the checkout to see its current state.

Branch naming and the base branch are configured under **Settings → Git → branches**,
with repository overrides under **links**. If a template uses `{me}`, set your branch
name there before starting work. Worktree destinations can be global or per repository;
[location precedence](mcp-and-reviews.md#where-reviews-and-work-are-saved) also explains
how review worktrees and ordinary agent working directories differ.

## Remove old local work

Select a branch or worktree and press `d`. The question describes exactly which local
thing will be removed. `y` confirms and `n` keeps it.

- **Branch:** radar refuses the base branch or a branch checked out in any worktree.
  The question names unpushed commits that deletion would lose. Deleting a local branch
  leaves its remote branch alone.
- **Worktree:** the question names the folder and uncommitted files that removal would
  lose. Its branch remains. Radar requires a readable current status; if the recorded
  worktree identity or status changes before confirmation, it asks you to review again.
- **Clone:** radar does not delete a whole clone from this screen.

Press `b` for **prune merged and gone**. This proposes eligible merged/gone branches
and clean worktrees together, with one confirmation and a total of any unpushed
commits at risk. Read that total: a clean worktree only means there are no uncommitted
files. Worktrees with unavailable status are skipped. The bulk menu also offers
**read every clone again**.

Local branch, checkout, rebase, worktree and delete dialogs remain available in
radar's read-only mode: that mode is not a filesystem lock. They require their own
choices or confirmations. **Push** obeys the write mode: creating a remote branch needs
write or edit; updating an existing remote branch needs edit. See
[the mode](README.md#the-mode).

## When nothing appears

| What you see | What to do |
|---|---|
| No clones known yet | Add the folder holding your checkouts in **Settings → Git → checkouts**, including an appropriate search depth |
| Looking for clones / reading | Wait for the local scan; cached rows may appear before fresh status |
| Empty dirty/ahead/stashes view | Select **all** to inspect the known clones |
| Status unavailable or a read error | Check that the folder and Git are accessible, then press `r`; do not treat cached counts as a fresh clean status |
| Origin names no repository | Repair the origin or add an explicit repository link |
| Editor or terminal cannot open here | Check the reported command and the Git terminal/editor settings; over SSH radar may hand the command to your clipboard |

Next, use [local workspaces](workspaces.md) to keep ticket notes and several repositories
together, or [Work](work.md) to connect their exact source items into a local task.
