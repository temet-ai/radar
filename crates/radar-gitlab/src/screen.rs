//! The repositories screen: a nav pane (views with counts, tenancies) │ a middle pane (a tree) of
//! tenancies → repos → merge requests (collapsible, the tree view) │ a
//! detail pane of whatever is selected: a merge request, a repo or a tenancy.

use std::collections::BTreeMap;

use radar_core::design::help_bar::KeyHint;
use radar_core::input::{Edit, TextInput};
use radar_core::plugin::{Binding, Effect, MenuItem, Outcome, Pane, Tone};
use radar_core::view::{Chord, Key};
use radar_core::ui::icon::{self, Icon};
use radar_core::{time, ui, widgets};

use crate::config::Source;
use crate::filter::Filter;
use crate::model::{Mr, MrState, Pipeline, Project, Role};
use crate::pages::status_mark;
use crate::{GitlabPlugin, Item, Row, act, head, join, why};

pub(crate) mod neutral;

// --- what the tree holds -----------------------------------------------------

/// A repo in the tree: the project as GitLab reports it (or a stand-in when
/// only its merge requests are known) and the view's merge requests in it.
struct RepoNode<'a> {
    project: Project,
    project_loaded: bool,
    mrs: Vec<Item<'a>>,
}

/// A tenancy in the tree, with its repos in display order.
struct TenancyNode<'a> {
    index: usize,
    source: &'a Source,
    repos: Vec<RepoNode<'a>>,
}

/// What the selection in the tree is. Short-lived (one per key or frame),
/// so the size of a repo's project does not matter.
#[allow(clippy::large_enum_variant)]
pub(crate) enum Selected<'a> {
    Tenancy(usize),
    Repo(Project, Vec<Item<'a>>),
    Mr(Item<'a>),
}

fn tenancy_id(s: &Source) -> String {
    format!("t:{}", s.group)
}
fn repo_id(path: &str) -> String {
    format!("r:{path}")
}
fn mr_id(path: &str, iid: u64) -> String {
    format!("m:{path}!{iid}")
}

/// The tree's content for the current view: every shown repo of every
/// tenancy (or only those with merge requests in the view, if quiet repos
/// are hidden), repos with merge requests first.
fn nodes<'a>(p: &'a GitlabPlugin, items: Vec<Item<'a>>) -> Vec<TenancyNode<'a>> {
    let mut by_repo: BTreeMap<String, Vec<Item<'a>>> = BTreeMap::new();
    for it in items {
        by_repo
            .entry(it.mr.project_path.clone())
            .or_default()
            .push(it);
    }
    let typed = Filter::parse(&p.filter);
    p.cfg
        .sources
        .iter()
        .enumerate()
        .map(|(index, source)| {
            let group = p.cfg.group_path(source);
            let mut projects: Vec<(Project, bool)> = p
                .data
                .get(&group)
                .map(|d| {
                    d.projects
                        .iter()
                        .filter(|pr| {
                            p.cfg
                                .source_for(&pr.full_path)
                                .is_some_and(|(s, _)| s.group == source.group)
                        })
                        .filter(|pr| !p.cfg.is_hidden(&pr.full_path))
                        .cloned()
                        .map(|project| (project, true))
                        .collect()
                })
                .unwrap_or_default();
            // Repos whose merge requests we have but whose project we do not.
            for path in by_repo.keys() {
                let ours = p
                    .cfg
                    .source_for(path)
                    .is_some_and(|(s, _)| s.group == source.group);
                if ours && !projects.iter().any(|(pr, _)| &pr.full_path == path) {
                    let name = path.rsplit('/').next().unwrap_or(path).to_string();
                    projects.push((
                        Project {
                            name,
                            full_path: path.clone(),
                            ..Default::default()
                        },
                        false,
                    ));
                }
            }
            let mut repos: Vec<RepoNode<'a>> = projects
                .into_iter()
                .map(|(project, project_loaded)| {
                    let mrs = by_repo.remove(&project.full_path).unwrap_or_default();
                    RepoNode { project, project_loaded, mrs }
                })
                .filter(|r| {
                    !r.mrs.is_empty() || quiet_shown(p, &typed, &r.project.name, &source.code)
                })
                .collect();
            let latest = |r: &RepoNode<'_>| {
                r.mrs
                    .iter()
                    .map(|it| it.mr.updated_at.clone())
                    .max()
                    .unwrap_or_else(|| r.project.last_activity.clone())
            };
            repos.sort_by(|a, b| {
                b.mrs
                    .is_empty()
                    .cmp(&a.mrs.is_empty())
                    .reverse()
                    .then_with(|| latest(b).cmp(&latest(a)))
            });
            TenancyNode {
                index,
                source,
                repos,
            }
        })
        // What you typed (`/`) or the author you picked narrows the tree: a
        // tenancy with nothing left in it is not part of the answer.
        .filter(|t| !narrowed(p) || !t.repos.is_empty())
        .collect()
}

/// Whether the tree shows only what answers your words or the author picked.
fn narrowed(p: &GitlabPlugin) -> bool {
    !p.filter.trim().is_empty() || p.author.is_some()
}

/// The first thing the filter found: a merge request, else a repo, else a
/// tenancy. Where ⏎ and ↓ from `/` land.
fn first_result(p: &GitlabPlugin) -> Option<Vec<String>> {
    let targets = targets(p);
    (1..=3)
        .rev()
        .find_map(|depth| targets.iter().find(|t| t.path.len() == depth))
        .map(|t| t.path.clone())
}

/// Leave `/` for the tree, on the first result (or, with `keep`, where the
/// selection was if it is still there).
fn to_results(p: &mut GitlabPlugin, keep: bool) -> Vec<Effect> {
    let before = p.tree.borrow().selected().to_vec();
    let still_there = targets(p).iter().any(|t| t.path == before);
    let mut fx = vec![Effect::Focus(Pane::Middle)];
    if keep && still_there {
        return fx;
    }
    if let Some(path) = first_result(p) {
        let mut state = p.tree.borrow_mut();
        for depth in 1..path.len() {
            state.open(path[..depth].to_vec());
        }
        state.select(path);
    }
    fx.extend(moved(p, &before));
    fx
}

pub(crate) fn selected<'a>(p: &'a GitlabPlugin, items: Vec<Item<'a>>) -> Option<Selected<'a>> {
    let path = p.tree.borrow().selected().to_vec();
    let last = path.last()?.clone();
    for t in nodes(p, items) {
        if last == tenancy_id(t.source) {
            return Some(Selected::Tenancy(t.index));
        }
        for r in t.repos {
            if last == repo_id(&r.project.full_path) {
                return Some(Selected::Repo(r.project, r.mrs));
            }
            if let Some(it) = r
                .mrs
                .into_iter()
                .find(|it| last == mr_id(&it.mr.project_path, it.mr.iid))
            {
                return Some(Selected::Mr(it));
            }
        }
    }
    None
}

/// Every row of the tree, in order, as go to (`:`) sees it.
pub(crate) fn targets(p: &GitlabPlugin) -> Vec<crate::goto::Target> {
    use crate::goto::Target;
    let mut out = Vec::new();
    for t in nodes(p, p.visible()) {
        let tid = tenancy_id(t.source);
        let name = t.source.group.rsplit('/').next().unwrap_or(&t.source.group);
        out.push(Target {
            path: vec![tid.clone()],
            name: format!("{} {name}", t.source.code),
            words: vec![
                t.source.code.clone(),
                name.to_string(),
                t.source.group.clone(),
            ],
        });
        for r in t.repos {
            let rid = repo_id(&r.project.full_path);
            out.push(Target {
                path: vec![tid.clone(), rid.clone()],
                name: r.project.name.clone(),
                words: vec![r.project.name.clone(), r.project.full_path.clone()],
            });
            for it in r.mrs {
                out.push(Target {
                    path: vec![
                        tid.clone(),
                        rid.clone(),
                        mr_id(&it.mr.project_path, it.mr.iid),
                    ],
                    name: format!("{} !{} {}", it.mr.project_name, it.mr.iid, it.mr.title),
                    words: vec![
                        format!("!{}", it.mr.iid),
                        it.mr.iid.to_string(),
                        it.mr.title.clone(),
                        it.mr.source_branch.clone(),
                    ],
                });
            }
        }
    }
    out
}

pub(crate) fn selected_mr<'a>(p: &'a GitlabPlugin, items: &[Item<'a>]) -> Option<Item<'a>> {
    match selected(
        p,
        items
            .iter()
            .map(|it| Item {
                mr: it.mr,
                role: it.role,
                tenant: it.tenant,
            })
            .collect(),
    )? {
        Selected::Mr(it) => Some(it),
        _ => None,
    }
}

// --- the plugin hooks ----------------------------------------------------------

/// Whether the current view shows only repos with a merge request in it.
/// Whether a repo with no merge request in view is listed: in a view that
/// lists every repo, unless what you typed (`/`) or the author you picked
/// narrows the tree, when it stays only if the words are about it.
fn quiet_shown(p: &GitlabPlugin, typed: &Filter, name: &str, tenant: &str) -> bool {
    !only_busy(p) && p.author.is_none() && typed.matches_repo(name, tenant)
}

fn only_busy(p: &GitlabPlugin) -> bool {
    p.current_view()
        .is_some_and(|v| v.repos == crate::config::RepoScope::WithMergeRequests)
}

/// How many repos the tree shows, for the title: what you are looking at.
fn repo_summary(nodes: &[TenancyNode<'_>], only_busy: bool) -> String {
    let n: usize = nodes.iter().map(|t| t.repos.len()).sum();
    let s = if n == 1 { "" } else { "s" };
    if only_busy {
        format!("{n} repo{s} with merge requests")
    } else {
        format!("{n} repo{s}")
    }
}

pub fn top_right(p: &GitlabPlugin) -> Option<String> {
    // While paused the host says so, once, in place of this.
    Some(ui::status::sync_words(
        "GitLab",
        p.sync.in_flight > 0,
        p.sync.last_ok,
        p.now,
        (p.sync.in_flight > 0 || p.sync.last_ok.is_some()).then_some(p.cfg.refresh_secs),
    ))
}

/// The merge requests waiting for your review: the tab's badge.
pub fn to_review(p: &GitlabPlugin) -> usize {
    p.all_roles().iter().filter(|r| **r == Role::Review).count()
}

pub fn status(p: &GitlabPlugin) -> Vec<(String, Tone)> {
    let roles = p.all_roles();
    let count = |rs: &[Role]| roles.iter().filter(|r| rs.contains(r)).count();
    let mut out = vec![(format!("{} open", roles.len()), Tone::Plain)];
    let need = count(&[Role::Review]);
    if need > 0 {
        out.push((format!("{need} to review"), Tone::Warn));
    }
    let broken = count(&[Role::Fix]);
    if broken > 0 {
        out.push((format!("{broken} broken"), Tone::Bad));
    }
    let ready = count(&[Role::Ready]);
    if ready > 0 {
        out.push((format!("{ready} ready"), Tone::Good));
    }
    if !p.sync.errors.is_empty() {
        out.push((format!("{} failed to sync", p.sync.errors.len()), Tone::Bad));
    }
    out
}

// --- the tree --------------------------------------------------------------------------

/// The tree's column of what stands between a merge request and merging.
pub(crate) const STATE: &str = "state";
/// The tree's column of what a merge request needs from you.
pub(crate) const FOR_YOU: &str = "for you";

/// A merge request's icon: its state, a draft apart.
pub(crate) fn mr_icon(mr: &Mr) -> Icon {
    match mr.state {
        MrState::Merged => Icon::Merged,
        MrState::Closed => Icon::Closed,
        MrState::Opened if mr.draft => Icon::Draft,
        MrState::Opened => Icon::Change,
    }
}

/// Where a comment lives, said under its author: the words, and the
/// address a click on them opens.
pub(crate) struct Whence<'a> {
    pub words: String,
    pub url: &'a str,
}

/// One comment as the detail pane and the item page draw it: the author's
/// initials on a chip, the name, its age (and where it sits), where it lives
/// as a link, then the body through the markdown renderer every description
/// uses, at most `most` lines. No bar down its edge: that is a quote's look.
#[allow(clippy::too_many_arguments)]
pub(crate) fn comment_blocks(
    p: &GitlabPlugin,
    project: &str,
    author: &str,
    at: &str,
    body: &str,
    place: Option<String>,
    whence: Option<Whence<'_>>,
    most: Option<usize>,
    now: i64,
) -> Vec<radar_core::view::Block> {
    use radar_core::view::{Block, Ink, Seg, Text};
    let tone = match person_icon(p, author) {
        Icon::You => Tone::Accent,
        Icon::Bot => Tone::Muted,
        _ => Tone::Info,
    };
    let mut head = Text::new();
    let initials = initials(author);
    if !initials.is_empty() {
        head.add(Seg::chip(&radar_core::compose::Chip::new(initials, tone)));
        head.add(Seg::new(" ", Ink::Muted));
    }
    head.add(Seg::new(author, Ink::Emphasis).strong());
    head.add(Seg::new(format!(" · {} ago", time::ago_str(at, now)), Ink::Muted));
    if let Some(place) = place {
        head.add(Seg::new(format!(" · on {place}"), Ink::Muted));
    }
    let mut doc = radar_core::ui::markdown::parse(body);
    crate::pictures::make_whole(&mut doc, &p.cfg.url, project);
    let mut blocks = vec![Block::line(head)];
    if let Some(w) = whence.filter(|w| !w.url.is_empty()) {
        blocks.push(Block::line(Text::from(vec![
            Seg::icon_or_lead(Icon::Link, "↗ ", Ink::AccentSecondary),
            Seg::new(w.words, Ink::AccentSecondary).linked(w.url),
        ])));
    }
    blocks.push(Block::Doc {
        doc,
        bar: None,
        most,
        urls: Some(false),
    });
    blocks
}

/// Two letters for a name: the first and last words' first letters, or a
/// one-word name's first two.
fn initials(name: &str) -> String {
    let words: Vec<&str> = name
        .split(|c: char| c.is_whitespace() || matches!(c, '.' | '_' | '-'))
        .filter(|w| !w.is_empty())
        .collect();
    let picked: String = match words.as_slice() {
        [] => String::new(),
        [one] => one.chars().take(2).collect(),
        [first, .., last] => first.chars().take(1).chain(last.chars().take(1)).collect(),
    };
    picked.to_uppercase()
}

/// A person's icon: you, a bot, or anyone.
pub(crate) fn person_icon(p: &GitlabPlugin, username: &str) -> Icon {
    if username == p.me {
        Icon::You
    } else if crate::model::is_bot(username) {
        Icon::Bot
    } else {
        Icon::Person
    }
}

/// A view's icon, from what its filter asks.
pub(crate) fn view_icon(filter: &str) -> Icon {
    let f = crate::filter::Filter::parse(filter);
    let has = |needle: &str| {
        filter
            .split_whitespace()
            .any(|w| !w.starts_with('-') && w.starts_with(needle))
    };
    if filter.trim().is_empty() {
        Icon::Change
    } else if has("is:review") {
        Icon::NeedsApproval
    } else if has("author:@me") {
        Icon::You
    } else if has("is:bot") {
        Icon::Bot
    } else if filter.split_whitespace().any(|w| w == "-author:@me") {
        Icon::Team
    } else if f.wants_ended() {
        Icon::Merged
    } else {
        Icon::Search
    }
}

/// The room the names keep beside the columns; columns go, least kept
/// first, until they have it.
const NAME: usize = 30;

/// The merge request a tree row's id names.
pub(crate) fn mr_of_row<'a>(p: &'a GitlabPlugin, id: &str) -> Option<&'a Mr> {
    let (path, iid) = id.strip_prefix("m:")?.rsplit_once('!')?;
    crate::pages::find_mr(p, path, iid.parse().ok()?)
}

// --- detail pane ------------------------------------------------------------------------

/// What the selection is called: the detail pane's title and the actions menu's.
/// The icon of what is selected in the tree.
fn sel_icon(sel: Option<&Selected<'_>>) -> Icon {
    match sel {
        Some(Selected::Mr(it)) => mr_icon(it.mr),
        Some(Selected::Repo(..)) => Icon::Repo,
        Some(Selected::Tenancy(_)) => Icon::Group,
        None => Icon::Menu,
    }
}

fn name_of(p: &GitlabPlugin, sel: Option<&Selected<'_>>) -> String {
    match sel {
        Some(Selected::Mr(it)) => format!("{} !{}", it.mr.project_name, it.mr.iid),
        Some(Selected::Repo(pr, _)) => pr.name.clone(),
        Some(Selected::Tenancy(i)) => p.cfg.sources[*i]
            .group
            .rsplit('/')
            .next()
            .unwrap_or("")
            .to_string(),
        None => "detail".into(),
    }
}

// --- keys and clicks ----------------------------------------------------------------

/// The URL `o`, `p` and `y` act on, for whatever is selected.
fn link(p: &GitlabPlugin, pipeline: bool) -> Option<String> {
    let base = p.cfg.url.trim_end_matches('/');
    match selected(p, p.visible())? {
        Selected::Mr(it) => {
            if pipeline {
                it.mr.pipeline_url.clone()
            } else {
                Some(it.mr.web_url.clone())
            }
        }
        Selected::Repo(pr, _) => {
            if pipeline {
                pr.main_pipeline_url
            } else {
                Some(format!("{base}/{}", pr.full_path))
            }
        }
        Selected::Tenancy(i) => {
            (!pipeline).then(|| format!("{base}/{}", p.cfg.group_path(&p.cfg.sources[i])))
        }
    }
}

/// After the selection moved: tell the bus which merge request it is on,
/// and when its pipeline (or a repo's main one) is red, ask which jobs
/// failed, for the detail pane to say, once the selection has rested on it.
pub(crate) fn moved(p: &mut GitlabPlugin, before: &[String]) -> Vec<Effect> {
    if p.tree.borrow().selected() == before {
        return vec![];
    }
    p.detail_scroll.top();
    let fx: Vec<Effect> = p
        .selected_payload()
        .map(|payload| Effect::Emit {
            topic: "gitlab.selected",
            payload,
        })
        .into_iter()
        .collect();
    let red = match selected(p, p.visible()) {
        Some(Selected::Mr(it)) => why::red(it.mr.pipeline, it.mr.pipeline_url.as_deref())
            .map(|pl| (it.mr.project_path.clone(), pl)),
        Some(Selected::Repo(pr, _)) => why::red(pr.main_pipeline, pr.main_pipeline_url.as_deref())
            .map(|pl| (pr.full_path.clone(), pl)),
        _ => None,
    };
    match red {
        Some((project, pl)) => why::rest_on(p, &project, pl),
        None => why::rest_off(p),
    }
    fx
}

/// `o` (the item) or `p` (its pipeline): open the selected row's link.
pub(crate) fn open_link(p: &GitlabPlugin, pipeline: bool) -> Outcome {
    Outcome::Handled(link(p, pipeline).map(Effect::Open).into_iter().collect())
}

/// `y`: copy the selected row's link.
pub(crate) fn copy_link(p: &GitlabPlugin) -> Outcome {
    Outcome::Handled(match link(p, false) {
        Some(url) => vec![Effect::Copy(url), Effect::Notify("link copied".into())],
        None => vec![],
    })
}

pub fn key(p: &mut GitlabPlugin, pane: Pane, key: Chord) -> Outcome {
    if let Some(mut input) = p.filter_input.take() {
        // ↓ keeps the words and goes down into what they found.
        if key.key == Key::Down {
            p.filter = input.value();
            return Outcome::Handled(to_results(p, false));
        }
        match input.key(key) {
            Edit::Cancel => p.filter.clear(),
            Edit::Submit(text) => {
                p.filter = text;
                return Outcome::Handled(to_results(p, true));
            }
            Edit::Typing => {
                p.filter = input.value();
                p.filter_input = Some(input);
            }
        }
        return Outcome::done();
    }
    match key.key {
        Key::Char('/') => {
            p.filter_input = Some(TextInput::new(&p.filter));
            return Outcome::done();
        }
        Key::Char('r') => return Outcome::Handled(p.refresh()),
        // The detail pane scrolls a page at a time from any pane, as
        // lazydocker's main panel does.
        // Not handled when there is nothing more to see: the host says so.
        Key::PageDown | Key::PageUp => {
            return if p.detail_scroll.page(key.key == Key::PageDown) {
                Outcome::done()
            } else {
                Outcome::Ignored
            };
        }
        Key::Char('.') => {
            // A setting of the view, saved with it (w).
            let Some(v) = p.cfg.views.get_mut(p.view) else {
                return Outcome::done();
            };
            v.repos = v.repos.flipped();
            return Outcome::done();
        }
        Key::Char('o') => return open_link(p, false),
        Key::Char('p') => return open_link(p, true),
        Key::Char('y') => return copy_link(p),
        _ => {}
    }
    // The selected merge request's rebase, merge, approve and check out.
    if let Some(out) = crate::list_actions::key(p, pane, &key) {
        return out;
    }
    match pane {
        Pane::Nav => {
            if let Some(down) = widgets::vertical(&key) {
                // ↑ on the first view is the host's: up to the top bar.
                if !down && p.view == 0 {
                    return Outcome::Ignored;
                }
                p.view = widgets::step(p.view, p.cfg.views.len(), down);
                return Outcome::done();
            }
            Outcome::Ignored
        }
        // In the detail pane, moving moves through what it shows: a line at a
        // time, or to the top and the end. The tree stays where it is.
        Pane::Detail => {
            if p.detail_scroll.key(&key) {
                Outcome::done()
            } else if key.key == Key::Enter {
                // As in Jira and Slack: ⏎ goes in, to the selected item's page.
                Outcome::Handled(crate::pages::open_from_tree(p))
            } else {
                Outcome::Ignored
            }
        }
        Pane::Middle => {
            // Moving and folding are the frontend's, as intents; ⏎ opens
            // the row's page.
            match key.key {
                Key::Enter => Outcome::Handled(crate::pages::open_from_tree(p)),
                Key::Esc if !p.filter.is_empty() => {
                    p.filter.clear();
                    Outcome::done()
                }
                _ => Outcome::Ignored,
            }
        }
    }
}

/// The status line while the filter is being typed.
pub fn hints(p: &GitlabPlugin) -> Vec<KeyHint<'static>> {
    if p.filter_input.is_some() {
        return vec![
            KeyHint::new("⏎", "keep filter"),
            KeyHint::new("esc", "clear"),
        ];
    }
    vec![]
}

/// Bulk commands (`b`): things done to everything the view shows.
pub fn bulk(p: &GitlabPlugin) -> Vec<MenuItem> {
    let items = p.visible();
    let n = items.len();
    let mut out = vec![
        MenuItem::new(
            'e',
            "open everything",
            "every tenancy and repo in the tree",
            "open",
        ),
        MenuItem::new('c', "close everything", "down to the tenancies", "close"),
    ];
    if n > 0 {
        let s = if n == 1 { "" } else { "s" };
        out.extend([
            MenuItem::new(
                'o',
                "open every merge request",
                format!("{n} in this view, in your browser"),
                "browse",
            ),
            MenuItem::new(
                'y',
                "copy every link",
                format!("{n} merge request{s}, one per line"),
                "copy",
            ),
        ]);
    }
    out.push(MenuItem::new(
        'r',
        "refresh everything",
        "every tenancy, now",
        "refresh",
    ));
    out
}

/// Run the bulk command `value`.
pub fn on_bulk(p: &mut GitlabPlugin, value: &str) -> Vec<Effect> {
    match value {
        "open" => {
            let nodes = nodes(p, p.visible());
            let mut state = p.tree.borrow_mut();
            for tn in &nodes {
                let tid = tenancy_id(tn.source);
                state.open(vec![tid.clone()]);
                for r in &tn.repos {
                    state.open(vec![tid.clone(), repo_id(&r.project.full_path)]);
                }
            }
            vec![]
        }
        "close" => {
            let mut state = p.tree.borrow_mut();
            let keep = state.selected().first().cloned();
            state.close_all();
            // The selection may have been inside something now closed.
            if let Some(t) = keep {
                state.select(vec![t]);
            }
            vec![]
        }
        "browse" => p
            .visible()
            .iter()
            .map(|it| Effect::Open(it.mr.web_url.clone()))
            .collect(),
        "copy" => {
            let links: Vec<String> = p.visible().iter().map(|it| it.mr.web_url.clone()).collect();
            let n = links.len();
            vec![
                Effect::Copy(links.join("\n")),
                Effect::Notify(format!("{n} links copied")),
            ]
        }
        "refresh" => p.refresh(),
        _ => vec![],
    }
}

/// The actions menu (`x`) for a pane: what can be done there to what is
/// selected, named after it. Each row's key does what the row says.
pub fn keys(p: &GitlabPlugin, pane: Pane) -> Vec<Row> {
    // The view's own actions: the nav pane and the tree act on the view.
    let view = |b: &mut Vec<Row>| {
        b.extend([
            act("gitlab.repos.filter", Binding::new("/", "filter the view by words")),
            // Says what pressing it will switch to.
            act(
                "gitlab.repos.toggle_empty_repos",
                Binding::new(
                    ".",
                    if only_busy(p) {
                        "show every repo in the view"
                    } else {
                        "show only repos with merge requests in the view"
                    },
                ),
            ),
            act("gitlab.repos.refresh", Binding::new("r", "refresh now")),
        ]);
    };
    let sel = selected(p, p.visible());
    // What o, p and y act on, said for this selection.
    let links = |b: &mut Vec<Row>| {
        let (open, pipeline, copy) = match &sel {
            Some(Selected::Mr(_)) => (
                "open it in GitLab",
                "open its pipeline".into(),
                "copy its link",
            ),
            Some(Selected::Repo(project, _)) => (
                "open the repo in GitLab",
                crate::pages::latest_pipeline_action(project),
                "copy the repo's link",
            ),
            Some(Selected::Tenancy(_)) => (
                "open the group in GitLab",
                String::new(),
                "copy the group's link",
            ),
            None => return,
        };
        b.push(act("gitlab.repos.open_link", Binding::new("o", open)));
        if link(p, true).is_some() {
            b.push(act("gitlab.repos.open_pipeline", Binding::new("p", pipeline)));
        }
        b.push(act("gitlab.repos.copy_link", Binding::new("y", copy)));
    };
    match pane {
        Pane::Nav => {
            let name = p.cfg.views.get(p.view).map_or("views", |v| v.name.as_str());
            let ic = p
                .cfg
                .views
                .get(p.view)
                .map_or(Icon::Views, |v| view_icon(&v.filter));
            let mut b = vec![
                head(Binding::heading(name).icon(ic)),
                act("gitlab.repos.open_view", Binding::new("⏎", "go to its repositories")),
            ];
            view(&mut b);
            b
        }
        Pane::Middle => {
            let mut b =
                vec![head(Binding::heading(name_of(p, sel.as_ref())).icon(sel_icon(sel.as_ref())))];
            let open = p.tree.borrow().is_open(p.tree.borrow().selected());
            if sel.is_some() {
                b.push(act("gitlab.repos.open", Binding::new("⏎", "open its page")));
            }
            match &sel {
                Some(Selected::Repo(_, mrs)) if !mrs.is_empty() => b.push(act(
                    "gitlab.repos.toggle_fold",
                    Binding::new(
                        "space",
                        if open {
                            "hide its merge requests"
                        } else {
                            "show its merge requests"
                        },
                    ),
                )),
                Some(Selected::Tenancy(_)) => b.push(act(
                    "gitlab.repos.toggle_fold",
                    Binding::new(
                        "space",
                        if open {
                            "hide its repos"
                        } else {
                            "show its repos"
                        },
                    ),
                )),
                _ => {}
            }
            links(&mut b);
            if let Some(Selected::Mr(it)) = &sel {
                b.extend(crate::list_actions::bindings(p, it.mr));
            }
            view(&mut b);
            b
        }
        Pane::Detail => {
            let mut b =
                vec![head(Binding::heading(name_of(p, sel.as_ref())).icon(sel_icon(sel.as_ref())))];
            if sel.is_some() {
                b.push(act("gitlab.repos.open", Binding::new("⏎", "open its page")));
            }
            links(&mut b);
            if let Some(Selected::Mr(it)) = &sel {
                b.extend(crate::list_actions::bindings(p, it.mr));
            }
            let (at, max) = (p.detail_scroll.at(), p.detail_scroll.max());
            if at < max {
                b.push(act("gitlab.repos.scroll_down", Binding::new("pgdn", "scroll down a page")));
            }
            if at > 0 {
                b.push(act("gitlab.repos.scroll_up", Binding::new("pgup", "scroll up a page")));
            }
            b.push(act("gitlab.repos.back", Binding::new("esc", "back to the middle pane")));
            b
        }
    }
}

/// Only offer a fold on a row that has children, in its current direction.
pub(crate) fn fold_keys(p: &GitlabPlugin) -> Vec<Row> {
    let path = p.tree.borrow().selected().to_vec();
    let has_children = match selected(p, p.visible()) {
        Some(Selected::Tenancy(index)) => nodes(p, p.visible())
            .iter()
            .any(|tenancy| tenancy.index == index && !tenancy.repos.is_empty()),
        Some(Selected::Repo(_, mrs)) => !mrs.is_empty(),
        Some(Selected::Mr(_)) | None => false,
    };
    if !has_children {
        return Vec::new();
    }
    if p.tree.borrow().is_open(&path) {
        vec![act("gitlab.repos.collapse", Binding::new("-", "collapse this group"))]
    } else {
        vec![act("gitlab.repos.expand", Binding::new("+", "expand this group"))]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::sample;

    fn press(p: &mut GitlabPlugin, pane: Pane, code: Key) -> Outcome {
        p.key("repos", pane, Chord::new(code))
    }

    /// ⏎ in the focused detail pane goes in, as it does in Jira and Slack.
    #[test]
    fn enter_in_the_focused_preview_opens_the_items_page() {
        let mut p = sample();
        draw(&p);
        press(&mut p, Pane::Middle, Key::Down);
        draw(&p);
        assert!(p.pages.is_empty());
        press(&mut p, Pane::Detail, Key::Enter);
        assert_eq!(p.pages.len(), 1, "its page is open");
    }

    #[test]
    fn a_view_gets_the_icon_of_what_its_filter_asks() {
        assert_eq!(view_icon(""), Icon::Change);
        assert_eq!(view_icon("is:review"), Icon::NeedsApproval);
        assert_eq!(view_icon("author:@me"), Icon::You);
        assert_eq!(view_icon("state:merged"), Icon::Merged);
        assert_eq!(view_icon("is:bot"), Icon::Bot);
        assert_eq!(view_icon("-author:@me -is:bot"), Icon::Team);
        assert_eq!(view_icon("label:backend"), Icon::Search);
    }

    #[test]
    fn a_merge_request_and_a_person_get_their_icons() {
        use crate::model::fixtures::mr;
        let p = sample();
        let mut m = mr(1, "alice", "acme/team-a/api");
        assert_eq!(mr_icon(&m), Icon::Change);
        m.draft = true;
        assert_eq!(mr_icon(&m), Icon::Draft);
        m.state = MrState::Merged;
        assert_eq!(mr_icon(&m), Icon::Merged);
        m.state = MrState::Closed;
        assert_eq!(mr_icon(&m), Icon::Closed);
        assert_eq!(person_icon(&p, "renovate-bot"), Icon::Bot);
        assert_eq!(person_icon(&p, "alice"), Icon::Person);
        assert_eq!(person_icon(&p, &p.me), Icon::You);
    }

    fn draw(p: &GitlabPlugin) {
        // Showing the screen fills the tree state (what is visible), as the app does.
        let _ = crate::tests::view_text(p, "repos", 160, 40);
    }

    #[test]
    fn down_in_the_preview_moves_through_it_not_to_the_next_merge_request() {
        let mut p = sample();
        draw(&p);
        for _ in 0..3 {
            press(&mut p, Pane::Middle, Key::Down);
        }
        let _ = crate::tests::view_text_in(&p, "repos", Pane::Detail, 160, 6);
        let on = p.tree.borrow().selected().to_vec();
        // Moving in the preview is the renderer's (Ink: ink/test/app/
        // detail-scroll.test.tsx); the keys are the preview's, not the tree's.
        for code in [Key::Down, Key::Char('j'), Key::Char('G'), Key::Up, Key::Char('g')] {
            press(&mut p, Pane::Detail, code);
            assert_eq!(p.tree.borrow().selected(), on, "the tree stays put on {code:?}");
        }
        assert_eq!(p.detail_scroll.at(), 0);
    }

    /// PgDn from the tree pages the preview by what the renderer laid out
    /// (#32): the part's scroll learns the preview's rows from the
    /// renderer's report, the view tells the renderer where to go, and the
    /// keys menu offers the page keys while there is more.
    #[test]
    fn page_down_from_the_tree_pages_the_preview_by_the_rows_the_renderer_laid_out() {
        use radar_core::headless::Headless;
        use radar_core::view::{Chord, Intent, Key, PaneId, PaneRows};
        let mut p = sample();
        draw(&p);
        for _ in 0..3 {
            press(&mut p, Pane::Middle, Key::Down);
        }
        let on = p.tree.borrow().selected().to_vec();
        let now = p.now;
        let mut host = radar_core::Host::new(vec![Box::new(sample())], now).unwrap();
        host.load_config(Some("")).unwrap();
        host.plugins[0] = Box::new(p);
        let mut ui = Headless::new(host, 160, 40);
        ui.focus(PaneId::Middle);
        let page = |ui: &mut Headless, down: bool| {
            let key = if down { Key::PageDown } else { Key::PageUp };
            ui.send(Intent::Key { chord: Chord::new(key) })
        };
        let at = |ui: &Headless| {
            let s = radar_core::Plugin::scroll(ui.host().plugins[0].as_ref(), "repos", Pane::Detail).unwrap();
            (s.at(), s.max(), s.height())
        };
        let told = |ui: &Headless| ui.screen().unwrap().pane(PaneId::Detail).unwrap().scroll;
        let offered = |ui: &Headless| -> Vec<String> {
            radar_core::Plugin::keys(ui.host().plugins[0].as_ref(), "repos", Pane::Detail)
                .into_iter()
                .filter(|b| b.what.starts_with("scroll"))
                .map(|b| b.what)
                .collect()
        };
        let rows = |at| [PaneRows { pane: PaneId::Detail, rows: 6, content: 20, at }];
        // Nothing laid out yet: nothing to page, nothing offered.
        page(&mut ui, true);
        assert_eq!(at(&ui), (0, 0, 0));
        assert!(offered(&ui).is_empty());
        ui.lay_out(&rows(0));
        assert_eq!(at(&ui), (0, 14, 6));
        assert_eq!(told(&ui), None, "where the renderer is is not sent back to it");
        assert_eq!(offered(&ui), ["scroll down a page"]);
        // A page keeps one row of the last: 5 down.
        page(&mut ui, true);
        assert_eq!(at(&ui).0, 5);
        assert_eq!(told(&ui), Some(5), "the renderer is told to scroll there");
        assert_eq!(offered(&ui), ["scroll down a page", "scroll up a page"]);
        ui.lay_out(&rows(5));
        assert_eq!(told(&ui), None);
        // The renderer went to the end itself (G): PgDn has nowhere to go.
        ui.lay_out(&rows(14));
        assert_eq!(at(&ui).0, 14);
        page(&mut ui, true);
        assert_eq!(at(&ui).0, 14);
        assert_eq!(offered(&ui), ["scroll up a page"]);
        page(&mut ui, false);
        assert_eq!(at(&ui).0, 9);
        assert_eq!(told(&ui), Some(9));
        // A report about another screen changes nothing.
        ui.host_mut().laid_out("elsewhere", &rows(0));
        assert_eq!(at(&ui).0, 9);
        page(&mut ui, false);
        page(&mut ui, false);
        assert_eq!(at(&ui).0, 0);
        let Some(radar_core::view::Node::Tree(tree)) = ui.screen().unwrap().pane(PaneId::Middle).map(|m| m.body.clone()) else {
            panic!("the middle pane is a tree");
        };
        assert_eq!(tree.selected, on, "the tree stays put");
    }

    #[test]
    fn bulk_commands_close_and_open_the_whole_tree_and_copy_every_link() {
        let mut p = sample();
        draw(&p);
        on_bulk(&mut p, "close");
        assert!(p.tree.borrow().open.is_empty());
        on_bulk(&mut p, "open");
        assert!(!p.tree.borrow().open.is_empty());
        let n = p.visible().len();
        let fx = on_bulk(&mut p, "copy");
        assert!(
            matches!(&fx[0], Effect::Copy(text) if text.lines().count() == n),
            "{fx:?}"
        );
        assert_eq!(on_bulk(&mut p, "browse").len(), n);
    }

    #[test]
    fn the_tree_groups_merge_requests_under_repo_under_tenancy() {
        let mut p = sample();
        p.view = p
            .cfg
            .views
            .iter()
            .position(|v| v.name == "Everything")
            .unwrap();
        let ns = nodes(&p, p.visible());
        assert_eq!(ns.len(), 2);
        let a = &ns[1];
        let names: Vec<&str> = a.repos.iter().map(|r| r.project.name.as_str()).collect();
        let busy: Vec<bool> = a.repos.iter().map(|r| !r.mrs.is_empty()).collect();
        assert_eq!(busy, [true, true, false], "busy repos first: {names:?}");
        assert!(names.contains(&"docs"), "quiet repos are shown too");
        assert!(!names.contains(&"renovate"), "hidden repos are not");
        // `.` flips the view's own setting, which is saved with the view.
        press(&mut p, Pane::Middle, Key::Char('.'));
        assert_eq!(
            p.cfg.views[p.view].repos,
            crate::config::RepoScope::WithMergeRequests
        );
        let ns = nodes(&p, p.visible());
        assert!(!ns[1].repos.iter().any(|r| r.project.name == "docs"));
    }

    #[test]
    fn everything_is_the_first_view_and_views_say_what_they_show() {
        let p = sample();
        assert_eq!(p.current_view().unwrap().name, "Everything");
        let names: Vec<&str> = p.cfg.views.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Everything",
                "Needs my action",
                "Opened by me",
                "Opened by teammates",
                "Opened by bots"
            ]
        );
    }

    #[test]
    fn plus_and_minus_fold_the_tree_and_minus_on_a_merge_request_closes_its_repo() {
        let mut p = sample();
        p.view = p
            .cfg
            .views
            .iter()
            .position(|v| v.name == "Everything")
            .unwrap();
        draw(&p);
        assert!(radar_core::Plugin::folds(&p, "repos", Pane::Middle));
        assert!(!radar_core::Plugin::folds(&p, "repos", Pane::Detail));
        // Down to a merge request: tenancy, repo, merge request.
        press(&mut p, Pane::Middle, Key::Char('j'));
        press(&mut p, Pane::Middle, Key::Char('j'));
        draw(&p);
        let mr = p.tree.borrow().selected().to_vec();
        assert_eq!(mr.len(), 3, "{mr:?}");
        assert!(
            matches!(
                press(&mut p, Pane::Middle, Key::Char('+')),
                Outcome::Ignored
            ),
            "nothing to open"
        );
        press(&mut p, Pane::Middle, Key::Char('-'));
        draw(&p);
        let repo = mr[..2].to_vec();
        assert_eq!(
            p.tree.borrow().selected(),
            repo.as_slice(),
            "its repo, selected"
        );
        assert!(!p.tree.borrow().is_open(&repo), "and closed");
        // Open-all is host-owned; it expands every held group through the
        // owner's existing Fold intent rather than an owner-local '=' key.
        let now = p.now;
        let mut host = radar_core::Host::new(vec![Box::new(p)], now).unwrap();
        host.focus = Pane::Middle;
        host.set_view_room(160, 40);
        let tree = |host: &radar_core::Host| {
            let view = host.screen_view(0, "repos").expect("repositories screen");
            let radar_core::view::Node::Tree(tree) = &view
                .pane(radar_core::view::PaneId::Middle)
                .expect("repositories pane")
                .body
            else {
                panic!("repositories tree")
            };
            tree.clone()
        };
        for key in ['=', '-', '+'] {
            assert!(host.update(crate::key_event(Chord::char(key))).is_empty(), "folds have no external effects");
            let after = tree(&host);
            assert_eq!(after.selected, repo, "folding keeps the selected repo");
            assert_eq!(after.open.contains(&repo), key != '-', "{key}");
            if key == '=' {
                assert!(after.groups().iter().all(|path| after.open.contains(path)),
                    "= opens every held tenancy and repo");
            }
        }
    }

    #[test]
    fn fold_menu_only_offers_actions_for_the_selected_group() {
        let mut p = sample();
        p.view = p.cfg.views.iter().position(|v| v.name == "Everything").unwrap();
        draw(&p);
        assert_eq!(fold_keys(&p).iter().map(|(_, b)| b.keys.as_str()).collect::<Vec<_>>(), vec!["-"]);
        press(&mut p, Pane::Middle, Key::Char('j'));
        draw(&p);
        assert_eq!(fold_keys(&p).iter().map(|(_, b)| b.keys.as_str()).collect::<Vec<_>>(), vec!["-"]);
        press(&mut p, Pane::Middle, Key::Char('j'));
        draw(&p);
        assert!(fold_keys(&p).is_empty(), "an MR row has no fold action");
    }

    #[test]
    fn moving_and_folding_the_tree() {
        let mut p = sample();
        p.view = p
            .cfg
            .views
            .iter()
            .position(|v| v.name == "Everything")
            .unwrap();
        draw(&p);
        assert_eq!(p.tree.borrow().selected(), ["t:team-b"]);
        press(&mut p, Pane::Middle, Key::Char('j'));
        draw(&p);
        assert_eq!(
            p.tree.borrow().selected().len(),
            2,
            "into a repo of the first tenancy"
        );
        press(&mut p, Pane::Middle, Key::Char('k'));
        press(&mut p, Pane::Middle, Key::Char(' ')); // space folds; ⏎ opens a page
        draw(&p);
        press(&mut p, Pane::Middle, Key::Char('j'));
        assert_eq!(
            p.tree.borrow().selected(),
            ["t:team-a"],
            "closed tenancy is skipped over"
        );
    }

    #[test]
    fn filter_keeps_and_clears() {
        let mut p = sample();
        press(&mut p, Pane::Middle, Key::Char('/'));
        for c in "worker".chars() {
            press(&mut p, Pane::Middle, Key::Char(c));
        }
        press(&mut p, Pane::Middle, Key::Enter);
        assert!(p.filter_input.is_none());
        assert_eq!(p.filter, "worker");
        press(&mut p, Pane::Middle, Key::Char('/'));
        press(&mut p, Pane::Middle, Key::Esc);
        assert!(p.filter.is_empty());
    }

    #[test]
    fn typed_words_hide_repos_without_merge_requests_unless_the_words_name_them() {
        let mut p = sample();
        p.view = p
            .cfg
            .views
            .iter()
            .position(|v| v.name == "Everything")
            .unwrap();
        let repos = |p: &GitlabPlugin| -> Vec<String> {
            nodes(p, p.visible())
                .iter()
                .flat_map(|t| t.repos.iter().map(|r| r.project.name.clone()))
                .collect()
        };
        assert!(
            repos(&p).contains(&"docs".to_string()),
            "no filter: every repo"
        );
        p.filter = "stats".into();
        assert_eq!(
            repos(&p),
            ["api"],
            "only the repo whose merge request says it"
        );
        p.filter = "docs".into();
        assert_eq!(repos(&p), ["docs"], "a quiet repo whose name the words say");
        p.filter = "docs is:review".into();
        assert!(repos(&p).is_empty(), "it has nothing to review");
    }

    #[test]
    fn typed_words_leave_out_tenancies_with_nothing_found() {
        let mut p = sample();
        p.view = p.cfg.views.iter().position(|v| v.name == "Everything").unwrap();
        let tenancies = |p: &GitlabPlugin| nodes(p, p.visible()).len();
        let all = tenancies(&p);
        assert!(all > 1, "the sample has more than one tenancy");
        p.filter = "stats".into();
        assert_eq!(tenancies(&p), 1, "only the tenancy holding what was found");
        p.filter = "nothing-says-this".into();
        assert_eq!(tenancies(&p), 0);
        p.filter.clear();
        assert_eq!(tenancies(&p), all, "no filter: every tenancy, empty or not");
    }

    #[test]
    fn down_from_the_filter_goes_to_the_first_result() {
        let mut p = sample();
        p.view = p.cfg.views.iter().position(|v| v.name == "Everything").unwrap();
        draw(&p);
        press(&mut p, Pane::Nav, Key::Char('/'));
        for c in "stats".chars() {
            press(&mut p, Pane::Nav, Key::Char(c));
        }
        let out = key(&mut p, Pane::Nav, Chord::new(Key::Down));
        let Outcome::Handled(fx) = out else { panic!("↓ is the filter's") };
        assert!(fx.iter().any(|e| matches!(e, Effect::Focus(Pane::Middle))));
        assert!(p.filter_input.is_none());
        assert_eq!(p.filter, "stats", "↓ keeps the words");
        let at = p.tree.borrow().selected().to_vec();
        assert_eq!(at.len(), 3, "on the merge request found: {at:?}");
        assert!(at[1].ends_with("/api"), "{at:?}");
    }

    #[test]
    fn enter_from_the_filter_goes_to_the_results() {
        let mut p = sample();
        draw(&p);
        press(&mut p, Pane::Nav, Key::Char('/'));
        press(&mut p, Pane::Nav, Key::Char('x'));
        let out = key(&mut p, Pane::Nav, Chord::new(Key::Enter));
        let Outcome::Handled(fx) = out else { panic!("⏎ is the filter's") };
        assert!(fx.iter().any(|e| matches!(e, Effect::Focus(Pane::Middle))));
    }
}
