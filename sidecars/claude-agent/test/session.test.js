import { test } from "node:test";
import assert from "node:assert/strict";
import { createServer } from "../src/server.js";
import { permissionKind, describeRequest } from "../src/map.js";

// A scripted stand-in for the SDK's query(): `script(ctx)` drives the message stream and may
// call ctx.canUseTool like Claude Code would.
function fakeSdk(script) {
  const calls = { options: null, interrupted: 0, closed: 0 };
  const sdk = {
    calls,
    query({ prompt, options }) {
      calls.options = options;
      const out = [];
      let wake = null;
      let ended = false;
      const push = (m) => {
        out.push(m);
        wake?.();
      };
      const ctx = {
        prompt,
        options,
        push,
        end: () => {
          ended = true;
          wake?.();
        },
        ask: (tool, input, o) => options.canUseTool(tool, input, { toolUseID: "tu1", signal: new AbortController().signal, ...o }),
      };
      script(ctx);
      return {
        async interrupt() {
          calls.interrupted++;
        },
        async setModel() {},
        async reinitialize() {},
        close() {
          calls.closed++;
          ctx.end();
        },
        supportedCommands: async () => ctx.commands ?? [],
        supportedModels: async () => [{ value: "sonnet", displayName: "Sonnet", description: "d" }],
        async *[Symbol.asyncIterator]() {
          for (;;) {
            if (out.length) yield out.shift();
            else if (ended) return;
            else await new Promise((r) => (wake = r));
          }
        },
      };
    },
    listSessions: async () => [],
    getSessionMessages: async () => [],
    deleteSession: async () => {},
  };
  return sdk;
}

const tick = () => new Promise((r) => setImmediate(r));

function harness(script) {
  const events = [];
  // The command lists are state, not events: kept apart so the event assertions read as history.
  const commands = [];
  const sdk = fakeSdk(script);
  const server = createServer({ sdk, emit: (e) => (e.type === "commands" ? commands : events).push(e), env: { PATH: "" } });
  return { sdk, events, commands, server };
}

const SID = "11111111-1111-4111-8111-111111111111";

test("every session is created with permissionMode default and no settings sources", async () => {
  const { sdk, server } = harness(() => {});
  const r = await server.handle({ id: 1, op: "create", session: "g1", cwd: "/tmp", session_id: SID, allow: ["Read"], deny: ["Bash(rm *)"] });
  assert.equal(r.ok, true);
  assert.equal(r.result.provider_session_id, SID);
  const o = sdk.calls.options;
  assert.equal(o.permissionMode, "default");
  assert.deepEqual(o.settingSources, []);
  assert.equal(o.includePartialMessages, true);
  assert.equal(o.sessionId, SID);
  assert.deepEqual(o.allowedTools, ["Read"]);
  assert.deepEqual(o.disallowedTools, ["Bash(rm *)"]);
  assert.equal(typeof o.canUseTool, "function");
  await server.closeAll();
});

test("a session reports the slash commands Claude Code accepts, with what they do and take", async () => {
  const { commands, server } = harness((ctx) => {
    ctx.commands = [
      { name: "compact", description: "Free up context", argumentHint: "[instructions]", builtin: true },
      { name: "context", description: "Show context usage", argumentHint: "" },
      { name: "clear", description: "Start over", argumentHint: "" },
      { name: "agents", description: "Sorted before the rest", argumentHint: "" },
      { name: "__remote-workflow", description: "internal", argumentHint: "" },
    ];
  });
  await server.handle({ id: 1, op: "create", session: "g1", cwd: "/tmp", session_id: SID });
  await tick();
  const line = commands[0];
  assert.equal(line.session, "g1");
  // Not history: no sequence number, so it does not take one from the session's events.
  assert.equal(line.seq, undefined);
  assert.deepEqual(line.commands, [
    { name: "agents", description: "Sorted before the rest", argument_hint: undefined },
    { name: "compact", description: "Free up context", argument_hint: "[instructions]" },
    { name: "context", description: "Show context usage", argument_hint: undefined },
  ]);
});

test("commands_changed replaces the list", async () => {
  const { commands, server } = harness((ctx) => {
    ctx.push({ type: "system", subtype: "commands_changed", uuid: "u1", session_id: SID, commands: [{ name: "deploy", description: "Ship it", argumentHint: "<env>" }] });
  });
  await server.handle({ id: 1, op: "create", session: "g1", cwd: "/tmp", session_id: SID });
  await tick();
  assert.deepEqual(commands.at(-1).commands, [{ name: "deploy", description: "Ship it", argument_hint: "<env>" }]);
});

test("resume passes resume, not sessionId", async () => {
  const { sdk, server } = harness(() => {});
  await server.handle({ id: 1, op: "create", session: "g1", cwd: "/tmp", resume: SID });
  assert.equal(sdk.calls.options.resume, SID);
  assert.equal(sdk.calls.options.sessionId, undefined);
  await server.closeAll();
});

test("streams ephemeral deltas then persisted messages, tool call and result, with sdk uuids", async () => {
  const { events, server } = harness((c) => {
    c.push({ type: "system", subtype: "init", uuid: "u0", session_id: SID, model: "m", cwd: "/tmp", permissionMode: "default", tools: [] });
    c.push({ type: "stream_event", uuid: "u1", session_id: SID, event: { type: "content_block_delta", index: 0, delta: { type: "text_delta", text: "Hel" } } });
    c.push({ type: "assistant", uuid: "u2", session_id: SID, parent_tool_use_id: null, message: { model: "m", content: [{ type: "text", text: "Hello" }, { type: "tool_use", id: "t1", name: "Bash", input: { command: "ls" } }] } });
    c.push({ type: "user", uuid: "u3", session_id: SID, parent_tool_use_id: null, message: { role: "user", content: [{ type: "tool_result", tool_use_id: "t1", content: "a\nb" }] } });
    c.push({ type: "result", subtype: "success", uuid: "u4", session_id: SID, is_error: false, result: "Hello", total_cost_usd: 0.01, usage: { input_tokens: 3, output_tokens: 4 } });
    c.end();
  });
  await server.handle({ id: 1, op: "create", session: "g1", cwd: "/tmp", session_id: SID });
  await tick();
  await tick();
  const kinds = events.map((e) => [e.kind, e.durability, e.provider_event_id]);
  assert.deepEqual(kinds, [
    ["session_started", "persisted", "u0"],
    ["text_delta", "ephemeral", "u1"],
    ["assistant_message", "persisted", "u2"],
    ["tool_call", "persisted", "tool:t1"],
    ["tool_result", "persisted", "result:t1"],
    ["turn_end", "persisted", "u4"],
    ["session_ended", "persisted", undefined],
  ]);
  assert.ok(events.every((e) => e.provider_session_id === SID && e.session === "g1"));
});

test("a permission request waits until answered; nothing is auto-approved", async () => {
  let answered;
  const { events, server } = harness((c) => {
    answered = c.ask("Bash", { command: "rm -rf x" }, { requestId: "r1" });
  });
  await server.handle({ id: 1, op: "create", session: "g1", cwd: "/tmp", session_id: SID });
  await tick();
  const req = events.find((e) => e.kind === "permission_request");
  assert.equal(req.payload.kind, "shell");
  assert.equal(req.payload.command, "rm -rf x");
  assert.equal(req.payload.request_id, "r1");
  assert.equal(req.provider_event_id, "perm:r1");
  const state = await Promise.race([answered, tick().then(() => "waiting")]);
  assert.equal(state, "waiting");

  const a = await server.handle({ id: 2, op: "answer", session: "g1", request_id: "r1", behavior: "deny", message: "no" });
  assert.deepEqual(a.result, { outcome: "answered" });
  assert.deepEqual(await answered, { behavior: "deny", message: "no" });
  assert.equal(events.at(-1).kind, "permission_resolved");
  await server.closeAll();
});

const ASK = {
  questions: [
    { question: "Which colour?", header: "Colour", options: [{ label: "red", description: "warm" }, { label: "blue", description: "cold" }], multiSelect: false },
  ],
};

test("AskUserQuestion is a question for the operator, not a permission, and waits", async () => {
  let answered;
  const { events, server } = harness((c) => {
    answered = c.ask("AskUserQuestion", ASK, { requestId: "q1" });
  });
  await server.handle({ id: 1, op: "create", session: "g1", cwd: "/tmp", session_id: SID });
  await tick();
  assert.equal(events.some((e) => e.kind === "permission_request"), false);
  const q = events.find((e) => e.kind === "question_request");
  assert.equal(q.provider_event_id, "ask:q1");
  assert.equal(q.payload.request_id, "q1");
  assert.deepEqual(q.payload.questions, [
    { question: "Which colour?", header: "Colour", options: [{ label: "red", description: "warm" }, { label: "blue", description: "cold" }], multi_select: false },
  ]);
  assert.equal(await Promise.race([answered, tick().then(() => "waiting")]), "waiting");
  // Approving it as a permission answers nothing for the operator.
  const wrong = await server.handle({ id: 2, op: "answer", session: "g1", request_id: "q1", behavior: "allow" });
  assert.equal(wrong.ok, false);

  const a = await server.handle({ id: 3, op: "answer_question", session: "g1", request_id: "q1", answers: { "Which colour?": "blue" } });
  assert.deepEqual(a.result, { outcome: "answered" });
  assert.deepEqual(await answered, { behavior: "allow", updatedInput: { ...ASK, answers: { "Which colour?": "blue" } } });
  const done = events.at(-1);
  assert.equal(done.kind, "question_resolved");
  assert.equal(done.provider_event_id, "ask-done:q1");
  assert.deepEqual(done.payload.answers, { "Which colour?": "blue" });
  const again = await server.handle({ id: 4, op: "answer_question", session: "g1", request_id: "q1", answers: { "Which colour?": "red" } });
  assert.deepEqual(again.result, { outcome: "duplicate" });
  await server.closeAll();
});

test("a declined question denies the tool with the reason; closing the session declines what waits", async () => {
  let first, second;
  const { events, server } = harness((c) => {
    first = c.ask("AskUserQuestion", ASK, { requestId: "q1" });
    second = c.ask("AskUserQuestion", ASK, { requestId: "q2", toolUseID: "tu2" });
  });
  await server.handle({ id: 1, op: "create", session: "g1", cwd: "/tmp", session_id: SID });
  await tick();
  await server.handle({ id: 2, op: "answer_question", session: "g1", request_id: "q1", answers: null, message: "not now" });
  assert.deepEqual(await first, { behavior: "deny", message: "not now" });
  assert.equal(events.find((e) => e.kind === "question_resolved").payload.answers, null);
  await server.closeAll();
  assert.equal((await second).behavior, "deny");
});

test("approve answers allow with the request's own input", async () => {
  let answered;
  const { server } = harness((c) => {
    answered = c.ask("Write", { file_path: "/tmp/a", content: "x" }, { requestId: "r2" });
  });
  await server.handle({ id: 1, op: "create", session: "g1", cwd: "/tmp", session_id: SID });
  await tick();
  await server.handle({ id: 2, op: "answer", session: "g1", request_id: "r2", behavior: "allow" });
  assert.deepEqual(await answered, { behavior: "allow", updatedInput: { file_path: "/tmp/a", content: "x" } });
  await server.closeAll();
});

test("approve for session allows with Claude Code's suggested rules kept for the session only", async () => {
  let answered;
  const suggestions = [
    { type: "addRules", rules: [{ toolName: "Bash", ruleContent: "mkdir:*" }], behavior: "allow", destination: "localSettings" },
    { type: "addDirectories", directories: ["/tmp/x"], destination: "projectSettings" },
    { type: "setMode", mode: "acceptEdits", destination: "session" },
    { type: "addRules", rules: [{ toolName: "Bash" }], behavior: "deny", destination: "userSettings" },
  ];
  const { server } = harness((c) => {
    answered = c.ask("Bash", { command: "mkdir a" }, { requestId: "r9", suggestions });
  });
  await server.handle({ id: 1, op: "create", session: "g1", cwd: "/tmp", session_id: SID });
  await tick();
  const r = await server.handle({ id: 2, op: "answer", session: "g1", request_id: "r9", behavior: "allow", scope: "session" });
  assert.equal(r.ok, true, r.error);
  assert.deepEqual(await answered, {
    behavior: "allow",
    updatedInput: { command: "mkdir a" },
    updatedPermissions: [
      { type: "addRules", rules: [{ toolName: "Bash", ruleContent: "mkdir:*" }], behavior: "allow", destination: "session" },
      { type: "addDirectories", directories: ["/tmp/x"], destination: "session" },
    ],
  });
  // Answering again is a duplicate, not a second approval.
  assert.equal((await server.handle({ id: 3, op: "answer", session: "g1", request_id: "r9", behavior: "allow", scope: "session" })).result.outcome, "duplicate");
  assert.equal((await server.handle({ id: 4, op: "answer", session: "g1", request_id: "r9", behavior: "allow", scope: "forever" })).ok, false);
  await server.closeAll();
});

test("approve for session with no suggestion allows the one request", async () => {
  let answered;
  const { server } = harness((c) => {
    answered = c.ask("Write", { file_path: "/tmp/a", content: "x" }, { requestId: "r10" });
  });
  await server.handle({ id: 1, op: "create", session: "g1", cwd: "/tmp", session_id: SID });
  await tick();
  await server.handle({ id: 2, op: "answer", session: "g1", request_id: "r10", behavior: "allow", scope: "session" });
  assert.deepEqual(await answered, { behavior: "allow", updatedInput: { file_path: "/tmp/a", content: "x" } });
  await server.closeAll();
});

test("ask rules: none over the operator's settings, every shell command without them, a review asks for commands, writes and the network", async () => {
  const ask = async (extra) => {
    const { sdk, server } = harness(() => {});
    await server.handle({ id: 1, op: "create", session: "g1", cwd: "/tmp", session_id: SID, ...extra });
    const s = sdk.calls.options.settings;
    await server.closeAll();
    return s?.permissions?.ask ?? null;
  };
  assert.equal(await ask({ setup: { project_config: true } }), null);
  assert.deepEqual(await ask({}), ["Bash"]);
  assert.deepEqual(await ask({ setup: { project_config: false } }), ["Bash"]);
  assert.deepEqual(await ask({ setup: { project_config: true, review: true } }), ["Bash", "Edit", "Write", "NotebookEdit", "WebFetch", "mcp__*"]);
  assert.deepEqual(await ask({ setup: { project_config: true }, ask: ["Edit"] }), ["Edit"]);
});

test("a request delivered twice is one request, re-emitted with the same id, answered once", async () => {
  let first, second;
  const { events, server } = harness((c) => {
    first = c.ask("Bash", { command: "ls" }, { requestId: "r3" });
    second = c.ask("Bash", { command: "ls" }, { requestId: "r3" });
  });
  await server.handle({ id: 1, op: "create", session: "g1", cwd: "/tmp", session_id: SID });
  await tick();
  const reqs = events.filter((e) => e.kind === "permission_request");
  assert.equal(reqs.length, 2);
  assert.equal(new Set(reqs.map((e) => e.provider_event_id)).size, 1);
  assert.equal((await server.handle({ id: 2, op: "answer", session: "g1", request_id: "r3", behavior: "allow" })).result.outcome, "answered");
  assert.equal((await server.handle({ id: 3, op: "answer", session: "g1", request_id: "r3", behavior: "allow" })).result.outcome, "duplicate");
  assert.equal((await first).behavior, "allow");
  assert.equal((await second).behavior, "allow");
  await server.closeAll();
});

test("answering an unknown request is an error, not an approval", async () => {
  const { server } = harness(() => {});
  await server.handle({ id: 1, op: "create", session: "g1", cwd: "/tmp", session_id: SID });
  const r = await server.handle({ id: 2, op: "answer", session: "g1", request_id: "nope", behavior: "allow" });
  assert.equal(r.ok, false);
  await server.closeAll();
});

test("pending re-emits unanswered requests for a reconnected radar", async () => {
  const { events, server } = harness((c) => {
    void c.ask("Bash", { command: "ls" }, { requestId: "r4" });
  });
  await server.handle({ id: 1, op: "create", session: "g1", cwd: "/tmp", session_id: SID });
  await tick();
  const before = events.length;
  const r = await server.handle({ id: 2, op: "pending", session: "g1" });
  assert.equal(r.result.count, 1);
  assert.equal(events.length, before + 1);
  await server.closeAll();
});

test("closing a session denies what is still pending and ends the query", async () => {
  let answered;
  const { sdk, server } = harness((c) => {
    answered = c.ask("Bash", { command: "ls" }, { requestId: "r5" });
  });
  await server.handle({ id: 1, op: "create", session: "g1", cwd: "/tmp", session_id: SID });
  await tick();
  await server.handle({ id: 2, op: "close", session: "g1" });
  assert.equal((await answered).behavior, "deny");
  assert.equal(sdk.calls.closed, 1);
});

test("interrupt and send reach the query; an unknown session is an error", async () => {
  const { sdk, server } = harness((c) => {
    c.prompt[Symbol.asyncIterator]().next().then((n) => c.push({ type: "assistant", uuid: "ua", session_id: SID, message: { content: [{ type: "text", text: n.value.message.content[0].text }] } }));
  });
  const { events } = { events: [] };
  void events;
  await server.handle({ id: 1, op: "create", session: "g1", cwd: "/tmp", session_id: SID });
  assert.equal((await server.handle({ id: 2, op: "send", session: "g1", text: "hi" })).ok, true);
  await server.handle({ id: 3, op: "interrupt", session: "g1" });
  assert.equal(sdk.calls.interrupted, 1);
  assert.equal((await server.handle({ id: 4, op: "send", session: "nope", text: "x" })).ok, false);
  await server.closeAll();
});

test("send emits the user message under the uuid the SDK stores it by; deltas carry their message id", async () => {
  const sent = [];
  const { events, server } = harness((c) => {
    (async () => {
      for await (const m of c.prompt) sent.push(m);
    })();
    c.push({ type: "stream_event", uuid: "s0", session_id: SID, event: { type: "message_start", message: { id: "msg_1" } } });
    c.push({ type: "stream_event", uuid: "s1", session_id: SID, event: { type: "content_block_delta", index: 0, delta: { type: "text_delta", text: "x" } } });
    c.push({ type: "assistant", uuid: "a1", session_id: SID, message: { id: "msg_1", content: [{ type: "text", text: "x" }] } });
  });
  await server.handle({ id: 1, op: "create", session: "g1", cwd: "/tmp", session_id: SID });
  await server.handle({ id: 2, op: "send", session: "g1", text: "hi" });
  await tick();
  const um = events.find((e) => e.kind === "user_message");
  assert.equal(um.payload.text, "hi");
  assert.equal(sent[0].uuid, um.provider_event_id);
  const d = events.find((e) => e.kind === "text_delta");
  const m = events.find((e) => e.kind === "assistant_message");
  assert.equal(d.payload.message_id, "msg_1");
  assert.equal(m.payload.message_id, "msg_1");
  await server.closeAll();
});

test("every line carries the session's own seq, a turn starts before the user message, history continues the count", async () => {
  const sdk = fakeSdk(() => {});
  sdk.getSessionMessages = async () => [
    { type: "assistant", uuid: "h1", session_id: SID, message: { id: "m", content: [{ type: "text", text: "old" }] } },
  ];
  const events = [];
  const server = createServer({ sdk, emit: (e) => e.type !== "commands" && events.push(e), env: { PATH: "" } });
  await server.handle({ id: 1, op: "create", session: "g1", cwd: "/tmp", session_id: SID });
  await server.handle({ id: 2, op: "send", session: "g1", text: "hi" });
  const h = await server.handle({ id: 3, op: "history", session: "g1", provider_session_id: SID, dir: "/tmp" });
  assert.deepEqual(events.map((e) => [e.kind, e.seq]), [["turn_started", 1], ["user_message", 2]]);
  assert.equal(events[0].durability, "ephemeral");
  assert.equal(h.result.events[0].seq, 3);
  await server.closeAll();
});

test("history replays what the operator typed as user messages, not tool results", async () => {
  const sdk = fakeSdk(() => {});
  sdk.getSessionMessages = async () => [
    { type: "user", uuid: "h1", session_id: SID, message: { role: "user", content: "list files" } },
    { type: "user", uuid: "h2", session_id: SID, message: { role: "user", content: [{ type: "text", text: "again" }] } },
  ];
  const server = createServer({ sdk, emit() {}, env: {} });
  const r = await server.handle({ id: 1, op: "history", session: "g1", provider_session_id: SID, dir: "/tmp" });
  assert.deepEqual(r.result.events.map((e) => [e.kind, e.provider_event_id, e.payload.text]), [
    ["user_message", "h1", "list files"],
    ["user_message", "h2", "again"],
  ]);
});

test("an SDK failure becomes an error event and the session ends", async () => {
  const sdk = fakeSdk(() => {});
  sdk.query = () => ({
    close() {},
    async *[Symbol.asyncIterator]() {
      throw new Error("boom");
    },
  });
  const events = [];
  const server = createServer({ sdk, emit: (e) => events.push(e), env: { PATH: "" } });
  await server.handle({ id: 1, op: "create", session: "g1", cwd: "/tmp", session_id: SID });
  await tick();
  assert.deepEqual(events.map((e) => e.kind), ["error", "session_ended"]);
  assert.equal(events[0].payload.message, "boom");
});

test("status: missing executable is not the same as signed out", async () => {
  const { server } = harness(() => {});
  assert.deepEqual((await server.handle({ id: 1, op: "status" })).result, { installed: false });

  const exec = (file, args, _o, cb) => {
    if (args[0] === "--version") cb(null, "2.1.296 (Claude Code)\n", "");
    else cb(null, JSON.stringify({ loggedIn: false }), "");
  };
  const s2 = createServer({ sdk: fakeSdk(() => {}), emit() {}, exec, env: {} });
  const r = (await s2.handle({ id: 1, op: "status", claude_path: "/x/claude" })).result;
  assert.equal(r.installed, true);
  assert.equal(r.version, "2.1.296");
  assert.equal(r.account.logged_in, false);
});

test("status forwards only the whitelisted account fields", async () => {
  const exec = (file, args, _o, cb) => {
    if (args[0] === "--version") cb(null, "2.1.296 (Claude Code)\n", "");
    else cb(null, JSON.stringify({ loggedIn: true, authMethod: "claude.ai", email: "a@b.c", subscriptionType: "max", orgId: "secret-org", token: "sk-ant-xyz" }), "");
  };
  const server = createServer({ sdk: fakeSdk(() => {}), emit() {}, exec, env: {} });
  const r = (await server.handle({ id: 1, op: "status", claude_path: "/x/claude" })).result;
  assert.deepEqual(r.account, { logged_in: true, auth_method: "claude.ai", email: "a@b.c", subscription_type: "max" });
  assert.ok(!JSON.stringify(r).includes("sk-ant"));
  assert.ok(!JSON.stringify(r).includes("secret-org"));
});

test("permission kinds and exact targets", () => {
  assert.equal(permissionKind("Bash"), "shell");
  assert.equal(permissionKind("Read"), "read");
  assert.equal(permissionKind("Edit"), "write");
  assert.equal(permissionKind("WebFetch"), "url");
  assert.equal(permissionKind("mcp__x__y"), "mcp");
  assert.equal(permissionKind("Task"), "other");
  const d = describeRequest("Edit", { file_path: "/a.rs", old_string: "a", new_string: "b" });
  assert.equal(d.path, "/a.rs");
  assert.equal(d.diff, "-a\n+b");
  assert.equal(describeRequest("WebFetch", { url: "https://x.y" }).url, "https://x.y");
});

test("history maps provider transcript to the same event ids as live", async () => {
  const sdk = fakeSdk(() => {});
  sdk.getSessionMessages = async () => [
    { type: "assistant", uuid: "u2", session_id: SID, message: { content: [{ type: "text", text: "Hello" }] }, parent_tool_use_id: null },
  ];
  const server = createServer({ sdk, emit() {}, env: {} });
  const r = await server.handle({ id: 1, op: "history", session: "g1", provider_session_id: SID, dir: "/tmp" });
  assert.deepEqual(r.result.events.map((e) => [e.kind, e.provider_event_id]), [["assistant_message", "u2"]]);
});
