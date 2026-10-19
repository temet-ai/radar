# Local workspaces and ticket notes

A local workspace is a top-level folder containing repositories and the notes you use
across them. Radar lets you open those folders, find a ticket's existing notes, and start
its work in several repositories from one place. This builds on the checkouts described
in [Local Git](git.md).

## Open a workspace or repository

Suppose the fictional Watch keeps its work under `~/work/ankh-morpork`, with clones in
`repositories/` and notes for `WATCH-12` in `notes/WATCH-12/`.

1. Open **Settings → Workspace → workspaces** and add the top folder. Set its clones
   folder to `repositories` and associate it with tracker project `WATCH`.
2. Press `W`, or choose **Local workspaces** beneath the **Repositories** screen.
3. Select the workspace to see its folder on the right. Expand it with `space` or `+`
   and select `watch` to see that repository's exact folder and project path.
4. Press `⏎` to open the selected workspace or repository folder in your editor.
   If no usable editor is configured, radar offers installed editors and remembers
   the choice for this workspace.

![Local workspaces with a repository folder selected](images/local-workspaces.png)

This [example capture](images/INDEX.md) uses a temporary demo workspace. The tree is the
workspace's local folder inventory; it does not require a service login.

The screen has a repository tree and a details pane. `⏎` launches the editor, rather
than opening another page inside radar. `r` in the tree reads repositories and notes
again for the configured workspaces. `-` folds a workspace, and `x` shows the available
actions. Use [Branches](git.md) for branch, dirty-file and worktree status.

### Choose the folders once

The workspace's Settings row holds its top folder, display name, tracker projects,
clones subfolder, notes location, editor and opening location. The default clones
subfolder is `repositories`; an empty clones setting means the top folder itself.
Radar uses the first workspace whose tracker-project list matches a ticket's project;
if none matches, it uses the first configured workspace. Put explicit project mappings
on workspaces when that fallback would be wrong.

For ticket actions, **open at: top** opens the workspace root. **open at: repo** opens
the ticket's existing notes folder when available, falling back to the root. The setting's
name does not mean “guess a repository from the ticket title.” On the Local workspaces
screen itself, `⏎` always opens the folder you selected.

## Read a ticket's notes

Continue with `WATCH-12`: open that ticket in Jira. When radar has found matching
notes, the ticket gains a **Notes** tab. Use `[` and `]` to reach it, select a note and
press `⏎` to open the file in the workspace's editor. A README's opening lines can also
appear in the ticket's Notes section. `esc` returns from the ticket page to the list.

Radar recognizes a note by its front matter or by a containing folder named for the
ticket, such as `WATCH-12` or `WATCH-12-patrol-routing`. A minimal note can begin:

```markdown
---
ticket: WATCH-12
title: Patrol routing
type: plan
---

Check the route before changing the watch repository.
```

With **notes: okf**, radar reads the workspace's `.okf.yaml` document/artifact include
and exclude paths and its artifact creation location. A notes path template can be
configured instead, for example `notes/{key}` or `artifacts/{date}/{key}-{slug}`.
Without an OKF definition, radar falls back to ticket-named folders under the notes
template's search root, or the workspace. Existing archived notes are included when
they lie in the indexed knowledge folders.

From the ticket's `x` menu, **look for notes again** refreshes the index. **Open the
workspace in an editor** and **open a terminal** use that workspace's opening location.
These actions are available through the menu when another part already owns their
usual shortcut. The Notes tab appears only when matching notes have been indexed.

## Start a ticket in several repositories

On the `WATCH-12` ticket, choose **start work on WATCH-12…** from `x` (or `s` where
offered). This prepares local notes and branches; it does not create or update a
remote merge request.

1. **Choose repositories.** Radar reads the workspace's clones and offers one row per
   repository. Existing branches, notes and exact linked repositories can preselect
   candidates. `⏎` on a repository toggles it. Review the choices, then select the
   first row, **start work in … repos**.
2. **Choose how to work.** Select **in the normal checkout** or **parked on a worktree**.
   The normal-checkout path sets aside local changes as described by the plan; a
   worktree gives the branch its own folder.
3. **Review the Git plan.** Radar shows the branch, base, destination and any stash,
   existing-branch or refusal information for each repository. Confirm to run it.
   Branch naming uses the Git settings and repository conventions; a template containing
   `{me}` requires your name to be configured under **Git → branches**.
4. **Read the result.** Radar records the repositories, working folders, branches,
   bases and stashes in the ticket README's **Branches** table. If an existing table
   has rows, radar asks before updating it. Other sections and rows for other
   repositories are preserved.
5. Radar refreshes the notes and opens the workspace at its configured location,
   using its editor or a terminal when no editor is set.

Choosing how to work can create a missing ticket README before the Git confirmation.
Cancelling the later branch plan does not remove that note. Existing ticket folders
are reused; new notes use the configured notes template or OKF creation path, with a
ticket folder beneath a shared date folder when needed. With no creation path set,
the default is `artifacts/{date}/{key}-{slug}`. A workspace can supply the initial
README text in `artifacts/.template.md`; radar fills the ticket fields and Branches table.

A refused or failed repository stops the remaining repository run. The result names
what succeeded and what was not started; it does not undo repositories already changed.
Inspect those results and [History](history.md) before deciding what to retry. A
configured after-worktree command is part of the worktree workflow, so read the plan's
command as well as the branch names.

These are local filesystem and Git actions. Radar's read-only mode does not prevent
them; use the actual choices and confirmations to decide what to create or change.

### Agent work and saved reviews

Workspace note routing is also one possible destination for local review files. Explicit
per-repository or global review folders take precedence. Agent working folders and Git
worktree destinations have separate settings; choosing a workspace's editor opening
location does not set all three. Use the single
[location reference](mcp-and-reviews.md#where-reviews-and-work-are-saved) to configure
their global defaults and repository overrides.

## When a workspace is missing or incomplete

| What you see | What to check |
|---|---|
| Add a local workspace in Settings | Add its existing top folder under **Workspace → workspaces** |
| Workspace has no repositories | Check the clones subfolder, then press `r`; adding a workspace does not clone anything |
| Duplicate folder identity | Remove duplicate workspace entries in Settings, or refresh an index with duplicate repository folders; radar will not choose an editor from an ambiguous row |
| No Notes tab on a ticket | Check the project-to-workspace mapping, note front matter/folder name and notes search paths; use **look for notes again** |
| Start work says no clones found | Correct the workspace's clones folder before starting again |
| A folder or editor cannot be opened | Check the exact path and editor in Settings; the launch result explains whether a command was copied instead |

Next, use [Work](work.md) to group the ticket, change and discussion as a local task.
