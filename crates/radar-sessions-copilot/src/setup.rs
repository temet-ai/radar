//! What a radar [`SessionSetup`] means to Copilot: the SDK's session options, the two calls a
//! session makes once it is open (select the agent, enable the skills), and Copilot's
//! agent and skill lists as a radar [`Catalog`].
//!
//! Everything here is pure, so tests assert each field without a CLI.

use std::path::PathBuf;

use radar_sessions::adapter::{CatalogEntry, SessionSetup};
use github_copilot_sdk::rpc::{AgentInfo, AgentsDiscoverRequest, Skill, SkillsDiscoverRequest};
use github_copilot_sdk::types::{McpServerConfig, McpStdioServerConfig};
use github_copilot_sdk::{IndexMap, ResumeSessionConfig, SessionConfig};

fn some_paths(paths: &[PathBuf]) -> Option<Vec<PathBuf>> {
    (!paths.is_empty()).then(|| paths.to_vec())
}

/// The options `SessionConfig` and `ResumeSessionConfig` share, from one setup:
///
/// - `extra_dirs` is `additional_directories`; `skill_dirs`, `instruction_dirs` and
///   `plugin_dirs` are the three `*_directories` (an empty list sets nothing).
/// - `project_config` is `enable_config_discovery`; off also sets
///   `skip_custom_instructions`, so only what the setup names is loaded.
/// - `local_only` is `custom_agents_local_only`.
/// - skills are on whenever the setup names a skill or a skill folder.
/// - `agent` and `skills` are not options: Copilot takes them as calls on the open
///   session, see [`Followup`].
macro_rules! apply_setup {
    ($cfg:expr, $s:expr) => {{
        let (cfg, s) = (&mut $cfg, $s);
        cfg.additional_directories = some_paths(&s.extra_dirs);
        cfg.skill_directories = some_paths(&s.skill_dirs);
        cfg.instruction_directories = some_paths(&s.instruction_dirs);
        cfg.plugin_directories = some_paths(&s.plugin_dirs);
        cfg.enable_config_discovery = Some(s.project_config);
        cfg.skip_custom_instructions = Some(!s.project_config);
        cfg.custom_agents_local_only = Some(s.local_only);
        if !s.skills.is_empty() || !s.skill_dirs.is_empty() {
            cfg.enable_skills = Some(true);
        }
        if let Some(servers) = mcp_servers(s) {
            cfg.mcp_servers = Some(servers);
        }
    }};
}

/// radar's own MCP server (`tool_server`) as Copilot's stdio server config, its tools named.
/// Copilot asks permission for each call (kind `mcp`, with `serverName` and `toolName`).
pub fn mcp_servers(setup: &SessionSetup) -> Option<IndexMap<String, McpServerConfig>> {
    let ts = setup.tool_server.as_ref()?;
    let server = McpServerConfig::Stdio(McpStdioServerConfig {
        tools: Some(ts.tools.clone()),
        command: ts.command.clone(),
        args: ts.args.clone(),
        ..Default::default()
    });
    Some(IndexMap::from([(ts.name.clone(), server)]))
}

/// `setup` applied to a new session's options.
pub fn apply_create(mut cfg: SessionConfig, setup: &SessionSetup) -> SessionConfig {
    apply_setup!(cfg, setup);
    cfg
}

/// `setup` applied to a resumed session's options. Copilot forgets most of this on a
/// cold resume, so the same fields are supplied again.
pub fn apply_resume(mut cfg: ResumeSessionConfig, setup: &SessionSetup) -> ResumeSessionConfig {
    apply_setup!(cfg, setup);
    cfg
}

/// What is done to an open session after create or resume. `agent` is selected with
/// `session.agent.select`: the SDK's `agent` option names one of `custom_agents` that
/// the caller supplies, and the file-based agents (`.github/agents`, `~/.copilot/agents`)
/// are not in that list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Followup {
    pub agent: Option<String>,
    /// Skills to enable by name, so none is filtered out by the user's own config.
    pub skills: Vec<String>,
}

impl Followup {
    pub fn of(setup: &SessionSetup) -> Self {
        Self {
            agent: setup.agent.clone().filter(|a| !a.trim().is_empty()),
            skills: setup.skills.clone(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.agent.is_none() && self.skills.is_empty()
    }
}

/// The names in `wanted` that `listed` (name, enabled) does not hold enabled.
pub fn missing_skills(
    wanted: &[String],
    listed: impl IntoIterator<Item = (String, bool)>,
) -> Vec<&str> {
    let ok: std::collections::HashSet<String> = listed
        .into_iter()
        .filter_map(|(n, enabled)| enabled.then_some(n))
        .collect();
    wanted
        .iter()
        .filter(|w| !ok.contains(w.as_str()))
        .map(String::as_str)
        .collect()
}

/// The request for the client-level agent discovery: user, plugin and `cwd`'s project.
pub fn agents_request(cwd: &std::path::Path) -> AgentsDiscoverRequest {
    AgentsDiscoverRequest {
        exclude_host_agents: None,
        project_paths: Some(vec![cwd.to_string_lossy().into_owned()]),
    }
}

/// The request for the client-level skill discovery: the same, plus the setup's folders.
pub fn skills_request(cwd: &std::path::Path, setup: &SessionSetup) -> SkillsDiscoverRequest {
    SkillsDiscoverRequest {
        exclude_host_skills: None,
        ignored_skills_locations: None,
        project_paths: Some(vec![cwd.to_string_lossy().into_owned()]),
        skill_directories: (!setup.skill_dirs.is_empty()).then(|| {
            setup
                .skill_dirs
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect()
        }),
    }
}

/// Whether the client-level discovery sees what a session with `setup` would: it cannot
/// turn config discovery off and does not know the plugin folders, so those two ask a
/// probe session instead.
pub fn discovery_is_faithful(setup: &SessionSetup) -> bool {
    setup.project_config && setup.plugin_dirs.is_empty()
}

/// The name of a serde enum variant (`personal-copilot`), `None` for the unknown one.
fn variant<T: serde::Serialize>(v: &T) -> Option<String> {
    match serde_json::to_value(v).ok()? {
        serde_json::Value::String(s) if !s.eq_ignore_ascii_case("unknown") => Some(s),
        _ => None,
    }
}

fn text(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// Agents a session can run as. Sub-agent-only ones are left out; the first of a name stays.
pub fn agent_entries(agents: &[AgentInfo]) -> Vec<CatalogEntry> {
    let mut out: Vec<CatalogEntry> = Vec::new();
    for a in agents {
        if a.user_invocable == Some(false) || a.name.trim().is_empty() {
            continue;
        }
        if out.iter().any(|e| e.name == a.name) {
            continue;
        }
        out.push(CatalogEntry {
            name: a.name.clone(),
            description: text(&a.description),
            source: a.source.as_ref().and_then(variant),
        });
    }
    out
}

/// A skill as the two lists give it: the server's (discovery) and the session's.
pub struct SkillRow {
    pub name: String,
    pub description: String,
    pub source: Option<String>,
}

impl From<&Skill> for SkillRow {
    fn from(s: &Skill) -> Self {
        Self {
            name: s.name.clone(),
            description: s.description.clone(),
            source: match (&s.plugin_name, variant(&s.source)) {
                (Some(p), _) => Some(format!("plugin {p}")),
                (None, v) => v,
            },
        }
    }
}

impl From<&github_copilot_sdk::rpc::ServerSkill> for SkillRow {
    fn from(s: &github_copilot_sdk::rpc::ServerSkill) -> Self {
        Self {
            name: s.name.clone(),
            description: s.description.clone(),
            source: variant(&s.source),
        }
    }
}

pub fn skill_entries(skills: impl IntoIterator<Item = SkillRow>) -> Vec<CatalogEntry> {
    let mut out: Vec<CatalogEntry> = Vec::new();
    for s in skills {
        if s.name.trim().is_empty() || out.iter().any(|e| e.name == s.name) {
            continue;
        }
        out.push(CatalogEntry {
            name: s.name,
            description: text(&s.description),
            source: s.source,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use github_copilot_sdk::session_events::SkillSource;
    use github_copilot_sdk::types::SessionId;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    fn full() -> SessionSetup {
        SessionSetup {
            extra_dirs: vec![p("/data/shared"), p("/data/more")],
            agent: Some("scratch-reviewer".into()),
            skill_dirs: vec![p("/skills/a")],
            skills: vec!["plan".into()],
            instruction_dirs: vec![p("/rules")],
            plugin_dirs: vec![p("/plugins/x")],
            project_config: true,
            local_only: true,
            review: false,
            tool_server: Some(radar_sessions::ToolServer {
                name: "radar".into(),
                command: "/opt/radar".into(),
                args: vec!["mcp".into(), "--socket".into(), "/run/radar/mcp/s.sock".into(), "--ticket".into(), "t0".into()],
                tools: vec!["echo".into()],
            }),
        }
    }

    /// The MCP servers as Copilot gets them on the wire.
    fn wire(servers: &Option<IndexMap<String, McpServerConfig>>) -> serde_json::Value {
        serde_json::to_value(servers).unwrap()
    }

    #[test]
    fn the_tool_server_reaches_a_new_and_a_resumed_session_as_stdio_with_its_tools_named() {
        let want = serde_json::json!({
            "radar": {
                "type": "stdio",
                "tools": ["echo"],
                "command": "/opt/radar",
                "args": ["mcp", "--socket", "/run/radar/mcp/s.sock", "--ticket", "t0"],
            }
        });
        assert_eq!(wire(&apply_create(SessionConfig::default(), &full()).mcp_servers), want);
        assert_eq!(wire(&apply_resume(ResumeSessionConfig::new(SessionId::new("s")), &full()).mcp_servers), want);
        // No server set up: the operator's own MCP config is left alone.
        assert!(apply_create(SessionConfig::default(), &SessionSetup::default()).mcp_servers.is_none());
    }

    #[test]
    fn every_field_reaches_a_new_session() {
        let c = apply_create(SessionConfig::default(), &full());
        assert_eq!(
            c.additional_directories,
            Some(vec![p("/data/shared"), p("/data/more")])
        );
        assert_eq!(c.skill_directories, Some(vec![p("/skills/a")]));
        assert_eq!(c.instruction_directories, Some(vec![p("/rules")]));
        assert_eq!(c.plugin_directories, Some(vec![p("/plugins/x")]));
        assert_eq!(c.enable_config_discovery, Some(true));
        assert_eq!(c.skip_custom_instructions, Some(false));
        assert_eq!(c.custom_agents_local_only, Some(true));
        assert_eq!(c.enable_skills, Some(true));
        // The agent is selected on the open session, never passed as an option.
        assert_eq!(c.agent, None);
        assert!(c.custom_agents.is_none());
        assert_eq!(c.disabled_skills, None);
    }

    #[test]
    fn every_field_reaches_a_resumed_session() {
        let c = apply_resume(ResumeSessionConfig::new(SessionId::new("s")), &full());
        assert_eq!(
            c.additional_directories,
            Some(vec![p("/data/shared"), p("/data/more")])
        );
        assert_eq!(c.skill_directories, Some(vec![p("/skills/a")]));
        assert_eq!(c.instruction_directories, Some(vec![p("/rules")]));
        assert_eq!(c.plugin_directories, Some(vec![p("/plugins/x")]));
        assert_eq!(c.enable_config_discovery, Some(true));
        assert_eq!(c.skip_custom_instructions, Some(false));
        assert_eq!(c.custom_agents_local_only, Some(true));
        assert_eq!(c.enable_skills, Some(true));
        assert_eq!(c.agent, None);
    }

    #[test]
    fn no_project_config_turns_discovery_and_custom_instructions_off() {
        let s = SessionSetup {
            project_config: false,
            ..Default::default()
        };
        for (disc, skip) in [
            {
                let c = apply_create(SessionConfig::default(), &s);
                (c.enable_config_discovery, c.skip_custom_instructions)
            },
            {
                let c = apply_resume(ResumeSessionConfig::new(SessionId::new("s")), &s);
                (c.enable_config_discovery, c.skip_custom_instructions)
            },
        ] {
            assert_eq!(disc, Some(false));
            assert_eq!(skip, Some(true));
        }
    }

    #[test]
    fn an_empty_setup_sets_no_folders_and_keeps_skills_as_they_were() {
        let s = SessionSetup {
            project_config: true,
            ..Default::default()
        };
        let c = apply_create(SessionConfig::default(), &s);
        assert_eq!(c.additional_directories, None);
        assert_eq!(c.skill_directories, None);
        assert_eq!(c.instruction_directories, None);
        assert_eq!(c.plugin_directories, None);
        assert_eq!(c.enable_skills, None);
        assert_eq!(c.custom_agents_local_only, Some(false));
    }

    #[test]
    fn a_skill_folder_alone_turns_skills_on() {
        let s = SessionSetup {
            skill_dirs: vec![p("/skills/a")],
            ..Default::default()
        };
        assert_eq!(
            apply_create(SessionConfig::default(), &s).enable_skills,
            Some(true)
        );
    }

    #[test]
    fn the_followup_holds_the_agent_and_the_named_skills() {
        let f = Followup::of(&full());
        assert_eq!(f.agent.as_deref(), Some("scratch-reviewer"));
        assert_eq!(f.skills, vec!["plan".to_string()]);
        assert!(!f.is_empty());
        let blank = SessionSetup {
            agent: Some("  ".into()),
            ..Default::default()
        };
        assert!(Followup::of(&blank).is_empty());
    }

    #[test]
    fn a_named_skill_that_is_off_or_absent_is_missing() {
        let wanted = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let listed = vec![
            ("a".to_string(), true),
            ("b".to_string(), false),
            ("z".to_string(), true),
        ];
        assert_eq!(missing_skills(&wanted, listed), vec!["b", "c"]);
    }

    #[test]
    fn discovery_stands_in_for_a_session_only_when_it_sees_the_same() {
        let mut s = full();
        s.plugin_dirs.clear();
        assert!(discovery_is_faithful(&s));
        s.plugin_dirs.push(p("/plugins/x"));
        assert!(!discovery_is_faithful(&s));
        s.plugin_dirs.clear();
        s.project_config = false;
        assert!(!discovery_is_faithful(&s));
    }

    #[test]
    fn discovery_requests_carry_the_directory_and_the_skill_folders() {
        let a = agents_request(std::path::Path::new("/work/repo"));
        assert_eq!(a.project_paths, Some(vec!["/work/repo".to_string()]));
        let s = skills_request(std::path::Path::new("/work/repo"), &full());
        assert_eq!(s.project_paths, Some(vec!["/work/repo".to_string()]));
        assert_eq!(s.skill_directories, Some(vec!["/skills/a".to_string()]));
        let none = skills_request(std::path::Path::new("/w"), &SessionSetup::default());
        assert_eq!(none.skill_directories, None);
    }

    fn agent(name: &str, desc: &str, invocable: Option<bool>) -> AgentInfo {
        AgentInfo {
            name: name.into(),
            id: name.into(),
            description: desc.into(),
            user_invocable: invocable,
            ..Default::default()
        }
    }

    #[test]
    fn agents_become_entries_without_subagent_only_ones_or_repeats() {
        let mut project = agent("reviewer", "Reviews code", None);
        project.source = Some(github_copilot_sdk::rpc::AgentInfoSource::Project);
        let list = vec![
            project,
            agent("helper", "only for other agents", Some(false)),
            agent("reviewer", "a repeat", None),
            agent("bare", "  ", Some(true)),
        ];
        let got = agent_entries(&list);
        assert_eq!(
            got,
            vec![
                CatalogEntry {
                    name: "reviewer".into(),
                    description: Some("Reviews code".into()),
                    source: Some("project".into()),
                },
                CatalogEntry {
                    name: "bare".into(),
                    description: None,
                    source: None,
                },
            ]
        );
    }

    #[test]
    fn skills_become_entries_with_their_source() {
        let custom = Skill {
            name: "triage".into(),
            description: "Sorts bugs".into(),
            source: SkillSource::Custom,
            ..Default::default()
        };
        let plugin = Skill {
            name: "deploy".into(),
            description: "".into(),
            source: SkillSource::Plugin,
            plugin_name: Some("ops".into()),
            ..Default::default()
        };
        let unknown = Skill {
            name: "odd".into(),
            description: "d".into(),
            ..Default::default()
        };
        let rows: Vec<SkillRow> = [&custom, &plugin, &unknown, &custom]
            .into_iter()
            .map(SkillRow::from)
            .collect();
        let got = skill_entries(rows);
        assert_eq!(got.len(), 3, "a repeated name is listed once");
        assert_eq!(got[0].source.as_deref(), Some("custom"));
        assert_eq!(got[1].source.as_deref(), Some("plugin ops"));
        assert_eq!(got[1].description, None);
        assert_eq!(got[2].source, None);
    }
}
