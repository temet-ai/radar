//! GitHub screen and pages as renderer-neutral data.

use radar_core::plugin::{Outcome, Pane};
use radar_core::ui::icon::Icon;
use radar_core::view::{self as v, Block, Ink, ListRow, Node, PaneId, PaneView, Seg, Text, TreeRow};

use super::{GithubPlugin, Target};

pub fn view(p: &GithubPlugin, cx: &v::ViewCx<'_>) -> v::ScreenView {
    p.links.borrow_mut().clear();
    p.under_actions.borrow_mut().clear();
    p.view_rows.set(cx.height);
    let mut screen = if p.pages.is_empty() {
        v::ScreenView::new("github", vec![nav_pane(p), middle_pane(p), detail_pane(p, cx)])
    } else {
        v::ScreenView::page("github", page(p, cx))
    };
    screen.medium_from = Some(70);
    screen.actions = super::keys(p, Pane::from(cx.focus))
        .iter()
        .map(crate::action_spec)
        .collect();
    screen
}

pub fn intent(p: &mut GithubPlugin, pane: Pane, intent: &v::Intent) -> Outcome {
    match intent {
        v::Intent::Context { target }
            if target.as_str().starts_with("tab:") || target.as_str().starts_with("under:") =>
        {
            Outcome::Ignored
        }
        v::Intent::Select { target }
        | v::Intent::Context { target }
        | v::Intent::Activate { target } => {
            let out = p.point("github", target.as_str());
            if matches!(intent, v::Intent::Activate { .. })
                && matches!(out, Outcome::Handled(_))
                && !target.as_str().starts_with("tab:")
                && !target.as_str().starts_with("under:")
            {
                return p.press(pane, v::Chord::new(v::Key::Enter));
            }
            out
        }
        v::Intent::Key { .. } | v::Intent::Invoke { .. } => {
            let rows = super::keys(p, pane);
            crate::chord_for(&rows, intent).map_or(Outcome::Ignored, |chord| p.press(pane, chord))
        }
        v::Intent::Fold { target, open } if pane == Pane::Middle => super::fold(p, target, *open),
        v::Intent::OpenAll | v::Intent::Fold { .. } | v::Intent::Show { .. } | v::Intent::End { .. } | v::Intent::EditSelect { .. } | v::Intent::EditUnselect | v::Intent::Dock(_) | v::Intent::Layout(_) | v::Intent::Copy { .. } | v::Intent::NoticeInput { .. } | v::Intent::DismissNotice { .. } => Outcome::Ignored,
    }
}

fn nav_pane(p: &GithubPlugin) -> PaneView {
    let mut rows = vec![
        ListRow::new("view:0", Text::plain("Everything"))
            .right(Text::of(format!("{} repos", p.repos.len()), Ink::Muted)),
    ];
    for (index, group) in p.cfg.groups.iter().enumerate() {
        rows.push(
            ListRow::new(
                format!("view:{}", index + 1),
                Text::from(vec![
                    Seg::icon(Icon::Group),
                    Seg::new(group.owner.as_deref().unwrap_or(&group.name), Ink::Primary),
                ]),
            )
            .right(Text::of(
                format!("{} repos", super::group_repos(p, index).len()),
                Ink::Muted,
            )),
        );
    }
    let mut pane = PaneView::new(
        PaneId::Nav,
        "views",
        Node::List {
            rows,
            selected: Some(p.group_at),
            editing: None,
        },
    );
    pane.width = Some(26);
    pane
}

pub(super) fn middle_pane(p: &GithubPlugin) -> PaneView {
    let items = super::tree_items(p);
    if items.is_empty() {
        return PaneView::new(
            PaneId::Middle,
            "repositories",
            Node::State {
                state: v::State::Empty,
                text: "No repositories in this group yet. Browse and select them in settings."
                    .into(),
            },
        );
    }
    let selected = super::selected_item(p)
        .and_then(|item| super::tree_path(p, item))
        .unwrap_or_default();
    let mut open = Vec::new();
    let mut status_width = 0;
    let rows = p.cfg.groups.iter().enumerate()
        .filter(|(group, _)| p.group_at == 0 || p.group_at == group + 1)
        .map(|(group, config)| {
            let group_path = vec![super::group_id(&config.name)];
            if !p.folded_groups.contains(&group) {
                open.push(group_path.clone());
            }
            let repositories = super::group_repos(p, group);
            let mut row = TreeRow::new(
                group_path[0].clone(),
                Text::from(vec![
                    Seg::icon(Icon::Group),
                    Seg::new(config.name.clone(), Ink::Emphasis),
                ]),
            ).cell("status", Text::of(format!("{} repos", repositories.len()), Ink::Muted));
            row.children = repositories.into_iter().map(|index| {
                let repo = &p.repos[index];
                let id = super::repo_id(&repo.id);
                if !p.folded_repos.contains(&index) {
                    open.push(vec![group_path[0].clone(), id.clone()]);
                }
                let mut row = TreeRow::new(id, Text::from(vec![
                    Seg::icon(Icon::Repo),
                    Seg::new(repo.id.full_name(), Ink::Primary),
                ]));
                // A background read keeps showing what is held.
                let status = if repo.loading && repo.pulls.is_none() {
                    Some(Text::of("loading", Ink::Info))
                } else if repo.error.is_some() {
                    Some(Text::of("failed", Ink::Bad))
                } else {
                    repo.pulls.as_ref().map(|pulls| Text::of(
                        format!("{} PRs", pulls.items.len()),
                        if pulls.completion.complete { Ink::Muted } else { Ink::Warn },
                    ))
                };
                if let Some(status) = status {
                    row = row.cell("status", status);
                }
                if let Some(pulls) = &repo.pulls {
                    row.children = pulls.items.iter().map(|pull| {
                        TreeRow::new(
                            super::pull_id(pull.number),
                            Text::plain(format!("#{}  {}", pull.number, pull.title)),
                        ).cell("status", Text::of(super::pull_state(pull), Ink::Secondary))
                    }).collect();
                }
                row
            }).collect();
            // Width is based on the complete model, so folding only changes
            // visibility, never a column or any row's presentation data.
            fn measure(row: &TreeRow, width: &mut usize) {
                for cell in &row.cells {
                    *width = (*width).max(cell.text.0.iter().map(|seg| radar_core::ui::width(&seg.text)).sum());
                }
                for child in &row.children { measure(child, width); }
            }
            measure(&row, &mut status_width);
            row
        }).collect();
    PaneView::new(
        PaneId::Middle,
        "repositories",
        Node::Tree(v::Tree {
            rows,
            selected,
            open,
            columns: vec![v::ColumnSpec::new("status", (status_width + 1) as u16, 1)],
            ..v::Tree::default()
        }),
    )
}

fn pulls(p: &GithubPlugin) -> PaneView {
    let title = p
        .repository()
        .map(|r| r.id.full_name())
        .unwrap_or_else(|| "pull requests".into());
    let mut pane = if let Some(repo) = p.repository() {
        if let Some(pulls) = &repo.pulls {
            if pulls.items.is_empty() {
                PaneView::new(
                    PaneId::Middle,
                    title,
                    Node::State {
                        state: v::State::Empty,
                        text: if pulls.completion.complete {
                            "No open pull requests."
                        } else {
                            "No pull requests in this partial result."
                        }
                        .into(),
                    },
                )
            } else {
                let rows = pulls
                    .items
                    .iter()
                    .enumerate()
                    .map(|(i, pull)| {
                        ListRow::new(
                            format!("pull:{i}"),
                            Text::plain(format!("#{}  {}", pull.number, pull.title)),
                        )
                        .right(Text::of(super::pull_state(pull), Ink::Secondary))
                    })
                    .collect();
                PaneView::new(
                    PaneId::Middle,
                    title,
                    Node::List {
                        rows,
                        selected: Some(p.pull_at),
                        editing: None,
                    },
                )
            }
        } else {
            PaneView::new(
                PaneId::Middle,
                title,
                Node::State {
                    state: if repo.loading {
                        v::State::Loading
                    } else {
                        v::State::Empty
                    },
                    text: if repo.loading {
                        "Loading open pull requests…"
                    } else {
                        "Pull requests have not been loaded."
                    }
                    .into(),
                },
            )
        }
    } else {
        PaneView::new(
            PaneId::Middle,
            title,
            Node::State {
                state: v::State::Empty,
                text: "No repositories in this group yet. Browse and select them in settings."
                    .into(),
            },
        )
    };
    pane.notes
        .push(Text::of("Open pull requests · read-only", Ink::Muted));
    if let Some(repo) = p.repository() {
        if let Some(error) = &repo.error {
            pane.notes.push(Text::of(
                format!("{}: {error}", repo.id.full_name()),
                Ink::Bad,
            ));
        }
        if let Some(pulls) = &repo.pulls
            && !pulls.completion.complete
        {
            pane.notes.push(Text::of(
                "Partial list: more pull requests may exist.",
                Ink::Warn,
            ));
        }
    }
    pane
}

fn detail_pane(p: &GithubPlugin, cx: &v::ViewCx<'_>) -> PaneView {
    let (title, mut blocks) = match super::selected_item(p) {
        Some(super::TreeItem::Pull(repo, index)) => {
            let pull = &p.repos[repo].pulls.as_ref().unwrap().items[index];
            (
                format!("{} #{}", pull.repository.full_name(), pull.number),
                pull_blocks(p, pull),
            )
        }
        Some(super::TreeItem::Repo(index)) => {
            let repo = &p.repos[index];
            (repo.id.full_name(), repository_blocks(p, repo))
        }
        Some(super::TreeItem::Group(index)) => {
            let group = &p.cfg.groups[index];
            (
                group.name.clone(),
                vec![
                    Block::line(Text::plain(format!(
                        "{} repositories",
                        super::group_repos(p, index).len()
                    ))),
                    Block::line(Text::plain("Choose a repository to see its details.")),
                ],
            )
        }
        None => (
            "about".into(),
            vec![Block::line(Text::plain("Choose a repository."))],
        ),
    };
    if let Some(target) = p.target(Pane::Detail)
        && let Some(line) = contributed(p, cx, &target, super::UnderLocation::Detail, &mut blocks)
    {
        blocks.insert(0, Block::line(line));
    }
    PaneView::new(
        PaneId::Detail,
        title,
        Node::Rich {
            blocks,
            pictures: None,
        },
    )
}

fn repository_blocks(p: &GithubPlugin, state: &crate::RepoState) -> Vec<Block> {
    let mut blocks = Vec::new();
    if state.loading && state.repository.is_none() {
        blocks.push(Block::line(Text::of("loading repository…", Ink::Info)));
    }
    if let Some(error) = &state.error {
        blocks.push(Block::line(Text::of(error, Ink::Bad)));
    }
    let Some(repo) = &state.repository else {
        blocks.push(Block::line(Text::of(
            "Repository details have not been loaded.",
            Ink::Muted,
        )));
        return blocks;
    };
    for (name, value) in [
        (
            "visibility",
            if repo.private { "private" } else { "public" }.to_owned(),
        ),
        ("default branch", repo.default_branch.clone()),
        (
            "archived",
            if repo.archived { "yes" } else { "no" }.to_owned(),
        ),
    ] {
        blocks.push(kv(name, value));
    }
    blocks.push(Block::line(Text::new()));
    blocks.push(Block::section(
        None,
        "actions",
        None,
        crate::actions::repository_summary(p, state)
            .into_iter()
            .map(Block::line)
            .collect(),
    ));
    blocks.push(Block::line(Text::new()));
    blocks.push(description(
        p,
        repo.description.as_deref().unwrap_or("No description."),
    ));
    blocks
}

fn pull_blocks(p: &GithubPlugin, pull: &crate::model::PullRequest) -> Vec<Block> {
    let mut blocks = vec![
        Block::line(Text::of(&pull.title, Ink::Emphasis)),
        Block::line(Text::new()),
    ];
    for (name, value) in [
        ("state", super::pull_state(pull).to_owned()),
        ("author", pull.author.clone()),
        (
            "from",
            pull.head_repository
                .as_ref()
                .map(|r| format!("{}:{}", r.full_name(), pull.head_branch))
                .unwrap_or_else(|| "unavailable source repository".into()),
        ),
        (
            "into",
            format!("{}:{}", pull.repository.full_name(), pull.base_branch),
        ),
        ("head commit", pull.head_sha.clone()),
    ] {
        blocks.push(kv(name, value));
    }
    blocks.push(Block::line(Text::new()));
    blocks.push(description(
        p,
        pull.body.as_deref().unwrap_or("No description."),
    ));
    blocks
}

fn kv(name: &str, value: String) -> Block {
    Block::line(Text::from(vec![
        Seg::new(format!("{name}  "), Ink::Muted),
        Seg::new(value, Ink::Primary),
    ]))
}

/// The `DESCRIPTION` section: `source` as markdown, its safe links kept
/// for the page's link list.
fn description(p: &GithubPlugin, source: &str) -> Block {
    let mut doc = radar_core::ui::markdown::parse(source);
    super::safe_blocks(&mut doc.blocks);
    let mut links = p.links.borrow_mut();
    for link in doc.links() {
        if super::safe_link(&link.url) && !links.iter().any(|old| old.url == link.url) {
            links.push(link);
        }
    }
    Block::section(
        None,
        "description",
        None,
        vec![Block::Doc {
            doc,
            bar: None,
            most: None,
            urls: Some(false),
        }],
    )
}

fn contributed(
    p: &GithubPlugin,
    cx: &v::ViewCx<'_>,
    target: &Target,
    location: super::UnderLocation,
    blocks: &mut Vec<Block>,
) -> Option<Text> {
    let subject = p.subject_for(target);
    let mut under = Text::new();
    for (part, words) in cx.parts.lines(&subject) {
        if !under.is_empty() {
            under.add(Seg::new(" · ", Ink::Muted));
        }
        for word in words {
            let mut segment = Seg::chip(&word);
            if let Some(id) = &word.action {
                let mut actions = p.under_actions.borrow_mut();
                let index = actions.len();
                actions.push(super::UnderAction {
                    generation: p.generation,
                    target: target.clone(),
                    location,
                    part,
                    subject: subject.clone(),
                    id: id.clone(),
                });
                segment.target = Some(v::Target::new(format!("under:{}:{index}", p.action_epoch)));
            }
            under.add(segment);
        }
    }
    for section in cx.parts.detail_sections(&subject) {
        blocks.push(Block::line(Text::new()));
        blocks.push(section);
    }
    (!under.is_empty()).then_some(under)
}

fn page(p: &GithubPlugin, cx: &v::ViewCx<'_>) -> v::PageView {
    let current = p.pages.last().expect("page selected");
    let target = &current.target;
    let id = target.repository();
    let title = p
        .pull_detail
        .as_ref()
        .filter(|pull| matches!(target, Target::Pull(_, number) if pull.number == *number))
        .map(|pull| pull.title.clone())
        .unwrap_or_else(|| crate::actions::title(p).unwrap_or_else(|| id.full_name()));
    let mut blocks = if crate::actions::active(p) {
        crate::actions::neutral_blocks(p)
    } else if matches!(target, Target::Repository(_)) && current.tab == 1 {
        vec![]
    } else {
        match target {
            Target::Repository(_) => p
                .repository()
                .map(|repo| repository_blocks(p, repo))
                .unwrap_or_default(),
            Target::Pull(_, _) if current.tab == 0 => p
                .pull_detail
                .as_ref()
                .map(|pull| pull_blocks(p, pull))
                .unwrap_or_else(|| {
                    vec![Block::line(Text::plain(
                        "Pull request details have not been loaded.",
                    ))]
                }),
            Target::Pull(_, _) => check_blocks(p),
            Target::Run(..) => vec![],
        }
    };
    let under = if !crate::actions::active(p)
        && !(matches!(target, Target::Repository(_)) && current.tab == 1)
    {
        contributed(
            p,
            cx,
            target,
            super::UnderLocation::Page {
                request: p.page_request,
                tab: current.tab,
            },
            &mut blocks,
        )
    } else {
        None
    };
    let mut pane = if matches!(target, Target::Repository(_)) && current.tab == 1 {
        pulls(p)
    } else {
        PaneView::new(
            PaneId::Middle,
            super::tabs(target)[current.tab].clone(),
            Node::Rich {
                blocks,
                pictures: None,
            },
        )
    };
    pane.pane = PaneId::Middle;
    if p.page_loading && p.pull_detail.is_none() {
        pane.head = Some(Text::of("loading pull request and checks…", Ink::Info));
    }
    if let Some(error) = &p.page_error {
        pane.notes.push(Text::of(error, Ink::Bad));
    }
    let mut crumbs = vec![
        Seg::new("GitHub", Ink::Muted),
        Seg::new(id.full_name(), Ink::Secondary),
    ];
    match target {
        Target::Pull(_, number) => crumbs.push(Seg::new(format!("#{number}"), Ink::Secondary)),
        Target::Run(_, run, attempt) => crumbs.push(Seg::new(
            format!("run {run} · attempt {attempt}"),
            Ink::Secondary,
        )),
        Target::Repository(_) => {}
    }
    v::PageView {
        crumbs,
        title: Text::of(title, Ink::Emphasis),
        details: Text::new(),
        under,
        tabs: super::tabs(target)
            .into_iter()
            .map(|name| v::TabView::new(name, None))
            .collect(),
        tab: current.tab,
        notice: None,
        panes: vec![pane],
        layout: None,
    }
}

fn check_blocks(p: &GithubPlugin) -> Vec<Block> {
    let mut blocks = vec![Block::line(Text::of(
        "Checks in the base and available source repositories at the exact head commit. Merge-ref workflows may differ. No mergeability or required-check coverage is established.",
        Ink::Muted,
    ))];
    if let Some(error) = &p.checks_error {
        blocks.push(Block::line(Text::of(error, Ink::Bad)));
    }
    let Some(checks) = &p.checks else {
        blocks.push(Block::line(Text::of(
            "Checks are unknown; no results loaded.",
            Ink::Warn,
        )));
        return blocks;
    };
    blocks.push(kv("head commit", checks.head_sha.clone()));
    blocks.push(kv(
        "source repository",
        checks
            .head_repository
            .as_ref()
            .map(|r| r.full_name())
            .unwrap_or_else(|| "unavailable".into()),
    ));
    for error in &checks.errors {
        blocks.push(Block::line(Text::of(error, Ink::Bad)));
    }
    let mut runs = Vec::new();
    if !checks.check_runs.completion.complete {
        runs.push(Text::of(
            "Partial or unavailable check runs; result unknown.",
            Ink::Warn,
        ));
        if let Some(reason) = &checks.check_runs.completion.reason {
            runs.push(Text::of(reason, Ink::Warn));
        }
    }
    for run in &checks.check_runs.items {
        runs.push(Text::plain(format!(
            "{}  {}  ({})",
            run.status_label(),
            run.name,
            run.repository.full_name()
        )));
    }
    if checks.check_runs.items.is_empty() {
        runs.push(Text::of(
            "No check runs reported. This does not mean checks passed.",
            Ink::Muted,
        ));
    }
    blocks.push(Block::line(Text::new()));
    blocks.push(Block::section_of(None, "check runs", None, runs));
    let mut statuses = Vec::new();
    if !checks.statuses.completion.complete {
        statuses.push(Text::of(
            "Partial or unavailable statuses; result unknown.",
            Ink::Warn,
        ));
        if let Some(reason) = &checks.statuses.completion.reason {
            statuses.push(Text::of(reason, Ink::Warn));
        }
    }
    for status in &checks.statuses.items {
        statuses.push(Text::plain(format!(
            "{}  {}  ({})",
            status.status_label(),
            status.context,
            status.repository.full_name()
        )));
        if let Some(description) = &status.description {
            statuses.push(Text::of(description, Ink::Secondary));
        }
    }
    if checks.statuses.items.is_empty() {
        statuses.push(Text::of(
            "No commit statuses reported. This does not mean checks passed.",
            Ink::Muted,
        ));
    }
    blocks.push(Block::line(Text::new()));
    blocks.push(Block::section_of(None, "commit statuses", None, statuses));
    blocks
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::config::{Group, Settings};
    use crate::model::{Collection, Completion, PullRequest, RepositoryId};
    use crate::screen::{TreeItem, tree_item_at, tree_path};

    /// A small owned model only: no configuration discovery, clock, transport,
    /// host or rich demo construction. Also reused by the host fold contract.
    pub(crate) fn tree_fixture() -> GithubPlugin {
        let mut p = GithubPlugin { cfg: Settings {
            groups: vec![
                Group { name: "Watch".into(), owner: None, repositories: vec!["ankh-morpork/watch".into()] },
                Group { name: "Witches".into(), owner: None, repositories: vec!["lancre/witches".into()] },
            ],
            ..Settings::default()
        }.validated().unwrap(), ..GithubPlugin::default() };
        for name in ["ankh-morpork/watch", "lancre/witches"] {
            let id = RepositoryId::parse(name).unwrap();
            let mut repo = crate::RepoState::new(id.clone());
            repo.pulls = Some(Collection {
                items: [12, 13].into_iter().map(|number| PullRequest {
                    repository: id.clone(), number,
                    title: format!("Watch patrol {number}"), body: None,
                    author: "vimes".into(), state: "open".into(), draft: number == 13,
                    head_sha: "a".repeat(40), head_repository: None,
                    head_branch: "patrol".into(), base_branch: "main".into(),
                    updated_at: String::new(), merged: false,
                }).collect(),
                completion: Completion::complete(1),
            });
            p.repos.push(repo);
        }
        p.started = true;
        p
    }

    pub(crate) fn tree(p: &GithubPlugin) -> v::Tree {
        let Node::Tree(tree) = middle_pane(p).body else { panic!("repositories must be a tree") };
        tree
    }

    fn path(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| (*part).into()).collect()
    }

    #[test]
    fn tree_paths_roundtrip_and_survive_group_repository_and_pull_reordering() {
        let mut p = tree_fixture();
        let before = tree(&p);
        assert_eq!(before.selected, path(&["group:Watch", "repo:ankh-morpork/watch", "pull:12"]));
        for (path, _) in before.visible() {
            let item = tree_item_at(&p, &path).expect("every visible row resolves");
            assert_eq!(tree_path(&p, item).as_ref(), Some(&path));
            assert_eq!(TreeRow::path_of(&TreeRow::target_of(&path)), path);
        }
        let watch = path(&["group:Watch", "repo:ankh-morpork/watch", "pull:12"]);
        let witches = path(&["group:Witches", "repo:lancre/witches", "pull:12"]);
        assert_ne!(TreeRow::target_of(&watch), TreeRow::target_of(&witches));
        p.cfg.groups.swap(0, 1);
        p.repos.swap(0, 1);
        p.repos[1].pulls.as_mut().unwrap().items.swap(0, 1);
        assert_eq!(tree_item_at(&p, &watch), Some(TreeItem::Pull(1, 1)));
        assert_eq!(tree_item_at(&p, &witches), Some(TreeItem::Pull(0, 0)));
        assert_eq!(tree_path(&p, TreeItem::Pull(1, 1)), Some(watch));
    }

    #[test]
    fn tree_targets_reject_wrong_ancestors_hidden_rows_and_replaced_pulls() {
        let mut p = tree_fixture();
        let watch = path(&["group:Watch", "repo:ankh-morpork/watch", "pull:12"]);
        for invalid in [
            path(&["group:Witches", "repo:ankh-morpork/watch", "pull:12"]),
            path(&["group:Watch", "repo:lancre/witches", "pull:12"]),
            path(&["repo:ankh-morpork/watch", "pull:12"]),
            path(&["group:Watch", "repo:ankh-morpork/watch", "pull:12", "extra"]),
            path(&["tree:2"]),
        ] {
            assert_eq!(tree_item_at(&p, &invalid), None);
        }
        p.folded_repos.insert(0);
        assert_eq!(tree_item_at(&p, &watch), None);
        p.folded_repos.clear();
        p.folded_groups.insert(0);
        assert_eq!(tree_item_at(&p, &watch[..2]), None);
        p.folded_groups.clear();
        p.group_at = 2;
        assert_eq!(tree_item_at(&p, &watch), None);
        p.group_at = 0;
        p.repos[0].pulls.as_mut().unwrap().items.remove(0);
        assert_eq!(tree_item_at(&p, &watch), None, "a refresh replacement cannot inherit the target");
    }

    #[test]
    fn freeform_group_names_cannot_alias_path_separators_or_escapes() {
        let mut p = tree_fixture();
        p.cfg.groups[0].name = "Watch\u{1f}%1F".into();
        let path = tree_path(&p, TreeItem::Pull(0, 0)).unwrap();
        assert_eq!(path[0], "group:Watch%1F%251F");
        assert_eq!(TreeRow::path_of(&TreeRow::target_of(&path)), path);
        assert_eq!(tree_item_at(&p, &path), Some(TreeItem::Pull(0, 0)));
        assert_ne!(crate::screen::group_id("Watch\u{1f}"), crate::screen::group_id("Watch%1F"));
    }

    #[test]
    fn fold_intent_changes_open_paths_without_rebuilding_or_losing_children() {
        let mut p = tree_fixture();
        p.tree_at = Some(TreeItem::Group(0));
        let target = TreeRow::target_of(&path(&["group:Watch"]));
        let before = tree(&p);
        assert!(matches!(intent(&mut p, Pane::Middle, &v::Intent::Fold { target: target.clone(), open: false }), Outcome::Handled(_)));
        let folded = tree(&p);
        assert_eq!(folded.rows, before.rows);
        assert_eq!(folded.columns, before.columns);
        assert_eq!(folded.selected, before.selected);
        assert_eq!(folded.open, before.open.into_iter().filter(|path| TreeRow::target_of(path) != target).collect::<Vec<_>>());
        assert_eq!(folded.visible().len(), 5);
        assert!(matches!(intent(&mut p, Pane::Middle, &v::Intent::Fold { target, open: true }), Outcome::Handled(_)));
        assert_eq!(tree(&p).visible().len(), 8);

        p.tree_at = Some(TreeItem::Repo(0));
        let repo = TreeRow::target_of(&path(&["group:Watch", "repo:ankh-morpork/watch"]));
        assert!(matches!(intent(&mut p, Pane::Middle, &v::Intent::Fold { target: repo, open: false }), Outcome::Handled(_)));
        assert!(p.folded_repos.contains(&0));
        assert_eq!(tree(&p).rows, folded.rows);
        let leaf = TreeRow::target_of(&path(&["group:Witches", "repo:lancre/witches", "pull:12"]));
        assert!(matches!(intent(&mut p, Pane::Middle, &v::Intent::Fold { target: leaf, open: false }), Outcome::Ignored));
    }

    #[test]
    fn raw_fold_keys_and_group_activate_keep_existing_selection_and_page_behavior() {
        let mut p = tree_fixture();
        for item in [TreeItem::Group(0), TreeItem::Repo(0)] {
            p.tree_at = Some(item);
            let path = tree_path(&p, item).unwrap();
            for (key, open) in [('-', false), ('+', true), (' ', false), (' ', true)] {
                assert!(matches!(intent(&mut p, Pane::Middle, &v::Intent::Key { chord: v::Chord::char(key) }), Outcome::Handled(_)));
                assert_eq!(tree(&p).open.contains(&path), open);
                assert_eq!(p.tree_at, Some(item));
                assert!(p.pages.is_empty());
            }
        }
        let target = TreeRow::target_of(&path(&["group:Watch"]));
        assert!(matches!(intent(&mut p, Pane::Middle, &v::Intent::Activate { target }), Outcome::Handled(_)));
        assert!(p.folded_groups.contains(&0));
        assert_eq!(p.tree_at, Some(TreeItem::Group(0)));
        assert!(p.pages.is_empty());
    }

    #[test]
    fn routed_minus_on_a_pull_or_closed_repository_collapses_and_selects_its_parent() {
        for (item, parent) in [
            (TreeItem::Pull(0, 0), TreeItem::Repo(0)),
            (TreeItem::Repo(0), TreeItem::Group(0)),
        ] {
            let mut p = tree_fixture();
            p.tree_at = Some(item);
            if matches!(item, TreeItem::Repo(_)) { p.folded_repos.insert(0); }
            let before = tree(&p);
            let routed = v::tree_step(&before, v::Chord::char('-')).expect("shared parent fold");
            assert!(matches!(intent(&mut p, Pane::Middle, &routed), Outcome::Handled(_)));
            assert_eq!(p.tree_at, Some(parent));
            assert_eq!(tree(&p).selected, tree_path(&p, parent).unwrap());
            assert!(!tree(&p).open.contains(&tree_path(&p, parent).unwrap()));
            assert_eq!(tree(&p).rows, before.rows);
        }
    }

    #[test]
    fn opening_a_visible_unselected_group_preserves_selection_and_rejects_empty_groups() {
        let mut p = tree_fixture();
        p.tree_at = Some(TreeItem::Pull(0, 0));
        p.folded_groups.insert(1);
        let target = TreeRow::target_of(&tree_path(&p, TreeItem::Group(1)).unwrap());
        assert!(matches!(intent(&mut p, Pane::Middle, &v::Intent::Fold { target, open: true }), Outcome::Handled(_)));
        assert_eq!(p.tree_at, Some(TreeItem::Pull(0, 0)));
        p.cfg.groups.push(crate::config::Group { name: "Unseen University".into(), owner: None, repositories: vec![] });
        let target = TreeRow::target_of(&tree_path(&p, TreeItem::Group(2)).unwrap());
        assert!(matches!(intent(&mut p, Pane::Middle, &v::Intent::Fold { target, open: false }), Outcome::Ignored));
    }

    #[test]
    fn tree_labels_icons_counts_and_loading_states_remain_owner_data() {
        let mut p = tree_fixture();
        let initial = tree(&p);
        let group = &initial.rows[0];
        assert!(!group.heading, "group Enter must remain reachable");
        assert_eq!(group.name, Text::from(vec![Seg::icon(Icon::Group), Seg::new("Watch", Ink::Emphasis)]));
        assert_eq!(group.cells[0].text, Text::of("1 repos", Ink::Muted));
        let repo = &group.children[0];
        assert_eq!(repo.name, Text::from(vec![Seg::icon(Icon::Repo), Seg::new("ankh-morpork/watch", Ink::Primary)]));
        assert_eq!(repo.cells[0].text, Text::of("2 PRs", Ink::Muted));
        assert_eq!(repo.children[1].name, Text::plain("#13  Watch patrol 13"));
        assert_eq!(repo.children[1].cells[0].text, Text::of("draft", Ink::Secondary));

        p.repos[0].loading = true;
        assert_eq!(tree(&p).rows, initial.rows, "a background read keeps held rows");
        p.repos[0].pulls.as_mut().unwrap().completion.complete = false;
        assert_eq!(tree(&p).rows[0].children[0].cells[0].text, Text::of("2 PRs", Ink::Warn));
        p.repos[0].error = Some("unavailable".into());
        assert_eq!(tree(&p).rows[0].children[0].cells[0].text, Text::of("failed", Ink::Bad));
        p.repos[0].pulls = None;
        assert_eq!(tree(&p).rows[0].children[0].cells[0].text, Text::of("loading", Ink::Info));
    }

    /// GUIDE 2.12: folding is shown by indentation, never by a chevron or
    /// `[+]`/`[-]` written into a row's text.
    #[test]
    fn tree_rows_carry_no_fold_marker_folded_or_open() {
        let mut p = tree_fixture();
        let check = |p: &GithubPlugin| {
            let shown = format!("{:?}", middle_pane(p));
            for mark in ["▾", "▸", "▶", "›", "[+]", "[-]"] {
                assert!(!shown.contains(mark), "tree rows carry {mark:?}: {shown}");
            }
        };
        check(&p);
        p.folded_groups.insert(0);
        p.folded_repos.insert(0);
        check(&p);
    }
    #[test]
    fn stale_or_hidden_context_and_page_only_targets_are_ignored_on_browse() {
        let mut p = tree_fixture();
        p.tree_at = Some(TreeItem::Repo(0));
        let child = TreeRow::target_of(&tree_path(&p, TreeItem::Pull(0, 0)).unwrap());
        p.folded_repos.insert(0);
        for target in [child, v::Target::new("pull:0")] {
            let before = tree(&p);
            assert!(matches!(intent(&mut p, Pane::Middle, &v::Intent::Context { target: target.clone() }), Outcome::Ignored));
            assert!(matches!(intent(&mut p, Pane::Middle, &v::Intent::Activate { target }), Outcome::Ignored));
            assert_eq!(tree(&p), before);
            assert!(p.pages.is_empty());
        }
        let repo = p.repos[0].id.clone();
        p.open_target(crate::join::Target::Repository(repo));
        for tab in [0, 2] {
            p.pages.last_mut().unwrap().tab = tab;
            assert!(matches!(intent(&mut p, Pane::Middle, &v::Intent::Context { target: v::Target::new("pull:0") }), Outcome::Ignored));
        }
        p.pages.last_mut().unwrap().tab = 1;
        assert!(matches!(intent(&mut p, Pane::Middle, &v::Intent::Select { target: v::Target::new("pull:0") }), Outcome::Handled(_)));
    }

}
