# Get started with radar

This walkthrough gets radar running with a local Git checkout. It does not need a
GitLab, GitHub, Jira or Slack account. You can connect a service after you know
your way around the workspace.

## Install radar

On macOS or Linux with Homebrew, run:

```sh
brew install temet-ai/radar/radar
```

The full name matters because Homebrew has another app named `radar`. You can
also use [mise or a release archive](../../README.md#install).

## Start and set up local code

Run:

```sh
radar
```

On the first run, setup asks which parts to configure. Choose **Local code** and
select a folder containing a Git checkout. Finish the remaining setup steps;
services and agents can wait. Radar will still show the Branches and Work
screens. [First run](README.md#1-first-run) describes every setup step.

## Find a branch

Press `B` to open Branches. The left pane lists the folders radar searches; the
middle pane shows checkouts, worktrees and branches. Select a checkout to see
its status on the right.

![The Branches screen shows local checkouts and their branch status](images/branches.png)

Press `x` on a selected row to see its available actions. Press `esc` to close
the menu. The bottom line shows the keys that work at your current position;
press `q` when you are ready to leave radar.

## Next steps

- [Connect a service](connect.md) to bring its work into radar.
- [Learn the navigation rules](README.md#2-getting-around) for every screen.
- [See all keys](keys.md) when you need an exact shortcut.
- [Use an agent](agents.md) beside a selected item.
