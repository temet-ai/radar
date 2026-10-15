//! What Settings → Agents saves in radar's config (the `[sessions]` table): the
//! defaults every new session starts from, per workspace. Nothing here is a
//! credential; [`looks_like_token`] refuses a value that looks like one, so a
//! token pasted by mistake never reaches the file.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The providers radar drives natively. A configured PTY CLI has its own id.
pub const PROVIDERS: [&str; 3] = ["copilot", "codex", "claude"];

/// What a standing rule applies to; Copilot's `kind(argument)` form.
pub const RULE_KINDS: [&str; 5] = ["shell", "read", "write", "url", "mcp"];

/// A standing allow or deny rule: `kind(argument)`, as Copilot writes them
/// (`shell(git diff)`, `read(src/**)`, `url(github.com)`). Each adapter maps
/// it onto its provider's own permission form.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Rule {
    pub kind: String,
    pub argument: String,
}

impl Rule {
    pub fn parse(text: &str) -> Result<Rule, RuleError> {
        let text = text.trim();
        let (kind, rest) = text.split_once('(').ok_or(RuleError::Shape)?;
        let argument = rest.strip_suffix(')').ok_or(RuleError::Shape)?.trim();
        let kind = kind.trim().to_lowercase();
        if !RULE_KINDS.contains(&kind.as_str()) {
            return Err(RuleError::Kind(kind));
        }
        if argument.is_empty() {
            return Err(RuleError::Empty);
        }
        if looks_like_token(argument) {
            return Err(RuleError::Secret);
        }
        Ok(Rule { kind, argument: argument.to_string() })
    }
}

/// `*` matches any run of characters (also across `/`); the rest is literal.
fn glob(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !text.starts_with(first) || text.len() < first.len() + last.len() || !text.ends_with(last) {
        return false;
    }
    let mut at = first.len();
    let end = text.len() - last.len();
    for mid in &parts[1..parts.len() - 1] {
        match text[at..end].find(mid) {
            Some(i) => at += i + mid.len(),
            None => return false,
        }
    }
    true
}

/// The command a provider wraps in a login shell: Codex asks to run
/// `/usr/bin/zsh -lc ls`; a rule is written against `ls`.
fn unwrap_shell(command: &str) -> &str {
    let c = command.trim();
    let mut words = c.splitn(3, ' ');
    let (prog, flag, rest) = (words.next().unwrap_or(""), words.next().unwrap_or(""), words.next());
    let name = prog.rsplit('/').next().unwrap_or("");
    if matches!(name, "sh" | "bash" | "zsh" | "dash" | "fish")
        && matches!(flag, "-c" | "-lc" | "-ic" | "-l")
        && let Some(rest) = rest
    {
        let rest = rest.trim();
        for q in ['\'', '"'] {
            if let Some(inner) = rest.strip_prefix(q).and_then(|r| r.strip_suffix(q)) {
                return inner.trim();
            }
        }
        return rest;
    }
    c
}

impl Rule {
    /// Whether a permission request is exactly what this rule names: a shell
    /// rule matches a command (every segment of it) equal to the argument or
    /// starting with it as whole words; read, write and url rules match the
    /// path or address with `*`; mcp matches `server` or `server/tool`.
    pub fn matches(&self, request: &crate::event::PermissionRequest) -> bool {
        use crate::event::PermissionTarget as T;
        let a = self.argument.as_str();
        match (self.kind.as_str(), &request.target) {
            ("shell", T::Command { command, .. }) => {
                let c = unwrap_shell(command);
                // The whole command must match, never one piece of a pipeline.
                let plain = !c.contains(['|', ';', '&', '`', '\n', '>', '<', '$']);
                plain && (c == a || (c.starts_with(a) && c[a.len()..].starts_with(' ')) || (a.contains('*') && glob(a, c)))
            }
            ("read", T::Path { path }) => glob(a, path),
            ("write", T::Write { path, .. }) => glob(a, path),
            ("url", T::Url { url }) => {
                let host = url.split("://").last().unwrap_or(url).split('/').next().unwrap_or("");
                glob(a, url) || glob(a, host)
            }
            ("mcp", T::Mcp { server, tool, .. }) => a == server || a == format!("{server}/{tool}") || glob(a, &format!("{server}/{tool}")),
            _ => false,
        }
    }
}

impl Defaults {
    /// The standing rule that decides `request`, deny before allow.
    pub fn standing(&self, request: &crate::event::PermissionRequest) -> Option<(bool, &Rule)> {
        if let Some(r) = self.deny.iter().find(|r| r.matches(request)) {
            return Some((false, r));
        }
        self.allow.iter().find(|r| r.matches(request)).map(|r| (true, r))
    }
}

impl std::fmt::Display for Rule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}({})", self.kind, self.argument)
    }
}

impl TryFrom<String> for Rule {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        Rule::parse(&s).map_err(|e| e.words())
    }
}

impl From<Rule> for String {
    fn from(r: Rule) -> String {
        r.to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleError {
    /// Not `kind(argument)`.
    Shape,
    Kind(String),
    Empty,
    Secret,
}

impl RuleError {
    pub fn words(&self) -> String {
        match self {
            RuleError::Shape => "write a rule as kind(argument), like shell(git diff)".into(),
            RuleError::Kind(k) => format!("{k:?} is not a kind: use {}", RULE_KINDS.join(", ")),
            RuleError::Empty => "the rule needs an argument inside the brackets".into(),
            RuleError::Secret => "that looks like a token; radar never stores one".into(),
        }
    }
}

/// The defaults of one workspace (or of every workspace with no entry of its own).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Defaults {
    /// A provider id: copilot, codex, claude, or a configured PTY CLI.
    pub provider: String,
    /// The model a new session asks for; empty is the provider's own default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Folders a session may read and write under besides its working
    /// directory. Empty: its working directory only.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub permitted_roots: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub allow: Vec<Rule>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub deny: Vec<Rule>,
    /// Sessions run on this machine only: no remote or delegated runs.
    pub local_only: bool,
    /// A reviewer session may ask to run commands (each one approved in the
    /// dock). Off: it only reads and runs git diff, log and show.
    pub reviewer_may_run_commands: bool,
    /// Skill folders (`<name>/SKILL.md`) every provider loads, besides its own.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skill_dirs: Vec<String>,
    /// Each CLI loads its own configuration of the user and the project:
    /// AGENTS.md or CLAUDE.md, its agents, skills, MCP servers and hooks.
    pub project_config: bool,
    /// Where a session started from the dock works. Empty: the folder radar was
    /// started in. A session for a change or a review works in its own checkout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub work_dir: Option<String>,
}

impl Default for Defaults {
    fn default() -> Self {
        Defaults {
            provider: "copilot".into(),
            model: None,
            permitted_roots: vec![],
            allow: vec![],
            deny: vec![],
            local_only: true,
            reviewer_may_run_commands: true,
            skill_dirs: vec![],
            project_config: true,
            work_dir: None,
        }
    }
}

/// One provider's own settings (`[sessions.providers.<id>]`): where its CLI
/// is and keeps its state, the agent it starts as, and the folders it loads
/// besides the shared ones.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderSettings {
    /// The executable, when it is not on PATH.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable: Option<String>,
    /// The CLI's own config folder (`COPILOT_HOME`, `CODEX_HOME`, `CLAUDE_CONFIG_DIR`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_dir: Option<String>,
    /// The agent a new session starts as; empty: the CLI's own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skill_dirs: Vec<String>,
    /// Folders of instruction files (AGENTS.md and the like).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub instruction_dirs: Vec<String>,
    /// Plugin folders, in this CLI's own plugin format.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub plugin_dirs: Vec<String>,
}

/// The kinds of folder list a provider has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderKind {
    Skills,
    Instructions,
    Plugins,
}

impl ProviderSettings {
    pub fn folders(&self, kind: FolderKind) -> &Vec<String> {
        match kind {
            FolderKind::Skills => &self.skill_dirs,
            FolderKind::Instructions => &self.instruction_dirs,
            FolderKind::Plugins => &self.plugin_dirs,
        }
    }
    pub fn folders_mut(&mut self, kind: FolderKind) -> &mut Vec<String> {
        match kind {
            FolderKind::Skills => &mut self.skill_dirs,
            FolderKind::Instructions => &mut self.instruction_dirs,
            FolderKind::Plugins => &mut self.plugin_dirs,
        }
    }
}

/// A path as written in the config, with a leading `~` for the home folder;
/// None when it is not absolute after that.
pub fn expand(path: &str) -> Option<std::path::PathBuf> {
    let path = path.trim();
    let p = match path.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            std::path::PathBuf::from(std::env::var_os("HOME")?).join(rest.trim_start_matches('/'))
        }
        _ => std::path::PathBuf::from(path),
    };
    p.is_absolute().then_some(p)
}

/// A CLI radar runs in a PTY tab and has no native adapter for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PtyCli {
    pub id: String,
    pub name: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Its own login command, run in a PTY tab from Settings → Agents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentsConfig {
    #[serde(flatten)]
    pub defaults: Defaults,
    /// Per workspace, by its name; a workspace without an entry uses `defaults`.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub workspaces: BTreeMap<String, Defaults>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pty: Vec<PtyCli>,
    /// Where a provider's executable is, when it is not on PATH. Kept for
    /// files written before `[sessions.providers]`; that table's
    /// `executable` wins.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub executables: BTreeMap<String, String>,
    /// Each provider's own settings, by provider id.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub providers: BTreeMap<String, ProviderSettings>,
    /// A folder of your own recipes (`<name>.md`); one named like a built-in
    /// recipe replaces it. Empty: the built-in recipes only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipes_dir: Option<String>,
    /// Where local reviews are saved when no work item takes them. Empty:
    /// `$XDG_STATE_HOME/radar/reviews`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviews_dir: Option<String>,
    /// A desktop notification when a session needs you (an approval, a
    /// question) or its turn ends while its tab is out of sight. Off unless
    /// asked for.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub notify: bool,
}

impl AgentsConfig {
    pub fn is_default(&self) -> bool {
        *self == AgentsConfig::default()
    }

    /// What a new session in `workspace` starts from.
    pub fn for_workspace(&self, workspace: &str) -> &Defaults {
        self.workspaces.get(workspace).unwrap_or(&self.defaults)
    }

    /// The defaults being edited: the workspace's own entry (made from the
    /// shared defaults on first edit) or the shared ones.
    pub fn editing(&mut self, workspace: Option<&str>) -> &mut Defaults {
        match workspace {
            Some(w) => self.workspaces.entry(w.to_string()).or_insert_with(|| self.defaults.clone()),
            None => &mut self.defaults,
        }
    }

    /// A provider's own settings; the defaults when it has none.
    pub fn provider(&self, id: &str) -> ProviderSettings {
        self.providers.get(id).cloned().unwrap_or_default()
    }

    /// Where `provider`'s CLI is and keeps its state. A terminal tab of a
    /// native CLI (`pty:codex`) runs the same executable as `codex`.
    pub fn provider_setup(&self, provider: &str) -> crate::adapter::ProviderSetup {
        let native = provider.strip_prefix("pty:").unwrap_or(provider);
        let own = self.provider(native);
        crate::adapter::ProviderSetup {
            executable: own
                .executable
                .as_deref()
                .or(self.executables.get(native).map(String::as_str))
                .and_then(expand),
            config_dir: own.config_dir.as_deref().and_then(expand),
        }
    }

    /// What a new or resumed session of `provider` in `workspace` is set up
    /// with: the workspace's defaults and the provider's own folders. A path
    /// that is not absolute is left out.
    pub fn session_setup(&self, workspace: Option<&str>, provider: &str) -> crate::adapter::SessionSetup {
        let d = workspace.map_or(&self.defaults, |w| self.for_workspace(w));
        let p = self.provider(provider);
        let paths = |list: &[String]| list.iter().filter_map(|s| expand(s)).collect::<Vec<_>>();
        let mut skill_dirs = paths(&d.skill_dirs);
        skill_dirs.extend(paths(&p.skill_dirs));
        crate::adapter::SessionSetup {
            extra_dirs: paths(&d.permitted_roots),
            agent: p.agent.clone().filter(|a| !a.trim().is_empty()),
            skill_dirs,
            skills: vec![],
            instruction_dirs: paths(&p.instruction_dirs),
            plugin_dirs: paths(&p.plugin_dirs),
            project_config: d.project_config,
            local_only: d.local_only,
            review: false,
            // Set per session by the part when it starts the session's listener, never from settings.
            tool_server: None,
        }
    }

    pub fn from_table(table: Option<&toml::Value>) -> anyhow::Result<AgentsConfig> {
        let mut cfg: AgentsConfig = match table {
            Some(t) => t.clone().try_into()?,
            None => AgentsConfig::default(),
        };
        cfg.sanitize();
        Ok(cfg)
    }

    pub fn to_table(&self) -> Option<toml::Value> {
        (!self.is_default()).then(|| toml::Value::try_from(self).ok()).flatten()
    }

    /// Drop whatever looks like a credential, from a file edited by hand.
    fn sanitize(&mut self) {
        let clean = |d: &mut Defaults| {
            if d.model.as_deref().is_some_and(looks_like_token) {
                d.model = None;
            }
            d.permitted_roots.retain(|r| !looks_like_token(r));
            d.skill_dirs.retain(|r| !looks_like_token(r));
        };
        clean(&mut self.defaults);
        self.workspaces.values_mut().for_each(clean);
        self.executables.retain(|_, v| !looks_like_token(v));
        for p in self.providers.values_mut() {
            for v in [&mut p.executable, &mut p.config_dir, &mut p.agent] {
                if v.as_deref().is_some_and(looks_like_token) {
                    *v = None;
                }
            }
            for kind in [FolderKind::Skills, FolderKind::Instructions, FolderKind::Plugins] {
                p.folders_mut(kind).retain(|r| !looks_like_token(r));
            }
        }
        for v in [&mut self.recipes_dir, &mut self.reviews_dir] {
            if v.as_deref().is_some_and(looks_like_token) {
                *v = None;
            }
        }
    }
}

/// Whether `s` has the shape of a credential: a known prefix, a JWT, or a long
/// unbroken run of key characters. Paths, models and rules never match.
/// Whether `s`, or any word inside it (a path piece, the inside of
/// `kind(argument)`), looks like a credential.
pub fn contains_token(s: &str) -> bool {
    looks_like_token(s)
        || s.split(['(', ')', '/', ' ', ',', ':', '"', '\'', '=']).any(looks_like_token)
}

pub fn looks_like_token(s: &str) -> bool {
    const PREFIXES: [&str; 12] = [
        "ghp_", "gho_", "ghu_", "ghs_", "github_pat_", "glpat-", "sk-", "xoxp-", "xoxb-", "xapp-", "AKIA", "Bearer ",
    ];
    let s = s.trim();
    if PREFIXES.iter().any(|p| s.starts_with(p)) {
        return true;
    }
    if s.starts_with("eyJ") && s.matches('.').count() == 2 {
        return true;
    }
    // A single word of 32+ key characters with no path or rule punctuation.
    s.len() >= 32
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '+' | '='))
        && s.chars().any(|c| c.is_ascii_digit())
        && s.chars().any(|c| c.is_ascii_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shell_rule_holds_for_a_command_wrapped_in_a_login_shell() {
        use crate::event::{PermissionKind, PermissionRequest, PermissionTarget};
        let ask = |c: &str| PermissionRequest {
            request_id: "r".into(),
            kind: PermissionKind::Shell,
            target: PermissionTarget::Command { command: c.into(), segments: vec![] },
            tool_call_id: None,
            can_offer_session_approval: false,
        };
        let ls = Rule::parse("shell(ls)").unwrap();
        let cat = Rule::parse("shell(cat *)").unwrap();
        assert!(ls.matches(&ask("ls")) && ls.matches(&ask("/usr/bin/zsh -lc ls")) && ls.matches(&ask("bash -c 'ls -la'")));
        assert!(cat.matches(&ask("/usr/bin/zsh -lc 'cat src/main.rs'")));
        assert!(!ls.matches(&ask("/usr/bin/zsh -lc 'ls; rm -rf x'")) && !ls.matches(&ask("lsof")));
    }

    #[test]
    fn rules_are_kind_and_argument() {
        let r = Rule::parse(" Shell(git diff) ").unwrap();
        assert_eq!((r.kind.as_str(), r.argument.as_str()), ("shell", "git diff"));
        assert_eq!(r.to_string(), "shell(git diff)");
        assert_eq!(Rule::parse("git diff"), Err(RuleError::Shape));
        assert_eq!(Rule::parse("shell()"), Err(RuleError::Empty));
        assert!(matches!(Rule::parse("exec(ls)"), Err(RuleError::Kind(_))));
        assert_eq!(Rule::parse("shell(ghp_abcdefghijklmnopqrstuvwxyz0123456789)"), Err(RuleError::Secret));
    }

    #[test]
    fn the_default_config_writes_nothing() {
        assert!(AgentsConfig::default().to_table().is_none());
        assert!(AgentsConfig::default().defaults.local_only);
        assert!(AgentsConfig::default().defaults.reviewer_may_run_commands);
    }

    #[test]
    fn a_config_round_trips_per_workspace() {
        let mut cfg = AgentsConfig::default();
        let d = cfg.editing(None);
        d.provider = "codex".into();
        d.model = Some("gpt-5".into());
        d.allow.push(Rule::parse("shell(git diff)").unwrap());
        d.deny.push(Rule::parse("url(example.com)").unwrap());
        let w = cfg.editing(Some("work"));
        w.local_only = false;
        w.permitted_roots.push("/srv/shared".into());
        cfg.pty.push(PtyCli { id: "aider".into(), name: "Aider".into(), command: "aider".into(), args: vec![], login: None });
        let table = cfg.to_table().unwrap();
        let back = AgentsConfig::from_table(Some(&table)).unwrap();
        assert_eq!(back, cfg);
        assert_eq!(back.for_workspace("work").permitted_roots, ["/srv/shared"]);
        assert!(!back.for_workspace("work").local_only);
        assert_eq!(back.for_workspace("other").provider, "codex");
    }

    #[test]
    fn a_bad_rule_in_the_file_is_an_error_not_a_silent_drop() {
        let table: toml::Value = toml::from_str("allow = [\"nonsense\"]").unwrap();
        assert!(AgentsConfig::from_table(Some(&table)).is_err());
    }

    #[test]
    fn credentials_are_not_kept() {
        let table: toml::Value =
            toml::from_str("model = \"ghp_abcdefghijklmnopqrstuvwxyz0123456789\"\npermitted_roots = [\"/srv/ok\", \"sk-abcdefghijklmnop\"]").unwrap();
        let cfg = AgentsConfig::from_table(Some(&table)).unwrap();
        assert_eq!(cfg.defaults.model, None);
        assert_eq!(cfg.defaults.permitted_roots, ["/srv/ok"]);
    }

    #[test]
    fn token_shapes_are_told_from_paths_and_models() {
        for ok in ["gpt-5.1-codex", "claude-opus-4-1", "/home/me/work/some-long-project-name-here", "copilot", "shell(git log --oneline)"] {
            assert!(!looks_like_token(ok), "{ok}");
        }
        for bad in ["github_pat_11AAAA", "xoxp-1234-5678", "eyJhbGciOi.eyJzdWIi.sig", "Bearer abc", "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6"] {
            assert!(looks_like_token(bad), "{bad}");
        }
    }
}
