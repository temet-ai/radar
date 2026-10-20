---
name: radar-agent-recording
description: Run a real agent CLI (GitHub Copilot through github-copilot-sdk, `codex app-server`, Claude Code through the Agent SDK sidecar) from radar once, safely, to record a provider stream for radar-sessions adapter tests, and check the recording holds no token, email, login or home path before it is committed. Use whenever a radar task says record live, recorded stream, record once, exercise against the real CLI, confirm the protocol live, or a test fixture is captured from copilot, codex or claude; and before committing any file that came from a live provider run.
---

# Recording a live agent CLI for radar

A recording is made once, with a tiny prompt, in a scratch repository, then replayed by
the adapter's tests. Three things go wrong: the operator's own settings answer every
permission so the recording has none, the provider keeps the session, and the
recording carries his email, login or home path into the repository.

## 1. Scratch repository

```bash
S=<your scratchpad>/live-<cli>        # never inside the radar checkout
mkdir -p "$S/repo" && git -C "$S/repo" init -q
printf 'hello\n' > "$S/repo/a.txt" && git -C "$S/repo" add . && git -C "$S/repo" -c user.name=t -c user.email=t@example.com commit -qm init
```

The session's working directory is `$S/repo`.

## 2. Ask for permission prompts explicitly

This machine's settings approve everything: `~/.codex/config.toml` has
`approval_policy = "never"` and `sandbox_mode = "danger-full-access"`, and
`~/.claude/settings.json` has `defaultMode: auto`. Override them per session:

| CLI | Set on the session |
|---|---|
| Codex | Start `codex app-server --disable hooks` (the operator's `~/.codex/hooks.json` runs a SessionStart hook). `thread/start` with `cwd`, `approvalPolicy: "untrusted"`, `sandbox: "read-only"`, `approvalsReviewer: "user"`; check the response echoes them. `"on-request"` let `ls` run unasked. |
| Claude Code | `query()` options `cwd`, `permissionMode: "default"`, `settingSources: []`, `canUseTool`, `includePartialMessages: true`, `pathToClaudeCodeExecutable: <command -v claude>`, `env: {PATH, HOME, SHELL: "/bin/bash"}` (the operator's zsh aliases `ls` to eza, which prints nothing without a tty, and the inherited `CLAUDE_CODE_PLUGIN_DIRS` loads his own plugins into the recording). Read-only commands inside `cwd` (`ls`) are allowed before `canUseTool` is asked: gate with the sidecar's own PreToolUse hook, or set `settings: {permissions: {ask: ["Bash"]}}` |
| Copilot | `SessionConfig::with_permission_handler(<radar's handler>)`, `working_directory`, `streaming: Some(true)`; `COPILOT_AUTO_UPDATE=false` in the client env. Never `ApproveAllHandler` |

Codex answers: `{"decision":"accept"}` approves once, `"decline"` denies and the turn
goes on (accepted although the request's `availableDecisions` omits it), `"cancel"`
denies and interrupts. `~/.codex/rules/default.rules` allows some commands under any
policy; do not record with one of those.

## 3. The prompts

Two turns cover the contract: one that runs a read command (`list the files with ls`),
approved once, and one that writes (`create b.txt containing hi`), denied. Then
interrupt a third turn mid-tool, disconnect, resume, and read the history back.

The recording is not done until it holds at least one permission request, one approve,
one deny and the provider's answer to the deny:

```bash
rg -c 'requestApproval|permission' <recording>   # 0 means the policy in step 2 did not apply
```

## 4. Delete the provider's session

| CLI | How |
|---|---|
| Copilot | `client.delete_session(&id)` |
| Codex | `thread/delete` with the thread id (an `ephemeral: true` thread leaves nothing, but cannot be resumed) |
| Claude Code | remove `~/.claude/projects/<cwd with every / and . as ->/` for the scratch cwd only (`persistSession: false` leaves nothing, but cannot be resumed) |

## 5. Scan before committing

```bash
scripts/scan-secrets <recording files or dir> [--also <literal>]
```

It exits 1 and prints `file:line: kind` (never the value) for token-shaped values,
emails other than example.com, `$HOME`, the git user's email and name, the hostname
(Codex sends it as `serverName`) and the Copilot login. Replace each with a placeholder
(`user@example.com`, `/home/user`, `host`, `<login>`), scan again, then commit. It
cannot know these, so replace them by hand: Codex `chatgptAccountId` and
`installationId` (placeholder UUIDs), and the scratch path. Streamed deltas can split a
path across lines: replace it before splitting, or in the joined text too. A test that writes config or state (Settings → Agents) can run
the same script over its output folder.

## Status without credentials

Ask each CLI, never its files: Copilot `client.get_auth_status()`, Codex `account/read`,
Claude Code `claude auth status`. Never open `~/.codex/auth.json`, `~/.copilot/` token
stores or Claude's credentials, and never put their output in a note unscanned.
