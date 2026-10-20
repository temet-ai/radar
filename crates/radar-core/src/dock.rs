//! The agent dock's contract (agent-dock plan, "The dock"): what the part
//! that owns agent sessions shows below the middle pane, and what the user
//! does there, described without a terminal.
//!
//! The host owns all state. A part supplies the dock's *content* through
//! [`crate::Plugin::dock`] (tabs, the active tab's transcript, composer,
//! status line, pickers); the host adds its own state (shown or hidden,
//! focus, the height the user dragged to) and sends the result on every
//! screen as [`crate::view::ScreenView::dock`]. Ink draws it and sends
//! [`DockIntent`]s; it keeps no state of record, so restarting Ink never
//! stops an agent.

use serde::{Deserialize, Serialize};

use crate::ui::icon::Icon;

/// The host key for the dock: shows it with the keys, then (pressed again
/// with the keys in it) hides it. `esc` in the dock sends [`DockIntent::Blur`].
pub const KEY: char = '`';

/// The share of the middle column the dock takes until the user drags its
/// edge, in percent. Ink lays this out; below [`NARROW`] columns the dock
/// opens full height over the middle pane instead.
pub const DEFAULT_SHARE: u8 = 40;
/// Below this many columns the dock opens full height over the middle pane.
pub const NARROW: u16 = 100;

/// The dock: its state (host) and its content (the part that owns sessions).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, ts_rs::TS)]
pub struct DockView {
    /// Shown below the middle pane. Host state; a key toggles it.
    pub visible: bool,
    /// Keys go to the dock (its composer, its approvals) and not to the
    /// pane. Host state.
    pub focused: bool,
    /// The dock's height in rows as the user dragged it, counting its top
    /// edge; `None` until then ([`DEFAULT_SHARE`] percent of the middle
    /// column). Host state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u16>,
    /// One tab per session, in the order they were opened.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tabs: Vec<DockTab>,
    /// The tab shown: a [`DockTab::id`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
    /// The active tab's transcript, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transcript: Vec<DockItem>,
    #[serde(default)]
    pub composer: DockComposer,
    #[serde(default)]
    pub status: DockStatus,
    /// A model or session picker open over the transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picker: Option<DockPicker>,
    /// What the active session is doing now (the line above the composer),
    /// or how its last turn ended; `None` before its first turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<DockActivity>,
    /// The dock takes the whole middle column ([`FULL_KEY`]). Host state,
    /// kept for the run like the dragged height.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub full: bool,
    /// The tab that waits for the user and is not in sight (the dock
    /// hidden, or another tab shown): the status line's chip names it and
    /// the dock key goes to it. Host state, from the tabs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attention: Option<String>,
    /// The transcript's revision as the host's pipe to a renderer sent it
    /// (`ipc::Bridge`); 0 everywhere else. A renderer keeps it with the
    /// transcript it holds, for the next [`DockView::kept`].
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    #[ts(optional, type = "number")]
    pub revision: u64,
    /// Over the pipe only: `transcript` is not the whole of it but what
    /// follows the first `kept.items` items of the transcript the renderer
    /// holds at `kept.revision`, so a streamed word sends one item, not the
    /// session's history. A renderer that holds another revision asks for
    /// the whole (`ipc::ToHost::Resync`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub kept: Option<DockKept>,
}

fn is_zero_u64(n: &u64) -> bool {
    *n == 0
}

/// The head of the transcript a renderer already holds ([`DockView::kept`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct DockKept {
    /// The revision it was sent at.
    #[ts(type = "number")]
    pub revision: u64,
    /// How many of its first items stand unchanged.
    pub items: u32,
}

/// The dock key with alt: the dock takes the whole middle column, and back.
pub const FULL_KEY: char = 'f';

/// What a session is doing, for the activity line (the agent-dock plan,
/// "The dock"): while a turn runs, what it does now and for how long; once
/// it ended, how it ended and how long it took.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct DockActivity {
    pub doing: Doing,
    /// What it is about, when there is one thing: the command a tool runs
    /// (`git diff`), the target an approval asks for.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
    /// Seconds the turn has run (while it runs) or took (once it ended), at
    /// the host's clock.
    pub elapsed: u32,
    /// Tokens the turn used, when the provider reports usage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u64>,
    /// Messages sent while the turn runs that wait for after it (radar's or the
    /// provider's queue).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub queued: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum Doing {
    /// The turn runs and nothing is written yet, or the agent reasons.
    Thinking,
    /// The agent's words are streaming.
    Writing,
    /// A tool runs ([`DockActivity::detail`] says which).
    Running,
    /// A permission request waits for the user.
    Approval,
    /// A question waits for the user's answer.
    Question,
    /// The last turn ended.
    Done,
    /// The user stopped the last turn.
    Stopped,
    /// The last turn failed.
    Failed,
    /// radar reads the session back from its provider (restored from the last
    /// run): the transcript and the live state follow.
    Resuming,
    /// Restored from the last run and not attached to its provider: the
    /// next message, or the resume key, attaches it.
    Detached,
}

/// A session's tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct DockTab {
    /// radar's own session id.
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::view::icon_name")]
    #[ts(as = "Option<String>")]
    pub icon: Option<Icon>,
    pub title: String,
    pub state: SessionState,
    /// A turn ended (done, stopped or failed) since the tab was last shown
    /// in the dock: the tab says so until it is seen.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unread: bool,
    /// Seconds the running turn has run, while one runs: its spinner's
    /// frame (the dock shows no count on a tab).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub running: Option<u32>,
    /// Set on a terminal tab: Ink draws the CLI's screen in the tab. The
    /// terminal's bytes do not travel as view data; how Ink reaches them is
    /// the terminal task's to settle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal: Option<TerminalView>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    /// Created, nothing sent yet.
    Idle,
    /// A turn is running.
    Streaming,
    /// A request waits for an answer; nothing is auto-approved.
    Approval,
    /// The turn failed or the connection was lost.
    Error,
    /// The user stopped the turn.
    Stopped,
    /// The session or its terminal ended.
    Ended,
    /// Restored from the last run, not attached to its provider yet: what
    /// its history shows waiting needs no one.
    Detached,
    /// Being read back from its provider.
    Resuming,
}

/// A terminal tab's surface ([`DockTab::terminal`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct TerminalView {
    pub id: String,
    /// Where Ink reaches the terminal's screen and keys (a per-terminal
    /// socket path); empty until the terminal task fills it.
    #[serde(default)]
    pub socket: String,
    pub columns: u16,
    pub rows: u16,
    pub state: TerminalState,
    pub command: String,
    pub cwd: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum TerminalState {
    Running,
    Exited { code: i32 },
    Failed,
}

/// One item of a transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "item", rename_all = "snake_case")]
pub enum DockItem {
    /// What the user sent. `queued` while it waits for the turn that runs
    /// to end; absent once the agent has it.
    User {
        id: String,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        queued: Option<Queued>,
    },
    /// The agent's words. `saved` is false while the text is streamed
    /// deltas and true once the provider's persisted message replaced them.
    /// Made with [`DockItem::assistant`], which reads `text` as markdown
    /// into `doc` (the dock lays it out as `ui::rich` lays out a
    /// description) and highlights its code blocks into `painted`: one entry
    /// per code block in document order, a line of runs per line of code,
    /// empty for a language no grammar knows.
    Assistant {
        id: String,
        text: String,
        saved: bool,
        #[serde(default, skip_serializing_if = "no_blocks")]
        #[ts(as = "Option<Vec<crate::ui::rich::Block>>", optional)]
        doc: crate::ui::rich::Doc,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        painted: Vec<Vec<Vec<crate::view::PaintRun>>>,
    },
    /// The agent's reasoning: folded by default.
    Reasoning {
        id: String,
        text: String,
        open: bool,
    },
    /// A tool call: one line when folded (`⋯ n` for its hidden lines), tree
    /// lines when open.
    Tool {
        id: String,
        /// What the call is, one line: `ls -la`, `src/lib.rs`.
        title: String,
        /// The tool's name as the provider gave it (Claude Code's `Read`,
        /// `Bash`, `Edit`, `mcp__server__tool`; Codex's `shell`; Copilot's
        /// `view`, `bash`): the dock tells its kind (read, search, shell,
        /// edit, web, mcp, agent) from it.
        kind: String,
        state: ToolState,
        open: bool,
        /// The result, as lines; shown when open.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        result: Vec<String>,
        /// What an edit or a write changes, as diff lines (`+`, `-`, ` `):
        /// folded to `+n −m`, shown when open.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        diff: Vec<String>,
    },
    /// A request the user must answer, with the exact target.
    Permission(DockPermission),
    /// The agent asks the user something and waits for the answer.
    Question(DockQuestion),
    Error { id: String, text: String },
    /// Something radar says (resumed, stopped, limits), not the agent.
    Notice { id: String, text: String },
}

/// Who holds a message sent while a turn runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum Queued {
    /// radar keeps it and sends it when the turn ends (the provider takes
    /// nothing mid-turn).
    Here,
    /// Sent: the provider keeps it and runs it after this turn.
    Provider,
}

fn no_blocks(d: &crate::ui::rich::Doc) -> bool {
    d.blocks.is_empty()
}

/// An assistant message's code blocks highlighted: [`DockItem::Assistant`]'s
/// `painted`.
pub type Painted = Vec<Vec<Vec<crate::view::PaintRun>>>;

impl DockItem {
    /// The agent's words, read as markdown for the dock to lay out. While
    /// `saved` is false the text is still coming, and its unfinished last
    /// line is read as if its open marks were closed ([`streamed`]), so a
    /// half-received `**Verdict` shows bold instead of its stars.
    pub fn assistant(id: impl Into<String>, text: impl Into<String>, saved: bool) -> Self {
        let text = text.into();
        let (doc, painted) = read(&text, saved);
        DockItem::Assistant { id: id.into(), text, saved, doc, painted }
    }
}

thread_local! {
    /// What [`read`] made, by text: the dock is built on every screen, and a
    /// message that has not changed is not parsed and highlighted again.
    static READ: std::cell::RefCell<std::collections::HashMap<(bool, String), (crate::ui::rich::Doc, Painted)>> =
        Default::default();
}

/// Messages kept read; past this many the cache starts again (a streamed
/// message leaves one entry per delta behind).
const READ_KEPT: usize = 256;

fn read(text: &str, saved: bool) -> (crate::ui::rich::Doc, Painted) {
    let key = (saved, text.to_string());
    if let Some(hit) = READ.with(|c| c.borrow().get(&key).cloned()) {
        return hit;
    }
    let source = if saved { std::borrow::Cow::Borrowed(text) } else { streamed(text) };
    let doc = crate::ui::markdown::parse(&source);
    let painted = paint_code(&doc);
    READ.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() >= READ_KEPT {
            c.clear();
        }
        c.insert(key, (doc.clone(), painted.clone()));
    });
    (doc, painted)
}

/// Markdown still arriving, made safe to show: the last line (not yet
/// ended by a newline) has its open inline code, `**` and `~~` closed, a
/// mark that opens at its very end dropped, and a link whose address is
/// still coming shown as its words. Inside an open code fence nothing
/// changes (the fence runs to the end, as it will once closed).
pub fn streamed(text: &str) -> std::borrow::Cow<'_, str> {
    let start = text.rfind('\n').map_or(0, |i| i + 1);
    let fence = |l: &str| {
        let t = l.trim_start();
        t.starts_with("```") || t.starts_with("~~~")
    };
    let (head, tail) = text.split_at(start);
    if head.lines().filter(|l| fence(l)).count() % 2 == 1 || fence(tail) || tail.is_empty() {
        return std::borrow::Cow::Borrowed(text);
    }
    // Marks close against the last word, not after the spaces that follow it.
    let space = &tail[tail.trim_end().len()..];
    let mut tail = tail.trim_end().to_string();
    // A link whose address is still coming: its words alone.
    if let Some(at) = tail.rfind("](")
        && !tail[at..].contains(')')
        && let Some(open) = tail[..at].rfind('[')
    {
        let words = tail[open + 1..at].to_string();
        tail.truncate(open);
        tail.push_str(&words);
    }
    // A mark that opens at the very end has no words yet: drop it.
    for mark in ["**", "~~", "`"] {
        let n = if mark == "`" { tail.matches('`').count() } else { outside_code(&tail).matches(mark).count() };
        if tail.ends_with(mark) && n % 2 == 1 {
            tail.truncate(tail.len() - mark.len());
        }
    }
    let mut close = String::new();
    if tail.matches('`').count() % 2 == 1 {
        close.push('`');
    }
    let outside = outside_code(&tail);
    if outside.matches("~~").count() % 2 == 1 {
        close.push_str("~~");
    }
    if outside.matches("**").count() % 2 == 1 {
        close.push_str("**");
    }
    std::borrow::Cow::Owned(format!("{head}{tail}{close}{space}"))
}

/// `s` without its code spans (what lies between backticks).
fn outside_code(s: &str) -> String {
    let mut out = String::new();
    let mut code = false;
    for ch in s.chars() {
        if ch == '`' {
            code = !code;
        } else if !code {
            out.push(ch);
        }
    }
    out
}

/// Code longer than this is not highlighted (it shows in the code slot).
const PAINT_MOST: usize = 2000;

/// The grammar for a fence's language word where the bundled grammars
/// name it otherwise: TypeScript and JSX read as JavaScript (no grammar of
/// their own), a shell session as shell.
fn grammar(lang: &str) -> &str {
    match lang.to_ascii_lowercase().as_str() {
        "typescript" | "ts" | "tsx" | "jsx" | "mjs" | "cjs" => "js",
        "shell" | "zsh" | "console" | "shellsession" => "sh",
        "yml" => "yaml",
        _ => lang,
    }
}

/// Every code block of `doc`, in document order, highlighted line by line as
/// `ui::rich` lays its lines out (tabs as four spaces, trailing blanks off).
fn paint_code(doc: &crate::ui::rich::Doc) -> Painted {
    use crate::ui::highlight::Highlighter;
    use crate::ui::rich::Block;
    fn walk(blocks: &[Block], out: &mut Painted) {
        for b in blocks {
            match b {
                Block::Code { lang, text } => {
                    let lines: Vec<String> = text.replace('\r', "").lines().map(|l| l.replace('\t', "    ").trim_end().to_string()).collect();
                    let painted = match Highlighter::new(grammar(lang)) {
                        Some(mut hl) if !lang.is_empty() && lines.len() <= PAINT_MOST => lines
                            .iter()
                            .map(|l| match hl.runs(l) {
                                Some(runs) => {
                                    let mut row: Vec<crate::view::PaintRun> = Vec::new();
                                    for (piece, token) in runs {
                                        match row.last_mut() {
                                            Some(last) if last.token == token.name() => last.text.push_str(piece),
                                            _ => row.push(crate::view::PaintRun { text: piece.to_string(), token: token.name().into() }),
                                        }
                                    }
                                    row
                                }
                                None => vec![crate::view::PaintRun { text: l.clone(), token: "plain".into() }],
                            })
                            .collect(),
                        _ => Vec::new(),
                    };
                    out.push(painted);
                }
                Block::List { items } => items.iter().for_each(|i| walk(&i.blocks, out)),
                Block::Quote { blocks } | Block::Callout { blocks, .. } | Block::Details { blocks, .. } => walk(blocks, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&doc.blocks, &mut out);
    // Nothing to send when no block was highlighted.
    if out.iter().all(Vec::is_empty) { Vec::new() } else { out }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum ToolState {
    Running,
    Done,
    Failed,
    Denied,
}

/// An approval request, drawn inline with what it will do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct DockPermission {
    /// The provider's request id; answers carry it back.
    pub id: String,
    /// `shell`, `read`, `write`, `url`, `mcp`, `other`.
    pub kind: String,
    /// The exact command, path, URL or tool, as the provider gave it.
    pub target: String,
    /// A diff for a write, as lines, when there is one ("view diff").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diff: Vec<String>,
    /// What can be done: the actions the adapter supports for this request.
    pub actions: Vec<PermissionAction>,
    /// Answered already (shows what was chosen), or still waiting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered: Option<PermissionAction>,
    /// The tool call it asks for ([`DockItem::Tool`]'s `id`), when the
    /// provider said: once answered, the call's own row tells it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// It waited when radar closed, and the process that asked is gone: nothing
    /// can answer it now. Drawn without buttons or keys, never "needs you".
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub lapsed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum PermissionAction {
    /// Approve this one request.
    ApproveOnce,
    /// Approve it and the same again for the rest of the session. Offered
    /// only where the request allows it.
    ApproveForSession,
    /// Approve it and add it to Settings → Agents → always allow, so the
    /// same request never asks again in any session.
    AlwaysAllow,
    Deny,
    /// Open the diff. Ink-side only; it sends no answer.
    ViewDiff,
}

/// A question the agent asked, drawn inline. A request may hold several
/// questions (Claude Code, Codex); they are answered one at a time, in
/// order, and the answers go to the agent once every one has its answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct DockQuestion {
    /// The provider's request id; answers carry it back.
    pub id: String,
    pub questions: Vec<DockAsk>,
    pub state: AskState,
}

/// One question of a [`DockQuestion`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct DockAsk {
    /// The question's key within its request; answers carry it back.
    pub id: String,
    /// A short label, when the agent gave one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    /// The question, exactly as the agent asked it.
    pub text: String,
    /// The choices, in the agent's order; picking one answers with its
    /// label. Empty when only typed words answer it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<DockAskChoice>,
    /// The composer's text may answer it (the user's own words).
    #[serde(default)]
    pub free_text: bool,
    /// Several choices may be picked before the answer is given.
    #[serde(default)]
    pub multiple: bool,
    /// Answered already (in this request, or the request is settled).
    #[serde(default)]
    pub answered: bool,
    /// What was answered, to show; empty for a secret answer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub answer: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct DockAskChoice {
    pub label: String,
    /// What the choice means, when the agent said.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum AskState {
    /// Waits for the user.
    Waiting,
    Answered,
    /// The user declined, or the turn ended before an answer.
    Declined,
    /// It waited when radar closed, and the process that asked is gone: nothing
    /// can answer it now.
    Lapsed,
}

/// The composer: the text being written is Ink's editor state until it is
/// sent; the host holds what it must keep across a restart.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, ts_rs::TS)]
pub struct DockComposer {
    /// A draft the host kept (restored after a restart), or empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub draft: String,
    /// Messages sent before, newest last, for the history keys.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<String>,
    /// Slash commands the adapter advertises.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<DockCommand>,
    /// Sending is possible now (no turn running that cannot queue).
    pub can_send: bool,
    /// A turn is running and can be stopped.
    pub can_stop: bool,
    /// Why the composer is off, when it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct DockCommand {
    /// Typed after the slash.
    pub name: String,
    pub detail: String,
}

/// The dock's status line.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, ts_rs::TS)]
pub struct DockStatus {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub provider: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    /// The agent the session runs as, when it is not the CLI's own.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub agent: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Usage in words the provider reports ("12k tokens"), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct DockPicker {
    pub kind: PickerKind,
    pub choices: Vec<DockChoice>,
    /// The choices are still being read from the CLI.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reading: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum PickerKind {
    Model,
    Session,
    /// The agents the provider offers: the next new session starts as the one chosen.
    Agent,
    /// The skills the provider has loaded: choosing one asks for it in the composer.
    Skill,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct DockChoice {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
    #[serde(default)]
    pub current: bool,
}

/// What the user does in the dock. The host takes `Show`, `Hide`, `Toggle`,
/// `Focus`, `Blur` and `Resize` (its state); everything else goes to the
/// part that supplies the dock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "dock", rename_all = "snake_case")]
pub enum DockIntent {
    /// Send the composer's text to the active session.
    Send { text: String },
    /// Stop the running turn.
    Stop,
    /// Answer a request (`id` is [`DockPermission::id`]).
    Approve { id: String },
    /// Approve it and the same again for the rest of the session.
    ApproveForSession { id: String },
    /// Approve it and add it to the standing allow rules.
    AlwaysAllow { id: String },
    Deny { id: String },
    /// Answer question `question` ([`DockAsk::id`]) of request `id`
    /// ([`DockQuestion::id`]): the labels of the choices picked, or the
    /// user's own words in `text`.
    Answer {
        id: String,
        question: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        choices: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    /// Refuse to answer request `id`; the agent hears that the user declined.
    Decline { id: String },
    /// Open or fold a transcript item (a tool call, reasoning).
    Fold { item: String, open: bool },
    /// Open a picker, or choose in the open one.
    OpenPicker { kind: PickerKind },
    PickModel { id: String },
    PickSession { id: String },
    PickAgent { id: String },
    PickSkill { id: String },
    ClosePicker,
    /// A new session with the default provider, or with `provider`.
    NewSession {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider: Option<String>,
    },
    /// Show another tab.
    SwitchTab { id: String },
    /// Close a tab: a terminal tab's process ends; a session whose turn runs
    /// is asked about first (the host's confirm) and stopped; it stays in the
    /// provider's own list, so the session picker resumes it. The next tab
    /// (else the previous) shows.
    CloseTab { id: String },
    /// Restart a terminal tab's command.
    RestartTerminal { id: String },
    /// Attach the active tab to its provider again (a session restored from
    /// the last run, or one whose resume failed) without sending anything.
    Resume,
    Show,
    Hide,
    Toggle,
    /// Keys go to the dock / back to the pane.
    Focus,
    Blur,
    /// The user dragged the top edge: the dock's new height in rows.
    Resize { rows: u16 },
    /// The dock takes the whole middle column, or goes back to its height.
    ToggleFull,
}

/// Why a tab wants the user, worst first: what the status line's chip says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Want {
    /// A request or a question waits for an answer.
    NeedsYou,
    /// Its last turn failed and was not seen.
    Failed,
    /// Its last turn ended and was not seen.
    Done,
}

impl Want {
    /// What a tab wants, if anything.
    pub fn of(tab: &DockTab) -> Option<Want> {
        match tab.state {
            SessionState::Approval => Some(Want::NeedsYou),
            SessionState::Error if tab.unread => Some(Want::Failed),
            _ if tab.unread => Some(Want::Done),
            _ => None,
        }
    }

    pub fn words(self) -> &'static str {
        match self {
            Want::NeedsYou => "needs you",
            Want::Failed => "failed",
            Want::Done => "done",
        }
    }
}

/// The tabs that want the user and are out of sight (the dock hidden, or
/// another tab shown), worst first and then in tab order.
pub fn wanting<'a>(tabs: &'a [DockTab], active: Option<&str>, visible: bool) -> Vec<(Want, &'a DockTab)> {
    let mut out: Vec<(Want, &DockTab)> = tabs
        .iter()
        .filter(|t| !(visible && active == Some(t.id.as_str())))
        .filter_map(|t| Some((Want::of(t)?, t)))
        .collect();
    out.sort_by_key(|(w, _)| *w);
    out
}

/// The host's dock state: what survives Ink restarting.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DockState {
    pub visible: bool,
    pub focused: bool,
    pub rows: Option<u16>,
    /// The dock takes the whole middle column.
    pub full: bool,
    /// Whether the part that owns the dock was last told it is shown.
    pub told_visible: Option<bool>,
}

impl DockState {
    /// Apply a host-owned intent; `false` for one the part must take.
    pub fn apply(&mut self, intent: &DockIntent) -> bool {
        match intent {
            DockIntent::Show => {
                self.visible = true;
                self.focused = true;
            }
            DockIntent::Hide => {
                self.visible = false;
                self.focused = false;
            }
            DockIntent::Toggle => {
                self.visible = !self.visible;
                self.focused = self.visible;
            }
            DockIntent::Focus => {
                self.visible = true;
                self.focused = true;
            }
            DockIntent::Blur => self.focused = false,
            DockIntent::Resize { rows } => {
                self.rows = Some((*rows).max(3));
                self.visible = true;
                // A drag sets a height: the dock shares the column again.
                self.full = false;
            }
            DockIntent::ToggleFull => {
                self.full = !self.full;
                self.visible = true;
            }
            _ => return false,
        }
        true
    }

    /// The content a part supplied, with the host's state laid over it.
    pub fn over(&self, mut content: DockView) -> DockView {
        content.visible = self.visible;
        content.focused = self.focused && self.visible;
        content.rows = self.rows;
        content.full = self.full;
        content.attention = wanting(&content.tabs, content.active.as_deref(), self.visible).first().map(|(_, t)| t.id.clone());
        content
    }
}
