//! What the Repositories tree can do to the merge request on its selected
//! row, without opening its page: rebase, merge and approve, as the page's
//! `u`, `m` and `a` do (the same questions, asked first, the same switch
//! for writes, the same progress and refresh after), and open, copy and
//! the pipeline. They are keys on the row, rows of its `x` menu, controls at
//! the detail pane's foot, the keys on the status line, and a click on the
//! row's STATE. Contributed actions remain in the `x` menu. Pinned controls
//! use the host's current-detail admission before the original key resolver.
//!
//! The tree keeps `a` for "show one author's merge requests", so approving
//! is [`APPROVE_KEY`] there.

use radar_core::design::help_bar::KeyHint;
use radar_core::plugin::{Binding, Effect, Outcome, Pane};
use radar_core::view::{ActionGroup, Chord, Key};

use crate::model::{Mr, MrState, Pipeline, Role};
use crate::mr_actions::{self, Acting};
use crate::pages::{act, mr_key, review};
use crate::ready::{self, Block};
use crate::{GitlabPlugin, screen};

/// Approve (and take the approval back) on a tree row. `a` is the tree's
/// "show one author's merge requests", `o p y` open, pipeline and copy, and
/// the other parts add `v > s t n` to a merge request's row; none of them
/// uses this.
pub(crate) const APPROVE_KEY: char = 'e';

/// One thing to do to a merge request from its row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Do {
    Rebase,
    Merge,
    Approve,
    Open,
    Copy,
    Pipeline,
}

impl Do {
    /// Every action, in the order the detail pane lists them.
    pub(crate) const ALL: [Do; 6] = [
        Self::Rebase,
        Self::Merge,
        Self::Approve,
        Self::Open,
        Self::Copy,
        Self::Pipeline,
    ];

    pub(crate) const fn key(self) -> char {
        match self {
            Self::Rebase => 'u',
            Self::Merge => 'm',
            Self::Approve => APPROVE_KEY,
            Self::Open => 'o',
            Self::Copy => 'y',
            Self::Pipeline => 'p',
        }
    }

    fn of_key(c: char) -> Option<Self> {
        Self::ALL.into_iter().find(|d| d.key() == c)
    }

    /// What it is called when it cannot be done.
    const fn name(self) -> &'static str {
        match self {
            Self::Rebase => "update by rebasing",
            Self::Merge => "merge",
            Self::Approve => "approve",
            Self::Open => "open",
            Self::Copy => "copy link",
            Self::Pipeline => "pipeline",
        }
    }

    /// Whether it asks first (and so its words end in `…`).
    const fn asks(self) -> bool {
        matches!(self, Self::Rebase | Self::Merge | Self::Approve)
    }
}

/// An action as offered on a row: what it will do, or why it cannot be done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Row {
    pub what: Do,
    /// What it will do, said for this merge request (`update by rebasing onto main`).
    pub label: String,
    /// Why it is not offered now.
    pub why: Option<String>,
}

impl Row {
    fn on(what: Do, label: impl Into<String>) -> Self {
        Self { what, label: label.into(), why: None }
    }

    fn off(what: Do, why: impl Into<String>) -> Self {
        Self { what, label: what.name().to_string(), why: Some(why.into()) }
    }

    /// `update by rebasing onto main`, or the same action followed by its blocker.
    #[cfg(test)]
    pub(crate) fn words(&self) -> String {
        match &self.why {
            Some(why) => format!("{} · {why}", self.label),
            None => self.label.clone(),
        }
    }
}

/// Whether a rebase or merge is under way on `mr`.
fn busy(p: &GitlabPlugin, mr: &Mr, what: &str) -> bool {
    p.acting
        .get(&mr_key(&mr.project_path, mr.iid))
        .is_some_and(|a| a.state == Acting::Running && a.what == what)
}

/// Every action on `mr`, offered or not, in the detail pane's order. The pipeline
/// is left out when it has none.
pub(crate) fn rows(p: &GitlabPlugin, mr: &Mr) -> Vec<Row> {
    let target = &mr.target_branch;
    let rebase = if busy(p, mr, "rebase") {
        Row::off(Do::Rebase, "asked, waiting for GitLab")
    } else if let Some(why) = mr_actions::rebase_blocker(mr) {
        Row::off(Do::Rebase, why)
    } else {
        Row::on(Do::Rebase, format!("update by rebasing onto {target}"))
    };
    let merge = if busy(p, mr, "merge") {
        Row::off(Do::Merge, "asked, waiting for GitLab")
    } else if let Some(why) = mr_actions::blocker(mr) {
        Row::off(Do::Merge, why)
    } else {
        Row::on(Do::Merge, format!("merge into {target}"))
    };
    let mine = mr.approved_by.contains(&p.me);
    let approve = if mr.state != MrState::Opened {
        Row::off(Do::Approve, format!("it is {} already", mr.state.word()))
    } else if mine {
        Row::on(Do::Approve, "take back your approval")
    } else {
        Row::on(Do::Approve, "approve")
    };
    let mut out = vec![
        rebase,
        merge,
        approve,
        Row::on(Do::Open, "open in GitLab"),
        Row::on(Do::Copy, "copy link"),
    ];
    if mr.pipeline_url.is_some() {
        out.push(Row::on(Do::Pipeline, "open its pipeline"));
    }
    out
}

/// Do `what` to `mr`: ask first where it writes or changes files, as the
/// page does; or say why not.
pub(crate) fn act(p: &mut GitlabPlugin, what: Do, mr: &Mr) -> Vec<Effect> {
    match what {
        Do::Rebase => match mr_actions::rebase_blocker(mr) {
            Some(why) => vec![mr_actions::journal(p, "gitlab.rebase.request", &mr.project_path, mr.iid), Effect::Report(radar_core::journal::Outcome::Blocked), Effect::Notify(format!("cannot rebase !{}: {why}", mr.iid))],
            None => act::ask_rebase(p, mr),
        },
        Do::Merge => act::ask_merge(p, mr),
        Do::Approve => review::ask_approve(p, &mr.project_path, mr.iid),
        Do::Open => vec![Effect::Open(mr.web_url.clone())],
        Do::Copy => vec![Effect::Copy(mr.web_url.clone()), Effect::Notify("link copied".into())],
        Do::Pipeline => match &mr.pipeline_url {
            Some(url) => vec![Effect::Open(url.clone())],
            None => vec![Effect::Notify(format!("!{} has no pipeline", mr.iid))],
        },
    }
}

/// The merge request selected in the tree.
fn selected(p: &GitlabPlugin) -> Option<Mr> {
    let items = p.visible();
    screen::selected_mr(p, &items).map(|it| it.mr.clone())
}

/// A key on a merge request's row (in the tree or its detail pane); `None` when
/// it is not one of these, or no merge request is selected. `o p y` are the
/// screen's own.
pub(crate) fn key(p: &mut GitlabPlugin, pane: Pane, key: &Chord) -> Option<Outcome> {
    if !matches!(pane, Pane::Middle | Pane::Detail) || !key.bare() {
        return None;
    }
    let Key::Char(c) = key.key else { return None };
    let what = Do::of_key(c).filter(|d| d.asks())?;
    let mr = selected(p)?;
    Some(Outcome::Handled(act(p, what, &mr)))
}

/// The rows of the tree row's `x` menu: the ones that change something
/// (`o p y` are in the screen's own).
pub(crate) fn bindings(p: &GitlabPlugin, mr: &Mr) -> Vec<crate::Row> {
    let rows = rows(p, mr);
    let ask = |r: &Row| {
        // The row's title already names the merge request: the label is the
        // action alone, and what stands in its way is a fact beside it.
        let verb = match r.what {
            Do::Rebase => "rebase",
            Do::Merge => "merge",
            Do::Approve if r.label == "take back your approval" => "take back your approval",
            _ => "approve",
        };
        let text = if r.why.is_none() { format!("{verb}…") } else { verb.to_string() };
        let id = match r.what {
            Do::Rebase => "gitlab.repos.rebase",
            Do::Merge => "gitlab.repos.merge",
            Do::Approve => "gitlab.repos.approve",
            Do::Open => "gitlab.repos.open_link",
            Do::Copy => "gitlab.repos.copy_link",
            Do::Pipeline => "gitlab.repos.open_pipeline",
        };
        // Rebase, merge and approve change what is there.
        let mut row = Binding::new(r.what.key().to_string(), text).edits().group(ActionGroup::Change);
        if let Some(why) = &r.why {
            row = row.hint(why.clone());
        } else if leads(p, mr, r.what) {
            row = row.lead();
        }
        crate::act(id, row)
    };
    rows.iter().filter(|r| matches!(r.what, Do::Rebase | Do::Merge | Do::Approve)).map(ask).collect()
}

/// Whether the merge request's state asks for `what` now: merge when it is
/// ready to merge, approve when the user's review is asked for.
fn leads(p: &GitlabPlugin, mr: &Mr, what: Do) -> bool {
    match what {
        Do::Merge => state_click(mr) == Some(StateClick::Merge),
        Do::Approve => mr.state == MrState::Opened && mr.role(&p.me) == Role::Review,
        _ => false,
    }
}

/// The status line's keys for the selected merge request: only those that
/// apply: updating by rebasing when it needs that, merging when GitLab
/// would take it, and approving when it waits for approvals.
pub(crate) fn hints(p: &GitlabPlugin, pane: Pane) -> Vec<KeyHint<'static>> {
    if !p.pages.is_empty() || !matches!(pane, Pane::Middle | Pane::Detail) {
        return vec![];
    }
    let Some(mr) = selected(p) else { return vec![] };
    let rows = rows(p, &mr);
    let on = |d: Do| rows.iter().any(|r| r.what == d && r.why.is_none());
    let mut out = Vec::new();
    if on(Do::Rebase) {
        out.push(KeyHint::new("u", "update by rebasing"));
    }
    if on(Do::Merge) {
        out.push(KeyHint::new("m", "merge"));
    }
    // Only when approvals are what leads the way (not under a draft).
    let wants_approval = matches!(ready::blocks(&mr).first(), Some(Block::Approvals(_)));
    if on(Do::Approve) && wants_approval && !mr.approved_by.contains(&p.me) {
        out.push(KeyHint::new("e", "approve"));
    }
    out
}

/// What a click on a row's STATE does, from the first thing it names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StateClick {
    /// `needs rebase`: ask to rebase.
    Rebase,
    /// `ready to merge`: ask to merge.
    Merge,
    /// `pipeline failed`: open why it is red.
    Why,
}

pub(crate) fn state_click(mr: &Mr) -> Option<StateClick> {
    if mr.state != MrState::Opened {
        return None;
    }
    match ready::blocks(mr).first() {
        Some(Block::Rebase) => Some(StateClick::Rebase),
        None => Some(StateClick::Merge),
        Some(Block::Pipeline(Some(Pipeline::Failed))) => Some(StateClick::Why),
        Some(_) => None,
    }
}

#[cfg(test)]
mod tests;
