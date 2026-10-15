//! Bounded, session-local original comment text. Never serialized or journaled.
use std::collections::BTreeMap;

use radar_core::Plugin;
use radar_core::journal::{ActionDescriptor, Origin, Outcome};
use radar_core::plugin::{Effect, Menu, MenuItem};

use crate::GitlabPlugin;
use crate::write::{Change, Compose, Target, WroteMsg};
use crate::write_context::ConnectionStamp;

pub(crate) const MAX_RECORDS: usize = 32;
pub(crate) const MAX_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
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
                "outcome uncertain: a comment may already exist; check GitLab before posting again"
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Record {
    pub connection: ConnectionStamp,
    pub source: String,
    pub change: Change,
    pub original: String,
    pub status: Status,
}

/// Original editor text and connection frozen beside the existing Change modal.
#[derive(Debug, Clone)]
pub(crate) struct Review {
    pub source: String,
    pub connection: ConnectionStamp,
    pub original: Option<String>,
    pub from_composer: bool,
    pub status: Status,
}
impl Review {
    pub(crate) fn capture(p: &GitlabPlugin, change: &Change) -> Self {
        if let Some(c) = &p.compose {
            Self {
                source: c.source.clone(),
                connection: c.connection,
                original: Some(c.text()),
                from_composer: true,
                status: c.status,
            }
        } else {
            Self {
                source: source(p),
                connection: p.connection_stamp(),
                original: change.text().map(str::to_owned),
                from_composer: false,
                status: Status::Unsent,
            }
        }
    }
    pub(crate) fn matches(&self, p: &GitlabPlugin, change: &Change) -> bool {
        if self.from_composer {
            p.compose.as_ref().is_some_and(|c| {
                c.connection == self.connection
                    && c.change() == *change
                    && Some(c.text()) == self.original
            })
        } else {
            p.compose.is_none()
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Destination {
    pub project: String,
    pub iid: u64,
    pub target: Target,
    pub connection: ConnectionStamp,
}
impl Destination {
    fn matches(&self, change: &Change) -> bool {
        change.mr() == (self.project.as_str(), self.iid)
            && change.target().as_ref() == Some(&self.target)
    }
}

#[derive(Debug)]
enum Choice {
    List {
        ids: Vec<u64>,
        destination: Option<Destination>,
        current: Option<String>,
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
    next_menu: u64,
    pub records: BTreeMap<u64, Record>,
    active: BTreeMap<u64, (ConnectionStamp, Change)>,
    choice: Option<(String, Choice)>,
}
impl Recovery {
    pub(crate) fn clear_choice(&mut self) {
        self.choice = None;
    }
    pub(crate) fn usage(&self) -> (usize, usize) {
        (
            self.records.len(),
            self.records.values().map(|r| r.original.len()).sum(),
        )
    }
    /// Reserve before the editor or reviewed Change is consumed. Pending writes
    /// use this same record, so a later failure cannot need additional capacity.
    pub(crate) fn reserve(
        &mut self,
        change: &Change,
        review: &Review,
        sending: bool,
    ) -> Result<u64, ()> {
        if let Some(text) = &review.original {
            let (count, bytes) = self.usage();
            if count >= MAX_RECORDS || text.len() > MAX_BYTES.saturating_sub(bytes) {
                return Err(());
            }
        }
        self.next = self
            .next
            .checked_add(1)
            .expect("GitLab comment sequence exhausted");
        let id = self.next;
        if let Some(original) = &review.original {
            self.records.insert(
                id,
                Record {
                    connection: review.connection,
                    source: review.source.clone(),
                    change: change.clone(),
                    original: original.clone(),
                    status: if sending {
                        Status::Pending
                    } else {
                        review.status
                    },
                },
            );
        }
        if sending {
            self.active.insert(id, (review.connection, change.clone()));
        }
        Ok(id)
    }
    pub(crate) fn owns(&self, msg: &WroteMsg) -> bool {
        self.active
            .get(&msg.operation)
            .is_some_and(|(connection, change)| {
                *connection == msg.connection && *change == msg.change
            })
    }
    pub(crate) fn finish(&mut self, msg: &WroteMsg) {
        self.active.remove(&msg.operation);
        if msg.result.is_ok() {
            self.records.remove(&msg.operation);
        } else if let Some(record) = self.records.get_mut(&msg.operation) {
            record.status = Status::Uncertain;
        }
    }
    #[cfg(test)]
    pub(crate) fn test_keep(&mut self, text: &str) -> u64 {
        self.reserve(
            &Change::Note {
                project: "fixture/held".into(),
                iid: 1,
                text: text.into(),
            },
            &Review {
                source: "https://fixture.example".into(),
                connection: Default::default(),
                original: Some(text.into()),
                from_composer: false,
                status: Status::Unsent,
            },
            false,
        )
        .unwrap()
    }
    #[cfg(test)]
    pub(crate) fn test_operation(&self, change: &Change) -> u64 {
        *self
            .active
            .iter()
            .find(|(_, (_, held))| held == change)
            .expect("dispatched operation")
            .0
    }
    fn menu(&mut self, choice: Choice, title: String, items: Vec<MenuItem>) -> Effect {
        self.next_menu = self
            .next_menu
            .checked_add(1)
            .expect("GitLab recovery menu sequence exhausted");
        let id = format!("gl-recovery:{}", self.next_menu);
        self.choice = Some((id.clone(), choice));
        Effect::Menu(Menu::new(id, title, items))
    }
}

/// Origin only: userinfo, path, query and fragment never appear in recovery labels.
pub(crate) fn source(p: &GitlabPlugin) -> String {
    reqwest::Url::parse(&p.cfg.url)
        .ok()
        .filter(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
        .map(|u| u.origin().ascii_serialization())
        .unwrap_or_else(|| "unknown configured server".into())
}

pub(crate) fn observed(kind: &'static str, outcome: Outcome) -> Effect {
    let mut action = ActionDescriptor::new(kind, Origin::Plugin("gitlab".into()));
    action.metadata.count = Some(1);
    Effect::Observe { action, outcome }
}
pub(crate) fn full() -> Vec<Effect> {
    vec![observed("gitlab.comment.recovery.full", Outcome::Blocked),
        Effect::Notify("kept comments are full; your text is still here · ctrl-r to copy or drop kept text before continuing".into())]
}
fn changed() -> Vec<Effect> {
    vec![
        observed("gitlab.comment.recovery.choice.skipped", Outcome::Blocked),
        Effect::Notify("that comment choice is no longer current; open kept comments again".into()),
    ]
}
pub(crate) fn editor_busy(p: &GitlabPlugin) -> bool {
    p.capturing()
}
pub(crate) fn has_target(p: &GitlabPlugin, destination: &Destination) -> bool {
    p.recovery
        .records
        .values()
        .any(|r| destination.matches(&r.change))
}

/// Global and Settings entry points are view/copy/drop only. Only a fresh `c`
/// at the exact frozen target offers editing, never an invisible Settings editor.
pub(crate) fn list(p: &mut GitlabPlugin, destination: Option<Destination>) -> Vec<Effect> {
    p.recovery.clear_choice();
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
                    r.source,
                    r.change.key(),
                    r.change.doing()
                ),
                format!(
                    "{}{}",
                    r.status.words(),
                    if p.is_current_connection(r.connection) {
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
            "new comment",
            "write a separate comment",
            "new",
        ));
    }
    let current = p.compose.as_ref().map(Compose::text);
    if current.is_some() {
        items.push(MenuItem::new(
            'y',
            "copy current draft",
            "leave the composer unchanged",
            "copy-current",
        ));
    }
    if items.is_empty() {
        return vec![Effect::Notify("no kept comments in this session".into())];
    }
    vec![p.recovery.menu(
        Choice::List {
            ids,
            destination,
            current,
        },
        "kept comments · this session".into(),
        items,
    )]
}
fn entry(p: &mut GitlabPlugin, id: u64, destination: Option<Destination>) -> Vec<Effect> {
    let Some(r) = p.recovery.records.get(&id) else {
        return changed();
    };
    let title = format!("comment #{id} · {} · {}", r.source, r.change.key());
    let status = r.status;
    let mut items: Vec<_> = radar_core::plugin::note_lines(&r.original, 72)
        .into_iter()
        .map(MenuItem::note)
        .collect();
    items.push(MenuItem::note(r.change.doing()));
    items.push(MenuItem::note(status.words()));
    let can_edit = status != Status::Pending
        && p.is_current_connection(r.connection)
        && destination
            .as_ref()
            .is_some_and(|d| d.connection == r.connection && d.matches(&r.change))
        && !editor_busy(p);
    if can_edit {
        items.push(MenuItem::new(
            'e',
            "edit",
            "restore this original target; review and confirm separately",
            "edit",
        ));
    } else if !p.is_current_connection(r.connection) {
        items.push(MenuItem::note(
            "earlier connection: copy text to a freshly chosen destination; never sent here",
        ));
    }
    items.push(MenuItem::new(
        'y',
        "copy",
        "leave the retained copy unchanged",
        "copy",
    ));
    if status != Status::Pending {
        items.push(MenuItem::new('d', "drop", "remove this kept copy…", "drop"));
    }
    vec![p.recovery.menu(
        Choice::Entry {
            id,
            status,
            destination: can_edit.then_some(destination).flatten(),
        },
        title,
        items,
    )]
}
pub(crate) fn on_menu(p: &mut GitlabPlugin, menu: &str, value: &str) -> Option<Vec<Effect>> {
    if !menu.starts_with("gl-recovery:") {
        return None;
    }
    if p.recovery.choice.as_ref().is_none_or(|(id, _)| id != menu) {
        return Some(changed());
    }
    let (_, choice) = p.recovery.choice.take().expect("checked recovery choice");
    Some(match choice {
        Choice::List {
            ids,
            destination,
            current,
        } => {
            if value == "copy-current" {
                current
                    .map(|text| vec![Effect::Copy(text)])
                    .unwrap_or_else(changed)
            } else if value == "new" {
                if let Some(d) =
                    destination.filter(|d| p.is_current_connection(d.connection) && !editor_busy(p))
                {
                    p.change = None;
                    p.review = None;
                    p.compose = Some(Compose::new(
                        &d.project,
                        d.iid,
                        d.target,
                        "",
                        d.connection,
                        source(p),
                        Status::Unsent,
                    ));
                    vec![]
                } else {
                    changed()
                }
            } else if let Some(id) = value
                .strip_prefix("entry:")
                .and_then(|id| id.parse().ok())
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
            match value {
                "copy" => vec![Effect::Copy(r.original.clone())],
                "drop" if status != Status::Pending => {
                    let title = format!("drop kept comment #{id}?");
                    let fx = p
                        .recovery
                        .menu(Choice::Drop { id, status }, title.clone(), vec![]);
                    let Effect::Menu(menu) = fx else {
                        unreachable!()
                    };
                    vec![Effect::Menu(
                        Menu::confirm(menu.id, title, "drop this copy", "keep it").saying(
                            "only this session's retained copy is removed; nothing is sent",
                        ),
                    )]
                }
                "edit"
                    if status != Status::Pending
                        && !editor_busy(p)
                        && p.is_current_connection(r.connection)
                        && destination.as_ref().is_some_and(|d| {
                            d.connection == r.connection && d.matches(&r.change)
                        }) =>
                {
                    let r = p
                        .recovery
                        .records
                        .remove(&id)
                        .expect("checked recovery record");
                    let (project, iid) = r.change.mr();
                    p.change = None;
                    p.review = None;
                    p.compose = Some(Compose::new(
                        project,
                        iid,
                        r.change.target().expect("comment target"),
                        &r.original,
                        r.connection,
                        r.source,
                        status,
                    ));
                    vec![
                        observed("gitlab.comment.recovery.restored", Outcome::Succeeded),
                        Effect::Notify(format!("comment recovered; {}", status.words())),
                    ]
                }
                _ => changed(),
            }
        }
        Choice::Drop { id, status } => {
            if value != "yes" {
                return Some(vec![]);
            }
            if status == Status::Pending
                || p.recovery
                    .records
                    .get(&id)
                    .is_none_or(|r| r.status != status)
            {
                return Some(changed());
            }
            p.recovery.records.remove(&id);
            vec![
                observed("gitlab.comment.recovery.dropped", Outcome::Succeeded),
                Effect::Notify("kept copy dropped; nothing sent".into()),
            ]
        }
    })
}

#[cfg(test)]
mod tests;
