//! Item pages: `⏎` on a row of the repositories tree opens a page for that
//! tenancy, repo or merge request, over the whole screen body. A page has a
//! header saying where you are, a title line and a row of tabs (`[` and `]`
//! switch them). `esc` goes back: to the page it was opened from, or to the
//! tree with its cursor where it was. Pages stack, so repo → merge request →
//! `esc` lands on the repo again.
//!
//! What the list query does not carry (a merge request's description,
//! discussions, jobs, commits and files; a repo's pipelines) is fetched when
//! a page opens and kept on the plugin.

pub(crate) mod act;
pub(crate) mod changes;
mod changes_neutral;
mod job;
mod mr;
pub(crate) mod neutral;
mod repo;
pub(crate) mod review;
mod tenancy;

use std::cell::{Cell, RefCell};

use radar_core::plugin::{Binding, Effect, Outcome, Pane};
use radar_core::{Row, Subject};
use radar_core::view::{Chord, Key};
use radar_core::ui::diff::DiffState;
use radar_core::ui::icon::Icon;
use radar_core::ui::log::LogState;
use radar_core::ui::Mark;
use radar_core::view::scroll::Scroll;

use crate::config::Source;
use crate::log_ownership::{self, JobKey};
use crate::model::{Mr, MrDetail, Pipeline, PipelineRun, Project};
use crate::screen::{self, Selected};
use crate::{GitlabPlugin, why};

/// What a page is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A tenancy, by its source's `group`.
    Tenancy(String),
    /// A repo, by its full path.
    Repo(String),
    Mr {
        project: String,
        iid: u64,
    },
    /// A job of a pipeline, by its id.
    Job {
        project: String,
        id: u64,
    },
}

impl Kind {
    /// The page's own subject, independent of its tab, row or review editor.
    pub(crate) fn identity(&self) -> String {
        match self {
            Self::Tenancy(group) => serde_json::json!(["tenancy", group]),
            Self::Repo(path) => serde_json::json!(["repository", path]),
            Self::Mr { project, iid } => serde_json::json!(["merge-request", project, iid]),
            Self::Job { project, id } => serde_json::json!(["job", project, id]),
        }
        .to_string()
    }

    /// The exact requested job, on a job's page.
    pub(crate) fn job(&self) -> Option<JobKey> {
        match self {
            Self::Job { project, id } => Some(JobKey::new(project, *id)),
            _ => None,
        }
    }
}

/// A page on the stack: what it shows, which tab, and where it is scrolled.
#[derive(Debug)]
pub(crate) struct Page {
    pub kind: Kind,
    pub tab: usize,
    /// Text tabs: the first line shown. List tabs: the first row shown.
    pub scroll: Scroll,
    /// List tabs: the cursor.
    pub row: Cell<usize>,
    /// The tabs other parts added, after the page's own, as last drawn.
    pub extra: RefCell<Vec<(String, Vec<Row>)>>,
    /// A job's page: where its log is, its folds and its search.
    pub log: LogState,
    /// A block that leads a tab above its list (why main is red): its scroll.
    pub lead: Scroll,
    /// A merge request's Changes tab: the file open (its index in the
    /// merge request's list), the diff's cursor, and the files looked at.
    pub file: Cell<usize>,
    /// The files as a folder tree: its state (cursor, open folders), the
    /// folders already opened once, whether it is on screen (then it is
    /// the page's first pane, `Nav`, beside the diff), and whether the
    /// first file in tree order was chosen to open.
    pub tree: RefCell<radar_core::view::TreeNav>,
    pub tree_seen: RefCell<std::collections::HashSet<Vec<String>>>,
    pub tree_shown: Cell<bool>,
    /// The diff was drawn side by side (wanted, and wide enough).
    pub split_shown: Cell<bool>,
    pub placed: Cell<bool>,
    pub diff: DiffState,
    pub seen: RefCell<std::collections::BTreeSet<String>>,
    /// Threads folded or unfolded by hand (`space`), by id.
    pub flipped: RefCell<std::collections::BTreeSet<String>>,
    /// A thread to put the diff's cursor on once its file's diff is drawn.
    pub jump: RefCell<Option<String>>,
    /// What the Overview's text links to (a ticket, a merge request, a
    /// thread, an address), as last drawn: `⏎` offers them.
    pub links: RefCell<Vec<radar_core::Subject>>,
    /// The chips of the line under the title that act on a click, as last
    /// drawn: the part that added them, the subject, the action's id.
    pub under: RefCell<Vec<(&'static str, radar_core::Subject, String)>>,
    /// Numeric file/thread and line hits refer to a payload actually painted.
    pub detail_painted: Cell<bool>,
    pub diff_painted: Cell<bool>,
    pub log_painted: Cell<bool>,
}

impl Page {
    fn new(kind: Kind) -> Self {
        Self {
            kind,
            tab: 0,
            scroll: Scroll::new(),
            row: Cell::new(0),
            extra: RefCell::new(Vec::new()),
            log: LogState::new(),
            lead: Scroll::new(),
            file: Cell::new(0),
            tree: RefCell::default(),
            tree_seen: RefCell::default(),
            tree_shown: Cell::new(false),
            split_shown: Cell::new(false),
            placed: Cell::new(false),
            diff: DiffState::new(),
            seen: RefCell::default(),
            flipped: RefCell::default(),
            jump: RefCell::default(),
            links: RefCell::default(),
            under: RefCell::default(),
            detail_painted: Cell::new(false),
            diff_painted: Cell::new(false),
            log_painted: Cell::new(false),
        }
    }

    /// The contributed tab showing, if one is.
    fn contributed(&self) -> Option<std::cell::Ref<'_, (String, Vec<Row>)>> {
        let i = self.tab.checked_sub(own_tabs(&self.kind))?;
        std::cell::Ref::filter_map(self.extra.borrow(), |e| e.get(i)).ok()
    }
}

/// Only dedicated MR reads and their retained remote-derived page buffers.
/// Source lists, other caches, page identity and authored/local state stay held.
pub(crate) fn invalidate(p: &mut GitlabPlugin) {
    p.mr_pages.clear();
    p.mr_page_reads.clear();
    p.diffs.clear();
    for page in &mut p.pages {
        if !matches!(page.kind, Kind::Mr { .. }) { continue; }
        page.extra.get_mut().clear();
        page.links.get_mut().clear();
        page.under.get_mut().clear();
        if page.tab >= own_tabs(&page.kind) {
            page.tab = own_tabs(&page.kind) - 1;
            page.row.set(0);
            page.scroll = Scroll::new();
        } else if page.tab == mr::THREADS {
            page.row.set(0);
            page.scroll = Scroll::new();
        }
        page.file.set(0);
        page.placed.set(false);
        *page.tree.get_mut() = Default::default();
        page.tree_seen.get_mut().clear();
        page.tree_shown.set(false);
        page.split_shown.set(false);
        // reset() keeps painted text; replace only this remote diff buffer.
        page.diff = DiffState::new();
        page.seen.get_mut().clear();
        page.flipped.get_mut().clear();
        *page.jump.get_mut() = None;
        page.detail_painted.set(false);
        page.diff_painted.set(false);
    }
}

/// Real connection changes retire transient diagnosis/log state and only the
/// Job pages' derived remote buffers. Their identity and authored query survive.
pub(crate) fn invalidate_logs(p: &mut GitlabPlugin) {
    p.failed.clear();
    p.failed_reads.clear();
    why::reset(p);
    p.jobs.clear();
    p.logs.clear();
    p.log_reads.clear();
    for page in &mut p.pages {
        if !matches!(page.kind, Kind::Job { .. }) {
            continue;
        }
        let query = page.log.query();
        page.log = LogState::new();
        page.log.search(&Default::default(), &query);
        page.log_painted.set(false);
        page.extra.get_mut().clear();
        page.links.get_mut().clear();
        page.under.get_mut().clear();
        page.tab = 0;
        page.row.set(0);
        page.scroll = Scroll::new();
        page.lead = Scroll::new();
    }
}

/// Accepted replacements invalidate coordinate hits until their fresh render.
/// Keep navigation and user text; this is not another request-generation fence.
pub(crate) fn unpaint(p: &GitlabPlugin, key: &str, detail: bool) {
    for page in &p.pages {
        if matches!(&page.kind, Kind::Mr { project, iid } if mr_key(project, *iid) == key) {
            if detail { page.detail_painted.set(false); }
            page.diff_painted.set(false);
        }
    }
}

/// Something fetched for a page: the last value, whether a fetch is under
/// way, and the last error. The value stays while a refresh runs.
#[derive(Debug)]
pub(crate) struct Fetched<T> {
    pub value: Option<T>,
    pub loading: bool,
    pub error: Option<String>,
    /// When the value arrived.
    pub at: i64,
}

impl<T> Default for Fetched<T> {
    fn default() -> Self {
        Self {
            value: None,
            loading: false,
            error: None,
            at: 0,
        }
    }
}

pub(crate) fn mr_key(project: &str, iid: u64) -> String {
    format!("{project}!{iid}")
}

// --- opening and closing ---------------------------------------------------------

/// `⏎` in the tree: open the selected row's page.
pub(crate) fn open_from_tree(p: &mut GitlabPlugin) -> Vec<Effect> {
    let kind = match screen::selected(p, p.visible()) {
        Some(Selected::Tenancy(i)) => Kind::Tenancy(p.cfg.sources[i].group.clone()),
        Some(Selected::Repo(pr, _)) => Kind::Repo(pr.full_path.clone()),
        Some(Selected::Mr(it)) => Kind::Mr {
            project: it.mr.project_path.clone(),
            iid: it.mr.iid,
        },
        None => return vec![],
    };
    p.pages.clear();
    let mut fx = vec![Effect::Focus(Pane::Middle)];
    fx.extend(push(p, kind));
    fx
}

/// A merge request's page, on its Pipeline tab: why it is red.
pub(crate) fn open_mr_pipeline(p: &mut GitlabPlugin, project: &str, iid: u64) -> Vec<Effect> {
    p.pages.clear();
    let kind = Kind::Mr { project: project.to_string(), iid };
    let mut fx = vec![Effect::Focus(Pane::Middle)];
    fx.extend(push(p, kind));
    if let Some(page) = p.pages.last_mut() {
        page.tab = mr::PIPELINE;
    }
    fx
}

pub(crate) fn push(p: &mut GitlabPlugin, kind: Kind) -> Vec<Effect> {
    let mut page = Page::new(kind.clone());
    match &kind {
        Kind::Job { project, id } => job::opened(p, &page, project, *id),
        // Main is red: open on its pipelines, why it is red first.
        Kind::Repo(path) => {
            let pr = project(p, path);
            if why::red(pr.main_pipeline, pr.main_pipeline_url.as_deref()).is_some() {
                page.tab = repo::PIPELINES;
            }
        }
        _ => {}
    }
    p.pages.push(page);
    let mut effects = vec![crate::page_history::opened(p, &kind)];
    effects.extend(fetch(p, &kind, false));
    effects
}

/// Fetch what the page needs, unless it is fresh (or `force`).
pub(crate) fn fetch(p: &mut GitlabPlugin, kind: &Kind, force: bool) -> Vec<Effect> {
    let (now, every) = (p.now, p.cfg.refresh_secs as i64);
    let fresh = |at: i64, loading: bool, has: bool| loading || (has && now - at < every);
    let url = p.cfg.url.clone();
    let provider = p.provider();
    match kind {
        Kind::Tenancy(_) => vec![],
        Kind::Job { project, id } => why::open_log(p, project, *id, force),
        Kind::Mr { project, iid } => {
            // A red pipeline: why, beside the page's own data.
            let red = find_mr(p, project, *iid)
                .and_then(|m| why::red(m.pipeline, m.pipeline_url.as_deref()));
            let mut fx = match red {
                Some(pl) => why::diagnose(p, project, pl, true, force),
                None => vec![],
            };
            let key = mr_key(project, *iid);
            let held = p.mr_pages.get(&key).is_some_and(|f| match f.value.as_ref() {
                Some((held, _)) => match listed_mr(p, project, *iid) {
                    // The sync shows what the page was made from: no request.
                    Some(listed) if finished(held) => unchanged(listed, held),
                    // Nothing to compare with, or still running: the timer.
                    _ => fresh(f.at, f.loading, true),
                },
                None => f.loading,
            });
            if !force && held {
                return fx;
            }
            let f = p.mr_pages.entry(key.clone()).or_default();
            f.loading = true;
            let (project, iid) = (project.clone(), *iid);
            let action = crate::page_history::action(&url, &project, iid);
            let requested = crate::page_identity::Requested::new(&project, iid);
            let read = crate::page_ownership::next(p);
            p.mr_page_reads.insert(key.clone(), read);
            fx.push(crate::page_history::spawn(
                action, key, read, requested, async move {
                    provider.run(|c| {
                        let project = project.clone();
                        async move { c.mr_detail_observed(&project, iid).await }
                    })
                    .await
                    .map_err(|e| format!("{e:#}"))
                },
            ));
            fx
        }
        Kind::Repo(path) => {
            let pr = project(p, path);
            let mut fx = match why::red(pr.main_pipeline, pr.main_pipeline_url.as_deref()) {
                Some(pl) => why::diagnose(p, path, pl, true, force),
                None => vec![],
            };
            let branch = branch_of(&pr);
            crate::runs_ownership::reconcile(p);
            if !force && crate::runs_ownership::cached(p, path)
                .is_some_and(|f| fresh(f.at, f.loading, f.value.is_some())) {
                return fx;
            }
            let request = crate::runs_ownership::begin(p, path, &branch);
            let path = path.clone();
            let action = crate::runs_history::action(&url, &path, &branch);
            fx.push(crate::runs_history::spawn(action, request, async move {
                provider.run(|c| {
                    let (path, branch) = (path.clone(), branch.clone());
                    async move { c.runs_observed(&path, &branch).await }
                })
                .await
                .map_err(|e| format!("{e:#}"))
            }));
            fx
        }
    }
}

/// A merge request page's data arrived. The merge request itself replaces
/// the list's copy, so a page's refresh updates the tree too; a pipeline
/// that turned red is diagnosed.
pub(crate) fn on_mr_page(
    p: &mut GitlabPlugin,
    key: String,
    result: Result<(Mr, MrDetail), String>,
) -> Vec<Effect> {
    // Reply handling must never recreate an invalidated cache.
    if !p.mr_pages.contains_key(&key) { return vec![]; }
    if let Ok((_, detail)) = &result {
        let head = detail.refs.as_ref().map_or("", |refs| refs.head.as_str());
        if p.diffs.get(&key).is_some_and(|diffs| diffs.head != head) {
            // Retire even a hidden MR's old head and all pending diff owners.
            p.diffs.remove(&key);
        }
        unpaint(p, &key, true);
    }
    let f = p.mr_pages.get_mut(&key).expect("existing owned detail cache");
    f.loading = false;
    let red = result
        .as_ref()
        .ok()
        .and_then(|(m, _)| Some((m.project_path.clone(), why::red(m.pipeline, m.pipeline_url.as_deref())?)));
    match result {
        Ok((mr, detail)) => {
            for data in p.data.values_mut() {
                if let Some(old) = data
                    .mrs
                    .iter_mut()
                    .find(|m| m.project_path == mr.project_path && m.iid == mr.iid)
                {
                    *old = mr.clone();
                }
            }
            f.value = Some((mr, detail));
            f.error = None;
            f.at = p.now;
        }
        Err(e) => f.error = Some(e),
    }
    let mut fx = match red {
        Some((project, pl)) => why::diagnose(p, &project, pl, true, false),
        None => vec![],
    };
    // The Changes tab waited for the files.
    fx.extend(changes::want(p));
    fx
}

pub(crate) fn on_runs(
    p: &mut GitlabPlugin,
    path: String,
    result: Result<Vec<PipelineRun>, String>,
) {
    let Some(f) = p.runs.get_mut(&path) else { return; };
    f.loading = false;
    match result {
        Ok(runs) => {
            f.value = Some(runs);
            f.error = None;
            f.at = p.now;
        }
        Err(e) => f.error = Some(e),
    }
}

/// True while anything a page shows is being fetched.
pub(crate) fn loading(p: &GitlabPlugin) -> bool {
    p.mr_pages.values().any(|f| f.loading)
        || p.runs.values().any(|f| f.loading)
        || why::loading(p)
        || changes::loading(p)
}

// --- what a page is about --------------------------------------------------------

pub(crate) fn find_mr<'a>(p: &'a GitlabPlugin, project: &str, iid: u64) -> Option<&'a Mr> {
    p.data
        .values()
        .flat_map(|d| d.mrs.iter())
        .find(|m| m.project_path == project && m.iid == iid)
        .or_else(|| {
            p.mr_pages
                .get(&mr_key(project, iid))
                .and_then(|f| f.value.as_ref())
                .map(|(m, _)| m)
        })
}

/// The merge request as the last sync listed it (not the page's own copy).
fn listed_mr<'a>(p: &'a GitlabPlugin, project: &str, iid: u64) -> Option<&'a Mr> {
    p.data
        .values()
        .flat_map(|d| d.mrs.iter())
        .find(|m| m.project_path == project && m.iid == iid)
}

/// The head pipeline is not still running, so the page's jobs and stages
/// change only when the merge request does.
fn finished(mr: &Mr) -> bool {
    !matches!(mr.pipeline, Some(Pipeline::Running | Pipeline::Pending))
}

/// The page's data was fetched for the merge request as listed now: same
/// `updatedAt` (a push, a comment, an approval) and same head pipeline.
fn unchanged(listed: &Mr, held: &Mr) -> bool {
    listed.updated_at == held.updated_at
        && listed.pipeline == held.pipeline
        && listed.pipeline_url == held.pipeline_url
}

/// A repo as the last sync saw it, or a stand-in named after its path.
pub(crate) fn project(p: &GitlabPlugin, path: &str) -> Project {
    p.data
        .values()
        .flat_map(|d| d.projects.iter())
        .find(|pr| pr.full_path == path)
        .cloned()
        .unwrap_or_else(|| Project {
            name: path.rsplit('/').next().unwrap_or(path).to_string(),
            full_path: path.to_string(),
            ..Default::default()
        })
}

pub(crate) fn branch_of(pr: &Project) -> String {
    if pr.default_branch.is_empty() {
        "main".into()
    } else {
        pr.default_branch.clone()
    }
}

/// The cached pipeline belongs to the default branch, whose name may be unknown.
pub(crate) fn latest_pipeline_action(pr: &Project) -> String {
    if pr.default_branch.is_empty() {
        "open the default branch's latest pipeline".into()
    } else {
        format!("open {}'s latest pipeline", pr.default_branch)
    }
}

fn source<'a>(p: &'a GitlabPlugin, group: &str) -> Option<&'a Source> {
    p.cfg.sources.iter().find(|s| s.group == group)
}

/// The tenancy a repo belongs to.
fn tenancy_of<'a>(p: &'a GitlabPlugin, path: &str) -> Option<&'a Source> {
    p.cfg.source_for(path).map(|(s, _)| s)
}

/// What a page is called: its actions menu's title and its last crumb.
fn name(p: &GitlabPlugin, kind: &Kind) -> String {
    match kind {
        Kind::Tenancy(g) => source(p, g).map_or_else(|| g.clone(), |s| s.code.clone()),
        Kind::Repo(path) => project(p, path).name,
        Kind::Mr { project: path, iid } => format!("{} !{iid}", project(p, path).name),
        Kind::Job { project, id } => job::name(p, project, *id),
    }
}

// --- shared looks ------------------------------------------------------------------

/// A stage's or job's status word (GitLab's, lower case) as a mark.
pub(crate) fn status_mark(status: &str) -> Mark {
    Pipeline::from_graphql(&status.to_ascii_uppercase()).map_or(Mark::Waiting, Pipeline::mark)
}

// --- keys and clicks -----------------------------------------------------------------

/// The page's own tabs.
pub(crate) fn own_tabs(kind: &Kind) -> usize {
    match kind {
        Kind::Tenancy(_) => 1,
        Kind::Repo(_) => 2,
        Kind::Mr { .. } => mr::TABS.len(),
        Kind::Job { .. } => 1,
    }
}

/// Its own tabs and the ones other parts added.
fn tab_count(page: &Page) -> usize {
    own_tabs(&page.kind) + page.extra.borrow().len()
}

/// Every tab's label: the page's own, then the contributed ones.
fn tab_labels(p: &GitlabPlugin, page: &Page) -> Vec<String> {
    let mut labels = match &page.kind {
        Kind::Tenancy(_) => tenancy::tabs(),
        Kind::Repo(path) => repo::tabs(p, path),
        Kind::Mr { .. } => mr::tabs(),
        Kind::Job { .. } => job::tabs(),
    };
    labels.extend(page.extra.borrow().iter().map(|(title, _)| title.clone()));
    labels
}

/// The page's tabs, for the host to move across (GUIDE 1.5).
pub(crate) fn tabs(p: &GitlabPlugin) -> Option<radar_core::plugin::PageTabs> {
    let page = p.pages.last()?;
    let names = tab_labels(p, page);
    let on = page.tab.min(names.len().saturating_sub(1));
    Some(radar_core::plugin::PageTabs { names, on })
}

/// Show tab `i` from its top, on the page's main pane (the diff, on the
/// Changes tab). The host asks on `← → h l`, the page on `[` `]`.
pub(crate) fn show_tab(p: &mut GitlabPlugin, i: usize) -> Vec<Effect> {
    if let Some(page) = p.pages.last_mut() {
        set_tab(page, i);
    }
    vec![Effect::Focus(Pane::Middle)]
}

fn set_tab(page: &mut Page, i: usize) {
    let i = i.min(tab_count(page).saturating_sub(1));
    if i != page.tab {
        page.tab = i;
        page.log_painted.set(false);
        page.scroll.top();
        page.row.set(0);
    }
}

/// The page's panes: the Changes tab's files beside its diff; else one.
pub(crate) fn panes(p: &GitlabPlugin) -> Vec<Pane> {
    match p.pages.last() {
        Some(page) if changes::on_changes(page).is_some() && page.tree_shown.get() => {
            vec![Pane::Nav, Pane::Middle]
        }
        _ => vec![Pane::Middle],
    }
}

/// How many rows the page's current tab lists, or `None` for a tab of text.
fn list_len(p: &GitlabPlugin, page: &Page) -> Option<usize> {
    if let Some(tab) = page.contributed() {
        let rows = &tab.1;
        return rows
            .iter()
            .any(|r| r.subject.is_some())
            .then_some(rows.len());
    }
    match &page.kind {
        Kind::Tenancy(g) => Some(tenancy::repos(p, g).len()),
        Kind::Repo(path) => Some(repo::len(p, path, page.tab)),
        Kind::Mr { project, iid } if page.tab == mr::PIPELINE => {
            let n = mr::job_rows(p, project, *iid).len();
            (n > 0).then_some(n)
        }
        Kind::Mr { project, iid } if page.tab == mr::THREADS => review::len(p, project, *iid),
        Kind::Mr { .. } | Kind::Job { .. } => None,
    }
}

/// What `o`, `p` and `y` act on here.
fn link(p: &GitlabPlugin, page: &Page, pipeline: bool) -> Option<String> {
    let base = p.cfg.url.trim_end_matches('/');
    match &page.kind {
        Kind::Tenancy(g) => {
            let s = source(p, g)?;
            (!pipeline).then(|| format!("{base}/{}", p.cfg.group_path(s)))
        }
        Kind::Repo(path) => {
            if !pipeline {
                return Some(format!("{base}/{path}"));
            }
            if page.tab == repo::PIPELINES {
                return repo::run(p, path, page.row.get()).and_then(|r| r.url.clone());
            }
            project(p, path).main_pipeline_url
        }
        Kind::Mr { project, iid } => {
            let mr = find_mr(p, project, *iid);
            if pipeline {
                mr.and_then(|m| m.pipeline_url.clone())
            } else {
                Some(
                    mr.map(|m| m.web_url.clone())
                        .unwrap_or_else(|| format!("{base}/{project}/-/merge_requests/{iid}")),
                )
            }
        }
        Kind::Job { project, id } => {
            let info = job::info(p, project, *id);
            if pipeline {
                info.filter(|j| j.pipeline > 0)
                    .map(|j| format!("{base}/{project}/-/pipelines/{}", j.pipeline))
            } else {
                Some(
                    info.map(|j| j.url.clone())
                        .filter(|u| !u.is_empty())
                        .unwrap_or_else(|| format!("{base}/{project}/-/jobs/{id}")),
                )
            }
        }
    }
}

/// On another part's tab, the row's subject's label and link: what `o` opens
/// and `y` copies there, rather than the page's own.
fn row_link(page: &Page) -> Option<(String, String)> {
    let tab = page.contributed()?;
    let s = tab.1.get(page.row.get())?.subject.as_ref()?;
    Some((s.label.clone(), s.url.clone()?))
}

/// What `⏎` does on the current row, if anything: a page to open, a link,
/// or (on another part's tab) its row's subject to show.
enum Enter {
    Page(Kind),
    Link(String),
    Show(Subject),
}

fn enter(p: &GitlabPlugin, page: &Page) -> Option<Enter> {
    let row = page.row.get();
    if let Some(tab) = page.contributed() {
        return tab.1.get(row)?.subject.clone().map(Enter::Show);
    }
    match &page.kind {
        Kind::Tenancy(g) => tenancy::repos(p, g)
            .get(row)
            .map(|r| Enter::Page(Kind::Repo(r.project.full_path.clone()))),
        Kind::Repo(path) if page.tab == repo::MRS => repo::mrs(p, path).get(row).map(|it| {
            Enter::Page(Kind::Mr {
                project: it.mr.project_path.clone(),
                iid: it.mr.iid,
            })
        }),
        Kind::Repo(path) => {
            let failed = repo::lead_rows(p, path);
            if let Some(j) = failed.get(row) {
                return Some(Enter::Page(Kind::Job {
                    project: path.clone(),
                    id: j.id,
                }));
            }
            repo::run(p, path, row - failed.len())
                .and_then(|r| r.url.clone())
                .map(Enter::Link)
        }
        Kind::Mr { project, iid } if page.tab == mr::PIPELINE => mr::job_rows(p, project, *iid)
            .get(row)
            .and_then(mr::JobRow::id)
            .map(|id| {
                Enter::Page(Kind::Job {
                    project: project.clone(),
                    id,
                })
            }),
        Kind::Mr { .. } | Kind::Job { .. } => None,
    }
}

pub(crate) fn key(p: &mut GitlabPlugin, pane: Pane, key: Chord) -> Outcome {
    let out = page_key(p, pane, key);
    // A tab or file newly showing asks for its diff.
    match out {
        Outcome::Handled(mut fx) => {
            fx.extend(changes::want(p));
            Outcome::Handled(fx)
        }
        Outcome::Ignored => Outcome::Ignored,
    }
}

fn page_key(p: &mut GitlabPlugin, pane: Pane, key: Chord) -> Outcome {
    let Some(page) = p.pages.last() else {
        return Outcome::Ignored;
    };
    let kind = page.kind.clone();
    if let Some((project, iid)) = changes::on_changes(page)
        && let Some(out) = changes::key(p, &project, iid, pane, &key)
    {
        return out;
    }
    if let Kind::Job { project, id } = &kind
        && let Some(out) = job::key(p, project, *id, key)
    {
        return out;
    }
    match key.key {
        Key::Esc => {
            p.pages.pop();
            let fx = if p.pages.is_empty() {
                vec![Effect::Focus(Pane::Middle)]
            } else {
                vec![]
            };
            Outcome::Handled(fx)
        }
        Key::Char(c @ ('[' | ']')) => {
            let page = p.pages.last().expect("checked");
            let n = tab_count(page);
            let to = if c == ']' { (page.tab + 1) % n } else { (page.tab + n - 1) % n };
            Outcome::Handled(show_tab(p, to))
        }
        Key::Char('r') => {
            let mut fx = fetch(p, &kind, true);
            if matches!(kind, Kind::Repo(_) | Kind::Tenancy(_)) {
                // The repo's and tenancy's merge requests and main pipelines
                // come from the tenancy sync.
                fx.extend(p.refresh());
            }
            if fx.is_empty() {
                fx.push(Effect::Notify("already refreshing".into()));
            }
            Outcome::Handled(fx)
        }
        Key::Char(c @ ('o' | 'p')) => {
            let page = p.pages.last().expect("checked");
            let row = (c == 'o').then(|| row_link(page)).flatten();
            Outcome::Handled(
                row.map(|(_, url)| url)
                    .or_else(|| link(p, page, c == 'p'))
                    .map(Effect::Open)
                    .into_iter()
                    .collect(),
            )
        }
        Key::Char('y') => {
            let page = p.pages.last().expect("checked");
            let row = row_link(page).map(|(_, url)| url);
            Outcome::Handled(match row.or_else(|| link(p, page, false)) {
                Some(url) => vec![Effect::Copy(url), Effect::Notify("link copied".into())],
                None => vec![],
            })
        }
        Key::Enter => follow(p, &kind),
        _ => {
            if let Key::Char(c) = key.key
                && let Some(out) = review::key(p, c).or_else(|| act::key(p, c, &kind))
            {
                return out;
            }
            let page = p.pages.last().expect("checked");
            if move_in(p, page, &key) {
                Outcome::done()
            } else {
                Outcome::Ignored
            }
        }
    }
}

/// ⏎ (or a click on a selected row) on the open page: follow what the row
/// leads to.
fn follow(p: &mut GitlabPlugin, kind: &Kind) -> Outcome {
        let page = p.pages.last().expect("checked");
        // The overview is text: ⏎ follows one of its links.
        if let Kind::Mr { iid, .. } = kind
            && page.tab == mr::OVERVIEW
            && page.contributed().is_none()
            && !page.links.borrow().is_empty()
        {
            return Outcome::Handled(act::links_menu(p, *iid));
        }
        // A thread on the diff: the Changes tab, at its line.
        if let Kind::Mr { project, iid } = kind
            && page.tab == mr::THREADS
            && page.contributed().is_none()
            && review::enter(p, page, project, *iid)
        {
            let page = p.pages.last_mut().expect("checked");
            page.tab = mr::CHANGES;
            return Outcome::done();
        }
        match enter(p, page) {
            Some(Enter::Page(kind)) => {
                seed_job(p, &kind);
                Outcome::Handled(push(p, kind))
            }
            Some(Enter::Link(url)) => Outcome::Handled(vec![Effect::Open(url)]),
            Some(Enter::Show(s)) => Outcome::Handled(vec![Effect::Show(s)]),
            None => Outcome::done(),
        }
}

/// Activate the selected row of the open page in `pane`: the Changes tab's
/// files and diff lines have their own, every other row is followed.
pub(crate) fn activate(p: &mut GitlabPlugin, pane: Pane) -> Outcome {
    let Some(page) = p.pages.last() else {
        return Outcome::Ignored;
    };
    let kind = page.kind.clone();
    let own = changes::on_changes(page)
        .and_then(|(project, iid)| changes::activate(p, &project, iid, pane));
    let out = own.unwrap_or_else(|| follow(p, &kind));
    match out {
        Outcome::Handled(mut fx) => {
            fx.extend(changes::want(p));
            Outcome::Handled(fx)
        }
        Outcome::Ignored => Outcome::Ignored,
    }
}

/// A job opened from a merge request's list of jobs: what that list knows
/// of it stands in for its title until GitLab's answer arrives.
fn seed_job(p: &mut GitlabPlugin, kind: &Kind) {
    let (Kind::Job { project, id }, Some(page)) = (kind, p.pages.last()) else {
        return;
    };
    if p.jobs.contains_key(&JobKey::new(project, *id)) {
        return;
    }
    let Kind::Mr { iid, .. } = &page.kind else {
        return;
    };
    if let Some(info) = mr::job_rows(p, project, *iid)
        .iter()
        .find(|r| r.id() == Some(*id))
        .and_then(|r| r.info(project))
    {
        log_ownership::seed(p, JobKey::new(project, *id), info);
    }
}

/// `↑↓ j k`, PgUp/PgDn, `g G`: the cursor on a list, the view on text.
fn move_in(p: &GitlabPlugin, page: &Page, key: &Chord) -> bool {
    let Some(n) = list_len(p, page) else {
        return page.scroll.key(key);
    };
    let step = page.scroll.height().saturating_sub(1).max(1) as usize;
    let last = n.saturating_sub(1);
    let at = page.row.get();
    let to = match key.key {
        Key::Char('j') | Key::Down => (at + 1).min(last),
        Key::Char('k') | Key::Up => at.saturating_sub(1),
        Key::PageDown => (at + step).min(last),
        Key::PageUp => at.saturating_sub(step),
        Key::Char('g') | Key::Home => 0,
        Key::Char('G') | Key::End => last,
        _ => return false,
    };
    page.row.set(to);
    // PgUp/PgDn that move nothing are not handled: the host says so; nor
    // is ↑ at the top: the host takes it up to the page's tab row.
    to != at || !matches!(key.key, Key::PageDown | Key::PageUp | Key::Up | Key::Char('k'))
}

pub(crate) fn click(p: &mut GitlabPlugin, id: &str) -> Outcome {
    if let Some(mut fx) = changes::click(p, id) {
        fx.extend(changes::want(p));
        return Outcome::Handled(fx);
    }
    let Some(page) = p.pages.last_mut() else {
        return Outcome::Ignored;
    };
    // A chip another part put under the title: its action, by that part.
    if let Some(i) = id.strip_prefix("under:").and_then(|n| n.parse::<usize>().ok()) {
        let act = page.under.borrow().get(i).cloned();
        return Outcome::Handled(
            act.map(|(part, subject, id)| radar_core::Effect::Act { part, subject, id })
                .into_iter()
                .collect(),
        );
    }
    if let Some(i) = id
        .strip_prefix("tab:")
        .and_then(|n| n.parse::<usize>().ok())
    {
        set_tab(page, i);
    } else if let Some(i) = id
        .strip_prefix("row:")
        .and_then(|n| n.parse::<usize>().ok())
    {
        if !matches!(page.kind, Kind::Mr { .. })
            || page.tab != mr::THREADS
            || page.detail_painted.get()
        {
            page.row.set(i);
        }
    } else if let (Some(n), Some(job)) = (
        id.strip_prefix("log:").and_then(|n| n.parse::<usize>().ok()),
        page.kind.job(),
    ) {
        let page = p.pages.last().expect("checked");
        job::click(p, page, &job.project, job.id, n);
    }
    Outcome::Handled(changes::want(p))
}

/// The page's actions menu (`x`): titled with the item, only its actions,
/// about the focused pane (the Changes tab's files or its diff).
pub(crate) fn keys(p: &GitlabPlugin, pane: Pane) -> Vec<crate::Row> {
    let Some(page) = p.pages.last() else {
        return vec![];
    };
    use crate::act;
    let ic = match &page.kind {
        Kind::Tenancy(_) => Icon::Group,
        Kind::Repo(_) => Icon::Repo,
        Kind::Mr { project, iid } => find_mr(p, project, *iid).map_or(Icon::Change, screen::mr_icon),
        Kind::Job { .. } => Icon::Job,
    };
    let mut b = vec![crate::head(Binding::heading(name(p, &page.kind)).icon(ic))];
    match enter(p, page) {
        Some(Enter::Page(Kind::Mr { iid, .. })) => {
            b.push(act("gitlab.page.open", Binding::new("⏎", format!("open !{iid}'s page"))))
        }
        Some(Enter::Page(Kind::Repo(path))) => b.push(act(
            "gitlab.page.open",
            Binding::new("⏎", format!("open {}'s page", project(p, &path).name)),
        )),
        Some(Enter::Page(Kind::Job { project, id })) => {
            let name = job::info(p, &project, id).map(|j| j.name.clone()).or_else(|| {
                let Kind::Mr { iid, .. } = &page.kind else { return None };
                mr::job_rows(p, &project, *iid)
                    .iter()
                    .find(|r| r.id() == Some(id))
                    .map(|r| r.name().to_string())
            });
            b.push(act(
                "gitlab.page.open",
                Binding::new("⏎", format!("open {}'s log", name.unwrap_or_else(|| format!("job #{id}")))),
            ))
        }
        Some(Enter::Link(_)) => b.push(act("gitlab.page.open", Binding::new("⏎", "open this pipeline in GitLab"))),
        Some(Enter::Show(s)) => b.push(act("gitlab.page.open", Binding::new("⏎", format!("open {}", s.label)))),
        _ => {
            let n = page.links.borrow().len();
            if matches!(page.kind, Kind::Mr { .. }) && page.tab == mr::OVERVIEW && page.contributed().is_none() && n > 0 {
                b.push(act(
                    "gitlab.page.follow_link",
                    Binding::new("⏎", format!("follow a link on this page… ({n} link{})", if n == 1 { "" } else { "s" })),
                ));
            }
        }
    }
    let (open, pipeline, copy) = match &page.kind {
        Kind::Tenancy(_) => (
            "open the group in GitLab",
            String::new(),
            "copy the group's link",
        ),
        Kind::Repo(_) if page.tab == repo::PIPELINES => (
            "open the repo in GitLab",
            "open this pipeline in GitLab".into(),
            "copy the repo's link",
        ),
        Kind::Repo(path) => (
            "open the repo in GitLab",
            latest_pipeline_action(&project(p, path)),
            "copy the repo's link",
        ),
        Kind::Mr { .. } => (
            "open it in GitLab",
            "open its pipeline".into(),
            "copy its link",
        ),
        Kind::Job { .. } => (
            "open the job in GitLab",
            "open its pipeline".into(),
            "copy its link",
        ),
    };
    match row_link(page) {
        Some((label, _)) => {
            b.push(act("gitlab.page.open_link", Binding::new("o", format!("open {label}'s link"))));
            if link(p, page, true).is_some() {
                b.push(act("gitlab.page.open_pipeline", Binding::new("p", pipeline)));
            }
            b.push(act("gitlab.page.copy_link", Binding::new("y", format!("copy {label}'s link"))));
        }
        None => {
            b.push(act("gitlab.page.open_link", Binding::new("o", open)));
            if link(p, page, true).is_some() {
                b.push(act("gitlab.page.open_pipeline", Binding::new("p", pipeline)));
            }
            b.push(act("gitlab.page.copy_link", Binding::new("y", copy)));
        }
    }
    b.push(act(
        "gitlab.page.refresh",
        Binding::new(
            "r",
            match &page.kind {
                Kind::Mr { .. } => "refresh this page",
                Kind::Job { .. } => "reread its log",
                _ => "refresh this page and its tenancy",
            },
        )
        .pane(),
    ));
    let mut own = act::bindings(p, page);
    if let Some((project, iid)) = changes::on_changes(page) {
        own.extend(changes::bindings(p, page, &project, iid, pane));
    }
    if let Some(id) = page.kind.job() {
        own.extend(job::bindings(p, page, &id.project, id.id));
    }
    b.extend(own);
    let n = tab_count(page);
    if n > 1 {
        let labels = tab_labels(p, page);
        let word = |i: usize| {
            labels[i % n]
                .split(" · ")
                .next()
                .unwrap_or_default()
                .to_string()
        };
        b.push(act(
            "gitlab.page.next_tab",
            Binding::new("]", format!("next tab: {}", word(page.tab + 1))).pane(),
        ));
        b.push(act(
            "gitlab.page.previous_tab",
            Binding::new("[", format!("previous tab: {}", word(page.tab + n - 1))).pane(),
        ));
    }
    let back = match p.pages.len() {
        0 | 1 => "the middle pane".to_string(),
        n => name(p, &p.pages[n - 2].kind),
    };
    match job::esc_words(page) {
        Some(words) => b.push(act("gitlab.page.back", Binding::new("esc", words))),
        None => b.push(act("gitlab.page.back", Binding::new("esc", format!("back to {back}")))),
    }
    b
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod action_labels_tests;

#[cfg(test)]
mod page_identity_tests {
    use super::*;

    #[test]
    fn page_identity_distinguishes_kind_project_and_item() {
        let project = "ankh-morpork/watch".to_string();
        let other = "lancre/watch".to_string();
        let kinds = [
            Kind::Tenancy(project.clone()),
            Kind::Repo(project.clone()),
            Kind::Mr { project: project.clone(), iid: 12 },
            Kind::Mr { project: project.clone(), iid: 13 },
            Kind::Mr { project: other.clone(), iid: 12 },
            Kind::Job { project: project.clone(), id: 12 },
            Kind::Job { project, id: 13 },
            Kind::Job { project: other, id: 12 },
        ];
        let keys: std::collections::BTreeSet<_> = kinds.iter().map(Kind::identity).collect();
        assert_eq!(keys.len(), kinds.len());
        assert_eq!(kinds[2].identity(), r#"["merge-request","ankh-morpork/watch",12]"#);
    }

    #[test]
    fn page_identity_ignores_tab_row_and_selected_file() {
        let mut page = Page::new(Kind::Mr { project: "ankh-morpork/watch".into(), iid: 12 });
        let identity = page.kind.identity();
        page.tab = mr::CHANGES;
        page.row.set(3);
        page.file.set(4);
        page.extra.borrow_mut().push(("Watch notes".into(), vec![]));
        assert_eq!(page.kind.identity(), identity);
    }
}
