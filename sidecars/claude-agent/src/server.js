import { execFile } from "node:child_process";
import { accessSync, constants } from "node:fs";
import { delimiter, join } from "node:path";
import { AsyncQueue } from "./queue.js";
import { mapMessage } from "./map.js";
import { Session } from "./session.js";
import { catalogOf, materializeSkillPlugin, setupOptions } from "./setup.js";

export const PROTOCOL = 1;

// Requests: one JSON object per line on stdin, { id, op, ...args }.
// Replies:  { type: "reply", id, ok: true, result } | { type: "reply", id, ok: false, error }.
// Events:   { type: "event", ... } (see map.js), unprompted.
//
// Credentials never pass through here. `status` asks the installed `claude` for its own
// account status and forwards only loggedIn, authMethod, email and subscriptionType.

export function findOnPath(name, env = process.env) {
  for (const dir of (env.PATH ?? "").split(delimiter)) {
    if (!dir) continue;
    const p = join(dir, name);
    try {
      accessSync(p, constants.X_OK);
      return p;
    } catch {
      // next
    }
  }
  return null;
}

// What a review session always asks radar about, whatever the operator's own settings allow:
// commands, writes, the network and MCP tools (radar's review policy decides them).
export const REVIEW_ASK = ["Bash", "Edit", "Write", "NotebookEdit", "WebFetch", "mcp__*"];

// The ask rules a session is created with (an ask rule beats every allow rule):
// - `req.ask` when radar names them;
// - a review session: REVIEW_ASK;
// - project configuration on: none. Claude Code applies the operator's own allow, deny
//   and ask rules from the user, project and local settings, runs its built-in read-only
//   commands unasked, and calls canUseTool (the dock) for the rest;
// - project configuration off: no settings file is read, so no rule of the operator's
//   applies, and every shell command asks (the read-only ones too), as before.
export function askRules(req) {
  if (req.ask) return req.ask;
  if (req.setup?.review) return REVIEW_ASK;
  return req.setup?.project_config ? [] : ["Bash"];
}

// The child gets the operator's environment (PATH, proxy, API-key variables pass through
// unread), minus the variables that would load his own plugins or look like a nested session.
const DROP_ENV = ["CLAUDE_CODE_PLUGIN_DIRS", "CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT", "CLAUDE_CODE_SESSION_ID"];

export function sessionEnv(env, extra) {
  const out = { ...env, ...(extra ?? {}) };
  for (const k of DROP_ENV) delete out[k];
  // Claude Code runs commands through $SHELL and snapshots its aliases; an interactive
  // zsh that aliases `ls` to a tool printing nothing without a tty makes every listing empty.
  if (!extra?.SHELL && !/\/(ba)?sh$/.test(out.SHELL ?? "")) out.SHELL = "/bin/bash";
  return out;
}

function run(exec, file, args, env) {
  return new Promise((resolve) => {
    exec(file, args, { timeout: 20000, env, maxBuffer: 1 << 20 }, (err, stdout, stderr) => {
      resolve({ ok: !err, stdout: String(stdout ?? ""), stderr: String(stderr ?? ""), code: err?.code });
    });
  });
}

export function createServer({ sdk, emit, exec = execFile, env = process.env, sidecarVersion = "0.1.0", sdkVersion = "unknown" }) {
  const sessions = new Map();

  // `sdk` is the module, or a function that loads it (main.js: fetched on first use). A failed
  // load is not kept, so the next request tries again.
  let loading = null;
  const getSdk = () => {
    if (typeof sdk !== "function") return Promise.resolve(sdk);
    loading ??= sdk().catch((e) => {
      loading = null;
      throw e;
    });
    return loading;
  };

  // CLAUDE_CONFIG_DIR as radar's Settings -> Agents sets it. A child gets it in its own environment;
  // the SDK's session listing and transcript reads run in this process, so they get it from here.
  const baseConfigDir = env.CLAUDE_CONFIG_DIR;
  const configEnv = (req) => (req.config_dir ? { CLAUDE_CONFIG_DIR: req.config_dir } : {});
  const useConfigDir = (req) => {
    if (req.config_dir) env.CLAUDE_CONFIG_DIR = req.config_dir;
    else if (baseConfigDir === undefined) delete env.CLAUDE_CONFIG_DIR;
    else env.CLAUDE_CONFIG_DIR = baseConfigDir;
  };

  const claudePath = (req) => req.claude_path || env.RADAR_CLAUDE_PATH || findOnPath("claude", env);

  const need = (req) => {
    const s = sessions.get(req.session);
    if (!s) throw new Error(`unknown session ${req.session}`);
    return s;
  };

  const ops = {
    async hello() {
      return { protocol: PROTOCOL, sidecar_version: sidecarVersion, sdk_version: sdkVersion };
    },

    // Installed? Version? Signed in as? Distinguishes a missing executable from a signed-out CLI.
    async status(req) {
      const path = claudePath(req);
      if (!path) return { installed: false };
      const cenv = { ...env, ...configEnv(req) };
      const ver = await run(exec, path, ["--version"], cenv);
      if (!ver.ok) return { installed: true, path, version: null, error: "version_failed" };
      const version = ver.stdout.trim().split(/\s+/)[0];
      const auth = await run(exec, path, ["auth", "status"], cenv);
      let account = { logged_in: false };
      try {
        const j = JSON.parse(auth.stdout);
        account = {
          logged_in: Boolean(j.loggedIn),
          auth_method: j.authMethod,
          email: j.email,
          subscription_type: j.subscriptionType,
        };
      } catch {
        account = { logged_in: false, error: "auth_status_unreadable" };
      }
      return { installed: true, path, version, account };
    },

    async models(req) {
      const q = (await getSdk()).query({
        prompt: new AsyncQueue(),
        options: {
          cwd: req.cwd,
          permissionMode: "default",
          settingSources: [],
          pathToClaudeCodeExecutable: claudePath(req),
          env: sessionEnv(env, configEnv(req)),
        },
      });
      try {
        const list = await q.supportedModels();
        return list.map((m) => ({ id: m.value, name: m.displayName, description: m.description }));
      } finally {
        q.close();
      }
    },

    // What a session in `cwd` can be set up with: the agents and skills Claude Code finds, from
    // a query opened with the setup and closed before any prompt, so nothing is persisted.
    // req: { cwd, setup?, config_dir?, claude_path? }
    async catalog(req) {
      const skillPlugin = materializeSkillPlugin(req.setup?.skill_dirs);
      const { options: so, env: setupEnv } = setupOptions(req.setup, { skillPlugin, catalog: true });
      let q;
      try {
        q = (await getSdk()).query({
          prompt: new AsyncQueue(),
          options: {
            ...so,
            cwd: req.cwd,
            permissionMode: "default",
            pathToClaudeCodeExecutable: claudePath(req),
            env: sessionEnv(env, { ...configEnv(req), ...setupEnv }),
          },
        });
        const [agents, commands] = await Promise.all([q.supportedAgents(), q.supportedCommands()]);
        return catalogOf(agents, commands, { cwd: req.cwd, extraDirs: req.setup?.extra_dirs ?? [], configDir: req.config_dir ?? env.CLAUDE_CONFIG_DIR });
      } finally {
        try {
          q?.close();
        } finally {
          skillPlugin?.cleanup();
        }
      }
    },

    // req: { session (radar id), cwd, session_id? (UUID for a new session) | resume?, model?,
    //        allow?, deny?, claude_path?, config_dir?, setup? (radar-sessions' SessionSetup) }
    async create(req) {
      if (!req.session) throw new Error("create needs session");
      if (sessions.has(req.session)) throw new Error(`session ${req.session} already exists`);
      const options = { cwd: req.cwd, pathToClaudeCodeExecutable: claudePath(req) };
      if (req.model) options.model = req.model;
      if (req.resume) options.resume = req.resume;
      else if (req.session_id) options.sessionId = req.session_id;
      if (req.allow?.length) options.allowedTools = req.allow;
      if (req.deny?.length) options.disallowedTools = req.deny;
      const ask = askRules(req);
      if (ask.length) options.settings = { permissions: { ask } };
      const skillPlugin = materializeSkillPlugin(req.setup?.skill_dirs);
      const { options: so, env: setupEnv } = setupOptions(req.setup, { skillPlugin });
      // radar's own tools are allowed beside the operator's standing allow rules.
      const allowed = [...new Set([...(options.allowedTools ?? []), ...(so.allowedTools ?? [])])];
      Object.assign(options, so);
      if (allowed.length) options.allowedTools = allowed;
      options.env = sessionEnv(env, { ...configEnv(req), ...setupEnv, ...req.env });
      const s = new Session({ queryFn: (await getSdk()).query, emit, session: req.session, options, cleanup: skillPlugin?.cleanup });
      sessions.set(req.session, s);
      try {
        s.start();
      } catch (e) {
        sessions.delete(req.session);
        skillPlugin?.cleanup();
        throw e;
      }
      return { provider_session_id: s.providerSessionId };
    },

    async send(req) {
      if (!need(req).send(String(req.text ?? ""))) throw new Error("session is closed");
      return {};
    },
    async interrupt(req) {
      await need(req).interrupt();
      return {};
    },
    async set_model(req) {
      await need(req).setModel(req.model);
      return {};
    },
    // req: { session, request_id, behavior: "allow" | "deny", message?, scope?: "session" }
    async answer(req) {
      if (req.behavior !== "allow" && req.behavior !== "deny") throw new Error("behavior must be allow or deny");
      if (req.scope !== undefined && req.scope !== "session") throw new Error("scope must be session when given");
      return { outcome: need(req).answer(req.request_id, req.behavior, req.message, { forSession: req.scope === "session" }) };
    },
    // req: { session, request_id, answers: { [question text]: string } | null (declined), message? }
    async answer_question(req) {
      const answers = req.answers ?? null;
      if (answers !== null && (typeof answers !== "object" || Array.isArray(answers))) throw new Error("answers must be an object or null");
      return { outcome: need(req).answerQuestion(req.request_id, answers, req.message) };
    },
    // Re-deliver pending permission requests and questions (after radar reconnects).
    async pending(req) {
      const s = need(req);
      for (const ev of s.pendingRequests()) emit(ev);
      return { count: s.pendingRequests().length };
    },
    async close(req) {
      const s = need(req);
      sessions.delete(req.session);
      await s.close();
      return {};
    },

    async list(req) {
      useConfigDir(req);
      const rows = await (await getSdk()).listSessions({ dir: req.dir, limit: req.limit });
      return rows.map((r) => ({
        provider_session_id: r.sessionId,
        title: r.customTitle ?? r.summary,
        last_modified: r.lastModified,
        cwd: r.cwd,
        git_branch: r.gitBranch,
      }));
    },

    // Provider-owned transcript as persisted events (same ids as live ones, so radar dedupes).
    async history(req) {
      useConfigDir(req);
      const msgs = await (await getSdk()).getSessionMessages(req.provider_session_id, { dir: req.dir, limit: req.limit, offset: req.offset });
      const ctx = { session: req.session, providerSessionId: req.provider_session_id };
      const events = msgs.flatMap((m) => mapMessage(ctx, m));
      // Numbered in the live session's sequence when there is one, so they sit before what follows.
      return { events: sessions.get(req.session)?.stamp(events) ?? events };
    },

    async delete(req) {
      useConfigDir(req);
      await (await getSdk()).deleteSession(req.provider_session_id, { dir: req.dir });
      return {};
    },
  };

  return {
    async handle(req) {
      const op = ops[req?.op];
      if (!op) return { type: "reply", id: req?.id, ok: false, error: `unknown op ${req?.op}` };
      try {
        return { type: "reply", id: req.id, ok: true, result: await op(req) };
      } catch (e) {
        return { type: "reply", id: req.id, ok: false, error: String(e?.message ?? e) };
      }
    },
    async closeAll() {
      await Promise.all([...sessions.values()].map((s) => s.close()));
      sessions.clear();
    },
  };
}
