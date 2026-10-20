//! Page content expressed as view data while the terminal adapter still
//! paints it. Each migrated page keeps the same domain state and actions.

use super::*;
use radar_core::time;
use radar_core::view::{
    self as v, Axis, Block, ColumnSpec, EntryView, Ink, Node, PageLayout, PageView, PaneId,
    PaneView, Seg, Size, TabView, TableRow, Text,
};

pub(crate) fn view(p: &GitlabPlugin, cx: &v::ViewCx<'_>) -> Option<v::ScreenView> {
    let page = p.pages.last()?;
    if page.tab >= own_tabs(&page.kind) {
        return contributed_view(p, page, cx);
    }
    match &page.kind {
        Kind::Tenancy(group) if page.tab == 0 => tenancy_view(p, page, group, cx),
        Kind::Repo(path) if page.tab == repo::MRS => repo_mrs_view(p, page, path, cx),
        Kind::Repo(path) if page.tab == repo::PIPELINES => repo_pipeline_view(p, page, path, cx),
        Kind::Mr { project, iid } if page.tab == mr::TABS.len() - 1 => {
            mr_commits_view(p, page, project, *iid, cx)
        }
        Kind::Mr { project, iid } if page.tab == mr::OVERVIEW => {
            mr_overview_view(p, page, project, *iid, cx)
        }
        Kind::Mr { project, iid } if page.tab == mr::THREADS => {
            mr_threads_view(p, page, project, *iid, cx)
        }
        Kind::Mr { project, iid } if page.tab == mr::CHANGES => {
            super::changes_neutral::view(p, page, project, *iid, cx)
        }
        Kind::Mr { project, iid } if page.tab == mr::PIPELINE => {
            mr_pipeline_view(p, page, project, *iid, cx)
        }
        Kind::Job { project, id } if page.tab == 0 => job_view(p, page, project, *id, cx),
        _ => None,
    }
}

fn job_view(
    p: &GitlabPlugin,
    page: &Page,
    project_path: &str,
    id: u64,
    cx: &v::ViewCx<'_>,
) -> Option<v::ScreenView> {
    let mut crumbs = vec![Seg::new("Repositories", Ink::Secondary)];
    if let Some(s) = tenancy_of(p, project_path) {
        crumbs.push(Seg::styled(
            s.code.clone(),
            format!("tenant.{} {}", s.color, p.cfg.style_for("tenant")),
        ));
    }
    crumbs.push(Seg::new(project(p, project_path).name, Ink::Secondary));
    if let Some(Kind::Mr { project, iid }) = p
        .pages
        .iter()
        .rev()
        .skip(1)
        .map(|page| &page.kind)
        .find(|kind| matches!(kind, Kind::Mr { .. }))
        && project == project_path
    {
        crumbs.push(Seg::new(format!("!{iid}"), Ink::AccentSecondary));
    }
    crumbs.push(Seg::new(job::name(p, project_path, id), Ink::Emphasis));
    let info = job::info(p, project_path, id);
    let title = info.map_or_else(
        || Text::of(format!("job #{id}"), Ink::Emphasis),
        |info| {
            Text::from(vec![
                Seg::icon(Icon::Job),
                Seg::new(info.name.clone(), Ink::Emphasis).strong(),
            ])
        },
    );
    let mut details = Text::new();
    if let Some(info) = info {
        let mark = status_mark(&info.status);
        details.add(Seg::icon(mark.icon()));
        details.add(Seg::styled(
            info.status.clone(),
            p.cfg.style_for(&info.status),
        ));
        if !info.stage.is_empty() {
            details.add(Seg::new(" · stage ", Ink::Muted));
            details.add(Seg::new(info.stage.clone(), Ink::AccentSecondary));
        }
        if let Some(seconds) = info.duration_secs {
            details.add(Seg::new(
                format!(" · {}", radar_core::ui::duration(seconds.into())),
                Ink::Muted,
            ));
        }
        let reason = crate::joblog::reason(&info.failure_reason);
        if !reason.is_empty() {
            details.add(Seg::new(format!(" · {reason}"), Ink::Bad));
        }
        if info.allow_failure {
            details.add(Seg::new(" · allowed to fail", Ink::Muted));
        }
    }
    *page.extra.borrow_mut() = crate::join::of_page(p, &page.kind)
        .map(|subject| cx.parts.tabs(&subject))
        .unwrap_or_default();
    let mut tabs = vec![TabView::new(
        job::tabs()[0].clone(),
        Some(Icon::Log),
    )];
    tabs.extend(
        page.extra
            .borrow()
            .iter()
            .map(|(name, _)| TabView::new(name.clone(), None)),
    );
    let key = crate::log_ownership::JobKey::new(project_path, id);
    let fetched = p.logs.get(&key);
    let log = fetched.and_then(|f| f.value.as_ref());
    let query = page.log.query();
    let panel_title = match log {
        Some(log) if !query.is_empty() => match page.log.found(&log.text) {
            (_, 0) => format!("log · /{query} · no match"),
            (0, n) => format!(
                "log · /{query} · {n} match{}",
                if n == 1 { "" } else { "es" }
            ),
            (on, n) => format!("log · /{query} · {on} of {n}"),
        },
        _ => "log".into(),
    };
    let mut pane = PaneView::new(
        PaneId::Middle,
        panel_title,
        Node::Document {
            lines: vec![],
            cut: true,
        },
    );
    pane.head =
        fetched.and_then(|f| v::kit::fetching(f.loading, f.value.is_some(), f.error.as_deref()));
    if let Some(input) = &p.log_search {
        pane.bottom_field = Some(v::FieldBar {
            mark: " / ".into(),
            text: input.value(),
            caret: Some(input.cursor()),
            hint: Text::new(),
        });
    }
    page.log_painted.set(false);
    if let Some(log) = log {
        if log.earlier() {
            pane.notes.push(Text::of(
                if crate::log_ownership::earlier_loading(p, &key) {
                    "⋯ loading the part before…".into()
                } else {
                    format!(
                        "⋯ {} earlier in the log, loaded when you reach the top",
                        job::size(log.start)
                    )
                },
                Ink::Muted,
            ));
        }
        let height = cx.height.saturating_sub(
            7 + u16::from(pane.head.is_some())
                + u16::from(log.earlier())
                + u16::from(pane.bottom_field.is_some()),
        );
        let (lines, top, total) = page
            .log
            .paint(&log.text, cx.width.saturating_sub(2), height);
        pane.body = Node::Log {
            lines,
            top,
            total,
            empty: "This job wrote nothing to its log.".into(),
        };
        page.log_painted.set(true);
    }
    let frame = PageView {
        crumbs,
        title,
        details,
        under: None,
        tabs,
        tab: page.tab,
        notice: None,
        panes: vec![pane],
        layout: None,
    };
    let mut view = v::ScreenView::page(crate::SCREEN.id, frame);
    view.actions = crate::actions(&p.rows(Pane::Middle));
    Some(view)
}

fn mr_threads_view(
    p: &GitlabPlugin,
    page: &Page,
    project: &str,
    iid: u64,
    cx: &v::ViewCx<'_>,
) -> Option<v::ScreenView> {
    let mut view = mr_commits_view(p, page, project, iid, cx)?;
    let frame = view.page.as_mut()?;
    frame.tab = mr::THREADS;
    let fetched = p.mr_pages.get(&mr_key(project, iid));
    let detail = fetched.and_then(|f| f.value.as_ref()).map(|(_, d)| d);
    page.detail_painted.set(detail.is_some());
    let mut pane = PaneView::new(
        PaneId::Middle,
        "threads",
        Node::Document {
            lines: vec![],
            cut: false,
        },
    );
    if let Some(error) = fetched.and_then(|f| f.error.as_deref()) {
        if detail.is_some() {
            frame.notice = Some(Text::from(vec![
                Seg::icon(Icon::Error),
                Seg::new("merge request details refresh failed: ", Ink::Bad),
                Seg::new(error, Ink::Bad).strong(),
            ]));
        } else {
            pane.head = Some(v::kit::failed(error));
        }
    } else if detail.is_none() && fetched.is_some_and(|f| f.loading) {
        pane.head = Some(v::kit::loading());
    }
    if let Some(detail) = detail {
        let width = cx.width.saturating_sub(5);
        let mut rows = vec![EntryView::new(
            "row:0",
            Text::from(vec![
                Seg::new(" + ", Ink::Accent).strong(),
                Seg::icon(Icon::Comment),
                Seg::new(format!("comment on !{iid}"), Ink::AccentSecondary).strong(),
                Seg::new(
                    "   a comment on the merge request, not on a line",
                    Ink::Muted,
                ),
            ]),
        )];
        for (i, thread) in review::threads(detail).into_iter().enumerate() {
            let (icon, word, ink) = match (thread.resolvable, thread.resolved) {
                (true, false) => (Icon::Thread, "open", Ink::Warn),
                (false, _) => (Icon::Comment, "comment", Ink::Secondary),
                (true, true) => (Icon::Resolved, "resolved", Ink::Good),
            };
            let mut head = Text::from(vec![
                Seg::icon(icon),
                if thread.resolvable {
                    Seg::new(word, ink).strong()
                } else {
                    Seg::new(word, ink)
                },
            ]);
            if let Some(place) = thread.place() {
                head.add(Seg::new(format!(" · {place}"), Ink::Secondary));
            }
            let mut entry = EntryView::new(format!("row:{}", i + 1), head);
            entry.before.push(Text::new());
            for (j, note) in thread.notes.iter().enumerate() {
                let mut who = Text::from(vec![Seg::new("│ ", Ink::AccentMuted)]);
                if j > 0 {
                    who.add(Seg::new("↳ ", Ink::Muted));
                }
                who.add(Seg::icon(Icon::Person));
                who.add(Seg::new(note.author.clone(), Ink::Secondary).strong());
                who.add(Seg::new(
                    format!(" · {} ago", time::ago_str(&note.at, cx.now)),
                    Ink::Muted,
                ));
                entry.body.push(who);
                for line in radar_core::ui::wrap_words(&note.body, width, 2) {
                    entry.body.push(Text::from(vec![
                        Seg::new("│ ", Ink::AccentMuted),
                        Seg::new(
                            line,
                            if thread.resolved {
                                Ink::Secondary
                            } else {
                                Ink::Primary
                            },
                        ),
                    ]));
                }
            }
            rows.push(entry);
        }
        pane.body = Node::Entries {
            rows,
            selected: Some(page.row.get().min(detail.discussions.len())),
            empty: String::new(),
        };
    } else {
        pane.body = Node::Document {
            lines: vec![Text::of("No threads loaded.", Ink::Muted)],
            cut: false,
        };
    }
    frame.panes = vec![pane];
    frame.layout = None;
    Some(view)
}

fn contributed_view(p: &GitlabPlugin, page: &Page, cx: &v::ViewCx<'_>) -> Option<v::ScreenView> {
    let mut view = match &page.kind {
        Kind::Tenancy(group) => tenancy_view(p, page, group, cx)?,
        Kind::Repo(path) => repo_mrs_view(p, page, path, cx)?,
        Kind::Mr { project, iid } => mr_commits_view(p, page, project, *iid, cx)?,
        Kind::Job { project, id } => job_view(p, page, project, *id, cx)?,
    };
    let frame = view.page.as_mut()?;
    let tab = page.contributed()?;
    frame.tab = page.tab;
    let (title, rows) = &*tab;
    let body = if rows.iter().any(|row| row.subject.is_some()) {
        Node::List {
            rows: rows
                .iter()
                .enumerate()
                .map(|(i, row)| {
                    v::ListRow::new(format!("row:{i}"), row.line.clone())
                        .linked(row.subject.as_ref().and_then(|s| s.url.clone()))
                })
                .collect(),
            selected: Some(page.row.get().min(rows.len().saturating_sub(1))),
            editing: None,
        }
    } else {
        Node::Document {
            lines: rows.iter().map(|row| row.line.clone()).collect(),
            cut: false,
        }
    };
    frame.panes = vec![PaneView::new(PaneId::Middle, title.to_lowercase(), body)];
    frame.layout = None;
    page.log_painted.set(false);
    Some(view)
}

fn tenancy_view(
    p: &GitlabPlugin,
    page: &Page,
    group: &str,
    cx: &v::ViewCx<'_>,
) -> Option<v::ScreenView> {
    // A contributed tab has its own rows and is migrated with the other pages.
    *page.extra.borrow_mut() = crate::join::of_page(p, &page.kind)
        .map(|s| cx.parts.tabs(&s))
        .unwrap_or_default();
    let source = source(p, group);
    let repos = tenancy::repos(p, group);
    let mrs: usize = repos.iter().map(|repo| repo.mrs).sum();
    let title = match source {
        Some(s) => Text::from(vec![
            Seg::icon(Icon::Group),
            Seg::new(p.cfg.group_path(s), Ink::Emphasis).strong(),
        ]),
        None => Text::of(group, Ink::Emphasis),
    };
    let mut tab_views = vec![TabView::new(
        tenancy::tabs()[0].clone(),
        Some(Icon::Repo),
    )];
    tab_views.extend(
        page.extra
            .borrow()
            .iter()
            .map(|(title, _)| TabView::new(title.clone(), None)),
    );
    let mut frame = PageView {
        crumbs: vec![
            Seg::new("Repositories", Ink::Secondary),
            source.map_or_else(
                || Seg::new("?", Ink::Muted),
                |s| {
                    Seg::styled(
                        s.code.clone(),
                        format!("tenant.{} {}", s.color, p.cfg.style_for("tenant")),
                    )
                },
            ),
        ],
        title,
        details: Text::of(
            format!(
                "{} repo{} · {} merge request{}",
                repos.len(),
                if repos.len() == 1 { "" } else { "s" },
                mrs,
                if mrs == 1 { "" } else { "s" }
            ),
            Ink::Muted,
        ),
        under: None,
        tabs: tab_views,
        tab: 0,
        notice: None,
        panes: vec![],
        layout: None,
    };
    let body = if source.is_none() {
        Node::State {
            state: v::State::Empty,
            text: "This tenancy is no longer tracked.".into(),
        }
    } else {
        let columns = vec![
            ColumnSpec::flex("repo", 10),
            ColumnSpec::new("main", 12, 0).icon(Icon::Pipeline),
            ColumnSpec::new("mrs", 5, 0).icon(Icon::Change),
            ColumnSpec::new("last activity", 14 + cx.icon_lead as u16, 0).icon(Icon::Updated),
        ];
        let rows = repos
            .iter()
            .enumerate()
            .map(|(i, repo)| {
                let busy = repo.mrs > 0;
                let main = repo.project.main_pipeline.map_or_else(
                    || Text::of("— none", Ink::Muted),
                    |pipeline| {
                        Text::from(vec![
                            Seg::icon(pipeline.mark().icon()),
                            Seg::styled(pipeline.word(), p.cfg.style_for(pipeline.word())),
                        ])
                    },
                );
                TableRow::new(format!("row:{i}"))
                    .cell(
                        "repo",
                        Text::from(vec![
                            Seg::icon(Icon::Repo),
                            if busy {
                                Seg::new(repo.project.name.clone(), Ink::Primary).strong()
                            } else {
                                Seg::new(repo.project.name.clone(), Ink::Secondary)
                            },
                        ]),
                    )
                    .cell("main", main)
                    .cell(
                        "mrs",
                        Text::of(
                            if busy {
                                repo.mrs.to_string()
                            } else {
                                "—".into()
                            },
                            if busy { Ink::Number } else { Ink::Muted },
                        ),
                    )
                    .cell(
                        "last activity",
                        Text::of(
                            if repo.latest.is_empty() {
                                "—".into()
                            } else {
                                format!("{} ago", time::ago_str(&repo.latest, cx.now))
                            },
                            Ink::Muted,
                        ),
                    )
            })
            .collect();
        Node::Table {
            columns,
            rows,
            selected: Some(page.row.get().min(repos.len().saturating_sub(1))),
            empty: None,
        }
    };
    let mut pane = PaneView::new(PaneId::Middle, "repos", body);
    if let Some(s) = source {
        let hidden = &s.hide;
        pane.footer = Some(Text::of(
            if hidden.is_empty() {
                "No repos hidden. Settings › tenancies hides the ones you never need.".into()
            } else {
                format!(
                    "{} repo{} hidden: {}. Settings › tenancies shows them again.",
                    hidden.len(),
                    if hidden.len() == 1 { "" } else { "s" },
                    hidden.join(", ")
                )
            },
            Ink::Muted,
        ));
    }
    frame.panes.push(pane);
    let mut view = v::ScreenView::page(crate::SCREEN.id, frame);
    view.actions = crate::actions(&p.rows(Pane::Middle));
    Some(view)
}

fn repo_mrs_view(
    p: &GitlabPlugin,
    page: &Page,
    path: &str,
    cx: &v::ViewCx<'_>,
) -> Option<v::ScreenView> {
    *page.extra.borrow_mut() = crate::join::of_page(p, &page.kind)
        .map(|s| cx.parts.tabs(&s))
        .unwrap_or_default();
    let pr = project(p, path);
    let subject = crate::join::repo(p, path);
    let mut details = Text::new();
    for chip in cx.parts.chips(&subject) {
        details.add(Seg::chip(&chip));
    }
    if !details.is_empty() {
        details.add(Seg::new("  ", Ink::Plain));
    }
    details.add(Seg::icon(Icon::Branch));
    details.add(Seg::new(
        format!("{} ", branch_of(&pr)),
        Ink::AccentSecondary,
    ));
    details.extend(pipeline_text(p, pr.main_pipeline));
    if let Some(commit) = &pr.main_commit {
        details.add(Seg::new(" · last commit ", Ink::Muted));
        details.add(Seg::icon(Icon::Commit));
        details.add(Seg::new(
            format!("{} ", commit.short_id),
            Ink::AccentSecondary,
        ));
        details.add(Seg::icon(Icon::Person));
        details.add(Seg::new(commit.author.clone(), Ink::Secondary));
        details.add(Seg::new(
            format!(" · {} ago", time::ago_str(&commit.at, cx.now)),
            Ink::Muted,
        ));
    } else if !pr.last_activity.is_empty() {
        details.add(Seg::new(
            format!(" · active {} ago", time::ago_str(&pr.last_activity, cx.now)),
            Ink::Muted,
        ));
    }
    if cx.width <= 80
        && let Some(commit) = &pr.main_commit
    {
        details = Text::from(vec![
            Seg::icon(Icon::Commit),
            Seg::new(format!("{} ", commit.short_id), Ink::AccentSecondary),
            Seg::icon(Icon::Person),
            Seg::new(commit.author.clone(), Ink::Secondary),
            Seg::new(
                format!(" · {} ago", time::ago_str(&commit.at, cx.now)),
                Ink::Muted,
            ),
        ]);
    }
    let mut crumbs = vec![Seg::new("Repositories", Ink::Secondary)];
    if let Some(s) = tenancy_of(p, path) {
        crumbs.push(Seg::styled(
            s.code.clone(),
            format!("tenant.{} {}", s.color, p.cfg.style_for("tenant")),
        ));
    }
    crumbs.push(Seg::new(pr.name.clone(), Ink::Emphasis));
    let tabs = repo::tabs(p, path);
    let mut tab_views = vec![
        TabView::new(tabs[0].clone(), Some(Icon::Change)),
        TabView::new(tabs[1].clone(), Some(Icon::Pipeline)),
    ];
    tab_views.extend(
        page.extra
            .borrow()
            .iter()
            .map(|(title, _)| TabView::new(title.clone(), None)),
    );
    let under = contributed_under(page, &cx.parts, &subject);
    let mut frame = PageView {
        crumbs,
        title: Text::from(vec![
            Seg::icon(Icon::Repo),
            Seg::new(pr.full_path.clone(), Ink::Emphasis).strong(),
        ]),
        details,
        under,
        tabs: tab_views,
        tab: 0,
        notice: None,
        panes: vec![],
        layout: None,
    };
    let lead = cx.icon_lead as u16;
    let mut columns = vec![
        ColumnSpec::new("", 7 + lead, 0),
        ColumnSpec::new("for you", 9 + lead, 0).icon(Icon::You),
        ColumnSpec::flex("title", 8),
        ColumnSpec::new("author", 12 + lead, 0).icon(Icon::Person),
        ColumnSpec::new("state", 19 + lead.saturating_sub(2), 0).icon(Icon::Pipeline),
    ];
    let items = repo::mrs(p, path);
    for item in &items {
        for cell in cx.parts.cells(&crate::join::of_mr(item.mr)) {
            if columns.iter().any(|col| col.heading == cell.heading) {
                continue;
            }
            let width = (radar_core::ui::width(&cell.text).max(radar_core::ui::width(&cell.heading))
                + cx.icon_lead)
                .min(24) as u16;
            columns.push(ColumnSpec::extra(cell.heading, width));
        }
    }
    columns.push(ColumnSpec::new("updated", 8 + lead, 0).icon(Icon::Updated));
    let rows = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let mr = item.mr;
            let subject = crate::join::of_mr(mr);
            let mine = mr.author.username == p.me;
            let who = if mine {
                "you".into()
            } else {
                radar_core::ui::short_name(&mr.author.name)
            };
            let mut title = Text::of(mr.title.clone(), Ink::Primary);
            for chip in cx.parts.chips(&subject) {
                title.add(Seg::new(" ", Ink::Plain));
                title.add(Seg::chip(&chip));
            }
            let mut row = TableRow::new(format!("row:{i}"))
                .cell(
                    "",
                    Text::from(vec![
                        Seg::icon(crate::screen::mr_icon(mr)),
                        Seg::new(format!("!{}", mr.iid), Ink::AccentSecondary),
                    ]),
                )
                .cell(
                    "for you",
                    Text::from(vec![Seg::styled(
                        item.role.word(),
                        p.cfg.style_for(item.role.word()),
                    )]),
                )
                .cell("title", title)
                .cell(
                    "author",
                    Text::from(vec![
                        Seg::icon(crate::screen::person_icon(p, &mr.author.username)),
                        Seg::new(who, Ink::Secondary),
                    ]),
                )
                .cell("state", mr_state_text(mr))
                .cell(
                    "updated",
                    Text::of(time::ago_str(&mr.updated_at, cx.now), Ink::Muted),
                );
            for cell in cx.parts.cells(&subject) {
                row.cells.push(v::cell_of(&cell));
            }
            row
        })
        .collect();
    frame.panes.push(PaneView::new(
        PaneId::Middle,
        "merge requests",
        Node::Table {
            columns,
            rows,
            selected: Some(page.row.get().min(items.len().saturating_sub(1))),
            empty: None,
        },
    ));
    let mut view = v::ScreenView::page(crate::SCREEN.id, frame);
    view.actions = crate::actions(&p.rows(Pane::Middle));
    Some(view)
}

fn repo_pipeline_view(
    p: &GitlabPlugin,
    page: &Page,
    path: &str,
    cx: &v::ViewCx<'_>,
) -> Option<v::ScreenView> {
    let mut view = repo_mrs_view(p, page, path, cx)?;
    let frame = view.page.as_mut()?;
    frame.tab = repo::PIPELINES;
    let pr = project(p, path);
    let branch = branch_of(&pr);
    let failed = repo::lead_rows(p, path);
    let runs = crate::runs_ownership::cached(p, path)
        .and_then(|f| f.value.as_deref())
        .unwrap_or_default();
    let lead = cx.icon_lead as u16;
    let by_w = if cx.width >= 123 { 24 } else { 16 };
    let columns = vec![
        ColumnSpec::new("status", 11 + lead.saturating_sub(2), 0).icon(Icon::Pipeline),
        ColumnSpec::new("run", 7, 0),
        ColumnSpec::new("commit", 10, 0).icon(Icon::Commit),
        ColumnSpec::flex("", 8),
        ColumnSpec::new("by", by_w, 0).icon(Icon::Person),
        ColumnSpec::new("took", 9, 0).icon(Icon::Time),
        ColumnSpec::new("age", 4 + lead, 0).icon(Icon::Updated),
    ];
    let rows = runs
        .iter()
        .enumerate()
        .map(|(i, run)| {
            TableRow::new(format!("row:{}", i + failed.len()))
                .cell("status", pipeline_text(p, run.status))
                .cell(
                    "run",
                    Text::of(format!("#{}", run.iid), Ink::AccentSecondary),
                )
                .cell("commit", Text::of(run.commit_sha.clone(), Ink::Muted))
                .cell("", Text::of(run.commit_title.clone(), Ink::Primary))
                .cell(
                    "by",
                    Text::of(
                        format!("{} · {}", run.source, radar_core::ui::short_name(&run.user)),
                        Ink::Secondary,
                    ),
                )
                .cell(
                    "took",
                    Text::of(
                        run.duration_secs.map_or_else(String::new, |seconds| {
                            radar_core::ui::duration(seconds.into())
                        }),
                        Ink::Muted,
                    ),
                )
                .cell(
                    "age",
                    Text::of(time::ago_str(&run.created_at, cx.now), Ink::Muted),
                )
        })
        .collect();
    let mut pane = PaneView::new(
        PaneId::Middle,
        format!("pipelines on {branch}"),
        Node::Table {
            columns,
            rows,
            selected: page.row.get().checked_sub(failed.len()),
            empty: Some(format!("No pipelines on {branch} lately.")),
        },
    );
    if let Some(fetched) = crate::runs_ownership::cached(p, path) {
        pane.head = v::kit::fetching(
            fetched.loading,
            fetched.value.is_some(),
            fetched.error.as_deref(),
        );
        if fetched.value.is_none() && pane.head.is_some() {
            pane.body = Node::Document {
                lines: vec![],
                cut: true,
            };
        }
    }
    frame.panes = vec![pane];
    if let Some(red) = crate::why::red(pr.main_pipeline, pr.main_pipeline_url.as_deref()) {
        let mut entries = Vec::new();
        for (i, info) in failed.iter().enumerate() {
            let mut head = Text::from(vec![
                Seg::icon(Icon::RunFailed),
                Seg::new(
                    radar_core::ui::pad(&info.name, 24.min(cx.width as usize / 3).max(8)),
                    Ink::Primary,
                )
                .strong(),
                Seg::new(info.stage.clone(), Ink::AccentSecondary),
            ]);
            if let Some(seconds) = info.duration_secs {
                head.add(Seg::new(
                    format!(" · {}", radar_core::ui::duration(seconds.into())),
                    Ink::Muted,
                ));
            }
            let reason = crate::joblog::reason(&info.failure_reason);
            if !reason.is_empty() {
                head.add(Seg::new(format!(" · {reason}"), Ink::Bad));
            }
            if info.allow_failure {
                head.add(Seg::new(" · allowed to fail", Ink::Muted));
            }
            let mut entry = EntryView::new(format!("row:{i}"), head);
            entry.body = failed_log_lines(p, path, info.id, cx.width.saturating_sub(4));
            entries.push(entry);
        }
        let fetched = p.failed.get(&crate::why::key(path, red));
        let refresh_error = fetched
            .filter(|f| f.value.is_some())
            .and_then(|f| f.error.as_deref());
        let cold_error = fetched
            .filter(|f| f.value.is_none())
            .and_then(|f| f.error.as_deref());
        let empty = if cold_error.is_some() {
            String::new()
        } else if refresh_error.is_some()
            || fetched
                .and_then(|f| f.value.as_ref())
                .is_some_and(Vec::is_empty)
        {
            "GitLab lists no failed job in it.".to_string()
        } else {
            "loading…".to_string()
        };
        let mut top = PaneView::new(
            PaneId::Middle,
            format!("why {branch} is red"),
            Node::Entries {
                rows: entries,
                selected: (page.row.get() < failed.len()).then_some(page.row.get()),
                empty,
            },
        );
        if let Some(error) = refresh_error {
            top.head = Some(v::kit::failed(error));
        } else if let Some(error) = cold_error {
            top.head = Some(v::kit::failed(error));
        }
        let content_h: usize = failed
            .iter()
            .map(|info| 1 + failed_log_lines(p, path, info.id, cx.width.saturating_sub(4)).len())
            .sum();
        let body_h = cx.height.saturating_sub(4);
        let want = (content_h as u16 + 2).max(3) + u16::from(top.head.is_some());
        let h = want.min(body_h * 3 / 5).max(3.min(body_h));
        frame.panes.push(top);
        frame.layout = Some(PageLayout::split(
            Axis::Vertical,
            [
                (Size::Length(h), PageLayout::pane(1)),
                (Size::Fill(1), PageLayout::pane(0)),
            ],
        ));
    }
    Some(view)
}

fn pipeline_text(p: &GitlabPlugin, pipeline: Option<Pipeline>) -> Text {
    pipeline.map_or_else(
        || Text::of("— none", Ink::Muted),
        |pipeline| {
            Text::from(vec![
                Seg::icon(pipeline.mark().icon()),
                Seg::styled(pipeline.word(), p.cfg.style_for(pipeline.word())),
            ])
        },
    )
}

fn contributed_under(
    page: &Page,
    parts: &radar_core::Parts<'_>,
    subject: &radar_core::Subject,
) -> Option<Text> {
    let (part, words) = parts.lines(subject).into_iter().next()?;
    let mut actions = page.under.borrow_mut();
    actions.clear();
    let mut text = Text::new();
    for word in words {
        let target = word.action.as_ref().map(|id| {
            let target = format!("under:{}", actions.len());
            actions.push((part, subject.clone(), id.clone()));
            target
        });
        if let Some(icon) = word.icon {
            let mut seg = Seg::icon(icon);
            if let Some(target) = &target {
                seg = seg.targeted(target.clone());
            }
            text.add(seg);
        }
        let mut seg = Seg::new(word.text, Ink::from(word.tone));
        if let Some(url) = word.url {
            seg = seg.linked(url);
        }
        if let Some(target) = target {
            seg = seg.targeted(target);
        }
        text.add(seg);
    }
    Some(text)
}

fn mr_state_text(mr: &Mr) -> Text {
    match mr.state {
        crate::model::MrState::Merged => Text::from(vec![
            Seg::icon(Icon::Merged),
            Seg::new("merged", Ink::Muted),
        ]),
        crate::model::MrState::Closed => Text::from(vec![
            Seg::icon(Icon::Closed),
            Seg::new("closed", Ink::Muted),
        ]),
        crate::model::MrState::Opened => match crate::ready::blocks(mr).first() {
            Some(block) => Text::from(vec![
                Seg::icon(block.icon()),
                if matches!(block.tone(), radar_core::Tone::Warn | radar_core::Tone::Bad) {
                    Seg::new(block.words(), Ink::from(block.tone())).strong()
                } else {
                    Seg::new(block.words(), Ink::from(block.tone()))
                },
            ]),
            None => Text::from(vec![
                Seg::icon(Icon::RunPassed),
                Seg::new("ready to merge", Ink::Good),
            ]),
        },
    }
}

pub(super) fn mr_commits_view(
    p: &GitlabPlugin,
    page: &Page,
    project_path: &str,
    iid: u64,
    cx: &v::ViewCx<'_>,
) -> Option<v::ScreenView> {
    let subject = crate::join::of_page(p, &page.kind);
    *page.extra.borrow_mut() = subject
        .as_ref()
        .map(|s| cx.parts.tabs(s))
        .unwrap_or_default();
    let mr = find_mr(p, project_path, iid);
    let pr = project(p, project_path);
    let mut crumbs = vec![Seg::new("Repositories", Ink::Secondary)];
    if let Some(s) = tenancy_of(p, project_path) {
        crumbs.push(Seg::styled(
            s.code.clone(),
            format!("tenant.{} {}", s.color, p.cfg.style_for("tenant")),
        ));
    }
    crumbs.push(Seg::new(pr.name, Ink::Secondary));
    crumbs.push(Seg::new(format!("!{iid}"), Ink::AccentSecondary));
    let title = mr.map_or_else(
        || Text::of(format!("!{iid}"), Ink::Emphasis),
        |mr| {
            Text::from(vec![
                Seg::icon(crate::screen::mr_icon(mr)),
                Seg::new(mr.title.clone(), Ink::Emphasis).strong(),
            ])
        },
    );
    let mut details = Text::new();
    if let Some(mr) = mr {
        if mr.state != crate::model::MrState::Opened {
            details.add(Seg::styled(
                mr.state.word(),
                p.cfg.style_for(mr.state.word()),
            ));
            details.add(Seg::new(" · ", Ink::Muted));
        }
        let who = if mr.author.name.is_empty() {
            &mr.author.username
        } else {
            &mr.author.name
        };
        details.add(Seg::icon(crate::screen::person_icon(
            p,
            &mr.author.username,
        )));
        details.add(Seg::new(who.clone(), Ink::Secondary));
        details.add(Seg::new(" · ", Ink::Muted));
        details.add(Seg::icon(Icon::Branch));
        details.add(Seg::new(
            format!(
                "{} → {}",
                radar_core::ui::fit(&mr.source_branch, 24).trim_end(),
                mr.target_branch
            ),
            Ink::AccentSecondary,
        ));
    }
    let icons = [
        Icon::Info,
        Icon::File,
        Icon::Thread,
        Icon::Pipeline,
        Icon::Commit,
    ];
    let mut tabs: Vec<TabView> = mr::tabs()
        .into_iter()
        .zip(icons)
        .zip(mr::attention(p, project_path, iid))
        .map(|((label, icon), attention)| TabView::new(label, Some(icon)).attention(attention))
        .collect();
    tabs.extend(
        page.extra
            .borrow()
            .iter()
            .map(|(title, _)| TabView::new(title.clone(), None)),
    );
    let under = subject
        .as_ref()
        .and_then(|subject| contributed_under(page, &cx.parts, subject));
    let mut frame = PageView {
        crumbs,
        title,
        details,
        under,
        tabs,
        tab: page.tab,
        notice: None,
        panes: vec![],
        layout: None,
    };
    let fetched = p.mr_pages.get(&mr_key(project_path, iid));
    let detail = fetched
        .and_then(|f| f.value.as_ref())
        .map(|(_, detail)| detail);
    let mut lines: Vec<Text> = Vec::new();
    if let Some(f) = fetched
        && let Some(err) = &f.error
    {
        lines.push(v::kit::sync_failed("merge request", err));
        lines.push(Text::new());
    }
    match detail {
        None if fetched.is_some_and(|f| f.loading) => {}
        None => lines.push(Text::of("No commits loaded.", Ink::Muted)),
        Some(detail) if detail.commits.is_empty() => {
            lines.push(Text::of("No commits.", Ink::Muted))
        }
        Some(detail) => {
            let title_w = (cx.width as usize).saturating_sub(10 + 18 + 4 + 4).max(10);
            for commit in &detail.commits {
                lines.push(Text::from(vec![
                    Seg::new(radar_core::ui::pad(&commit.short_id, 10), Ink::AccentSecondary),
                    Seg::new(
                        radar_core::ui::pad(&radar_core::ui::fit(&commit.title, title_w), title_w),
                        Ink::Primary,
                    ),
                    Seg::new(radar_core::ui::pad(&commit.author, 18), Ink::Secondary),
                    Seg::new(time::ago_str(&commit.at, cx.now), Ink::Muted),
                ]));
            }
        }
    }
    frame.panes.push(PaneView::new(
        PaneId::Middle,
        "commits",
        Node::Document { lines, cut: false },
    ));
    let mut view = v::ScreenView::page(crate::SCREEN.id, frame);
    view.actions = crate::actions(&p.rows(Pane::Middle));
    Some(view)
}

fn mr_overview_view(
    p: &GitlabPlugin,
    page: &Page,
    project_path: &str,
    iid: u64,
    cx: &v::ViewCx<'_>,
) -> Option<v::ScreenView> {
    let mut view = mr_commits_view(p, page, project_path, iid, cx)?;
    let frame = view.page.as_mut()?;
    let Some(mr) = find_mr(p, project_path, iid) else {
        frame.panes = vec![PaneView::new(
            PaneId::Middle,
            "merge request",
            Node::State {
                state: v::State::Empty,
                text:
                    "This merge request is not in the last sync: it may have been merged or closed."
                        .into(),
            },
        )];
        frame.tab = mr::OVERVIEW;
        return Some(view);
    };
    let fetched = p.mr_pages.get(&mr_key(project_path, iid));
    let detail = fetched
        .and_then(|f| f.value.as_ref())
        .map(|(_, detail)| detail);
    let mut checks = merge_checks(p, mr, detail);
    if let Some(acting) = p.acting.get(&mr_key(project_path, iid)) {
        use crate::mr_actions::Acting;
        checks.push(Text::new());
        let (icon, ink, words) = match &acting.state {
            Acting::Running => (
                Icon::RunRunning,
                Ink::Info,
                format!("{} asked of GitLab…", acting.what),
            ),
            Acting::Done(message) => (Icon::RunPassed, Ink::Good, message.clone()),
            Acting::Failed(error) => (Icon::Error, Ink::Bad, error.clone()),
        };
        checks.push(Text::from(vec![
            Seg::icon(icon),
            Seg::new(words, ink),
            Seg::new(
                format!("  {} ago", time::ago(acting.at, cx.now)),
                Ink::Muted,
            ),
        ]));
    }
    let mut blocks = Vec::new();
    if fetched.is_some_and(|f| f.loading) {
        blocks.push(Block::line(Text::of("loading…", Ink::Muted)));
        blocks.push(Block::line(Text::new()));
    }
    if let Some(err) = fetched.and_then(|f| f.error.as_deref()) {
        blocks.push(Block::line(v::kit::sync_failed("merge request", err)));
        blocks.push(Block::line(Text::new()));
    }
    if mr.state != crate::model::MrState::Opened {
        blocks.push(Block::line(Text::from(vec![Seg::styled(
            mr.state.word(),
            p.cfg.style_for(mr.state.word()),
        )])));
    }
    if !mr.labels.is_empty() {
        blocks.push(Block::line(Text::from(vec![
            Seg::new("LABELS  ", Ink::Muted).strong(),
            Seg::styled(
                mr.labels
                    .iter()
                    .map(|label| format!("#{label}"))
                    .collect::<Vec<_>>()
                    .join(" · "),
                p.cfg.style_for("label"),
            ),
        ])));
    }
    if mr.state != crate::model::MrState::Opened || !mr.labels.is_empty() {
        blocks.push(Block::line(Text::new()));
    }
    let mut written = Vec::new();
    let mut texts = Vec::new();
    if let Some(detail) = detail {
        if detail.description.trim().is_empty() {
            blocks.push(Block::line(Text::of("No description.", Ink::Muted)));
        } else {
            let mut doc = radar_core::ui::markdown::parse(&detail.description);
            crate::pictures::make_whole(&mut doc, &p.cfg.url, project_path);
            written = doc.links();
            texts.push(detail.description.clone());
            blocks.push(Block::Doc {
                doc,
                bar: None,
                most: None,
                urls: Some(false),
            });
        }
        blocks.push(Block::line(Text::new()));
    }
    for (title, section) in cx.parts.sections(&crate::join::of_mr(mr)) {
        blocks.push(Block::line(v::kit::heading(&title)));
        texts.extend(section.iter().map(|line| line.words()));
        blocks.extend(section.into_iter().map(Block::line));
        blocks.push(Block::line(Text::new()));
    }
    blocks.push(Block::line(Text::from(vec![
        Seg::icon(Icon::Thread),
        Seg::new("LATEST ACTIVITY", Ink::Muted).strong(),
    ])));
    let latest = detail
        .and_then(|d| {
            d.discussions
                .iter()
                .flat_map(|thread| thread.notes.iter().map(move |note| (thread, note)))
                .max_by_key(|(_, note)| time::parse_rfc3339(&note.at).unwrap_or(0))
                .map(|(thread, note)| {
                    (
                        note.author.clone(),
                        note.at.clone(),
                        note.body.clone(),
                        thread.place(),
                    )
                })
        })
        .or_else(|| {
            mr.last_comment.as_ref().map(|comment| {
                (
                    comment.author.clone(),
                    comment.at.clone(),
                    comment.body.clone(),
                    None,
                )
            })
        });
    match latest {
        Some((author, at, body, place)) => {
            texts.push(body.clone());
            blocks.extend(crate::screen::comment_blocks(
                p,
                project_path,
                &author,
                &at,
                &body,
                place,
                None,
                None,
                cx.now,
            ));
        }
        None => blocks.push(Block::line(Text::of("No comments yet.", Ink::Muted))),
    }
    *page.links.borrow_mut() = crate::join::links_in(&cx.parts, &written, &texts);
    let main = PaneView::new(
        PaneId::Middle,
        "overview",
        Node::Rich {
            blocks,
            pictures: Some(project_path.into()),
        },
    );
    let side = PaneView::new(
        PaneId::Detail,
        "can it merge?",
        Node::Document {
            lines: checks.clone(),
            cut: true,
        },
    );
    frame.panes = vec![main, side];
    frame.tab = mr::OVERVIEW;
    let wide = PageLayout::split(
        Axis::Horizontal,
        [
            (Size::Fill(1), PageLayout::pane(0)),
            (Size::Length(36), PageLayout::pane(1)),
        ],
    );
    let narrow = PageLayout::split(
        Axis::Vertical,
        [
            (
                Size::Length((checks.len().min(6) + 2) as u16),
                PageLayout::pane(1),
            ),
            (Size::Fill(1), PageLayout::pane(0)),
        ],
    );
    frame.layout = Some(PageLayout::Responsive {
        min_width: 100,
        wide: Box::new(wide),
        narrow: Box::new(narrow),
    });
    Some(view)
}

fn merge_checks(p: &GitlabPlugin, mr: &Mr, detail: Option<&MrDetail>) -> Vec<Text> {
    use crate::ready::Block as Check;
    let blockers = crate::ready::blocks(mr);
    let mark = |icon: Icon, words: String| {
        Text::from(vec![Seg::icon(icon), Seg::new(words, Ink::Primary)])
    };
    let yes = |words: &str| mark(Icon::RunPassed, words.into());
    let info = |words: String| mark(Icon::Info, words);
    let block = |b: &Check| mark(b.icon(), b.words());
    let find = |predicate: fn(&Check) -> bool| blockers.iter().find(|b| predicate(b));
    let mut out = Vec::new();
    if mr.state == crate::model::MrState::Opened {
        out.push(mr_state_text(mr));
    }
    out.push(
        match (find(|b| matches!(b, Check::Pipeline(_))), mr.pipeline) {
            (Some(b), _) => block(b),
            (None, Some(Pipeline::Passed)) => yes("pipeline passed"),
            (None, Some(Pipeline::Skipped)) => yes("pipeline skipped"),
            (None, None) => info("no pipeline".into()),
            (None, Some(pipeline)) => info(format!("pipeline {}, not required", pipeline.word())),
        },
    );
    out.push(
        match (
            find(|b| *b == Check::Conflicts),
            find(|b| *b == Check::Rebase),
        ) {
            (Some(b), _) | (None, Some(b)) => block(b),
            _ => yes("no conflicts, up to date"),
        },
    );
    out.push(match find(|b| matches!(b, Check::Approvals(_))) {
        Some(b) => mark(b.icon(), format!("{} ({})", b.words(), mr.approvals_words())),
        None if mr.approved_by.is_empty() => yes("no approvals needed"),
        None => yes(&format!("approvals {}", mr.approvals_words())),
    });
    let open = detail.map_or(
        mr.threads.saturating_sub(mr.threads_resolved) as usize,
        |d| {
            d.discussions
                .iter()
                .filter(|thread| thread.is_open())
                .count()
        },
    );
    out.push(match find(|b| matches!(b, Check::Threads(_))) {
        Some(_) => block(&Check::Threads(open.max(1) as u32)),
        None if open == 0 => yes("no open threads"),
        None if mr.checks.is_some() => info(format!(
            "{open} open thread{}, not required",
            if open == 1 { "" } else { "s" }
        )),
        None => mark(
            Icon::Thread,
            format!("{open} open thread{}", if open == 1 { "" } else { "s" }),
        ),
    });
    out.push(match find(|b| *b == Check::Draft) {
        Some(b) => block(b),
        None => yes("not a draft"),
    });
    out.extend(
        blockers
            .iter()
            .filter(|b| {
                matches!(
                    b,
                    Check::ChangesRequested | Check::Checking | Check::Blocked | Check::Other(_)
                )
            })
            .map(block),
    );
    out.push(Text::new());
    if !mr.approved_by.is_empty() {
        out.push(Text::from(vec![
            Seg::new("approved by ", Ink::Muted),
            Seg::new(mr.approved_by.join(", "), Ink::Secondary),
        ]));
    }
    let role = mr.role(&p.me);
    let (sentence, word) = match mr.state {
        crate::model::MrState::Opened => (role.sentence(), role.word()),
        crate::model::MrState::Merged => ("nothing: it is merged", "merged"),
        crate::model::MrState::Closed => ("nothing: it was closed", "closed"),
    };
    out.push(Text::from(vec![
        Seg::new("for you: ", Ink::Muted),
        Seg::styled(sentence, p.cfg.style_for(word)),
    ]));
    out
}

fn mr_pipeline_view(
    p: &GitlabPlugin,
    page: &Page,
    project_path: &str,
    iid: u64,
    cx: &v::ViewCx<'_>,
) -> Option<v::ScreenView> {
    let mut view = mr_commits_view(p, page, project_path, iid, cx)?;
    let frame = view.page.as_mut()?;
    frame.tab = mr::PIPELINE;
    let fetched = p.mr_pages.get(&mr_key(project_path, iid));
    let detail = fetched
        .and_then(|f| f.value.as_ref())
        .map(|(_, detail)| detail);
    let mr = find_mr(p, project_path, iid);
    let title = detail
        .and_then(|detail| detail.pipeline.as_ref())
        .filter(|pipeline| !pipeline.iid.is_empty())
        .map_or_else(
            || "pipeline".into(),
            |pipeline| format!("pipeline #{}", pipeline.iid),
        );
    let jobs = mr::job_rows(p, project_path, iid);
    let body = if !jobs.is_empty() {
        let mut rows = Vec::new();
        let mut lead = Vec::new();
        if fetched.is_some_and(|f| f.loading) {
            lead.push(Text::of("loading…", Ink::Muted));
        }
        if let Some(error) = fetched.and_then(|f| f.error.as_deref()) {
            lead.push(v::kit::sync_failed("merge request", error));
        }
        if let Some(mr) = mr {
            let mut head = pipeline_text(p, mr.pipeline);
            if let Some(seconds) = mr.duration_secs {
                head.add(Seg::new(
                    format!(" · {}", radar_core::ui::duration(seconds.into())),
                    Ink::Muted,
                ));
            }
            lead.push(head);
            lead.push(Text::new());
            if crate::why::red(mr.pipeline, mr.pipeline_url.as_deref()).is_some() {
                lead.push(Text::from(vec![
                    Seg::icon(Icon::RunFailed),
                    Seg::new("WHY IT IS RED", Ink::Muted).strong(),
                ]));
            }
        }
        let mut all_jobs = false;
        for (i, job) in jobs.iter().enumerate() {
            match job {
                mr::JobRow::Failed(info) => {
                    let width = (cx.width as usize / 3).clamp(8, 24);
                    let reason = crate::joblog::reason(&info.failure_reason);
                    let mut head = Text::from(vec![
                        Seg::icon(Icon::RunFailed),
                        Seg::new(radar_core::ui::pad(&info.name, width), Ink::Primary).strong(),
                        Seg::new(info.stage.clone(), Ink::AccentSecondary),
                    ]);
                    if let Some(seconds) = info.duration_secs {
                        head.add(Seg::new(
                            format!(" · {}", radar_core::ui::duration(seconds.into())),
                            Ink::Muted,
                        ));
                    }
                    if !reason.is_empty() {
                        head.add(Seg::new(format!(" · {reason}"), Ink::Bad));
                    }
                    if info.allow_failure {
                        head.add(Seg::new(" · allowed to fail", Ink::Muted));
                    }
                    let mut entry = EntryView::new(format!("row:{i}"), head);
                    entry.before = std::mem::take(&mut lead);
                    entry.body =
                        failed_log_lines(p, project_path, info.id, cx.width.saturating_sub(4));
                    rows.push(entry);
                }
                mr::JobRow::Job { stage, first, job } => {
                    let mut before = std::mem::take(&mut lead);
                    if !all_jobs {
                        all_jobs = true;
                        if !rows.is_empty() {
                            before.push(Text::new());
                        }
                        before.push(v::kit::heading("all jobs"));
                    }
                    if *first {
                        let status = detail
                            .and_then(|detail| detail.pipeline.as_ref())
                            .and_then(|pipeline| {
                                pipeline.stages.iter().find(|item| &item.name == stage)
                            })
                            .map_or("", |item| item.status.as_str());
                        let mark = status_mark(status);
                        before.push(Text::from(vec![
                            Seg::icon(mark.icon()),
                            Seg::styled(
                                stage.clone(),
                                p.cfg.style_for(
                                    Pipeline::from_graphql(&status.to_ascii_uppercase())
                                        .map_or("pending", Pipeline::word),
                                ),
                            )
                            .strong(),
                        ]));
                    }
                    let mark = status_mark(&job.status);
                    let mut head = Text::from(vec![
                        Seg::new("  ", Ink::Plain),
                        Seg::icon(mark.icon()),
                        Seg::new(" ", Ink::Plain),
                        Seg::new(radar_core::ui::pad(&job.name, 14), Ink::Primary),
                        Seg::styled(
                            radar_core::ui::pad(&job.status, 10),
                            p.cfg.style_for(
                                Pipeline::from_graphql(&job.status.to_ascii_uppercase())
                                    .map_or("pending", Pipeline::word),
                            ),
                        ),
                        Seg::new(
                            job.duration_secs.map_or_else(String::new, |seconds| {
                                radar_core::ui::duration(seconds.into())
                            }),
                            Ink::Muted,
                        ),
                    ]);
                    if job.allow_failure && job.status == "failed" {
                        head.add(Seg::new(" · allowed to fail", Ink::Muted));
                    }
                    let mut entry = EntryView::new(format!("row:{i}"), head);
                    entry.before = before;
                    rows.push(entry);
                }
            }
        }
        Node::Entries {
            rows,
            selected: Some(page.row.get().min(jobs.len() - 1)),
            empty: "No jobs in this pipeline.".into(),
        }
    } else {
        let mut lines = Vec::new();
        if fetched.is_some_and(|f| f.loading) {
            lines.push(Text::of("loading…", Ink::Muted));
            lines.push(Text::new());
        }
        if let Some(error) = fetched.and_then(|f| f.error.as_deref()) {
            lines.push(v::kit::sync_failed("merge request", error));
            lines.push(Text::new());
        }
        match mr {
            None => lines.push(Text::of(
                "This merge request is not in the last sync.",
                Ink::Muted,
            )),
            Some(mr)
                if mr.pipeline.is_none() && detail.and_then(|d| d.pipeline.as_ref()).is_none() =>
            {
                lines.push(Text::of("No pipeline for this merge request.", Ink::Muted))
            }
            Some(mr) => {
                let mut head = pipeline_text(p, mr.pipeline);
                if let Some(seconds) = mr.duration_secs {
                    head.add(Seg::new(
                        format!(" · {}", radar_core::ui::duration(seconds.into())),
                        Ink::Muted,
                    ));
                }
                lines.push(head);
                lines.push(Text::new());
                for stage in &mr.stages {
                    let mark = status_mark(&stage.status);
                    lines.push(Text::from(vec![
                        Seg::icon(mark.icon()),
                        Seg::new(format!(" {}", stage.name), Ink::Primary),
                    ]));
                }
            }
        }
        Node::Document { lines, cut: false }
    };
    frame.panes = vec![PaneView::new(PaneId::Middle, title, body)];
    Some(view)
}

fn failed_log_lines(p: &GitlabPlugin, project_path: &str, id: u64, width: u16) -> Vec<Text> {
    let bar = || Seg::new("  │ ", Ink::AccentMuted);
    let Some(fetched) = p
        .logs
        .get(&crate::log_ownership::JobKey::new(project_path, id))
    else {
        return vec![];
    };
    if let Some(error) = &fetched.error {
        return vec![Text::from(vec![
            bar(),
            Seg::new(format!("could not load its log: {error}"), Ink::Bad),
        ])];
    }
    let Some(log) = &fetched.value else {
        return vec![Text::from(vec![
            bar(),
            Seg::new("loading its log…", Ink::Muted),
        ])];
    };
    let Some(excerpt) = &log.excerpt else {
        return vec![Text::from(vec![
            bar(),
            Seg::new("Its log is empty.", Ink::Muted),
        ])];
    };
    let mut out = Vec::new();
    if excerpt.error.is_none() {
        out.push(Text::from(vec![
            bar(),
            Seg::new("no error recognised; its last lines:", Ink::Muted),
        ]));
    }
    for i in excerpt.lines.clone() {
        let words = log.text.lines[i].text();
        if words.trim().is_empty() {
            out.push(Text::from(vec![bar()]));
            continue;
        }
        let indent = (words.len() - words.trim_start().len()).min(8);
        let prefix = format!("  │ {}", " ".repeat(indent));
        let ink = if excerpt.error == Some(i) {
            Ink::Bad
        } else {
            Ink::Secondary
        };
        for line in radar_core::ui::wrap_words(
            words.trim_start(),
            width.saturating_sub(5 + indent as u16),
            radar_core::ui::width(&prefix),
        ) {
            out.push(Text::from(vec![
                Seg::new(prefix.clone(), Ink::AccentMuted),
                Seg::new(line, ink),
            ]));
        }
    }
    out
}

pub(crate) fn intent(p: &mut GitlabPlugin, pane: Pane, intent: &v::Intent) -> Outcome {
    match intent {
        v::Intent::Select { target } | v::Intent::Activate { target }
            if target.as_str().starts_with("under:") =>
        {
            click(p, target.as_str())
        }
        v::Intent::Select { target } | v::Intent::Activate { target }
            if v::TabView::index(target).is_some() =>
        {
            Outcome::Handled(show_tab(p, v::TabView::index(target).unwrap_or(0)))
        }
        v::Intent::Select { target }
        | v::Intent::Context { target }
        | v::Intent::Activate { target } => {
            if target.as_str().starts_with("node:") {
                return click(p, target.as_str());
            }
            if target.as_str().starts_with("line:") || target.as_str().starts_with("comment:") {
                return click(p, target.as_str());
            }
            if pane == Pane::Nav
                && (target.as_str().starts_with("d:") || target.as_str().starts_with("f:"))
                && let Some(page) = p.pages.last()
            {
                page.tree.borrow_mut().select(v::TreeRow::path_of(target));
                if matches!(intent, v::Intent::Activate { .. }) {
                    return crate::pages::activate(p, pane);
                }
                return Outcome::done();
            }
            if let Some(row) = target
                .as_str()
                .strip_prefix("log:")
                .and_then(|s| s.parse::<usize>().ok())
                && let Some(page) = p.pages.last()
                && let Kind::Job { project, id } = &page.kind
            {
                job::click(p, page, project, *id, row);
                return Outcome::done();
            }
            let Some(i) = target
                .as_str()
                .strip_prefix("row:")
                .and_then(|s| s.parse::<usize>().ok())
            else {
                return Outcome::Ignored;
            };
            let Some(page) = p.pages.last() else {
                return Outcome::Ignored;
            };
            // A thread row from before the threads were loaded again is not
            // the row now there: wait for the next frame.
            if matches!(page.kind, Kind::Mr { .. })
                && page.tab == mr::THREADS
                && !page.detail_painted.get()
            {
                return Outcome::done();
            }
            page.row.set(i);
            if matches!(intent, v::Intent::Activate { .. }) {
                crate::pages::activate(p, pane)
            } else {
                Outcome::done()
            }
        }
        v::Intent::Fold { target, open } => {
            // A folder of the Changes tab's files.
            let Some(page) = p.pages.last() else {
                return Outcome::Ignored;
            };
            let path = v::TreeRow::path_of(target);
            if *open {
                page.tree.borrow_mut().open(path);
            } else {
                let mut tree = page.tree.borrow_mut();
                crate::nav::NavExt::close(&mut *tree, &path);
                if tree.selected().len() > path.len() && tree.selected().starts_with(&path) {
                    tree.select(path);
                }
            }
            Outcome::done()
        }
        v::Intent::Key { .. } | v::Intent::Invoke { .. } => {
            intent.chord().map_or(Outcome::Ignored, |chord| {
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
        _ => Outcome::Ignored,
    }
}
