//! Icons: one semantic set, drawn from a pack.
//!
//! A part never writes a glyph. It names what it means ([`Icon::Bug`],
//! [`Icon::RunFailed`]) and the kit draws that name from the pack the user
//! chose, in the icon's colour:
//!
//! ```ignore
//! let mut name = vec![icon::lead(theme, Icon::Repo), Span::raw("api")];
//! ```
//!
//! **Packs.** A pack maps names to glyphs, each with an optional style spec,
//! and declares how many cells a glyph takes (`width`, 1 or 2). Built in:
//! `nerd` (Nerd Font symbols, one cell), `unicode` (plain symbols every font
//! has, one cell, never emoji), `emoji` (two cells) and `off` (no icon and no
//! gap: the screens look as they do without icons). A user's packs are TOML
//! files in `<config dir>/radar/icons/<name>.toml` ([`load_dir`]); a pack
//! `extends` another, and every name it leaves out comes from that one, and
//! in the end from `unicode`, so a partial pack always works. Overrides
//! under `[icons]` in config.toml go on top of the chosen pack.
//!
//! **Choosing.** `[theme] icons = "auto" | <pack>`. `auto` is `nerd` where
//! the terminal is known to ship or commonly use Nerd Font symbols (Ghostty,
//! kitty, WezTerm, also inside a multiplexer that keeps their variables),
//! else `unicode`; over SSH `unicode`. Until the runner says what `auto` is
//! here ([`set_auto`]), `auto` is `off`: tests and snapshots draw no icons
//! unless they ask for a pack.
//!
//! **Width.** A glyph wider than its pack's width is refused (with a
//! warning, the next pack's glyph is used); a narrower one is padded, so
//! every icon of a pack takes the same cells and columns stay aligned.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ratatui::text::Span;
use serde::{Deserialize, Serialize};

use crate::theme::Theme;

/// What an icon is defined with: its name in packs and config, the heading
/// it is listed under, its default style, its glyph in each built-in pack,
/// and what it means.
struct Def {
    name: &'static str,
    group: &'static str,
    style: &'static str,
    nerd: &'static str,
    unicode: &'static str,
    emoji: &'static str,
    about: &'static str,
}

macro_rules! icons {
    ($( $v:ident $name:literal $group:literal $style:literal $nerd:literal $uni:literal $emoji:literal $about:literal ;)*) => {
        /// A semantic icon: what it means, never what it looks like.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum Icon { $($v),* }

        impl Icon {
            /// Every icon, in the order the icons section lists them.
            pub const ALL: &'static [Icon] = &[$(Icon::$v),*];
        }

        const DEFS: &[Def] = &[$(Def {
            name: $name,
            group: $group,
            style: $style,
            nerd: $nerd,
            unicode: $uni,
            emoji: $emoji,
            about: $about,
        }),*];
    };
}

icons! {
    // Code: groups, repositories and the changes asked to be merged.
    Group "group" "code" "accent.primary" "\u{f42b}" "◆" "🏢" "a group of repositories, a tenancy";
    Repo "repo" "code" "accent.secondary" "\u{f401}" "▤" "📦" "a repository";
    Branch "branch" "code" "accent.secondary" "\u{f418}" "⎇" "🌿" "a branch";
    Commit "commit" "code" "text.muted" "\u{f417}" "◦" "🔘" "a commit";
    Change "change" "code" "accent.primary" "\u{f407}" "⇄" "🔀" "a change asked to be merged, open";
    Draft "draft" "code" "text.muted" "\u{f4dd}" "◌" "📝" "a change still in draft";
    Merged "merged" "code" "glow.purple" "\u{f419}" "◈" "🟣" "a change merged";
    Closed "closed" "code" "status.error" "\u{f4dc}" "⊘" "⛔" "a change closed without merging";
    Approved "approved" "code" "status.success" "\u{f496}" "✓" "👍" "approved";
    NeedsApproval "needs_approval" "code" "status.warning" "\u{f441}" "◎" "👀" "waiting for approval, to review";
    Thread "thread" "code" "accent.secondary" "\u{f442}" "❝" "💬" "a discussion, its threads";
    Comment "comment" "code" "text.secondary" "\u{f41f}" "✎" "💭" "a comment, write one";
    Resolved "resolved" "code" "status.success" "\u{f42e}" "✓" "🆗" "a thread resolved";
    Conflict "conflict" "code" "status.error" "\u{f421}" "≠" "💥" "conflicts to fix";
    Rebase "rebase" "code" "status.warning" "\u{f46a}" "↻" "🔁" "needs a rebase";
    Label "label" "code" "accent.tertiary" "\u{f412}" "⌗" "🔖" "a label";
    // Runs: pipelines and their jobs.
    Pipeline "pipeline" "runs" "status.info" "\u{f52e}" "⋮" "🚦" "a pipeline, a run of jobs";
    RunPassed "run_passed" "runs" "status.success" "\u{f49e}" "✓" "✅" "passed";
    RunFailed "run_failed" "runs" "status.error" "\u{f52f}" "✗" "❌" "failed";
    RunRunning "run_running" "runs" "status.info" "\u{f192}" "●" "🔄" "running";
    RunPending "run_pending" "runs" "text.muted" "\u{f43a}" "◷" "⏳" "waiting to run";
    RunSkipped "run_skipped" "runs" "text.muted" "\u{f4e7}" "»" "⏩" "skipped";
    RunManual "run_manual" "runs" "accent.primary" "\u{f04b}" "▷" "👆" "waits to be started by hand";
    RunCanceled "run_canceled" "runs" "text.muted" "\u{f05e}" "⊘" "🚫" "canceled";
    Job "job" "runs" "status.info" "\u{f013}" "▸" "👷" "a job";
    Log "log" "runs" "text.muted" "\u{f4ed}" "≣" "📜" "a log";
    // Work items: their kinds, priorities and states.
    Issue "issue" "work" "accent.secondary" "\u{f41b}" "◇" "📋" "a work item of any kind";
    Epic "epic" "work" "glow.purple" "\u{f0e7}" "✦" "⚡" "an epic";
    Story "story" "work" "status.success" "\u{f02e}" "▣" "📗" "a story";
    Task "task" "work" "status.info" "\u{f4a7}" "☐" "📘" "a task";
    Bug "bug" "work" "status.error" "\u{f46f}" "✱" "🐛" "a bug";
    Subtask "subtask" "work" "status.info" "\u{f4a0}" "↳" "🔹" "a sub-task";
    PriorityHighest "priority_highest" "work" "status.error" "\u{f102}" "⇈" "⏫" "highest priority";
    PriorityHigh "priority_high" "work" "status.error" "\u{f106}" "↑" "🔼" "high priority";
    PriorityMedium "priority_medium" "work" "status.warning" "\u{f48b}" "=" "🟡" "medium priority";
    PriorityLow "priority_low" "work" "status.info" "\u{f107}" "↓" "🔽" "low priority";
    PriorityLowest "priority_lowest" "work" "status.info" "\u{f103}" "⇊" "⏬" "lowest priority";
    Todo "todo" "work" "text.muted" "\u{f4aa}" "○" "⚪" "to do";
    InProgress "in_progress" "work" "status.info" "\u{f042}" "◐" "🔵" "in progress";
    Done "done" "work" "status.success" "\u{f058}" "●" "🟢" "done";
    Site "site" "work" "accent.secondary" "\u{f0ac}" "⊕" "🌐" "a site, a server";
    Board "board" "work" "accent.secondary" "\u{f0ae}" "⊟" "📊" "a board, a list of work";
    // People.
    Person "person" "people" "text.secondary" "\u{f415}" "☺" "👤" "a person";
    You "you" "people" "accent.primary" "\u{f007}" "☻" "🙋" "you";
    Bot "bot" "people" "text.muted" "\u{f06a9}" "◙" "🤖" "a bot";
    Team "team" "people" "text.secondary" "\u{f0c0}" "⚇" "👥" "a team, several people";
    // Chat.
    Chat "chat" "chat" "accent.secondary" "\u{f086}" "✉" "💬" "conversations";
    Channel "channel" "chat" "text.secondary" "\u{f292}" "#" "📢" "a channel";
    PrivateChannel "private_channel" "chat" "status.warning" "\u{f023}" "⊡" "🔒" "a private channel";
    Direct "direct" "chat" "accent.primary" "\u{f0e0}" "✉" "📩" "a direct message";
    Reply "reply" "chat" "accent.secondary" "\u{f112}" "↩" "🧵" "a reply in a thread";
    Mention "mention" "chat" "accent.primary" "\u{f486}" "@" "📣" "you are mentioned";
    Reaction "reaction" "chat" "status.warning" "\u{f4a2}" "♥" "😀" "reactions";
    Unread "unread" "chat" "accent.primary" "\u{f444}" "•" "🆕" "unread";
    Alert "alert" "chat" "status.warning" "\u{f49a}" "▲" "🔔" "alerts";
    AlertFiring "alert_firing" "chat" "status.error" "\u{f490}" "▲" "🔥" "an alert firing";
    AlertResolved "alert_resolved" "chat" "status.success" "\u{f478}" "△" "💚" "an alert resolved";
    // The app: places, actions, states of a panel.
    Settings "settings" "app" "text.secondary" "\u{f423}" "⚙" "🔧" "settings";
    Action "action" "app" "accent.secondary" "\u{f04b}" "▷" "⚡" "an available action";
    Copy "copy" "app" "accent.secondary" "\u{f0c5}" "⧉" "📋" "copy to the clipboard";
    Link "link" "app" "accent.secondary" "\u{f44c}" "↗" "🔗" "a link";
    Search "search" "app" "accent.secondary" "\u{f422}" "⊙" "🔍" "search, a filter";
    Time "time" "app" "text.muted" "\u{f43a}" "◷" "🕒" "a time, how long";
    Updated "updated" "app" "text.muted" "\u{f1da}" "↺" "🔃" "when it last changed";
    Calendar "calendar" "app" "text.muted" "\u{f455}" "▦" "📅" "a date";
    Info "info" "app" "status.info" "\u{f449}" "ℹ" "💡" "information";
    Warning "warning" "app" "status.warning" "\u{f421}" "⚠" "🚧" "a warning";
    Error "error" "app" "status.error" "\u{f46e}" "✗" "🛑" "an error";
    Empty "empty" "app" "text.muted" "\u{f48d}" "∅" "📭" "nothing here";
    Menu "menu" "app" "text.muted" "\u{f0c9}" "≡" "📋" "the actions menu";
    Keys "keys" "app" "text.muted" "\u{f11c}" "⌘" "🎹" "keys";
    Quit "quit" "app" "text.muted" "\u{f08b}" "✕" "🚪" "quit";
    Close "close" "app" "text.muted" "\u{f467}" "×" "❌" "close a tab";
    Navigate "navigate" "app" "text.muted" "\u{f047}" "↔" "🧭" "move about";
    Scroll "scroll" "app" "text.muted" "\u{f07d}" "↕" "📜" "scroll";
    Bulk "bulk" "app" "text.muted" "\u{f03a}" "◫" "📚" "everything shown at once";
    Fold "fold" "app" "text.muted" "\u{f196}" "±" "🔽" "groups expand and collapse";
    Colours "colours" "app" "accent.primary" "\u{f1fc}" "◑" "🎨" "colours and the theme";
    Icons "icons" "app" "status.warning" "\u{f118}" "☺" "😀" "icons";
    Parts "parts" "app" "accent.secondary" "\u{f12e}" "⊞" "🧩" "the parts in this build";
    Connection "connection" "app" "status.success" "\u{f1e6}" "⇌" "🔌" "a connection, signing in";
    Views "views" "app" "accent.secondary" "\u{f0b0}" "▽" "👓" "views, what a list shows";
    Look "look" "app" "accent.primary" "\u{f4c5}" "◑" "💅" "the look of words";
    Local "local" "app" "text.secondary" "\u{f109}" "⌂" "💻" "this machine, a checkout";
    Editor "editor" "app" "accent.secondary" "\u{f121}" "✎" "📝" "an editor";
    Terminal "terminal" "app" "text.secondary" "\u{f489}" "$" "🐚" "a terminal";
    ModeRead "mode_read" "app" "status.info" "\u{f06e}" "◉" "🔒" "read only: radar changes nothing";
    ModeWrite "mode_write" "app" "accent.secondary" "\u{f040}" "✎" "📝" "write: radar may add things";
    ModeEdit "mode_edit" "app" "accent.primary" "\u{f044}" "✎" "🔨" "edit: radar may add and change things";
    // Files, by type.
    File "file" "files" "text.secondary" "\u{f4a5}" "▫" "📄" "a file";
    Folder "folder" "files" "accent.secondary" "\u{f413}" "▪" "📁" "a folder";
    FolderOpen "folder_open" "files" "accent.secondary" "\u{f07c}" "▭" "📂" "an open folder";
    FileRust "file_rust" "files" "status.warning" "\u{e7a8}" "▫" "🦀" "Rust";
    FileJava "file_java" "files" "status.error" "\u{e738}" "▫" "☕" "Java";
    FileKotlin "file_kotlin" "files" "glow.purple" "\u{e634}" "▫" "🎯" "Kotlin";
    FileTypescript "file_typescript" "files" "status.info" "\u{e628}" "▫" "🔷" "TypeScript";
    FileJavascript "file_javascript" "files" "status.warning" "\u{e74e}" "▫" "🟨" "JavaScript";
    FilePython "file_python" "files" "status.info" "\u{e606}" "▫" "🐍" "Python";
    FileGo "file_go" "files" "glow.cyan" "\u{e627}" "▫" "🐹" "Go";
    FileMarkdown "file_markdown" "files" "text.secondary" "\u{e73e}" "▫" "📝" "Markdown";
    FileJson "file_json" "files" "status.warning" "\u{e60b}" "▫" "🧾" "JSON";
    FileYaml "file_yaml" "files" "status.error" "\u{e8eb}" "▫" "📒" "YAML";
    FileToml "file_toml" "files" "text.secondary" "\u{e6b2}" "▫" "📙" "TOML";
    FileDocker "file_docker" "files" "status.info" "\u{f308}" "▫" "🐳" "a Dockerfile";
    FileSql "file_sql" "files" "glow.pink" "\u{f1c0}" "▫" "💾" "SQL";
    FileHtml "file_html" "files" "status.error" "\u{e736}" "▫" "🌐" "HTML";
    FileCss "file_css" "files" "status.info" "\u{e749}" "▫" "🎨" "CSS";
    FileShell "file_shell" "files" "status.success" "\u{e795}" "▫" "🐚" "a shell script";
    FileC "file_c" "files" "status.info" "\u{e61e}" "▫" "🔤" "C";
    FileCpp "file_cpp" "files" "status.info" "\u{e61d}" "▫" "➕" "C++";
    FileRuby "file_ruby" "files" "status.error" "\u{e739}" "▫" "💎" "Ruby";
    FilePhp "file_php" "files" "glow.purple" "\u{e73d}" "▫" "🐘" "PHP";
    FileSwift "file_swift" "files" "status.warning" "\u{e755}" "▫" "🐦" "Swift";
    FileImage "file_image" "files" "glow.pink" "\u{f1c5}" "▫" "📷" "an image";
    FileGit "file_git" "files" "status.warning" "\u{e702}" "▫" "🌱" "git's own files";
    FileLock "file_lock" "files" "text.muted" "\u{f023}" "▫" "🔒" "a lock file";
    FileXml "file_xml" "files" "status.warning" "\u{f1c9}" "▫" "📰" "XML";
    FileText "file_text" "files" "text.secondary" "\u{f15c}" "▫" "📃" "plain text";
}

impl Icon {
    fn def(self) -> &'static Def {
        &DEFS[self as usize]
    }

    /// Its name in packs and under `[icons]`: `run_failed`.
    pub fn name(self) -> &'static str {
        self.def().name
    }

    /// What it means, for people.
    pub fn about(self) -> &'static str {
        self.def().about
    }

    /// The heading it is listed under.
    pub fn group(self) -> &'static str {
        self.def().group
    }

    /// The style it is drawn in when the pack does not say.
    pub fn style(self) -> &'static str {
        self.def().style
    }

    pub fn from_name(name: &str) -> Option<Icon> {
        DEFS.iter().position(|d| d.name == name).map(|i| Icon::ALL[i])
    }

    /// The icon for a file, by its name: its extension, or a name that is
    /// its type (`Dockerfile`, `Cargo.lock`).
    pub fn for_file(path: &str) -> Icon {
        let name = path.rsplit('/').next().unwrap_or(path);
        let lower = name.to_ascii_lowercase();
        if lower == "dockerfile" || lower.starts_with("dockerfile.") || lower.ends_with(".dockerfile") {
            return Icon::FileDocker;
        }
        if lower.ends_with(".lock") || lower == "package-lock.json" {
            return Icon::FileLock;
        }
        if lower.starts_with(".git") {
            return Icon::FileGit;
        }
        if lower == "makefile" || lower == "justfile" {
            return Icon::FileShell;
        }
        let ext = lower.rsplit_once('.').map_or("", |(_, e)| e);
        match ext {
            "rs" => Icon::FileRust,
            "java" => Icon::FileJava,
            "kt" | "kts" => Icon::FileKotlin,
            "ts" | "tsx" | "mts" | "cts" => Icon::FileTypescript,
            "js" | "jsx" | "mjs" | "cjs" => Icon::FileJavascript,
            "py" | "pyi" => Icon::FilePython,
            "go" => Icon::FileGo,
            "md" | "markdown" | "mdx" => Icon::FileMarkdown,
            "json" | "jsonc" | "json5" => Icon::FileJson,
            "yaml" | "yml" => Icon::FileYaml,
            "toml" => Icon::FileToml,
            "sql" => Icon::FileSql,
            "html" | "htm" => Icon::FileHtml,
            "css" | "scss" | "sass" | "less" => Icon::FileCss,
            "sh" | "bash" | "zsh" | "fish" => Icon::FileShell,
            "c" | "h" => Icon::FileC,
            "cpp" | "cc" | "cxx" | "hpp" | "hh" => Icon::FileCpp,
            "rb" => Icon::FileRuby,
            "php" => Icon::FilePhp,
            "swift" => Icon::FileSwift,
            "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "ico" => Icon::FileImage,
            "xml" | "gradle" => Icon::FileXml,
            "txt" | "text" | "log" => Icon::FileText,
            _ => Icon::File,
        }
    }
}

// ---------------------------------------------------------------------
// Packs.

/// One icon in a pack: its glyph and, when the pack says, its style spec
/// (`"status.error bold"`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Glyph {
    Plain(String),
    Styled {
        glyph: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        style: Option<String>,
    },
}

impl Glyph {
    pub fn new(glyph: impl Into<String>) -> Self {
        Glyph::Plain(glyph.into())
    }

    pub fn text(&self) -> &str {
        match self {
            Glyph::Plain(g) | Glyph::Styled { glyph: g, .. } => g,
        }
    }

    pub fn style(&self) -> Option<&str> {
        match self {
            Glyph::Plain(_) => None,
            Glyph::Styled { style, .. } => style.as_deref(),
        }
    }
}

/// A pack: a name, what it is, the pack it extends, how many cells each
/// glyph takes, and its glyphs by icon name.
#[derive(Debug, Clone, PartialEq)]
pub struct Pack {
    pub name: String,
    pub description: String,
    pub extends: Option<String>,
    /// Cells per glyph: 1, or 2 for emoji; `None` takes the extended pack's.
    pub width: Option<usize>,
    pub icons: BTreeMap<String, Glyph>,
    /// Where it was read from; `None` for a built-in pack.
    pub path: Option<PathBuf>,
}

/// The built-in packs, in the order a menu offers them.
pub const BUILT_IN: [&str; 4] = ["nerd", "unicode", "emoji", "off"];

/// What a built-in pack is, for people.
pub fn built_in_about(name: &str) -> &'static str {
    match name {
        "nerd" => "Nerd Font symbols: needs a Nerd Font, or a terminal that brings its symbols",
        "unicode" => "plain symbols every font has",
        "emoji" => "colourful emoji, two cells wide",
        "off" => "no icons",
        _ => "",
    }
}

impl Pack {
    /// A built-in pack by name.
    pub fn built_in(name: &str) -> Option<Pack> {
        let (width, pick): (usize, fn(&Def) -> &'static str) = match name {
            "nerd" => (1, |d| d.nerd),
            "unicode" => (1, |d| d.unicode),
            "emoji" => (2, |d| d.emoji),
            "off" => (0, |_| ""),
            _ => return None,
        };
        Some(Pack {
            name: name.into(),
            description: built_in_about(name).into(),
            extends: None,
            width: Some(width),
            icons: DEFS
                .iter()
                .map(|d| (d.name.to_string(), Glyph::new(pick(d))))
                .collect(),
            path: None,
        })
    }

    /// A pack from its file's text; `stem` names it when the file does not.
    /// What it gets wrong that can be passed over (an unknown name, a glyph
    /// too wide) comes back as warnings; what cannot is an error.
    pub fn parse(text: &str, stem: &str) -> Result<(Pack, Vec<String>), String> {
        let table: toml::Table = toml::from_str(text).map_err(|e| e.message().to_string())?;
        let mut warnings = Vec::new();
        let field = |k: &str| table.get(k).and_then(|v| v.as_str()).map(str::to_string);
        let name = field("name").unwrap_or_else(|| stem.to_string());
        let width = match table.get("width") {
            None => None,
            Some(v) => match v.as_integer() {
                Some(w @ 1..=2) => Some(w as usize),
                _ => return Err(format!("width must be 1 or 2, not {v}")),
            },
        };
        let mut icons = BTreeMap::new();
        if let Some(t) = table.get("icons") {
            let t = t.as_table().ok_or("[icons] must be a table")?;
            for (k, v) in t {
                match Glyph::deserialize(v.clone()) {
                    Ok(g) => {
                        if Icon::from_name(k).is_none() {
                            warnings.push(format!("{name}: no icon is called {k:?}"));
                        }
                        icons.insert(k.clone(), g);
                    }
                    Err(_) => warnings.push(format!(
                        "{name}: {k} must be \"glyph\" or {{ glyph = \"…\", style = \"…\" }}"
                    )),
                }
            }
        }
        for k in table.keys() {
            if !["name", "description", "extends", "width", "icons"].contains(&k.as_str()) {
                warnings.push(format!("{name}: {k:?} is not a pack setting"));
            }
        }
        Ok((
            Pack {
                name,
                description: field("description").unwrap_or_default(),
                extends: field("extends"),
                width,
                icons,
                path: None,
            },
            warnings,
        ))
    }
}

/// Whether `glyph` may be drawn in a pack whose glyphs take `width` cells:
/// at most that wide, and nothing a terminal draws at a width of its own
/// choosing (joined emoji, skin tones, flags, controls). Empty means no
/// icon for that name.
pub fn check(glyph: &str, width: usize) -> Result<(), String> {
    if glyph.chars().any(|c| c.is_control()) {
        return Err("it has a control character".into());
    }
    if glyph.chars().any(|c| {
        matches!(c, '\u{200d}' | '\u{fe0f}' | '\u{20e3}' | '\u{1f3fb}'..='\u{1f3ff}' | '\u{1f1e6}'..='\u{1f1ff}')
    }) {
        return Err("joined emoji, skin tones and flags draw at different widths in different terminals".into());
    }
    let w = super::width(glyph);
    if w > width {
        return Err(format!(
            "it is {w} cells wide and this pack's icons take {width} (declare width = 2 for emoji)"
        ));
    }
    Ok(())
}

/// The packs in `dir` (`*.toml`), sorted by name, and what was wrong with
/// them. A pack named like a built-in one, or like another, is passed over.
pub fn load_dir(dir: &Path) -> (Vec<Pack>, Vec<String>) {
    let mut packs: Vec<Pack> = Vec::new();
    let mut warnings = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (packs, warnings);
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    paths.sort();
    for path in paths {
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("pack").to_string();
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                warnings.push(format!("{}: {e}", path.display()));
                continue;
            }
        };
        match Pack::parse(&text, &stem) {
            Ok((mut pack, w)) => {
                warnings.extend(w.into_iter().map(|w| format!("{}: {w}", path.display())));
                if BUILT_IN.contains(&pack.name.as_str()) || pack.name == "auto" {
                    warnings.push(format!("{}: {:?} is a built-in pack's name", path.display(), pack.name));
                    continue;
                }
                if packs.iter().any(|p| p.name == pack.name) {
                    warnings.push(format!("{}: a pack called {:?} came first", path.display(), pack.name));
                    continue;
                }
                pack.path = Some(path);
                packs.push(pack);
            }
            Err(e) => warnings.push(format!("{}: {e}", path.display())),
        }
    }
    (packs, warnings)
}

/// The overrides in config.toml's `[icons]` table. An entry that is not a
/// glyph is passed over (and logged); one with an unknown name is kept, so
/// the file keeps it, and warned about when resolving.
pub fn overrides(table: Option<&toml::Value>) -> BTreeMap<String, Glyph> {
    let mut out = BTreeMap::new();
    for (k, v) in table.and_then(|t| t.as_table()).into_iter().flatten() {
        match Glyph::deserialize(v.clone()) {
            Ok(g) => {
                out.insert(k.clone(), g);
            }
            Err(_) => crate::log::warn(format!(
                "[icons] {k}: must be \"glyph\" or {{ glyph = \"…\", style = \"…\" }}"
            )),
        }
    }
    out
}

/// Where a user's packs live: `icons` in radar's config folder
/// ([`crate::runner::config_home`]).
pub fn dir() -> PathBuf {
    crate::runner::config_home().join("icons")
}

// ---------------------------------------------------------------------
// Resolving: the chosen pack, the packs under it, the overrides.

/// One icon as it is drawn: its glyph padded to the pack's width, its
/// style spec, and where it came from (a pack's name, or `[icons]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub glyph: String,
    pub style: String,
    pub from: String,
}

/// The icons in use: every icon resolved, and the pack they came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Icons {
    pub pack: String,
    /// Cells each glyph takes; 0 when icons are off.
    pub width: usize,
    all: Vec<Resolved>,
}

impl Icons {
    /// No icons and no gaps.
    pub fn off() -> Self {
        Self {
            pack: "off".into(),
            width: 0,
            all: Icon::ALL
                .iter()
                .map(|i| Resolved {
                    glyph: String::new(),
                    style: i.style().into(),
                    from: "off".into(),
                })
                .collect(),
        }
    }

    pub fn get(&self, icon: Icon) -> &Resolved {
        &self.all[icon as usize]
    }

    pub fn on(&self) -> bool {
        self.width > 0
    }
}

/// The pack called `name` among the built-in and `user` packs.
pub fn find(name: &str, user: &[Pack]) -> Option<Pack> {
    Pack::built_in(name).or_else(|| user.iter().find(|p| p.name == name).cloned())
}

/// What `auto` is here: the pack the runner chose, or `off` before it did
/// (tests, snapshots).
pub fn auto_choice() -> String {
    HERE.with(|h| h.borrow().auto.clone()).unwrap_or_else(|| "off".into())
}

/// The icons for the choice `name` (`auto`, a built-in or a user's pack),
/// with `overrides` on top; and what was wrong, as warnings.
pub fn resolve(name: &str, user: &[Pack], overrides: &BTreeMap<String, Glyph>) -> (Icons, Vec<String>) {
    let mut warnings = Vec::new();
    let name = if name == "auto" || name.is_empty() { auto_choice() } else { name.to_string() };
    let top = match find(&name, user) {
        Some(p) => p,
        None => {
            warnings.push(format!("no icon pack is called {name:?}; using unicode"));
            Pack::built_in("unicode").expect("built in")
        }
    };
    if top.name == "off" {
        return (Icons::off(), warnings);
    }
    // The pack, what it extends, and so on; unicode last.
    let mut chain = vec![top];
    while let Some(next) = chain.last().and_then(|p| p.extends.clone()) {
        if chain.len() > 8 || chain.iter().any(|p| p.name == next) {
            warnings.push(format!("{}: extends goes round in a circle", chain[0].name));
            break;
        }
        match find(&next, user) {
            Some(p) if p.name != "off" => chain.push(p),
            Some(_) => break,
            None => {
                warnings.push(format!("{}: extends {next:?}, which is not a pack", chain.last().map_or("", |p| &p.name)));
                break;
            }
        }
    }
    if chain.last().is_none_or(|p| p.name != "unicode") {
        chain.push(Pack::built_in("unicode").expect("built in"));
    }
    let width = chain.iter().find_map(|p| p.width).unwrap_or(1);
    for p in &chain {
        for k in p.icons.keys() {
            if Icon::from_name(k).is_none() && p.path.is_some() {
                warnings.push(format!("{}: no icon is called {k:?}", p.name));
            }
        }
    }
    for k in overrides.keys() {
        if Icon::from_name(k).is_none() {
            warnings.push(format!("[icons]: no icon is called {k:?}"));
        }
    }
    let all = Icon::ALL
        .iter()
        .map(|&icon| {
            let candidates = std::iter::once(("[icons]".to_string(), overrides.get(icon.name())))
                .chain(chain.iter().map(|p| (p.name.clone(), p.icons.get(icon.name()))));
            for (from, g) in candidates {
                let Some(g) = g else { continue };
                match check(g.text(), width) {
                    Ok(()) => {
                        let glyph = if g.text().is_empty() {
                            String::new()
                        } else {
                            super::pad(g.text(), width)
                        };
                        return Resolved {
                            glyph,
                            style: g.style().unwrap_or(icon.style()).to_string(),
                            from,
                        };
                    }
                    Err(why) => warnings.push(format!("{from}: {} {:?} refused: {why}", icon.name(), g.text())),
                }
            }
            Resolved {
                glyph: String::new(),
                style: icon.style().into(),
                from: "none".into(),
            }
        })
        .collect();
    (Icons { pack: name, width, all }, warnings)
}

/// A pack written out whole as TOML, every icon with its glyph and style:
/// a starting point for one's own.
pub fn export(name: &str, user: &[Pack]) -> Option<String> {
    find(name, user)?;
    let (icons, _) = resolve(name, user, &BTreeMap::new());
    let mut out = format!(
        "# An icon pack for radar, written from {name:?}. Put it in {} as <name>.toml.\n\
         name = \"my-{name}\"\n\
         description = \"my icons, starting from {name}\"\n\
         extends = \"{name}\"\n\
         width = {}\n\n[icons]\n",
        dir().display(),
        icons.width.max(1)
    );
    let mut group = "";
    for &icon in Icon::ALL {
        if icon.group() != group {
            group = icon.group();
            out.push_str(&format!("\n# {group}\n"));
        }
        let r = icons.get(icon);
        let glyph = toml::Value::String(r.glyph.trim_end().to_string()).to_string();
        let style = toml::Value::String(r.style.clone()).to_string();
        out.push_str(&format!("{} = {{ glyph = {glyph}, style = {style} }}  # {}\n", icon.name(), icon.about()));
    }
    Some(out)
}

/// A line of a few icons of a pack, to show what it looks like.
pub fn preview(name: &str, user: &[Pack]) -> String {
    let (icons, _) = resolve(name, user, &BTreeMap::new());
    if !icons.on() {
        return String::new();
    }
    [
        Icon::Repo,
        Icon::Branch,
        Icon::Change,
        Icon::RunPassed,
        Icon::RunFailed,
        Icon::Bug,
        Icon::Story,
        Icon::Person,
        Icon::Channel,
        Icon::Settings,
    ]
    .iter()
    .map(|&i| icons.get(i).glyph.clone())
    .collect::<Vec<_>>()
    .join(" ")
}

// ---------------------------------------------------------------------
// What `auto` is here, and the user's packs, as the runner found them.

#[derive(Default)]
struct Here {
    auto: Option<String>,
    packs: Vec<Pack>,
    /// The icons the host's theme draws with, for words built where no
    /// theme is in reach ([`plain`]).
    current: Option<Icons>,
}

/// Say which icons the host draws with now (the host does, each time it
/// builds its theme).
pub fn set_current(icons: &Icons) {
    HERE.with(|h| h.borrow_mut().current = Some(icons.clone()));
}

/// The icons the host draws with now; off before it built a theme (tests).
pub fn current() -> Icons {
    HERE.with(|h| h.borrow().current.clone()).unwrap_or_else(Icons::off)
}

impl Icons {
    /// Its glyphs are Nerd Font symbols (in a private use area), so a
    /// mark a part brings may be its Nerd Font one.
    pub fn nerd(&self) -> bool {
        let private = |c: char| matches!(c as u32, 0xE000..=0xF8FF | 0xF0000..=0x10FFFF);
        self.get(Icon::Settings).glyph.chars().any(private)
    }
}

/// The icon and a space as plain text, from the icons the host draws with
/// now: for words that truly have no style. A menu's title or row, a keys
/// menu heading, a contributed chip and a status-line message name their
/// icon instead (`Menu::icon`, `MenuItem::icon`, `Binding::icon`,
/// `Chip::icon`, `Effect::NotifyWith`) and the host draws it in its own
/// colour. Empty when icons are off, and before the host built a theme
/// (tests). Words carry no colour, so the icon takes theirs.
pub fn plain(icon: Icon) -> String {
    HERE.with(|h| {
        h.borrow()
            .current
            .as_ref()
            .map(|i| i.get(icon).glyph.clone())
            .filter(|g| !g.is_empty())
            .map(|g| format!("{g} "))
            .unwrap_or_default()
    })
}

/// For words that name their icon (`MenuItem::icon`, `Chip::icon`,
/// `Effect::notify`) and showed a symbol before icons: with icons on, the
/// icon to name and no text; with them off, no icon and `symbol` and a
/// space to put before the words, so the screen reads as it did.
///
/// ```ignore
/// let (icon, mark) = icon::plain_or(Icon::Error, "✗");
/// Effect::notify(icon, format!("{mark}could not save"))
/// ```
pub fn plain_or(icon: Icon, symbol: &str) -> (Option<Icon>, String) {
    if plain(icon).is_empty() { (None, format!("{symbol} ")) } else { (Some(icon), String::new()) }
}

/// `words` after the icon, as [`plain`] gives it.
pub fn plain_with(icon: Icon, words: &str) -> String {
    format!("{}{words}", plain(icon))
}

thread_local! {
    static HERE: RefCell<Here> = RefCell::new(Here::default());
}

/// Say what `auto` means on this thread (the runner, from [`detect`]).
pub fn set_auto(pack: &str) {
    HERE.with(|h| h.borrow_mut().auto = Some(pack.to_string()));
}

/// The user's packs on this thread (the runner, from [`load_dir`]).
pub fn set_packs(packs: Vec<Pack>) {
    HERE.with(|h| h.borrow_mut().packs = packs);
}

/// The user's packs the runner loaded; none in tests unless a test set them.
pub fn user_packs() -> Vec<Pack> {
    HERE.with(|h| h.borrow().packs.clone())
}

/// Every pack a menu offers: the built-in ones, then the user's.
pub fn pack_names() -> Vec<String> {
    BUILT_IN
        .iter()
        .map(|s| s.to_string())
        .chain(user_packs().into_iter().map(|p| p.name))
        .collect()
}

/// What `auto` picks in this environment: `nerd` where the terminal is
/// known to ship or commonly use Nerd Font symbols, else `unicode`; over
/// SSH `unicode` (the font is at the other end).
pub fn detect(var: &dyn Fn(&str) -> Option<String>) -> &'static str {
    let set = |k: &str| var(k).is_some_and(|v| !v.is_empty());
    if set("SSH_CONNECTION") || set("SSH_TTY") || set("SSH_CLIENT") {
        return "unicode";
    }
    let program = var("TERM_PROGRAM").unwrap_or_default().to_lowercase();
    let term = var("TERM").unwrap_or_default();
    if ["ghostty", "wezterm", "kitty"].contains(&program.as_str())
        || ["ghostty", "kitty", "wezterm"].iter().any(|t| term.contains(t))
        // Inside a multiplexer the outer terminal still shows in what it set.
        || set("GHOSTTY_RESOURCES_DIR")
        || set("KITTY_WINDOW_ID")
        || set("WEZTERM_PANE")
    {
        return "nerd";
    }
    "unicode"
}

// ---------------------------------------------------------------------
// Drawing.

/// The icon in its colour, padded to the pack's width; empty when icons
/// are off or the pack has none for it.
pub fn span(theme: &Theme, icon: Icon) -> Span<'static> {
    let r = theme.icons().get(icon);
    if r.glyph.is_empty() {
        return Span::raw("");
    }
    Span::styled(r.glyph.clone(), theme.spec(&r.style))
}

/// The icon and a space, to put before a name; empty (no gap) when there
/// is no icon.
pub fn lead(theme: &Theme, icon: Icon) -> Span<'static> {
    let mut s = span(theme, icon);
    if !s.content.is_empty() {
        s.content = format!("{} ", s.content).into();
    }
    s
}

/// The icon and a space as plain text, for words that are a string (a tab,
/// a menu row, a chip): empty when there is no icon.
pub fn text(theme: &Theme, icon: Icon) -> String {
    let r = theme.icons().get(icon);
    if r.glyph.is_empty() { String::new() } else { format!("{} ", r.glyph) }
}

/// `words` with the icon before them, as plain text.
pub fn with(theme: &Theme, icon: Icon, words: &str) -> String {
    format!("{}{words}", text(theme, icon))
}

/// The icon in its colour when the pack has one, else `fallback` in
/// `style`: for a state a screen showed as a symbol before icons (`✓`,
/// `●`), so it reads the same with icons off.
pub fn or(theme: &Theme, icon: Icon, fallback: &str, style: ratatui::style::Style) -> Span<'static> {
    let s = span(theme, icon);
    if s.content.is_empty() {
        Span::styled(fallback.to_string(), style)
    } else {
        s
    }
}

/// Whether icons are drawn at all.
pub fn on(theme: &Theme) -> bool {
    theme.icons().on()
}

/// The cells [`lead`] takes: the pack's width and a space, or none.
pub fn lead_width(theme: &Theme) -> usize {
    let w = theme.icons().width;
    if w == 0 { 0 } else { w + 1 }
}

#[cfg(test)]
mod tests;
