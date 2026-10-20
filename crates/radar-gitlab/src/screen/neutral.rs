//! The repositories screen as renderer-neutral data. The terminal frontend
//! draws these nodes; other frontends receive the same state and targets.

use super::*;
use radar_core::view::{
    self as v, Block, ColumnSpec, Ink, Node as ViewNode, PaneId, PaneView, Seg, Text, TreeRow,
    detail::{self, DetailContent, DetailFact, DetailHeader, FootAction, FootActions},
};

pub fn view(p: &GitlabPlugin, cx: &v::ViewCx<'_>) -> v::ScreenView {
    let nav = nav_pane(p);
    let middle = middle_pane(p, cx);
    let detail = detail_pane(p, cx);
    let mut view = v::ScreenView::new(crate::SCREEN.id, vec![nav, middle, detail]);
    view.medium_from = Some(70);
    view.actions = crate::actions(&p.rows(Pane::from(cx.focus)));
    view
}

pub fn intent(p: &mut GitlabPlugin, pane: Pane, intent: &v::Intent) -> Outcome {
    match intent {
        v::Intent::Select { target } | v::Intent::Context { target }
            if target.as_str().starts_with("view:") =>
        {
            if let Some(i) = target
                .as_str()
                .strip_prefix("view:")
                .and_then(|s| s.parse::<usize>().ok())
            {
                p.view = i.min(p.cfg.views.len().saturating_sub(1));
            }
            Outcome::done()
        }
        v::Intent::Select { target } if target.as_str() == "authors" => {
            Outcome::Handled(vec![Effect::Menu(p.author_menu())])
        }
        v::Intent::Select { target }
        | v::Intent::Activate { target }
        | v::Intent::Context { target }
            if target.as_str().starts_with("state:") =>
        {
            let Some(id) = target.as_str().strip_prefix("state:") else {
                return Outcome::Ignored;
            };
            let before = p.tree.borrow().selected().to_vec();
            let path = nodes(p, p.visible()).into_iter().find_map(|tenancy| {
                for repo in tenancy.repos {
                    if repo
                        .mrs
                        .iter()
                        .any(|item| mr_id(&item.mr.project_path, item.mr.iid) == id)
                    {
                        return Some(vec![
                            tenancy_id(tenancy.source),
                            repo_id(&repo.project.full_path),
                            id.to_string(),
                        ]);
                    }
                }
                None
            });
            let Some(path) = path else {
                return Outcome::Ignored;
            };
            p.tree.borrow_mut().select(path);
            let mut fx = moved(p, &before);
            if matches!(intent, v::Intent::Context { .. }) {
                return Outcome::Handled(fx);
            }
            if let Some(mr) = mr_of_row(p, id).cloned() {
                use crate::list_actions::{Do, StateClick, act, state_click};
                match state_click(&mr) {
                    Some(StateClick::Rebase) => fx.extend(act(p, Do::Rebase, &mr)),
                    Some(StateClick::Merge) => fx.extend(act(p, Do::Merge, &mr)),
                    Some(StateClick::Why) => {
                        fx.extend(crate::pages::open_mr_pipeline(p, &mr.project_path, mr.iid))
                    }
                    None => {}
                }
            }
            Outcome::Handled(fx)
        }
        v::Intent::Select { target }
        | v::Intent::Context { target }
        | v::Intent::Activate { target } => {
            let before = p.tree.borrow().selected().to_vec();
            let path = TreeRow::path_of(target);
            // A target the tree does not hold (a pane's empty space) is not ours.
            if !targets(p).iter().any(|t| t.path == path) {
                return Outcome::Ignored;
            }
            p.tree.borrow_mut().select(path);
            let mut fx = moved(p, &before);
            if matches!(intent, v::Intent::Activate { .. }) && pane != Pane::Nav {
                // ⏎ goes in, to the selected item's page.
                fx.extend(crate::pages::open_from_tree(p));
            }
            Outcome::Handled(fx)
        }
        v::Intent::Fold { target, open } => {
            let path = TreeRow::path_of(target);
            if !targets(p).iter().any(|t| t.path == path) {
                return Outcome::Ignored;
            }
            if *open {
                p.tree.borrow_mut().open(path);
                return Outcome::done();
            }
            let before = p.tree.borrow().selected().to_vec();
            crate::nav::NavExt::close(&mut *p.tree.borrow_mut(), &path);
            // The cursor was on a row inside what closed: it goes to the group.
            if before.len() > path.len() && before.starts_with(&path) {
                p.tree.borrow_mut().select(path);
                return Outcome::Handled(moved(p, &before));
            }
            Outcome::done()
        }
        v::Intent::Key { .. } | v::Intent::Invoke { .. } => {
            intent.chord().map_or(Outcome::Ignored, |chord| {
                // ↑ is the host's (to the top bar), unless a field is being
                // typed into: then `k` is a letter.
                let typing = p.filter_input.is_some() || p.goto.is_some() || p.compose.is_some();
                if !typing
                    && matches!(chord.key, v::Key::Up | v::Key::Char('k'))
                    && !chord.ctrl
                    && !chord.alt
                {
                    return Outcome::Ignored;
                }
                p.on_key(pane, chord)
            })
        }
        v::Intent::EditSelect { anchor, extent } => match p.compose.as_mut() {
            Some(c) => {
                c.composer.input.select(*anchor, *extent);
                Outcome::done()
            }
            None => Outcome::Ignored,
        },
        v::Intent::EditUnselect => match p.compose.as_mut() {
            Some(c) => {
                c.composer.input.unselect();
                Outcome::done()
            }
            None => Outcome::Ignored,
        },
        v::Intent::Show { .. } | v::Intent::End { .. } | v::Intent::Dock(_) | v::Intent::Copy { .. } | v::Intent::NoticeInput { .. } | v::Intent::DismissNotice { .. } | v::Intent::OpenAll => Outcome::Ignored,
    }
}

fn nav_pane(p: &GitlabPlugin) -> PaneView {
    let rows = p
        .cfg
        .views
        .iter()
        .zip(p.view_counts())
        .enumerate()
        .map(|(i, (view, count))| {
            let icon = view_icon(&view.filter);
            let mut row = v::ListRow::new(
                format!("view:{i}"),
                Text::from(vec![
                    Seg::icon(icon),
                    Seg::new(view.name.clone(), Ink::Plain),
                ]),
            );
            if icon == Icon::NeedsApproval && count > 0 {
                row.right = Some(Text::of(count.to_string(), Ink::Number));
            }
            row
        })
        .collect();
    let mut pane = PaneView::new(
        PaneId::Nav,
        "views",
        ViewNode::List {
            rows,
            selected: Some(p.view),
            editing: None,
        },
    );
    pane.width = Some(26);
    pane
}

fn columns(lead: u16) -> Vec<ColumnSpec> {
    let over = lead.saturating_sub(2);
    vec![
        ColumnSpec::new(STATE, 20 + over, 6).icon(Icon::Pipeline),
        ColumnSpec::new("approvals", 11 + over, 1).icon(Icon::Approved),
        ColumnSpec::new("threads", 8 + lead, 2).icon(Icon::Thread),
        ColumnSpec::new("updated", 8 + lead, 4).icon(Icon::Updated),
        ColumnSpec::new(FOR_YOU, 9 + lead, 5).icon(Icon::You),
    ]
}

fn extra_columns(
    p: &GitlabPlugin,
    cx: &v::ViewCx<'_>,
    nodes: &[TenancyNode<'_>],
) -> Vec<ColumnSpec> {
    let mut out: Vec<ColumnSpec> = Vec::new();
    for tn in nodes {
        for repo in &tn.repos {
            let subjects = std::iter::once(join::repo(p, &repo.project.full_path))
                .chain(repo.mrs.iter().map(|it| join::of_mr(it.mr)));
            for subject in subjects {
                for cell in cx.parts.cells(&subject) {
                    let lead = cell.icon.map_or(0, |icon| ui::width(&icon::plain(icon)));
                    let width = ((lead + ui::width(&cell.text)).max(ui::width(&cell.heading)) + 1)
                        .min(24) as u16;
                    match out.iter_mut().find(|col| col.heading == cell.heading) {
                        Some(col) => col.width = col.width.max(width),
                        None => {
                            let mut col = ColumnSpec::extra(cell.heading, width);
                            col.keep = 3;
                            out.push(col);
                        }
                    }
                }
            }
        }
    }
    out
}

fn middle_pane(p: &GitlabPlugin, cx: &v::ViewCx<'_>) -> PaneView {
    let nodes = nodes(p, p.visible());
    let title = p
        .current_view()
        .map_or_else(|| "repositories".to_string(), |view| view.name.clone());
    let mut pane = PaneView::new(PaneId::Middle, title, tree_body(p, cx, &nodes));
    if let Some(g) = &p.goto {
        let hint = if g.input.value().trim().is_empty() {
            Text::of(
                "go to: part of a tenancy, repo or merge request title, or its number",
                Ink::Muted,
            )
        } else if let Some(name) = &g.found {
            Text::from(vec![
                Seg::new("at ", Ink::Muted),
                Seg::new(name.clone(), Ink::Secondary),
            ])
        } else {
            Text::of("nothing by that name; esc goes back", Ink::Warn)
        };
        pane.fields.push(v::FieldBar {
            mark: " : ".into(),
            text: g.input.value(),
            caret: Some(g.input.cursor()),
            hint,
        });
    }
    if p.filter_input.is_some() || !p.filter.is_empty() {
        pane.fields.push(v::FieldBar {
            mark: " / ".into(),
            text: p
                .filter_input
                .as_ref()
                .map_or_else(|| p.filter.clone(), TextInput::value),
            caret: p.filter_input.as_ref().map(TextInput::cursor),
            hint: Text::of(crate::filter::describe(&p.filter), Ink::Muted),
        });
    }
    pane.notes = shows_lines(p);
    pane.notes_most = Some(3);
    pane
}

fn shows_lines(p: &GitlabPlugin) -> Vec<Text> {
    let mut lines: Vec<Text> = p
        .sync
        .errors
        .iter()
        .map(|(group, why)| {
            let code = p
                .cfg
                .sources
                .iter()
                .find(|source| p.cfg.group_path(source) == *group)
                .map_or("?", |source| source.code.as_str());
            v::kit::sync_failed(code, &p.sync_failure(group, why))
        })
        .collect();
    if let Some(view) = p.current_view() {
        let mut words = if view.filter.trim().is_empty() {
            vec!["every open merge request".into()]
        } else {
            view.filter
                .split_whitespace()
                .map(crate::filter::describe_term)
                .collect()
        };
        if Filter::parse(&view.filter).wants_ended() {
            words.push(format!(
                "merged and closed ones from the last {} days",
                p.cfg.history_days
            ));
        } else if !view.filter.trim().is_empty() {
            words.push("open ones only".into());
        }
        words.push(
            if only_busy(p) {
                "listing only repos that have one"
            } else {
                "listing every repo, also those without"
            }
            .into(),
        );
        let first = words.remove(0);
        let mut summary = Text::of(first, Ink::Secondary);
        if !words.is_empty() {
            summary.add(Seg::new(format!(" · {}", words.join(" · ")), Ink::Muted));
        }
        lines.push(summary);
    }
    lines
}

fn tree_body(p: &GitlabPlugin, cx: &v::ViewCx<'_>, nodes: &[TenancyNode<'_>]) -> ViewNode {
    if p.cfg.sources.is_empty() {
        return ViewNode::State {
            state: v::State::Empty,
            text: "No tenancies yet. Press , for settings, then add one under tenancies.".into(),
        };
    }
    if p.data.is_empty() {
        return ViewNode::State {
            state: if p.sync.in_flight > 0 {
                v::State::Loading
            } else {
                v::State::Empty
            },
            text: if p.sync.in_flight > 0 {
                "Loading from GitLab…"
            } else {
                "No data yet. Press r to refresh."
            }
            .into(),
        };
    }
    let rows = tree_rows(p, cx, nodes);
    open_new(p, &rows);
    let mut columns = columns(cx.icon_lead as u16);
    columns.splice(1..1, extra_columns(p, cx, nodes));
    let who = p.author.as_deref().unwrap_or("everyone");
    let summary = repo_summary(nodes, only_busy(p));
    let header = Text::from(vec![
        Seg::new("OPENED BY ", Ink::Plain),
        Seg::icon(if p.author.is_some() {
            Icon::Person
        } else {
            Icon::Team
        }),
        if p.author.is_some() {
            Seg::new(who, Ink::AccentSecondary).strong()
        } else {
            Seg::new(who, Ink::Secondary)
        },
        Seg::new(format!(" · {summary}"), Ink::Muted),
    ]);
    let state = p.tree.borrow();
    ViewNode::Tree(v::Tree {
        header,
        columns,
        ordered_columns: true,
        rows,
        selected: state.selected().to_vec(),
        open: crate::nav::NavExt::open_sorted(&*state),
        name_width: NAME as u16,
        look: Some(v::TreeLook {
            lines: p.cfg.style_for("lines"),
            band: p.cfg.style_for("band"),
            heading: p.cfg.style_for("heading"),
            band_level: 1,
        }),
        header_target: Some(v::Target::new("authors")),
        note: None,
        cut_names: false,
        editing: None,
        pointer_action: false,
    })
}

fn open_new(p: &GitlabPlugin, rows: &[TreeRow]) {
    let mut state = p.tree.borrow_mut();
    let mut seen = p.opened_once.borrow_mut();
    for tenancy in rows {
        let top = vec![tenancy.id.clone()];
        if seen.insert(top.clone()) {
            state.open(top.clone());
        }
        for repo in &tenancy.children {
            let path = vec![tenancy.id.clone(), repo.id.clone()];
            if !repo.children.is_empty() && seen.insert(path.clone()) {
                state.open(path);
            }
        }
    }
    if state.selected().is_empty()
        && let Some(first) = rows.first()
    {
        state.select(vec![first.id.clone()]);
    }
}

fn tree_rows(p: &GitlabPlugin, cx: &v::ViewCx<'_>, nodes: &[TenancyNode<'_>]) -> Vec<TreeRow> {
    nodes
        .iter()
        .map(|tenancy| {
            let red = tenancy
                .repos
                .iter()
                .filter(|repo| repo.project.main_pipeline == Some(Pipeline::Failed))
                .count();
            let mut name = Text::from(vec![
                Seg::icon(Icon::Group),
                Seg::styled(
                    format!("{} ", tenancy.source.code.to_uppercase()),
                    format!(
                        "tenant.{} {}",
                        tenancy.source.color,
                        p.cfg.style_for("tenant")
                    ),
                ),
                Seg::new(
                    tenancy
                        .source
                        .group
                        .rsplit('/')
                        .next()
                        .unwrap_or(&tenancy.source.group),
                    Ink::Emphasis,
                )
                .strong(),
            ]);
            if red > 0 {
                name.add(Seg::new("   ", Ink::Muted));
                name.add(Seg::styled(
                    format!("main red in {red}"),
                    p.cfg.style_for("failed"),
                ));
            }
            TreeRow::new(tenancy_id(tenancy.source), name)
                .stripe(format!("tenant.{}", tenancy.source.color))
                .children(tenancy.repos.iter().map(|repo| repo_row(p, cx, repo)))
        })
        .collect()
}

fn repo_row(p: &GitlabPlugin, cx: &v::ViewCx<'_>, repo: &RepoNode<'_>) -> TreeRow {
    let subject = join::repo(p, &repo.project.full_path);
    let busy = !repo.mrs.is_empty();
    let mark = cx.parts.mark(&subject);
    let mut name = Text::from(vec![
        Seg::icon(mark.as_ref().map_or(Icon::Repo, |(icon, _)| *icon)),
        if busy {
            Seg::new(repo.project.name.clone(), Ink::Primary).strong()
        } else {
            Seg::new(repo.project.name.clone(), Ink::Secondary)
        },
    ]);
    if let Some((_, words)) = mark {
        for word in words {
            if let Some(icon) = word.icon {
                name.add(Seg::icon(icon));
            }
            let mut seg = Seg::new(word.text, Ink::from(word.tone));
            if let Some(url) = word.url {
                seg = seg.linked(url);
            }
            name.add(seg);
        }
    }
    let state = match repo.project.main_pipeline {
        Some(pipeline) => Text::from(vec![
            Seg::icon(pipeline.mark().icon()),
            Seg::styled(pipeline.word(), p.cfg.style_for(pipeline.word())),
        ]),
        None if repo.project_loaded => Text::of("— no recent run", Ink::Muted),
        None => Text::new(),
    };
    let mut row = TreeRow::new(repo_id(&repo.project.full_path), name)
        .chips(1, chip_text(&cx.parts.chips(&subject)))
        .cell(STATE, state)
        .cell(
            "updated",
            Text::of(
                time::ago_str(&repo.project.last_activity, cx.now),
                Ink::Muted,
            ),
        )
        .cell(
            FOR_YOU,
            if busy {
                Text::from(vec![
                    Seg::icon(Icon::Change),
                    Seg::new(
                        format!(
                            "{} MR{}",
                            repo.mrs.len(),
                            if repo.mrs.len() == 1 { "" } else { "s" }
                        ),
                        Ink::Number,
                    ),
                ])
            } else {
                Text::of("—", Ink::Muted)
            },
        );
    for cell in cx.parts.cells(&subject) {
        row = row.cell_view(v::cell_of(&cell));
    }
    row.children(repo.mrs.iter().map(|item| mr_row(p, cx, item)))
}

fn mr_row(p: &GitlabPlugin, cx: &v::ViewCx<'_>, item: &Item<'_>) -> TreeRow {
    let mr = item.mr;
    let subject = join::of_mr(mr);
    let mine = mr.author.username == p.me;
    let who = if mine {
        "you".into()
    } else {
        ui::short_name(&mr.author.name)
    };
    let name = Text::from(vec![
        Seg::icon(mr_icon(mr)),
        Seg::new(format!("!{} ", mr.iid), Ink::AccentSecondary),
        Seg::new(mr.title.clone(), Ink::Primary),
        Seg::new("  ", Ink::Muted),
        Seg::icon(person_icon(p, &mr.author.username)),
        if mine {
            Seg::styled(who, p.cfg.style_for("you"))
        } else {
            Seg::new(who, Ink::Muted)
        },
    ]);
    let blocks = crate::ready::blocks(mr);
    let state = if let Some(first) = blocks.first() {
        Text::from(vec![
            Seg::icon(first.icon()),
            if matches!(first.tone(), Tone::Warn | Tone::Bad) {
                Seg::new(first.words(), Ink::from(first.tone())).strong()
            } else {
                Seg::new(first.words(), Ink::from(first.tone()))
            },
        ])
    } else {
        Text::from(vec![
            Seg::icon(Icon::RunPassed),
            Seg::new("ready to merge", Ink::Good),
        ])
    };
    let open = mr.threads.saturating_sub(mr.threads_resolved);
    let threads = match (mr.threads, open) {
        (0, _) => Text::of("—", Ink::Muted),
        (_, 0) => Text::from(vec![
            Seg::icon(Icon::Resolved),
            Seg::new("resolved", Ink::Muted),
        ]),
        (_, n) => Text::from(vec![
            Seg::icon(Icon::Thread),
            Seg::new(format!("{n} open"), Ink::Warn),
        ]),
    };
    let mut row = TreeRow::new(mr_id(&mr.project_path, mr.iid), name)
        .chips(2, chip_text(&cx.parts.chips(&subject)))
        .cell(STATE, state)
        .cell(
            "approvals",
            if mr.approved {
                Text::from(vec![
                    Seg::icon(Icon::Approved),
                    Seg::new(mr.approvals_words(), Ink::Good),
                ])
            } else {
                Text::of(mr.approvals_words(), Ink::Secondary)
            },
        )
        .cell("threads", threads)
        .cell(
            "updated",
            Text::of(time::ago_str(&mr.updated_at, cx.now), Ink::Muted),
        )
        .cell(
            FOR_YOU,
            Text::from(vec![Seg::styled(item.word(), p.cfg.style_for(item.word()))]),
        );
    row.glow = Some(join::motion_key(mr));
    for cell in cx.parts.cells(&subject) {
        row = row.cell_view(v::cell_of(&cell));
    }
    if crate::list_actions::state_click(mr).is_some()
        && let Some(cell) = row.cells.iter_mut().find(|cell| cell.heading == STATE)
    {
        cell.target = Some(v::Target::new(format!("state:{}", row.id)));
    }
    row
}

fn chip_text(chips: &[radar_core::compose::Chip]) -> Text {
    let mut out = Text::new();
    for chip in chips {
        out.add(Seg::new(" ", Ink::Plain));
        out.add(Seg::chip(chip));
    }
    out
}

fn detail_pane(p: &GitlabPlugin, cx: &v::ViewCx<'_>) -> PaneView {
    let selected = selected(p, p.visible());
    let title = name_of(p, selected.as_ref());
    let blocks = match selected {
        Some(Selected::Tenancy(i)) => tenancy_detail(p, i).into_iter().map(Block::line).collect(),
        Some(Selected::Repo(ref repo, ref mrs)) => repo_detail(p, repo, mrs, cx.now),
        Some(Selected::Mr(ref item)) => {
            return detail::pane(title, mr_detail(p, item, cx), mr_foot(p, item.mr));
        }
        None => {
            let mut blocks = vec![Block::line(Text::of(
                "Select a tenancy, repo or merge request.",
                Ink::Muted,
            ))];
            for (group, err) in &p.sync.errors {
                blocks.push(Block::line(Text::new()));
                blocks.push(Block::line(v::kit::sync_failed(
                    group,
                    &p.sync_failure(group, err),
                )));
            }
            blocks
        }
    };
    PaneView::new(
        PaneId::Detail,
        title,
        ViewNode::Rich {
            blocks,
            pictures: None,
        },
    )
}

fn tenancy_detail(p: &GitlabPlugin, i: usize) -> Vec<Text> {
    let source = &p.cfg.sources[i];
    let nodes = nodes(p, p.visible());
    let tenancy = nodes.iter().find(|node| node.index == i);
    let repos = tenancy.map_or(0, |node| node.repos.len());
    let busy = tenancy.map_or(0, |node| {
        node.repos
            .iter()
            .filter(|repo| !repo.mrs.is_empty())
            .count()
    });
    let red = tenancy.map_or(0, |node| {
        node.repos
            .iter()
            .filter(|repo| repo.project.main_pipeline == Some(Pipeline::Failed))
            .count()
    });
    let mrs = tenancy.map_or(0, |node| {
        node.repos
            .iter()
            .flat_map(|repo| repo.mrs.iter())
            .filter(|item| item.mr.state == MrState::Opened)
            .count()
    });
    let mut lines = vec![
        Text::from(vec![
            Seg::icon(Icon::Group),
            Seg::styled(
                format!("{} ", source.code),
                format!("tenant.{} bold", source.color),
            ),
            Seg::new(p.cfg.group_path(source), Ink::Emphasis).strong(),
        ]),
        Text::new(),
        v::kit::labelled(
            "repos",
            12,
            Text::plain(format!("{repos} shown · {} hidden", source.hide.len())),
        ),
        v::kit::labelled(
            "with MRs",
            12,
            Text::plain(format!("{busy} repos, in this view")),
        ),
        v::kit::labelled("open MRs", 12, Text::of(mrs.to_string(), Ink::Number)),
    ];
    if red > 0 {
        lines.push(v::kit::labelled(
            "main red",
            12,
            Text::new()
                .push(Seg::icon(Icon::Error))
                .push(Seg::new(format!("{red} repo{}", if red == 1 { "" } else { "s" }), Ink::Bad)),
        ));
    }
    lines
}

fn repo_detail(p: &GitlabPlugin, repo: &Project, mrs: &[Item<'_>], now: i64) -> Vec<Block> {
    let branch = if repo.default_branch.is_empty() {
        "main"
    } else {
        &repo.default_branch
    };
    let main = repo.main_pipeline.map_or_else(
        || Text::of("no recent pipeline", Ink::Muted),
        |pipeline| {
            Text::from(vec![
                Seg::icon(pipeline.mark().icon()),
                Seg::styled(pipeline.word(), p.cfg.style_for(pipeline.word())),
            ])
        },
    );
    let mut lines = vec![
        Text::from(vec![
            Seg::icon(Icon::Repo),
            Seg::new(repo.full_path.clone(), Ink::Emphasis).strong(),
        ]),
        Text::new(),
        v::kit::labelled(branch, 12, main),
        v::kit::labelled(
            "active",
            12,
            Text::from(vec![
                Seg::icon(Icon::Updated),
                Seg::new(
                    format!("{} ago", time::ago_str(&repo.last_activity, now)),
                    Ink::Primary,
                ),
            ]),
        ),
    ];
    if let Some(words) = why::red(repo.main_pipeline, repo.main_pipeline_url.as_deref())
        .and_then(|pipeline| why::summary(p, &repo.full_path, pipeline))
    {
        lines.push(
            Text::plain("            ")
                .push(Seg::icon(Icon::Error))
                .push(Seg::new(words, Ink::Bad)),
        );
    }
    if let Some(commit) = &repo.main_commit {
        lines.push(v::kit::labelled(
            "last commit",
            12,
            Text::from(vec![
                Seg::icon(Icon::Commit),
                Seg::new(format!("{} ", commit.short_id), Ink::AccentSecondary),
                Seg::icon(Icon::Person),
                Seg::new(commit.author.clone(), Ink::Secondary),
                Seg::new(
                    format!(" · {} ago", time::ago_str(&commit.at, now)),
                    Ink::Muted,
                ),
            ]),
        ));
        lines.push(Text::plain(format!("            {}", commit.title)));
    }
    let mut blocks: Vec<Block> = lines.into_iter().map(Block::line).collect();
    blocks.push(Block::line(Text::new()));
    blocks.push(Block::section_of(
        Some(Icon::Change),
        "merge requests",
        Some(format!("{} in this view", mrs.len())),
        mrs.iter()
            .map(|item| {
                Text::from(vec![
                    Seg::icon(mr_icon(item.mr)),
                    Seg::new(
                        format!("{:<6}", format!("!{}", item.mr.iid)),
                        Ink::AccentSecondary,
                    ),
                    Seg::styled(format!("{:<8}", item.word()), p.cfg.style_for(item.word())),
                    Seg::new(item.mr.title.clone(), Ink::Primary),
                ])
            })
            .collect(),
    ));
    blocks
}

/// Controls come from the actual Detail menu, even while another pane has focus.
/// Contributed actions remain in that menu; they do not have direct owner IDs.
fn mr_foot(p: &GitlabPlugin, mr: &Mr) -> FootActions {
    let offered = crate::actions(&p.rows(Pane::Detail));
    let mut controls = Vec::new();
    for row in crate::list_actions::rows(p, mr) {
        use crate::list_actions::Do;
        let (id, caption) = match row.what {
            Do::Rebase => ("gitlab.repos.rebase", "update"),
            Do::Merge => ("gitlab.repos.merge", "merge"),
            Do::Approve if mr.approved_by.contains(&p.me) => {
                ("gitlab.repos.approve", "take back approval")
            }
            Do::Approve => ("gitlab.repos.approve", "approve"),
            Do::Open => ("gitlab.repos.open_link", "open"),
            Do::Copy => ("gitlab.repos.copy_link", "copy link"),
            Do::Pipeline => ("gitlab.repos.open_pipeline", "pipeline"),
        };
        if let Some(action) = offered.iter().find(|action| action.id == id) {
            let mut control = FootAction::new(action.clone()).caption(caption);
            if let Some(reason) = row.why {
                control = control.disabled(reason);
            }
            controls.push(control);
        }
    }
    if let Some(action) = offered
        .into_iter()
        .find(|action| action.id == "gitlab.repos.open")
    {
        controls.push(FootAction::new(action).caption("page"));
    }
    FootActions::new(controls)
}

/// How many lines of the last comment the detail pane shows.
const COMMENT_MOST: usize = 12;

fn mr_detail(p: &GitlabPlugin, item: &Item<'_>, cx: &v::ViewCx<'_>) -> DetailContent {
    let mr = item.mr;
    let who = if mr.author.username == p.me && !p.me.is_empty() {
        "you".into()
    } else {
        crate::author_name(mr)
    };
    let mut path = Text::from(vec![Seg::icon(Icon::Repo)]);
    let mut has_component = false;
    for component in [
        Seg::styled(item.tenant.code.clone(), format!("tenant.{} {}", item.tenant.color, p.cfg.style_for("tenant"))),
        Seg::new(mr.project_path.clone(), Ink::Secondary),
        Seg::new(if mr.iid > 0 { format!("!{}", mr.iid) } else { String::new() }, Ink::AccentSecondary),
    ] {
        if component.text.trim().is_empty() { continue; }
        if has_component { path.add(Seg::new(" / ", Ink::Secondary)); }
        path.add(component);
        has_component = true;
    }
    let mut header = DetailHeader::new(
        path,
        Text::from(vec![
            Seg::icon(mr_icon(mr)),
            Seg::new(mr.title.clone(), Ink::Emphasis).strong(),
        ]),
    );
    let branch = match (
        mr.source_branch.trim().is_empty(),
        mr.target_branch.trim().is_empty(),
    ) {
        (false, false) => format!("{} → {}", mr.source_branch, mr.target_branch),
        (false, true) => mr.source_branch.clone(),
        (true, false) => format!("into {}", mr.target_branch),
        (true, true) => String::new(),
    };
    if !branch.is_empty() {
        header.branch = Some(Text::from(vec![
            Seg::icon(Icon::Branch),
            Seg::new(branch, Ink::AccentSecondary),
        ]));
    }
    if !who.trim().is_empty() {
        header.author = Some(Text::from(vec![
            Seg::icon(person_icon(p, &mr.author.username)),
            Seg::new(who, Ink::Secondary),
        ]));
    }
    if !mr.updated_at.trim().is_empty() {
        header.age = Some(Text::of(
            format!("{} ago", time::ago_str(&mr.updated_at, cx.now)),
            Ink::Muted,
        ));
    }
    let mut facts = vec![
        DetailFact::new("project", Text::of(mr.project_name.clone(), Ink::Secondary)),
        DetailFact::new(
            "state",
            Text::from(vec![Seg::styled(
                mr.state.word(),
                p.cfg.style_for(item.word()),
            )]),
        ),
        DetailFact::new(
            "approvals",
            Text::of(
                format!(
                    "{} required · {} left",
                    mr.approvals_required, mr.approvals_left
                ),
                Ink::Number,
            ),
        ),
        DetailFact::new(
            "reviewers",
            Text::of(mr.reviewers.join(", "), Ink::Secondary),
        ),
        DetailFact::new(
            "approved by",
            Text::of(mr.approved_by.join(", "), Ink::Secondary),
        ),
    ];
    if !mr.labels.is_empty() {
        facts.push(DetailFact::new(
            "labels",
            Text::from(vec![
                Seg::icon(Icon::Label),
                Seg::styled(
                    mr.labels
                        .iter()
                        .map(|label| format!("#{label}"))
                        .collect::<Vec<_>>()
                        .join(" · "),
                    p.cfg.style_for("label"),
                ),
            ]),
        ));
    }
    let mut blocks = Vec::new();
    // What stands between it and merging, or how it ended.
    let mut state = if mr.state == MrState::Opened {
        let blockers = crate::ready::blocks(mr);
        if blockers.is_empty() {
            vec![Text::from(vec![
                Seg::icon(Icon::RunPassed),
                Seg::new("ready to merge", Ink::Good),
            ])]
        } else {
            blockers
                .into_iter()
                .map(|blocker| {
                    Text::from(vec![
                        Seg::icon(blocker.icon()),
                        Seg::new(blocker.words(), Ink::from(blocker.tone())),
                    ])
                })
                .collect()
        }
    } else {
        vec![Text::of(
            format!(
                "{} {} ago",
                mr.state.word(),
                time::ago_str(&mr.ended_at, cx.now)
            ),
            Ink::Secondary,
        )]
    };
    if mr.state == MrState::Opened {
        state.insert(
            0,
            Text::of(
                item.role.sentence(),
                Ink::from(match item.role {
                    Role::Review => Tone::Warn,
                    Role::Fix => Tone::Bad,
                    Role::Ready => Tone::Good,
                    Role::Waiting => Tone::Info,
                    _ => Tone::Muted,
                }),
            ),
        );
    }
    let subject = join::of_mr(mr);
    state.extend(
        cx.parts
            .chips(&subject)
            .iter()
            .map(|chip| Text::from(vec![Seg::chip(chip)])),
    );
    blocks.push(Block::section_of(Some(Icon::Info), "state", None, state));
    let pipeline = mr.pipeline.map_or_else(
        || "none".to_string(),
        |pipeline| pipeline.word().to_string(),
    );
    let pipeline = mr.duration_secs.map_or(pipeline.clone(), |seconds| {
        format!("{pipeline} · {}", ui::duration(seconds.into()))
    });
    let stages = mr
        .stages
        .iter()
        .map(|stage| v::MeterStage {
            name: stage.name.clone(),
            mark: status_mark(&stage.status),
            look: Pipeline::from_graphql(&stage.status.to_ascii_uppercase()).map_or_else(
                || "text.muted".into(),
                |pipeline| p.cfg.style_for(pipeline.word()),
            ),
        })
        .collect();
    // One cell per stage across the width: a bar, and its name under it;
    // when red, which job failed and why, once known.
    let mut run = vec![Block::Meter { stages }];
    if let Some(words) = why::red(mr.pipeline, mr.pipeline_url.as_deref())
        .and_then(|pipeline| why::summary(p, &mr.project_path, pipeline))
    {
        run.push(Block::line(v::kit::marked(Icon::Error, &words, Ink::Bad)));
    }
    blocks.push(Block::section(
        Some(Icon::Pipeline),
        "pipeline",
        Some(pipeline),
        run,
    ));
    blocks.push(Block::section_of(
        Some(Icon::File),
        "changes",
        None,
        vec![Text::from(vec![
            Seg::new(
                format!(
                    "{} file{}  ",
                    mr.files,
                    if mr.files == 1 { "" } else { "s" }
                ),
                Ink::Primary,
            ),
            Seg::new(format!("+{}", mr.additions), Ink::Added),
            Seg::new(format!(" −{}", mr.deletions), Ink::Removed),
        ])],
    ));
    // A plain comment is not a thread: with none, say what is shown.
    let thread_detail = match (mr.threads, mr.threads.saturating_sub(mr.threads_resolved)) {
        (0, _) if mr.last_comment.is_some() => "none · the last comment".to_string(),
        (0, _) => "none".to_string(),
        (n, 0) => format!("{n}, all resolved"),
        (n, open) => format!("{n}, {open} open"),
    };
    // The last comment: who, where it lives on GitLab (a click opens it
    // there, else the merge request), and its start; the page has it whole.
    let threads = mr.last_comment.as_ref().map_or_else(Vec::new, |comment| {
        let whence = crate::screen::Whence {
            words: format!("comment on !{} in GitLab", mr.iid),
            url: comment.url.as_deref().unwrap_or(&mr.web_url),
        };
        crate::screen::comment_blocks(
            p,
            &mr.project_path,
            &comment.author,
            &comment.at,
            &comment.body,
            None,
            Some(whence),
            Some(COMMENT_MOST),
            cx.now,
        )
    });
    blocks.push(Block::section(
        Some(Icon::Thread),
        "threads",
        Some(thread_detail),
        threads,
    ));
    blocks.extend(cx.parts.detail_sections(&subject));
    DetailContent::new(header, facts, blocks)
}

#[cfg(test)]
mod detail_tests;
