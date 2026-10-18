//! Bounded, session-local ownership for user-initiated action reads and menus.
//! This is deliberately conservative: any later user input supersedes a waiting
//! action read. Results still report what Jira returned, but never queue a dialog.

use radar_core::Plugin;
use radar_core::journal::{ActionDescriptor, Origin, Outcome};
use radar_core::plugin::{Effect, Menu};

use crate::JiraPlugin;
use crate::write::Draft;

#[derive(Debug, Clone)]
pub(crate) struct Read {
    pub id: u64,
    pub action: ActionDescriptor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Editor {
    site: String,
    draft: Draft,
    text: String,
    context: crate::write_recovery::Context,
    status: crate::write_recovery::Status,
    saved_description: String,
}

#[derive(Debug, Default)]
pub(crate) struct Dialogs {
    next: u64,
    read: Option<Read>,
    /// Only the synchronous key, project-picker, or contributed-action launch
    /// consumes its originating Host::input callback. Never use for bus reads.
    launching: bool,
    menu: Option<(String, Option<Editor>)>,
}

impl Dialogs {
    fn next(&mut self) -> u64 {
        self.next = self
            .next
            .checked_add(1)
            .expect("Jira dialog sequence exhausted");
        self.next
    }

    pub(crate) fn input(&mut self) {
        if !std::mem::take(&mut self.launching) {
            self.read = None;
        }
    }

    pub(crate) fn invalidate(&mut self) {
        self.read = None;
        self.launching = false;
        self.menu = None;
    }

    pub(crate) fn read(&mut self, action: ActionDescriptor) -> Read {
        let id = self.next();
        let read = Read { id, action };
        self.read = Some(read.clone());
        self.launching = true;
        read
    }

    pub(crate) fn complete(&mut self, read: &Read) -> bool {
        if self
            .read
            .as_ref()
            .is_none_or(|pending| pending.id != read.id)
        {
            return false;
        }
        self.read = None;
        self.launching = false;
        true
    }

    #[cfg(test)]
    pub(crate) fn current_read(&self) -> Read {
        self.read.clone().expect("a pending action read")
    }

    #[cfg(test)]
    pub(crate) fn menu_id(&self) -> String {
        self.menu.as_ref().expect("an owned menu").0.clone()
    }
}

fn editor(p: &JiraPlugin) -> Option<Editor> {
    p.compose.as_ref().map(|c| Editor {
        site: c.site.clone(),
        draft: c.draft.clone(),
        text: c.text(),
        context: c.context.clone(),
        status: c.status,
        saved_description: c.saved_description.clone(),
    })
}

/// A direct user action can replace snapshots of menus closed with Escape.
/// An editor is not a stale menu snapshot: it must be finished explicitly.
pub(crate) fn begin(p: &mut JiraPlugin) -> bool {
    if p.capturing() {
        return false;
    }
    p.dialogs.invalidate();
    p.change = None;
    p.moving = None;
    p.new_ticket = None;
    p.starting = None;
    true
}

pub(crate) fn occupied(p: &JiraPlugin) -> bool {
    p.capturing() || p.change.is_some() || p.starting.is_some() || p.dialogs.menu.is_some()
}

pub(crate) fn menu(p: &mut JiraPlugin, mut menu: Menu) -> Effect {
    let owner = editor(p);
    p.dialogs.read = None;
    p.dialogs.launching = false;
    menu.id = format!("{}:{}", menu.id, p.dialogs.next());
    p.dialogs.menu = Some((menu.id.clone(), owner));
    Effect::Menu(menu)
}

/// Menu values only apply to the exact one-use prompt and editor it displayed.
pub(crate) fn claim(p: &mut JiraPlugin, id: &str) -> bool {
    let owner = editor(p);
    let Some((current, expected)) = &p.dialogs.menu else {
        return false;
    };
    if current != id {
        return false;
    }
    let matches = *expected == owner;
    p.dialogs.menu = None;
    matches
}

pub(crate) fn skipped(retry: &str) -> Vec<Effect> {
    let mut action = ActionDescriptor::new(
        "jira.action_menu.application.skipped",
        Origin::Plugin("jira".into()),
    );
    action.metadata.count = Some(1);
    vec![
        Effect::Observe {
            action,
            outcome: Outcome::Blocked,
        },
        Effect::Notify(format!("Jira action choice was superseded; {retry}")),
    ]
}

/// Count-only application evidence keeps the captured read scope, never payloads.
pub(crate) fn read_skipped(read: &Read, retry: &str) -> Vec<Effect> {
    let mut effects = skipped(retry);
    let Effect::Observe { action, .. } = &mut effects[0] else {
        unreachable!()
    };
    action.subjects.clone_from(&read.action.subjects);
    action.metadata.redacted = read.action.metadata.redacted;
    action.metadata.omitted_subjects = read.action.metadata.omitted_subjects;
    effects
}

/// Transport/parsing facts finish the original read before independent local
/// application evidence. Preserve empty/error notices along with the retry hint.
pub(crate) fn read_result(
    read: &Read,
    outcome: Outcome,
    current: bool,
    retry: &str,
    mut application: Vec<Effect>,
) -> Vec<Effect> {
    let mut effects = vec![Effect::Report(outcome)];
    if !current {
        let mut skipped = read_skipped(read, retry);
        let notice = skipped.pop().expect("skip notice");
        effects.extend(skipped);
        if let Some(message) = application
            .iter_mut()
            .rev()
            .find_map(|effect| match effect {
                Effect::Notify(message) | Effect::NotifyWith(_, message) => Some(message),
                Effect::Notice(message) => Some(&mut message.text),
                _ => None,
            })
        {
            message.push_str(&format!("; action choice was superseded; {retry}"));
        } else {
            application.push(notice);
        }
    }
    effects.extend(application);
    effects
}
