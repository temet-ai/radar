//! What radar offers after a `/` in a Codex session, and what each does.
//!
//! The app-server has no slash commands: `/compact` in a turn is a sentence for the model (Codex's
//! own slash commands are its terminal UI's). What it has are the calls those commands make and
//! the skills it lists, so radar offers those under the slash:
//!
//! - `/compact` runs `thread/compact/start`;
//! - `/review [instructions]` runs `review/start` on the uncommitted changes (or on the
//!   instructions);
//! - `/<skill> [text]` is a turn whose input is the skill and the text, from `skills/list`
//!   (what Codex's terminal UI does with `$skill`).
//!
//! Each was run against Codex 0.158.0.

use std::path::Path;

use radar_sessions::SlashCommand;
use serde_json::{Value, json};

use crate::map::{bounded, str_at};

/// What running a command does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Run {
    Compact,
    Review,
    Skill { name: String, path: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cmd {
    pub command: SlashCommand,
    pub run: Run,
}

/// The commands every session has, before `skills/list` has answered.
pub fn builtin() -> Vec<Cmd> {
    vec![
        Cmd { command: SlashCommand::new("compact", "summarise the conversation to free up context"), run: Run::Compact },
        Cmd { command: SlashCommand::new("review", "review the uncommitted changes, or what the text asks").with_hint("[instructions]"), run: Run::Review },
    ]
}

/// The enabled skills in a `skills/list` answer for `cwd`, as commands (a skill whose name is a
/// built-in's is left out: `/review` is the review, not a skill by the name).
pub fn from_skills(result: &Value, cwd: &Path) -> Vec<Cmd> {
    let entries = result.get("data").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let cwd = cwd.to_string_lossy();
    // The answer is per working directory; ours, else the first.
    let Some(entry) = entries.iter().find(|e| str_at(e, "cwd") == Some(&cwd)).or(entries.first()) else {
        return vec![];
    };
    let builtin = builtin();
    let mut skills: Vec<Cmd> = entry
        .get("skills")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|s| s.get("enabled").and_then(Value::as_bool).unwrap_or(true))
        .filter_map(|s| {
            let name = str_at(s, "name").filter(|n| !n.is_empty())?;
            let path = str_at(s, "path").filter(|p| !p.is_empty())?;
            if builtin.iter().any(|b| b.command.name.eq_ignore_ascii_case(name)) {
                return None;
            }
            let about = s
                .pointer("/interface/shortDescription")
                .and_then(Value::as_str)
                .or_else(|| str_at(s, "shortDescription"))
                .filter(|d| !d.trim().is_empty())
                .or_else(|| str_at(s, "description"))
                .unwrap_or_default();
            Some(Cmd { command: SlashCommand::new(name, one_line(about)).with_hint("[text]"), run: Run::Skill { name: name.into(), path: path.into() } })
        })
        .collect();
    // Codex lists them by where they came from; a hundred of them are found by letter.
    skills.sort_by_key(|c| c.command.name.to_lowercase());
    skills
}

/// The built-ins, then the skills.
pub fn all(skills: Vec<Cmd>) -> Vec<Cmd> {
    let mut all = builtin();
    all.extend(skills);
    all
}

/// The command `word` names (case-insensitive).
pub fn find<'a>(cmds: &'a [Cmd], word: &str) -> Option<&'a Cmd> {
    cmds.iter().find(|c| c.command.name.eq_ignore_ascii_case(word))
}

/// The `input` of the turn a skill starts: the skill, then the text after it.
pub fn skill_input(name: &str, path: &str, text: Option<&str>) -> Value {
    let mut input = vec![json!({ "type": "skill", "name": name, "path": path })];
    if let Some(t) = text {
        input.push(json!({ "type": "text", "text": t }));
    }
    Value::Array(input)
}

/// What `/review` looks at.
pub fn review_target(instructions: Option<&str>) -> Value {
    match instructions {
        Some(i) => json!({ "type": "custom", "instructions": i }),
        None => json!({ "type": "uncommittedChanges" }),
    }
}

/// A description as one line of at most 100 characters.
fn one_line(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let first = flat.find(". ").map_or(flat.as_str(), |i| &flat[..=i]);
    bounded(first, 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The shape `skills/list` answered with on Codex 0.158.0.
    fn listed() -> Value {
        json!({ "data": [
            { "cwd": "/other", "errors": [], "skills": [ { "name": "elsewhere", "description": "x", "enabled": true, "path": "/s/elsewhere/SKILL.md", "scope": "repo" } ] },
            { "cwd": "/work/repo", "errors": [], "skills": [
                { "name": "pdf:pdf", "description": "Read, create, inspect, render, and verify PDF files where visual layout matters. Use it for anything with pages.", "shortDescription": null, "enabled": true, "path": "/p/pdf/SKILL.md", "scope": "user", "interface": { "displayName": "PDF", "shortDescription": "Work with PDF files" } },
                { "name": "skill-creator", "description": "Create or update a skill", "enabled": true, "path": "/home/u/.codex/skills/.system/skill-creator/SKILL.md", "scope": "system" },
                { "name": "Alpha", "description": "listed after the others, sorted before them", "enabled": true, "path": "/s/alpha/SKILL.md", "scope": "repo" },
                { "name": "off", "description": "disabled", "enabled": false, "path": "/s/off/SKILL.md", "scope": "user" },
                { "name": "Review", "description": "a skill named like the built-in", "enabled": true, "path": "/s/review/SKILL.md", "scope": "user" },
                { "description": "no name", "enabled": true, "path": "/s/x/SKILL.md", "scope": "user" }
            ] }
        ] })
    }

    #[test]
    fn the_enabled_skills_of_the_sessions_directory_become_commands_with_a_short_line_each() {
        let got = from_skills(&listed(), Path::new("/work/repo"));
        assert_eq!(
            got.iter().map(|c| (c.command.name.as_str(), c.command.description.as_str())).collect::<Vec<_>>(),
            vec![("Alpha", "listed after the others, sorted before them"), ("pdf:pdf", "Work with PDF files"), ("skill-creator", "Create or update a skill")]
        );
        let got = &got[1..];
        assert_eq!(got[1].run, Run::Skill { name: "skill-creator".into(), path: "/home/u/.codex/skills/.system/skill-creator/SKILL.md".into() });
        assert_eq!(got[0].command.argument_hint.as_deref(), Some("[text]"));
        // No entry for the directory: the first answer stands.
        assert_eq!(from_skills(&listed(), Path::new("/elsewhere-entirely")).iter().map(|c| c.command.name.as_str()).collect::<Vec<_>>(), vec!["elsewhere"]);
        assert!(from_skills(&json!({ "data": [] }), Path::new("/work/repo")).is_empty());
    }

    #[test]
    fn a_long_description_is_cut_to_its_first_sentence_and_a_hundred_characters() {
        let skills = json!({ "data": [ { "cwd": "/w", "skills": [
            { "name": "a", "description": "Read, create, inspect, render, and verify PDF files where visual layout matters. Use it for anything.", "path": "/a" },
            { "name": "b", "description": "word ".repeat(60), "path": "/b" }
        ] } ] });
        let got = from_skills(&skills, Path::new("/w"));
        assert_eq!(got[0].command.description, "Read, create, inspect, render, and verify PDF files where visual layout matters.");
        assert!(got[1].command.description.chars().count() <= 101, "{}", got[1].command.description);
        assert!(got[1].command.description.ends_with('…'));
    }

    #[test]
    fn commands_are_found_by_name_whatever_the_case_and_the_builtins_come_first() {
        let cmds = all(from_skills(&listed(), Path::new("/work/repo")));
        assert_eq!(cmds.iter().map(|c| c.command.name.as_str()).take(3).collect::<Vec<_>>(), vec!["compact", "review", "Alpha"]);
        assert_eq!(find(&cmds, "REVIEW").unwrap().run, Run::Review);
        assert!(find(&cmds, "nonesuch").is_none());
    }

    #[test]
    fn what_the_calls_are_given() {
        assert_eq!(
            skill_input("pdf:pdf", "/p/pdf/SKILL.md", Some("merge a and b")),
            json!([{ "type": "skill", "name": "pdf:pdf", "path": "/p/pdf/SKILL.md" }, { "type": "text", "text": "merge a and b" }])
        );
        assert_eq!(skill_input("x", "/x", None), json!([{ "type": "skill", "name": "x", "path": "/x" }]));
        assert_eq!(review_target(None), json!({ "type": "uncommittedChanges" }));
        assert_eq!(review_target(Some("focus on errors")), json!({ "type": "custom", "instructions": "focus on errors" }));
    }
}
