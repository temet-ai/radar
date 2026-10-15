// Pure mapping from Agent SDK messages and permission callbacks to the sidecar's
// event lines. No I/O here, so it is unit tested without a Claude Code process.
//
// Event line: { type: "event", session, provider_session_id, provider_event_id,
//               durability: "persisted" | "ephemeral", kind, parent?, payload }
// `provider_event_id` is the SDK's own uuid (or one derived from it plus the block's id),
// never a counter: radar dedupes on it.

const READ_TOOLS = new Set(["Read", "Glob", "Grep", "LS", "NotebookRead"]);
const WRITE_TOOLS = new Set(["Write", "Edit", "MultiEdit", "NotebookEdit"]);
const SHELL_TOOLS = new Set(["Bash", "BashOutput", "KillShell"]);
const URL_TOOLS = new Set(["WebFetch", "WebSearch"]);

export function permissionKind(toolName) {
  if (SHELL_TOOLS.has(toolName)) return "shell";
  if (READ_TOOLS.has(toolName)) return "read";
  if (WRITE_TOOLS.has(toolName)) return "write";
  if (URL_TOOLS.has(toolName)) return "url";
  if (toolName.startsWith("mcp__")) return "mcp";
  return "other";
}

// The exact target of a request, in the fields radar's PermissionRequest carries.
export function describeRequest(toolName, input, blockedPath) {
  const kind = permissionKind(toolName);
  const i = input ?? {};
  const out = { kind, tool_name: toolName };
  if (kind === "shell") out.command = String(i.command ?? "");
  if (kind === "read") out.path = String(i.file_path ?? i.path ?? i.pattern ?? blockedPath ?? "");
  if (kind === "write") {
    out.path = String(i.file_path ?? i.notebook_path ?? blockedPath ?? "");
    out.diff = writeDiff(toolName, i);
  }
  if (kind === "url") out.url = String(i.url ?? i.query ?? "");
  if (kind === "mcp" || kind === "other") out.input = i;
  return out;
}

function writeDiff(toolName, i) {
  const lines = [];
  const edit = (oldS, newS) => {
    for (const l of String(oldS ?? "").split("\n")) lines.push(`-${l}`);
    for (const l of String(newS ?? "").split("\n")) lines.push(`+${l}`);
  };
  if (toolName === "Edit") edit(i.old_string, i.new_string);
  else if (toolName === "MultiEdit") for (const e of i.edits ?? []) edit(e.old_string, e.new_string);
  else if (toolName === "Write") for (const l of String(i.content ?? "").split("\n")) lines.push(`+${l}`);
  else return undefined;
  return lines.join("\n");
}

function base(ctx, uuid, durability, kind, payload, extra = {}) {
  return {
    type: "event",
    session: ctx.session,
    provider_session_id: ctx.providerSessionId,
    provider_event_id: uuid,
    durability,
    kind,
    ...extra,
    payload,
  };
}

// Commands that are Claude Code's own plumbing or would leave radar's transcript behind:
// `clear` starts a new conversation under the same process, which radar's session would not follow.
const HIDDEN_COMMANDS = new Set(["clear"]);

// The slash commands a session accepts as a prompt (`/compact`, `/context`, a skill), from
// `Query.supportedCommands()` or a `commands_changed` message. Not an event: the list is
// current state, replaced whole each time, never history. Names starting `_` are internal.
export function commandsLine(ctx, list) {
  const commands = (list ?? [])
    .filter((c) => c?.name && !c.name.startsWith("_") && !HIDDEN_COMMANDS.has(c.name))
    .map((c) => ({
      name: c.name,
      description: c.description ?? "",
      argument_hint: c.argumentHint || undefined,
    }))
    // Claude Code lists its skills before its own commands; a menu of fifty is found by letter.
    .sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
  return { type: "commands", session: ctx.session, commands };
}

// One SDK message -> zero or more event lines.
export function mapMessage(ctx, msg) {
  const parent = msg.parent_tool_use_id ?? undefined;
  const extra = parent ? { parent } : {};
  switch (msg.type) {
    case "system": {
      // Skills found while the agent works, a plugin reloaded: the full list again.
      if (msg.subtype === "commands_changed") return [commandsLine(ctx, msg.commands)];
      if (msg.subtype === "init") {
        return [
          base(ctx, msg.uuid, "persisted", "session_started", {
            model: msg.model,
            cwd: msg.cwd,
            permission_mode: msg.permissionMode,
            tools: msg.tools,
            claude_code_version: msg.claude_code_version,
          }),
        ];
      }
      return [];
    }
    case "stream_event": {
      const ev = msg.event ?? {};
      if (ev.type === "message_start") ctx.messageId = ev.message?.id;
      if (ev.type === "content_block_delta") {
        const d = ev.delta ?? {};
        const message_id = ctx.messageId;
        if (d.type === "text_delta") {
          return [base(ctx, msg.uuid, "ephemeral", "text_delta", { message_id, index: ev.index, text: d.text }, extra)];
        }
        if (d.type === "thinking_delta") {
          return [base(ctx, msg.uuid, "ephemeral", "reasoning_delta", { message_id, index: ev.index, text: d.thinking }, extra)];
        }
      }
      return [];
    }
    case "assistant": {
      const blocks = msg.message?.content ?? [];
      const out = [];
      const text = blocks.filter((b) => b.type === "text").map((b) => b.text).join("");
      const reasoning = blocks.filter((b) => b.type === "thinking").map((b) => b.thinking).join("");
      const message_id = msg.message?.id;
      if (reasoning) out.push(base(ctx, `${msg.uuid}:reasoning`, "persisted", "reasoning", { message_id, text: reasoning }, extra));
      if (text) {
        out.push(base(ctx, msg.uuid, "persisted", "assistant_message", { message_id, text, model: msg.message?.model }, extra));
      }
      for (const b of blocks) {
        if (b.type === "tool_use") {
          out.push(
            base(ctx, `tool:${b.id}`, "persisted", "tool_call", {
              tool_use_id: b.id,
              name: b.name,
              input: b.input,
            }, extra),
          );
        }
      }
      return out;
    }
    case "user": {
      const content = msg.message?.content;
      // What the operator typed (replayed from history; live turns emit it from send()).
      if (typeof content === "string") return isMarker(content) ? [] : [userMessage(ctx, msg.uuid, content)];
      if (!Array.isArray(content)) return [];
      const out = [];
      const typed = content.filter((b) => b.type === "text").map((b) => b.text).join("");
      if (typed && !isMarker(typed) && !content.some((b) => b.type === "tool_result") && !msg.isSynthetic) {
        out.push(userMessage(ctx, msg.uuid, typed));
      }
      for (const b of content) {
        if (b.type === "tool_result") {
          out.push(
            base(ctx, `result:${b.tool_use_id}`, "persisted", "tool_result", {
              tool_use_id: b.tool_use_id,
              is_error: Boolean(b.is_error),
              content: flattenResult(b.content),
            }, extra),
          );
        }
      }
      return out;
    }
    case "result": {
      return [
        base(ctx, msg.uuid, "persisted", "turn_end", {
          subtype: msg.subtype,
          is_error: Boolean(msg.is_error),
          result: msg.result,
          errors: msg.errors,
          num_turns: msg.num_turns,
          total_cost_usd: msg.total_cost_usd,
          usage: msg.usage
            ? { input_tokens: msg.usage.input_tokens, output_tokens: msg.usage.output_tokens }
            : undefined,
        }),
      ];
    }
    default:
      return [];
  }
}

// Claude Code stores its own interrupt marker as a user turn; it is not typed text.
const isMarker = (text) => text.startsWith("[Request interrupted");

export function userMessage(ctx, uuid, text) {
  return base(ctx, uuid, "persisted", "user_message", { text });
}

function flattenResult(content) {
  if (typeof content === "string") return content;
  if (Array.isArray(content)) {
    return content.map((c) => (c.type === "text" ? c.text : `[${c.type}]`)).join("\n");
  }
  return "";
}

export function permissionEvent(ctx, requestId, toolName, input, opts = {}) {
  return base(ctx, `perm:${requestId}`, "persisted", "permission_request", {
    request_id: requestId,
    tool_use_id: opts.toolUseID,
    reason: opts.decisionReason,
    ...describeRequest(toolName, input, opts.blockedPath),
  });
}

// AskUserQuestion's input (questions[] of { question, header, options[{ label, description }],
// multiSelect }) as a question request. Each question is keyed by its text: the answers go
// back keyed by it. "Other" is never among the options; typed words always answer.
export function questionEvent(ctx, requestId, input, opts = {}) {
  const questions = (input?.questions ?? []).map((q) => ({
    question: String(q.question ?? ""),
    header: q.header ? String(q.header) : undefined,
    options: (q.options ?? []).map((o) => ({ label: String(o.label ?? ""), description: o.description ? String(o.description) : undefined })),
    multi_select: Boolean(q.multiSelect),
  }));
  return base(ctx, `ask:${requestId}`, "persisted", "question_request", {
    request_id: requestId,
    tool_use_id: opts.toolUseID,
    questions,
  });
}

// `answers`: question text -> answer, or null when it was declined.
export function questionResolvedEvent(ctx, requestId, answers) {
  return base(ctx, `ask-done:${requestId}`, "persisted", "question_resolved", {
    request_id: requestId,
    answers,
  });
}

export function permissionResolvedEvent(ctx, requestId, behavior) {
  return base(ctx, `perm-done:${requestId}`, "persisted", "permission_resolved", {
    request_id: requestId,
    behavior,
  });
}
