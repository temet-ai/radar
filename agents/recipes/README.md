# Recipes

A recipe says what kind of job an agent session is given, and the limits it runs under. radar sends a
recipe's instructions as the task of the opening prompt it builds (see `agents/briefing.md`), followed by the
information about the items sent. One file per recipe: `agents/recipes/<name>.md`.

## Format

A front-matter block of `key: value` lines between two `---` lines, then the instructions for the agent.
Every key is required, one line each.

```
---
name: ask
version: 1
inputs: what radar must supply (the operator's request, which attached items)
scope: where the agent may work and look
permissions: what it may do without asking, what it must ask for, what it may never do
expected-output: what it hands back, and how
stopping-point: when it is done and must stop
---
Instructions, plain Markdown, written to the agent.
```

- Two optional keys say how the session runs:
  - `agent:` the agent the session runs as: one name for every CLI (`agent: reviewer`), or
    `provider=name` pairs (`agent: copilot=reviewer, claude=code-reviewer`). Without it, the agent set
    for that CLI in Settings → Agents. A CLI that has no named agents (Codex) says so and runs its own.
  - `skills:` skills the session is asked to use, comma separated (`skills: security-review, notes`). The
    prompt names them in a line and the CLI makes sure each one is loaded; a skill the CLI
    does not have, or has switched off, is said in the dock, never switched on in the CLI's own config.
- Your own recipes live in the recipes folder (Settings → Agents → recipes folder; by default
  `~/.config/radar/recipes`). A file named like a built-in (`ask.md`, `review-mr.md`) replaces it, and a file
  that does not parse is not used (the built-in stands in, and Settings says why). In Settings, `e` copies the
  built-ins there and opens the folder in your `$VISUAL`/`$EDITOR`; ⏎ on a recipe sets its agent and skills.
- `name` is the file name without `.md`. `version` is a whole number; raise it whenever the instructions or
  any limit change, so a transcript or a saved review says which recipe version produced it.
- `scope`, `permissions`, `expected-output` and `stopping-point` are shown in Settings. What is actually
  allowed is enforced by the host through the adapter (each command approved in the dock, writes denied), so a
  recipe never loosens it. Anything the agent needs to know to do the job goes in the instructions, as part of
  the task.
- Instructions never contain a secret, a name of a person or an absolute path of the operator's machine.
  The information about the items follows the instructions in the prompt; the instructions refer to it ("the
  item below"), they do not repeat it.
- Write the instructions as a person would ask: the task, in plain words. No commentary about radar or about where
  the information came from, and no rules the task does not need.

## Recipes here

| Recipe | For |
|---|---|
| `ask` | any attached item and a question about it |
| `review-mr` | a local review of a merge request in a worktree at its head sha (added with the review journey) |
