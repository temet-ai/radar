import { randomUUID } from "node:crypto";
import { AsyncQueue } from "./queue.js";
import { commandsLine, mapMessage, permissionEvent, permissionResolvedEvent, questionEvent, questionResolvedEvent, userMessage } from "./map.js";

// The rules Claude Code suggested for a request (canUseTool's `suggestions`), as an approval
// for this session: only rules that allow and directories to add, each kept with destination
// "session", so nothing is written to the operator's settings files and no mode changes.
export function sessionRules(suggestions) {
  return (suggestions ?? [])
    .filter((u) => (u.type === "addRules" && u.behavior === "allow") || u.type === "addDirectories")
    .map((u) => ({ ...u, destination: "session" }));
}

// One radar session = one long-lived Claude Code query fed by streaming input.
//
// Permissions: every session is created with permissionMode "default" (it wins over the
// defaultMode in the operator's settings files), and settings sources only when radar's setup
// turns project configuration on. Then Claude Code applies the operator's own allow, deny
// and ask rules and calls canUseTool only for what they leave (server.js askRules says which
// ask rules radar adds). A request waits, with no timeout, until `answer()` is called. A request
// delivered twice (Query.reinitialize, a reconnect) is answered idempotently by its id.
export class Session {
  #queryFn;
  #emit;
  #emitRaw;
  #input = new AsyncQueue();
  #query = null;
  #pending = new Map(); // requestId -> { promise, resolve, event }
  #answered = new Map(); // requestId -> behavior
  #ctx;
  #opts;
  #done = null;
  #seq = 0;

  // Numbers history events in this session's sequence, after what it has already emitted.
  stamp(events) {
    return events.map((ev) => ({ ...ev, seq: ++this.#seq }));
  }

  #cleanup;

  constructor({ queryFn, emit, session, options, cleanup }) {
    this.#cleanup = cleanup;
    this.#queryFn = queryFn;
    // Every line a session emits carries `seq`: this session's own order, from 1 in this
    // process. radar turns it into the envelope's sequence, continuing across restarts.
    this.#emit = (ev) => emit({ ...ev, seq: ++this.#seq });
    // Lines that are state, not history, take no number.
    this.#emitRaw = emit;
    this.#opts = options;
    this.#ctx = { session, providerSessionId: options.resume ?? options.sessionId };
  }

  get providerSessionId() {
    return this.#ctx.providerSessionId;
  }

  start() {
    const options = {
      ...this.#opts,
      permissionMode: "default",
      // Only what radar's setup asks for; the default would be every source.
      settingSources: this.#opts.settingSources ?? [],
      includePartialMessages: true,
      canUseTool: (toolName, input, o) => this.#canUseTool(toolName, input, o),
    };
    this.#query = this.#queryFn({ prompt: this.#input, options });
    this.#done = this.#run();
    this.#reportCommands();
    return this;
  }

  // The commands this session accepts, once Claude Code has said (it loads skills and plugins
  // first). Later changes come as `commands_changed` messages. A failure here costs the menu,
  // not the session.
  #reportCommands() {
    const ask = this.#query?.supportedCommands;
    if (typeof ask !== "function") return;
    Promise.resolve(ask.call(this.#query))
      .then((list) => this.#emitRaw(commandsLine(this.#ctx, list)))
      .catch(() => {});
  }

  async #run() {
    try {
      for await (const msg of this.#query) {
        if (msg.session_id) this.#ctx.providerSessionId = msg.session_id;
        for (const ev of mapMessage(this.#ctx, msg)) (ev.type === "commands" ? this.#emitRaw : this.#emit)(ev);
      }
      this.#emit(this.#lifecycle("session_ended", { reason: "finished" }));
    } catch (e) {
      this.#emit(this.#lifecycle("error", { message: String(e?.message ?? e) }));
      this.#emit(this.#lifecycle("session_ended", { reason: "error" }));
    }
  }

  #lifecycle(kind, payload) {
    return {
      type: "event",
      session: this.#ctx.session,
      provider_session_id: this.#ctx.providerSessionId,
      // No SDK uuid exists for these; they are not deduplicated across replays.
      provider_event_id: undefined,
      durability: "persisted",
      kind,
      payload,
    };
  }

  #canUseTool(toolName, input, o) {
    const id = o.requestId ?? o.toolUseID;
    const known = this.#pending.get(id);
    if (known) {
      // Delivered again: same promise, same event (radar dedupes on the event id).
      this.#emit(known.event);
      return known.promise;
    }
    if (this.#answered.has(id)) {
      const behavior = this.#answered.get(id);
      return Promise.resolve(
        behavior === "allow"
          ? { behavior: "allow", updatedInput: input }
          : { behavior: "deny", message: "Already denied." },
      );
    }
    let resolve;
    const promise = new Promise((r) => (resolve = r));
    // AskUserQuestion reaches canUseTool like any tool; it is a question for the operator,
    // not a permission, and its answers go back in updatedInput.answers.
    const question = toolName === "AskUserQuestion";
    const event = question ? questionEvent(this.#ctx, id, input, o) : permissionEvent(this.#ctx, id, toolName, input, o);
    this.#pending.set(id, { promise, resolve, event, input, question, suggestions: o.suggestions ?? [] });
    o.signal?.addEventListener("abort", () => {
      if (this.#pending.has(id)) this.answer(id, "deny", "Request cancelled.");
    });
    this.#emit(event);
    return promise;
  }

  send(text) {
    // The uuid we give the turn is the one Claude Code stores it under, so the live event
    // and the history replay carry the same event id.
    const uuid = randomUUID();
    const ok = this.#input.push({
      type: "user",
      uuid,
      message: { role: "user", content: [{ type: "text", text }] },
      parent_tool_use_id: null,
    });
    if (ok) {
      this.#emit({ ...this.#lifecycle("turn_started", {}), durability: "ephemeral" });
      this.#emit(userMessage(this.#ctx, uuid, text));
    }
    return ok;
  }

  async interrupt() {
    await this.#query?.interrupt();
  }

  async setModel(model) {
    await this.#query?.setModel(model);
  }

  // A question (AskUserQuestion) answered: `answers` maps each question's text to the label
  // picked or the operator's own words; null declines it. Returns "answered" or "duplicate";
  // an unknown id, or a permission request, throws.
  answerQuestion(requestId, answers, message) {
    const p = this.#pending.get(requestId);
    if (!p) {
      if (this.#answered.has(requestId)) return "duplicate";
      throw new Error(`unknown question ${requestId}`);
    }
    if (!p.question) throw new Error(`${requestId} is a permission request, not a question`);
    this.#pending.delete(requestId);
    this.#answered.set(requestId, answers ? "allow" : "deny");
    p.resolve(
      answers
        ? { behavior: "allow", updatedInput: { ...p.input, answers } }
        : { behavior: "deny", message: message || "The operator declined to answer." },
    );
    this.#emit(questionResolvedEvent(this.#ctx, requestId, answers ?? null));
    return "answered";
  }

  // behavior: "allow" | "deny". `forSession`: allow it for the rest of the session, with the
  // rules Claude Code suggested for this request kept for the session only (sessionRules).
  // Returns "answered" or "duplicate"; an unknown id throws.
  answer(requestId, behavior, message, { forSession = false } = {}) {
    const p = this.#pending.get(requestId);
    if (!p) {
      if (this.#answered.has(requestId)) return "duplicate";
      throw new Error(`unknown permission request ${requestId}`);
    }
    if (p.question) {
      // Closing the session or a cancelled call declines a question; nothing approves one unanswered.
      if (behavior === "allow") throw new Error(`${requestId} is a question; answer it with answer_question`);
      return this.answerQuestion(requestId, null, message);
    }
    this.#pending.delete(requestId);
    this.#answered.set(requestId, behavior);
    const rules = behavior === "allow" && forSession ? sessionRules(p.suggestions) : [];
    p.resolve(
      behavior === "allow"
        ? { behavior: "allow", updatedInput: p.input, ...(rules.length ? { updatedPermissions: rules } : {}) }
        : { behavior: "deny", message: message || "Denied in radar." },
    );
    this.#emit(permissionResolvedEvent(this.#ctx, requestId, behavior));
    return "answered";
  }

  pendingRequests() {
    return [...this.#pending.values()].map((p) => p.event);
  }

  async reinitialize() {
    await this.#query?.reinitialize();
  }

  async close() {
    for (const id of [...this.#pending.keys()]) this.answer(id, "deny", "Session closed.");
    this.#input.end();
    try {
      this.#query?.close();
    } catch {
      // already closed
    }
    await this.#done;
    this.#cleanup?.();
  }
}
