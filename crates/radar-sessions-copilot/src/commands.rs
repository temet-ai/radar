//! Copilot's slash commands, as radar runs them.
//!
//! The CLI server does not read a `/command` in a message: sent with `session.send`, `/env` is
//! a prompt to the model (verified live against Copilot CLI 1.0.97). The commands run through
//! `session.commands.invoke`, which answers with text to show, a prompt to submit, a plain
//! completion, or a list of subcommands. `session.commands.list` says which commands a session
//! has. Both are experimental in the SDK, so the answers are read here from their JSON (by
//! `kind`) and anything unfamiliar is shown as text, never guessed at.
//!
//! radar offers only the commands in [`RUNS`]: those whose answer radar can draw in its transcript and
//! that leave the operator in charge. The others the CLI lists are named in [`why_not`], so
//! typing one explains itself instead of reaching the model as a sentence.

use radar_sessions::SlashCommand;
use serde_json::Value;

/// A command the session has (`session.commands.list`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandInfo {
    pub name: String,
    pub aliases: Vec<String>,
    pub description: String,
    /// The hint for what follows the name (`<delay> <prompt>`).
    pub hint: Option<String>,
}

impl CommandInfo {
    /// Whether `word` (case-insensitive, no slash) names this command or an alias of it.
    pub fn answers_to(&self, word: &str) -> bool {
        self.name.eq_ignore_ascii_case(word) || self.aliases.iter().any(|a| a.eq_ignore_ascii_case(word))
    }
}

/// What invoking a command did (`session.commands.invoke`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Invoked {
    /// Text the CLI would print.
    Text(String),
    /// A prompt for the model; `display` is what the transcript shows for it, `plan` that it is
    /// asked in Copilot's plan mode.
    Prompt { prompt: String, display: String, plan: bool },
    /// Done, with a message when it has one.
    Done(Option<String>),
    /// The command wants a subcommand: a title and `(name, description)` options.
    Choose { title: String, options: Vec<(String, String)> },
}

/// The commands radar runs, each tried against the live CLI: they answer with text or a prompt,
/// or change something about the session radar can see (`model`, `rename`).
pub const RUNS: &[&str] = &[
    "compact",
    "context",
    "env",
    "usage",
    "list-dirs",
    "model",
    "plan",
    "rename",
    "review",
    "security-review",
    "init",
    "blame",
    "research",
    "chronicle",
    "memory",
    "reset-allowed-tools",
];

/// Why radar does not run a command the CLI lists; `None` for one it runs.
pub fn why_not(name: &str) -> Option<&'static str> {
    if RUNS.contains(&name) {
        return None;
    }
    Some(match name {
        "allow-all" | "permissions" | "autopilot" | "add-dir" => "it changes what the agent may do without asking, and in radar the operator answers each request",
        "share" | "remote" | "computer" => "it leaves this machine, and radar runs Copilot locally only",
        "every" | "after" => "it runs prompts unattended",
        "cwd" => "the session stays in the folder radar started it in",
        _ => "it needs Copilot's own screen; open Copilot in a terminal tab for it",
    })
}

/// The commands in a `session.commands.list` answer.
pub fn parse_list(v: &Value) -> Vec<CommandInfo> {
    v["commands"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|c| {
                    Some(CommandInfo {
                        name: c["name"].as_str().filter(|n| !n.is_empty())?.to_string(),
                        aliases: c["aliases"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default(),
                        description: c["description"].as_str().unwrap_or_default().to_string(),
                        hint: c["input"]["hint"].as_str().filter(|h| !h.is_empty()).map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// What the composer's menu offers: the listed commands radar runs, in the CLI's order.
pub fn advertised(list: &[CommandInfo]) -> Vec<SlashCommand> {
    list.iter()
        .filter(|c| RUNS.contains(&c.name.as_str()))
        .map(|c| {
            let mut s = SlashCommand::new(&c.name, one_line(&c.description));
            if let Some(h) = &c.hint {
                s = s.with_hint(h);
            }
            s
        })
        .collect()
}

/// A description as one short line (the CLI's run to several sentences).
fn one_line(text: &str) -> String {
    let first = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let end = first.find(". ").map_or(first.len(), |i| i + 1);
    first[..end].to_string()
}

/// A command's output as a transcript note: bounded, since `/env` and `/skills` run to pages.
pub fn cap(text: &str) -> String {
    const MAX: usize = 3000;
    let text = text.trim();
    if text.chars().count() <= MAX {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(MAX).collect();
    cut.push_str("\n…");
    cut
}

/// The CLI's own words from an RPC failure (`... failed with message: Nothing to compact.`).
pub fn reason(error: &str) -> String {
    error.rsplit_once("failed with message: ").map_or(error, |(_, m)| m).trim().to_string()
}

/// The answer of `session.commands.invoke`, from its JSON.
pub fn parse_invoked(v: &Value) -> Invoked {
    let s = |k: &str| v[k].as_str().unwrap_or_default().to_string();
    match v["kind"].as_str().unwrap_or_default() {
        "text" => Invoked::Text(s("text")),
        "agent-prompt" => {
            let prompt = s("prompt");
            let display = v["displayPrompt"].as_str().map(str::to_string).unwrap_or_else(|| prompt.clone());
            Invoked::Prompt { prompt, display, plan: v["mode"] == "plan" }
        }
        "completed" => Invoked::Done(v["message"].as_str().filter(|m| !m.is_empty()).map(str::to_string)),
        "select-subcommand" => Invoked::Choose {
            title: s("title"),
            options: v["options"]
                .as_array()
                .map(|a| a.iter().map(|o| (o["name"].as_str().unwrap_or_default().to_string(), o["description"].as_str().unwrap_or_default().to_string())).collect())
                .unwrap_or_default(),
        },
        // A dialog, a timeline entry, a model change: nothing radar draws, so say what the CLI said, if anything.
        other => Invoked::Text(v["entry"]["text"].as_str().map_or_else(|| format!("({other} is not something radar can show)"), str::to_string)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Shapes recorded from `session.commands.list` and `.invoke` on Copilot CLI 1.0.97.
    fn listed() -> Value {
        json!({"commands": [
            {"name": "add-dir", "kind": "builtin", "description": "Allow file access to a directory", "allowDuringAgentExecution": true, "input": {"hint": "directory"}},
            {"name": "allow-all", "kind": "builtin", "aliases": ["yolo"], "description": "Enable all permissions", "input": {"hint": "[on|off|show]"}},
            {"name": "env", "kind": "builtin", "description": "Show loaded environment details (instructions, MCP servers, skills).", "allowDuringAgentExecution": true},
            {"name": "plan", "kind": "builtin", "description": "Create an implementation plan before coding", "input": {"hint": "prompt"}},
            {"name": "model", "kind": "builtin", "aliases": ["models"], "description": "Select the AI model for this session (use 'auto' to let Copilot pick automatically). Use /config model to set the user default.", "input": {"hint": "model"}},
        ]})
    }

    #[test]
    fn the_menu_offers_the_listed_commands_radar_runs_with_one_line_each_and_their_hint() {
        let list = parse_list(&listed());
        assert_eq!(list.len(), 5);
        assert_eq!(
            advertised(&list),
            vec![
                SlashCommand::new("env", "Show loaded environment details (instructions, MCP servers, skills)."),
                SlashCommand::new("plan", "Create an implementation plan before coding").with_hint("prompt"),
                SlashCommand::new("model", "Select the AI model for this session (use 'auto' to let Copilot pick automatically).").with_hint("model"),
            ]
        );
    }

    #[test]
    fn a_command_is_found_by_name_or_alias_whatever_the_case() {
        let list = parse_list(&listed());
        let found = |w: &str| list.iter().find(|c| c.answers_to(w)).map(|c| c.name.as_str());
        assert_eq!(found("MODELS"), Some("model"));
        assert_eq!(found("yolo"), Some("allow-all"));
        assert_eq!(found("nonesuch"), None);
    }

    #[test]
    fn what_radar_will_not_run_says_why_and_what_it_runs_says_nothing() {
        assert!(why_not("env").is_none());
        assert!(why_not("allow-all").unwrap().contains("operator answers"));
        assert!(why_not("share").unwrap().contains("locally"));
        assert!(why_not("mcp").unwrap().contains("terminal tab"));
    }

    #[test]
    fn the_four_answers_the_cli_gave() {
        assert_eq!(parse_invoked(&json!({"kind": "text", "markdown": true, "text": "Allowed directories"})), Invoked::Text("Allowed directories".into()));
        assert_eq!(
            parse_invoked(&json!({"kind": "agent-prompt", "displayPrompt": "add a readme", "mode": "plan", "prompt": "[[PLAN]] ... add a readme"})),
            Invoked::Prompt { prompt: "[[PLAN]] ... add a readme".into(), display: "add a readme".into(), plan: true }
        );
        assert_eq!(
            parse_invoked(&json!({"kind": "agent-prompt", "displayPrompt": "Review: my code changes", "prompt": "The user has requested a code review"})),
            Invoked::Prompt { prompt: "The user has requested a code review".into(), display: "Review: my code changes".into(), plan: false }
        );
        assert_eq!(parse_invoked(&json!({"kind": "completed"})), Invoked::Done(None));
        assert_eq!(parse_invoked(&json!({"kind": "completed", "message": "Model set"})), Invoked::Done(Some("Model set".into())));
        assert_eq!(
            parse_invoked(&json!({"kind": "select-subcommand", "command": "chronicle", "title": "Chronicle", "options": [{"name": "standup", "description": "Report on the last day", "group": "Insights"}]})),
            Invoked::Choose { title: "Chronicle".into(), options: vec![("standup".into(), "Report on the last day".into())] }
        );
        assert!(matches!(parse_invoked(&json!({"kind": "show-dialog"})), Invoked::Text(t) if t.contains("show-dialog")));
    }
}
