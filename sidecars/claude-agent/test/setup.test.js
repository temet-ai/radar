import { test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createServer } from "../src/server.js";
import { catalogOf, findSkills, materializeSkillPlugin, setupOptions, SKILL_PLUGIN } from "../src/setup.js";

const scratch = () => mkdtempSync(join(tmpdir(), "radar-setup-test-"));

function skill(dir, name) {
  mkdirSync(join(dir, name, "refs"), { recursive: true });
  writeFileSync(join(dir, name, "SKILL.md"), `---\nname: ${name}\ndescription: ${name} skill\n---\nbody\n`);
  writeFileSync(join(dir, name, "refs", "a.md"), "ref");
}

function fakeSdk({ agents = [], commands = [] } = {}) {
  const calls = { options: [], closed: 0, prompts: 0 };
  return {
    calls,
    query({ prompt, options }) {
      calls.options.push(options);
      let ended = false;
      return {
        close() {
          calls.closed++;
          ended = true;
        },
        supportedAgents: async () => agents,
        supportedCommands: async () => commands,
        async *[Symbol.asyncIterator]() {
          while (!ended) await new Promise((r) => setImmediate(r));
        },
      };
    },
    listSessions: async () => [],
    getSessionMessages: async () => [],
    deleteSession: async () => {},
  };
}

const SID = "22222222-2222-4222-8222-222222222222";

test("no setup: no settings sources, nothing added", () => {
  const { options, env } = setupOptions(undefined);
  assert.deepEqual(options, { settingSources: [] });
  assert.deepEqual(env, {});
});

test("project_config turns on user, project and local sources; off is none", () => {
  assert.deepEqual(setupOptions({ project_config: true }).options.settingSources, ["user", "project", "local"]);
  assert.deepEqual(setupOptions({ project_config: false }).options.settingSources, []);
});

test("extra dirs and instruction dirs are added directories; instruction dirs switch CLAUDE.md loading on", () => {
  const a = setupOptions({ extra_dirs: ["/x", "/y"] });
  assert.deepEqual(a.options.additionalDirectories, ["/x", "/y"]);
  assert.deepEqual(a.env, {});
  const b = setupOptions({ extra_dirs: ["/x"], instruction_dirs: ["/x", "/i"] });
  assert.deepEqual(b.options.additionalDirectories, ["/x", "/i"]);
  assert.equal(b.env.CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD, "1");
});

test("an instruction folder's AGENTS.md is appended to Claude Code's prompt unless its CLAUDE.md loads instead", () => {
  const root = mkdtempSync(join(tmpdir(), "radar-agentsmd-"));
  const [only, both] = [join(root, "only"), join(root, "both")];
  for (const d of [only, both]) mkdirSync(d);
  writeFileSync(join(only, "AGENTS.md"), "Use tabs.\n");
  writeFileSync(join(both, "AGENTS.md"), "Use spaces.\n");
  writeFileSync(join(both, "CLAUDE.md"), "Claude's own.\n");
  try {
    const on = setupOptions({ instruction_dirs: [only, both], project_config: true }).options.systemPrompt;
    assert.equal(on.type, "preset");
    assert.equal(on.preset, "claude_code");
    assert.match(on.append, /Use tabs\./);
    assert.doesNotMatch(on.append, /Use spaces\./, "its CLAUDE.md loads instead, as Claude Code would choose");
    // With the project source off a CLAUDE.md in an added folder does not load, so its AGENTS.md is given.
    assert.match(setupOptions({ instruction_dirs: [both], project_config: false }).options.systemPrompt.append, /Use spaces\./);
    assert.equal(setupOptions({ extra_dirs: [only] }).options.systemPrompt, undefined, "only instruction folders, never extra folders");
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("agent and plugin dirs map straight; the skills filter only when non-empty", () => {
  const a = setupOptions({ agent: "reviewer", plugin_dirs: ["/p1", "/p2"], skills: [] });
  assert.equal(a.options.agent, "reviewer");
  assert.deepEqual(a.options.plugins, [{ type: "local", path: "/p1" }, { type: "local", path: "/p2" }]);
  assert.equal("skills" in a.options, false, "empty leaves the SDK default");
  const b = setupOptions({ skills: ["pdf", "notes"] });
  assert.deepEqual(b.options.skills, ["pdf", "notes"]);
  assert.equal("agent" in setupOptions({}).options, false);
});

test("a catalog query leaves out the agent and the skill filter", () => {
  const o = setupOptions({ agent: "a", skills: ["s"], project_config: true }, { catalog: true }).options;
  assert.equal("agent" in o, false);
  assert.equal("skills" in o, false);
  assert.deepEqual(o.settingSources, ["user", "project", "local"]);
});

const TOOL_SERVER = { name: "radar", command: "/opt/radar", args: ["mcp", "--socket", "/run/radar/mcp/g1.sock", "--ticket", "t0"], tools: ["echo"] };

test("radar's tool server is a stdio MCP server loaded at once, and its tools are allowed by name", () => {
  const o = setupOptions({ tool_server: TOOL_SERVER }).options;
  assert.deepEqual(o.mcpServers, {
    radar: { type: "stdio", command: "/opt/radar", args: ["mcp", "--socket", "/run/radar/mcp/g1.sock", "--ticket", "t0"], alwaysLoad: true },
  });
  assert.deepEqual(o.allowedTools, ["mcp__radar__echo"]);
  // A catalog query loads the server too (its tools may be listed) but allows nothing.
  const c = setupOptions({ tool_server: TOOL_SERVER }, { catalog: true }).options;
  assert.equal("allowedTools" in c, false);
  assert.equal("mcpServers" in setupOptions({}).options, false);
});

test("create and resume keep the operator's allow rules beside radar's tools", async () => {
  for (const extra of [{ session_id: SID }, { resume: SID }]) {
    const sdk = fakeSdk();
    const server = createServer({ sdk, emit: () => {}, env: { PATH: "" } });
    const r = await server.handle({ id: 1, op: "create", session: "g1", cwd: "/tmp", allow: ["Bash(git diff:*)"], setup: { tool_server: TOOL_SERVER }, ...extra });
    assert.equal(r.ok, true, r.error);
    const o = sdk.calls.options[0];
    assert.deepEqual(o.allowedTools, ["Bash(git diff:*)", "mcp__radar__echo"]);
    assert.equal(o.mcpServers.radar.command, "/opt/radar");
    assert.equal(o.mcpServers.radar.alwaysLoad, true);
    await server.handle({ id: 2, op: "close", session: "g1" });
  }
});

test("skill folders become one generated plugin of copies; a requested skill is also named by its plugin name", () => {
  const dir = scratch();
  const one = scratch();
  skill(dir, "notes");
  skill(dir, "lint");
  mkdirSync(join(dir, "not-a-skill"));
  writeFileSync(join(one, "SKILL.md"), "---\nname: solo\n---\n");
  assert.deepEqual(findSkills(dir).map((s) => s.name), ["lint", "notes"]);
  assert.deepEqual(findSkills(one).map((s) => s.path), [one], "a folder that holds SKILL.md is one skill");
  const p = materializeSkillPlugin([dir]);
  try {
    assert.deepEqual(JSON.parse(readFileSync(join(p.path, ".claude-plugin", "plugin.json"), "utf8")).name, SKILL_PLUGIN);
    assert.ok(existsSync(join(p.path, "skills", "notes", "SKILL.md")));
    assert.ok(existsSync(join(p.path, "skills", "notes", "refs", "a.md")), "supporting files come along");
    assert.deepEqual(p.names, ["lint", "notes"]);
    const { options } = setupOptions({ skill_dirs: [dir], plugin_dirs: ["/p"], skills: ["notes", "other"] }, { skillPlugin: p });
    assert.deepEqual(options.plugins, [{ type: "local", path: "/p" }, { type: "local", path: p.path }]);
    assert.deepEqual(options.skills, ["notes", `${SKILL_PLUGIN}:notes`, "other"]);
  } finally {
    p.cleanup();
  }
  assert.equal(existsSync(p.path), false, "cleanup removes the generated plugin");
  assert.equal(materializeSkillPlugin([]), null);
  assert.equal(materializeSkillPlugin([scratch()]), null);
  rmSync(dir, { recursive: true });
});

test("create sends the setup to the SDK, keeps permissionMode default, and removes the generated plugin on close", async () => {
  const dir = scratch();
  skill(dir, "notes");
  const sdk = fakeSdk();
  const server = createServer({ sdk, emit: () => {}, env: { PATH: "", CLAUDE_CONFIG_DIR: "/operator", SHELL: "/bin/bash" } });
  const setup = { extra_dirs: ["/e"], agent: "rev", skill_dirs: [dir], skills: ["notes"], instruction_dirs: ["/i"], plugin_dirs: ["/p"], project_config: true, local_only: true };
  const r = await server.handle({ id: 1, op: "create", session: "g1", cwd: "/tmp", session_id: SID, setup, config_dir: "/cfg" });
  assert.equal(r.ok, true, r.error);
  const o = sdk.calls.options[0];
  assert.equal(o.permissionMode, "default", "project configuration never changes the mode");
  assert.deepEqual(o.settingSources, ["user", "project", "local"]);
  assert.deepEqual(o.additionalDirectories, ["/e", "/i"]);
  assert.equal(o.agent, "rev");
  assert.equal(o.plugins[0].path, "/p");
  const gen = o.plugins[1].path;
  assert.ok(existsSync(gen));
  assert.deepEqual(o.skills, ["notes", "radar-skills:notes"]);
  assert.equal(o.env.CLAUDE_CONFIG_DIR, "/cfg");
  assert.equal(o.env.CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD, "1");
  // The operator's own allow rules in those settings apply: radar adds no ask rule over them.
  assert.equal("settings" in o, false, "no forced ask rule with project configuration on");
  await server.handle({ id: 2, op: "close", session: "g1" });
  assert.equal(existsSync(gen), false);
  rmSync(dir, { recursive: true });
});

test("a create without a setup loads no settings and no extras", async () => {
  const sdk = fakeSdk();
  const server = createServer({ sdk, emit: () => {}, env: { PATH: "" } });
  await server.handle({ id: 1, op: "create", session: "g1", cwd: "/tmp", session_id: SID });
  const o = sdk.calls.options[0];
  assert.deepEqual(o.settingSources, []);
  for (const k of ["additionalDirectories", "agent", "plugins", "skills"]) assert.equal(k in o, false, k);
  assert.equal("CLAUDE_CONFIG_DIR" in o.env, false);
  await server.handle({ id: 2, op: "close", session: "g1" });
});

test("resume carries the setup too", async () => {
  const sdk = fakeSdk();
  const server = createServer({ sdk, emit: () => {}, env: { PATH: "" } });
  await server.handle({ id: 1, op: "create", session: "g1", cwd: "/tmp", resume: SID, setup: { agent: "rev", project_config: true } });
  const o = sdk.calls.options[0];
  assert.equal(o.resume, SID);
  assert.equal(o.agent, "rev");
  assert.deepEqual(o.settingSources, ["user", "project", "local"]);
  await server.handle({ id: 2, op: "close", session: "g1" });
});

test("catalog opens a query with the setup, sends no prompt, lists agents and skills, and closes it", async () => {
  const cwd = scratch();
  mkdirSync(join(cwd, ".claude", "agents"), { recursive: true });
  writeFileSync(join(cwd, ".claude", "agents", "reviewer.md"), "---\nname: reviewer\n---\n");
  skill(join(cwd, ".claude", "skills"), "notes");
  const sdk = fakeSdk({
    agents: [
      { name: "reviewer", description: "Reviews", model: "inherit" },
      { name: "Explore", description: "Searches" },
    ],
    commands: [
      { name: "compact", description: "Compact", argumentHint: "", builtin: true },
      { name: "notes", description: "Take notes", argumentHint: "" },
      { name: "radar-skills:lint", description: "Lint", argumentHint: "" },
      { name: "acme:deploy", description: "", argumentHint: "" },
      { name: "notes", description: "dup", argumentHint: "" },
    ],
  });
  const server = createServer({ sdk, emit: () => {}, env: { PATH: "" } });
  const r = await server.handle({ id: 1, op: "catalog", cwd, config_dir: "/cfg", setup: { agent: "x", skills: ["y"], project_config: true, extra_dirs: ["/e"] } });
  assert.equal(r.ok, true, r.error);
  assert.deepEqual(r.result.agents, [
    { name: "reviewer", description: "Reviews", source: "project" },
    { name: "Explore", description: "Searches", source: null },
  ]);
  assert.deepEqual(r.result.skills, [
    { name: "notes", description: "Take notes", source: "project" },
    { name: "radar-skills:lint", description: "Lint", source: "setup" },
    { name: "acme:deploy", description: null, source: "acme" },
  ]);
  const o = sdk.calls.options[0];
  assert.deepEqual(o.settingSources, ["user", "project", "local"]);
  assert.deepEqual(o.additionalDirectories, ["/e"]);
  assert.equal("agent" in o, false);
  assert.equal("skills" in o, false);
  assert.equal(o.env.CLAUDE_CONFIG_DIR, "/cfg");
  assert.equal(sdk.calls.closed, 1, "the query is closed, so no session is kept");
  rmSync(cwd, { recursive: true });
});

test("catalog closes its query and removes the generated plugin when the SDK fails", async () => {
  const dir = scratch();
  skill(dir, "notes");
  const sdk = fakeSdk();
  const q = sdk.query;
  let gen;
  sdk.query = (a) => {
    const r = q(a);
    gen = a.options.plugins[0].path;
    r.supportedAgents = async () => {
      throw new Error("no init");
    };
    return r;
  };
  const server = createServer({ sdk, emit: () => {}, env: { PATH: "" } });
  const r = await server.handle({ id: 1, op: "catalog", cwd: "/tmp", setup: { skill_dirs: [dir] } });
  assert.equal(r.ok, false);
  assert.match(r.error, /no init/);
  assert.equal(sdk.calls.closed, 1);
  assert.equal(existsSync(gen), false);
  rmSync(dir, { recursive: true });
});

test("catalogOf reads sources from the files: user folder, added folder", () => {
  const cfg = scratch();
  const extra = scratch();
  skill(join(cfg, "skills"), "mine");
  skill(join(extra, ".claude", "skills"), "shared");
  const out = catalogOf([], [{ name: "mine", description: "m" }, { name: "shared", description: "s" }, { name: "x", description: "x" }], { cwd: scratch(), extraDirs: [extra], configDir: cfg });
  assert.deepEqual(out.skills.map((s) => s.source), ["user", extra, null]);
});

test("a config dir reaches the SDK's session reads in this process, and goes back when absent", async () => {
  const seen = [];
  const sdk = fakeSdk();
  sdk.listSessions = async () => {
    seen.push(env.CLAUDE_CONFIG_DIR);
    return [];
  };
  const env = { PATH: "", CLAUDE_CONFIG_DIR: "/operator" };
  const server = createServer({ sdk, emit: () => {}, env });
  await server.handle({ id: 1, op: "list", config_dir: "/cfg" });
  await server.handle({ id: 2, op: "list" });
  assert.deepEqual(seen, ["/cfg", "/operator"]);
});
