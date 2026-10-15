//! What git does to a checkout when asked from a repo or merge request: a new
//! branch off a fresh base (cb's checks), rebase, push, a parked worktree,
//! linking a clone, picking the worktree acted on; and starting work on a
//! ticket across repos, which arrives over the bus (`workspace.start`) and is
//! answered with `git.started`.
//!
//! Every action is asked first and says what would be lost. Reading what
//! the question needs and running the answer both happen off the event loop;
//! the questions carry their state in the menu id (`git-act:step\tdir\t…`),
//! except the start-work question, which holds several repos in [`State`].

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use radar_core::journal::{
    ActionDescriptor, Origin, Outcome as JournalOutcome, SubjectKey, SubjectRole, SubjectSnapshot,
};
use radar_core::plugin::{Effect, Menu, MenuItem, Msg};
use radar_core::ui::icon::Icon;
use radar_core::writes;
use serde_json::{Value, json};

use crate::GitPlugin;
use crate::branch::{self, Ticket};
use crate::local::{self, Checkout};
use crate::ops::{self, Plan};

/// The menus this module opens are `git-act:<step>\t<args…>`.
pub(crate) const MENU: &str = "git-act";

pub(crate) const START: &str = "workspace.start";
pub(crate) const STARTED: &str = "git.started";

/// What the user asked of a checkout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    New,
    Rebase,
    Push,
    Park,
}

/// The act side of the plugin while it runs.
#[derive(Debug, Default)]
pub struct State {
    /// The start-work question being asked.
    pub start: Option<Started>,
    /// Folders an action is running in.
    pub busy: BTreeSet<String>,
}

/// A start-work question that is open.
#[derive(Debug, Clone, PartialEq)]
pub struct Started {
    pub key: String,
    pub items: Vec<Item>,
}

/// One repo of a start: what would be done there.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub dir: String,
    pub repo: String,
    pub branch: String,
    pub base: String,
    /// Park the branch on a worktree at `worktree` instead of switching the clone.
    pub worktree: String,
    /// The stash to make first (a dirty clone switched in place), or empty.
    pub stash: String,
    /// Said by the plan: the branch is there already.
    pub exists: bool,
    /// Why this repo cannot be started, from the plan.
    pub refused: String,
}

/// How one repo of a start went.
#[derive(Debug, Clone, PartialEq)]
pub struct StartResult {
    pub dir: String,
    pub repo: String,
    pub branch: String,
    pub base: String,
    pub worktree: String,
    pub stash: String,
    pub ok: bool,
    pub why: String,
    /// Recording evidence, independent of the existing bus/UI success flag.
    pub status: StartStatus,
    pub parked: Option<WorktreeResult>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartStatus {
    Succeeded,
    Partial,
    Unknown,
    Refused,
    Skipped,
}

impl StartStatus {
    fn outcome(self) -> JournalOutcome {
        match self {
            Self::Succeeded => JournalOutcome::Succeeded,
            Self::Partial => JournalOutcome::Partial,
            Self::Unknown => JournalOutcome::Unknown,
            Self::Refused | Self::Skipped => JournalOutcome::Blocked,
        }
    }
}

/// A process error cannot prove whether the worktree or hook changed anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeStatus {
    Succeeded,
    Unknown,
    Skipped,
}

impl WorktreeStatus {
    fn outcome(self) -> JournalOutcome {
        match self {
            Self::Succeeded => JournalOutcome::Succeeded,
            Self::Unknown => JournalOutcome::Unknown,
            Self::Skipped => JournalOutcome::Blocked,
        }
    }
}

/// Safe step identities and observations, never configured commands or output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeResult {
    pub clone: String,
    pub path: String,
    pub branch: String,
    pub creation: WorktreeStatus,
    pub after: Option<WorktreeStatus>,
}

impl WorktreeResult {
    pub(crate) fn outcome(&self) -> JournalOutcome {
        self.status().outcome()
    }

    fn status(&self) -> StartStatus {
        match self.creation {
            WorktreeStatus::Succeeded if self.after == Some(WorktreeStatus::Unknown) => {
                StartStatus::Partial
            }
            WorktreeStatus::Succeeded => StartStatus::Succeeded,
            WorktreeStatus::Unknown => StartStatus::Unknown,
            WorktreeStatus::Skipped => StartStatus::Skipped,
        }
    }
}

struct ParkResult {
    said: Result<String, String>,
    observed: WorktreeResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorktreeStep {
    Create,
    After,
}

/// What a task found or did.
pub enum ActMsg {
    /// What the question needs, read from the checkout.
    Planned {
        op: Op,
        dir: String,
        project: String,
        branch: String,
        plan: Plan,
        /// A push would rewrite what origin has.
        forced: bool,
        /// Commits the branch has that origin's copy has not.
        ahead: usize,
        /// Commits origin's copy has that the branch has not (a forced push drops them).
        behind: usize,
        /// Said before the question (a stash that was made).
        said: Option<String>,
    },
    /// An action ran.
    Done {
        dir: String,
        result: Result<String, String>,
        parked: Option<WorktreeResult>,
        /// The checkout as it is now.
        reread: Option<Checkout>,
        /// Look for checkouts again (a worktree was made).
        rescan: bool,
    },
    /// Every repo of a start was read.
    StartPlanned { key: String, items: Vec<Item> },
    /// A start ran.
    Finished {
        key: String,
        results: Vec<StartResult>,
    },
}

fn blocking(f: impl FnOnce() -> ActMsg + Send + 'static) -> Effect {
    Effect::Spawn(Box::pin(async move {
        let msg = tokio::task::spawn_blocking(f)
            .await
            .unwrap_or_else(|e| ActMsg::Done {
                dir: String::new(),
                result: Err(e.to_string()),
                parked: None,
                reread: None,
                rescan: false,
            });
        Box::new(msg) as Msg
    }))
}

// --- names and words -----------------------------------------------------------

/// The first ticket key (`ABC-12`) written in `text`.
pub(crate) fn find_key(text: &str) -> Option<String> {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let starts = b[i].is_ascii_uppercase() && (i == 0 || !b[i - 1].is_ascii_alphanumeric());
        if starts {
            let mut j = i;
            while j < b.len() && (b[j].is_ascii_uppercase() || b[j].is_ascii_digit()) {
                j += 1;
            }
            if j < b.len() && b[j] == b'-' && b[i..j].iter().any(u8::is_ascii_uppercase) {
                let mut k = j + 1;
                while k < b.len() && b[k].is_ascii_digit() {
                    k += 1;
                }
                if k > j + 1 && !b.get(k).is_some_and(u8::is_ascii_alphabetic) {
                    return Some(text[i..k].to_string());
                }
            }
        }
        i += 1;
    }
    None
}

/// `YYYYMMDD` of `now` in the local time zone.
pub(crate) fn date(now: i64) -> String {
    let days = (now + radar_core::time::local_offset(now)).div_euclid(86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}{m:02}{d:02}")
}

/// The stash a dirty checkout is offered: `ABC-12-local-20261009`.
pub(crate) fn stash_name(branch: &str, now: i64) -> String {
    let key =
        find_key(branch).unwrap_or_else(|| branch.rsplit('/').next().unwrap_or(branch).to_string());
    format!("{key}-local-{}", date(now))
}

/// Why no branch can be named for `project` yet: the template wants `{me}`
/// and no name is set.
pub(crate) fn name_problem(p: &GitPlugin, project: &str) -> Option<String> {
    let link = local::link_for(p, project);
    let (template, _) = branch::pick(
        project,
        link.map(|l| l.branch.as_str()),
        &p.cfg.conventions,
        &p.cfg.branch,
    );
    (p.cfg.me.trim().is_empty() && template.contains("{me}")).then(|| {
        "the branch template uses {me} and your name is not set: set it in Settings › git › branches"
            .to_string()
    })
}

/// The branch name by the repo's convention for a ticket.
pub(crate) fn branch_for(p: &GitPlugin, project: &str, t: &Ticket) -> String {
    let link = local::link_for(p, project);
    let (template, _) = branch::pick(
        project,
        link.map(|l| l.branch.as_str()),
        &p.cfg.conventions,
        &p.cfg.branch,
    );
    branch::render(template, &p.cfg.me, t, &p.cfg.type_map)
}

/// The base branch for a repo when the config names one; empty: origin's.
pub(crate) fn base_for(p: &GitPlugin, project: &str) -> String {
    let link = local::link_for(p, project);
    let (_, conv) = branch::pick(
        project,
        link.map(|l| l.branch.as_str()),
        &p.cfg.conventions,
        &p.cfg.branch,
    );
    link.map(|l| l.base.as_str())
        .filter(|b| !b.is_empty())
        .or(conv)
        .unwrap_or_default()
        .to_string()
}

/// Where a branch parked on a worktree goes: the template beside the clone.
pub(crate) fn worktree_path(p: &GitPlugin, clone: &str, branch: &str) -> String {
    let topic = branch.rsplit('/').next().unwrap_or(branch);
    let at = p
        .cfg
        .worktree
        .replace("{clone}", clone.trim_end_matches('/'))
        .replace("{topic}", topic);
    local::expand(&at).display().to_string()
}

fn place(dir: &str) -> String {
    let mut place = local::tilde(dir);
    if place.chars().count() > 40 {
        place = Path::new(dir)
            .file_name()
            .map_or(place, |n| n.to_string_lossy().into_owned());
    }
    place
}

fn id(step: &str, args: &[&str]) -> String {
    let mut s = format!("{MENU}:{step}");
    for a in args {
        s.push('\t');
        s.push_str(a);
    }
    s
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

// --- git, read only --------------------------------------------------------------

fn git_out(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("SSH_ASKPASS_REQUIRE", "force")
        .env("SSH_ASKPASS", "false")
        .stdin(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Read what the question for `op` needs. Changes nothing.
fn read(
    op: Op,
    dir: &str,
    project: &str,
    branch: &str,
    base: &str,
    said: Option<String>,
) -> ActMsg {
    let d = Path::new(dir);
    let base = (!base.is_empty()).then_some(base);
    let mut plan = ops::plan_new_branch(d, branch, base);
    let branch = if branch.is_empty() {
        plan.current.clone()
    } else {
        branch.to_string()
    };
    if op != Op::New && !branch.is_empty() {
        plan = ops::plan_new_branch(d, &branch, base);
    }
    let (mut forced, mut ahead, mut behind) = (false, plan.unpushed, 0usize);
    if op == Op::Push && plan.exists_remote {
        let remote = format!("origin/{branch}");
        forced = git_out(d, &["merge-base", "--is-ancestor", &remote, &branch]).is_none();
        ahead = git_out(d, &["rev-list", "--count", &format!("{remote}..{branch}")])
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);
        behind = git_out(d, &["rev-list", "--count", &format!("{branch}..{remote}")])
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);
    }
    ActMsg::Planned {
        op,
        dir: dir.to_string(),
        project: project.to_string(),
        branch,
        plan,
        forced,
        ahead,
        behind,
        said,
    }
}

fn journal(kind: &'static str) -> Effect {
    Effect::Journal(ActionDescriptor::new(kind, Origin::Plugin("git".into())))
}

pub(crate) fn journal_at(kind: &'static str, dir: &str, repo: &str, branch: &str) -> Effect {
    let mut action = ActionDescriptor::new(kind, Origin::Plugin("git".into()));
    let Some(dir) = local::journal_path(dir) else {
        action.metadata.redacted = true;
        action.metadata.omitted_subjects = 1;
        return Effect::Journal(action);
    };
    let dir = dir.display().to_string();
    // A local checkout is scoped by its absolute folder. No server or account is
    // guessed from a project path alone; remote subjects can be joined separately.
    for (kind, id, role) in [
        ("checkout", dir.as_str(), SubjectRole::Primary),
        ("repo", repo, SubjectRole::Context),
        ("branch", branch, SubjectRole::Context),
    ] {
        if !dir.is_empty() && !id.is_empty() {
            match SubjectKey::new("local", &dir, kind, id) {
                Ok(key) => action = action.with_subject(SubjectSnapshot::new(key, role)),
                Err(_) => {
                    action.metadata.redacted = true;
                    action.metadata.omitted_subjects += 1;
                }
            }
        }
    }
    Effect::Journal(action)
}

pub(crate) fn journal_worktree(result: &WorktreeResult) -> Vec<Effect> {
    let mut created = journal_at("git.worktree.add", &result.clone, "", &result.branch);
    if let Effect::Journal(action) = &mut created {
        for subject in &mut action.subjects {
            if subject.key.kind == "checkout" {
                subject.role = SubjectRole::Source;
            }
        }
        let clone = local::journal_path(&result.clone)
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let path = local::journal_path(&result.path)
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        match SubjectKey::new("local", &clone, "worktree", &path) {
            Ok(key) => action
                .subjects
                .push(SubjectSnapshot::new(key, SubjectRole::Destination)),
            Err(_) => {
                action.metadata.redacted = true;
                action.metadata.omitted_subjects += 1;
            }
        }
    }
    let mut effects = vec![created, Effect::Report(result.creation.outcome())];
    if let Some(status) = result.after {
        let mut hook = journal_at("git.worktree.after", &result.path, "", "");
        if status == WorktreeStatus::Skipped
            && let Effect::Journal(action) = &mut hook
        {
            action.metadata.reason = Some(radar_core::journal::Reason::Interrupted);
        }
        effects.extend([hook, Effect::Report(status.outcome())]);
    }
    effects
}

fn operation(op: Op) -> &'static str {
    match op {
        Op::New => "git.branch.create",
        Op::Rebase => "git.branch.rebase",
        Op::Push => "git.branch.push",
        Op::Park => "git.worktree.create",
    }
}

// --- starting an action ----------------------------------------------------------

/// `n`, `u`, `U` or park on `c`: read the checkout, then ask.
pub fn begin(
    p: &mut GitPlugin,
    op: Op,
    project: &str,
    c: &Checkout,
    branch: String,
) -> Vec<Effect> {
    if !p.act.busy.insert(c.dir.clone()) {
        return vec![
            journal_at(operation(op), &c.dir, project, &branch),
            Effect::Report(JournalOutcome::Blocked),
            Effect::Notify("already working in that checkout".into()),
        ];
    }
    let event = journal_at(operation(op), &c.dir, project, &branch);
    let (dir, project, base) = (c.dir.clone(), project.to_string(), base_for(p, project));
    vec![
        event,
        blocking(move || read(op, &dir, &project, &branch, &base, None)),
    ]
}

/// The question once the checkout is read.
#[allow(clippy::too_many_arguments)]
pub fn on_planned(
    p: &mut GitPlugin,
    op: Op,
    dir: String,
    project: String,
    branch: String,
    plan: Plan,
    forced: bool,
    ahead: usize,
    behind: usize,
    said: Option<String>,
) -> Vec<Effect> {
    p.act.busy.remove(&dir);
    let mut fx: Vec<Effect> = said
        .into_iter()
        .map(|s| Effect::Notify(writes::done_line(&s)))
        .collect();
    let q = match op {
        Op::New => new_question(p, &dir, &project, &branch, &plan),
        Op::Park => park_question(p, &dir, &project, &branch, &plan),
        Op::Rebase => rebase_question(&dir, &plan),
        Op::Push => push_question(&dir, &branch, &plan, forced, ahead, behind),
    };
    match q {
        Ok(menu) => fx.push(Effect::Menu(menu)),
        Err(why) => {
            fx.push(Effect::Report(JournalOutcome::Blocked));
            fx.push(Effect::Notify(writes::failed_line(&why)));
        }
    }
    fx
}

fn new_question(
    p: &GitPlugin,
    dir: &str,
    project: &str,
    branch: &str,
    plan: &Plan,
) -> Result<Menu, String> {
    if let Err(why) = branch::valid_ref(branch) {
        return Err(format!(
            "\"{branch}\" is not a branch name git takes: {why}"
        ));
    }
    let args = [dir, branch, plan.base.as_str(), project];
    let title = |what: &str| format!("{what} in {}?", place(dir));
    if plan.dirty > 0 {
        let name = stash_name(branch, p.now);
        let notes = vec![format!(
            "{} uncommitted changes here: git will not switch with them",
            plural(plan.dirty, "file has", "files have")
        )];
        return Ok(ask3(
            &id("dirty", &args),
            title(&format!("make {branch}")),
            notes,
            [
                ('s', "stash", format!("stash it as {name}"), "stash"),
                ('w', "worktree", "park on a worktree instead".into(), "park"),
                ('n', "no", "leave it".into(), "no"),
            ],
        ));
    }
    if plan.exists_local {
        let mut notes = vec![format!(
            "{branch} is here already; it is on {}",
            plan.current
        )];
        if plan.unpushed > 0 {
            notes.push(format!(
                "{} on {branch} {} on no remote: recreating it loses {}",
                plural(plan.unpushed, "commit", "commits"),
                if plan.unpushed == 1 { "is" } else { "are" },
                if plan.unpushed == 1 { "it" } else { "them" },
            ));
        }
        return Ok(ask3(
            &id("exists", &args),
            title(&format!("{branch} exists")),
            notes,
            [
                ('s', "switch", "switch to it".into(), "switch"),
                (
                    'r',
                    "recreate",
                    format!("recreate it off fresh {}", plan.base),
                    "recreate",
                ),
                ('n', "no", "leave it".into(), "no"),
            ],
        ));
    }
    let (what, saying) = if plan.exists_remote {
        (
            format!("check out {branch}"),
            format!("origin has {branch}: it is checked out tracking it"),
        )
    } else {
        (
            format!("make {branch}"),
            format!(
                "fetches {} and starts {branch} from origin/{}",
                plan.base, plan.base
            ),
        )
    };
    Ok(
        Menu::confirm(id("new", &args), title(&what), "do it", "leave it")
            .saying(saying)
            .icon(Icon::Branch),
    )
}

fn park_question(
    p: &GitPlugin,
    dir: &str,
    _project: &str,
    branch: &str,
    plan: &Plan,
) -> Result<Menu, String> {
    if let Err(why) = branch::valid_ref(branch) {
        return Err(format!(
            "\"{branch}\" is not a branch name git takes: {why}"
        ));
    }
    let clone = clone_of(p, dir);
    let path = worktree_path(p, &clone, branch);
    if Path::new(&path).exists() {
        return Err(format!("{} is there already", local::tilde(&path)));
    }
    let mut m = Menu::confirm(
        id("park", &[&clone, branch, &plan.base, &path]),
        format!("park {branch} on a worktree?"),
        "park it",
        "leave it",
    )
    .saying(format!("at {}", local::tilde(&path)))
    .saying("beside the clone")
    .saying(format!("{branch} comes off origin/{} when new", plan.base))
    .icon(Icon::Branch);
    if !p.cfg.after_worktree.is_empty() {
        m = m.saying(format!("then runs {}", p.cfg.after_worktree));
    }
    Ok(m)
}

fn rebase_question(dir: &str, plan: &Plan) -> Result<Menu, String> {
    if plan.current.is_empty() {
        return Err("no branch is checked out: nothing to rebase".into());
    }
    if plan.current == plan.base {
        return Err(format!("{} is the base: nothing to rebase", plan.base));
    }
    if plan.dirty > 0 {
        return Err(format!(
            "{} uncommitted changes: stash or commit them first",
            plural(plan.dirty, "file has", "files have")
        ));
    }
    Ok(Menu::confirm(
        id("rebase", &[dir, &plan.current, &plan.base]),
        format!("rebase {} on origin/{}?", plan.current, plan.base),
        "rebase it",
        "leave it",
    )
    .saying(format!("fetches {0}, brings it up to date, then rebases; a conflict stops it and leaves git's state (git rebase --abort undoes it)", plan.base))
    .icon(Icon::Branch))
}

fn push_question(
    dir: &str,
    branch: &str,
    plan: &Plan,
    forced: bool,
    ahead: usize,
    behind: usize,
) -> Result<Menu, String> {
    if branch.is_empty() {
        return Err("no branch is checked out: nothing to push".into());
    }
    let (kind, mut notes) = if !plan.exists_remote {
        (
            "new",
            vec![format!(
                "origin does not have {branch}: it is created there"
            )],
        )
    } else if forced {
        (
            "force",
            vec![
                if behind > 0 {
                    format!(
                        "{} on origin/{branch} are not here: pushing rewrites origin and drops them",
                        plural(behind, "commit", "commits")
                    )
                } else {
                    format!("origin/{branch} is not behind {branch}: this rewrites it")
                },
                "pushed with --force-with-lease:".into(),
                "it stops if someone else pushed since you fetched".into(),
            ],
        )
    } else {
        ("update", vec![])
    };
    if ahead > 0 {
        notes.push(format!("{} to send", plural(ahead, "commit", "commits")));
    }
    let m = Menu::write_plain(
        id("push", &[dir, branch, kind]),
        format!("push {branch} to origin?"),
        notes,
        "push it",
        "leave it",
    )
    .icon(Icon::Branch);
    Ok(if kind == "new" { m.adds() } else { m.edits() })
}

/// A question with three buttons, as a dialog.
/// The menu that asks where to open a merge request's branch:
/// `git-open:<what>\t<clone>\t<branch>`.
pub(crate) const OPEN_MENU: &str = "git-open";

/// A terminal or editor for a merge request: on its branch. A checkout on
/// it already opens; else ask whether to check it out here, open it in a
/// worktree beside the clone (the clone stays as it is), or open the clone
/// on its own branch.
pub(crate) fn open_on(p: &mut GitPlugin, what: &str, project: &str, c: &Checkout, branch: &str) -> Vec<Effect> {
    if let Some(on) = local::checkouts_for(p, project).into_iter().find(|k| k.branch == branch).cloned() {
        return open_in(p, what, &on);
    }
    let clone = clone_of(p, &c.dir);
    let at = worktree_path(p, &clone, branch);
    let notes = vec![
        format!("{} is on {}", local::tilde(&c.dir), c.branch),
        format!("a worktree would be at {}", local::tilde(&at)),
    ];
    vec![Effect::Menu(ask3(
        &format!("{OPEN_MENU}:{what}\t{}\t{branch}", c.dir),
        format!("open {branch}: where?"),
        notes,
        [
            ('w', "worktree", "in a worktree beside it".into(), "worktree"),
            ('c', "here", format!("check it out in {}", short(&c.dir)), "here"),
            ('o', "as is", format!("on {} as it is", c.branch), "as"),
        ],
    ))]
}

/// A folder's last name, for a button.
fn short(dir: &str) -> String {
    Path::new(dir).file_name().map_or_else(|| dir.to_string(), |n| n.to_string_lossy().into_owned())
}

/// Open `what` (`terminal` or `editor`) in `c`.
pub(crate) fn open_in(p: &GitPlugin, what: &str, c: &Checkout) -> Vec<Effect> {
    match what {
        "editor" => local::open_editor(p, c),
        _ => local::open_terminal(p, c),
    }
}

/// The answer to [`open_on`]'s question.
pub(crate) fn on_open(p: &mut GitPlugin, arg: &str, value: &str) -> Vec<Effect> {
    let mut parts = arg.split('\t');
    let (Some(what), Some(dir), Some(branch)) = (parts.next(), parts.next(), parts.next()) else {
        return vec![];
    };
    let (what, dir, branch) = (what.to_string(), dir.to_string(), branch.to_string());
    match value {
        "as" => match local::read_checkout(Path::new(&dir)) {
            Some(c) => open_in(p, &what, &c),
            None => vec![],
        },
        "here" => {
            p.local.then_open.insert(dir.clone(), what);
            local::switch_branch(p, &dir, &branch)
        }
        "worktree" => {
            let clone = clone_of(p, &dir);
            let at = worktree_path(p, &clone, &branch);
            let after = String::new();
            p.local.then_open.insert(at.clone(), what);
            vec![
                journal_at("git.worktree.create", &clone, "", &branch),
                Effect::Report(JournalOutcome::Confirmed),
                Effect::Notify(format!("putting {branch} in a worktree at {}…", local::tilde(&at))),
                Effect::Spawn(Box::pin(async move {
                    let failed_dir = at.clone();
                    let task = tokio::task::spawn_blocking(move || {
                        let result = park(&clone, &at, &branch, "main", &after);
                        local::LocalMsg::Worktree {
                            dir: at,
                            result: result.said,
                            parked: Some(result.observed),
                        }
                    });
                    let msg = task.await.unwrap_or_else(|e| local::LocalMsg::Worktree {
                        dir: failed_dir,
                        result: Err(e.to_string()),
                        parked: None,
                    });
                    Box::new(msg) as radar_core::plugin::Msg
                })),
            ]
        }
        _ => vec![],
    }
}

fn ask3(
    menu: &str,
    title: String,
    notes: Vec<String>,
    buttons: [(char, &str, String, &str); 3],
) -> Menu {
    let mut items: Vec<MenuItem> = notes.into_iter().map(MenuItem::note).collect();
    for (key, label, hint, value) in buttons {
        items.push(MenuItem::new(key, label, hint, value));
    }
    let mut m = Menu::new(menu, title, items).icon(Icon::Branch);
    m.question = true;
    m
}

/// The clone a worktree belongs to (or the folder itself).
fn clone_of(p: &GitPlugin, dir: &str) -> String {
    let all = p
        .local
        .linked
        .iter()
        .chain(p.local.index.iter().flat_map(|i| i.checkouts.iter()));
    all.into_iter()
        .find(|c| c.dir == dir)
        .map(|c| {
            if c.main.is_empty() {
                c.dir.clone()
            } else {
                c.main.clone()
            }
        })
        .unwrap_or_else(|| dir.to_string())
}

// --- answers ---------------------------------------------------------------------

/// A choice in a menu this module opened; `None` if not one of these.
pub(crate) fn on_menu(p: &mut GitPlugin, menu: &str, value: &str) -> Option<Vec<Effect>> {
    let rest = menu.strip_prefix(MENU)?.strip_prefix(':')?;
    let mut it = rest.split('\t');
    let step = it.next()?;
    let a: Vec<&str> = it.collect();
    let arg = |i: usize| a.get(i).copied().unwrap_or_default().to_string();
    Some(match step {
        "link" => link(p, &arg(0), value),
        "use" => use_worktree(p, &arg(0), value),
        "start" => run_start(p, value),
        _ if value == "no" => vec![Effect::Report(JournalOutcome::Cancelled)],
        "dirty" if value == "stash" => {
            let (dir, branch, base, project) = (arg(0), arg(1), arg(2), arg(3));
            let name = stash_name(&branch, p.now);
            p.act.busy.insert(dir.clone());
            vec![blocking(move || match ops::stash(Path::new(&dir), &name) {
                Ok(said) => read(Op::New, &dir, &project, &branch, &base, Some(said)),
                Err(e) => done(&dir, Err(e), false),
            })]
        }
        "dirty" | "park" if value == "park" || step == "park" => {
            let (clone, branch, base) = if step == "park" {
                (arg(0), arg(1), arg(2))
            } else {
                (clone_of(p, &arg(0)), arg(1), arg(2))
            };
            let path = if step == "park" {
                arg(3)
            } else {
                worktree_path(p, &clone, &branch)
            };
            let after = p.cfg.after_worktree.clone();
            vec![
                Effect::Notify(format!("parking {branch} on a worktree…")),
                blocking(move || {
                    let result = park(&clone, &path, &branch, &base, &after);
                    done_with_worktree(&clone, result.said, true, Some(result.observed))
                }),
            ]
        }
        "exists" | "new" => {
            let (dir, branch, base) = (arg(0), arg(1), arg(2));
            let recreate = value == "recreate";
            vec![
                Effect::Notify(format!("making {branch}…")),
                blocking(move || {
                    let result = ops::new_branch(Path::new(&dir), &branch, &base, recreate);
                    done(&dir, result, false)
                }),
            ]
        }
        "rebase" => {
            let (dir, base) = (arg(0), arg(2));
            vec![
                Effect::Notify(format!("rebasing on origin/{base}…")),
                blocking(move || done(&dir, ops::rebase(Path::new(&dir), &base), false)),
            ]
        }
        "push" => {
            let (dir, branch, kind) = (arg(0), arg(1), arg(2));
            let change = if kind == "new" {
                writes::Change::Add
            } else {
                writes::Change::Edit
            };
            if let Some(no) = writes::not_sent(change) {
                return Some(vec![
                    Effect::Report(JournalOutcome::Blocked),
                    Effect::Notify(no),
                ]);
            }
            let force = kind == "force";
            vec![
                Effect::Notify(format!("pushing {branch}…")),
                blocking(move || done(&dir, ops::push(Path::new(&dir), &branch, force), false)),
            ]
        }
        _ => return None,
    })
}

fn done(dir: &str, result: Result<String, String>, rescan: bool) -> ActMsg {
    done_with_worktree(dir, result, rescan, None)
}

fn done_with_worktree(
    dir: &str,
    result: Result<String, String>,
    rescan: bool,
    parked: Option<WorktreeResult>,
) -> ActMsg {
    ActMsg::Done {
        reread: local::read_checkout(Path::new(dir)),
        dir: dir.to_string(),
        result,
        parked,
        rescan,
    }
}

fn park(clone: &str, path: &str, branch: &str, base: &str, after: &str) -> ParkResult {
    park_with(clone, path, branch, after, |step| match step {
        WorktreeStep::Create => ops::add_worktree(Path::new(clone), Path::new(path), branch, base),
        WorktreeStep::After => ops::after(Path::new(path), after),
    })
}

fn park_with(
    clone: &str,
    path: &str,
    branch: &str,
    after: &str,
    mut execute: impl FnMut(WorktreeStep) -> Result<String, String>,
) -> ParkResult {
    let mut observed = WorktreeResult {
        clone: clone.into(),
        path: path.into(),
        branch: branch.into(),
        creation: WorktreeStatus::Unknown,
        after: (!after.is_empty()).then_some(WorktreeStatus::Skipped),
    };
    let mut said = match execute(WorktreeStep::Create) {
        Ok(said) => {
            observed.creation = WorktreeStatus::Succeeded;
            said
        }
        Err(why) => {
            return ParkResult {
                said: Err(why),
                observed,
            };
        }
    };
    if !after.is_empty() {
        match execute(WorktreeStep::After) {
            Ok(last) => {
                observed.after = Some(WorktreeStatus::Succeeded);
                if !last.is_empty() {
                    said.push_str(&format!(" · {last}"));
                }
            }
            Err(e) => {
                observed.after = Some(WorktreeStatus::Unknown);
                said.push_str(&format!(" · {after} failed: {e}"));
            }
        }
    }
    ParkResult {
        said: Ok(said),
        observed,
    }
}

/// A task finished.
pub fn on_msg(p: &mut GitPlugin, msg: ActMsg) -> Vec<Effect> {
    match msg {
        ActMsg::Planned {
            op,
            dir,
            project,
            branch,
            plan,
            forced,
            ahead,
            behind,
            said,
        } => on_planned(
            p, op, dir, project, branch, plan, forced, ahead, behind, said,
        ),
        ActMsg::Done {
            dir,
            result,
            parked,
            reread,
            rescan,
        } => {
            p.act.busy.remove(&dir);
            // Git may have changed a checkout or remote before returning an error.
            // Preserve that uncertainty rather than treating all errors as no-op failures.
            let outcome = parked.as_ref().map_or_else(
                || {
                    if result.is_ok() {
                        JournalOutcome::Succeeded
                    } else {
                        JournalOutcome::Unknown
                    }
                },
                |result| result.status().outcome(),
            );
            let mut fx = vec![Effect::Report(outcome)];
            match result {
                Ok(said) => {
                    if let Some(now) = reread {
                        for list in [
                            p.local.index.as_mut().map(|i| &mut i.checkouts),
                            Some(&mut p.local.linked),
                        ]
                        .into_iter()
                        .flatten()
                        {
                            if let Some(c) = list.iter_mut().find(|c| c.dir == now.dir) {
                                c.branch = now.branch.clone();
                            }
                        }
                    }
                    fx.push(Effect::PersistCache);
                    fx.push(Effect::Notify(writes::done_line(&said)));
                    if rescan {
                        local::refresh_links(p);
                        fx.extend(local::scan_now(p, false));
                    }
                }
                Err(e) => fx.push(Effect::Notify(writes::failed_line(&e))),
            }
            // Preserve the original context for UI/cache/rescan effects. Step
            // summaries are independent terminal observations after it finishes.
            if let Some(result) = parked {
                fx.extend(journal_worktree(&result));
            }
            fx
        }
        ActMsg::StartPlanned { key, items } => ask_start(p, key, items),
        ActMsg::Finished { key, results } => finished(p, &key, results),
    }
}

// --- link and use ----------------------------------------------------------------

/// Pick the clone to link `project` to, from the checkouts found here.
/// Link a repo to a folder on this machine: a field to type or paste the
/// folder in, offering this repo's clones radar found (when nothing is typed)
/// and the folders the typed path goes on to, each said for what it is.
pub fn ask_link(p: &GitPlugin, project: &str) -> Vec<Effect> {
    // Nothing known of it here: look for its clone first, then ask.
    if candidates(p, project).is_empty() && !p.local.looked.contains_key(project) {
        return local::look(p, project);
    }
    open_link(p, project)
}

/// This repo's clones known here: in use or found by a scan, then those a
/// look for it found; each once, by folder.
fn candidates(p: &GitPlugin, project: &str) -> Vec<Checkout> {
    let mut out: Vec<Checkout> = local::chosen(p, project).cloned().into_iter().collect();
    let scanned = p.local.index.iter().flat_map(|i| i.checkouts.iter());
    let looked = p.local.looked.get(project).into_iter().flatten();
    for c in scanned.filter(|c| c.project.eq_ignore_ascii_case(project)).chain(looked) {
        if c.main.is_empty() && !out.iter().any(|o| o.dir == c.dir) {
            out.push(c.clone());
        }
    }
    out
}

/// The field to type the folder in, starting on the clone radar would pick.
pub fn open_link(p: &GitPlugin, project: &str) -> Vec<Effect> {
    let found = candidates(p, project);
    let start = found.first().map(|c| local::tilde(&c.dir)).unwrap_or_default();
    let mut fx = Vec::new();
    match (p.local.looked.get(project).map(Vec::len), found.len()) {
        (Some(_), 0) => fx.push(Effect::Notify(
            "no clone of it found in your usual folders: type or paste its folder".into(),
        )),
        (Some(n), _) => fx.push(Effect::Notify(format!("found {n} clone{} of it", if n == 1 { "" } else { "s" }))),
        (None, _) => {}
    }
    fx.push(Effect::Menu(
        Menu::typed(id("link", &[project]), format!("link {project} to a local folder"), start)
            .icon(Icon::Link),
    ));
    fx
}

/// What a typed folder is, for the link field: its verdict, and offers.
pub fn typing(p: &GitPlugin, menu: &str, text: &str) -> radar_core::Typing {
    let Some(project) = menu.strip_prefix(&id("link", &[""])) else {
        return radar_core::Typing::default();
    };
    let said = |dir: &str| match local::read_checkout(&local::expand(&local::clean_folder(dir))) {
        Some(c) if c.project.eq_ignore_ascii_case(project) => format!("a clone of it, on {}", c.branch),
        Some(c) if c.project.is_empty() => "a git folder".to_string(),
        Some(c) => format!("a clone of {}", c.project),
        None => String::new(),
    };
    let found: Vec<String> = candidates(p, project).iter().map(|c| local::tilde(&c.dir)).collect();
    let typed = text.trim();
    let offers: Vec<MenuItem> = if typed.is_empty() || found.iter().any(|f| f == typed) {
        // This repo's clones (found by origin or by name), then where to
        // start typing: the folders radar looks in, the usual ones.
        let places = local::usual_places(&p.cfg.roots)
            .into_iter()
            .map(|r| format!("{}/", r.trim_end_matches('/')));
        found
            .into_iter()
            .chain(places)
            .take(8)
            .map(|d| MenuItem::pick(d.clone(), said(&d), d))
            .collect()
    } else {
        radar_core::system::folders_like(text, 8)
            .into_iter()
            .map(|d| MenuItem::pick(d.clone(), said(&d), d))
            .collect()
    };
    let verdict = (!text.trim().is_empty()).then(|| crate::settings::link_dir_typed(project, text));
    radar_core::Typing { verdict, offers }
}

fn link(p: &mut GitPlugin, project: &str, dir: &str) -> Vec<Effect> {
    let dir = local::clean_folder(dir);
    if dir.is_empty() {
        return vec![];
    }
    let dir = local::expand(&dir).display().to_string();
    p.cfg
        .links
        .retain(|l| !l.repo.eq_ignore_ascii_case(project));
    p.cfg.links.push(local::Link {
        repo: project.to_string(),
        dir: local::tilde(&dir),
        ..Default::default()
    });
    local::refresh_links(p);
    vec![Effect::Notify(writes::done_line(&format!(
        "{project} is linked to {}",
        local::tilde(&dir)
    )))]
}

/// Pick which of a repo's worktrees (or its clone) is acted on.
pub fn ask_use(p: &GitPlugin, project: &str) -> Vec<Effect> {
    let all = local::checkouts_for(p, project);
    if all.len() < 2 {
        return vec![Effect::Notify(
            "this repo has only one checkout here".into(),
        )];
    }
    let now = local::chosen(p, project).map(|c| c.dir.clone());
    let items = all
        .into_iter()
        .map(|c| {
            let kind = if c.main.is_empty() {
                "clone"
            } else {
                "worktree"
            };
            let on = if now.as_deref() == Some(c.dir.as_str()) {
                " · in use"
            } else {
                ""
            };
            MenuItem::pick(
                local::tilde(&c.dir),
                format!("{kind} · on {}{on}", c.branch),
                c.dir.clone(),
            )
        })
        .collect();
    vec![Effect::Menu(
        Menu::new(
            id("use", &[project]),
            format!("which checkout of {project}?"),
            items,
        )
        .icon(Icon::Local),
    )]
}

fn use_worktree(p: &mut GitPlugin, project: &str, dir: &str) -> Vec<Effect> {
    if dir.is_empty() {
        return vec![];
    }
    p.local
        .use_worktree
        .retain(|r, _| !r.eq_ignore_ascii_case(project));
    p.local
        .use_worktree
        .insert(project.to_string(), dir.to_string());
    vec![
        Effect::PersistCache,
        Effect::Notify(writes::done_line(&format!(
            "{project} is worked on in {}",
            local::tilde(dir)
        ))),
    ]
}

// --- start work over the bus -----------------------------------------------------

/// `workspace.start`: read every repo, then ask once.
pub(crate) fn on_bus(p: &mut GitPlugin, topic: &str, payload: &Value) -> Vec<Effect> {
    if topic != START {
        return vec![];
    }
    let t = &payload["ticket"];
    let text = |v: &Value| v.as_str().unwrap_or_default().to_string();
    let (key, title, kind) = (text(&t["key"]), text(&t["title"]), text(&t["type"]));
    let repos: Vec<(String, String, bool)> = payload["repos"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| (text(&r["dir"]), text(&r["repo"]), r["how"] == "worktree"))
        .filter(|(dir, _, _)| !dir.is_empty())
        .collect();
    if key.is_empty() || repos.is_empty() {
        return vec![Effect::Notify(
            "start work: no ticket or no repos to start in".into(),
        )];
    }
    // Names, bases and paths come from the config now; git is read off the loop.
    let now = p.now;
    let work: Vec<(String, String, bool, String, String, String, String)> = repos
        .into_iter()
        .map(|(dir, repo, worktree)| {
            let project = if repo.is_empty() {
                local::read_checkout(Path::new(&dir))
                    .map(|c| c.project)
                    .unwrap_or_default()
            } else {
                repo.clone()
            };
            let branch = branch_for(
                p,
                &project,
                &Ticket {
                    key: &key,
                    title: &title,
                    kind: &kind,
                },
            );
            let base = base_for(p, &project);
            let path = if worktree {
                worktree_path(p, &clone_of(p, &dir), &branch)
            } else {
                String::new()
            };
            let problem = name_problem(p, &project).unwrap_or_default();
            (dir, repo, worktree, branch, base, path, problem)
        })
        .collect();
    let k = key.clone();
    vec![
        journal("git.workspace.start"),
        Effect::Notify(format!(
            "reading {} for {key}…",
            plural(work.len(), "repo", "repos")
        )),
        blocking(move || ActMsg::StartPlanned {
            items: work
                .into_iter()
                .map(|(dir, repo, worktree, branch, base, path, problem)| {
                    let mut item = plan_item(&dir, &repo, worktree, &branch, &base, &path, now);
                    if !problem.is_empty() {
                        item.refused = problem;
                    }
                    item
                })
                .collect(),
            key: k,
        }),
    ]
}

fn plan_item(
    dir: &str,
    repo: &str,
    worktree: bool,
    branch: &str,
    base: &str,
    path: &str,
    now: i64,
) -> Item {
    let d = Path::new(dir);
    let plan = ops::plan_new_branch(d, branch, (!base.is_empty()).then_some(base));
    let refused = match branch::valid_ref(branch) {
        Err(why) => format!("\"{branch}\" is not a branch name git takes: {why}"),
        Ok(()) if !worktree && plan.current == branch => String::new(),
        Ok(()) if worktree && Path::new(path).exists() => {
            format!("{} is there already", local::tilde(path))
        }
        Ok(()) => String::new(),
    };
    Item {
        dir: dir.to_string(),
        repo: repo.to_string(),
        branch: branch.to_string(),
        base: plan.base.clone(),
        worktree: if worktree {
            path.to_string()
        } else {
            String::new()
        },
        stash: if !worktree && plan.dirty > 0 && plan.current != branch {
            stash_name(branch, now)
        } else {
            String::new()
        },
        exists: plan.exists_local,
        refused,
    }
}

/// One line per repo for the one question.
pub(crate) fn item_line(i: &Item) -> String {
    let name = if i.repo.is_empty() {
        Path::new(&i.dir)
            .file_name()
            .map_or(i.dir.clone(), |n| n.to_string_lossy().into_owned())
    } else {
        i.repo.clone()
    };
    if !i.refused.is_empty() {
        return format!("{name}: {} (refused)", i.refused);
    }
    let mut line = if i.exists {
        format!("{name}: switch to {}", i.branch)
    } else {
        format!("{name}: {} from origin/{}", i.branch, i.base)
    };
    if !i.stash.is_empty() {
        line.push_str(&format!(", stash {}", i.stash));
    }
    if !i.worktree.is_empty() {
        line.push_str(&format!(", worktree at {}", local::tilde(&i.worktree)));
    }
    line
}

fn ask_start(p: &mut GitPlugin, key: String, items: Vec<Item>) -> Vec<Effect> {
    let mut m = Menu::confirm(
        id("start", &[&key]),
        format!("start {key} in {}?", plural(items.len(), "repo", "repos")),
        "run it",
        "leave it",
    )
    .icon(Icon::Branch);
    for i in &items {
        m = m.saying(item_line(i));
    }
    if !p.cfg.after_worktree.is_empty() && items.iter().any(|i| !i.worktree.is_empty()) {
        m = m.saying(format!(
            "then runs {} in each new worktree",
            p.cfg.after_worktree
        ));
    }
    // The first refusal stops the run (run_items): say which repos that leaves out.
    if let Some(at) = items.iter().position(|i| !i.refused.is_empty()) {
        let left = items.len() - at - 1;
        m = m.saying(if left == 0 {
            "a refused repo stops the run".to_string()
        } else {
            format!(
                "the run stops at the first refused repo: the {} after it are not run",
                plural(left, "repo", "repos")
            )
        });
    }
    p.act.start = Some(Started { key, items });
    vec![Effect::Menu(m)]
}

fn run_start(p: &mut GitPlugin, value: &str) -> Vec<Effect> {
    let Some(start) = p.act.start.take() else {
        return vec![];
    };
    if value != "yes" {
        return vec![
            Effect::Report(JournalOutcome::Cancelled),
            Effect::Notify(format!("{} left as it is; nothing done", start.key)),
        ];
    }
    let after = p.cfg.after_worktree.clone();
    let Started { key, items } = start;
    let k = key.clone();
    vec![
        Effect::Report(JournalOutcome::Confirmed),
        Effect::Report(JournalOutcome::Started),
        Effect::Notify(format!("starting {key}…")),
        blocking(move || ActMsg::Finished {
            results: run_items(&items, &after),
            key: k,
        }),
    ]
}

/// Do every repo in order; the first refusal stops the rest.
pub(crate) fn run_items(items: &[Item], after: &str) -> Vec<StartResult> {
    run_items_with(items, after, start_one)
}

fn run_items_with(
    items: &[Item],
    after: &str,
    mut execute: impl FnMut(&Item, &str, &mut StartResult) -> Result<(), String>,
) -> Vec<StartResult> {
    let mut results: Vec<StartResult> = Vec::new();
    let mut stopped: Option<String> = None;
    for i in items {
        let mut r = StartResult {
            dir: i.dir.clone(),
            repo: i.repo.clone(),
            branch: i.branch.clone(),
            base: i.base.clone(),
            worktree: String::new(),
            stash: String::new(),
            ok: false,
            why: String::new(),
            status: StartStatus::Skipped,
            parked: None,
        };
        if let Some(at) = &stopped {
            r.why = format!("not run: stopped at {at}");
            results.push(r);
            continue;
        }
        let outcome = if !i.refused.is_empty() {
            r.status = StartStatus::Refused;
            Err(i.refused.clone())
        } else {
            r.status = StartStatus::Unknown;
            execute(i, after, &mut r)
        };
        match outcome {
            Ok(()) => {
                r.ok = true;
                r.status = r
                    .parked
                    .as_ref()
                    .map_or(StartStatus::Succeeded, WorktreeResult::status);
            }
            Err(why) => {
                r.why = why;
                stopped = Some(if i.repo.is_empty() {
                    i.dir.clone()
                } else {
                    i.repo.clone()
                });
            }
        }
        results.push(r);
    }
    results
}

fn start_one(i: &Item, after: &str, r: &mut StartResult) -> Result<(), String> {
    let dir = Path::new(&i.dir);
    if !i.worktree.is_empty() {
        return started_worktree(i, r, park(&i.dir, &i.worktree, &i.branch, &i.base, after));
    }
    if !i.stash.is_empty() {
        ops::stash(dir, &i.stash)?;
        r.stash = i.stash.clone();
    }
    ops::new_branch(dir, &i.branch, &i.base, false)?;
    Ok(())
}

fn started_worktree(i: &Item, r: &mut StartResult, result: ParkResult) -> Result<(), String> {
    r.parked = Some(result.observed);
    result.said?;
    r.worktree = i.worktree.clone();
    Ok(())
}

/// `git.started`: how each repo went, for whoever started the work.
pub(crate) fn started_payload(key: &str, results: &[StartResult]) -> Value {
    json!({
        "ticket": key,
        "results": results.iter().map(|r| json!({
            "dir": r.dir,
            "repo": r.repo,
            "branch": r.branch,
            "base": r.base,
            "worktree": r.worktree,
            "stash": r.stash,
            "ok": r.ok,
            "why": r.why,
        })).collect::<Vec<_>>(),
    })
}

/// Fold the results into what is said and emitted.
pub(crate) fn finished(p: &mut GitPlugin, key: &str, results: Vec<StartResult>) -> Vec<Effect> {
    let ok = results.iter().filter(|r| r.ok).count();
    let said = if ok == results.len() {
        writes::done_line(&format!("started {key} in {}", plural(ok, "repo", "repos")))
    } else {
        let bad = results
            .iter()
            .find(|r| !r.ok && r.status != StartStatus::Skipped)
            .or_else(|| results.iter().find(|r| !r.ok));
        writes::failed_line(&format!(
            "{key}: {ok} of {} done; {}",
            results.len(),
            bad.map_or(String::new(), |b| format!(
                "{}: {}",
                if b.repo.is_empty() { &b.dir } else { &b.repo },
                b.why
            ))
        ))
    };
    let outcome = if results.is_empty() {
        JournalOutcome::Blocked
    } else if results.iter().all(|r| r.status == StartStatus::Succeeded) {
        JournalOutcome::Succeeded
    } else if results
        .iter()
        .any(|r| matches!(r.status, StartStatus::Succeeded | StartStatus::Partial))
    {
        JournalOutcome::Partial
    } else if results.iter().any(|r| r.status == StartStatus::Unknown) {
        JournalOutcome::Unknown
    } else {
        JournalOutcome::Blocked
    };
    let mut fx = vec![Effect::Report(outcome)];
    for result in &results {
        let mut event = journal_at(
            "git.workspace.repo.result",
            &result.dir,
            &result.repo,
            &result.branch,
        );
        if result.status == StartStatus::Skipped
            && let Effect::Journal(action) = &mut event
        {
            action.metadata.reason = Some(radar_core::journal::Reason::Interrupted);
        }
        fx.extend([event, Effect::Report(result.status.outcome())]);
        if let Some(parked) = &result.parked {
            fx.extend(journal_worktree(parked));
        }
    }
    fx.extend(vec![
        journal("git.workspace.completed"),
        Effect::Report(outcome),
        Effect::Notify(said),
        Effect::Emit {
            topic: STARTED,
            payload: started_payload(key, &results),
        },
        Effect::PersistCache,
    ]);
    local::refresh_links(p);
    fx.extend(local::scan_now(p, false));
    fx
}

/// The folder a path would be (for tests and callers that only have a string).
#[allow(dead_code)]
pub(crate) fn path_of(s: &str) -> PathBuf {
    PathBuf::from(s)
}

#[cfg(test)]
mod journal_tests {
    use super::*;

    fn item(repo: &str) -> Item {
        Item {
            dir: format!("/work/{repo}"),
            repo: format!("acme/{repo}"),
            branch: "alice/ABC-12".into(),
            base: "main".into(),
            worktree: format!("/work/{repo}-ABC-12"),
            stash: String::new(),
            exists: false,
            refused: String::new(),
        }
    }

    fn simulated_park(i: &Item, after: &str, hook: Result<String, String>) -> ParkResult {
        park_with(&i.dir, &i.worktree, &i.branch, after, |step| match step {
            WorktreeStep::Create => Ok("created tree".into()),
            WorktreeStep::After => hook.clone(),
        })
    }

    fn actions(effects: &[Effect]) -> Vec<&ActionDescriptor> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Journal(action) => Some(action),
                _ => None,
            })
            .collect()
    }

    fn outcomes(effects: &[Effect]) -> Vec<JournalOutcome> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Report(outcome) => Some(*outcome),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn git_results_are_explicit_and_errors_keep_uncertainty() {
        let mut p = GitPlugin::new();
        let result = |result| ActMsg::Done {
            dir: "/work/api".into(),
            result,
            parked: None,
            reread: None,
            rescan: false,
        };
        assert_eq!(
            outcomes(&on_msg(&mut p, result(Ok("done".into())))),
            [JournalOutcome::Succeeded]
        );
        assert_eq!(
            outcomes(&on_msg(&mut p, result(Err("private stderr".into())))),
            [JournalOutcome::Unknown]
        );
        let cancelled = on_menu(&mut p, "git-act:new\t/work/api", "no").unwrap();
        assert_eq!(outcomes(&cancelled), [JournalOutcome::Cancelled]);
    }

    #[test]
    fn partial_start_keeps_each_repo_outcome_and_identity() {
        let make = |repo: &str, status: StartStatus, why: &str| StartResult {
            dir: format!("/work/{repo}"),
            repo: format!("acme/{repo}"),
            branch: "alice/ABC-12".into(),
            base: "main".into(),
            worktree: String::new(),
            stash: String::new(),
            ok: status == StartStatus::Succeeded,
            why: why.into(),
            status,
            parked: None,
        };
        let mut p = GitPlugin::new();
        let effects = finished(
            &mut p,
            "ABC-12",
            vec![
                make("api", StartStatus::Succeeded, ""),
                make("web", StartStatus::Unknown, "private stderr"),
                make("docs", StartStatus::Skipped, "not run: stopped at web"),
            ],
        );
        assert_eq!(
            outcomes(&effects),
            [
                JournalOutcome::Partial,
                JournalOutcome::Succeeded,
                JournalOutcome::Unknown,
                JournalOutcome::Blocked,
                JournalOutcome::Partial
            ]
        );
        let repos: Vec<_> = effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Journal(action) if action.kind.as_str() == "git.workspace.repo.result" => {
                    Some(
                        action
                            .subjects
                            .iter()
                            .find(|s| s.key.kind == "repo")
                            .unwrap()
                            .key
                            .id
                            .as_str(),
                    )
                }
                _ => None,
            })
            .collect();
        assert_eq!(repos, ["acme/api", "acme/web", "acme/docs"]);
        for effect in &effects {
            if let Effect::Journal(action) = effect {
                assert!(!format!("{action:?}").contains("private stderr"));
            }
        }
    }

    #[test]
    fn park_retains_typed_creation_hook_and_skipped_results_without_changing_display() {
        let i = item("api");
        for (after, hook, expected, status) in [
            (
                "",
                Err("must not run".into()),
                "created tree",
                StartStatus::Succeeded,
            ),
            (
                "setup",
                Ok(String::new()),
                "created tree",
                StartStatus::Succeeded,
            ),
            (
                "setup",
                Ok("ready".into()),
                "created tree · ready",
                StartStatus::Succeeded,
            ),
            (
                "setup",
                Err("private output".into()),
                "created tree · setup failed: private output",
                StartStatus::Partial,
            ),
        ] {
            let mut calls = Vec::new();
            let result = park_with(&i.dir, &i.worktree, &i.branch, after, |step| {
                calls.push(step);
                match step {
                    WorktreeStep::Create => Ok("created tree".into()),
                    WorktreeStep::After => hook.clone(),
                }
            });
            assert_eq!(result.said, Ok(expected.into()));
            assert_eq!(result.observed.status(), status);
            assert_eq!(calls.len(), if after.is_empty() { 1 } else { 2 });
            assert_eq!(result.observed.after.is_some(), !after.is_empty());
        }
        let result = park_with(&i.dir, &i.worktree, &i.branch, "setup", |step| {
            assert_eq!(step, WorktreeStep::Create, "the hook must not run");
            Err("opaque creation error".into())
        });
        assert_eq!(result.said, Err("opaque creation error".into()));
        assert_eq!(result.observed.creation, WorktreeStatus::Unknown);
        assert_eq!(result.observed.after, Some(WorktreeStatus::Skipped));
        let fx = journal_worktree(&result.observed);
        assert_eq!(
            outcomes(&fx),
            [JournalOutcome::Unknown, JournalOutcome::Blocked]
        );
        assert_eq!(
            actions(&fx)[1].metadata.reason,
            Some(radar_core::journal::Reason::Interrupted)
        );
    }

    #[test]
    fn park_hook_error_preserves_success_ui_and_refresh_before_private_step_records() {
        let i = item("api");
        let result = simulated_park(&i, "SECRET_COMMAND", Err("SECRET_OUTPUT".into()));
        let said = result.said.clone().unwrap();
        let mut p = GitPlugin::new();
        p.cfg.roots = vec!["/work".into()];
        p.act.busy.insert(i.dir.clone());
        let fx = on_msg(
            &mut p,
            ActMsg::Done {
                dir: i.dir.clone(),
                result: result.said,
                parked: Some(result.observed),
                reread: None,
                rescan: true,
            },
        );
        assert!(p.act.busy.is_empty());
        assert!(p.local.scanning);
        assert_eq!(
            outcomes(&fx),
            [
                JournalOutcome::Partial,
                JournalOutcome::Succeeded,
                JournalOutcome::Unknown
            ]
        );
        let first_step = fx
            .iter()
            .position(|e| matches!(e, Effect::Journal(a) if a.kind.as_str() == "git.worktree.add"))
            .unwrap();
        assert!(
            fx[..first_step]
                .iter()
                .any(|e| matches!(e, Effect::Spawn(_)))
        );
        assert!(
            fx[..first_step]
                .iter()
                .any(|e| matches!(e, Effect::PersistCache))
        );
        assert!(
            fx[..first_step]
                .iter()
                .any(|e| matches!(e, Effect::Notify(m) if m == &writes::done_line(&said)))
        );
        let recorded = actions(&fx[first_step..]);
        assert_eq!(recorded[0].kind.as_str(), "git.worktree.add");
        assert_eq!(recorded[1].kind.as_str(), "git.worktree.after");
        assert_eq!(recorded[0].subjects[0].role, SubjectRole::Source);
        let destination = recorded[0]
            .subjects
            .iter()
            .find(|s| s.role == SubjectRole::Destination)
            .unwrap();
        assert_eq!(destination.key.instance, i.dir);
        assert_eq!(destination.key.id, i.worktree);
        assert_eq!(recorded[1].subjects.len(), 1);
        assert_eq!(recorded[1].subjects[0].key.id, i.worktree);
        assert!(!format!("{recorded:?}").contains("SECRET_"));
    }

    #[test]
    fn workspace_hook_errors_keep_continuation_and_exact_bus_payload_but_report_partial() {
        let items = [item("api"), item("web")];
        let mut calls = Vec::new();
        let results = run_items_with(&items, "SECRET_COMMAND", |i, after, r| {
            calls.push(i.repo.clone());
            started_worktree(i, r, simulated_park(i, after, Err("SECRET_OUTPUT".into())))
        });
        assert_eq!(calls, ["acme/api", "acme/web"]);
        assert!(
            results
                .iter()
                .all(|r| r.ok && r.why.is_empty() && r.status == StartStatus::Partial)
        );
        assert_eq!(
            started_payload("ABC-12", &results),
            json!({
                "ticket": "ABC-12",
                "results": items.iter().map(|i| json!({
                    "dir": i.dir, "repo": i.repo, "branch": i.branch, "base": i.base,
                    "worktree": i.worktree, "stash": "", "ok": true, "why": "",
                })).collect::<Vec<_>>(),
            })
        );
        let fx = finished(&mut GitPlugin::new(), "ABC-12", results);
        assert_eq!(
            outcomes(&fx),
            [
                JournalOutcome::Partial,
                JournalOutcome::Partial,
                JournalOutcome::Succeeded,
                JournalOutcome::Unknown,
                JournalOutcome::Partial,
                JournalOutcome::Succeeded,
                JournalOutcome::Unknown,
                JournalOutcome::Partial,
            ]
        );
        assert!(fx.iter().any(|e| matches!(e, Effect::Notify(m) if m == &writes::done_line("started ABC-12 in 2 repos"))));
        assert!(!format!("{:?}", actions(&fx)).contains("SECRET_"));
    }

    #[test]
    fn workspace_refused_and_skipped_are_typed_not_inferred_from_error_text() {
        let mut items = [item("api"), item("web")];
        items[0].refused = "refused before execution".into();
        let results = run_items_with(&items, "", |_, _, _| panic!("no work allowed"));
        assert_eq!(results[0].status, StartStatus::Refused);
        assert_eq!(results[1].status, StartStatus::Skipped);
        assert_eq!(
            outcomes(&finished(&mut GitPlugin::new(), "ABC-12", results)),
            [JournalOutcome::Blocked; 4]
        );
        items[0].refused.clear();
        let mut calls = 0;
        let results = run_items_with(&items, "", |_, _, _| {
            calls += 1;
            Err("not run: stopped at SECRET_PROCESS_OUTPUT".into())
        });
        assert_eq!(calls, 1);
        assert_eq!(results[0].status, StartStatus::Unknown);
        assert_eq!(results[1].status, StartStatus::Skipped);
        let fx = finished(&mut GitPlugin::new(), "ABC-12", results);
        assert_eq!(
            outcomes(&fx),
            [
                JournalOutcome::Unknown,
                JournalOutcome::Unknown,
                JournalOutcome::Blocked,
                JournalOutcome::Unknown
            ]
        );
        assert_eq!(
            actions(&fx)[1].metadata.reason,
            Some(radar_core::journal::Reason::Interrupted)
        );
        assert!(!format!("{:?}", actions(&fx)).contains("SECRET_"));
    }

    #[test]
    fn park_step_subjects_reject_unsafe_identities() {
        let mut result = simulated_park(&item("api"), "setup", Ok(String::new())).observed;
        result.path = "/work/token=SECRET_PATH".into();
        result.branch = "bad\nbranch".into();
        let fx = journal_worktree(&result);
        let recorded = actions(&fx);
        assert_eq!(recorded[0].subjects.len(), 1);
        assert_eq!(recorded[0].metadata.omitted_subjects, 2);
        assert!(recorded.iter().all(|a| a.metadata.redacted));
        assert!(recorded[1].subjects.is_empty());
        assert!(!format!("{recorded:?}").contains("SECRET_"));
    }

    #[test]
    fn park_aggregate_task_steps_and_rescan_keep_independent_host_identities() {
        use radar_core::host::{Event, HostEffect, JournalRecord};
        use radar_core::journal::ActionId;
        use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use radar_core::{Host, Plugin};

        struct Harness(GitPlugin);
        impl Plugin for Harness {
            fn id(&self) -> &'static str {
                "git"
            }
            fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
                Ok(())
            }
            fn config(&self) -> Option<toml::Value> {
                None
            }
            fn on_menu(&mut self, menu: &str, value: &str) -> Vec<Effect> {
                on_menu(&mut self.0, menu, value).unwrap_or_default()
            }
            fn on_msg(&mut self, msg: Msg) -> Vec<Effect> {
                match msg.downcast::<ActMsg>() {
                    Ok(msg) => on_msg(&mut self.0, *msg),
                    Err(_) => {
                        let i = item("api");
                        begin(
                            &mut self.0,
                            Op::Park,
                            &i.repo,
                            &Checkout {
                                dir: i.dir,
                                branch: "main".into(),
                                ..Checkout::default()
                            },
                            i.branch,
                        )
                    }
                }
            }
        }
        fn task(effects: &[HostEffect]) -> (ActionId, Option<ActionId>) {
            effects
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
                .expect("spawned task")
        }
        let i = item("api");
        let mut plugin = GitPlugin::new();
        plugin.cfg.roots = vec!["/work".into()];
        plugin.cfg.worktree = i.worktree.clone();
        plugin.cfg.after_worktree = "SECRET_COMMAND".into();
        let mut host =
            Host::new(vec![Box::new(Harness(plugin))], radar_core::plugin::DEMO_NOW).unwrap();
        host.enable_journal();
        let planning = host.update(Event::Msg {
            plugin: 0,
            msg: Box::new(()),
        });
        let aggregate = planning
            .iter()
            .find_map(|effect| match effect {
                HostEffect::Journal(JournalRecord::Begin { id, descriptor, .. })
                    if descriptor.kind.as_str() == "git.worktree.create" =>
                {
                    Some(id.clone())
                }
                _ => None,
            })
            .expect("park aggregate");
        let (read, semantic) = task(&planning);
        // Supply observed task replies without running git or the configured hook.
        host.update(Event::Tracked {
            action: read,
            semantic,
            event: Box::new(Event::Msg {
                plugin: 0,
                msg: Box::new(ActMsg::Planned {
                    op: Op::Park,
                    dir: i.dir.clone(),
                    project: i.repo.clone(),
                    branch: i.branch.clone(),
                    plan: Plan {
                        base: "main".into(),
                        current: "main".into(),
                        ..Plan::default()
                    },
                    forced: false,
                    ahead: 0,
                    behind: 0,
                    said: None,
                }),
            }),
        });
        let launched = host.update(Event::Key(KeyEvent::new(
            KeyCode::Char('y'),
            KeyModifiers::NONE,
        )));
        let (run, semantic) = task(&launched);
        assert_eq!(semantic, Some(aggregate.clone()));
        let result = simulated_park(&i, "SECRET_COMMAND", Err("SECRET_OUTPUT".into()));
        let completed = host.update(Event::Tracked {
            action: run.clone(),
            semantic,
            event: Box::new(Event::Msg {
                plugin: 0,
                msg: Box::new(ActMsg::Done {
                    dir: i.dir,
                    result: result.said,
                    parked: Some(result.observed),
                    reread: None,
                    rescan: true,
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
        let terminal = |id: &ActionId| {
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
        assert_eq!(terminal(&run), [JournalOutcome::Partial]);
        let steps: Vec<_> = records
            .iter()
            .filter_map(|record| match record {
                JournalRecord::Begin {
                    id,
                    descriptor,
                    parent,
                } if matches!(
                    descriptor.kind.as_str(),
                    "git.worktree.add" | "git.worktree.after"
                ) =>
                {
                    assert_eq!(parent, &Some(run.clone()));
                    Some(id.clone())
                }
                _ => None,
            })
            .collect();
        assert_eq!(steps.len(), 2);
        assert_ne!(steps[0], steps[1]);
        assert_eq!(terminal(&steps[0]), [JournalOutcome::Succeeded]);
        assert_eq!(terminal(&steps[1]), [JournalOutcome::Unknown]);
        assert!(!records.iter().any(|record| matches!(record,
            JournalRecord::Outcome { id, outcome: JournalOutcome::Started, .. } if steps.contains(id))));
        let first_step = records
            .iter()
            .position(|record| {
                matches!(record,
            JournalRecord::Begin { id, .. } if id == &steps[0])
            })
            .unwrap();
        assert!(records[..first_step].iter().any(|record| matches!(record,
            JournalRecord::Outcome { id, outcome: JournalOutcome::Partial, .. } if id == &aggregate)));
        let scan = records[..first_step]
            .iter()
            .find_map(|record| match record {
                JournalRecord::Begin {
                    id,
                    descriptor,
                    parent,
                } if descriptor.kind.as_str() == "git.checkouts.scan" => {
                    assert_eq!(parent, &Some(run.clone()));
                    Some(id.clone())
                }
                _ => None,
            })
            .expect("scan before step observations");
        assert_eq!(task(&completed).1, Some(scan));
        assert!(!format!("{records:?}").contains("SECRET_"));
    }
}
