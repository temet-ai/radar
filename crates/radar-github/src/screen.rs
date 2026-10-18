//! Browse, the detail pane, and item pages, using the shared UI kit.

use radar_core::plugin::{Binding, Effect, Menu, MenuItem, Outcome, Pane};
use radar_core::ui::{self, rows::Move};
use radar_core::view::{self as v, Chord, Key};

use crate::{GithubPlugin, join::Target, model::PullRequest};

pub(crate) mod neutral;

pub(crate) struct UnderAction {
    generation: u64,
    target: Target,
    location: UnderLocation,
    part: &'static str,
    subject: radar_core::Subject,
    id: String,
}

#[derive(Clone, Copy)]
enum UnderLocation {
    Page { request: u64, tab: usize },
    Detail,
}

pub(crate) fn tabs(target: &Target) -> Vec<String> {
    match target {
        Target::Repository(_) => vec!["Overview".into(), "Pull requests".into(), "Actions".into()],
        Target::Pull(..) => vec!["Overview".into(), "Checks".into(), "Actions".into()],
        Target::Run(..) => vec!["Overview".into(), "Jobs".into()],
    }
}

pub(crate) fn show_tab(p: &mut GithubPlugin, tab: usize) -> Vec<Effect> {
    if let Some(page) = p.pages.last_mut() {
        page.tab = tab.min(tabs(&page.target).len() - 1);
        p.detail_scroll.top();
        p.links.borrow_mut().clear();
        p.under_actions.borrow_mut().clear();
        p.link_menu = None;
    }
    if p.pages
        .last()
        .is_some_and(|page| matches!(page.target, Target::Pull(..)) && page.tab != 2)
        && p.pull_detail.is_none()
        && !p.page_loading
    {
        return p.refresh_page();
    }
    crate::actions::ensure(p)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TreeItem {
    Group(usize),
    Repo(usize),
    Pull(usize, usize),
}

fn group_repos(p: &GithubPlugin, group: usize) -> Vec<usize> {
    p.cfg.groups[group]
        .repositories
        .iter()
        .filter_map(|name| p.repos.iter().position(|repo| repo.id.full_name() == *name))
        .collect()
}

/// Configuration names, repository identities and PR numbers survive insertion
/// and refresh reordering. Escape the path separator (and its escape marker)
/// because group names, unlike GitHub repository names, are free-form text.
fn group_id(name: &str) -> String {
    format!("group:{}", name.replace('%', "%25").replace(v::PATH_SEP, "%1F"))
}

fn repo_id(repo: &crate::model::RepositoryId) -> String {
    format!("repo:{}", repo.full_name())
}

fn pull_id(number: u64) -> String {
    format!("pull:{number}")
}

fn tree_path(p: &GithubPlugin, item: TreeItem) -> Option<Vec<String>> {
    if let TreeItem::Group(group) = item {
        return Some(vec![group_id(&p.cfg.groups.get(group)?.name)]);
    }
    let (repo, pull) = match item {
        TreeItem::Repo(repo) => (repo, None),
        TreeItem::Pull(repo, pull) => (repo, Some(pull)),
        TreeItem::Group(_) => unreachable!(),
    };
    let repo = p.repos.get(repo)?;
    let group = p.cfg.groups.iter().find(|group| group.repositories.contains(&repo.id.full_name()))?;
    let mut path = vec![group_id(&group.name), repo_id(&repo.id)];
    if let Some(pull) = pull {
        path.push(pull_id(repo.pulls.as_ref()?.items.get(pull)?.number));
    }
    Some(path)
}

/// Resolve only a currently visible row, under its exact ancestors. An old
/// target must never select the row now occupying its former numeric position.
fn tree_item_at(p: &GithubPlugin, path: &[String]) -> Option<TreeItem> {
    if !p.pages.is_empty() || path.is_empty() || path.len() > 3 {
        return None;
    }
    let group = p.cfg.groups.iter().position(|group| group_id(&group.name) == path[0])?;
    if p.group_at != 0 && p.group_at != group + 1 {
        return None;
    }
    if path.len() == 1 {
        return Some(TreeItem::Group(group));
    }
    if p.folded_groups.contains(&group) {
        return None;
    }
    let repo = group_repos(p, group).into_iter().find(|&repo| repo_id(&p.repos[repo].id) == path[1])?;
    if path.len() == 2 {
        return Some(TreeItem::Repo(repo));
    }
    if p.folded_repos.contains(&repo) {
        return None;
    }
    let pull = p.repos[repo].pulls.as_ref()?.items.iter().position(|pull| pull_id(pull.number) == path[2])?;
    Some(TreeItem::Pull(repo, pull))
}

fn fold(p: &mut GithubPlugin, target: &v::Target, open: bool) -> Outcome {
    let path = v::TreeRow::path_of(target);
    let Some(item) = tree_item_at(p, &path) else { return Outcome::Ignored };
    // Shared '-' may address the selected row's parent. Opening a group
    // must leave selection alone, including the host's parent-first open-all.
    let selected_item = selected_item(p);
    let selected = selected_item.and_then(|item| tree_path(p, item));
    let has_children = match item {
        TreeItem::Group(group) => !group_repos(p, group).is_empty(),
        TreeItem::Repo(repo) => p.repos[repo].pulls.as_ref().is_some_and(|pulls| !pulls.items.is_empty()),
        TreeItem::Pull(..) => false,
    };
    if !has_children {
        return Outcome::Ignored;
    }
    let folded = match item {
        TreeItem::Group(index) => (&mut p.folded_groups, index),
        TreeItem::Repo(index) => (&mut p.folded_repos, index),
        TreeItem::Pull(..) => return Outcome::Ignored,
    };
    if open {
        folded.0.remove(&folded.1);
    } else {
        folded.0.insert(folded.1);
    }
    if open {
        // Freeze the visible selection even when it was the default fallback.
        p.tree_at = selected_item;
    } else if selected.is_some_and(|selected| selected.starts_with(&path) && selected.len() > path.len()) {
        select_item(p, item);
    }
    // Match press/point: an accepted input expires previously rendered actions.
    p.action_epoch = p.action_epoch.wrapping_add(1);
    Outcome::done()
}

fn tree_items(p: &GithubPlugin) -> Vec<TreeItem> {
    let mut items = Vec::new();
    for group in 0..p.cfg.groups.len() {
        if p.group_at != 0 && p.group_at != group + 1 {
            continue;
        }
        items.push(TreeItem::Group(group));
        if p.folded_groups.contains(&group) {
            continue;
        }
        for repo in group_repos(p, group) {
            items.push(TreeItem::Repo(repo));
            if p.folded_repos.contains(&repo) {
                continue;
            }
            if let Some(pulls) = &p.repos[repo].pulls {
                for index in 0..pulls.items.len() {
                    items.push(TreeItem::Pull(repo, index));
                }
            }
        }
    }
    items
}

pub(crate) fn selected_item(p: &GithubPlugin) -> Option<TreeItem> {
    let items = tree_items(p);
    p.tree_at
        .filter(|item| items.contains(item))
        .or_else(|| {
            items
                .iter()
                .copied()
                .find(|item| *item == TreeItem::Pull(p.repo_at, p.pull_at))
        })
        .or_else(|| {
            items
                .iter()
                .copied()
                .find(|item| *item == TreeItem::Repo(p.repo_at))
        })
        .or_else(|| items.first().copied())
}

fn select_item(p: &mut GithubPlugin, item: TreeItem) {
    p.tree_at = Some(item);
    match item {
        TreeItem::Group(_) => {}
        TreeItem::Repo(repo) => {
            p.repo_at = repo;
            p.pull_at = 0;
        }
        TreeItem::Pull(repo, pull) => {
            p.repo_at = repo;
            p.pull_at = pull;
        }
    }
    p.detail_scroll.top();
    p.links.borrow_mut().clear();
}

fn select_view(p: &mut GithubPlugin, view: usize) {
    p.group_at = view;
    let items = tree_items(p);
    let selected = items
        .iter()
        .copied()
        .find(|item| matches!(item, TreeItem::Pull(..)))
        .or_else(|| {
            items
                .iter()
                .copied()
                .find(|item| matches!(item, TreeItem::Repo(..)))
        })
        .or_else(|| items.first().copied());
    p.pull_scroll.top();
    if let Some(item) = selected {
        select_item(p, item);
    } else {
        p.tree_at = None;
        p.detail_scroll.top();
    }
}

fn pull_state(pull: &PullRequest) -> &'static str {
    if pull.merged {
        "merged"
    } else if pull.state == "closed" {
        "closed"
    } else if pull.state == "open" && pull.draft {
        "draft"
    } else if pull.state == "open" {
        "open"
    } else {
        "unknown"
    }
}

fn safe_inlines(runs: &mut [ui::rich::Inline]) {
    for run in runs {
        if matches!(&run.kind, ui::rich::Kind::Link { url } if !safe_link(url))
            || matches!(&run.kind, ui::rich::Kind::Image { src } if !safe_link(src))
        {
            run.kind = ui::rich::Kind::Text;
        }
    }
}

fn safe_blocks(blocks: &mut [ui::rich::Block]) {
    use ui::rich::Block;
    for block in blocks {
        match block {
            Block::Para { text } | Block::Heading { text, .. } => safe_inlines(text),
            Block::List { items } => {
                for item in items {
                    safe_blocks(&mut item.blocks);
                }
            }
            Block::Quote { blocks } | Block::Callout { blocks, .. } => safe_blocks(blocks),
            Block::Table { head, rows, .. } => {
                for cell in head {
                    safe_inlines(cell);
                }
                for row in rows {
                    for cell in row {
                        safe_inlines(cell);
                    }
                }
            }
            Block::Details { summary, blocks } => {
                safe_inlines(summary);
                safe_blocks(blocks);
            }
            Block::Media(media) => {
                if !safe_link(&media.url) {
                    media.url.clear();
                }
                if !safe_link(&media.src) {
                    media.src.clear();
                }
            }
            Block::Code { .. } | Block::Rule => {}
        }
    }
}

fn safe_link(url: &str) -> bool {
    !url.chars().any(|c| c.is_control() || c.is_whitespace())
        && !radar_core::log::contains_secret(url)
        && url::Url::parse(url).is_ok_and(|u| {
            matches!(u.scheme(), "http" | "https")
                && u.username().is_empty()
                && u.password().is_none()
                && u.host_str().is_some()
        })
}

pub(crate) fn key(p: &mut GithubPlugin, pane: Pane, key: Chord) -> Outcome {
    use radar_core::view_tui::key_of;
    if key.ctrl || key.alt {
        return Outcome::Ignored;
    }
    if let Some(outcome) = crate::actions::key(p, key) {
        return outcome;
    }
    match key.key {
        Key::Char('r') => {
            return Outcome::Handled(if p.pages.is_empty() {
                p.refresh_all()
            } else {
                p.refresh_page()
            });
        }
        Key::Char('o' | 'y') => {
            let Some(target) = p.target(pane) else {
                return Outcome::Ignored;
            };
            let url = GithubPlugin::target_url(&target);
            return Outcome::Handled(if key.key == Key::Char('o') {
                vec![Effect::Open(url)]
            } else {
                vec![Effect::Copy(url), Effect::Notify("link copied".into())]
            });
        }
        Key::Esc if !p.pages.is_empty() => {
            let left = p.pages.pop().unwrap();
            if let Some(index) = left
                .back_repo
                .and_then(|id| p.repos.iter().position(|r| r.id == id))
            {
                p.repo_at = index;
                p.pull_at = left
                    .back_pull
                    .and_then(|number| {
                        p.repos[index]
                            .pulls
                            .as_ref()?
                            .items
                            .iter()
                            .position(|pull| pull.number == number)
                    })
                    .unwrap_or(0);
            }
            p.page_request = p.page_request.wrapping_add(1);
            p.clear_page();
            p.restore_page();
            let mut effects = if p
                .pages
                .last()
                .is_some_and(|page| matches!(page.target, Target::Pull(_, _)))
            {
                p.refresh_page()
            } else {
                vec![]
            };
            effects.extend(crate::actions::ensure(p));
            return Outcome::Handled(effects);
        }
        Key::Char('[' | ']') if !p.pages.is_empty() => {
            let page = p.pages.last().unwrap();
            let count = tabs(&page.target).len();
            let tab = if key.key == Key::Char(']') {
                (page.tab + 1) % count
            } else {
                (page.tab + count - 1) % count
            };
            return Outcome::Handled(show_tab(p, tab));
        }
        Key::Enter => {
            if p.pages.is_empty() {
                if pane == Pane::Nav {
                    return Outcome::Ignored;
                }
                if pane == Pane::Middle
                    && let Some(TreeItem::Group(index)) = selected_item(p)
                {
                    if !p.folded_groups.insert(index) {
                        p.folded_groups.remove(&index);
                    }
                    return Outcome::done();
                }
            }
            if p.pages.last().is_some_and(|page| page.tab == 0) {
                return links_menu(p, pane);
            }
            if p.pages
                .last()
                .is_some_and(|page| matches!(page.target, Target::Pull(_, _)))
            {
                return Outcome::Ignored;
            }
            let Some(target) = p.target(pane) else {
                return Outcome::Ignored;
            };
            return Outcome::Handled(p.open_target(target));
        }
        _ => {}
    }
    if p.pages.is_empty() && pane == Pane::Middle
        && matches!(key.key, Key::Left | Key::Right | Key::Char(' ' | '+' | '-'))
        && let v::Node::Tree(tree) = neutral::middle_pane(p).body
    {
        return v::tree_step(&tree, key).map_or(Outcome::Ignored, |step| neutral::intent(p, pane, &step));
    }
    if matches!(key.key, Key::PageDown | Key::PageUp)
        && p.pages.is_empty()
        && pane == Pane::Detail
    {
        return if p.detail_scroll.key(&key_of(key)) {
            Outcome::done()
        } else {
            Outcome::Ignored
        };
    }
    let text_page = p
        .pages
        .last()
        .is_some_and(|page| page.tab == 0 || matches!(page.target, Target::Pull(_, _)));
    if text_page || (p.pages.is_empty() && pane == Pane::Detail) {
        return if p.detail_scroll.key(&key_of(key)) {
            Outcome::done()
        } else {
            Outcome::Ignored
        };
    }
    let items = if p.pages.is_empty() && pane == Pane::Middle {
        tree_items(p)
    } else {
        Vec::new()
    };
    let (at, len, height) = if p.pages.is_empty() {
        match pane {
            Pane::Nav => (p.group_at, p.cfg.groups.len() + 1, p.repo_scroll.height()),
            Pane::Middle => (
                selected_item(p)
                    .and_then(|item| items.iter().position(|at| *at == item))
                    .unwrap_or(0),
                items.len(),
                p.pull_scroll.height(),
            ),
            Pane::Detail => return Outcome::Ignored,
        }
    } else {
        (
            p.pull_at,
            p.repository()
                .and_then(|r| r.pulls.as_ref())
                .map_or(0, |pulls| pulls.items.len()),
            p.pull_scroll.height(),
        )
    };
    let Some(movement) = Move::from_key(&key_of(key), height) else {
        return Outcome::Ignored;
    };
    let to = ui::rows::select(len, at, movement, |_| true);
    if to == at {
        return Outcome::Ignored;
    }
    if p.pages.is_empty() && pane == Pane::Nav {
        select_view(p, to);
    } else if p.pages.is_empty() {
        select_item(p, items[to]);
    } else {
        p.pull_at = to;
        p.detail_scroll.top();
        p.links.borrow_mut().clear();
    }
    Outcome::done()
}

pub(crate) fn click(p: &mut GithubPlugin, id: &str) -> Outcome {
    if let Some(outcome) = crate::actions::click(p, id) {
        return outcome;
    }
    if let Some(value) = id.strip_prefix("under:") {
        let index = if let Some((epoch, index)) = value.split_once(':') {
            if epoch.parse::<u64>().ok() != Some(p.action_epoch) { return Outcome::Ignored; }
            index.parse::<usize>().ok()
        } else { value.parse::<usize>().ok() };
        let Some(index) = index else { return Outcome::Ignored; };
        let actions = p.under_actions.borrow();
        let Some(action) = actions.get(index) else {
            return Outcome::Ignored;
        };
        let current = p.subject_for(&action.target);
        let here = match action.location {
            UnderLocation::Page { request, tab } => {
                request == p.page_request
                    && p.pages
                        .last()
                        .is_some_and(|page| page.target == action.target && page.tab == tab)
            }
            UnderLocation::Detail => {
                p.pages.is_empty() && p.target(Pane::Detail).as_ref() == Some(&action.target)
            }
        };
        if action.generation != p.generation
            || !here
            || current != action.subject
            || current.url != action.subject.url
            || current.fields != action.subject.fields
        {
            return Outcome::Handled(vec![Effect::Notify(
                "page changed; reopen its actions".into(),
            )]);
        }
        return Outcome::Handled(vec![Effect::Act {
            part: action.part,
            subject: action.subject.clone(),
            id: action.id.clone(),
        }]);
    }
    if let Some(index) = id
        .strip_prefix("view:")
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|i| p.pages.is_empty() && *i <= p.cfg.groups.len())
    {
        select_view(p, index);
        return Outcome::done();
    }
    if let Some(item) = tree_item_at(p, &v::TreeRow::path_of(&v::Target::new(id))) {
        select_item(p, item);
        return Outcome::done();
    }
    if let Some(index) = id
        .strip_prefix("pull:")
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|i| {
            p.pages.last().is_some_and(|page| matches!(page.target, Target::Repository(_)) && page.tab == 1)
                && p.repository()
                .and_then(|r| r.pulls.as_ref())
                .is_some_and(|p| *i < p.items.len())
        })
    {
        p.pull_at = index;
        p.detail_scroll.top();
        p.links.borrow_mut().clear();
        return Outcome::done();
    }
    if let Some(tab) = id
        .strip_prefix("tab:")
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|i| {
            p.pages
                .last()
                .is_some_and(|page| *i < tabs(&page.target).len())
        })
    {
        return Outcome::Handled(show_tab(p, tab));
    }
    Outcome::Ignored
}

pub(crate) fn keys(p: &GithubPlugin, pane: Pane) -> Vec<crate::Act> {
    if crate::actions::active(p) {
        return crate::actions::keys(p);
    }
    let surface = if p.pages.is_empty() { "browse" } else { "page" };
    let act = |verb: &str, b: Binding| crate::act(surface, verb, b);
    let heading = if p.pages.is_empty() && pane == Pane::Nav {
        if p.group_at == 0 {
            "Everything".into()
        } else {
            p.cfg
                .groups
                .get(p.group_at - 1)
                .map(|group| group.name.clone())
                .unwrap_or_else(|| "GitHub".into())
        }
    } else if p.pages.is_empty() && pane == Pane::Middle {
        match selected_item(p) {
            Some(TreeItem::Group(index)) => p.cfg.groups[index].name.clone(),
            _ => p
                .target(pane)
                .map(|t| p.subject_for(&t).label)
                .unwrap_or_else(|| "Repositories".into()),
        }
    } else {
        p.target(pane)
            .map(|t| p.subject_for(&t).label)
            .unwrap_or_else(|| "GitHub".into())
    };
    let mut rows = vec![crate::head(Binding::heading(heading))];
    if p.pages.is_empty() && pane == Pane::Nav {
        rows.push(act("choose_group", Binding::new("↑ ↓", "choose a group")));
    }
    if p.pages.is_empty() && pane == Pane::Middle {
        rows.push(act(
            "browse",
            Binding::new("↑ ↓", "browse repositories and pull requests"),
        ));
        if matches!(
            selected_item(p),
            Some(TreeItem::Group(_) | TreeItem::Repo(_))
        ) {
            rows.push(act(
                "fold",
                Binding::new("space", "fold or expand a group or repository"),
            ));
        }
    }
    if p.target(pane).is_some() {
        if p.pages.is_empty()
            || p.pages
                .last()
                .is_some_and(|page| matches!(page.target, Target::Repository(_)) && page.tab == 1)
        {
            rows.push(act("open_page", Binding::new("⏎", "open its page")));
        } else if p.pages.last().is_some_and(|page| page.tab == 0) {
            rows.push(act(
                "follow_link",
                Binding::new("⏎", "follow a link on this page…"),
            ));
        }
        rows.extend([
            act("open_on_github", Binding::new("o", "open on GitHub")),
            act("copy_link", Binding::new("y", "copy the link")),
        ]);
    }
    rows.push(act(
        "refresh",
        Binding::new(
            "r",
            if p.pages.is_empty() {
                "refresh configured repositories"
            } else {
                "refresh this page"
            },
        ),
    ));
    if !p.pages.is_empty() {
        rows.extend([
            act("previous_tab", Binding::new("[", "previous tab")),
            act("next_tab", Binding::new("]", "next tab")),
            act("back", Binding::new("esc", "back to the previous page")),
        ]);
    }
    rows
}

fn links_menu(p: &mut GithubPlugin, pane: Pane) -> Outcome {
    let links = p.links.borrow().clone();
    if links.is_empty() {
        return Outcome::Handled(vec![Effect::Notify("no links on this page".into())]);
    }
    let Some(target) = p.target(pane) else {
        return Outcome::Ignored;
    };
    let items = links
        .iter()
        .enumerate()
        .map(|(i, link)| MenuItem::pick(&link.text, &link.url, i.to_string()))
        .collect();
    p.link_menu = Some((p.generation, p.page_request, target, links));
    Outcome::Handled(vec![Effect::Menu(Menu::new(
        "links",
        "links on this page",
        items,
    ))])
}

pub(crate) fn follow_link(p: &mut GithubPlugin, value: &str) -> Vec<Effect> {
    let Some((generation, request, target, links)) = p.link_menu.take() else {
        return vec![];
    };
    if generation != p.generation
        || request != p.page_request
        || p.target(Pane::Middle).as_ref() != Some(&target)
    {
        return vec![Effect::Notify("page changed; reopen its links".into())];
    }
    value
        .parse::<usize>()
        .ok()
        .and_then(|i| links.get(i))
        .map(|link| vec![Effect::Show(radar_core::Subject::link(&link.url))])
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_views_and_tree_keep_repositories_and_pulls_reachable() {
        let mut p = crate::demo::sample();
        p.cfg.groups[0].repositories = vec!["acme/api".into()];
        p.cfg.groups.push(crate::config::Group {
            name: "Docs".into(),
            owner: None,
            repositories: vec!["acme/docs".into()],
        });
        assert_eq!(
            tree_items(&p),
            vec![
                TreeItem::Group(0),
                TreeItem::Repo(0),
                TreeItem::Pull(0, 0),
                TreeItem::Pull(0, 1),
                TreeItem::Group(1),
                TreeItem::Repo(1),
            ]
        );
        select_view(&mut p, 2);
        assert_eq!(tree_items(&p), vec![TreeItem::Group(1), TreeItem::Repo(1)]);
        assert_eq!(
            p.target(Pane::Middle),
            Some(Target::Repository(p.repos[1].id.clone()))
        );
        assert_eq!(p.target(Pane::Nav), None);
        // A repository without held pulls is a leaf: space cannot fold it.
        assert!(matches!(
            key(&mut p, Pane::Middle, Chord::char(' ')),
            Outcome::Ignored
        ));
        assert_eq!(selected_item(&p), Some(TreeItem::Repo(1)));
        assert!(!p.folded_repos.contains(&1));
        assert!(p.pages.is_empty());
        assert_eq!(tree_items(&p), vec![TreeItem::Group(1), TreeItem::Repo(1)]);
        select_item(&mut p, TreeItem::Group(1));
        key(&mut p, Pane::Middle, Chord::char(' '));
        assert_eq!(tree_items(&p), vec![TreeItem::Group(1)]);
        key(&mut p, Pane::Middle, Chord::char('+'));
        assert_eq!(tree_items(&p), vec![TreeItem::Group(1), TreeItem::Repo(1)]);
    }

    #[test]
    fn closed_draft_and_future_states_are_never_open_drafts() {
        let id = crate::model::RepositoryId::parse("acme/api").unwrap();
        let mut pull = crate::demo::pull(&id, 43, "draft");
        assert_eq!(pull_state(&pull), "draft");
        pull.state = "closed".into();
        assert_eq!(pull_state(&pull), "closed");
        pull.state = "future".into();
        assert_eq!(pull_state(&pull), "unknown");
        pull.merged = true;
        assert_eq!(pull_state(&pull), "merged");
    }

    #[test]
    fn unsafe_description_links_are_not_actionable() {
        for url in [
            "javascript:alert(1)",
            "file:///tmp/local",
            "https://user:password@example.com/",
            "https://example.com/\npath",
            "https://example.com/has space",
        ] {
            assert!(!safe_link(url), "{url}");
        }
        assert!(safe_link("https://example.com/docs#section"));
    }

    #[test]
    fn unsafe_markdown_links_are_removed_before_terminal_hyperlink_layout() {
        let mut doc = ui::markdown::parse(
            "[bad](https://user:password@example.com/private)\n\n> [nested](file:///tmp/private)\n\n![image](https://user:password@example.com/image.png)",
        );
        radar_core::log::secret("private-body-link-canary-19");
        doc.blocks.push(ui::rich::Block::para(vec![
            ui::rich::Inline {
                text: "private link".into(),
                kind: ui::rich::Kind::Link {
                    url: "https://example.com/private-body-link-canary-19".into(),
                },
                ..Default::default()
            },
            ui::rich::Inline {
                text: "control link".into(),
                kind: ui::rich::Kind::Link {
                    url: "https://example.com/\u{1b}injected".into(),
                },
                ..Default::default()
            },
        ]));
        safe_blocks(&mut doc.blocks);
        let theme = radar_core::Theme::new(radar_core::design::theme::Tier::Mono);
        ui::link::set(true);
        let laid = ui::rich::layout(&doc, &ui::rich::Look::new(&theme), 80);
        ui::link::set(false);
        assert!(laid.links.is_empty());
        assert!(laid.media.iter().all(|m| m.src.is_empty()));
        let text = ui::rich::text_of(&laid.lines);
        assert!(text.contains("private link"));
        assert!(text.contains("control link"));
        assert!(!text.contains("private-body-link-canary-19"));
    }
}
