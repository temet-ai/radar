//! GitHub screen and pages as renderer-neutral data.

use radar_core::plugin::{Outcome, Pane};
use radar_core::ui::icon::Icon;
use radar_core::view::{self as v, Block, Ink, ListRow, Node, PaneId, PaneView, Seg, Text};

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
        v::Intent::Fold { .. } | v::Intent::Show { .. } | v::Intent::End { .. } | v::Intent::EditSelect { .. } | v::Intent::EditUnselect | v::Intent::Dock(_) => Outcome::Ignored,
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

fn middle_pane(p: &GithubPlugin) -> PaneView {
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
    let selected = super::selected_item(p).and_then(|item| items.iter().position(|at| *at == item));
    let rows = items
        .iter()
        .enumerate()
        .map(|(position, item)| {
            let id = format!("tree:{position}");
            match *item {
                super::TreeItem::Group(index) => {
                    ListRow::heading(
                        id,
                        Text::from(vec![
                            Seg::icon(Icon::Group),
                            Seg::new(p.cfg.groups[index].name.clone(), Ink::Emphasis),
                        ]),
                    )
                    .right(Text::of(
                        format!("{} repos", super::group_repos(p, index).len()),
                        Ink::Muted,
                    ))
                }
                super::TreeItem::Repo(index) => {
                    let repo = &p.repos[index];
                    let mut row = ListRow::new(
                        id,
                        Text::from(vec![
                            Seg::new("  ", Ink::Muted),
                            Seg::icon(Icon::Repo),
                            Seg::new(repo.id.full_name(), Ink::Primary),
                        ]),
                    );
                    // A background read keeps showing what is held.
                    if repo.loading && repo.pulls.is_none() {
                        row.right = Some(Text::of("loading", Ink::Info));
                    } else if repo.error.is_some() {
                        row.right = Some(Text::of("failed", Ink::Bad));
                    } else if let Some(pulls) = &repo.pulls {
                        row.right = Some(Text::of(
                            format!("{} PRs", pulls.items.len()),
                            if pulls.completion.complete {
                                Ink::Muted
                            } else {
                                Ink::Warn
                            },
                        ));
                    }
                    row
                }
                super::TreeItem::Pull(repo, index) => {
                    let pull = &p.repos[repo].pulls.as_ref().unwrap().items[index];
                    ListRow::new(
                        id,
                        Text::plain(format!("    #{}  {}", pull.number, pull.title)),
                    )
                    .right(Text::of(super::pull_state(pull), Ink::Secondary))
                }
            }
        })
        .collect();
    PaneView::new(
        PaneId::Middle,
        "repositories",
        Node::List {
            rows,
            selected,
            editing: None,
        },
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
            .map(|l| Block::line(Text::plain(l.to_string())))
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
    for (title, lines) in cx.parts.sections(&subject) {
        blocks.push(Block::line(Text::new()));
        blocks.push(Block::section_of(
            None,
            &title,
            None,
            lines.iter().map(|line| Text::plain(line.to_string())).collect(),
        ));
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
mod tests {
    use super::*;

    /// GUIDE 2.12: folding is shown by indentation, never by a chevron or
    /// `[+]`/`[-]` written into a row's text.
    #[test]
    fn tree_rows_carry_no_fold_marker_folded_or_open() {
        let mut p = crate::demo::sample();
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
}
