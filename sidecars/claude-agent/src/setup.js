import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { basename, join } from "node:path";

// What radar's Settings -> Agents sets up for a session (radar-sessions' SessionSetup), mapped onto
// the Agent SDK's options. Every path is absolute.
//
//   extra_dirs        -> additionalDirectories (the SDK passes each as --add-dir)
//   instruction_dirs  -> additionalDirectories, plus CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD=1,
//                        the one switch that makes an added folder's CLAUDE.md load (with the
//                        `project` setting source, so only when project_config is on). Claude Code
//                        reads AGENTS.md in the working folder and above (v2.1.277+) but never in an
//                        added folder, so a folder's AGENTS.md is appended to Claude Code's own
//                        system prompt by the sidecar, by Claude's own rule: only when the folder
//                        has no CLAUDE.md, .claude/CLAUDE.md or CLAUDE.local.md that would load
//                        instead (a CLAUDE.md there does not load with project_config off, so then
//                        the AGENTS.md is appended too).
//   project_config    -> settingSources ["user","project","local"] (off: [])
//   agent             -> agent
//   plugin_dirs       -> plugins [{type:"local", path}]
//   skill_dirs        -> Claude Code has no option that takes a skill folder. Skills load from the
//                        settings sources, from an added folder's .claude/skills and from plugins,
//                        so the skill folders are copied into one generated plugin ("radar-skills",
//                        plugin layout skills/<name>/SKILL.md) that is loaded with the others.
//                        Its skills are called radar-skills:<name>.
//   skills            -> skills [...] only when non-empty (a context filter: the rest are hidden);
//                        empty leaves the SDK default, so every skill found stays available.
//   tool_server       -> mcpServers[name] = {type:"stdio", command, args, alwaysLoad:true} (radar's own
//                        MCP server; alwaysLoad keeps its tools out of tool-search deferral, so they
//                        are there on the first turn) and allowedTools mcp__<name>__<tool> for each of
//                        its tools by name, never a wildcard. An ask or deny rule still wins.

export const SKILL_PLUGIN = "radar-skills";
const INSTRUCTIONS_ENV = "CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD";

const uniq = (xs) => [...new Set(xs)];

function isDir(p) {
  try {
    return statSync(p).isDirectory();
  } catch {
    return false;
  }
}

// The skills a skill folder holds: <dir>/<name>/SKILL.md for each name, or <dir> itself when it
// holds a SKILL.md.
export function findSkills(dir) {
  if (existsSync(join(dir, "SKILL.md"))) return [{ name: basename(dir), path: dir }];
  let entries = [];
  try {
    entries = readdirSync(dir).sort();
  } catch {
    return [];
  }
  return entries.filter((n) => isDir(join(dir, n)) && existsSync(join(dir, n, "SKILL.md"))).map((name) => ({ name, path: join(dir, name) }));
}

// Copies the skill folders into a generated plugin under the temp folder. A symlink would be
// refused (a plugin may not reach outside its own folder), a copy is what the layout asks for.
// The first folder to hold a name wins. Returns null when there is nothing to load.
export function materializeSkillPlugin(skillDirs, { tmp = tmpdir() } = {}) {
  const found = new Map();
  for (const dir of skillDirs ?? []) for (const s of findSkills(dir)) if (!found.has(s.name)) found.set(s.name, s.path);
  if (found.size === 0) return null;
  const root = mkdtempSync(join(tmp, "radar-claude-skills-"));
  mkdirSync(join(root, ".claude-plugin"));
  writeFileSync(join(root, ".claude-plugin", "plugin.json"), JSON.stringify({ name: SKILL_PLUGIN, description: "Skill folders set up in radar" }));
  mkdirSync(join(root, "skills"));
  for (const [name, path] of found) cpSync(path, join(root, "skills", name), { recursive: true, dereference: true });
  return { path: root, names: [...found.keys()], cleanup: () => rmSync(root, { recursive: true, force: true }) };
}

// The AGENTS.md files of the instruction folders Claude Code would not read itself, as text to
// append to its system prompt: a folder's AGENTS.md (or .claude/AGENTS.md) when no CLAUDE.md of
// that folder loads (none there, or project configuration off). Null when there is none.
export function instructionAgentsMd(dirs, projectConfig) {
  const own = (d) => ["CLAUDE.md", join(".claude", "CLAUDE.md"), "CLAUDE.local.md"].some((f) => existsSync(join(d, f)));
  const parts = [];
  for (const d of uniq(dirs ?? [])) {
    if (projectConfig && own(d)) continue;
    for (const f of ["AGENTS.md", join(".claude", "AGENTS.md")]) {
      const path = join(d, f);
      let text;
      try {
        text = readFileSync(path, "utf8").trim();
      } catch {
        continue;
      }
      if (text) parts.push(`# Instructions from ${path}\n\n${text}`);
    }
  }
  return parts.length ? parts.join("\n\n") : null;
}

// The SDK options for a setup. `skillPlugin` is what materializeSkillPlugin returned.
// `catalog` leaves out the agent and the skill filter: it asks what exists, not what a session uses.
// Returns { options, env }: `env` is merged over the child's environment.
export function setupOptions(setup, { skillPlugin = null, catalog = false } = {}) {
  const s = setup ?? {};
  const options = { settingSources: s.project_config ? ["user", "project", "local"] : [] };
  const env = {};
  const instruction = s.instruction_dirs ?? [];
  const dirs = uniq([...(s.extra_dirs ?? []), ...instruction]);
  if (dirs.length) options.additionalDirectories = dirs;
  if (instruction.length) env[INSTRUCTIONS_ENV] = "1";
  const agentsMd = instructionAgentsMd(instruction, !!s.project_config);
  if (agentsMd) options.systemPrompt = { type: "preset", preset: "claude_code", append: agentsMd };
  const plugins = uniq([...(s.plugin_dirs ?? []), ...(skillPlugin ? [skillPlugin.path] : [])]);
  if (plugins.length) options.plugins = plugins.map((path) => ({ type: "local", path }));
  const ts = s.tool_server;
  if (ts?.name && ts.command) {
    options.mcpServers = { [ts.name]: { type: "stdio", command: ts.command, args: ts.args ?? [], alwaysLoad: true } };
    const tools = (ts.tools ?? []).map((t) => `mcp__${ts.name}__${t}`);
    if (tools.length && !catalog) options.allowedTools = tools;
  }
  if (!catalog) {
    if (s.agent) options.agent = s.agent;
    const want = s.skills ?? [];
    if (want.length) {
      // A bare name also matches the generated plugin's copy of it.
      const own = new Set(skillPlugin?.names ?? []);
      options.skills = uniq(want.flatMap((n) => (own.has(n) ? [n, `${SKILL_PLUGIN}:${n}`] : [n])));
    }
  }
  return { options, env };
}

// Where a catalog entry comes from, as far as the files say: a plugin by its prefix, else the
// folder it sits in (project, an added folder, the user's).
export function sourceOf(kind, name, { cwd, extraDirs = [], configDir, home = homedir() }) {
  const colon = name.indexOf(":");
  if (colon > 0) return name.slice(0, colon) === SKILL_PLUGIN ? "setup" : name.slice(0, colon);
  const rel = (base) => (kind === "skill" ? join(base, "skills", name, "SKILL.md") : join(base, "agents", `${name}.md`));
  if (cwd && existsSync(rel(join(cwd, ".claude")))) return "project";
  for (const d of extraDirs) if (existsSync(rel(join(d, ".claude")))) return d;
  if (existsSync(rel(configDir || join(home, ".claude")))) return "user";
  return null;
}

// supportedAgents() and supportedCommands() as radar's Catalog. A skill is a command that is not
// Claude Code's own.
export function catalogOf(agents, commands, ctx) {
  const seen = new Set();
  const skills = [];
  for (const c of commands ?? []) {
    if (c.builtin || seen.has(c.name)) continue;
    seen.add(c.name);
    skills.push({ name: c.name, description: c.description || null, source: sourceOf("skill", c.name, ctx) });
  }
  return {
    agents: (agents ?? []).map((a) => ({ name: a.name, description: a.description || null, source: sourceOf("agent", a.name, ctx) })),
    skills,
  };
}
