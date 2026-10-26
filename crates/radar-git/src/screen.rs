//! The Branches screen (`B`): a nav pane (all, dirty, ahead of remote, stashes)
//! │ a middle pane (a tree) of clone → its worktrees → its local branches, with ahead and
//! behind, what is uncommitted, stashes and when each last changed │ a
//! detail pane of what is selected.
//!
//! What it shows is read off the event loop, one clone at a time (a few
//! `git` calls each, none of which touches the network or changes anything),
//! kept in memory, and read again on `r` and every minute while the screen
//! has been looked at. `⏎` on a worktree makes it the one acted on for its
//! repo; `d` deletes a branch or a worktree after saying what is lost; `b`
//! prunes the merged and gone ones, clean ones only (`prune.rs`).

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;
use std::process::{Command, Stdio};

#[cfg(test)]
use radar_core::view::Key as KeyCode;

use radar_core::design::help_bar::KeyHint;
use radar_core::journal::{ActionDescriptor, Origin, Outcome as JournalOutcome};
use radar_core::plugin::{Binding, Effect, Menu, MenuItem, Msg, Outcome, Pane, ScreenSpec, Tone};
use radar_core::ui::icon::{self, Icon};
use radar_core::view::{
    self as v, ColumnSpec, Ink, Intent, Key, Node, PaneId, PaneView, Seg, Text, TreeNav, TreeRow,
};
use radar_core::{Subject, time, ui};

use crate::GitPlugin;
use crate::local::{self, Checkout};
use crate::prune;

mod removal;

pub const SCREEN: ScreenSpec = ScreenSpec {
    id: "branch-screen",
    name: "branches",
    key: 'B',
};

/// How long what was read is good for while the screen is looked at.
const REFRESH_SECS: i64 = 60;

/// The room the names keep beside the columns.
const NAME: u16 = 30;

// --- what is read ------------------------------------------------------------------

/// A local branch.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Branch {
    pub name: String,
    /// `origin/x`, empty when it tracks nothing.
    pub upstream: String,
    pub ahead: usize,
    pub behind: usize,
    /// It tracks a branch the remote no longer has.
    pub gone: bool,
    /// When its tip was committed.
    pub updated: i64,
    pub sha: String,
    pub subject: String,
    /// Commits that no remote has.
    pub unpushed: usize,
    /// Merged into the repo's base branch.
    pub merged: bool,
}

/// Whether this worktree's status was read, or only an earlier result is held.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StatusRead {
    #[default]
    Current,
    Unavailable,
    LastKnown,
}

/// A place a branch is checked out: the clone's own folder (first) or a
/// worktree.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tree {
    pub dir: String,
    /// Empty when detached.
    pub branch: String,
    /// The commit, when detached.
    pub head: String,
    /// Files with uncommitted changes (untracked ones too).
    pub dirty: usize,
    pub status: StatusRead,
    pub ahead: usize,
    pub behind: usize,
}

/// Everything read from one clone.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Detail {
    pub dir: String,
    /// The branch the others are measured against: origin's HEAD, else main.
    pub base: String,
    /// The clone's own folder, then its worktrees.
    pub trees: Vec<Tree>,
    pub branches: Vec<Branch>,
    /// One line per stash.
    pub stashes: Vec<String>,
    /// Why it could not be read.
    pub error: String,
}

impl Detail {
    /// Uncommitted work in the clone or a worktree.
    pub fn dirty(&self) -> bool {
        self.trees.iter().any(|t| t.dirty > 0)
    }

    pub fn ahead(&self) -> usize {
        self.branches.iter().filter(|b| b.ahead > 0).count()
    }

    fn on(&self, branch: &str) -> Vec<&Tree> {
        self.trees.iter().filter(|t| t.branch == branch).collect()
    }
}

/// git in `dir`; never asks for a password, the terminal is radar's. The last
/// line git printed on a failure, without its `fatal:`.
pub(crate) fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("SSH_ASKPASS_REQUIRE", "force")
        .env("SSH_ASKPASS", "false")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("git failed")
            .trim()
            .trim_start_matches("fatal: ")
            .trim_start_matches("error: ")
            .to_string())
    }
}

/// `[ahead 2, behind 1]`, `[ahead 2]`, `[gone]` or nothing.
fn parse_track(track: &str) -> (usize, usize, bool) {
    let inner = track.trim().trim_start_matches('[').trim_end_matches(']');
    if inner == "gone" {
        return (0, 0, true);
    }
    let (mut ahead, mut behind) = (0, 0);
    for part in inner.split(',') {
        let mut words = part.split_whitespace();
        match (words.next(), words.next().and_then(|n| n.parse().ok())) {
            (Some("ahead"), Some(n)) => ahead = n,
            (Some("behind"), Some(n)) => behind = n,
            _ => {}
        }
    }
    (ahead, behind, false)
}

/// `for-each-ref` lines of name, upstream, track, time, sha, subject, tab
/// separated.
fn parse_refs(text: &str) -> Vec<Branch> {
    text.lines()
        .filter_map(|line| {
            let mut f = line.splitn(6, '\t');
            let name = f.next().filter(|n| !n.is_empty())?;
            let upstream = f.next().unwrap_or_default();
            let (ahead, behind, gone) = parse_track(f.next().unwrap_or_default());
            let updated = f.next().and_then(|t| t.parse().ok()).unwrap_or(0);
            let sha = f.next().unwrap_or_default();
            let subject = f.next().unwrap_or_default();
            Some(Branch {
                name: name.into(),
                upstream: upstream.into(),
                ahead,
                behind,
                gone,
                updated,
                sha: sha.into(),
                subject: subject.into(),
                ..Branch::default()
            })
        })
        .collect()
}

/// `git worktree list --porcelain`: each folder with its branch, or the
/// commit when it is detached. Prunable ones (the folder is gone) are left out.
fn parse_worktrees(text: &str) -> Vec<Tree> {
    let mut out = Vec::new();
    for block in text.split("\n\n") {
        let mut tree = Tree::default();
        let mut skip = false;
        for line in block.lines() {
            if let Some(dir) = line.strip_prefix("worktree ") {
                tree.dir = dir.to_string();
            } else if let Some(sha) = line.strip_prefix("HEAD ") {
                tree.head = sha.chars().take(7).collect();
            } else if let Some(b) = line.strip_prefix("branch ") {
                tree.branch = b.trim_start_matches("refs/heads/").to_string();
            } else if line == "bare" || line.starts_with("prunable") {
                skip = true;
            }
        }
        if !tree.dir.is_empty() && !skip {
            out.push(tree);
        }
    }
    out
}

/// `status --porcelain=2 --branch`: files changed, and ahead and behind the
/// upstream.
fn parse_status(text: &str) -> (usize, usize, usize) {
    let (mut dirty, mut ahead, mut behind) = (0, 0, 0);
    for line in text.lines() {
        if let Some(ab) = line.strip_prefix("# branch.ab ") {
            for w in ab.split_whitespace() {
                if let Some(n) = w.strip_prefix('+') {
                    ahead = n.parse().unwrap_or(0);
                } else if let Some(n) = w.strip_prefix('-') {
                    behind = n.parse().unwrap_or(0);
                }
            }
        } else if !line.starts_with('#') && !line.is_empty() {
            dirty += 1;
        }
    }
    (dirty, ahead, behind)
}

/// Read a clone: its branches, worktrees, stashes and what is uncommitted.
/// Blocking; changes nothing.
#[cfg(test)]
pub fn read(dir: &str) -> Detail {
    read_with(dir, git).0
}

/// A count is evidence only when its list was actually read. Failed and skipped
/// lists must not become a successful zero through the display's fallbacks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum ReadCount {
    #[default]
    NotRead,
    Failed,
    Observed {
        count: usize,
        complete: bool,
    },
}

impl ReadCount {
    fn observed(count: usize) -> Self {
        Self::Observed {
            count,
            complete: true,
        }
    }

    fn incomplete(&mut self) {
        if let Self::Observed { complete, .. } = self {
            *complete = false;
        }
    }

    fn outcome(self) -> Option<JournalOutcome> {
        match self {
            Self::NotRead => None,
            Self::Failed => Some(JournalOutcome::Failed),
            Self::Observed { complete: true, .. } => Some(JournalOutcome::Succeeded),
            Self::Observed {
                complete: false, ..
            } => Some(JournalOutcome::Partial),
        }
    }
}

/// Task-local evidence, separate from the cached/displayed Detail. No raw git
/// output or per-item identities are part of the journal observation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ReadObservation {
    branches: ReadCount,
    worktrees: ReadCount,
    stashes: ReadCount,
}

impl ReadObservation {
    fn outcome(self) -> JournalOutcome {
        if self.branches == ReadCount::Failed {
            JournalOutcome::Failed
        } else if [self.branches, self.worktrees, self.stashes]
            .iter()
            .all(|count| count.outcome() == Some(JournalOutcome::Succeeded))
        {
            JournalOutcome::Succeeded
        } else {
            JournalOutcome::Partial
        }
    }

    fn effects(self, action: &ActionDescriptor) -> Vec<Effect> {
        // Complete the read and its exact task before recording bounded count
        // observations. These are observed items, not a new inventory query.
        let mut fx = vec![Effect::Report(self.outcome())];
        for (kind, count) in [
            ("git.branches.read.branches", self.branches),
            ("git.branches.read.worktrees", self.worktrees),
            ("git.branches.read.stashes", self.stashes),
        ] {
            let Some(outcome) = count.outcome() else {
                continue;
            };
            let mut observed = action.clone();
            observed.kind = radar_core::journal::ActionKind::new(kind);
            if let ReadCount::Observed { count, .. } = count {
                set_read_count(&mut observed, count);
            }
            fx.push(Effect::Observe {
                action: observed,
                outcome,
            });
        }
        fx
    }
}

fn set_read_count(action: &mut ActionDescriptor, count: usize) {
    action.metadata.count = Some(u32::try_from(count).unwrap_or(u32::MAX));
    action.metadata.truncated |= count > u32::MAX as usize;
}

fn read_with(
    dir: &str,
    mut git: impl FnMut(&Path, &[&str]) -> Result<String, String>,
) -> (Detail, ReadObservation) {
    let path = Path::new(dir);
    let mut d = Detail {
        dir: dir.into(),
        ..Detail::default()
    };
    let mut observed = ReadObservation::default();
    let format = "--format=%(refname:short)%09%(upstream:short)%09%(upstream:track)%09%(committerdate:unix)%09%(objectname:short)%09%(subject)";
    match git(path, &["for-each-ref", format, "refs/heads"]) {
        Ok(text) => {
            d.branches = parse_refs(&text);
            observed.branches = ReadCount::observed(d.branches.len());
        }
        Err(e) => {
            d.error = e;
            observed.branches = ReadCount::Failed;
            return (d, observed);
        }
    }
    // Missing origin/HEAD and remote refs are ordinary discovery fallbacks,
    // not failed reads when the local base can supply the measurement.
    d.base = git(
        path,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    )
    .ok()
    .and_then(|s| s.strip_prefix("origin/").map(str::to_string))
    .or_else(|| {
        ["main", "master"]
            .into_iter()
            .find(|n| d.branches.iter().any(|b| b.name == *n))
            .map(str::to_string)
    })
    .unwrap_or_default();
    if !d.base.is_empty() {
        let remote = format!("refs/remotes/origin/{}", d.base);
        let against = if git(path, &["rev-parse", "--verify", "--quiet", &remote]).is_ok() {
            format!("origin/{}", d.base)
        } else {
            d.base.clone()
        };
        let merged = git(
            path,
            &["branch", "--merged", &against, "--format=%(refname:short)"],
        )
        .unwrap_or_else(|_| {
            observed.branches.incomplete();
            String::new()
        });
        let merged: HashSet<&str> = merged.lines().collect();
        for b in &mut d.branches {
            b.merged = merged.contains(b.name.as_str());
        }
    }
    for b in &mut d.branches {
        b.unpushed = if !b.upstream.is_empty() && !b.gone {
            b.ahead
        } else {
            git(
                path,
                &["rev-list", "--count", &b.name, "--not", "--remotes"],
            )
            .ok()
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| {
                observed.branches.incomplete();
                0
            })
        };
    }
    match git(path, &["worktree", "list", "--porcelain"]) {
        Ok(text) => {
            d.trees = parse_worktrees(&text);
            observed.worktrees = ReadCount::observed(d.trees.len());
        }
        Err(_) => observed.worktrees = ReadCount::Failed,
    }
    if d.trees.is_empty() {
        d.trees.push(Tree {
            dir: dir.into(),
            ..Tree::default()
        });
    }
    for t in &mut d.trees {
        match git(Path::new(&t.dir), &["status", "--porcelain=2", "--branch"]) {
            Ok(status) => (t.dirty, t.ahead, t.behind) = parse_status(&status),
            Err(_) => {
                observed.worktrees.incomplete();
                t.status = StatusRead::Unavailable;
            }
        }
    }
    match git(path, &["stash", "list", "--format=%gs"]) {
        Ok(text) => {
            d.stashes = text.lines().map(str::to_string).collect();
            observed.stashes = ReadCount::observed(d.stashes.len());
        }
        Err(_) => observed.stashes = ReadCount::Failed,
    }
    (d, observed)
}

// --- state -------------------------------------------------------------------------

/// The views of the nav pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    All,
    Dirty,
    Ahead,
    Stashes,
}

impl View {
    const ALL: [View; 4] = [View::All, View::Dirty, View::Ahead, View::Stashes];

    fn name(self) -> &'static str {
        match self {
            View::All => "all",
            View::Dirty => "dirty",
            View::Ahead => "ahead of remote",
            View::Stashes => "stashes",
        }
    }

    fn icon(self) -> Icon {
        match self {
            View::All => Icon::Views,
            View::Dirty => Icon::Warning,
            View::Ahead => Icon::Commit,
            View::Stashes => Icon::Folder,
        }
    }

    fn words(self) -> &'static str {
        match self {
            View::All => "every clone, its worktrees and its local branches",
            View::Dirty => "clones and worktrees with uncommitted changes",
            View::Ahead => "branches with commits their remote does not have yet",
            View::Stashes => "clones with something set aside in a stash",
        }
    }
}

/// What the screen holds while it runs.
#[derive(Debug, Default)]
pub struct Screen {
    pub details: BTreeMap<String, Detail>,
    /// Clones being read.
    pub loading: BTreeSet<String>,
    /// Clones being changed (a delete or a prune running).
    pub changing: BTreeSet<String>,
    /// When the clones were last asked for.
    pub read_at: i64,
    /// The last wholly successful refresh, distinct from the attempt time.
    pub last_ok: Option<i64>,
    /// Reads that ended without a complete result; kept visible during retries.
    pub incomplete: BTreeSet<String>,
    /// Outstanding members of a full refresh. A single post-action reread
    /// must not make every clone appear freshly synced.
    round: BTreeSet<String>,
    /// The screen has been looked at (its view asked for): reading starts
    /// then, not before.
    pub seen: Cell<bool>,
    pub view: usize,
    /// The tree's selection and open groups (what is selected decides what
    /// the actions do); groups open the first time they appear.
    pub tree: RefCell<TreeNav>,
    /// A transient, single-use worktree removal question, never cached.
    removal: removal::State,
}

impl Screen {
    fn view(&self) -> View {
        View::ALL[self.view.min(View::ALL.len() - 1)]
    }
}

/// What a task off the event loop found.
pub(crate) enum BranchMsg {
    Read {
        dir: String,
        detail: Box<Detail>,
        observed: ReadObservation,
        /// The validated identity at dispatch, never rebuilt from later settings.
        action: ActionDescriptor,
    },
    ReadInterrupted {
        dir: String,
        why: String,
    },
    /// A delete ended: what to say, and the clones to read again.
    Done {
        said: Result<String, String>,
        reread: Vec<String>,
    },
    /// A prune ended with independently observed steps, including skipped work.
    Pruned {
        result: prune::RunResult,
        reread: Vec<String>,
    },
}

fn blocking(f: impl FnOnce() -> BranchMsg + Send + 'static) -> Effect {
    Effect::Spawn(Box::pin(async move {
        let msg = tokio::task::spawn_blocking(f)
            .await
            .unwrap_or_else(|e| BranchMsg::Done {
                said: Err(e.to_string()),
                reread: vec![],
            });
        Box::new(msg) as Msg
    }))
}

fn read_action(dir: &str) -> ActionDescriptor {
    let Effect::Journal(action) = crate::act::journal_at("git.branches.read", dir, "", "") else {
        unreachable!("journal_at always returns a descriptor")
    };
    action
}

fn read_task(
    dir: String,
    read: impl FnOnce(&str) -> (Detail, ReadObservation) + Send + 'static,
) -> Effect {
    let action = read_action(&dir);
    let captured = action.clone();
    Effect::SpawnAction {
        action,
        task: Box::pin(async move {
            let requested = dir.clone();
            let msg = tokio::task::spawn_blocking(move || {
                let (detail, observed) = read(&requested);
                BranchMsg::Read {
                    dir: requested,
                    detail: Box::new(detail),
                    observed,
                    action: captured,
                }
            })
            .await
            .unwrap_or_else(|e| BranchMsg::ReadInterrupted {
                dir,
                why: e.to_string(),
            });
            Box::new(msg) as Msg
        }),
    }
}

fn request_read(p: &mut GitPlugin, dir: String) -> Effect {
    if p.branches.loading.insert(dir.clone()) {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(fixture) = p.fixture.clone() {
            let pending = fixture.pending();
            return read_task(dir, move |dir| {
                let _pending = pending;
                read_with(dir, |path, args| fixture.git(path, args))
            });
        }
        read_task(dir, |dir| read_with(dir, git))
    } else {
        let mut action = read_action(&dir);
        action.kind = radar_core::journal::ActionKind::new("git.branches.read.suppressed");
        // One requested read was not started because this exact clone is already
        // loading. The in-flight action remains open and keeps its own outcome.
        action.metadata.count = Some(1);
        Effect::Observe {
            action,
            outcome: JournalOutcome::Blocked,
        }
    }
}

/// A clone to show: its folder and the repo its origin names.
struct CloneRow {
    dir: String,
    project: String,
}

/// The clones on this machine: found by the scan or linked, worktrees left
/// to their clone.
fn clones(p: &GitPlugin) -> Vec<CloneRow> {
    let mut out: Vec<CloneRow> = Vec::new();
    let found = p.local.index.iter().flat_map(|i| i.checkouts.iter());
    for c in found
        .chain(p.local.linked.iter())
        .filter(|c| c.main.is_empty())
    {
        if !out.iter().any(|o| o.dir == c.dir) {
            out.push(CloneRow {
                dir: c.dir.clone(),
                project: c.project.clone(),
            });
        }
    }
    out.sort_by(|a, b| a.dir.cmp(&b.dir));
    out
}

fn checkout_of<'a>(p: &'a GitPlugin, dir: &str) -> Option<&'a Checkout> {
    let found = p.local.index.iter().flat_map(|i| i.checkouts.iter());
    found.chain(p.local.linked.iter()).find(|c| c.dir == dir)
}

fn base_name(dir: &str) -> String {
    Path::new(dir)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| dir.to_string())
}

/// Ask for every clone to be read (those not being read already).
pub fn refresh(p: &mut GitPlugin) -> Vec<Effect> {
    p.branches.read_at = p.now;
    let known: BTreeSet<String> = clones(p).into_iter().map(|c| c.dir).collect();
    p.branches.details.retain(|dir, _| known.contains(dir));
    p.branches.incomplete.retain(|dir| known.contains(dir));
    p.branches.round = known.clone();
    let mut fx = Vec::new();
    for dir in known {
        fx.push(request_read(p, dir));
    }
    fx
}

/// Reading starts once the screen has been looked at, then again each minute.
pub fn tick(p: &mut GitPlugin) -> Vec<Effect> {
    let b = &p.branches;
    let due = b.read_at == 0 || p.now - b.read_at >= REFRESH_SECS;
    if b.seen.get() && due {
        refresh(p)
    } else {
        vec![]
    }
}

pub fn on_msg(p: &mut GitPlugin, msg: BranchMsg) -> Vec<Effect> {
    match msg {
        BranchMsg::Read {
            dir,
            mut detail,
            observed,
            action,
        } => {
            p.branches.loading.remove(&dir);
            let completed_round = p.branches.round.remove(&dir) && p.branches.round.is_empty();
            if observed.outcome() == JournalOutcome::Succeeded {
                p.branches.incomplete.remove(&dir);
            } else {
                p.branches.incomplete.insert(dir.clone());
            }
            // A failed status read may keep measurements only for the same
            // worktree/checkout revision. They remain explicitly last known.
            if let Some(previous) = p.branches.details.get(&dir) {
                for tree in &mut detail.trees {
                    if tree.status == StatusRead::Unavailable
                        && let Some(old) = previous.trees.iter().find(|old| {
                            old.dir == tree.dir
                                && old.branch == tree.branch
                                && old.head == tree.head
                                && old.status != StatusRead::Unavailable
                        })
                    {
                        (tree.dirty, tree.ahead, tree.behind) = (old.dirty, old.ahead, old.behind);
                        tree.status = StatusRead::LastKnown;
                    }
                }
            }
            p.branches.details.insert(dir, *detail);
            if completed_round && p.branches.incomplete.is_empty() {
                p.branches.last_ok = Some(p.now);
            }
            observed.effects(&action)
        }
        BranchMsg::ReadInterrupted { dir, why } => {
            p.branches.loading.remove(&dir);
            p.branches.round.remove(&dir);
            // The task did not establish current status. Keep useful numbers,
            // but never let a stale dirty count authorize a forced removal.
            if let Some(detail) = p.branches.details.get_mut(&dir) {
                for tree in &mut detail.trees {
                    if tree.status == StatusRead::Current {
                        tree.status = StatusRead::LastKnown;
                    }
                }
            }
            p.branches.incomplete.insert(dir);
            vec![
                Effect::Report(JournalOutcome::Unknown),
                Effect::error(None, radar_core::writes::failed_line(&why)),
            ]
        }
        BranchMsg::Done { said, reread } => {
            let outcome = if said.is_ok() {
                JournalOutcome::Succeeded
            } else {
                JournalOutcome::Unknown
            };
            let mut fx = vec![Effect::Report(outcome)];
            fx.extend(finished(p, said, reread));
            fx
        }
        BranchMsg::Pruned { result, reread } => {
            // Finish the inherited aggregate before creating independent step
            // identities. Rereads have their own scoped semantic action.
            let mut fx = vec![Effect::Report(result.outcome())];
            fx.extend(finished(p, result.said, reread));
            for step in result.steps {
                fx.push(journal_prune_step(&step));
                fx.push(Effect::Report(step.status.outcome()));
            }
            fx
        }
    }
}

fn finished(p: &mut GitPlugin, said: Result<String, String>, reread: Vec<String>) -> Vec<Effect> {
    let mut fx = vec![match said {
        Ok(said) => Effect::Notify(said),
        Err(why) => Effect::error(None, radar_core::writes::failed_line(&why)),
    }];
    for dir in reread {
        p.branches.changing.remove(&dir);
        fx.push(request_read(p, dir));
    }
    fx
}

pub fn busy(p: &GitPlugin) -> bool {
    !p.branches.loading.is_empty() || !p.branches.changing.is_empty()
}

// --- what is selected --------------------------------------------------------------

/// A row of the tree.
#[derive(Debug)]
pub(crate) enum Sel<'a> {
    Clone(&'a Detail),
    Tree(&'a Detail, &'a Tree),
    Branch(&'a Detail, &'a Branch),
}

fn clone_id(dir: &str) -> String {
    format!("c:{dir}")
}

fn tree_id(dir: &str) -> String {
    format!("w:{dir}")
}

fn branch_id(name: &str) -> String {
    format!("b:{name}")
}

/// A route to an already-visible clone or held worktree, never a name match.
pub(crate) fn checkout_route(p: &GitPlugin, dir: &str) -> Option<Vec<String>> {
    for clone in clones(p) {
        if clone.dir == dir {
            return Some(vec![clone_id(dir)]);
        }
        if p.branches.details.get(&clone.dir).is_some_and(|detail| {
            detail.trees.iter().skip(1).any(|tree| tree.dir == dir)
        }) {
            return Some(vec![clone_id(&clone.dir), tree_id(dir)]);
        }
    }
    None
}

/// Source navigation only changes the current view, never the active checkout.
pub(crate) fn show_checkout(p: &mut GitPlugin, dir: &str) -> radar_core::Showing {
    let Some(path) = checkout_route(p, dir) else {
        return radar_core::Showing::Cannot;
    };
    p.branches.view = 0;
    let mut tree = p.branches.tree.borrow_mut();
    if path.len() > 1 {
        tree.open(path[..1].to_vec());
    }
    tree.select(path);
    radar_core::Showing::Page {
        screen: SCREEN.id,
        effects: vec![Effect::Focus(Pane::Middle)],
    }
}

pub(crate) fn source_subject(p: &GitPlugin, pane: Pane) -> Option<Subject> {
    if pane == Pane::Nav {
        return None;
    }
    let tree = p.branches.tree.borrow();
    let path = tree.selected();
    let clone = path.first()?.strip_prefix("c:")?;
    let dir = path
        .get(1)
        .and_then(|id| id.strip_prefix("w:"))
        .unwrap_or(clone);
    let key = radar_core::journal::SubjectKey::new("local", dir, "checkout", dir).ok()?;
    crate::source_navigation::from_journal(p, &key)
}

pub(crate) fn selected(p: &GitPlugin) -> Option<Sel<'_>> {
    let state = p.branches.tree.borrow();
    let path = state.selected();
    let dir = path.first()?.strip_prefix("c:")?;
    let d = p.branches.details.get(dir)?;
    match path.get(1).map(String::as_str) {
        None => Some(Sel::Clone(d)),
        Some(id) => {
            if let Some(dir) = id.strip_prefix("w:") {
                d.trees
                    .iter()
                    .find(|t| t.dir == dir)
                    .map(|t| Sel::Tree(d, t))
            } else {
                let name = id.strip_prefix("b:")?;
                d.branches
                    .iter()
                    .find(|b| b.name == name)
                    .map(|b| Sel::Branch(d, b))
            }
        }
    }
}

/// The selected row's own name, for menus and the detail pane's title.
fn name_of(sel: Option<&Sel<'_>>) -> String {
    match sel {
        Some(Sel::Clone(d)) => base_name(&d.dir),
        Some(Sel::Tree(_, t)) => base_name(&t.dir),
        Some(Sel::Branch(_, b)) => b.name.clone(),
        None => "branches".into(),
    }
}

fn sel_icon(sel: Option<&Sel<'_>>) -> Icon {
    match sel {
        Some(Sel::Clone(_)) | None => Icon::Repo,
        Some(Sel::Tree(..)) => Icon::Folder,
        Some(Sel::Branch(..)) => Icon::Branch,
    }
}

// --- the nav pane and the tree ---------------------------------------------------------

fn visible(view: View, d: Option<&Detail>) -> bool {
    match view {
        View::All => true,
        View::Dirty => {
            d.is_some_and(|d| d.dirty() || d.trees.iter().any(|t| t.status != StatusRead::Current))
        }
        View::Ahead => d.is_some_and(|d| d.ahead() > 0),
        View::Stashes => d.is_some_and(|d| !d.stashes.is_empty()),
    }
}

/// What each view would show, for the nav pane: the clones that are dirty, the
/// branches ahead, the stashes held.
fn counts(p: &GitPlugin) -> [usize; 4] {
    let details = p.branches.details.values();
    let (mut dirty, mut ahead, mut stashes) = (0, 0, 0);
    for d in details {
        dirty += d
            .trees
            .iter()
            .filter(|t| t.status == StatusRead::Current && t.dirty > 0)
            .count();
        ahead += d.ahead();
        stashes += d.stashes.len();
    }
    [0, dirty, ahead, stashes]
}

/// The screen's own columns; `lead` is the cells an icon's lead takes.
fn columns(lead: u16) -> Vec<ColumnSpec> {
    vec![
        ColumnSpec::new("ahead", 7, 5),
        ColumnSpec::new("behind", 8, 4),
        ColumnSpec::new("dirty", 11 + lead, 6).icon(Icon::Warning),
        ColumnSpec::new("stash", 7 + lead, 2).icon(Icon::Folder),
        ColumnSpec::new("updated", 8 + lead, 3).icon(Icon::Updated),
    ]
}

fn plain(text: impl Into<String>) -> Text {
    Text::of(text, Ink::Muted)
}

fn ahead_cell(ahead: usize) -> Text {
    if ahead > 0 {
        Text::of(format!("↑{ahead}"), Ink::Number)
    } else {
        plain("—")
    }
}

fn behind_cell(behind: usize, gone: bool) -> Text {
    if gone {
        Text::of("gone", Ink::Warn)
    } else if behind > 0 {
        Text::of(format!("↓{behind}"), Ink::Warn)
    } else {
        plain("—")
    }
}

fn dirty_cell(dirty: Option<usize>) -> Text {
    match dirty {
        Some(0) => plain("clean"),
        Some(n) => Text::new()
            .push(Seg::icon(Icon::Warning))
            .push(Seg::new(format!("{n} changed"), Ink::Warn)),
        None => plain("—"),
    }
}

/// The narrow cell never labels an unread status clean; its detail pane has detail.
fn tree_dirty_cell(tree: &Tree) -> Text {
    match tree.status {
        StatusRead::Current => dirty_cell(Some(tree.dirty)),
        StatusRead::Unavailable => Text::of("unknown", Ink::Warn),
        StatusRead::LastKnown => Text::of("last known", Ink::Warn),
    }
}

fn tree_changes(tree: &Tree) -> Text {
    let mut words = if tree.status == StatusRead::Unavailable {
        Text::of("unavailable", Ink::Warn)
    } else {
        tree_dirty_cell(tree)
    };
    if tree.status == StatusRead::LastKnown {
        words.add(Seg::new(": ", Ink::Plain));
        words.add(Seg::new(format!("{} changed", tree.dirty), Ink::Muted));
    }
    words
}

fn status_note(tree: &Tree) -> Option<Text> {
    (tree.status != StatusRead::Current).then(|| Text::of(
        "Working tree status could not be read. Read the clones again before removing this worktree.",
        Ink::Warn,
    ))
}

fn stash_cell(n: usize) -> Text {
    if n > 0 {
        Text::of(n.to_string(), Ink::Number)
    } else {
        plain("—")
    }
}

fn updated_cell(at: i64, now: i64) -> Text {
    if at > 0 {
        plain(time::ago(at, now))
    } else {
        plain("—")
    }
}

/// A row's own cells, then those other parts add for its subject.
fn with_cells(mut row: TreeRow, own: Vec<(&str, Text)>, extra: Vec<radar_core::Cell>) -> TreeRow {
    for (h, s) in own {
        row = row.cell(h, s);
    }
    row.cells.extend(extra.iter().map(v::cell_of));
    row
}

/// The subject other parts see for a branch.
fn subject_of(c: &CloneRow, branch: &str) -> Subject {
    Subject::new("checkout", c.dir.clone(), base_name(&c.dir))
        .with("repo", c.project.clone())
        .with("branch", branch)
}

/// Whether the tree has the row a target names, among the rows `tree_rows`
/// builds (other parts' cells do not change which rows there are).
fn holds_row(p: &GitPlugin, target: &v::Target) -> bool {
    let cx = v::ViewCx {
        now: 0,
        focus: v::PaneId::Middle,
        width: 160,
        height: 40,
        parts: radar_core::compose::Parts::none(),
        icon_lead: 0,
        setup: false,
    };
    let tree = v::Tree { rows: tree_rows(p, &cx), ..v::Tree::default() };
    tree.row(&TreeRow::path_of(target)).is_some()
}

fn tree_rows(p: &GitPlugin, cx: &v::ViewCx<'_>) -> Vec<TreeRow> {
    let now = cx.now;
    let view = p.branches.view();
    let mut out = Vec::new();
    for c in clones(p) {
        let d = p.branches.details.get(&c.dir);
        if !visible(view, d) {
            continue;
        }
        let top = d.and_then(|d| d.trees.first());
        let chosen = local::chosen(p, &c.project).map(|c| c.dir.clone());
        let in_use = |dir: &str| chosen.as_deref() == Some(dir);
        let mut name = Text::new()
            .push(Seg::icon(Icon::Repo))
            .push(Seg::new(base_name(&c.dir), Ink::Primary).strong());
        if let Some(top) = top {
            name.add(Seg::new("  on ", Ink::Muted));
            name.add(Seg::icon(Icon::Branch));
            name.add(Seg::new(
                if top.branch.is_empty() {
                    format!("detached at {}", top.head)
                } else {
                    top.branch.clone()
                },
                Ink::Muted,
            ));
        }
        let has_trees = d.is_some_and(|d| d.trees.len() > 1);
        if has_trees && in_use(&c.dir) {
            name.add(Seg::new("  ", Ink::Plain));
            name.add(Seg::icon(Icon::Local));
            name.add(Seg::new("in use", Ink::Good));
        }
        let mut node = TreeRow::new(clone_id(&c.dir), name);
        let newest = d.map_or(0, |d| {
            d.branches.iter().map(|b| b.updated).max().unwrap_or(0)
        });
        let own = match (d, top) {
            (Some(d), Some(top)) if d.error.is_empty() => vec![
                (
                    "ahead",
                    if top.status == StatusRead::Current {
                        ahead_cell(top.ahead)
                    } else {
                        plain("—")
                    },
                ),
                (
                    "behind",
                    if top.status == StatusRead::Current {
                        behind_cell(top.behind, false)
                    } else {
                        plain("—")
                    },
                ),
                ("dirty", tree_dirty_cell(top)),
                ("stash", stash_cell(d.stashes.len())),
                ("updated", updated_cell(newest, now)),
            ],
            (Some(d), _) => vec![("dirty", Text::of(d.error.clone(), Ink::Bad))],
            (None, _) => vec![("dirty", plain("reading…"))],
        };
        node = with_cells(node, own, vec![]);
        if let Some(d) = d {
            // Worktrees, then the local branches.
            if matches!(view, View::All | View::Dirty) {
                for tree in
                    d.trees.iter().skip(1).filter(|w| {
                        view == View::All || w.dirty > 0 || w.status != StatusRead::Current
                    })
                {
                    let mut name = Text::new()
                        .push(Seg::icon(Icon::Folder))
                        .push(Seg::new(base_name(&tree.dir), Ink::Primary))
                        .push(Seg::new("  on ", Ink::Muted))
                        .push(Seg::icon(Icon::Branch))
                        .push(Seg::new(
                            if tree.branch.is_empty() {
                                format!("detached at {}", tree.head)
                            } else {
                                tree.branch.clone()
                            },
                            Ink::Muted,
                        ));
                    if in_use(&tree.dir) {
                        name.add(Seg::new("  ", Ink::Plain));
                        name.add(Seg::icon(Icon::Local));
                        name.add(Seg::new("in use", Ink::Good));
                    }
                    let updated = d
                        .branches
                        .iter()
                        .find(|b| b.name == tree.branch)
                        .map_or(0, |b| b.updated);
                    let own = vec![
                        (
                            "ahead",
                            if tree.status == StatusRead::Current {
                                ahead_cell(tree.ahead)
                            } else {
                                plain("—")
                            },
                        ),
                        (
                            "behind",
                            if tree.status == StatusRead::Current {
                                behind_cell(tree.behind, false)
                            } else {
                                plain("—")
                            },
                        ),
                        ("dirty", tree_dirty_cell(tree)),
                        ("stash", plain("—")),
                        ("updated", updated_cell(updated, now)),
                    ];
                    node = node.child(with_cells(
                        TreeRow::new(tree_id(&tree.dir), name),
                        own,
                        vec![],
                    ));
                }
            }
            if matches!(view, View::All | View::Ahead) {
                for b in d
                    .branches
                    .iter()
                    .filter(|b| view == View::All || b.ahead > 0)
                {
                    let here = d.on(&b.name);
                    let mut name = Text::new()
                        .push(Seg::icon(if b.merged && b.name != d.base {
                            Icon::Merged
                        } else {
                            Icon::Branch
                        }))
                        .push(Seg::new(b.name.clone(), Ink::Primary));
                    if let Some(w) = here.first() {
                        let where_ = if w.dir == d.dir {
                            "here".to_string()
                        } else {
                            format!("in {}", base_name(&w.dir))
                        };
                        name.add(Seg::new("  ", Ink::Plain));
                        name.add(Seg::icon(Icon::Local));
                        name.add(Seg::new(where_, Ink::Good));
                    } else if b.merged && b.name != d.base {
                        name.add(Seg::new("  merged", Ink::Muted));
                    }
                    let dirty = here
                        .iter()
                        .find(|w| w.status != StatusRead::Current)
                        .map_or_else(
                            || dirty_cell(here.iter().map(|w| w.dirty).max()),
                            |w| tree_dirty_cell(w),
                        );
                    let own = vec![
                        ("ahead", ahead_cell(b.ahead)),
                        ("behind", behind_cell(b.behind, b.gone)),
                        ("dirty", dirty),
                        ("stash", plain("—")),
                        ("updated", updated_cell(b.updated, now)),
                    ];
                    let extra = cx.parts.cells(&subject_of(&c, &b.name));
                    node = node.child(with_cells(
                        TreeRow::new(branch_id(&b.name), name),
                        own,
                        extra,
                    ));
                }
            }
        }
        out.push(node);
    }
    out
}

/// The columns other parts add to the branches' rows: each heading once, as
/// wide as its widest value (within reason).
fn extra_columns(p: &GitPlugin, cx: &v::ViewCx<'_>) -> Vec<ColumnSpec> {
    let mut out: Vec<ColumnSpec> = Vec::new();
    for c in clones(p) {
        let Some(d) = p.branches.details.get(&c.dir) else {
            continue;
        };
        for b in &d.branches {
            for cell in cx.parts.cells(&subject_of(&c, &b.name)) {
                let lead = cell.icon.map_or(0, |i| ui::width(&icon::plain(i)));
                let w = ((lead + ui::width(&cell.text)).max(ui::width(&cell.heading)) + 1).min(24);
                let w = w as u16;
                match out.iter_mut().find(|x| x.heading == cell.heading) {
                    Some(x) => x.width = x.width.max(w),
                    None => out.push(ColumnSpec::new(cell.heading, w, 1)),
                }
            }
        }
    }
    out
}

/// The screen as view data: the nav pane of views, the tree, the detail pane of
/// what is selected, and the actions where the focus is.
pub fn view(p: &GitPlugin, cx: &v::ViewCx<'_>) -> v::ScreenView {
    p.branches.seen.set(true);
    let mut nav = PaneView::new(PaneId::Nav, "views", nav(p));
    nav.width = Some(26);
    // The tree first: it keeps the selection on a row that is there.
    let middle = middle(p, cx);
    let sel = selected(p);
    let detail = PaneView::new(
        PaneId::Detail,
        name_of(sel.as_ref()),
        Node::Rich {
            blocks: detail_blocks(p, cx.now),
            pictures: None,
        },
    );
    let mut out = v::ScreenView::new(SCREEN.id, vec![nav, middle, detail]);
    out.actions = keys(p, cx.focus.into())
        .iter()
        .map(|b| v::ActionSpec::named(action_id(b), b))
        .collect();
    out
}

/// The stable id of a row of the branch screen, chosen here by hand from its key
/// (`git.branch.<verb>`); a label edit never changes it. A heading has no id.
fn action_id(b: &Binding) -> &'static str {
    if b.is_heading() {
        return "";
    }
    match b.keys.as_str() {
        "⏎" => "git.branch.use",
        "d" => "git.branch.delete",
        "A" => crate::agent_context::SEND_ID,
        "r" => "git.branch.read_again",
        other => unreachable!("the branch screen has no stable id for key {other:?}"),
    }
}

/// The nav pane: each view, with a count only where it is something to do.
fn nav(p: &GitPlugin) -> Node {
    let n = counts(p);
    let rows = View::ALL
        .iter()
        .zip(n)
        .enumerate()
        .map(|(i, (view, n))| {
            let name = Text::new()
                .push(Seg::icon(view.icon()))
                .push(Seg::new(view.name(), Ink::Plain));
            let row = v::ListRow::new(format!("view:{i}"), name);
            if n > 0 {
                row.right(Text::of(n.to_string(), Ink::Number))
            } else {
                row
            }
        })
        .collect();
    Node::List {
        rows,
        selected: Some(p.branches.view),
        editing: None,
    }
}

fn summary(p: &GitPlugin) -> String {
    let rows = clones(p);
    let n = rows.len();
    let trees: usize = p
        .branches
        .details
        .values()
        .map(|d| d.trees.len().saturating_sub(1))
        .sum();
    let s = if n == 1 { "" } else { "s" };
    if trees > 0 {
        let w = if trees == 1 { "" } else { "s" };
        format!("{n} clone{s}, {trees} worktree{w}")
    } else {
        format!("{n} clone{s}")
    }
}

/// The middle pane: what the view shows at its foot, and the tree of
/// clones, or why there is none.
fn middle(p: &GitPlugin, cx: &v::ViewCx<'_>) -> PaneView {
    let state = |state, text: &str| Node::State {
        state,
        text: text.to_string(),
    };
    let body = if clones(p).is_empty() {
        state(
            v::State::Empty,
            if p.local.scanning {
                "Looking for clones on this machine…"
            } else {
                "No clones known yet. Press , for settings, then add a folder under git › checkouts."
            },
        )
    } else if p.branches.details.is_empty() {
        if p.branches.loading.is_empty() && !p.branches.incomplete.is_empty() {
            state(
                v::State::Empty,
                "Could not read the clones. Read them again from the actions menu.",
            )
        } else {
            state(v::State::Loading, "Reading the clones…")
        }
    } else {
        let rows = tree_rows(p, cx);
        if rows.is_empty() {
            state(
                v::State::Empty,
                match p.branches.view() {
                    View::All => "Nothing to show.",
                    View::Dirty => "No uncommitted changes shown in this view.",
                    View::Ahead => "No branches with unpushed commits shown.",
                    View::Stashes => "No stashes shown in this view.",
                },
            )
        } else {
            let mut nav = p.branches.tree.borrow_mut();
            nav.sync(&rows);
            // Other parts' columns go before `updated`.
            let mut all = columns(cx.icon_lead as u16);
            let at = all.len() - 1;
            all.splice(at..at, extra_columns(p, cx));
            Node::Tree(v::Tree {
                header: Text::new()
                    .push(Seg::new("CLONES", Ink::Muted).strong())
                    .push(Seg::new(format!(" · {}", summary(p)), Ink::Muted)),
                columns: all,
                rows,
                selected: nav.selected.clone(),
                open: nav.open.clone(),
                name_width: NAME,
                cut_names: true,
                ..v::Tree::default()
            })
        }
    };
    let mut pane = PaneView::new(PaneId::Middle, p.branches.view().name(), body);
    if !p.branches.incomplete.is_empty() {
        pane.notes
            .push(Text::of("Some clone data could not be read.", Ink::Warn));
    }
    // At the foot: what the view shows, in words.
    pane.notes.push(Text::of(
        if p.branches.view() == View::Dirty
            && p.branches
                .details
                .values()
                .any(|d| d.trees.iter().any(|t| t.status != StatusRead::Current))
        {
            "clones and worktrees with changes or unread status"
        } else {
            p.branches.view().words()
        },
        Ink::Secondary,
    ));
    pane
}

// --- detail pane -----------------------------------------------------------------------

fn unpushed_words(n: usize) -> String {
    format!("{n} commit{} on no remote", if n == 1 { "" } else { "s" })
}

fn files_words(n: usize) -> String {
    format!("{n} file{}", if n == 1 { "" } else { "s" })
}

/// Labels in a column, each value after its label.
fn kv(rows: Vec<(&str, Text)>) -> Vec<Text> {
    let w = rows.iter().map(|(l, _)| ui::width(l)).max().unwrap_or(0) + 2;
    rows.into_iter()
        .map(|(label, value)| {
            let mut line = Text::of(ui::pad(label, w), Ink::Muted);
            line.extend(value);
            line
        })
        .collect()
}


/// The detail pane: what is selected, its facts, then its worktrees and
/// stashes in sections.
fn detail_blocks(p: &GitPlugin, now: i64) -> Vec<v::Block> {
    let text = |s: String| Text::of(s, Ink::Primary);
    let mut lines = Vec::new();
    let mut sections = Vec::new();
    match selected(p) {
        None => {
            lines.push(plain("Select a clone, a worktree or a branch."));
        }
        Some(Sel::Clone(d)) => {
            let top = d.trees.first();
            let mut rows = vec![("folder", text(local::display_path(p, &d.dir)))];
            if let Some(c) = checkout_of(p, &d.dir) {
                if !c.remote.is_empty() {
                    rows.push(("origin", text(c.remote.clone())));
                }
                if !c.project.is_empty() {
                    rows.push(("repo", text(c.project.clone())));
                }
            }
            if let Some(top) = top {
                rows.push((
                    "on",
                    text(if top.branch.is_empty() {
                        format!("detached at {}", top.head)
                    } else {
                        top.branch.clone()
                    }),
                ));
                rows.push(("changes", tree_changes(top)));
            }
            if !d.base.is_empty() {
                rows.push(("base", text(d.base.clone())));
            }
            lines.extend(kv(rows));
            lines.extend(top.and_then(status_note));
            if !d.error.is_empty() {
                lines.push(Text::new());
                lines.push(
                    Text::new()
                        .push(Seg::icon(Icon::Error))
                        .push(Seg::new(format!("could not load: {}", d.error), Ink::Bad)),
                );
            }
            if d.trees.len() > 1 {
                let mut trees = Vec::new();
                for w in d.trees.iter().skip(1) {
                    let mut line = Text::new().push(Seg::icon(Icon::Folder));
                    line.extend(text(local::display_path(p, &w.dir)));
                    if !w.branch.is_empty() {
                        line.add(Seg::new(format!("  {}", w.branch), Ink::Muted));
                    }
                    if w.status != StatusRead::Current || w.dirty > 0 {
                        line.add(Seg::new("  ", Ink::Plain));
                        line.extend(tree_changes(w));
                    }
                    trees.push(line);
                }
                sections.push(v::Block::line(Text::new()));
                sections.push(v::Block::section_of(Some(Icon::Folder), "worktrees", None, trees));
            }
            if !d.stashes.is_empty() {
                let stashes = d
                    .stashes
                    .iter()
                    .map(|s| {
                        let mut line = Text::new().push(Seg::icon(Icon::Folder));
                        line.extend(text(s.clone()));
                        line
                    })
                    .collect();
                sections.push(v::Block::line(Text::new()));
                sections.push(v::Block::section_of(Some(Icon::Folder), "stashes", None, stashes));
            }
        }
        Some(Sel::Tree(d, w)) => {
            let mut rows = vec![("folder", text(local::display_path(p, &w.dir)))];
            rows.push(("clone", text(local::display_path(p, &d.dir))));
            rows.push((
                "on",
                text(if w.branch.is_empty() {
                    format!("detached at {}", w.head)
                } else {
                    w.branch.clone()
                }),
            ));
            rows.push(("changes", tree_changes(w)));
            if w.status == StatusRead::Current && w.ahead + w.behind > 0 {
                rows.push((
                    "remote",
                    Text::of(format!("↑{} ↓{}", w.ahead, w.behind), Ink::Number),
                ));
            }
            lines.extend(kv(rows));
            lines.extend(status_note(w));
            lines.push(Text::new());
            lines.push(plain("⏎ makes it the one radar acts on for this repo"));
        }
        Some(Sel::Branch(d, b)) => {
            let mut rows = vec![("clone", text(local::display_path(p, &d.dir)))];
            rows.push((
                "upstream",
                if b.gone {
                    Text::of(format!("{} (gone from the remote)", b.upstream), Ink::Warn)
                } else if b.upstream.is_empty() {
                    plain("none: it is only here")
                } else {
                    text(b.upstream.clone())
                },
            ));
            if b.ahead + b.behind > 0 {
                rows.push((
                    "remote",
                    Text::of(format!("↑{} ↓{}", b.ahead, b.behind), Ink::Number),
                ));
            }
            if b.unpushed > 0 {
                rows.push(("unpushed", Text::of(unpushed_words(b.unpushed), Ink::Warn)));
            }
            if !d.base.is_empty() && b.name != d.base {
                rows.push((
                    "merged",
                    plain(if b.merged {
                        format!("into {}", d.base)
                    } else {
                        format!("not into {} yet", d.base)
                    }),
                ));
            }
            let here: Vec<String> = d.on(&b.name).iter().map(|w| local::display_path(p, &w.dir)).collect();
            if !here.is_empty() {
                rows.push(("checked out", text(here.join(", "))));
            }
            if b.updated > 0 {
                rows.push((
                    "last commit",
                    text(format!("{} {} ago", b.sha, time::ago(b.updated, now))),
                ));
            }
            lines.extend(kv(rows));
            if !b.subject.is_empty() {
                lines.push(Text::new());
                lines.push(Text::of(b.subject.clone(), Ink::Secondary));
            }
        }
    }
    lines.into_iter().map(v::Block::line).chain(sections).collect()
}

pub fn top_right(p: &GitPlugin) -> Option<String> {
    if !busy(p) && !p.branches.incomplete.is_empty() {
        return Some(match p.branches.last_ok {
            Some(at) => format!(
                "read incomplete · last complete {} ago",
                time::ago(at, p.now)
            ),
            None => "read incomplete · no complete sync yet".into(),
        });
    }
    Some(ui::status::sync_words(
        "clones",
        busy(p),
        p.branches.last_ok,
        p.now,
        Some(REFRESH_SECS as u64),
    ))
}

pub fn status(p: &GitPlugin) -> Vec<(String, Tone)> {
    let n = counts(p);
    let mut out = Vec::new();
    if n[1] > 0 {
        out.push((format!("{} dirty", n[1]), Tone::Warn));
    }
    if n[2] > 0 {
        out.push((format!("{} ahead", n[2]), Tone::Info));
    }
    out
}

// --- keys and clicks ---------------------------------------------------------------

/// The nav pane's view a target names.
fn view_at(target: &v::Target) -> Option<usize> {
    target.as_str().strip_prefix("view:")?.parse().ok()
}

/// What the user did on the screen, from any frontend: choose a view in the
/// nav pane, move or fold in the middle pane, `⏎` on a clone or worktree to use it,
/// `r` to read again, `d` to delete.
pub fn intent(p: &mut GitPlugin, pane: Pane, intent: &Intent) -> Outcome {
    let handled = |fx: Option<Vec<Effect>>| fx.map_or(Outcome::Ignored, Outcome::Handled);
    match intent {
        Intent::Select { target } | Intent::Context { target } if view_at(target).is_some() => {
            p.branches.view = view_at(target).unwrap_or(0).min(View::ALL.len() - 1);
            Outcome::done()
        }
        // `⏎` on the nav pane is the host's: it goes into the tree.
        Intent::Activate { target } if view_at(target).is_some() => Outcome::Ignored,
        // A target the tree does not hold (a pane's empty space) is not ours.
        Intent::Select { target }
        | Intent::Context { target }
        | Intent::Fold { target, .. }
        | Intent::Activate { target }
            if !holds_row(p, target) =>
        {
            Outcome::Ignored
        }
        Intent::Select { .. } | Intent::Context { .. } | Intent::Fold { .. } => {
            p.branches.tree.borrow_mut().apply(intent);
            Outcome::done()
        }
        Intent::Activate { target } => {
            p.branches.tree.borrow_mut().select(TreeRow::path_of(target));
            handled(use_it(p))
        }
        // The host turns a chosen cell into a selection and a show.
        Intent::Show { .. } | Intent::End { .. } | Intent::EditSelect { .. } | Intent::EditUnselect | Intent::Dock(_) | Intent::Copy { .. } | Intent::NoticeInput { .. } | Intent::DismissNotice { .. } | Intent::OpenAll => Outcome::Ignored,
        Intent::Invoke { .. } | Intent::Key { .. } => match intent.chord().map(|c| c.key) {
            Some(Key::Char('r')) => Outcome::Handled(refresh_now(p)),
            Some(Key::Char('d')) => Outcome::Handled(delete(p)),
            Some(Key::Char('A')) if pane != Pane::Nav => {
                Outcome::Handled(crate::agent_context::send_selected(p))
            }
            Some(Key::Enter) if pane == Pane::Middle => handled(use_it(p)),
            _ => Outcome::Ignored,
        },
    }
}

/// A key on the screen, routed as the host routes it: the screen's view says
/// what the key means (tests that press keys).
#[cfg(test)]
pub(crate) fn key(p: &mut GitPlugin, pane: Pane, chord: radar_core::view::Chord) -> Outcome {
    use radar_core::Plugin;
    use radar_core::route::{Frontend, Routed, to_part};
    let fe = Frontend::default();
    let (width, height) = fe.room();
    let cx = v::ViewCx {
        now: radar_core::plugin::DEMO_NOW,
        focus: pane.into(),
        width,
        height,
        parts: radar_core::compose::Parts::none(),
        icon_lead: fe.lead(),
        setup: false,
    };
    let routed = if p.capturing() {
        Routed::Intent(Intent::Key { chord })
    } else {
        match p.view(SCREEN.id, &cx) {
            Some(view) => radar_core::route::key(&fe, &view, pane.into(), chord),
            None => Routed::Ignored,
        }
    };
    match routed {
        Routed::Intent(i) => to_part(p, SCREEN.id, pane, &i),
        Routed::Done => Outcome::done(),
        Routed::Ignored => Outcome::Ignored,
    }
}

/// A click on a target the screen's view drew (tests).
#[cfg(test)]
pub(crate) fn click(p: &mut GitPlugin, id: &str) -> Outcome {
    intent(p, Pane::Middle, &Intent::Select { target: v::Target::new(id) })
}

pub fn panes() -> Vec<Pane> {
    vec![Pane::Nav, Pane::Middle, Pane::Detail]
}

pub fn row_hints(p: &GitPlugin, pane: Pane) -> Vec<KeyHint<'static>> {
    if pane == Pane::Nav {
        return vec![];
    }
    let mut hints = row_hints_of(p, pane);
    if crate::agent_context::can_send(p) {
        hints.push(KeyHint::new("A", "ask an agent"));
    }
    hints
}

fn row_hints_of(p: &GitPlugin, pane: Pane) -> Vec<KeyHint<'static>> {
    match selected(p) {
        Some(Sel::Tree(_, tree)) if tree.status != StatusRead::Current => {
            let mut hints = Vec::new();
            if pane == Pane::Middle {
                hints.push(KeyHint::new("⏎", "use it"));
            }
            hints.push(KeyHint::new("r", "retry status"));
            hints
        }
        Some(Sel::Tree(..)) if pane == Pane::Middle => {
            vec![KeyHint::new("⏎", "use it"), KeyHint::new("d", "delete")]
        }
        Some(Sel::Tree(..)) => vec![KeyHint::new("d", "delete")],
        Some(Sel::Clone(_)) if pane == Pane::Middle => vec![KeyHint::new("⏎", "use it")],
        Some(Sel::Branch(..)) => vec![KeyHint::new("d", "delete")],
        _ => vec![],
    }
}

fn refresh_now(p: &mut GitPlugin) -> Vec<Effect> {
    let mut fx = refresh(p);
    fx.push(Effect::Notify("reading the clones again…".into()));
    fx
}

/// The actions menu (`x`): what can be done to what is selected.
pub fn keys(p: &GitPlugin, pane: Pane) -> Vec<Binding> {
    let sel = selected(p);
    let mut out = Vec::new();
    if pane != Pane::Nav && sel.is_some() {
        out.push(Binding::heading(name_of(sel.as_ref())).icon(sel_icon(sel.as_ref())));
        match &sel {
            Some(Sel::Tree(_, tree)) => {
                if pane == Pane::Middle {
                    out.push(Binding::new("⏎", "use this worktree for its repo"));
                }
                if tree.status == StatusRead::Current {
                    out.push(Binding::new("d", "delete the worktree…"));
                }
            }
            Some(Sel::Clone(_)) if pane == Pane::Middle => {
                out.push(Binding::new("⏎", "use the clone itself for its repo"));
            }
            Some(Sel::Branch(..)) => out.push(Binding::new("d", "delete the branch…")),
            Some(Sel::Clone(_)) | None => {}
        }
        if crate::agent_context::can_send(p) {
            out.push(Binding::new("A", "ask an agent about it…").group(radar_core::view::ActionGroup::Ask));
        }
    }
    out.push(Binding::new("r", "read the clones again").pane());
    out
}

pub fn folds(pane: Pane) -> bool {
    pane == Pane::Middle
}

// --- use, delete and prune ---------------------------------------------------------

/// `⏎` on a clone or a worktree: the one acted on for its repo.
fn use_it(p: &mut GitPlugin) -> Option<Vec<Effect>> {
    let (dir, clone) = match selected(p)? {
        Sel::Clone(d) => (d.dir.clone(), d.dir.clone()),
        Sel::Tree(d, t) => (t.dir.clone(), d.dir.clone()),
        Sel::Branch(..) => return None,
    };
    let project = checkout_of(p, &clone)
        .map(|c| c.project.clone())
        .unwrap_or_default();
    if project.is_empty() {
        return Some(vec![Effect::Notify(
            "its origin names no repo, so there is nothing to use it for".into(),
        )]);
    }
    if !local::checkouts_for(p, &project)
        .iter()
        .any(|c| c.dir == dir)
    {
        return Some(vec![Effect::Notify(
            "not looked for yet: press s on a repo to look for checkouts again".into(),
        )]);
    }
    if dir == clone {
        p.local.use_worktree.remove(&project);
    } else {
        p.local.use_worktree.insert(project.clone(), dir.clone());
    }
    Some(vec![
        Effect::PersistCache,
        Effect::Notify(format!("{project}: acting on {}", local::display_path(p, &dir))),
    ])
}

const DELETE: &str = "branches-delete";
const PRUNE: &str = "branches-prune";

/// A question: `lines` before the buttons.
fn ask(id: String, title: String, lines: Vec<String>, yes: &str, no: &str) -> Effect {
    let mut m = Menu::confirm(id, title, yes, no);
    for (i, l) in lines.into_iter().enumerate() {
        m.items.insert(i, MenuItem::note(l));
    }
    Effect::Menu(m)
}

/// What deleting a branch loses, one line each.
pub(crate) fn lost_branch(d: &Detail, b: &Branch) -> Vec<String> {
    let mut out = Vec::new();
    if b.unpushed > 0 {
        let s = if b.unpushed == 1 { "is" } else { "are" };
        let n = b.unpushed;
        out.push(format!(
            "{n} commit{} {s} on no remote: {} lost",
            if n == 1 { "" } else { "s" },
            if n == 1 { "it is" } else { "they are" }
        ));
    } else if b.merged && !d.base.is_empty() {
        out.push(format!("it is merged into {}: nothing is lost", d.base));
    } else {
        out.push("everything on it is on a remote: nothing is lost".to_string());
    }
    if b.gone {
        out.push(format!(
            "its upstream {} is already gone from the remote",
            b.upstream
        ));
    } else if !b.upstream.is_empty() {
        out.push(format!("{} on the remote stays", b.upstream));
    }
    out
}

/// What removing a worktree loses, one line each.
pub(crate) fn lost_tree(t: &Tree) -> Vec<String> {
    let mut out = Vec::new();
    if t.status != StatusRead::Current {
        out.push("uncommitted changes are unknown: read this worktree again first".into());
    } else if t.dirty > 0 {
        out.push(format!("{} uncommitted in it: lost", files_words(t.dirty)));
    } else {
        out.push("nothing uncommitted in it".to_string());
    }
    out.push(format!("the folder {} is removed", local::tilde(&t.dir)));
    if !t.branch.is_empty() {
        out.push(format!("its branch {} stays", t.branch));
    }
    out
}

/// `d`: ask before deleting the selected branch or worktree, or say why not.
fn delete(p: &mut GitPlugin) -> Vec<Effect> {
    p.branches.removal.pending = None;
    let say = |s: String| vec![Effect::Notify(s)];
    match selected(p) {
        None => say("nothing selected to delete".into()),
        Some(Sel::Clone(_)) => {
            say("a clone is not deleted here: delete a branch or a worktree under it".into())
        }
        Some(Sel::Tree(..)) => removal::ask(p),
        Some(Sel::Branch(d, b)) => {
            if let Some(w) = d.on(&b.name).first() {
                return say(format!(
                    "{} is checked out in {}: switch away from it first",
                    b.name,
                    local::tilde(&w.dir)
                ));
            }
            if b.name == d.base {
                return say(format!(
                    "{} is this repo's base branch: radar keeps it",
                    b.name
                ));
            }
            let title = format!("delete the branch {} in {}?", b.name, base_name(&d.dir));
            vec![ask(
                format!("{DELETE}:branch\t{}\t{}", d.dir, b.name),
                title,
                lost_branch(d, b),
                "delete it",
                "keep it",
            )]
        }
    }
}

/// The candidates prune would take, from what was read.
fn candidates(p: &GitPlugin) -> Vec<prune::Candidate> {
    let shown: Vec<&Detail> = clones(p)
        .iter()
        .filter_map(|c| p.branches.details.get(&c.dir))
        .collect();
    prune::select(shown)
}

fn skipped_status(p: &GitPlugin) -> Option<String> {
    let n = clones(p)
        .iter()
        .filter_map(|c| p.branches.details.get(&c.dir))
        .flat_map(|d| d.trees.iter().skip(1))
        .filter(|tree| tree.status != StatusRead::Current)
        .count();
    (n > 0).then(|| {
        format!(
            "{n} worktree{} skipped: status unavailable; read the clones again first",
            if n == 1 { "" } else { "s" }
        )
    })
}

pub fn bulk(p: &GitPlugin) -> Vec<MenuItem> {
    let mut out = Vec::new();
    out.extend(skipped_status(p).map(MenuItem::note));
    let n = candidates(p).len();
    if n > 0 {
        out.push(MenuItem::new(
            'p',
            "prune merged and gone",
            format!("{n} merged or gone, clean ones only, asked once"),
            "prune",
        ));
    }
    out.push(MenuItem::new(
        'r',
        "read every clone again",
        "now",
        "branches-read",
    ));
    out
}

pub fn on_bulk(p: &mut GitPlugin, value: &str) -> Option<Vec<Effect>> {
    match value {
        "branches-read" => Some(refresh_now(p)),
        "prune" => Some(ask_prune(p)),
        _ => None,
    }
}

fn ask_prune(p: &GitPlugin) -> Vec<Effect> {
    let list = candidates(p);
    if list.is_empty() {
        return vec![Effect::Notify(
            "nothing to prune: no merged or gone branch is free to go".into(),
        )];
    }
    let (trees, branches) = (
        list.iter().filter(|c| !c.tree.is_empty()).count(),
        list.iter().filter(|c| c.tree.is_empty()).count(),
    );
    let mut lines: Vec<String> = list.iter().take(12).map(prune::Candidate::line).collect();
    lines.extend(skipped_status(p));
    if list.len() > 12 {
        lines.push(format!("… and {} more", list.len() - 12));
    }
    let lost: usize = list.iter().map(|c| c.lost).sum();
    lines.push(if lost > 0 {
        format!("{} in all: they are lost", unpushed_words(lost))
    } else {
        "nothing on them is unpushed or uncommitted".into()
    });
    if !p.cfg.after_worktree.is_empty() && trees > 0 {
        lines.push(format!("then: {}", p.cfg.after_worktree));
    }
    let title = format!(
        "prune {trees} worktree{} and {branches} branch{}?",
        if trees == 1 { "" } else { "s" },
        if branches == 1 { "" } else { "es" }
    );
    vec![ask(PRUNE.into(), title, lines, "prune them", "leave them")]
}

fn journal_target(kind: &'static str, clone: &str, branch: &str, tree: &str) -> Effect {
    let mut effect = crate::act::journal_at(kind, clone, "", branch);
    if !tree.is_empty()
        && let Effect::Journal(action) = &mut effect
    {
        let clone = local::journal_path(clone)
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let tree = local::journal_path(tree)
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        match radar_core::journal::SubjectKey::new("local", &clone, "worktree", &tree) {
            Ok(key) => action.subjects.push(radar_core::journal::SubjectSnapshot::new(
                key,
                radar_core::journal::SubjectRole::Destination,
            )),
            Err(_) => {
                action.metadata.redacted = true;
                action.metadata.omitted_subjects += 1;
            }
        }
    }
    effect
}

fn journal_prune_step(result: &prune::StepResult) -> Effect {
    let (kind, branch, tree) = match &result.step {
        prune::Step::Worktree { tree, branch } => {
            ("git.prune.worktree.delete", branch.as_str(), tree.as_str())
        }
        prune::Step::Branch { branch } => ("git.prune.branch.delete", branch.as_str(), ""),
        prune::Step::After => ("git.prune.after", "", ""),
    };
    let mut effect = journal_target(kind, &result.clone, branch, tree);
    if let Effect::Journal(action) = &mut effect {
        action.metadata.reason = match result.status {
            prune::StepStatus::InvalidInput => Some(radar_core::journal::Reason::InvalidInput),
            prune::StepStatus::Skipped => Some(radar_core::journal::Reason::Interrupted),
            _ => None,
        };
    }
    effect
}

/// The answer to a question of this screen's, or `None` when the menu is
/// not its own.
pub fn on_menu(p: &mut GitPlugin, menu: &str, value: &str) -> Option<Vec<Effect>> {
    if menu == PRUNE {
        if value != "yes" {
            return Some(vec![Effect::Report(JournalOutcome::Cancelled)]);
        }
        let list = candidates(p);
        let mut dirs: Vec<String> = list.iter().map(|c| c.clone.clone()).collect();
        dirs.dedup();
        let after = p.cfg.after_worktree.clone();
        p.branches.changing.extend(dirs.iter().cloned());
        // Keep every planned target without overflowing a descriptor's subject
        // bound. Plan success means the target was enumerated, not deleted yet.
        let mut effects = Vec::new();
        for target in &list {
            effects.push(journal_target(
                "git.prune.plan",
                &target.clone,
                &target.branch,
                &target.tree,
            ));
            effects.push(Effect::Report(JournalOutcome::Succeeded));
        }
        effects.extend(vec![
            Effect::Journal(ActionDescriptor::new(
                "git.prune",
                Origin::Plugin("git".into()),
            )),
            Effect::Report(JournalOutcome::Confirmed),
            Effect::Report(JournalOutcome::Started),
            Effect::Spawn(Box::pin(async move {
                let result = tokio::task::spawn_blocking(move || prune::run(&list, &after)).await;
                // A task panic cannot establish any step's outcome, but the
                // known affected clones must still be refreshed and unblocked.
                let msg = match result {
                    Ok(result) => BranchMsg::Pruned {
                        result,
                        reread: dirs,
                    },
                    Err(e) => BranchMsg::Done {
                        said: Err(e.to_string()),
                        reread: dirs,
                    },
                };
                Box::new(msg) as Msg
            })),
        ]);
        return Some(effects);
    }
    let rest = menu.strip_prefix(DELETE)?.strip_prefix(':')?;
    if rest.split('\t').next() == Some("tree") {
        return Some(removal::answer(p, menu, value));
    }
    if value != "yes" {
        return Some(vec![Effect::Report(JournalOutcome::Cancelled)]);
    }
    let mut parts = rest.splitn(3, '\t');
    let (kind, clone, what) = (
        parts.next()?,
        parts.next()?.to_string(),
        parts.next()?.to_string(),
    );
    if kind != "branch" {
        return None;
    }
    p.branches.changing.insert(clone.clone());
    let event = journal_target("git.branch.delete", &clone, &what, "");
    Some(vec![
        event,
        Effect::Report(JournalOutcome::Confirmed),
        Effect::Report(JournalOutcome::Started),
        blocking(move || {
            let path = Path::new(&clone);
            let said = prune::delete_branch(path, &what);
            BranchMsg::Done {
                said,
                reread: vec![clone],
            }
        }),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::branches;
    use radar_core::view::Chord;
    use radar_core::Plugin;
    use radar_core::host::{Event, Host, View as HostView};

    /// A context press selects the shown row under it for its menu; the
    /// pane's blank background selects nothing and opens no menu. Each shown
    /// row's target is in the text form's braces.
    #[test]
    fn tree_context_selection_requires_a_drawn_row() {
        use radar_core::view::{Intent, Target};
        use radar_core::{Outcome, Pane, Plugin};

        let mut p = branches();
        let mut host = hosted();
        host.plugins[0] = Box::new(branches());
        // The text form joins a path with `/`; the tree's own rows give the real targets.
        let _ = crate::tests::shown(&host, 160, 40);
        let view = host.screen_view(0, SCREEN.id).expect("the branches screen");
        let Node::Tree(tree) = &view.pane(PaneId::Middle).expect("the tree pane").body else {
            panic!("the clones tree")
        };
        let (clone, child) = (&tree.rows[0], &tree.rows[0].children[0]);
        let rows: Vec<String> = [vec![clone.id.clone()], vec![clone.id.clone(), child.id.clone()]]
            .iter()
            .map(|path| TreeRow::target_of(path).as_str().to_string())
            .collect();
        let target = rows[1].clone();
        for _ in 0..2 {
            let context = Intent::Context { target: Target::new(&target) };
            let out = p.intent(SCREEN.id, Pane::Middle, &context);
            assert!(matches!(out, Outcome::Handled(_)));
            assert_eq!(p.branches.tree.borrow().selected(), TreeRow::path_of(&Target::new(&target)));
        }
        // A press on the pane's blank space is the host's own `pane:middle`
        // target: the pane is focused and the part hears nothing of it.
        let clone_row = rows
            .iter()
            .find(|t| TreeRow::path_of(&Target::new(t.as_str())).len() == 1)
            .expect("a clone row");
        host.focus = Pane::Nav;
        host.update(Event::Intent(Intent::Select { target: Target::new(clone_row) }));
        let kept = host.plugins[0].subject(SCREEN.id, Pane::Middle);
        assert!(kept.is_some(), "the shown row is selected");
        host.update(Event::Intent(Intent::Select { target: Target::new("pane:middle") }));
        assert_eq!(host.focus, Pane::Middle);
        assert!(host.menu.is_none(), "a blank press opens no menu");
        assert_eq!(
            host.plugins[0].subject(SCREEN.id, Pane::Middle),
            kept,
            "a blank press keeps the selection"
        );
    }

    fn hosted() -> Host {
        let mut host = Host::new(vec![Box::new(branches())], radar_core::plugin::DEMO_NOW).unwrap();
        host.load_config(Some("")).unwrap();
        host.plugins[0] = Box::new(branches());
        host.view = HostView::Screen(0);
        host
    }

    /// Clones and branches with real-length names, and a column of other
    
    #[test]
    fn long_names_stay_whole_in_the_view() {
        let mut p = branches();
        let long = "/work/acme/banking-foreign-exchange-settlement-service";
        let mut c = p.local.index.as_ref().unwrap().checkouts[1].clone();
        c.dir = long.into();
        c.main = String::new();
        c.project = "acme/team-a/banking-foreign-exchange-settlement-service".into();
        p.local.index.as_mut().unwrap().checkouts.push(c);
        let mut d = p.branches.details["/work/acme/web"].clone();
        d.dir = long.into();
        d.trees[0].dir = long.into();
        d.trees[0].branch = "alice/ABC-1234-add-corporate-finance-reporting".into();
        d.branches[0].name = d.trees[0].branch.clone();
        p.branches.details.insert(long.into(), d);
        let mut host = Host::new(vec![Box::new(branches())], radar_core::plugin::DEMO_NOW).unwrap();
        host.load_config(Some("")).unwrap();
        host.plugins[0] = Box::new(p);
        host.view = HostView::Screen(0);
        let text = crate::tests::shown(&host, 160, 40);
        // The view holds the names whole; where they are cut is the renderer's.
        assert!(
            text.contains("banking-foreign-exchange-settlement-service")
                && text.contains("alice/ABC-1234-add-corporate-finance-reporting"),
            "{text}"
        );
        insta::assert_snapshot!(text);
    }

    fn press(p: &mut GitPlugin, pane: Pane, code: KeyCode) -> Vec<Effect> {
        match key(p, pane, Chord::new(code)) {
            Outcome::Handled(fx) => fx,
            Outcome::Ignored => vec![],
        }
    }

    /// Select a row of the tree: the clone, and a worktree (`w:`) or branch (`b:`) under it.
    fn select(p: &GitPlugin, dir: &str, child: Option<&str>) {
        let mut path = vec![clone_id(dir)];
        path.extend(child.map(str::to_string));
        p.branches.tree.borrow_mut().select(path);
    }

    fn menu(fx: &[Effect]) -> &Menu {
        match fx {
            [Effect::Menu(m)] => m,
            other => panic!("expected one question: {other:?}"),
        }
    }

    fn said(m: &Menu) -> Vec<&str> {
        m.items
            .iter()
            .filter(|i| i.note)
            .map(|i| i.label.as_str())
            .collect()
    }

    #[test]
    fn the_screen() {
        let host = hosted();
        // The sync words are only on the top bar, which the text form leaves out.
        assert_eq!(
            crate::tests::top_bar_words(&host).as_deref(),
            Some("synced 20s ago · every 1m")
        );
        insta::assert_snapshot!(crate::tests::shown(&host, 160, 40));
    }

    #[test]
    fn the_dirty_view() {
        let mut host = hosted();
        let _ = crate::tests::shown(&host, 160, 40);
        host.focus = Pane::Nav;
        host.update(Event::Intent(Intent::Key { chord: Chord::new(KeyCode::Down) }));
        insta::assert_snapshot!(crate::tests::shown(&host, 160, 40));
    }

    #[test]
    fn contextual_empty_branch_views_describe_only_what_is_shown() {
        let mut p = branches();
        for detail in p.branches.details.values_mut() {
            for tree in &mut detail.trees {
                tree.dirty = 0;
                tree.ahead = 0;
                tree.behind = 3;
            }
            for branch in &mut detail.branches {
                branch.ahead = 0;
                branch.behind = 3;
            }
            detail.stashes.clear();
        }
        // A behind-only branch is absent from Ahead, but is not level with its remote.
        assert!(p.branches.details.values().all(|detail| {
            !visible(View::Ahead, Some(detail)) && detail.branches.iter().any(|b| b.behind > 0)
        }));
        let mut host = hosted();
        host.plugins[0] = Box::new(p);
        host.focus = Pane::Nav;
        for expected in [
            "No uncommitted changes shown in this view.",
            "No branches with unpushed commits shown.",
            "No stashes shown in this view.",
        ] {
            host.update(Event::Intent(Intent::Key { chord: Chord::new(KeyCode::Down) }));
            let text = crate::tests::shown(&host, 160, 40);
            assert!(text.contains(expected), "{text}");
        }
    }

    #[test]
    fn contextual_empty_dirty_view_does_not_claim_unread_or_failed_clones_are_clean() {
        let mut p = branches();
        p.branches.view = 1;
        p.branches.details.remove("/work/acme/api");
        p.branches.loading.insert("/work/acme/api".into());
        let web = p.branches.details.get_mut("/work/acme/web").unwrap();
        web.error = "could not read working tree".into();
        web.trees.clear();
        let mut host = hosted();
        host.plugins[0] = Box::new(p);
        let text = crate::tests::shown(&host, 160, 40);
        assert!(
            text.contains("No uncommitted changes shown in this view."),
            "{text}"
        );
        assert!(!text.contains("Nothing uncommitted in any clone or worktree."));
    }

    #[test]
    fn the_rail_counts_what_is_to_do_and_filters_the_tree() {
        let p = branches();
        assert_eq!(
            counts(&p),
            [0, 1, 2, 1],
            "one dirty checkout, two branches ahead, one stash"
        );
        let (api, web) = (
            p.branches.details.get("/work/acme/api"),
            p.branches.details.get("/work/acme/web"),
        );
        assert!(visible(View::All, web));
        assert!(!visible(View::Dirty, web) && visible(View::Dirty, api));
        assert!(!visible(View::Ahead, web) && visible(View::Ahead, api));
        assert!(!visible(View::Stashes, web) && visible(View::Stashes, api));
        assert!(
            !visible(View::Dirty, None),
            "a clone not read yet is in none of the filtered views"
        );
        let status: Vec<String> = status(&p).into_iter().map(|(w, _)| w).collect();
        assert_eq!(status, ["1 dirty", "2 ahead"]);
        // Moving down the nav pane changes the view; ↑ from the first is the host's.
        let mut p = branches();
        assert!(matches!(
            key(
                &mut p,
                Pane::Nav,
                Chord::new(KeyCode::Up)
            ),
            Outcome::Ignored
        ));
        press(&mut p, Pane::Nav, KeyCode::Down);
        assert_eq!(p.branches.view(), View::Dirty);
        let _ = click(&mut p, "view:3");
        assert_eq!(p.branches.view(), View::Stashes);
    }

    #[test]
    fn deleting_a_branch_says_what_is_lost_and_refuses_the_ones_in_use() {
        let mut p = branches();
        let api = "/work/acme/api";
        // Not on a remote: the commit is named as lost.
        select(&p, api, Some("b:spike"));
        let fx = press(&mut p, Pane::Middle, KeyCode::Char('d'));
        let m = menu(&fx);
        assert_eq!(m.title, "delete the branch spike in api?");
        assert!(m.question);
        assert_eq!(said(m), ["1 commit is on no remote: it is lost"]);
        assert_eq!(m.id, "branches-delete:branch\t/work/acme/api\tspike");
        // Its upstream is gone: say so.
        select(&p, api, Some("b:alice/ABC-7"));
        let fx = press(&mut p, Pane::Middle, KeyCode::Char('d'));
        assert_eq!(
            said(menu(&fx)),
            [
                "everything on it is on a remote: nothing is lost",
                "its upstream origin/alice/ABC-7 is already gone from the remote"
            ]
        );
        // Merged.
        select(&p, api, Some("b:alice/ABC-9"));
        let fx = press(&mut p, Pane::Middle, KeyCode::Char('d'));
        assert_eq!(
            said(menu(&fx)),
            [
                "it is merged into main: nothing is lost",
                "origin/alice/ABC-9 on the remote stays"
            ]
        );
        // Checked out in the clone, checked out in a worktree, the base.
        for (name, why) in [
            (
                "alice/ABC-12",
                "alice/ABC-12 is checked out in /work/acme/api: switch away from it first",
            ),
            (
                "alice/ABC-14",
                "alice/ABC-14 is checked out in /work/acme/api-hotfix: switch away from it first",
            ),
            ("main", "main is this repo's base branch: radar keeps it"),
        ] {
            select(&p, api, Some(&format!("b:{name}")));
            let fx = press(&mut p, Pane::Middle, KeyCode::Char('d'));
            assert!(
                matches!(&fx[..], [Effect::Notify(m)] if m == why),
                "{name}: {fx:?}"
            );
        }
        // A clone is not deleted here.
        select(&p, api, None);
        let fx = press(&mut p, Pane::Middle, KeyCode::Char('d'));
        assert!(
            matches!(&fx[..], [Effect::Notify(m)] if m.starts_with("a clone is not deleted here")),
            "{fx:?}"
        );
    }

    #[test]
    fn removing_a_worktree_counts_the_uncommitted_files() {
        let mut p = branches();
        select(&p, "/work/acme/api", Some("w:/work/acme/api-hotfix"));
        let fx = press(&mut p, Pane::Middle, KeyCode::Char('d'));
        let m = menu(&fx);
        assert_eq!(m.title, "remove the worktree api-hotfix?");
        assert_eq!(
            said(m),
            [
                "nothing uncommitted in it",
                "the folder /work/acme/api-hotfix is removed",
                "its branch alice/ABC-14 stays"
            ]
        );
        p.branches.details.get_mut("/work/acme/api").unwrap().trees[1].dirty = 2;
        let fx = press(&mut p, Pane::Middle, KeyCode::Char('d'));
        assert_eq!(said(menu(&fx))[0], "2 files uncommitted in it: lost");
    }

    #[test]
    fn a_no_changes_nothing_and_a_yes_runs_off_the_event_loop() {
        let mut p = branches();
        let id = "branches-delete:branch\t/work/acme/api\tspike";
        assert!(matches!(
            &on_menu(&mut p, id, "no").unwrap()[..],
            [Effect::Report(JournalOutcome::Cancelled)]
        ));
        assert!(p.branches.changing.is_empty());
        let fx = on_menu(&mut p, id, "yes").unwrap();
        assert!(
            matches!(
                &fx[..],
                [
                    Effect::Journal(_),
                    Effect::Report(JournalOutcome::Confirmed),
                    Effect::Report(JournalOutcome::Started),
                    Effect::Spawn(_)
                ]
            ),
            "{fx:?}"
        );
        let Effect::Journal(action) = &fx[0] else {
            panic!("delete descriptor")
        };
        assert_eq!(
            action
                .subjects
                .iter()
                .find(|s| s.key.kind == "branch")
                .unwrap()
                .key
                .id,
            "spike"
        );
        assert!(busy(&p) && p.branches.changing.contains("/work/acme/api"));
        // Another part's menu is not this screen's.
        assert!(on_menu(&mut p, "git-act:x", "yes").is_none());
        // When it ends: say so, and read the clone again.
        let fx = on_msg(
            &mut p,
            BranchMsg::Done {
                said: Ok("deleted branch spike".into()),
                reread: vec!["/work/acme/api".into()],
            },
        );
        assert!(matches!(&fx[0], Effect::Report(JournalOutcome::Succeeded)));
        assert!(matches!(&fx[1], Effect::Notify(m) if m == "deleted branch spike"));
        assert!(matches!(&fx[2], Effect::SpawnAction { .. }));
        assert!(p.branches.changing.is_empty() && p.branches.loading.contains("/work/acme/api"));
        let fx = on_msg(
            &mut p,
            BranchMsg::Done {
                said: Err("branch is checked out".into()),
                reread: vec![],
            },
        );
        assert!(
            matches!(&fx[..], [Effect::Report(JournalOutcome::Unknown), Effect::Notice(m)] if m.tone == radar_core::view::chrome::NoticeTone::Error && m.ends_with("branch is checked out") && m.contains('✗')),
            "{fx:?}"
        );
    }

    #[test]
    fn enter_on_a_worktree_makes_it_the_one_in_use_and_on_the_clone_gives_it_back() {
        let mut p = branches();
        let project = "acme/team-a/api";
        select(&p, "/work/acme/api", Some("w:/work/acme/api-hotfix"));
        let fx = press(&mut p, Pane::Middle, KeyCode::Enter);
        assert!(fx.iter().any(|e| matches!(e, Effect::PersistCache)));
        assert_eq!(
            p.local.use_worktree.get(project).map(String::as_str),
            Some("/work/acme/api-hotfix")
        );
        assert_eq!(
            local::chosen(&p, project).unwrap().dir,
            "/work/acme/api-hotfix"
        );
        select(&p, "/work/acme/api", None);
        press(&mut p, Pane::Middle, KeyCode::Enter);
        assert!(p.local.use_worktree.is_empty());
        assert_eq!(local::chosen(&p, project).unwrap().dir, "/work/acme/api");
        // On a branch ⏎ is the host's.
        select(&p, "/work/acme/api", Some("b:spike"));
        assert!(matches!(
            key(
                &mut p,
                Pane::Middle,
                Chord::new(KeyCode::Enter)
            ),
            Outcome::Ignored
        ));
    }

    #[test]
    fn contextual_use_it_hint_matches_the_focused_panes_enter_action() {
        for child in [None, Some("w:/work/acme/api-hotfix")] {
            for pane in [Pane::Nav, Pane::Middle, Pane::Detail] {
                let mut p = branches();
                select(&p, "/work/acme/api", child);
                let can_use = pane == Pane::Middle;
                assert_eq!(
                    row_hints(&p, pane).iter().any(|hint| hint.keys == "⏎"),
                    can_use,
                    "hint for {child:?} in {pane:?}"
                );
                assert_eq!(
                    keys(&p, pane).iter().any(|binding| binding.keys == "⏎"),
                    can_use,
                    "menu for {child:?} in {pane:?}"
                );
                let before = p.local.use_worktree.clone();
                let outcome = key(
                    &mut p,
                    pane,
                    Chord::new(KeyCode::Enter),
                );
                if can_use {
                    assert!(matches!(outcome, Outcome::Handled(ref fx)
                        if fx.iter().any(|effect| matches!(effect, Effect::PersistCache))));
                } else {
                    assert!(matches!(outcome, Outcome::Ignored));
                    assert_eq!(p.local.use_worktree, before);
                }
            }
        }
    }

    #[test]
    fn prune_lists_what_it_takes_asks_once_and_does_nothing_on_no() {
        let mut p = branches();
        p.cfg.after_worktree = "sync-ide".into();
        let items = bulk(&p);
        assert_eq!(
            items.iter().map(|i| i.value.as_str()).collect::<Vec<_>>(),
            ["prune", "branches-read"]
        );
        assert_eq!(
            items[0].hint,
            "2 merged or gone, clean ones only, asked once"
        );
        let fx = on_bulk(&mut p, "prune").unwrap();
        let m = menu(&fx);
        assert_eq!(m.title, "prune 0 worktrees and 2 branches?");
        assert_eq!(
            said(m),
            [
                "branch alice/ABC-9 · merged",
                "branch alice/ABC-7 · upstream gone",
                "nothing on them is unpushed or uncommitted",
            ]
        );
        assert!(matches!(
            &on_menu(&mut p, PRUNE, "no").unwrap()[..],
            [Effect::Report(JournalOutcome::Cancelled)]
        ));
        assert!(p.branches.changing.is_empty());
        let fx = on_menu(&mut p, PRUNE, "yes").unwrap();
        assert!(matches!(
            &fx[fx.len() - 4..],
            [
                Effect::Journal(_),
                Effect::Report(JournalOutcome::Confirmed),
                Effect::Report(JournalOutcome::Started),
                Effect::Spawn(_)
            ]
        ));
        let planned: Vec<_> = fx
            .iter()
            .filter_map(|effect| match effect {
                Effect::Journal(action) if action.kind.as_str() == "git.prune.plan" => Some(
                    action
                        .subjects
                        .iter()
                        .find(|s| s.key.kind == "branch")
                        .unwrap()
                        .key
                        .id
                        .as_str(),
                ),
                _ => None,
            })
            .collect();
        assert_eq!(planned, ["alice/ABC-9", "alice/ABC-7"]);
        assert!(p.branches.changing.contains("/work/acme/api"));
        // With nothing to take, the key says so and the bulk menu does not offer it.
        p.branches
            .details
            .get_mut("/work/acme/api")
            .unwrap()
            .branches
            .retain(|b| !b.merged && !b.gone);
        assert_eq!(bulk(&p).len(), 1);
        let fx = on_bulk(&mut p, "prune").unwrap();
        assert!(
            matches!(&fx[..], [Effect::Notify(m)] if m.starts_with("nothing to prune")),
            "{fx:?}"
        );
    }

    fn prune_step(clone: &str, step: prune::Step, status: prune::StepStatus) -> prune::StepResult {
        prune::StepResult {
            clone: clone.into(),
            step,
            status,
        }
    }

    #[test]
    fn prune_reports_scoped_steps_without_commands_or_error_text() {
        use prune::{Step, StepStatus};
        let mut p = branches();
        p.cfg.after_worktree = "SECRET_CONFIGURED_COMMAND".into();
        p.branches.changing.insert("/work/a".into());
        let fx = on_msg(
            &mut p,
            BranchMsg::Pruned {
                result: prune::RunResult {
                    said: Ok("removed a tree · SECRET_HOOK_ERROR".into()),
                    steps: vec![
                        prune_step(
                            "/work/a",
                            Step::Worktree {
                                tree: "/work/a-tree".into(),
                                branch: "same".into(),
                            },
                            StepStatus::Succeeded,
                        ),
                        prune_step(
                            "/work/a",
                            Step::Branch {
                                branch: "same".into(),
                            },
                            StepStatus::Succeeded,
                        ),
                        prune_step(
                            "/work/b",
                            Step::Branch {
                                branch: "same".into(),
                            },
                            StepStatus::Unknown,
                        ),
                        prune_step(
                            "/work/c",
                            Step::Branch {
                                branch: "later".into(),
                            },
                            StepStatus::Skipped,
                        ),
                        prune_step("/work/a", Step::After, StepStatus::Skipped),
                        prune_step(
                            "/work/d",
                            Step::Branch {
                                branch: "-D".into(),
                            },
                            StepStatus::InvalidInput,
                        ),
                    ],
                },
                reread: vec!["/work/a".into()],
            },
        );
        assert!(matches!(
            &fx[..3],
            [
                Effect::Report(JournalOutcome::Partial),
                Effect::Notify(_),
                Effect::SpawnAction { .. }
            ]
        ));
        assert!(p.branches.changing.is_empty());
        assert!(p.branches.loading.contains("/work/a"));
        let mut actions = Vec::new();
        let mut outcomes = Vec::<Vec<JournalOutcome>>::new();
        for effect in &fx[3..] {
            match effect {
                Effect::Journal(action) => {
                    actions.push(action);
                    outcomes.push(vec![]);
                }
                Effect::Report(outcome) => outcomes.last_mut().unwrap().push(*outcome),
                _ => panic!("step records only"),
            }
        }
        assert_eq!(
            outcomes,
            [
                vec![JournalOutcome::Succeeded],
                vec![JournalOutcome::Succeeded],
                vec![JournalOutcome::Unknown],
                vec![JournalOutcome::Blocked],
                vec![JournalOutcome::Blocked],
                vec![JournalOutcome::Blocked],
            ]
        );
        assert_eq!(
            actions[3].metadata.reason,
            Some(radar_core::journal::Reason::Interrupted)
        );
        assert_eq!(
            actions[5].metadata.reason,
            Some(radar_core::journal::Reason::InvalidInput)
        );
        let branches: Vec<_> = actions
            .iter()
            .flat_map(|a| &a.subjects)
            .filter(|s| s.key.kind == "branch" && s.key.id == "same")
            .map(|s| &s.key)
            .collect();
        assert_eq!(branches[1].instance, "/work/a");
        assert_eq!(branches[2].instance, "/work/b");
        assert_ne!(branches[1], branches[2]);
        let tree = actions[0]
            .subjects
            .iter()
            .find(|s| s.key.kind == "worktree")
            .unwrap();
        assert_eq!(tree.key.instance, "/work/a");
        assert_eq!(tree.key.id, "/work/a-tree");
        assert_eq!(actions[4].kind.as_str(), "git.prune.after");
        assert_eq!(actions[4].subjects.len(), 1);
        assert_eq!(actions[4].subjects[0].key.kind, "checkout");
        let captured = format!("{actions:?}");
        assert!(!captured.contains("SECRET_"));
        assert!(!captured.contains("removed a tree"));
    }

    #[test]
    fn prune_step_subjects_reject_unsafe_identities() {
        let result = prune_step(
            "/work/api",
            prune::Step::Worktree {
                tree: "/work/token=secret".into(),
                branch: "bad\nbranch".into(),
            },
            prune::StepStatus::Unknown,
        );
        let Effect::Journal(action) = journal_prune_step(&result) else {
            panic!("journal")
        };
        assert!(action.metadata.redacted);
        assert_eq!(action.metadata.omitted_subjects, 2);
        assert_eq!(action.subjects.len(), 1);
        assert!(!format!("{action:?}").contains("secret"));
    }

    #[test]
    fn prune_aggregate_and_each_step_keep_independent_host_identities() {
        use radar_core::host::{HostEffect, JournalRecord};
        use prune::{Step, StepStatus};
        let mut host = hosted();
        host.enable_journal();
        for key in ['b', 'p'] {
            host.update(Event::Intent(Intent::Key { chord: Chord::new(KeyCode::Char(key)) }));
        }
        let launched = host.update(Event::Intent(Intent::Key { chord: Chord::new(KeyCode::Char('y')) }));
        let aggregate = launched
            .iter()
            .find_map(|effect| match effect {
                HostEffect::Journal(JournalRecord::Begin { id, descriptor, .. })
                    if descriptor.kind.as_str() == "git.prune" =>
                {
                    Some(id.clone())
                }
                _ => None,
            })
            .expect("aggregate prune action");
        let (task, semantic) = launched
            .iter()
            .find_map(|effect| match effect {
                HostEffect::Tracked {
                    action,
                    semantic,
                    effect,
                } if matches!(&**effect, HostEffect::Spawn { .. }) => {
                    Some((action.clone(), semantic.clone()))
                }
                _ => None,
            })
            .expect("prune task");
        assert_eq!(semantic, Some(aggregate.clone()));
        // Do not execute the task: this fixture supplies the observed result.
        let completed = host.update(Event::Tracked {
            action: task.clone(),
            semantic,
            event: Box::new(Event::Msg {
                plugin: 0,
                msg: Box::new(BranchMsg::Pruned {
                    result: prune::RunResult {
                        said: Err("SECRET_PROCESS_ERROR".into()),
                        steps: vec![
                            prune_step(
                                "/work/acme/api",
                                Step::Branch {
                                    branch: "alice/ABC-9".into(),
                                },
                                StepStatus::Succeeded,
                            ),
                            prune_step(
                                "/work/acme/api",
                                Step::Branch {
                                    branch: "alice/ABC-7".into(),
                                },
                                StepStatus::Unknown,
                            ),
                        ],
                    },
                    reread: vec!["/work/acme/api".into()],
                }),
            }),
        });
        let records: Vec<_> = completed
            .iter()
            .filter_map(|effect| match effect {
                HostEffect::Journal(record) => Some(record),
                _ => None,
            })
            .collect();
        let terminal = |id: &radar_core::journal::ActionId| {
            records
                .iter()
                .filter_map(|record| match record {
                    JournalRecord::Outcome {
                        id: found, outcome, ..
                    } if found == id && outcome.is_terminal() => Some(*outcome),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(terminal(&aggregate), [JournalOutcome::Partial]);
        assert_eq!(terminal(&task), [JournalOutcome::Partial]);
        let steps: Vec<_> = records
            .iter()
            .filter_map(|record| match record {
                JournalRecord::Begin {
                    id,
                    descriptor,
                    parent,
                } if descriptor.kind.as_str() == "git.prune.branch.delete" => {
                    assert_eq!(*parent, Some(task.clone()));
                    Some(id.clone())
                }
                _ => None,
            })
            .collect();
        assert_eq!(steps.len(), 2);
        assert_ne!(steps[0], steps[1]);
        assert_ne!(steps[0], aggregate);
        assert_eq!(terminal(&steps[0]), [JournalOutcome::Succeeded]);
        assert_eq!(terminal(&steps[1]), [JournalOutcome::Unknown]);
        let first_step = records
            .iter()
            .position(|record| {
                matches!(record,
            JournalRecord::Begin { id, .. } if id == &steps[0])
            })
            .unwrap();
        assert!(records[..first_step].iter().any(|record| matches!(record,
            JournalRecord::Outcome { id, outcome: JournalOutcome::Partial, .. } if id == &aggregate)));
        assert!(records[..first_step].iter().any(|record| matches!(record,
            JournalRecord::Begin { descriptor, parent, .. } if descriptor.kind.as_str() == "git.branches.read" && parent == &Some(task.clone()))));
        assert!(!format!("{records:?}").contains("SECRET_PROCESS_ERROR"));
    }

    /// Exercise each read boundary without subprocesses or process-global PATH
    /// changes. Discovery probes deliberately fail, as in an unpushed local repo.
    fn read_fixture(dir: &str, failure: &str) -> (Detail, ReadObservation) {
        read_with(dir, |path, args| {
            if args[0] == failure
                || (failure == "worktree-status" && args[0] == "status" && path != Path::new(dir))
            {
                return Err("SECRET_READ_ERROR".into());
            }
            Ok(match args[0] {
                "for-each-ref" => concat!(
                    "main\t\t\t10\tabc1234\tSECRET_COMMIT_SUBJECT\n",
                    "SECRET_BRANCH_NAME\torigin/side\t[ahead 1]\t20\tdef1234\tsecond"
                )
                .into(),
                "symbolic-ref" | "rev-parse" => return Err("ordinary missing remote".into()),
                "branch" => "main".into(),
                "rev-list" if failure == "invalid-count" => "not a number".into(),
                "rev-list" => "2".into(),
                "worktree" => format!(
                    "worktree {dir}\nHEAD abc1234\nbranch refs/heads/main\n\n\
                     worktree {dir}/SECRET_WORKTREE_PATH\nHEAD def1234\nbranch refs/heads/side"
                ),
                "status" => "# branch.ab +1 -2\n? SECRET_DIRTY_FILE".into(),
                "stash" => "SECRET_STASH_MESSAGE".into(),
                _ => panic!("unexpected fixture command"),
            })
        })
    }

    fn read_descriptors(effects: &[Effect]) -> Vec<&ActionDescriptor> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Observe { action, .. } => Some(action),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn branch_read_observations_keep_counts_and_expected_base_fallbacks_private() {
        let (detail, observed) = read_fixture("/work/a", "");
        assert_eq!(detail.base, "main");
        assert_eq!(detail.branches[0].unpushed, 2);
        assert_eq!(detail.trees[0].dirty, 1);
        assert_eq!(observed.outcome(), JournalOutcome::Succeeded);
        let fx = observed.effects(&read_action("/work/a"));
        assert!(matches!(fx[0], Effect::Report(JournalOutcome::Succeeded)));
        let counts: Vec<_> = fx
            .iter()
            .filter_map(|effect| match effect {
                Effect::Observe { action, outcome } => {
                    assert_eq!(*outcome, JournalOutcome::Succeeded);
                    assert_eq!(action.subjects.len(), 1);
                    assert_eq!(action.subjects[0].key.instance, "/work/a");
                    assert_eq!(action.subjects[0].key.id, "/work/a");
                    Some((action.kind.as_str(), action.metadata.count))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            counts,
            [
                ("git.branches.read.branches", Some(2)),
                ("git.branches.read.worktrees", Some(2)),
                ("git.branches.read.stashes", Some(1)),
            ]
        );
        assert!(!format!("{:?}", read_descriptors(&fx)).contains("SECRET_"));
    }

    #[test]
    fn a_failed_initial_branch_read_does_not_claim_empty_lists_were_observed() {
        let (detail, observed) = read_fixture("/work/a", "for-each-ref");
        assert_eq!(detail.error, "SECRET_READ_ERROR");
        assert!(detail.branches.is_empty() && detail.trees.is_empty() && detail.stashes.is_empty());
        assert_eq!(observed.outcome(), JournalOutcome::Failed);
        let fx = observed.effects(&read_action("/work/a"));
        assert!(matches!(
            &fx[..],
            [Effect::Report(JournalOutcome::Failed), Effect::Observe { action, outcome: JournalOutcome::Failed }]
                if action.kind.as_str() == "git.branches.read.branches" && action.metadata.count.is_none()
        ));
        assert!(!format!("{:?}", read_descriptors(&fx)).contains("SECRET_"));
    }

    #[test]
    fn swallowed_branch_read_errors_are_partial_without_changing_display_fallbacks() {
        for failure in [
            "branch",
            "rev-list",
            "invalid-count",
            "worktree",
            "worktree-status",
            "stash",
        ] {
            let (detail, observed) = read_fixture("/work/a", failure);
            assert!(
                detail.error.is_empty(),
                "{failure}: the old display is unchanged"
            );
            assert_eq!(observed.outcome(), JournalOutcome::Partial, "{failure}");
            match failure {
                "branch" => assert!(detail.branches.iter().all(|branch| !branch.merged)),
                "rev-list" | "invalid-count" => assert_eq!(detail.branches[0].unpushed, 0),
                "worktree" => {
                    assert_eq!(detail.trees.len(), 1, "the display retains its fallback");
                    assert_eq!(observed.worktrees, ReadCount::Failed);
                }
                "worktree-status" => {
                    assert_eq!(detail.trees[0].dirty, 1);
                    assert_eq!(detail.trees[1].dirty, 0);
                    assert_eq!(
                        observed.worktrees,
                        ReadCount::Observed {
                            count: 2,
                            complete: false
                        }
                    );
                }
                "stash" => {
                    assert!(detail.stashes.is_empty());
                    assert_eq!(observed.stashes, ReadCount::Failed);
                }
                _ => unreachable!(),
            }
            let fx = observed.effects(&read_action("/work/a"));
            assert_eq!(fx.len(), 4, "one bounded observation per category");
            assert!(!format!("{:?}", read_descriptors(&fx)).contains("SECRET_"));
        }
    }

    #[test]
    fn an_empty_repository_is_a_successful_zero_not_a_read_failure() {
        let (_, observed) = read_with("/work/a", |_, args| {
            if args[0] == "symbolic-ref" {
                Err("no origin yet".into())
            } else {
                Ok(String::new())
            }
        });
        assert_eq!(observed.outcome(), JournalOutcome::Succeeded);
        assert_eq!(observed.branches, ReadCount::observed(0));
        assert_eq!(observed.worktrees, ReadCount::observed(0));
        assert_eq!(observed.stashes, ReadCount::observed(0));
    }

    #[test]
    fn duplicate_branch_reads_are_observed_without_starting_another_task() {
        let mut p = branches();
        let first = refresh(&mut p);
        assert_eq!(first.len(), 2);
        assert!(
            first
                .iter()
                .all(|fx| matches!(fx, Effect::SpawnAction { .. }))
        );
        let again = refresh(&mut p);
        assert_eq!(again.len(), 2);
        assert!(again.iter().all(|fx| matches!(fx,
            Effect::Observe { action, outcome: JournalOutcome::Blocked }
                if action.kind.as_str() == "git.branches.read.suppressed"
                    && action.metadata.count == Some(1))));
        assert_eq!(p.branches.loading.len(), 2);
        let fx = finished(&mut p, Ok("done".into()), vec!["/work/acme/api".into()]);
        assert!(matches!(
            fx[1],
            Effect::Observe {
                outcome: JournalOutcome::Blocked,
                ..
            }
        ));
        assert_eq!(p.branches.loading.len(), 2);
    }

    #[tokio::test]
    async fn interrupted_branch_reads_clear_only_the_requested_loading_entry() {
        let mut p = branches();
        p.branches
            .loading
            .extend(["/work/acme/api".into(), "/work/acme/web".into()]);
        p.branches.changing.insert("/work/acme/api".into());
        let before = p.branches.details.clone();
        let Effect::SpawnAction { task, .. } =
            read_task("/work/acme/api".into(), |_| panic!("SECRET_READ_PANIC"))
        else {
            panic!("read task")
        };
        let msg = *task.await.downcast::<BranchMsg>().unwrap();
        assert!(matches!(&msg, BranchMsg::ReadInterrupted { dir, .. } if dir == "/work/acme/api"));
        let fx = on_msg(&mut p, msg);
        assert!(matches!(
            &fx[..],
            [Effect::Report(JournalOutcome::Unknown), Effect::Notice(message)] if message.tone == radar_core::view::chrome::NoticeTone::Error
        ));
        assert!(!p.branches.loading.contains("/work/acme/api"));
        assert!(p.branches.loading.contains("/work/acme/web"));
        assert!(p.branches.changing.contains("/work/acme/api"));
        let mut expected = before;
        for tree in &mut expected.get_mut("/work/acme/api").unwrap().trees {
            tree.status = StatusRead::LastKnown;
        }
        assert_eq!(
            p.branches.details, expected,
            "only the interrupted clone's availability changes"
        );
    }

    #[tokio::test]
    async fn branch_read_summaries_keep_dispatch_identity_after_settings_change() {
        let mut p = branches();
        let Effect::SpawnAction { action, task } =
            read_task("/work/a".into(), |dir| read_fixture(dir, ""))
        else {
            panic!("read task")
        };
        p.configure(Some(&toml::toml! { roots = ["/work/changed"] }.into()))
            .unwrap();
        p.local.index = None;
        let msg = *task.await.downcast::<BranchMsg>().unwrap();
        let fx = on_msg(&mut p, msg);
        assert_eq!(action.subjects[0].key.id, "/work/a");
        for effect in fx {
            if let Effect::Observe {
                action: observed, ..
            } = effect
            {
                assert_eq!(observed.subjects, action.subjects);
                assert!(!format!("{observed:?}").contains("changed"));
            }
        }
    }

    #[test]
    fn branch_read_subjects_reject_unsafe_identity_and_counts_saturate_explicitly() {
        let action = read_action("/work/token=SECRET_PATH");
        assert!(action.subjects.is_empty());
        assert!(action.metadata.redacted);
        assert_eq!(action.metadata.omitted_subjects, 1);
        let (_, observed) = read_fixture("/work/a", "");
        assert!(!format!("{:?}", read_descriptors(&observed.effects(&action))).contains("SECRET_"));
        let mut action = read_action("/work/a");
        set_read_count(&mut action, usize::MAX);
        assert_eq!(action.metadata.count, Some(u32::MAX));
        assert_eq!(action.metadata.truncated, usize::BITS > 32);
    }

    fn tracked_read_tasks(
        effects: &[radar_core::host::HostEffect],
    ) -> Vec<(radar_core::journal::ActionId, radar_core::journal::ActionId)> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                radar_core::host::HostEffect::Tracked {
                    action,
                    semantic: Some(semantic),
                    effect,
                } if matches!(&**effect, radar_core::host::HostEffect::Spawn { .. }) => {
                    Some((action.clone(), semantic.clone()))
                }
                _ => None,
            })
            .collect()
    }

    fn terminal_outcomes(
        effects: &[radar_core::host::HostEffect],
        action: &radar_core::journal::ActionId,
    ) -> Vec<JournalOutcome> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                radar_core::host::HostEffect::Journal(radar_core::host::JournalRecord::Outcome {
                    id,
                    outcome,
                    ..
                }) if id == action && outcome.is_terminal() => Some(*outcome),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_failed_branch_reread_cannot_overwrite_a_successful_delete() {
        use radar_core::host::{HostEffect, JournalRecord};
        let mut host = hosted();
        let p = branches();
        select(&p, "/work/acme/api", Some("b:spike"));
        host.plugins[0] = Box::new(p);
        host.enable_journal();
        host.update(Event::Intent(Intent::Key { chord: Chord::new(KeyCode::Char('d')) }));
        let launched = host.update(Event::Intent(Intent::Key { chord: Chord::new(KeyCode::Char('y')) }));
        let [(write_task, write)] = &tracked_read_tasks(&launched)[..] else {
            panic!("one delete task")
        };
        // Supply observations; never run the destructive task from the fixture.
        let finished = host.update(Event::Tracked {
            action: write_task.clone(),
            semantic: Some(write.clone()),
            event: Box::new(Event::Msg {
                plugin: 0,
                msg: Box::new(BranchMsg::Done {
                    said: Ok("deleted branch spike".into()),
                    reread: vec!["/work/acme/api".into()],
                }),
            }),
        });
        assert_eq!(
            terminal_outcomes(&finished, write),
            [JournalOutcome::Succeeded]
        );
        assert_eq!(
            terminal_outcomes(&finished, write_task),
            [JournalOutcome::Succeeded]
        );
        let [(read_task, read)] = &tracked_read_tasks(&finished)[..] else {
            panic!("one independent read task")
        };
        assert_ne!(read, write);
        assert!(finished.iter().any(|effect| matches!(effect,
            HostEffect::Journal(JournalRecord::Begin { id, parent, descriptor })
                if id == read && parent == &Some(write_task.clone())
                    && descriptor.kind.as_str() == "git.branches.read")));
        let interrupted = host.update(Event::Tracked {
            action: read_task.clone(),
            semantic: Some(read.clone()),
            event: Box::new(Event::Msg {
                plugin: 0,
                msg: Box::new(BranchMsg::ReadInterrupted {
                    dir: "/work/acme/api".into(),
                    why: "SECRET_READ_PANIC".into(),
                }),
            }),
        });
        assert_eq!(
            terminal_outcomes(&interrupted, read),
            [JournalOutcome::Unknown]
        );
        assert_eq!(
            terminal_outcomes(&interrupted, read_task),
            [JournalOutcome::Unknown]
        );
        assert!(terminal_outcomes(&interrupted, write).is_empty());
        assert!(terminal_outcomes(&interrupted, write_task).is_empty());
        let records: Vec<_> = interrupted
            .iter()
            .filter_map(|effect| match effect {
                HostEffect::Journal(record) => Some(record),
                _ => None,
            })
            .collect();
        assert!(!format!("{records:?}").contains("SECRET_"));
    }

    #[test]
    fn out_of_order_branch_reads_keep_their_host_context_after_settings_change() {
        use radar_core::host::{HostEffect, JournalRecord};
        let mut host = hosted();
        host.enable_journal();
        let launched = host.update(Event::Intent(Intent::Key { chord: Chord::new(KeyCode::Char('r')) }));
        let tasks = tracked_read_tasks(&launched);
        assert_eq!(tasks.len(), 2);
        let actions: Vec<_> =
            tasks
                .iter()
                .map(|(_, read)| {
                    launched
                        .iter()
                        .find_map(|effect| match effect {
                            HostEffect::Journal(JournalRecord::Begin {
                                id, descriptor, ..
                            }) if id == read => Some(descriptor.clone()),
                            _ => None,
                        })
                        .unwrap()
                })
                .collect();
        host.plugins[0]
            .configure(Some(&toml::toml! { roots = ["/work/changed"] }.into()))
            .unwrap();
        for i in [1, 0] {
            let (task, read) = &tasks[i];
            let action = actions[i].clone();
            let dir = action.subjects[0].key.id.clone();
            let (detail, observed) = read_fixture(&dir, if i == 0 { "stash" } else { "" });
            let outcome = observed.outcome();
            let completed = host.update(Event::Tracked {
                action: task.clone(),
                semantic: Some(read.clone()),
                event: Box::new(Event::Msg {
                    plugin: 0,
                    msg: Box::new(BranchMsg::Read {
                        dir,
                        detail: Box::new(detail),
                        observed,
                        action: action.clone(),
                    }),
                }),
            });
            assert_eq!(terminal_outcomes(&completed, read), [outcome]);
            assert_eq!(terminal_outcomes(&completed, task), [outcome]);
            assert!(terminal_outcomes(&completed, &tasks[1 - i].1).is_empty());
            let counts: Vec<_> = completed
                .iter()
                .filter_map(|effect| match effect {
                    HostEffect::Journal(JournalRecord::Begin {
                        descriptor, parent, ..
                    }) if descriptor.kind.as_str().starts_with("git.branches.read.") => {
                        assert_eq!(parent, &Some(task.clone()));
                        assert_eq!(descriptor.subjects, action.subjects);
                        Some(descriptor)
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(counts.len(), 3);
            assert!(!format!("{counts:?}").contains("SECRET_"));
            assert!(!format!("{counts:?}").contains("changed"));
        }
    }

    #[test]
    fn reading_starts_when_the_screen_has_been_looked_at_and_comes_again_each_minute() {
        let mut p = GitPlugin::default();
        p.local.index = Some(local::Index {
            checkouts: vec![Checkout {
                dir: "/work/a".into(),
                ..Checkout::default()
            }],
            ..local::Index::default()
        });
        p.now = 1_000;
        assert!(tick(&mut p).is_empty(), "nobody has looked");
        p.branches.seen.set(true);
        let fx = tick(&mut p);
        assert!(matches!(&fx[..], [Effect::SpawnAction { .. }]));
        assert!(p.branches.loading.contains("/work/a"));
        p.now += 30;
        assert!(tick(&mut p).is_empty(), "not due");
        let d = Detail {
            dir: "/work/a".into(),
            ..Detail::default()
        };
        on_msg(
            &mut p,
            BranchMsg::Read {
                dir: "/work/a".into(),
                detail: Box::new(d),
                observed: ReadObservation::default(),
                action: read_action("/work/a"),
            },
        );
        assert!(p.branches.loading.is_empty() && p.branches.details.contains_key("/work/a"));
        p.now += 31;
        assert!(
            matches!(&tick(&mut p)[..], [Effect::SpawnAction { .. }]),
            "a minute on"
        );
        // A clone that left the index is dropped with what was read of it.
        p.local.index.as_mut().unwrap().checkouts.clear();
        p.now += 61;
        tick(&mut p);
        assert!(p.branches.details.is_empty());
    }

    /// An action of the branch screen invoked by its id, through the host, does what
    /// its key does: `git.branch.read_again` is `r`.
    #[test]
    fn an_action_invoked_by_its_id_runs_what_its_key_runs() {
        use radar_core::headless::Headless;
        use radar_core::view::{Chord, Intent};
        let make = || {
            let mut host = Host::new(vec![Box::new(branches())], radar_core::plugin::DEMO_NOW).unwrap();
            host.load_config(Some("")).unwrap();
            host.plugins[0] = Box::new(branches());
            host.view = HostView::Screen(0);
            let mut headless = Headless::new(host, 80, 24);
            headless.focus(radar_core::view::PaneId::Middle);
            headless
        };
        let by_key = make().send(Intent::Key { chord: Chord::parse("r").unwrap() });
        let by_id = make().send(Intent::Invoke { action: "git.branch.read_again".into() });
        assert!(!by_key.is_empty(), "the key does something on the branch screen");
        assert_eq!(format!("{by_id:?}"), format!("{by_key:?}"));
    }

    /// Every row and target of the screen is its own and acts.
    #[test]
    fn every_row_and_target_is_taken() {
        use radar_core::check::{Check, Place};
        let make = || Box::new(branches()) as Box<dyn Plugin>;
        let mut check = Check::new(&make);
        let mut places = vec![
            Place::screen(SCREEN.id, Pane::Nav),
            Place::screen(SCREEN.id, Pane::Detail),
        ];
        for row in 0..9 {
            places.push(Place::screen(SCREEN.id, Pane::Middle).down(row));
        }
        let gaps = check.places(&places);
        assert!(gaps.is_empty(), "{} gaps:\n{}", gaps.len(), gaps.join("\n"));
        assert!(check.checked > 10, "only {} checked", check.checked);
    }

    #[test]
    fn git_s_words_are_read_into_numbers() {
        assert_eq!(parse_track(""), (0, 0, false));
        assert_eq!(parse_track("[ahead 2]"), (2, 0, false));
        assert_eq!(parse_track("[ahead 2, behind 11]"), (2, 11, false));
        assert_eq!(parse_track("[behind 1]"), (0, 1, false));
        assert_eq!(parse_track("[gone]"), (0, 0, true));
        let refs = "main\torigin/main\t\t1791270000\tabc1234\tfirst\nfix/x\torigin/fix/x\t[ahead 1]\t1791271000\tdef5678\tA\tsubject\n";
        let b = parse_refs(refs);
        assert_eq!(
            (b[0].name.as_str(), b[0].upstream.as_str(), b[0].ahead),
            ("main", "origin/main", 0)
        );
        assert_eq!(
            (
                b[1].name.as_str(),
                b[1].ahead,
                b[1].updated,
                b[1].subject.as_str()
            ),
            ("fix/x", 1, 1_791_271_000, "A\tsubject")
        );
        assert_eq!(
            parse_status(
                "# branch.oid x\n# branch.head main\n# branch.ab +3 -1\n1 .M N... 1 1 1 a b c\n? new.txt\n"
            ),
            (2, 3, 1)
        );
        let trees = parse_worktrees(
            "worktree /w/api\nHEAD 1234567890\nbranch refs/heads/main\n\nworktree /w/api-x\nHEAD abcdef0123\ndetached\n\nworktree /w/gone\nHEAD 1\nbranch refs/heads/old\nprunable gitdir file points to non-existent location\n",
        );
        assert_eq!(trees.len(), 2);
        assert_eq!(
            (
                trees[0].branch.as_str(),
                trees[1].head.as_str(),
                trees[1].branch.as_str()
            ),
            ("main", "abcdef0", "")
        );
    }

    #[test]
    fn a_clone_is_read_from_a_real_repository() {
        let dir = std::env::temp_dir().join(format!("radar-screen-read-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sh = |args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args([
                    "-c",
                    "user.name=t",
                    "-c",
                    "user.email=t@example.com",
                    "-c",
                    "commit.gpgsign=false",
                    "-c",
                    "init.defaultBranch=main",
                ])
                .args(args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        sh(&["init"]);
        std::fs::write(dir.join("a"), "a").unwrap();
        sh(&["add", "."]);
        sh(&["commit", "-m", "first"]);
        sh(&["stash", "list"]);
        std::fs::write(dir.join("b"), "b").unwrap();
        sh(&["branch", "side"]);
        let (d, observed) = read_with(dir.to_str().unwrap(), git);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(observed.outcome(), JournalOutcome::Succeeded);
        assert_eq!(d.error, "");
        assert_eq!(d.base, "main");
        assert_eq!(d.trees.len(), 1);
        assert_eq!(
            (d.trees[0].branch.as_str(), d.trees[0].dirty),
            ("main", 1),
            "an untracked file counts"
        );
        let names: Vec<&str> = d.branches.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["main", "side"]);
        assert!(
            d.branches
                .iter()
                .all(|b| b.unpushed == 1 || b.name == "side" && b.unpushed == 1),
            "no remote: every commit is unpushed: {d:?}"
        );
        assert!(d.stashes.is_empty());
    }
}

#[cfg(test)]
#[path = "screen/status_availability_tests.rs"]
mod status_availability_tests;
