# claude-agent sidecar

A small Node package that hosts the Claude Agent SDK (`@anthropic-ai/claude-agent-sdk`, pinned to the
Claude Code version it bundles) for radar. radar's `radar-sessions-claude` crate starts it, speaks to it over
stdio and restarts it if it exits. It drives the operator's own installed `claude` binary
(`pathToClaudeCodeExecutable`), not the SDK's bundled one.

```
cd sidecars/claude-agent && npm install --omit=optional && npm test
```

## Protocol (version 1)

One JSON object per line.

On start: `{"type":"ready","protocol":1,"sidecar_version":…,"sdk_version":…}`.

radar → sidecar: `{"id":n,"op":…,…}`. sidecar → radar: `{"type":"reply","id":n,"ok":true,"result":…}` or
`{"type":"reply","id":n,"ok":false,"error":"…"}`, and unprompted `{"type":"event",…}` lines.

| op | arguments | result |
|---|---|---|
| `hello` | | protocol, versions |
| `status` | `claude_path?`, `config_dir?` | `{installed, path, version, account:{logged_in, auth_method, email, subscription_type}}`, from `claude --version` and `claude auth status`; nothing else is forwarded |
| `models` | `claude_path?`, `config_dir?`, `cwd?` | `[{id, name, description}]` |
| `create` | `session` (radar id), `cwd`, `session_id` (new UUID) or `resume`, `model?`, `allow?`, `deny?`, `ask?` (default: see Permissions), `env?`, `claude_path?`, `config_dir?`, `setup?` (see Setup) | `{provider_session_id}` |
| `catalog` | `cwd`, `setup?`, `claude_path?`, `config_dir?` | `{agents:[{name, description, source}], skills:[…]}` from a query opened with the setup and closed before any prompt, so no session is stored |
| `send` | `session`, `text` | |
| `interrupt` | `session` | |
| `answer` | `session`, `request_id`, `behavior` (`allow`/`deny`), `message?`, `scope?` (`session`: approve for session) | `{outcome: "answered" \| "duplicate"}`; an unknown id is an error |
| `pending` | `session` | re-emits unanswered permission requests |
| `set_model`, `close` | `session`, … | |
| `list`, `history`, `delete` | `dir`, `provider_session_id`, `config_dir?`, … | provider-owned sessions and transcript |

Event lines: `{type:"event", session, provider_session_id, provider_event_id, durability:"persisted"|"ephemeral", kind, parent?, payload}`.
Kinds: `session_started`, `user_message`, `text_delta`, `reasoning_delta` (ephemeral), `assistant_message`,
`reasoning`, `tool_call`, `tool_result`, `permission_request`, `permission_resolved`, `turn_end`, `error`,
`session_ended`. `provider_event_id` is the SDK's uuid (or `tool:<id>`, `result:<id>`, `perm:<request id>`), so a
history replay carries the ids the live stream had.

Commands line: `{type:"commands", session, commands:[{name, description, argument_hint?}]}`, sent once a session
has started (from `Query.supportedCommands()`) and again on every `commands_changed` message. It is state, not
history: the whole list each time, no `seq`, no event id. Claude Code runs a prompt starting `/name args` as that
command (`/compact`, `/context`, a skill), also in streaming input, so radar offers exactly this list. Names starting
with `_` (internal) and `clear` (it starts a new conversation under radar's session) are left out.

## Setup

`setup` is radar-sessions' `SessionSetup`; `claude_path` and `config_dir` are the provider setup (`pathToClaudeCodeExecutable`,
and `CLAUDE_CONFIG_DIR` in the child's environment and, for `list`, `history` and `delete`, in this process). They are sent with
every request, so a change applies from the next query. Mapping (`src/setup.js`):

| setup | SDK |
|---|---|
| `project_config` | `settingSources` `["user","project","local"]`; off: `[]`. Never changes `permissionMode: "default"` |
| `extra_dirs` | `additionalDirectories` (`--add-dir`: also loads the folder's `.claude/skills`, `agents`, `commands`) |
| `instruction_dirs` | `additionalDirectories` and `CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD=1`. Loads CLAUDE.md there, not AGENTS.md, and only with the `project` source |
| `agent` | `agent` |
| `plugin_dirs` | `plugins: [{type:"local", path}]` |
| `skill_dirs` | no SDK option takes a skill folder: each `<dir>/<name>/SKILL.md` is copied into one generated plugin `radar-skills` (temp folder, removed when the session closes), so the skills are called `radar-skills:<name>` |
| `skills` | `skills: [...]` only when non-empty (a context filter that hides the rest); a name found in the skill folders is also given as `radar-skills:<name>` |
| `local_only` | nothing: Claude Code has no switch for it |

With `project_config` on, the operator's allow, deny and ask rules in those settings files apply, but not their `defaultMode`.

## Permissions

Every session starts with `permissionMode: "default"`, so the `defaultMode` in the operator's own settings (this
machine has `auto`) never answers for the dock. What else applies (`askRules` in `src/server.js`; an ask rule beats
every allow rule):

- **Project configuration on** (`settingSources` `["user","project","local"]`): radar adds no ask rule. Claude Code applies
  the operator's own `permissions.allow`, `deny` and `ask` from `~/.claude/settings.json`, the project's
  `.claude/settings.json` and `.claude/settings.local.json`, runs its built-in read-only commands (`ls`, `cat`,
  read-only `git`, ...) unasked, as it does in a terminal, and calls `canUseTool` (the dock) only for the rest.
  Claude Code's own trust rule holds: the allow rules in a project's `.claude/settings.json` apply in an SDK session
  only when the operator has trusted that folder in Claude Code; `~/.claude/settings.json` and an untracked
  `.claude/settings.local.json` apply always (checked live on 2026-10-14: a `Bash(touch:*)` allow in
  `settings.local.json` ran `touch` unasked, the same rule in an untrusted folder's `settings.json` asked).
- **Project configuration off** (`settingSources: []`): no settings file is read, so none of the operator's rules
  apply; `ask: ["Bash"]` makes every shell command ask, the read-only ones too.
- **A review session** (`setup.review`): `ask` on `Bash`, `Edit`, `Write`, `NotebookEdit`, `WebFetch` and `mcp__*`,
  whatever the operator's settings allow; radar's review policy decides them.
- `ask` in the request replaces all of this.

A request waits, with no timeout, until radar answers it. `answer` with `scope: "session"` approves it for the session:
the rules Claude Code suggested for it (`canUseTool`'s `suggestions`: allow rules and directories only, never a mode
change) go back as `updatedPermissions` with destination `session`, so nothing is written to a settings file. A
request delivered twice (`Query.reinitialize`, a reconnect) is the same request: it is answered once and a second
answer is a no-op. Closing a session denies what is still pending.

## Credentials

The sidecar never reads a credential file or prints a token. Anthropic does not allow third-party products to offer
claude.ai login or rate limits unless it approved them (Agent SDK overview), so radar offers no sign-in of its own:
sign in with `claude auth login` (a PTY tab in Settings → Agents); API-key authentication is the route for a product.

## Recording

`node scripts/record.mjs <scratch git repo> <out.ndjson>` runs one live session (ls approved, a write denied, a
turn interrupted, resumed in a second process). Scrub with `scripts/scan-secrets`
before committing; delete the session afterwards.
