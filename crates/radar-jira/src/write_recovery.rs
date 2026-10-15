//! Bounded original text for this session only. No serialization or credential values.
use std::collections::BTreeMap;

use radar_core::Plugin;
use radar_core::journal::{ActionDescriptor, Origin, Outcome};
use radar_core::plugin::{Effect, Menu, MenuItem};

use crate::write::{Change, Draft};
use crate::write_context::{self, Dispatch};
use crate::{JiraPlugin, write, write_dialog};

pub(crate) const MAX_RECORDS: usize = 32;
pub(crate) const MAX_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Status {
    #[default]
    Unsent,
    Pending,
    Uncertain,
}
impl Status {
    pub(crate) fn words(self) -> &'static str {
        match self {
            Self::Unsent => "not sent",
            Self::Pending => "send pending; outcome unknown; do not retry",
            Self::Uncertain => {
                "outcome uncertain: this may already exist; check Jira before sending again"
            }
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Context {
    pub generation: Option<u64>,
    pub source: String,
    /// Sanitized provider instance for truthful history after connection changes.
    pub instance: String,
}
impl Context {
    pub(crate) fn capture(p: &mut JiraPlugin, site: &str) -> Self {
        write_context::reconcile(p);
        let source = p
            .cfg
            .site(site)
            .and_then(|s| reqwest::Url::parse(&s.url).ok())
            .filter(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
            .map(|u| u.origin().ascii_serialization())
            .unwrap_or_else(|| "unknown configured server".into());
        let instance = p
            .cfg
            .site(site)
            .and_then(|s| s.base_url().ok())
            .and_then(|url| radar_core::journal::SubjectKey::new("jira", &url, "ticket", "scope").ok())
            .map(|key| key.instance)
            .unwrap_or_default();
        Self {
            generation: p.connections.generation(site),
            source,
            instance,
        }
    }
    fn current(&self, p: &JiraPlugin, site: &str) -> bool {
        self.generation.is_some() && self.generation == p.connections.generation(site)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Original {
    Comment(String),
    Create {
        summary: String,
        description: String,
    },
}
impl Original {
    fn bytes(&self) -> usize {
        match self {
            Self::Comment(text) => text.len(),
            Self::Create {
                summary,
                description,
            } => summary.len().saturating_add(description.len()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Record {
    pub context: Context,
    pub change: Change,
    pub original: Original,
    pub status: Status,
}
impl Record {
    pub(crate) fn capture(p: &mut JiraPlugin, change: &Change) -> Option<Self> {
        let context = Context::capture(p, change.site());
        let original = match change {
            Change::Comment { text, .. } => Original::Comment(text.clone()),
            Change::Create {
                summary,
                description,
                ..
            } => Original::Create {
                summary: summary.clone(),
                description: description.clone(),
            },
            _ => return None,
        };
        let mut record = Self {
            context,
            change: change.clone(),
            original,
            status: Status::Unsent,
        };
        if let Some(c) = p
            .compose
            .as_ref()
            .filter(|c| c.change().as_ref() == Some(change))
        {
            record.original = c.original();
            record.status = c.status;
            if !c.context.source.is_empty() {
                record.context = c.context.clone();
            }
        }
        Some(record)
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Destination {
    Comment {
        site: String,
        key: String,
        context: Context,
    },
    Create {
        site: String,
        project: String,
        context: Context,
    },
}
impl Destination {
    pub(crate) fn comment(p: &mut JiraPlugin, site: &str, key: &str) -> Self {
        Self::Comment {
            site: site.into(),
            key: key.into(),
            context: Context::capture(p, site),
        }
    }
    pub(crate) fn create(p: &mut JiraPlugin, site: &str, project: &str) -> Self {
        Self::Create {
            site: site.into(),
            project: project.into(),
            context: Context::capture(p, site),
        }
    }
    fn matches(&self, change: &Change) -> bool {
        match (self, change) {
            (
                Self::Comment { site, key, .. },
                Change::Comment {
                    site: s, key: k, ..
                },
            ) => site == s && key == k,
            (
                Self::Create { site, project, .. },
                Change::Create {
                    site: s,
                    project: pr,
                    ..
                },
            ) => site == s && project == pr,
            _ => false,
        }
    }
    fn current(&self, p: &JiraPlugin) -> bool {
        match self {
            Self::Comment { site, context, .. } | Self::Create { site, context, .. } => {
                context.current(p, site)
            }
        }
    }
    fn context(&self) -> &Context {
        match self {
            Self::Comment { context, .. } | Self::Create { context, .. } => context,
        }
    }
}

#[derive(Debug)]
enum Choice {
    List {
        ids: Vec<u64>,
        destination: Option<Destination>,
        current: Option<Original>,
    },
    Entry {
        id: u64,
        status: Status,
        destination: Option<Destination>,
    },
    Drop {
        id: u64,
        status: Status,
    },
}
#[derive(Debug, Default)]
pub(crate) struct Recovery {
    next: u64,
    pub records: BTreeMap<u64, Record>,
    active: BTreeMap<u64, u64>, // accepted dispatch -> reserved original record
    choice: Option<Choice>,
}
impl Recovery {
    pub(crate) fn usage(&self) -> (usize, usize) {
        (
            self.records.len(),
            self.records.values().map(|r| r.original.bytes()).sum(),
        )
    }
    pub(crate) fn reserve(&mut self, record: Record) -> Result<u64, ()> {
        let (count, bytes) = self.usage();
        if count >= MAX_RECORDS || record.original.bytes() > MAX_BYTES.saturating_sub(bytes) {
            return Err(());
        }
        self.next = self
            .next
            .checked_add(1)
            .expect("Jira recovery sequence exhausted");
        self.records.insert(self.next, record);
        Ok(self.next)
    }
    pub(crate) fn accept(&mut self, id: u64, dispatch: &Dispatch) {
        self.records.get_mut(&id).expect("reserved original").status = Status::Pending;
        self.active.insert(dispatch.id, id);
    }
    pub(crate) fn owns(&self, change: &Change, dispatch: &Dispatch) -> bool {
        self.active
            .get(&dispatch.id)
            .and_then(|id| self.records.get(id))
            .is_some_and(|r| {
                r.status == Status::Pending
                    && r.change == *change
                    && r.context.generation == Some(dispatch.generation)
            })
    }
    pub(crate) fn finish(&mut self, change: &Change, dispatch: &Dispatch, succeeded: bool) {
        if !self.owns(change, dispatch) {
            return;
        }
        let id = self.active.remove(&dispatch.id).expect("owned dispatch");
        if succeeded {
            self.records.remove(&id);
        } else {
            self.records.get_mut(&id).expect("owned original").status = Status::Uncertain;
        }
    }
}

pub(crate) fn observed(kind: &'static str, outcome: Outcome) -> Effect {
    let mut action = ActionDescriptor::new(kind, Origin::Plugin("jira".into()));
    action.metadata.count = Some(1);
    Effect::Observe { action, outcome }
}
pub(crate) fn full() -> Vec<Effect> {
    vec![
        observed("jira.text.recovery.full", Outcome::Blocked),
        Effect::Notify(
            "kept Jira text is full; your editor is unchanged · ctrl-r to copy or drop kept text"
                .into(),
        ),
    ]
}
fn changed() -> Vec<Effect> {
    vec![
        observed("jira.text.recovery.choice.skipped", Outcome::Blocked),
        Effect::Notify(
            "that kept-text choice is no longer current; open kept Jira text again".into(),
        ),
    ]
}
pub(crate) fn has_target(p: &JiraPlugin, d: &Destination) -> bool {
    p.recovery.records.values().any(|r| d.matches(&r.change))
}
fn menu(p: &mut JiraPlugin, choice: Choice, menu: Menu) -> Vec<Effect> {
    p.recovery.choice = Some(choice);
    vec![write_dialog::menu(p, menu)]
}
/// Global access is read/copy/drop only, including offline and active editors.
pub(crate) fn list(p: &mut JiraPlugin, destination: Option<Destination>) -> Vec<Effect> {
    write_context::reconcile(p);
    p.recovery.choice = None;
    p.dialogs.invalidate();
    let ids: Vec<_> = p
        .recovery
        .records
        .iter()
        .filter(|(_, r)| destination.as_ref().is_none_or(|d| d.matches(&r.change)))
        .map(|(&id, _)| id)
        .collect();
    let mut items: Vec<_> = ids
        .iter()
        .map(|id| {
            let r = &p.recovery.records[id];
            MenuItem::pick(
                format!(
                    "#{id} · {} · {} · {}",
                    r.context.source,
                    r.change.site(),
                    r.change.words()
                ),
                format!(
                    "{}{}",
                    r.status.words(),
                    if r.context.current(p, r.change.site()) {
                        ""
                    } else {
                        " · earlier connection; copy only"
                    }
                ),
                format!("entry:{id}"),
            )
        })
        .collect();
    if destination.is_some() {
        items.push(MenuItem::new(
            'n',
            "compose new",
            "keep retained copies separate",
            "new",
        ));
    }
    let current = p.compose.as_ref().map(crate::write::Compose::original);
    match &current {
        Some(Original::Comment(_)) => items.push(MenuItem::new(
            'y',
            "copy current comment",
            "leave the editor unchanged",
            "copy-current-comment",
        )),
        Some(Original::Create { .. }) => {
            items.push(MenuItem::new(
                's',
                "copy current summary",
                "leave the editor unchanged",
                "copy-current-summary",
            ));
            items.push(MenuItem::new(
                'y',
                "copy current description",
                "leave the editor unchanged",
                "copy-current-description",
            ));
        }
        None => {}
    }
    if items.is_empty() {
        return vec![Effect::Notify("no kept Jira text in this session".into())];
    }
    menu(
        p,
        Choice::List {
            ids,
            destination,
            current,
        },
        Menu::new("j-recovery", "kept Jira text · this session", items),
    )
}
fn entry(p: &mut JiraPlugin, id: u64, destination: Option<Destination>) -> Vec<Effect> {
    let Some(r) = p.recovery.records.get(&id) else {
        return changed();
    };
    let mut items = Vec::new();
    match &r.original {
        Original::Comment(text) => {
            items.extend(
                radar_core::plugin::note_lines(text, 72)
                    .into_iter()
                    .map(MenuItem::note),
            );
            items.push(MenuItem::new(
                'y',
                "copy comment",
                "exact original text",
                "copy-comment",
            ));
        }
        Original::Create {
            summary,
            description,
        } => {
            items.push(MenuItem::heading("summary"));
            items.extend(
                radar_core::plugin::note_lines(summary, 72)
                    .into_iter()
                    .map(MenuItem::note),
            );
            items.push(MenuItem::heading("description"));
            items.extend(
                radar_core::plugin::note_lines(description, 72)
                    .into_iter()
                    .map(MenuItem::note),
            );
            items.push(MenuItem::new(
                's',
                "copy summary",
                "exact original field",
                "copy-summary",
            ));
            items.push(MenuItem::new(
                'y',
                "copy description",
                "exact original field",
                "copy-description",
            ));
        }
    }
    let status = r.status;
    items.push(MenuItem::note(status.words()));
    let can_edit = status != Status::Pending
        && !p.capturing()
        && r.context.current(p, r.change.site())
        && destination
            .as_ref()
            .is_some_and(|d| d.current(p) && d.matches(&r.change) && d.context() == &r.context);
    if can_edit {
        items.push(MenuItem::new(
            'e',
            "edit original",
            "same project/type/target; review again before sending",
            "edit",
        ));
    }
    if !r.context.current(p, r.change.site()) {
        items.push(MenuItem::note(
            "earlier connection: view or copy only; never sent on the replacement connection",
        ));
    }
    if status != Status::Pending {
        items.push(MenuItem::new(
            'd',
            "drop",
            "remove this retained copy…",
            "drop",
        ));
    }
    let title = format!(
        "kept text #{id} · {} · {}",
        r.context.source,
        r.change.words()
    );
    menu(
        p,
        Choice::Entry {
            id,
            status,
            destination: can_edit.then_some(destination).flatten(),
        },
        Menu::new("j-recovery", title, items),
    )
}

pub(crate) fn on_menu(p: &mut JiraPlugin, id: &str, value: &str) -> Option<Vec<Effect>> {
    if !id.starts_with("j-recovery:") {
        return None;
    }
    if !write_dialog::claim(p, id) {
        return Some(changed());
    }
    write_context::reconcile(p);
    let Some(choice) = p.recovery.choice.take() else {
        return Some(changed());
    };
    Some(match choice {
        Choice::List {
            ids,
            destination,
            current,
        } => {
            let copied = match (value, current) {
                ("copy-current-comment", Some(Original::Comment(text))) => Some(text),
                ("copy-current-summary", Some(Original::Create { summary, .. })) => Some(summary),
                ("copy-current-description", Some(Original::Create { description, .. })) => {
                    Some(description)
                }
                _ => None,
            };
            if let Some(text) = copied {
                vec![Effect::Copy(text)]
            } else if value == "new" {
                match destination.filter(|d| d.current(p) && !p.capturing()) {
                    Some(Destination::Comment { site, key, context }) => {
                        write::open_compose(
                            p,
                            &site,
                            Draft::Comment { key },
                            "",
                            context,
                            Status::Unsent,
                            String::new(),
                        );
                        vec![]
                    }
                    Some(Destination::Create { site, project, .. }) => {
                        write::ask_types(p, &site, &project)
                    }
                    None => changed(),
                }
            } else if let Some(id) = value
                .strip_prefix("entry:")
                .and_then(|v| v.parse().ok())
                .filter(|id| ids.contains(id))
            {
                entry(p, id, destination)
            } else {
                vec![]
            }
        }
        Choice::Entry {
            id,
            status,
            destination,
        } => {
            let Some(r) = p.recovery.records.get(&id).filter(|r| r.status == status) else {
                return Some(changed());
            };
            match (value, &r.original) {
                ("copy-comment", Original::Comment(text)) => vec![Effect::Copy(text.clone())],
                ("copy-summary", Original::Create { summary, .. }) => {
                    vec![Effect::Copy(summary.clone())]
                }
                ("copy-description", Original::Create { description, .. }) => {
                    vec![Effect::Copy(description.clone())]
                }
                ("drop", _) if status != Status::Pending => menu(
                    p,
                    Choice::Drop { id, status },
                    Menu::confirm(
                        "j-recovery",
                        format!("drop kept text #{id}?"),
                        "drop this copy",
                        "keep it",
                    )
                    .saying("only this session's retained copy is removed; nothing is sent"),
                ),
                ("edit", _)
                    if status != Status::Pending
                        && !p.capturing()
                        && r.context.current(p, r.change.site())
                        && destination.as_ref().is_some_and(|d| {
                            d.current(p) && d.matches(&r.change) && d.context() == &r.context
                        }) =>
                {
                    let r = p.recovery.records.remove(&id).expect("checked original");
                    match (r.change, r.original) {
                        (Change::Comment { site, key, .. }, Original::Comment(text)) => {
                            write::open_compose(
                                p,
                                &site,
                                Draft::Comment { key },
                                &text,
                                r.context,
                                status,
                                String::new(),
                            )
                        }
                        (
                            Change::Create {
                                site,
                                project,
                                kind,
                                ..
                            },
                            Original::Create {
                                summary,
                                description,
                            },
                        ) => write::open_compose(
                            p,
                            &site,
                            Draft::Summary { project, kind },
                            &summary,
                            r.context,
                            status,
                            description,
                        ),
                        _ => unreachable!("typed original"),
                    }
                    vec![
                        observed("jira.text.recovery.restored", Outcome::Succeeded),
                        Effect::Notify(format!("original text recovered; {}", status.words())),
                    ]
                }
                _ => changed(),
            }
        }
        Choice::Drop { id, status } => {
            if value != "yes" {
                vec![]
            } else if status == Status::Pending
                || p.recovery
                    .records
                    .get(&id)
                    .is_none_or(|r| r.status != status)
            {
                changed()
            } else {
                p.recovery.records.remove(&id);
                vec![
                    observed("jira.text.recovery.dropped", Outcome::Succeeded),
                    Effect::Notify("kept copy dropped; nothing sent".into()),
                ]
            }
        }
    })
}

#[cfg(test)]
mod tests;
