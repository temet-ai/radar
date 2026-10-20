//! Changing tickets from radar: `t` moves one to another status, `c` comments
//! on it, `n` creates a ticket, `i` assigns one to you. Each is asked
//! first, in radar, with what will be sent shown exactly; nothing is sent
//! before `y`. A write is sent once (never retried); what Jira said is
//! shown plainly on the status line and on the ticket's page, and the
//! ticket is fetched again afterwards. Text Jira refused, or that was
//! declined at the question, is kept as a separate session-local original:
//! `ctrl-r` views/copies it; `c` (or `n`) offers deliberate recovery.
//! A comment and a new ticket add (write mode); a move and an assignment
//! change what is there (edit mode). While radar's mode refuses one its
//! question says so and `y` sends nothing. The question, the mode and the
//! words are the kit's (`radar_core::writes`).
//!
//! Comments and descriptions are typed in a small composer: `⏎` starts a
//! new line, `ctrl-s` shows what will post and asks, `esc` stops (asking
//! first when there is text). They go to Jira as ADF (`adf::from_text`).

use radar_core::design::help_bar::KeyHint;
use radar_core::journal::{
    ActionDescriptor, Origin, Outcome as JournalOutcome, SubjectKey, SubjectRole, SubjectSnapshot,
};
use radar_core::plugin::{Binding, Effect, Menu, MenuItem, Msg, Outcome, Pane};
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use radar_core::ui::composer::{self, Composer, Typed};
use radar_core::ui::icon::Icon;
use radar_core::writes::{self, Service};
use radar_core::log;
use serde_json::Value;

use crate::client;
use crate::model::{Category, IssueType, Me, Transition};
use crate::screen::{self, Selected};
use crate::write_context::Dispatch;
use crate::write_recovery::{self as recovery, Context, Original, Record, Status};
use crate::{JiraMsg, JiraPlugin, detail_key, page, write_context, write_dialog};

pub(crate) const MOVE_KEY: char = 't';
pub(crate) const COMMENT_KEY: char = 'c';
pub(crate) const NEW_KEY: char = 'n';
pub(crate) const ASSIGN_KEY: char = 'i';

const MOVE_MENU: &str = "j-move";
const CONFIRM_MENU: &str = "j-confirm";
const PROJECT_MENU: &str = "j-project";
const TYPE_MENU: &str = "j-type";
const DROP_MENU: &str = "j-drop";

/// Jira, as writes name it.
pub(crate) const SERVICE: Service = Service::new("Jira");

/// A change to Jira, built and waiting for its `y`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Change {
    Move {
        site: String,
        key: String,
        to: Transition,
        from: String,
        from_category: Category,
    },
    Comment {
        site: String,
        key: String,
        text: String,
    },
    Create {
        site: String,
        project: String,
        kind: IssueType,
        summary: String,
        description: String,
    },
    Assign {
        site: String,
        key: String,
        me: Me,
        from: String,
    },
}

impl Change {
    /// Intent and identity only; never the comment, summary, description or assignee.
    pub(crate) fn journal(&self, p: &JiraPlugin) -> Effect {
        Effect::Journal(self.descriptor(p))
    }

    pub(crate) fn descriptor(&self, p: &JiraPlugin) -> ActionDescriptor {
        let operation = match self {
            Self::Move { .. } => "jira.ticket.move",
            Self::Comment { .. } => "jira.comment.post",
            Self::Create { .. } => "jira.ticket.create",
            Self::Assign { .. } => "jira.ticket.assign_self",
        };
        let Effect::Journal(mut action) = journal(
            p,
            operation,
            self.site(),
            self.about(),
            matches!(self, Self::Create { .. }),
        ) else {
            unreachable!()
        };
        if let Some(c) = p
            .compose
            .as_ref()
            .filter(|c| c.change().as_ref() == Some(self) && !c.context.source.is_empty())
        {
            action.subjects.clear();
            if let Ok(key) = SubjectKey::new(
                "jira",
                &c.context.instance,
                if matches!(self, Self::Create { .. }) {
                    "project"
                } else {
                    "ticket"
                },
                self.about(),
            ) {
                action = action.with_subject(SubjectSnapshot::new(key, SubjectRole::Primary));
                action.metadata.redacted = false;
                action.metadata.omitted_subjects = 0;
            } else {
                action.metadata.redacted = true;
                action.metadata.omitted_subjects = 1;
            }
        }
        action
    }

    pub fn site(&self) -> &str {
        match self {
            Self::Move { site, .. } | Self::Comment { site, .. } | Self::Create { site, .. } | Self::Assign { site, .. } => site,
        }
    }

    /// The ticket it is about, or the project a new one goes in.
    pub fn about(&self) -> &str {
        match self {
            Self::Move { key, .. } | Self::Comment { key, .. } | Self::Assign { key, .. } => key,
            Self::Create { project, .. } => project,
        }
    }

    /// Whether it adds (a comment, a new ticket) or changes what is there
    /// (a move, an assignment).
    pub fn kind(&self) -> writes::Change {
        match self {
            Self::Comment { .. } | Self::Create { .. } => writes::Change::Add,
            Self::Move { .. } | Self::Assign { .. } => writes::Change::Edit,
        }
    }

    /// What it does, in words: `move ABC-12 to In Review`.
    pub fn words(&self) -> String {
        match self {
            Self::Move { key, to, .. } => format!("move {key} to {}", to.to),
            Self::Comment { key, .. } => format!("comment on {key}"),
            Self::Create { project, kind, .. } => format!("create a {} in {project}", kind.name),
            Self::Assign { key, .. } => format!("assign {key} to you"),
        }
    }

    /// The request that makes it.
    pub fn request(&self) -> anyhow::Result<client::Req> {
        match self {
            Self::Move { key, to, .. } => client::transition_req(key, &to.id),
            Self::Comment { key, text, .. } => client::comment_req(key, text),
            Self::Create {
                project,
                kind,
                summary,
                description,
                ..
            } => client::create_req(project, &kind.id, summary, description),
            Self::Assign { key, me, .. } => client::assign_req(key, &me.account_id),
        }
    }

    /// What is said once Jira took it (after `✓`): `ABC-12 is In Review now`.
    fn done(&self, answer: &Value) -> String {
        match self {
            Self::Move { key, to, .. } => format!("{key} is {} now", to.to),
            Self::Comment { key, .. } => format!("comment posted on {key}"),
            Self::Create { .. } => format!(
                "created {}",
                answer.get("key").and_then(Value::as_str).unwrap_or("the ticket")
            ),
            Self::Assign { key, me, .. } => format!("{key} is assigned to {}", me.name),
        }
    }
}

pub(crate) fn journal(p: &JiraPlugin, operation: &'static str, site: &str, id: &str, project: bool) -> Effect {
    let mut action = ActionDescriptor::new(operation, Origin::Plugin("jira".into()));
    let instance = p.cfg.site(site).and_then(|s| s.base_url().ok()).unwrap_or_default();
    if let Ok(key) = SubjectKey::new("jira", &instance, if project { "project" } else { "ticket" }, id) {
        action = action.with_subject(SubjectSnapshot::new(key, SubjectRole::Primary));
    } else {
        action.metadata.redacted = true;
        action.metadata.omitted_subjects = 1;
    }
    Effect::Journal(action)
}

/// What text is being typed for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Draft {
    Comment { key: String },
    Summary { project: String, kind: IssueType },
    Description { project: String, kind: IssueType, summary: String },
}

/// Text being typed, in the kit's composer (one line for a summary).
#[derive(Debug)]
pub(crate) struct Compose {
    pub site: String,
    pub draft: Draft,
    pub composer: Composer,
    pub context: Context,
    pub status: Status,
    /// Original description while revisiting the summary stage.
    pub saved_description: String,
}

impl Compose {
    fn new(site: &str, draft: Draft, text: &str) -> Self {
        let composer = if matches!(draft, Draft::Summary { .. }) {
            Composer::one_line(text)
        } else {
            Composer::new(text)
        };
        Self {
            site: site.into(),
            draft,
            composer,
            context: Context::default(),
            status: Status::Unsent,
            saved_description: String::new(),
        }
    }

    /// Everything typed, the line being edited included.
    pub fn text(&self) -> String {
        self.composer.text()
    }

    pub(crate) fn original(&self) -> Original {
        match &self.draft {
            Draft::Comment { .. } => Original::Comment(self.text()),
            Draft::Summary { .. } => Original::Create {
                summary: self.text(),
                description: self.saved_description.clone(),
            },
            Draft::Description { summary, .. } => Original::Create {
                summary: summary.clone(),
                description: self.text(),
            },
        }
    }

    pub(crate) fn change(&self) -> Option<Change> {
        Some(match &self.draft {
            Draft::Comment { key } => Change::Comment {
                site: self.site.clone(),
                key: key.clone(),
                text: self.text().trim_end().to_owned(),
            },
            Draft::Description {
                project,
                kind,
                summary,
            } => Change::Create {
                site: self.site.clone(),
                project: project.clone(),
                kind: kind.clone(),
                summary: summary.trim().to_owned(),
                description: self.text().trim_end().to_owned(),
            },
            Draft::Summary { .. } => return None,
        })
    }

    fn title(&self) -> String {
        match &self.draft {
            Draft::Comment { key } => format!("comment on {key}"),
            Draft::Summary { project, kind } => {
                format!("new {} in {project} · its summary", kind.name)
            }
            Draft::Description { project, kind, .. } => {
                format!(
                    "new {} in {project} · its description (may stay empty)",
                    kind.name
                )
            }
        }
    }
}

/// A change sent, and what came of it.
#[derive(Debug, Clone)]
pub(crate) struct Sent {
    pub dispatch: Option<Dispatch>,
    pub words: String,
    /// `None` while Jira has not answered.
    pub result: Option<Result<String, String>>,
}

/// A new ticket being put together: where, once the type is picked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NewTicket {
    pub site: String,
    pub project: String,
    pub types: Vec<IssueType>,
}

// --- where an action applies ---------------------------------------------------------

/// The ticket the focused pane is about: the page's, or the tree's.
pub(crate) fn target(p: &JiraPlugin, pane: Pane) -> Option<(String, String)> {
    if let Some(page) = p.pages.last() {
        return Some((page.site.clone(), page.key.clone()));
    }
    if pane == Pane::Nav {
        return None;
    }
    match screen::selected(p)? {
        Selected::Ticket(it) => Some((it.site.name.clone(), it.t.key.clone())),
        _ => None,
    }
}

/// The site a new ticket goes on from here, and the project to offer
/// first: the selection's, else the first site that watches one.
fn new_here(p: &JiraPlugin, pane: Pane) -> Option<(String, Option<String>)> {
    if let Some(page) = p.pages.last() {
        let project = page.key.split('-').next().map(str::to_string);
        return Some((page.site.clone(), project));
    }
    let picked = if pane == Pane::Nav {
        None
    } else {
        screen::selected(p)
    };
    match picked {
        Some(Selected::Ticket(it)) => Some((it.site.name.clone(), Some(it.t.project.clone()))),
        Some(Selected::Site(i) | Selected::Group(i, _)) => {
            Some((p.cfg.sites[i].name.clone(), None))
        }
        None => p
            .cfg
            .sites
            .iter()
            .find(|s| !s.projects.is_empty())
            .map(|s| (s.name.clone(), None)),
    }
}

/// The ticket as held: its status and assignee.
fn held<'a>(p: &'a JiraPlugin, site: &str, key: &str) -> Option<&'a crate::model::Ticket> {
    p.details
        .get(&detail_key(site, key))
        .and_then(|f| f.value.as_ref())
        .map(|d| &d.ticket)
        .or_else(|| p.data.get(site).and_then(|d| d.tickets.get(key)))
}

pub(crate) fn me<'a>(p: &'a JiraPlugin, site: &str) -> Option<&'a Me> {
    p.data.get(site).and_then(|d| d.me.as_ref()).filter(|m| !m.account_id.is_empty())
}

/// Why `i` cannot assign this ticket to you now, if it cannot.
pub(crate) fn assign_blocked(p: &JiraPlugin, site: &str, key: &str) -> Option<String> {
    let mine = me(p, site)?;
    held(p, site, key)
        .filter(|t| t.assignee_id == mine.account_id)
        .map(|_| format!("{key} is already yours"))
}

// --- the actions menu's rows ---------------------------------------------------------

/// The `x` rows for changing what the pane is about, under `on Jira`.
pub(crate) fn bindings(p: &JiraPlugin, pane: Pane) -> Vec<Binding> {
    let mut b = Vec::new();
    if let Some((site, key)) = target(p, pane) {
        b.push(Binding::new(MOVE_KEY.to_string(), format!("move {key} to another status…")).edits());
        b.push(Binding::new(COMMENT_KEY.to_string(), format!("comment on {key}…")).adds());
        b.push(
            Binding::new(
                ASSIGN_KEY.to_string(),
                match assign_blocked(p, &site, &key) {
                    Some(why) => format!("assign to me · {why}"),
                    None => "assign it to me…".into(),
                },
            )
            .edits(),
        );
    }
    if let Some((site, project)) = new_here(p, pane)
        && p.cfg.site(&site).is_some_and(|s| !s.projects.is_empty())
    {
        b.push(Binding::new(
            NEW_KEY.to_string(),
            match project {
                Some(pr) => format!("new ticket in {pr}…"),
                None => format!("new ticket on {site}…"),
            },
        ).adds());
    }
    if !b.is_empty() {
        b.insert(0, Binding::heading("on Jira"));
    }
    b
}

/// `t`, `c`, `n`, `i`: start a change, or say why it cannot be made.
pub(crate) fn key(p: &mut JiraPlugin, pane: Pane, key: KeyEvent) -> Option<Outcome> {
    if key.modifiers != KeyModifiers::NONE && key.modifiers != KeyModifiers::SHIFT {
        return None;
    }
    let KeyCode::Char(c) = key.code else {
        return None;
    };
    let fx = match c {
        MOVE_KEY => {
            let (site, key) = target(p, pane)?;
            start_move(p, &site, &key)
        }
        COMMENT_KEY => {
            let (site, key) = target(p, pane)?;
            start_comment(p, &site, &key)
        }
        ASSIGN_KEY => {
            let (site, key) = target(p, pane)?;
            assign_me(p, &site, &key)
        }
        NEW_KEY => {
            let (site, project) = new_here(p, pane)?;
            start_new(p, &site, project.as_deref())
        }
        _ => return None,
    };
    Some(Outcome::Handled(fx))
}

// --- moving a ticket ------------------------------------------------------------------

/// Ask Jira where the ticket can go; the menu comes with the answer.
pub(crate) fn start_move(p: &mut JiraPlugin, site: &str, key: &str) -> Vec<Effect> {
    if !write_dialog::begin(p) {
        return write_dialog::skipped("finish the current editor, then try again");
    }
    let Some(s) = p.cfg.site(site).cloned() else {
        return vec![];
    };
    let id = format!("moves\0{site}\0{key}\0");
    if p.asking.iter().any(|pending| pending.starts_with(&id)) {
        return vec![Effect::Notify("already asking Jira".into())];
    }
    let Effect::Journal(action) = journal(p, "jira.read.moves", site, key, false) else {
        unreachable!()
    };
    let read = p.dialogs.read(action.clone());
    p.asking.insert(format!("{id}{}", read.id));
    let key = key.to_string();
    let provider = p.provider.clone();
    vec![
        Effect::Notify(format!("asking Jira where {key} can go…")),
        Effect::SpawnAction {
            action,
            task: Box::pin(async move {
                let result = async { provider.client(&s)?.transitions(&key).await }
                    .await
                    .map_err(|e| format!("{e:#}"));
                Box::new(JiraMsg::Moves {
                    read,
                    site: s.name,
                    key,
                    result,
                }) as Msg
            }),
        },
    ]
}

/// How usual a move is from `now`. From to do: starting it, then another
/// to-do status, then done. From doing: the next doing step (review, test)
/// before done, back to to do last. From done: reopening. Staying where it
/// is comes last of all; Jira's own order holds within each.
fn rank(now: Category, now_status: &str, t: &Transition) -> u8 {
    if t.to == now_status {
        return 5;
    }
    use Category::{Doing, Done, Todo};
    match (now, t.category) {
        (Todo, Doing) | (Doing, Doing) | (Done, Doing) => 0,
        (Todo, Todo) | (Doing, Done) | (Done, Todo) => 1,
        (Todo, Done) | (Done, Done) => 2,
        (Doing, Todo) => 3,
    }
}

/// The moves in the order they are offered.
pub(crate) fn ordered(now: Category, now_status: &str, mut ts: Vec<Transition>) -> Vec<Transition> {
    ts.sort_by_key(|t| rank(now, now_status, t));
    ts
}

pub(crate) fn on_moves(
    p: &mut JiraPlugin,
    read: write_dialog::Read,
    site: &str,
    key: &str,
    result: Result<Vec<Transition>, String>,
) -> Vec<Effect> {
    let current = p.dialogs.complete(&read) && !write_dialog::occupied(p);
    let outcome = if result.is_ok() {
        JournalOutcome::Succeeded
    } else {
        JournalOutcome::Failed
    };
    let apply = on_moves_result(p, &read, site, key, result, current);
    write_dialog::read_result(&read, outcome, current, "press t again when ready", apply)
}

fn on_moves_result(
    p: &mut JiraPlugin,
    read: &write_dialog::Read,
    site: &str,
    key: &str,
    result: Result<Vec<Transition>, String>,
    current: bool,
) -> Vec<Effect> {
    p.asking
        .remove(&format!("moves\0{site}\0{key}\0{}", read.id));
    let ts = match result {
        Ok(ts) => ts,
        Err(e) => {
            return vec![crate::model::failed_notice(&format!(
                "could not ask where {key} can go: {e}"
            ))];
        }
    };
    let (status, category) = held(p, site, key).map_or((String::new(), Category::Todo), |t| {
        (t.status.clone(), t.category)
    });
    if ts.is_empty() {
        return vec![Effect::Notify(format!("{key} has no moves you may make"))];
    }
    if !current {
        return vec![];
    }
    let ts = ordered(category, &status, ts);
    let items = move_items(category, &status, &ts);
    p.moving = Some((site.to_string(), key.to_string(), ts));
    vec![write_dialog::menu(
        p,
        Menu::new(MOVE_MENU, format!("move {key} to…"), items),
    )]
}

/// A workflow with this many moves or more is grouped.
const GROUP_FROM: usize = 7;
/// How many of the usual next moves are set apart at the top of a grouped list.
const USUAL: usize = 3;

/// The rows of the moves menu, for moves already [`ordered`]. A short list
/// is one run under "now". A long one (a workflow with many "doing"
/// statuses) is the usual next moves first, then every other move under
/// its status category, so it scans by where each leads. Keys 1 to 9 go to
/// the first nine moves as shown; headings are never chosen.
pub(crate) fn move_items(category: Category, status: &str, ts: &[Transition]) -> Vec<MenuItem> {
    let mut items = Vec::new();
    if !status.is_empty() {
        let (icon, mark) = crate::model::mark_or(category);
        items.push(MenuItem::heading(format!("now {mark}{status}")).icon(icon));
    }
    let row = |n: usize, t: &Transition| {
        let (icon, mark) = crate::model::mark_or(t.category);
        let label = format!("{mark}{}", t.to);
        let mut hint = if t.name != t.to { format!("Jira's “{}”", t.name) } else { String::new() };
        if t.to == status {
            hint = if hint.is_empty() { "stays here".into() } else { format!("{hint} · stays here") };
        }
        match char::from_digit(n as u32 + 1, 10) {
            Some(d) if n < 9 => MenuItem::new(d, label, hint, t.id.clone()),
            _ => MenuItem::pick(label, hint, t.id.clone()),
        }
        .icon(icon)
    };
    if ts.len() < GROUP_FROM {
        items.extend(ts.iter().enumerate().map(|(n, t)| row(n, t)));
        return items;
    }
    let usual: Vec<&Transition> = ts.iter().filter(|t| t.to != status).take(USUAL).collect();
    let mut n = 0;
    items.push(MenuItem::heading("usual next"));
    for t in &usual {
        items.push(row(n, t));
        n += 1;
    }
    for (c, name) in [(Category::Todo, "to do"), (Category::Doing, "in progress"), (Category::Done, "done")] {
        let group: Vec<&Transition> = ts
            .iter()
            .filter(|t| t.category == c && !usual.iter().any(|u| u.id == t.id))
            .collect();
        if group.is_empty() {
            continue;
        }
        let (icon, mark) = crate::model::mark_or(c);
        items.push(MenuItem::heading(format!("{mark}{name} · {}", group.len())).icon(icon));
        for t in group {
            items.push(row(n, t));
            n += 1;
        }
    }
    items
}

// --- commenting ---------------------------------------------------------------------

pub(crate) fn start_comment(p: &mut JiraPlugin, site: &str, key: &str) -> Vec<Effect> {
    if !write_dialog::begin(p) {
        return write_dialog::skipped("finish the current editor, then try again");
    }
    let destination = recovery::Destination::comment(p, site, key);
    if recovery::has_target(p, &destination) {
        return recovery::list(p, Some(destination));
    }
    let context = Context::capture(p, site);
    open_compose(
        p,
        site,
        Draft::Comment { key: key.into() },
        "",
        context,
        Status::Unsent,
        String::new(),
    );
    vec![]
}

pub(crate) fn open_compose(
    p: &mut JiraPlugin,
    site: &str,
    draft: Draft,
    text: &str,
    context: Context,
    status: Status,
    saved_description: String,
) {
    p.dialogs.invalidate();
    p.change = None;
    let mut compose = Compose::new(site, draft, text);
    compose.context = context;
    compose.status = status;
    compose.saved_description = saved_description;
    p.compose = Some(compose);
}

/// Keys while text is being typed.
pub(crate) fn compose_key(p: &mut JiraPlugin, key: KeyEvent) -> Outcome {
    let Some(c) = p.compose.as_mut() else {
        return Outcome::Ignored;
    };
    match c.composer.key(key) {
        Typed::Review => Outcome::Handled(review(p)),
        Typed::Stop if matches!(c.original(), Original::Create { ref summary, ref description } if !summary.is_empty() || !description.is_empty()) =>
        {
            let menu = composer::stop_menu(DROP_MENU, what_typed(c));
            Outcome::Handled(vec![write_dialog::menu(p, menu)])
        }
        Typed::Stop => {
            p.dialogs.invalidate();
            p.change = None;
            p.compose = None;
            Outcome::done()
        }
        Typed::AskStop => {
            let menu = composer::stop_menu(DROP_MENU, what_typed(c));
            Outcome::Handled(vec![write_dialog::menu(p, menu)])
        }
        Typed::Typing => Outcome::done(),
    }
}

fn what_typed(c: &Compose) -> &'static str {
    match c.draft {
        Draft::Comment { .. } => "comment",
        Draft::Summary { .. } | Draft::Description { .. } => "new ticket",
    }
}

pub(crate) fn compose_paste(p: &mut JiraPlugin, text: &str) {
    if let Some(c) = p.compose.as_mut() {
        c.composer.paste(text);
    }
}

/// `ctrl-s` (or `⏎` on a summary): the next step, or what will be sent
/// and the question.
fn review(p: &mut JiraPlugin) -> Vec<Effect> {
    let Some(c) = p.compose.as_ref() else {
        return vec![];
    };
    let text = c.text();
    match &c.draft {
        Draft::Comment { key } => {
            if text.trim().is_empty() {
                return vec![Effect::Notify(
                    "nothing to post yet: type the comment".into(),
                )];
            }
            let change = Change::Comment {
                site: c.site.clone(),
                key: key.clone(),
                text: text.trim_end().to_string(),
            };
            ask(p, change)
        }
        Draft::Summary { project, kind } => {
            let summary = text.trim().to_string();
            if summary.is_empty() {
                return vec![Effect::error(None, "not yet: a ticket needs a summary")];
            }
            let draft = Draft::Description {
                project: project.clone(),
                kind: kind.clone(),
                summary: text,
            };
            let site = c.site.clone();
            let context = c.context.clone();
            let status = c.status;
            let description = c.saved_description.clone();
            open_compose(
                p,
                &site,
                draft,
                &description,
                context,
                status,
                String::new(),
            );
            vec![]
        }
        Draft::Description {
            project,
            kind,
            summary,
        } => {
            let change = Change::Create {
                site: c.site.clone(),
                project: project.clone(),
                kind: kind.clone(),
                summary: summary.trim().to_string(),
                description: text.trim_end().to_string(),
            };
            ask(p, change)
        }
    }
}

/// `text` as Jira will show it (it goes as ADF), laid out 72 wide.
fn shown(text: &str) -> String {
    let doc = crate::adf::doc(&crate::adf::from_text(text));
    crate::page::text_at(&doc, 72).trim_end_matches('\n').to_string()
}

/// The question before `change` is sent, showing what will be sent: the
/// kit's write confirm (the whole text) or its plain one.
fn ask(p: &mut JiraPlugin, change: Change) -> Vec<Effect> {
    let mut menu = match &change {
        Change::Move {
            key,
            to,
            from,
            from_category,
            ..
        } => Menu::write_plain(
            CONFIRM_MENU,
            format!("move {key} to {}?", to.to),
            vec![format!(
                "{}{from} → {}{} (Jira's “{}”)",
                crate::model::mark(*from_category),
                crate::model::mark(to.category),
                to.to,
                to.name
            )],
            "move it",
            format!("leave it {from}"),
        ),
        Change::Comment { key, text, .. } => Menu::write_confirm(
            CONFIRM_MENU,
            format!("post this on {key}?"),
            &shown(text),
            "post it",
            "drop it",
        ),
        Change::Create {
            project,
            kind,
            summary,
            description,
            site,
        } => {
            let mut all = format!("{} · {summary}", kind.name);
            if !description.trim().is_empty() {
                all.push_str("\n\n");
                all.push_str(&shown(description));
            }
            Menu::write_confirm(
                CONFIRM_MENU,
                format!("create this in {project} on {site}?"),
                &all,
                "create it",
                "drop it",
            )
        }
        Change::Assign { key, me, from, .. } => Menu::write_plain(
            CONFIRM_MENU,
            format!("assign {key} to you?"),
            vec![format!(
                "{} → {} (you)",
                if from.is_empty() { "unassigned" } else { from },
                me.name
            )],
            "assign it",
            format!(
                "leave it with {}",
                if from.is_empty() { "nobody" } else { from }
            ),
        ),
    };
    if p.compose
        .as_ref()
        .is_some_and(|c| c.status == Status::Uncertain && c.change().as_ref() == Some(&change))
    {
        menu = menu.saying(Status::Uncertain.words());
    }
    // What the question is about, after the host's warning.
    let icon = match &change {
        Change::Move { to, .. } => to.category.icon(),
        Change::Comment { .. } => radar_core::ui::icon::Icon::Comment,
        Change::Create { kind, .. } => crate::model::kind_icon(&kind.name, false),
        Change::Assign { .. } => radar_core::ui::icon::Icon::You,
    };
    let journal = change.journal(p);
    p.change = Some(change);
    // The kit's write confirm adds, its plain one edits: as `kind` says.
    vec![journal, write_dialog::menu(p, menu.icon(icon))]
}

// --- a new ticket ----------------------------------------------------------------------

pub(crate) fn start_new(p: &mut JiraPlugin, site: &str, project: Option<&str>) -> Vec<Effect> {
    if !write_dialog::begin(p) {
        return write_dialog::skipped("finish the current editor, then try again");
    }
    let Some(s) = p.cfg.site(site) else {
        return vec![];
    };
    let watched: Vec<String> = s.projects.iter().map(|pr| pr.key.clone()).collect();
    match project.filter(|pr| watched.iter().any(|w| w == pr)) {
        Some(pr) => new_in_project(p, site, pr),
        None if watched.len() == 1 => new_in_project(p, site, &watched[0]),
        None if watched.is_empty() => vec![Effect::Notify(format!(
            "{site} watches no project yet: settings, projects adds one"
        ))],
        None => {
            let items = watched
                .iter()
                .enumerate()
                .map(|(i, k)| match char::from_digit(i as u32 + 1, 10) {
                    Some(d) if i < 9 => {
                        MenuItem::new(d, k.clone(), format!("on {site}"), format!("{site}\0{k}"))
                    }
                    _ => MenuItem::pick(k.clone(), format!("on {site}"), format!("{site}\0{k}")),
                })
                .collect();
            vec![write_dialog::menu(
                p,
                Menu::new(PROJECT_MENU, "a new ticket in which project?", items),
            )]
        }
    }
}

fn new_in_project(p: &mut JiraPlugin, site: &str, project: &str) -> Vec<Effect> {
    let destination = recovery::Destination::create(p, site, project);
    if recovery::has_target(p, &destination) {
        recovery::list(p, Some(destination))
    } else {
        ask_types(p, site, project)
    }
}

pub(crate) fn ask_types(p: &mut JiraPlugin, site: &str, project: &str) -> Vec<Effect> {
    let Some(s) = p.cfg.site(site).cloned() else {
        return vec![];
    };
    let id = format!("types\0{site}\0{project}\0");
    if p.asking.iter().any(|pending| pending.starts_with(&id)) {
        return vec![Effect::Notify("already asking Jira".into())];
    }
    let Effect::Journal(action) = journal(p, "jira.read.types", site, project, true) else {
        unreachable!()
    };
    let read = p.dialogs.read(action.clone());
    p.asking.insert(format!("{id}{}", read.id));
    let project = project.to_string();
    let provider = p.provider.clone();
    vec![
        Effect::Notify(format!("asking Jira what tickets {project} takes…")),
        Effect::SpawnAction {
            action,
            task: Box::pin(async move {
                let result = async { provider.client(&s)?.issue_types(&project).await }
                    .await
                    .map_err(|e| format!("{e:#}"));
                Box::new(JiraMsg::Types {
                    read,
                    site: s.name,
                    project,
                    result,
                }) as Msg
            }),
        },
    ]
}

pub(crate) fn on_types(
    p: &mut JiraPlugin,
    read: write_dialog::Read,
    site: &str,
    project: &str,
    result: Result<Vec<IssueType>, String>,
) -> Vec<Effect> {
    let current = p.dialogs.complete(&read) && !write_dialog::occupied(p);
    let outcome = if result.is_ok() {
        JournalOutcome::Succeeded
    } else {
        JournalOutcome::Failed
    };
    let apply = on_types_result(p, &read, site, project, result, current);
    write_dialog::read_result(&read, outcome, current, "press n again when ready", apply)
}

fn on_types_result(
    p: &mut JiraPlugin,
    read: &write_dialog::Read,
    site: &str,
    project: &str,
    result: Result<Vec<IssueType>, String>,
    current: bool,
) -> Vec<Effect> {
    p.asking
        .remove(&format!("types\0{site}\0{project}\0{}", read.id));
    let types = match result {
        Ok(t) if t.is_empty() => {
            return vec![Effect::Notify(format!(
                "{project} takes no tickets you may create"
            ))];
        }
        Ok(t) => t,
        Err(e) => {
            return vec![crate::model::failed_notice(&format!(
                "could not ask what {project} takes: {e}"
            ))];
        }
    };
    if !current {
        return vec![];
    }
    let items = types
        .iter()
        .enumerate()
        .map(|(i, t)| match char::from_digit(i as u32 + 1, 10) {
            Some(d) if i < 9 => MenuItem::new(d, t.name.clone(), "", t.id.clone()),
            _ => MenuItem::pick(t.name.clone(), "", t.id.clone()),
        })
        .collect();
    p.new_ticket = Some(NewTicket {
        site: site.into(),
        project: project.into(),
        types,
    });
    vec![write_dialog::menu(
        p,
        Menu::new(
            TYPE_MENU,
            format!("a new ticket in {project}: which type?"),
            items,
        ),
    )]
}

// --- assigning ------------------------------------------------------------------------

pub(crate) fn assign_me(p: &mut JiraPlugin, site: &str, key: &str) -> Vec<Effect> {
    if !write_dialog::begin(p) {
        return write_dialog::skipped("finish the current editor, then try again");
    }
    if let Some(why) = assign_blocked(p, site, key) {
        return vec![Effect::Notify(why)];
    }
    match me(p, site).cloned() {
        Some(me) => {
            let from = held(p, site, key)
                .map(|t| t.assignee.clone())
                .unwrap_or_default();
            ask(
                p,
                Change::Assign {
                    site: site.into(),
                    key: key.into(),
                    me,
                    from,
                },
            )
        }
        // Who you are is asked once, then the question comes.
        None => {
            let Some(s) = p.cfg.site(site).cloned() else {
                return vec![];
            };
            let id = format!("me\0{site}\0{key}\0");
            if p.asking.iter().any(|pending| pending.starts_with(&id)) {
                return vec![Effect::Notify("already asking Jira".into())];
            }
            let Effect::Journal(action) = journal(p, "jira.read.who", site, key, false) else {
                unreachable!()
            };
            let read = p.dialogs.read(action.clone());
            p.asking.insert(format!("{id}{}", read.id));
            let key = key.to_string();
            let provider = p.provider.clone();
            vec![Effect::SpawnAction {
                action,
                task: Box::pin(async move {
                    let result = async { provider.client(&s)?.myself().await }
                        .await
                        .map_err(|e| format!("{e:#}"));
                    Box::new(JiraMsg::Who {
                        read,
                        site: s.name,
                        key,
                        result,
                    }) as Msg
                }),
            }]
        }
    }
}

pub(crate) fn on_who(
    p: &mut JiraPlugin,
    read: write_dialog::Read,
    site: &str,
    key: &str,
    result: Result<Me, String>,
) -> Vec<Effect> {
    let current = p.dialogs.complete(&read) && !write_dialog::occupied(p);
    let outcome = if result.is_ok() {
        JournalOutcome::Succeeded
    } else {
        JournalOutcome::Failed
    };
    let apply = on_who_result(p, &read, site, key, result, current);
    write_dialog::read_result(&read, outcome, current, "press i again when ready", apply)
}

fn on_who_result(
    p: &mut JiraPlugin,
    read: &write_dialog::Read,
    site: &str,
    key: &str,
    result: Result<Me, String>,
    current: bool,
) -> Vec<Effect> {
    p.asking.remove(&format!("me\0{site}\0{key}\0{}", read.id));
    match result {
        Ok(me) if !me.account_id.is_empty() => {
            p.data.entry(site.to_string()).or_default().me = Some(me);
            if !current {
                return vec![];
            }
            assign_me(p, site, key)
        }
        Ok(_) => vec![crate::model::failed_notice(&format!(
            "Jira did not say who you are on {site}"
        ))],
        Err(e) => vec![crate::model::failed_notice(&format!(
            "could not ask who you are on {site}: {e}"
        ))],
    }
}

// --- sending ------------------------------------------------------------------------------

/// What a change is known by while it is sent: the ticket, or the project.
fn sent_id(change: &Change) -> String {
    detail_key(change.site(), change.about())
}

pub(crate) fn send(p: &mut JiraPlugin, change: Change) -> Vec<Effect> {
    let action = change.descriptor(p);
    dispatch(p, change, action).0
}

/// The optional context proves this call actually accepted a dispatch. A refused
/// first step must not leave a continuation waiting for some other completion.
pub(crate) fn dispatch(
    p: &mut JiraPlugin,
    change: Change,
    action: ActionDescriptor,
) -> (Vec<Effect>, Option<Dispatch>) {
    let mut fx = vec![
        Effect::Journal(action.clone()),
        Effect::Report(JournalOutcome::Confirmed),
    ];
    write_context::reconcile(p);
    let Some(site) = p.cfg.site(change.site()).cloned() else {
        fx.push(Effect::Report(JournalOutcome::Blocked));
        return (fx, None);
    };
    let id = sent_id(&change);
    if p.sent.get(&id).is_some_and(|s| s.result.is_none()) {
        fx.extend([
            Effect::Report(JournalOutcome::Blocked),
            Effect::Notify("Jira has not answered the last change yet".into()),
        ]);
        return (fx, None);
    }
    let record = Record::capture(p, &change);
    if record
        .as_ref()
        .is_some_and(|r| r.context.generation != p.connections.generation(change.site()))
    {
        fx.extend([Effect::Report(JournalOutcome::Blocked), Effect::Notify("Jira connection changed; original text is still here; copy or keep it before choosing a new destination".into())]);
        return (fx, None);
    }
    let reserved = if let Some(record) = record {
        match p.recovery.reserve(record) {
            Ok(id) => Some(id),
            Err(()) => {
                fx.push(Effect::Report(JournalOutcome::Blocked));
                fx.extend(recovery::full());
                return (fx, None);
            }
        }
    } else {
        None
    };
    consume_review(p, &change);
    if let Some(why) = writes::not_sent(change.kind()) {
        let mut refused = finish(p, change, Err(why), JournalOutcome::Blocked, None);
        if reserved.is_some_and(|id| p.recovery.records[&id].status == Status::Uncertain) {
            for effect in &mut refused {
                let message = match effect {
                    Effect::Notify(message) | Effect::NotifyWith(_, message) => message,
                    Effect::Notice(message) => &mut message.text,
                    _ => continue,
                };
                message.push_str(&format!(
                    "; current attempt not sent; {}",
                    Status::Uncertain.words()
                ));
            }
        }
        fx.extend(refused);
        return (fx, None);
    }
    p.next_write = p
        .next_write
        .checked_add(1)
        .expect("Jira write sequence exhausted");
    let dispatch = Dispatch {
        id: p.next_write,
        generation: p
            .connections
            .generation(&site.name)
            .expect("configured site"),
        action,
    };
    if let Some(id) = reserved {
        p.recovery.accept(id, &dispatch);
    }
    let words = change.words();
    p.sent.insert(
        id,
        Sent {
            dispatch: Some(dispatch.clone()),
            words: words.clone(),
            result: None,
        },
    );
    log::info(format!("jira {}: {words}", site.name));
    let sent = dispatch.clone();
    let provider = p.provider.clone();
    fx.extend([
        Effect::Report(JournalOutcome::Started),
        Effect::Notify(SERVICE.asking(&words)),
        Effect::Spawn(Box::pin(async move {
            let result = async {
                let req = change.request()?;
                provider.client(&site)?
                    .write(req, &format!("ticket {}", change.about()))
                    .await
            }
            .await
            .map_err(|e| format!("{e:#}"));
            Box::new(JiraMsg::Wrote {
                dispatch: sent,
                change,
                result,
            }) as Msg
        })),
    ]);
    (fx, Some(dispatch))
}

pub(crate) fn on_wrote(
    p: &mut JiraPlugin,
    change: Change,
    result: Result<Value, String>,
    dispatch: &Dispatch,
) -> Vec<Effect> {
    // Transport errors can leave a committed write without an acknowledgement.
    let outcome = if result.is_ok() {
        JournalOutcome::Succeeded
    } else {
        JournalOutcome::Unknown
    };
    let created = if let (Change::Create { .. }, Ok(answer)) = (&change, &result) {
        let mut action =
            ActionDescriptor::new("jira.ticket.create.result", Origin::Plugin("jira".into()));
        let key = answer
            .get("key")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let instance = dispatch
            .action
            .subjects
            .first()
            .map_or("", |s| s.key.instance.as_str());
        if let Ok(subject) = SubjectKey::new("jira", instance, "ticket", key) {
            action = action.with_subject(SubjectSnapshot::new(subject, SubjectRole::Result));
        } else {
            action.metadata.redacted = true;
            action.metadata.omitted_subjects = 1;
        }
        Some(action)
    } else {
        None
    };
    let mut fx = finish(p, change, result, outcome, Some(dispatch));
    if let Some(action) = created {
        // Keep the actual task outcome, then identify the object it created.
        fx.splice(
            1..1,
            [
                Effect::Journal(action),
                Effect::Report(JournalOutcome::Succeeded),
            ],
        );
    }
    fx
}

fn finish(
    p: &mut JiraPlugin,
    change: Change,
    result: Result<Value, String>,
    outcome: JournalOutcome,
    dispatch: Option<&Dispatch>,
) -> Vec<Effect> {
    let id = sent_id(&change);
    let words = change.words();
    let owns = dispatch.is_none_or(|d| {
        p.sent
            .get(&id)
            .filter(|s| s.result.is_none())
            .and_then(|s| s.dispatch.as_ref())
            .is_some_and(|sent| sent.id == d.id)
    });
    let text_change = matches!(change, Change::Comment { .. } | Change::Create { .. });
    let owns = owns && dispatch.is_none_or(|d| !text_change || p.recovery.owns(&change, d));
    if owns && let Some(dispatch) = dispatch {
        p.recovery.finish(&change, dispatch, result.is_ok());
    }
    let current = dispatch.is_none_or(|d| write_context::current(p, change.site(), d));
    match result {
        Ok(answer) => {
            let said = change.done(&answer);
            if owns {
                p.sent.insert(
                    id,
                    Sent {
                        dispatch: dispatch.cloned(),
                        words,
                        result: Some(Ok(said.clone())),
                    },
                );
            }
            let mut fx = vec![Effect::Report(outcome), writes::done(&said)];
            if !current || !owns {
                if let Some(dispatch) = dispatch {
                    fx.push(write_context::skipped(dispatch));
                }
                // The remote success remains true; only its local follow-ups are
                // skipped. Never fetch/open the old key on a replacement site.
                return fx;
            }
            match &change {
                Change::Create { site, .. } => {
                    if let Some(key) = answer.get("key").and_then(Value::as_str) {
                        fx.extend(page::open(p, site, key));
                    }
                }
                other => fx.extend(page::fetch(p, other.site(), other.about(), true)),
            }
            fx.extend(p.refresh());
            fx
        }
        Err(e) => {
            log::warn(format!("jira {}: {words} failed: {e}", change.site()));
            let mut said = if dispatch.is_some() && text_change {
                format!(
                    "Jira did not confirm {words}: {e}; {}",
                    Status::Uncertain.words()
                )
            } else if dispatch.is_some() {
                format!("Jira did not confirm {words}: {e}; check Jira before trying again")
            } else {
                SERVICE.did_not(&words, &e)
            };
            if owns {
                if text_change {
                    said.push_str("; original text kept · ctrl-r");
                }
                p.sent.insert(
                    id,
                    Sent {
                        dispatch: dispatch.cloned(),
                        words,
                        result: Some(Err(said.clone())),
                    },
                );
            }
            let mut fx = vec![Effect::Report(outcome), writes::failed(&said)];
            if !owns && let Some(dispatch) = dispatch {
                fx.push(write_context::skipped(dispatch));
            }
            fx
        }
    }
}

/// Consume only the reviewed editor belonging to this exact normalized change.
fn consume_review(p: &mut JiraPlugin, change: &Change) {
    if p.change.as_ref() == Some(change) {
        p.change = None;
    }
    if p.compose
        .as_ref()
        .is_some_and(|c| c.change().as_ref() == Some(change))
    {
        p.compose = None;
    }
}

// --- menus and messages -----------------------------------------------------------------

/// A choice in one of these menus; `None` when the menu is not theirs.
pub(crate) fn on_menu(p: &mut JiraPlugin, menu: &str, value: &str) -> Option<Vec<Effect>> {
    let (kind, _) = menu.split_once(':')?;
    if !matches!(
        kind,
        MOVE_MENU | CONFIRM_MENU | DROP_MENU | PROJECT_MENU | TYPE_MENU
    ) {
        return None;
    }
    if !write_dialog::claim(p, menu) {
        return Some(write_dialog::skipped("open the action again when ready"));
    }
    let menu = kind;
    Some(match menu {
        MOVE_MENU => {
            let Some((site, key, ts)) = p.moving.take() else {
                return Some(vec![]);
            };
            let Some(to) = ts.into_iter().find(|t| t.id == value) else {
                return Some(vec![]);
            };
            let (from, from_category) = held(p, &site, &key)
                .map_or((String::new(), Category::Todo), |t| {
                    (t.status.clone(), t.category)
                });
            ask(
                p,
                Change::Move {
                    site,
                    key,
                    to,
                    from,
                    from_category,
                },
            )
        }
        CONFIRM_MENU => match value {
            "yes" => {
                // Refuse before consuming the intent or editor: a pending write
                // must not discard newer text, cursor state, or its next review.
                if let Some(change) = p.change.as_ref()
                    && p.sent
                        .get(&sent_id(change))
                        .is_some_and(|s| s.result.is_none())
                {
                    return Some(vec![
                        change.journal(p),
                        Effect::Report(JournalOutcome::Confirmed),
                        Effect::Report(JournalOutcome::Blocked),
                        Effect::Notify("Jira has not answered the last change yet".into()),
                    ]);
                }
                let Some(change) = p.change.clone() else {
                    return Some(vec![]);
                };
                send(p, change)
            }
            "edit" => {
                p.change = None;
                vec![Effect::Report(JournalOutcome::Cancelled)]
            }
            _ => {
                let Some(change) = p.change.clone() else {
                    return Some(vec![]);
                };
                let record = Record::capture(p, &change);
                let kept = record.is_some();
                let uncertain = record
                    .as_ref()
                    .is_some_and(|r| r.status == Status::Uncertain);
                if let Some(record) = record
                    && p.recovery.reserve(record).is_err()
                {
                    let mut fx = vec![Effect::Report(JournalOutcome::Cancelled)];
                    fx.extend(recovery::full());
                    return Some(fx);
                }
                consume_review(p, &change);
                let mut said = if kept {
                    "nothing sent; original text kept · ctrl-r"
                } else {
                    "nothing sent"
                }
                .to_string();
                if uncertain {
                    said = format!(
                        "current attempt not sent; {}; original text kept · ctrl-r",
                        Status::Uncertain.words()
                    );
                }
                vec![
                    Effect::Report(JournalOutcome::Cancelled),
                    Effect::Notify(said),
                ]
            }
        },
        DROP_MENU => {
            if value == "yes" {
                p.change = None;
                p.compose = None;
                vec![Effect::Notify("dropped; nothing sent".into())]
            } else {
                vec![]
            }
        }
        PROJECT_MENU => {
            let (site, project) = value.split_once('\0')?;
            let (site, project) = (site.to_string(), project.to_string());
            new_in_project(p, &site, &project)
        }
        TYPE_MENU => {
            let Some(nt) = p.new_ticket.take() else {
                return Some(vec![]);
            };
            let Some(kind) = nt.types.into_iter().find(|t| t.id == value) else {
                return Some(vec![]);
            };
            let context = Context::capture(p, &nt.site);
            open_compose(
                p,
                &nt.site,
                Draft::Summary {
                    project: nt.project,
                    kind,
                },
                "",
                context,
                Status::Unsent,
                String::new(),
            );
            vec![]
        }
        _ => return None,
    })
}

pub(crate) fn busy(p: &JiraPlugin) -> bool {
    !p.asking.is_empty() || p.sent.values().any(|s| s.result.is_none())
}

// --- drawing ----------------------------------------------------------------------------

/// The last change to a ticket and what came of it, for its page and
/// detail pane.
pub(crate) fn outcome(p: &JiraPlugin, site: &str, key: &str) -> Option<radar_core::view::Text> {
    use radar_core::view::{Ink, Seg, Text};
    let s = p.sent.get(&detail_key(site, key))?;
    Some(match &s.result {
        None => Text::new()
            .push(Seg::icon(Icon::RunRunning))
            .push(Seg::new(format!("asking Jira to {}…", s.words), Ink::Info)),
        Some(Ok(said)) => Text::new().push(Seg::icon(Icon::RunPassed)).push(Seg::new(said.clone(), Ink::Good)),
        Some(Err(said)) => Text::new()
            .push(Seg::icon(Icon::Error))
            .push(Seg::styled(said.clone(), p.cfg.style_for("failed"))),
    })
}

/// The composer, over the foot of the body, as an editor.
pub(crate) fn overlay(p: &JiraPlugin) -> Option<radar_core::view::PaneView> {
    let c = p.compose.as_ref()?;
    let title = if c.status == Status::Uncertain {
        format!("{} · outcome uncertain; check Jira", c.title())
    } else {
        c.title()
    };
    let editor = radar_core::view::Editor {
        title: title.clone(),
        lines: c.composer.lines.clone(),
        line: c.composer.input.value(),
        caret: c.composer.input.cursor(),
        selection: c.composer.input.selection(),
    };
    Some(radar_core::view::PaneView::new(radar_core::view::PaneId::Middle, title, editor.node()))
}

/// The status line's keys while typing.
pub(crate) fn hints(p: &JiraPlugin) -> Option<Vec<KeyHint<'static>>> {
    Some(p.compose.as_ref()?.composer.hints())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_records_ticket_scope_and_outcome_without_comment_or_error_text() {
        let mut p = crate::demo::sample();
        p.cfg
            .sites
            .iter_mut()
            .find(|s| s.name == "acme")
            .unwrap()
            .url = "https://user:password@acme.atlassian.net/?token=secret".into();
        let change = Change::Comment {
            site: "acme".into(),
            key: "ABC-12".into(),
            text: "private authored text".into(),
        };
        let fx = ask(&mut p, change.clone());
        let Effect::Journal(action) = &fx[0] else {
            panic!("missing journal intent")
        };
        assert_eq!(action.kind.as_str(), "jira.comment.post");
        assert_eq!(
            action.subjects[0].key.instance,
            "https://acme.atlassian.net/"
        );
        assert_eq!(action.subjects[0].key.id, "ABC-12");
        let stored = format!("{action:?}");
        for private in ["password", "secret", "private authored", "token"] {
            assert!(!stored.contains(private), "journal contains {private}");
        }
        let menu = p.dialogs.menu_id();
        assert!(
            on_menu(&mut p, &menu, "no")
                .unwrap()
                .iter()
                .any(|e| matches!(e, Effect::Report(JournalOutcome::Cancelled)))
        );
        writes::set_mode(writes::Mode::Write, None);
        let _ = send(&mut p, change.clone());
        let dispatch = p.sent[&sent_id(&change)].dispatch.clone().unwrap();
        assert!(matches!(
            on_wrote(
                &mut p,
                change.clone(),
                Ok(Value::Null),
                &dispatch
            )[0],
            Effect::Report(JournalOutcome::Succeeded)
        ));
        assert!(matches!(
            on_wrote(
                &mut p,
                change.clone(),
                Err("raw secret error".into()),
                &dispatch
            )[0],
            Effect::Report(JournalOutcome::Unknown)
        ));
        let old = writes::mode();
        writes::set_mode(writes::Mode::ReadOnly, None);
        let blocked = send(&mut p, change);
        writes::set_mode(old.0, old.1);
        assert!(
            blocked
                .iter()
                .any(|e| matches!(e, Effect::Report(JournalOutcome::Blocked)))
        );
        assert!(!blocked.iter().any(|e| matches!(e, Effect::Spawn(_))));
    }

    #[test]
    fn created_ticket_result_uses_the_instance_captured_before_the_write() {
        let mut p = crate::demo::sample();
        let change = Change::Create {
            site: "acme".into(), project: "ABC".into(),
            kind: IssueType { id: "1".into(), name: "Task".into(), subtask: false },
            summary: "private summary".into(), description: "private description".into(),
        };
        p.cfg.sites[0].url = "https://original.example".into();
        writes::set_mode(writes::Mode::Write, None);
        let _ = send(&mut p, change.clone());
        let dispatch = p.sent[&sent_id(&change)].dispatch.clone().unwrap();
        p.cfg.sites[0].url = "https://replacement.example".into();
        let fx = on_wrote(&mut p, change, Ok(serde_json::json!({"key": "ABC-31"})), &dispatch);
        let Effect::Journal(action) = &fx[1] else { panic!("missing created ticket") };
        assert_eq!(action.kind.as_str(), "jira.ticket.create.result");
        assert_eq!(action.subjects[0].key.instance, "https://original.example/");
        assert_eq!(action.subjects[0].key.id, "ABC-31");
        assert_eq!(action.subjects[0].role, SubjectRole::Result);
        assert!(!format!("{action:?}").contains("private"));
    }

    fn t(id: &str, to: &str, c: Category) -> Transition {
        Transition {
            id: id.into(),
            name: to.into(),
            to: to.into(),
            category: c,
        }
    }

    #[test]
    fn the_usual_next_move_comes_first() {
        let ts = vec![
            t("1", "To Do", Category::Todo),
            t("2", "Done", Category::Done),
            t("3", "In Review", Category::Doing),
            t("4", "In Progress", Category::Doing),
        ];
        let got: Vec<String> = ordered(Category::Doing, "In Progress", ts.clone()).into_iter().map(|t| t.to).collect();
        assert_eq!(got, ["In Review", "Done", "To Do", "In Progress"]);
        let got: Vec<String> = ordered(Category::Todo, "To Do", ts.clone()).into_iter().map(|t| t.to).collect();
        assert_eq!(got, ["In Review", "In Progress", "Done", "To Do"]);
        let got: Vec<String> = ordered(Category::Done, "Done", ts).into_iter().map(|t| t.to).collect();
        assert_eq!(got, ["In Review", "In Progress", "To Do", "Done"]);
    }

    #[test]
    fn a_long_list_of_moves_is_the_usual_ones_then_each_status_category() {
        let ts = vec![
            t("1", "To Do", Category::Todo),
            t("2", "Done", Category::Done),
            t("3", "In Review", Category::Doing),
            t("4", "In Progress", Category::Doing),
            t("5", "In Test", Category::Doing),
            t("6", "Blocked", Category::Doing),
            t("7", "Ready for release", Category::Doing),
            t("8", "Won't do", Category::Done),
        ];
        // Short lists stay one run.
        let short = move_items(
            Category::Doing,
            "In Progress",
            &ordered(Category::Doing, "In Progress", ts[..3].to_vec()),
        );
        assert!(
            short
                .iter()
                .all(|i| i.heading == (i.label.starts_with("now"))),
            "only the now row is a heading"
        );
        // Long: the usual three, then the rest by category, each once.
        let ordered_ts = ordered(Category::Doing, "In Progress", ts);
        let items = move_items(Category::Doing, "In Progress", &ordered_ts);
        let rows: Vec<String> = items
            .iter()
            .map(|i| {
                if i.heading {
                    format!("# {}", i.label)
                } else {
                    format!("{} {}", i.keys, i.label)
                }
            })
            .collect();
        assert_eq!(
            rows,
            [
                "# now ◐ In Progress",
                "# usual next",
                "1 ◐ In Review",
                "2 ◐ In Test",
                "3 ◐ Blocked",
                "# ○ to do · 1",
                "4 ○ To Do",
                "# ◐ in progress · 2",
                "5 ◐ Ready for release",
                "6 ◐ In Progress",
                "# ● done · 2",
                "7 ● Done",
                "8 ● Won't do",
            ]
        );
        let mut values: Vec<&str> = items
            .iter()
            .filter(|i| !i.heading)
            .map(|i| i.value.as_str())
            .collect();
        values.sort();
        assert_eq!(
            values,
            ["1", "2", "3", "4", "5", "6", "7", "8"],
            "every move once"
        );
    }

    #[test]
    fn the_composer_keeps_lines_and_backspace_joins_them_again() {
        let mut c = Compose::new(
            "acme",
            Draft::Comment {
                key: "ABC-12".into(),
            },
            "one\ntwo",
        );
        assert_eq!(c.composer.lines, ["one"]);
        assert_eq!(c.composer.input.value(), "two");
        assert_eq!(c.text(), "one\ntwo");
        c.composer.input = radar_core::input::TextInput::new("");
        let mut p = JiraPlugin::new();
        p.compose = Some(c);
        compose_key(
            &mut p,
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
        );
        assert_eq!(p.compose.as_ref().unwrap().composer.input.value(), "one");
        compose_paste(&mut p, " more\nnext line\nlast");
        assert_eq!(
            p.compose.as_ref().unwrap().text(),
            "one more\nnext line\nlast"
        );
    }
}

#[cfg(test)]
#[path = "write_busy_tests.rs"]
mod busy_tests;

#[cfg(test)]
#[path = "write_dialog_tests.rs"]
mod dialog_tests;
