//! A [`SessionSetup`] in Codex's own terms: what goes into `thread/start` and
//! `thread/resume`, the skills calls, and what Codex cannot take (said, not
//! dropped silently). Pure functions, so the exact JSON is testable.
//!
//! | setup field        | Codex                                                                  |
//! |--------------------|------------------------------------------------------------------------|
//! | `extra_dirs`       | `config.sandbox_workspace_write.writable_roots` (workspace-write only) |
//! | `skill_dirs`       | `skills/extraRoots/set` (the app-server's, before the thread starts)   |
//! | `skills`           | `skills/list`; a missing or switched-off one is said, never switched on |
//! | `project_config`   | off: `config.project_doc_max_bytes = 0` (no project AGENTS.md)         |
//! | `agent`            | none: Codex cannot select or list named agents                         |
//! | `instruction_dirs` | none: Codex reads AGENTS.md from the project and `CODEX_HOME` only     |
//! | `plugin_dirs`      | none: plugins install from a marketplace into `CODEX_HOME`             |
//! | `local_only`       | always so: radar runs Codex locally and starts no cloud task              |

use std::path::{Path, PathBuf};

use radar_sessions::adapter::{CatalogEntry, SessionSetup};
use serde_json::{Map, Value, json};

use crate::adapter::SessionPolicy;
use crate::map::str_at;

/// The `config` member of `thread/start` / `thread/resume`: per-thread config
/// overrides. None when the setup changes nothing. The sandbox mode and the
/// approval policy are sent beside it and never come from here.
pub fn thread_config(policy: Option<&SessionPolicy>, setup: &SessionSetup) -> Option<Value> {
    let mut cfg = Map::new();
    // Read-only has nothing to write to, and full access has no roots to name. Under the
    // operator's own config (`None`) the sandbox is Codex's to choose: the roots are named,
    // and Codex reads them only when that sandbox is workspace-write.
    let workspace_write = policy.is_none_or(|p| p.sandbox == "workspace-write");
    if workspace_write && !setup.extra_dirs.is_empty() {
        cfg.insert(
            "sandbox_workspace_write".into(),
            json!({ "writable_roots": setup.extra_dirs }),
        );
    }
    if !setup.project_config {
        cfg.insert("project_doc_max_bytes".into(), json!(0));
    }
    if let Some(ts) = &setup.tool_server {
        // radar's own MCP server for this thread only, its tools named.
        cfg.insert(
            "mcp_servers".into(),
            json!({ ts.name.as_str(): { "command": ts.command, "args": ts.args, "enabled_tools": ts.tools } }),
        );
    }
    (!cfg.is_empty()).then_some(Value::Object(cfg))
}

/// What the setup asked for and Codex cannot do: one line each, shown in the
/// session as a notice so the operator is not left believing it was applied.
/// Project configuration off is not here: it is applied (the project's
/// AGENTS.md is skipped) and its limits are in the capabilities, because it is
/// the default of a bare setup and a notice on every such session would be noise.
pub fn unsupported_notices(setup: &SessionSetup) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(agent) = &setup.agent {
        out.push(format!(
            "Codex cannot run as a named agent; `{agent}` was not applied and the session runs as plain Codex."
        ));
    }
    if !setup.instruction_dirs.is_empty() {
        out.push(
            "Codex reads AGENTS.md from the project and its own home only; the instruction folders of this setup were not loaded."
                .into(),
        );
    }
    if !setup.plugin_dirs.is_empty() {
        out.push(
            "Codex installs plugins from a marketplace into its own home; the plugin folders of this setup were not loaded."
                .into(),
        );
    }
    out
}

/// A skill as `skills/list` reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillRow {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub enabled: bool,
    pub scope: Option<String>,
    pub plugin: Option<String>,
}

/// The skills of every cwd in a `skills/list` result.
pub fn skill_rows(result: &Value) -> Vec<SkillRow> {
    let mut rows = Vec::new();
    for entry in result
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for s in entry
            .get("skills")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let (Some(name), Some(path)) = (str_at(s, "name"), str_at(s, "path")) else {
                continue;
            };
            rows.push(SkillRow {
                name: name.into(),
                description: str_at(s, "description").unwrap_or_default().into(),
                path: path.into(),
                enabled: s.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                scope: str_at(s, "scope").map(str::to_string),
                plugin: str_at(s, "pluginId").map(str::to_string),
            });
        }
    }
    rows
}

/// The catalog for a setup: the enabled skills, by name. `mine` are the
/// setup's own skill folders (named as the source); `others` are folders other
/// sessions loaded into the same app-server, which this setup did not ask for.
pub fn catalog_skills(rows: &[SkillRow], mine: &[PathBuf], others: &[PathBuf]) -> Vec<CatalogEntry> {
    let under = |p: &Path, roots: &[PathBuf]| roots.iter().find(|r| p.starts_with(r)).cloned();
    let mut out: Vec<CatalogEntry> = rows
        .iter()
        .filter(|r| r.enabled)
        .filter(|r| under(&r.path, mine).is_some() || under(&r.path, others).is_none())
        .map(|r| CatalogEntry {
            name: r.name.clone(),
            description: Some(r.description.clone()).filter(|d| !d.is_empty()),
            source: Some(if let Some(p) = &r.plugin {
                format!("plugin {p}")
            } else if let Some(dir) = under(&r.path, mine) {
                dir.display().to_string()
            } else {
                match r.scope.as_deref() {
                    Some("repo") => "project".into(),
                    Some(other) => other.into(),
                    None => "codex".into(),
                }
            }),
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.dedup_by(|a, b| a.name == b.name);
    out
}

/// What to do about each skill a setup names, given what Codex lists.
#[derive(Debug, PartialEq, Eq)]
pub enum SkillStep {
    /// Codex has no skill of that name.
    Missing(String),
    /// Listed but switched off in Codex's own config: said, never switched on
    /// (that would write the operator's Codex config).
    Enable { name: String, path: PathBuf },
}

pub fn skill_steps(wanted: &[String], rows: &[SkillRow]) -> Vec<SkillStep> {
    let mut steps = Vec::new();
    for name in wanted {
        let named: Vec<&SkillRow> = rows.iter().filter(|r| &r.name == name).collect();
        match named.first() {
            None => steps.push(SkillStep::Missing(name.clone())),
            Some(first) if named.iter().all(|r| !r.enabled) => steps.push(SkillStep::Enable {
                name: name.clone(),
                path: first.path.clone(),
            }),
            Some(_) => {}
        }
    }
    steps
}


#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, path: &str, enabled: bool, scope: &str) -> SkillRow {
        SkillRow {
            name: name.into(),
            description: format!("{name} does a thing"),
            path: path.into(),
            enabled,
            scope: Some(scope.into()),
            plugin: None,
        }
    }

    #[test]
    fn a_setup_with_dirs_names_them_as_writable_roots_and_asks_for_no_project_doc() {
        let setup = SessionSetup {
            extra_dirs: vec!["/a/lib".into(), "/b/docs".into()],
            project_config: false,
            ..Default::default()
        };
        assert_eq!(
            thread_config(Some(&SessionPolicy::default()), &setup),
            Some(json!({
                "sandbox_workspace_write": { "writable_roots": ["/a/lib", "/b/docs"] },
                "project_doc_max_bytes": 0,
            }))
        );
    }

    #[test]
    fn writable_roots_are_for_workspace_write_only() {
        let setup = SessionSetup {
            extra_dirs: vec!["/a/lib".into()],
            project_config: true,
            ..Default::default()
        };
        assert_eq!(thread_config(Some(&SessionPolicy::read_only()), &setup), None);
        // Under the operator's own config Codex picks the sandbox: the roots are named.
        assert_eq!(
            thread_config(None, &setup),
            Some(json!({ "sandbox_workspace_write": { "writable_roots": ["/a/lib"] } }))
        );
        let full = SessionPolicy {
            approval_policy: "untrusted".into(),
            sandbox: "danger-full-access".into(),
        };
        assert_eq!(thread_config(Some(&full), &setup), None);
        assert_eq!(
            thread_config(Some(&SessionPolicy::default()), &SessionSetup::default())
                .unwrap()
                .as_object()
                .unwrap()
                .keys()
                .collect::<Vec<_>>(),
            ["project_doc_max_bytes"],
            "a default setup has project_config off"
        );
    }

    #[test]
    fn the_tool_server_is_a_per_thread_mcp_server_with_its_tools_named() {
        let setup = SessionSetup {
            project_config: true,
            tool_server: Some(radar_sessions::ToolServer {
                name: "radar".into(),
                command: "/opt/radar".into(),
                args: vec!["mcp".into(), "--socket".into(), "/run/radar/mcp/s.sock".into(), "--ticket".into(), "t0".into()],
                tools: vec!["echo".into()],
            }),
            ..Default::default()
        };
        assert_eq!(
            thread_config(Some(&SessionPolicy::default()), &setup),
            Some(json!({
                "mcp_servers": { "radar": {
                    "command": "/opt/radar",
                    "args": ["mcp", "--socket", "/run/radar/mcp/s.sock", "--ticket", "t0"],
                    "enabled_tools": ["echo"],
                } }
            }))
        );
    }

    #[test]
    fn what_codex_cannot_take_is_said() {
        let setup = SessionSetup {
            agent: Some("reviewer".into()),
            instruction_dirs: vec!["/i".into()],
            plugin_dirs: vec!["/p".into()],
            project_config: true,
            ..Default::default()
        };
        let notes = unsupported_notices(&setup);
        assert_eq!(notes.len(), 3);
        assert!(notes[0].contains("`reviewer`"));
        assert!(notes[1].contains("instruction folders"));
        assert!(notes[2].contains("plugin folders"));
        assert!(
            unsupported_notices(&SessionSetup {
                project_config: true,
                ..Default::default()
            })
            .is_empty()
        );
    }

    #[test]
    fn skills_list_maps_to_catalog_entries() {
        // Shape recorded from `codex app-server` 0.158.0 skills/list.
        let listed = json!({ "data": [{
            "cwd": "/work/repo",
            "errors": [],
            "skills": [
                { "name": "demo-skill", "description": "A demo skill", "path": "/x/skills/demo-skill/SKILL.md", "scope": "user", "enabled": true, "pluginId": null },
                { "name": "imagegen", "description": "Make images", "path": "/home/u/.codex/skills/.system/imagegen/SKILL.md", "scope": "system", "enabled": true, "pluginId": null },
                { "name": "off", "description": "Switched off", "path": "/home/u/.codex/skills/off/SKILL.md", "scope": "user", "enabled": false, "pluginId": null },
                { "name": "lint", "description": "", "path": "/work/repo/.agents/skills/lint/SKILL.md", "scope": "repo", "enabled": true, "pluginId": null },
                { "name": "other", "description": "Another session's", "path": "/y/skills/other/SKILL.md", "scope": "user", "enabled": true, "pluginId": null },
                { "name": "docs", "description": "From a plugin", "path": "/p/docs/SKILL.md", "scope": "user", "enabled": true, "pluginId": "docs@acme" },
            ],
        }]});
        let rows = skill_rows(&listed);
        assert_eq!(rows.len(), 6);
        let got = catalog_skills(&rows, &["/x/skills".into()], &["/y/skills".into()]);
        let names: Vec<_> = got.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["demo-skill", "docs", "imagegen", "lint"], "disabled and foreign are left out, sorted");
        let by = |n: &str| got.iter().find(|e| e.name == n).unwrap();
        assert_eq!(by("demo-skill").source.as_deref(), Some("/x/skills"));
        assert_eq!(by("demo-skill").description.as_deref(), Some("A demo skill"));
        assert_eq!(by("imagegen").source.as_deref(), Some("system"));
        assert_eq!(by("lint").source.as_deref(), Some("project"));
        assert_eq!(by("lint").description, None);
        assert_eq!(by("docs").source.as_deref(), Some("plugin docs@acme"));
    }

    #[test]
    fn a_named_skill_is_found_enabled_or_missing() {
        let rows = vec![
            row("a", "/s/a/SKILL.md", true, "user"),
            row("b", "/s/b/SKILL.md", false, "user"),
        ];
        let wanted = ["a".to_string(), "b".into(), "c".into()];
        assert_eq!(
            skill_steps(&wanted, &rows),
            vec![
                SkillStep::Enable {
                    name: "b".into(),
                    path: "/s/b/SKILL.md".into()
                },
                SkillStep::Missing("c".into()),
            ]
        );
    }
}
