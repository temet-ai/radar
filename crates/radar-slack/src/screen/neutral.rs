//! Slack's feed as data for terminal and future graphical frontends.

use std::collections::BTreeMap;

use radar_core::Plugin;
use radar_core::plugin::{Outcome, Pane};
use radar_core::ui::icon::Icon;
use radar_core::view::{
    self as v, Block, ColumnSpec, Ink, Node, PaneId, PaneView, Seg, Text, TreeRow,
};

use super::{
    Item, NavRow, for_you_groups, found_activity, found_thread, nav_row, nav_rows, thread_passes,
};
use crate::alerts::State as AlertState;
use crate::model::{Thread, ts_key};
use crate::{SlackPlugin, alert_view};

pub fn view(p: &SlackPlugin, cx: &v::ViewCx<'_>) -> v::ScreenView {
    let mut screen = v::ScreenView::new(
        crate::SCREEN.id,
        vec![nav_pane(p), middle_pane(p, cx), detail_pane(p, cx)],
    );
    screen.actions = crate::actions(&p.rows(Pane::from(cx.focus)));
    if let Some(reply) = &p.replying {
        let title = format!(
            "✎ reply in {}",
            p.reply_place(&reply.channel, &reply.thread)
        );
        screen.overlay = Some(Box::new(PaneView::new(
            PaneId::Middle,
            &title,
            v::Editor {
                title: title.clone(),
                lines: reply.composer.lines.clone(),
                line: reply.composer.input.value(),
                caret: reply.composer.input.cursor(),
                selection: reply.composer.input.selection(),
            }
            .node(),
        )));
    }
    screen
}

pub fn intent(p: &mut SlackPlugin, pane: Pane, intent: &v::Intent) -> Outcome {
    match intent {
        v::Intent::Select { target } | v::Intent::Context { target }
            if target.as_str().starts_with("rail:") =>
        {
            let Some(i) = target
                .as_str()
                .strip_prefix("rail:")
                .and_then(|n| n.parse().ok())
            else {
                return Outcome::Ignored;
            };
            super::pick_rail(p, i);
            Outcome::done()
        }
        v::Intent::Select { target }
        | v::Intent::Context { target }
        | v::Intent::Activate { target } => {
            let path = TreeRow::path_of(target);
            if !p.shown.borrow().contains_key(&path) {
                return Outcome::Ignored;
            }
            let before = p.tree.borrow().selected().to_vec();
            p.tree.borrow_mut().select(path);
            super::moved(p, &before);
            if matches!(intent, v::Intent::Activate { .. }) {
                return p.handle_key(Pane::Middle,
                    radar_core::view_tui::key_of(v::Chord::new(v::Key::Enter)),
                );
            }
            Outcome::done()
        }
        v::Intent::Fold { target, open } => {
            let path = TreeRow::path_of(target);
            if *open {
                p.tree.borrow_mut().open(path);
            } else {
                let mut state = p.tree.borrow_mut();
                state.close(&path);
                if state.selected().starts_with(&path) && state.selected().len() > path.len() {
                    state.select(path);
                }
            }
            Outcome::done()
        }
        v::Intent::Key { .. } | v::Intent::Invoke { .. } => {
            intent.chord().map_or(Outcome::Ignored, |chord| {
                if !p.capturing()
                    && matches!(chord.key, v::Key::Up | v::Key::Char('k'))
                    && !chord.ctrl
                    && !chord.alt
                {
                    return Outcome::Ignored;
                }
                p.handle_key(pane, radar_core::view_tui::key_of(chord))
            })
        }
        v::Intent::EditSelect { anchor, extent } => match p.replying.as_mut() {
            Some(reply) => {
                reply.composer.input.select(*anchor, *extent);
                Outcome::done()
            }
            None => Outcome::Ignored,
        },
        v::Intent::EditUnselect => match p.replying.as_mut() {
            Some(reply) => {
                reply.composer.input.unselect();
                Outcome::done()
            }
            None => Outcome::Ignored,
        },
        v::Intent::Show { .. } | v::Intent::End { .. } | v::Intent::Dock(_) | v::Intent::Copy { .. } | v::Intent::NoticeInput { .. } | v::Intent::DismissNotice { .. } | v::Intent::OpenAll => Outcome::Ignored,
    }
}

fn nav_pane(p: &SlackPlugin) -> PaneView {
    let rows = nav_rows(p)
        .into_iter()
        .enumerate()
        .map(|(i, r)| {
            let (icon, name, right) = match r {
                NavRow::ForYou => (
                    Icon::Mention,
                    "For you".to_string(),
                    Some(super::for_you_count(p)),
                ),
                NavRow::Discussions => (Icon::Thread, "Discussions".to_string(), None),
                NavRow::Alerts => (
                    Icon::Alert,
                    "Alerts".to_string(),
                    Some(alert_view::firing(p)),
                ),
                NavRow::Channel(i) => {
                    let c = &p.cfg.channels[i];
                    (
                        crate::view::channel_icon(&c.id),
                        format!("#{}", c.alias),
                        Some(p.data.new_count(&c.id)),
                    )
                }
            };
            let mut label = Text::new();
            if let NavRow::Channel(index) = r {
                label.add(Seg::styled(
                    "▌ ",
                    format!("slack.{}", p.cfg.channels[index].color),
                ));
            }
            label.add(Seg::icon(icon));
            label.add(Seg::new(name, Ink::Emphasis));
            let mut row = v::ListRow::new(format!("rail:{i}"), label);
            row.right = right.filter(|n| *n > 0).map(|n| {
                let label = if matches!(r, NavRow::Channel(_)) {
                    format!("{n} NEW")
                } else {
                    n.to_string()
                };
                Text::of(label, Ink::Info)
            });
            if let NavRow::Channel(i) = r {
                let channel = &p.cfg.channels[i];
                if p.sync
                    .errors
                    .contains_key(&crate::SyncErrorKey::Channel(channel.alias.clone()))
                {
                    row.right = Some(Text::from(vec![Seg::icon_or(Icon::Error, "✗", Ink::Bad)]));
                }
            }
            row
        })
        .collect();
    let mut pane = PaneView::new(
        PaneId::Nav,
        "feed",
        Node::List {
            rows,
            selected: Some(p.nav_at),
            editing: None,
        },
    );
    pane.width = Some(26);
    pane
}

fn thread_row(
    p: &SlackPlugin,
    th: &Thread<'_>,
    with_channel: bool,
    path: &[String],
    shown: &mut BTreeMap<Vec<String>, Item>,
) -> TreeRow {
    let mut name = Text::new();
    if p.data.is_new(th.channel, &th.parent.ts, &th.parent.ts) {
        name.add(Seg::icon_or_lead(Icon::Unread, "● ", Ink::Accent));
    }
    if with_channel {
        name.add(Seg::styled(
            format!("#{} ", p.alias(th.channel)),
            p.cfg.style_for("channel"),
        ));
    }
    name.add(Seg::new(crate::view::who(&p.data, th.parent), Ink::Emphasis).strong());
    name.add(Seg::new(
        format!("  {}", crate::view::first_line(&p.data, &p.cfg, th.parent)),
        Ink::Primary,
    ));
    let mut row = TreeRow::new(&th.parent.ts, name)
        .glow(crate::join::motion_key(th.channel, &th.parent.ts))
        .cell(
            "when",
            Text::of(crate::view::age(th.last(), p.now), Ink::Muted),
        );
    if th.reply_count() > 0 {
        row = row.cell(
            "replies",
            Text::from(vec![
                Seg::icon(Icon::Reply),
                Seg::new(th.reply_count().to_string(), Ink::Secondary),
            ]),
        );
    }
    let mut here = path.to_vec();
    here.push(th.parent.ts.clone());
    shown.insert(
        here.clone(),
        Item::Thread {
            channel: th.channel.into(),
            ts: th.parent.ts.clone(),
        },
    );
    for reply in th.replies {
        let mut reply_path = here.clone();
        reply_path.push(reply.ts.clone());
        shown.insert(
            reply_path,
            Item::Reply {
                channel: th.channel.into(),
                thread: th.parent.ts.clone(),
                ts: reply.ts.clone(),
            },
        );
        let name = Text::from(vec![
            Seg::new(crate::view::who(&p.data, reply), Ink::Emphasis),
            Seg::new(
                format!("  {}", crate::view::first_line(&p.data, &p.cfg, reply)),
                Ink::Primary,
            ),
        ]);
        row = row.child(TreeRow::new(&reply.ts, name).cell(
            "when",
            Text::of(crate::view::age(&reply.ts, p.now), Ink::Muted),
        ));
    }
    row
}

fn middle_pane(p: &SlackPlugin, cx: &v::ViewCx<'_>) -> PaneView {
    let mut shown = BTreeMap::new();
    let mut rows = Vec::new();
    let (title, header) = match nav_row(p) {
        NavRow::ForYou => {
            for (id, words, items) in for_you_groups(p) {
                let items: Vec<_> = items
                    .into_iter()
                    .filter(|i| super::found_passes(p, &p.data.for_you[*i]))
                    .collect();
                if items.is_empty() {
                    continue;
                }
                let mut group = TreeRow::new(
                    id,
                    Text::from(vec![
                        Seg::icon(Icon::Mention),
                        Seg::new(words, Ink::Emphasis),
                    ]),
                );
                shown.insert(vec![id.into()], Item::Heading(words.into()));
                for i in items {
                    let f = &p.data.for_you[i];
                    let key = format!("{}-{}", f.channel_id, f.message.ts);
                    shown.insert(vec![id.into(), key.clone()], Item::Found(i));
                    let mut name = Text::new();
                    if p.data.is_new(&f.channel_id, found_thread(f), &f.message.ts) {
                        name.add(Seg::icon_or_lead(Icon::Unread, "● ", Ink::Accent));
                    }
                    name.add(Seg::styled(
                        format!("#{} ", p.alias(&f.channel_id)),
                        p.cfg.style_for("channel"),
                    ));
                    name.add(Seg::new(
                        crate::view::who(&p.data, &f.message),
                        Ink::Emphasis,
                    ));
                    name.add(Seg::new(
                        format!("  {}", crate::view::first_line(&p.data, &p.cfg, &f.message)),
                        Ink::Primary,
                    ));
                    group = group.child(TreeRow::new(key, name).cell(
                        "when",
                        Text::of(crate::view::age(found_activity(p, f), p.now), Ink::Muted),
                    ));
                }
                rows.push(group);
            }
            let yours: Vec<_> = p
                .data
                .yours(&p.cfg)
                .into_iter()
                .filter(|th| thread_passes(p, th))
                .collect();
            if !yours.is_empty() {
                let path = vec!["threads".into()];
                shown.insert(path.clone(), Item::Heading("threads you are in".into()));
                rows.push(
                    TreeRow::new(
                        "threads",
                        Text::from(vec![
                            Seg::icon(Icon::Thread),
                            Seg::new("threads you are in", Ink::Emphasis),
                        ]),
                    )
                    .children(
                        yours
                            .iter()
                            .map(|th| thread_row(p, th, true, &path, &mut shown)),
                    ),
                );
            }
            (
                "For you".to_string(),
                format!("last {} days", crate::read::FOR_YOU_DAYS),
            )
        }
        NavRow::Discussions => {
            let threads: Vec<_> = p
                .data
                .all_threads(&p.cfg)
                .into_iter()
                .filter(|th| {
                    p.cfg.by_id(th.channel).is_some_and(|c| {
                        !matches!(
                            c.kind,
                            crate::config::Kind::Test | crate::config::Kind::Alerts
                        )
                    })
                })
                .filter(|th| thread_passes(p, th))
                .collect();
            let mut groups: Vec<(radar_core::Subject, Vec<usize>)> = Vec::new();
            let mut general = Vec::new();
            for (i, th) in threads.iter().enumerate() {
                let subjects = subjects_of(cx, th);
                if subjects.is_empty() {
                    general.push(i);
                }
                for subject in subjects {
                    match groups.iter_mut().find(|(s, _)| *s == subject) {
                        Some((_, list)) => list.push(i),
                        None => groups.push((subject, vec![i])),
                    }
                }
            }
            for (subject, indices) in groups {
                let id = format!("{}:{}", subject.kind, subject.id);
                let path = vec![id.clone()];
                let mut name = Text::of(subject.label.clone(), Ink::Emphasis);
                if let Some(title) = subject.field("title") {
                    name.add(Seg::new(format!("  {title}"), Ink::Secondary));
                }
                let mut row = TreeRow::new(id, name);
                for chip in cx.parts.chips(&subject) {
                    row = row.chips(1, Text::from(vec![Seg::chip(&chip)]));
                }
                row.children = indices
                    .into_iter()
                    .map(|i| thread_row(p, &threads[i], true, &path, &mut shown))
                    .collect();
                shown.insert(path, Item::Subject(subject));
                rows.push(row);
            }
            if !general.is_empty() {
                let path = vec!["general".into()];
                shown.insert(path.clone(), Item::Heading("general".into()));
                rows.push(
                    TreeRow::new("general", Text::of("general", Ink::Emphasis)).children(
                        general
                            .into_iter()
                            .map(|i| thread_row(p, &threads[i], true, &path, &mut shown)),
                    ),
                );
            }
            (
                "Discussions".into(),
                if cx.parts.has("merge-request") || cx.parts.has("ticket") {
                    "BY WHAT THEY ARE ABOUT".into()
                } else {
                    "THE MOST ACTIVE FIRST".into()
                },
            )
        }
        NavRow::Alerts => {
            rows = alert_rows(p, &mut shown);
            ("Alerts".into(), "firing first, then what resolved".into())
        }
        NavRow::Channel(i) => {
            let channel = &p.cfg.channels[i];
            let mut days: Vec<(String, Vec<Thread<'_>>)> = Vec::new();
            let mut threads = p.data.threads(&channel.id);
            threads.retain(|th| thread_passes(p, th));
            threads.sort_by_key(|th| std::cmp::Reverse(ts_key(th.last())));
            for th in threads {
                let day = crate::view::day(th.last(), p.now);
                match days.last_mut() {
                    Some((d, list)) if *d == day => list.push(th),
                    _ => days.push((day, vec![th])),
                }
            }
            for (day, threads) in days {
                let path = vec![day.clone()];
                shown.insert(path.clone(), Item::Heading(day.clone()));
                rows.push(
                    TreeRow::new(&day, Text::of(&day, Ink::Emphasis))
                        .stripe(format!("slack.{}", channel.color))
                        .children(
                            threads
                                .iter()
                                .map(|th| thread_row(p, th, false, &path, &mut shown)),
                        ),
                );
            }
            let n = p.data.new_count(&channel.id);
            (
                format!("#{}", channel.alias),
                if n > 0 {
                    format!("{n} new")
                } else {
                    "CAUGHT UP".into()
                },
            )
        }
    };
    *p.shown.borrow_mut() = shown;
    open_groups(p, &rows);
    let state = p.tree.borrow();
    let note = rows.is_empty().then(|| v::Note {
        state: v::State::Empty,
        text: if p.cfg.channels.is_empty() {
            "No channels yet. Press , for settings, then add one under channels: paste a link to it or type its name.".into()
        } else if p.cfg.token.trim().is_empty() {
            "Not set up: press , for settings, then set the token under connection.".into()
        } else if p.sync.last_ok.is_none()
            && p.data.channels.is_empty()
            && p.data.for_you.is_empty()
        {
            "Reading Slack…".into()
        } else {
            "Nothing to show in this feed yet.".into()
        },
    });
    let tree = v::Tree {
        header: Text::of(header, Ink::Secondary),
        columns: if nav_row(p) == NavRow::Alerts {
            vec![
                ColumnSpec::new("priority", 9, 1),
                ColumnSpec::new("notified", 9, 2),
                ColumnSpec::new("for", 8, 3).icon(Icon::Time),
            ]
        } else {
            vec![
                ColumnSpec::new("replies", 9, 1).icon(Icon::Reply),
                ColumnSpec::new("when", 10, 2).icon(Icon::Time),
            ]
        },
        ordered_columns: false,
        rows,
        selected: state.selected().to_vec(),
        open: {
            // A set: sorted, so two hosts send the same view.
            let mut open: Vec<Vec<String>> = state.opened().iter().cloned().collect();
            open.sort();
            open
        },
        name_width: 30,
        look: Some(v::TreeLook {
            lines: p.cfg.style_for("lines"),
            band: p.cfg.style_for("band"),
            heading: p.cfg.style_for("heading"),
            band_level: 1,
        }),
        header_target: None,
        note,
        cut_names: false,
        editing: None,
        pointer_action: false,
    };
    let mut pane = PaneView::new(PaneId::Middle, title, Node::Tree(tree));
    pane.notes = notes(p);
    pane.notes_most = Some(4);
    if p.filter_input.is_some() || !p.filter.is_empty() {
        pane.fields.push(v::FieldBar {
            mark: "/".into(),
            text: p.filter.clone(),
            caret: p.filter_input.as_ref().map(|i| i.cursor()),
            hint: Text::of("filter by words", Ink::Muted),
        });
    }
    if let Some(mapping) = &p.mapping {
        pane.head = Some(Text::of(
            format!("{} belongs to (a repo path, or a service):", mapping.name),
            Ink::Accent,
        ));
        pane.bottom_field = Some(v::FieldBar {
            mark: "⏎".into(),
            text: mapping.input.value(),
            caret: Some(mapping.input.cursor()),
            hint: Text::of(
                format!("{} belongs to (a repo path, or a service):", mapping.name),
                Ink::Accent,
            ),
        });
    }
    pane
}

fn subjects_of(cx: &v::ViewCx<'_>, th: &Thread<'_>) -> Vec<radar_core::Subject> {
    let mut out = Vec::new();
    for text in th.links().into_iter().chain(
        th.texts()
            .map(|text| crate::mrkdwn::plain(text, &crate::mrkdwn::NoNames)),
    ) {
        for (_, subject) in cx.parts.recognise(&text) {
            if !subject.is(crate::join::THREAD)
                && !subject.is(radar_core::compose::LINK)
                && !out.contains(&subject)
            {
                out.push(subject);
            }
        }
    }
    out
}

fn open_groups(p: &SlackPlugin, rows: &[TreeRow]) {
    let mut state = p.tree.borrow_mut();
    let mut seen = p.opened_once.borrow_mut();
    for row in rows {
        let path = vec![row.id.clone()];
        if !row.children.is_empty() && seen.insert(path.clone()) {
            state.open(path);
        }
        if nav_row(p) == NavRow::Alerts {
            for child in &row.children {
                let path = vec![row.id.clone(), child.id.clone()];
                if !child.children.is_empty() && seen.insert(path.clone()) {
                    state.open(path);
                }
            }
        }
    }
    if state.selected().is_empty()
        && let Some(first) = rows.first()
    {
        let mut path = vec![first.id.clone()];
        if let Some(child) = first.children.first() {
            path.push(child.id.clone());
            if nav_row(p) == NavRow::Alerts
                && let Some(alert) = child.children.first()
            {
                path.push(alert.id.clone());
            }
        }
        state.select(path);
    }
}

fn notes(p: &SlackPlugin) -> Vec<Text> {
    let mut notes: Vec<_> = p
        .sync
        .errors
        .iter()
        .map(|(key, error)| {
            let name = match key {
                crate::SyncErrorKey::Bootstrap => "Slack".into(),
                crate::SyncErrorKey::ForYou => "For you".into(),
                crate::SyncErrorKey::Followed => "Followed review asks".into(),
                crate::SyncErrorKey::Channel(alias) => format!("#{alias}"),
            };
            v::kit::sync_failed(&name, error)
        })
        .collect();
    match nav_row(p) {
        NavRow::ForYou => notes.push(Text::of(
            format!(
                "mentions, messages and new replies in the last {} days",
                crate::read::FOR_YOU_DAYS
            ),
            Ink::Secondary,
        )),
        NavRow::Discussions => notes.push(Text::of(
            "threads in watched channels, grouped by linked work",
            Ink::Secondary,
        )),
        NavRow::Alerts => notes.extend(
            alert_view::about(p)
                .into_iter()
                .map(|line| Text::of(line, Ink::Muted)),
        ),
        NavRow::Channel(i) => notes.push(Text::of(
            format!(
                "the last {} days · {}",
                p.cfg.days, p.cfg.channels[i].purpose
            ),
            Ink::Secondary,
        )),
    }
    notes
}

fn alert_rows(p: &SlackPlugin, shown: &mut BTreeMap<Vec<String>, Item>) -> Vec<TreeRow> {
    let alerts = alert_view::shown(p);
    let mut out = Vec::new();
    for (state, title) in [
        (AlertState::Firing, "firing"),
        (AlertState::Failed, "failed jobs"),
        (AlertState::Resolved, "recently resolved"),
    ] {
        let selected: Vec<_> = alerts.iter().filter(|a| a.state == state).collect();
        if selected.is_empty() {
            continue;
        }
        let sid = format!("s:{}", state.word());
        shown.insert(vec![sid.clone()], Item::Heading(title.into()));
        let mut section = TreeRow::new(&sid, Text::of(title, Ink::Emphasis));
        let mut groups: Vec<((bool, String, String), Vec<_>)> = Vec::new();
        for alert in selected {
            let service = if alert.place.mapped() {
                alert.place.service.clone()
            } else {
                crate::alerts::UNMAPPED.into()
            };
            let key = (!alert.place.mapped(), service, alert.environment.clone());
            match groups.iter_mut().find(|(k, _)| *k == key) {
                Some((_, list)) => list.push(alert),
                None => groups.push((key, vec![alert])),
            }
        }
        groups.sort_by(|a, b| a.0.cmp(&b.0));
        for ((unmapped, service, environment), alerts) in groups {
            let gid = format!("g:{service}:{environment}");
            let ink = if unmapped { Ink::Warn } else { Ink::Emphasis };
            let group_name = Text::from(vec![
                Seg::new(service.clone(), ink),
                Seg::new(format!("  {environment}"), Ink::Secondary),
            ]);
            shown.insert(
                vec![sid.clone(), gid.clone()],
                Item::Heading(format!("{service} {environment}").trim().into()),
            );
            let mut group = TreeRow::new(&gid, group_name);
            for alert in alerts {
                let id = format!("{}-{}", alert.channel, alert.ts);
                shown.insert(
                    vec![sid.clone(), gid.clone(), id.clone()],
                    Item::Alert {
                        channel: alert.channel.clone(),
                        ts: alert.ts.clone(),
                    },
                );
                let ink = if alert.state == AlertState::Resolved {
                    Ink::Secondary
                } else {
                    Ink::Primary
                };
                let mut row = TreeRow::new(
                    id,
                    Text::from(vec![
                        Seg::icon_or_lead(
                            alert.state.icon(),
                            format!("{} ", alert.state.mark()),
                            ink,
                        ),
                        Seg::new(alert.name.clone(), ink),
                        Seg::new(
                            if alert.notices > 1 {
                                format!(" ×{}", alert.notices)
                            } else {
                                String::new()
                            },
                            Ink::Muted,
                        ),
                    ]),
                );
                if !alert.summary.is_empty() {
                    row = row.detail(Text::of(format!("  {}", alert.summary), Ink::Muted));
                }
                row = row
                    .cell("priority", Text::of(&alert.priority, Ink::Warn))
                    .cell(
                        "notified",
                        Text::of(crate::view::age(&alert.last_ts, p.now), Ink::Muted),
                    )
                    .cell(
                        "for",
                        Text::of(
                            alert
                                .lasted(p.now)
                                .map_or_else(|| "—".into(), alert_view::lasted),
                            ink,
                        ),
                    );
                group = group.child(row);
            }
            section = section.child(group);
        }
        out.push(section);
    }
    out
}

pub(super) fn detail_pane(p: &SlackPlugin, cx: &v::ViewCx<'_>) -> PaneView {
    let item = super::selected(p);
    let title = match &item {
        Some(Item::Subject(s)) => s.label.clone(),
        Some(Item::Heading(s)) => s.clone(),
        Some(Item::Alert { .. }) => "alert".into(),
        Some(item) => super::message_title(p, item).unwrap_or_else(|| "message".into()),
        None => "detail".into(),
    };
    let mut blocks = Vec::new();
    match item {
        None => blocks.push(Block::line(Text::of("Nothing selected.", Ink::Muted))),
        Some(Item::Heading(title)) => blocks.push(Block::line(Text::of(title, Ink::Secondary))),
        Some(Item::Subject(subject)) => {
            blocks.push(Block::line(Text::of(subject.label.clone(), Ink::Emphasis)));
            if let Some(title) = subject.field("title") {
                blocks.push(Block::line(Text::of(title, Ink::Primary)));
            }
            if let Some(url) = subject.url {
                blocks.push(Block::line(Text::from(vec![
                    Seg::new(url.clone(), Ink::Accent).linked(url),
                ])));
            }
        }
        Some(Item::Alert { channel, ts }) => {
            if let Some(alert) = alert_view::find(p, &channel, &ts) {
                blocks.push(Block::line(Text::of(alert.name.clone(), Ink::Emphasis)));
                let when = |seconds| crate::view::when_secs(seconds, cx.now);
                let state = match alert.state {
                    AlertState::Firing => format!(
                        "▲ firing for {} · since {}",
                        alert_view::lasted(alert.lasted(cx.now).unwrap_or(0)),
                        when(alert.since)
                    ),
                    AlertState::Resolved => match (alert.lasted(cx.now), alert.ended) {
                        (Some(duration), Some(ended)) => format!(
                            "✓ resolved after {} · {} to {}",
                            alert_view::lasted(duration),
                            when(alert.since),
                            when(ended)
                        ),
                        (_, ended) => format!(
                            "✓ resolved {} · its firing is before what radar has read",
                            when(ended.unwrap_or(alert.since))
                        ),
                    },
                    AlertState::Failed => format!(
                        "✗ failed {} ago · {}",
                        alert_view::lasted(cx.now - alert.since),
                        when(alert.since)
                    ),
                };
                blocks.push(Block::line(Text::of(state, Ink::Warn)));
                if alert.notices > 1 {
                    blocks.push(Block::line(Text::of(
                        format!(
                            "notified {} times, the last {} ago",
                            alert.notices,
                            alert_view::lasted(cx.now - alert.changed())
                        ),
                        Ink::Muted,
                    )));
                }
                blocks.push(Block::line(Text::new()));
                for (key, value) in [
                    ("environment", &alert.environment),
                    ("priority", &alert.priority),
                ] {
                    if !value.is_empty() {
                        blocks.push(Block::line(Text::from(vec![
                            Seg::new(format!("{key}  "), Ink::Muted),
                            Seg::new(value.clone(), Ink::Primary),
                        ])));
                    }
                }
                blocks.push(Block::line(Text::of(
                    format!("channel  #{}", p.alias(&channel)),
                    Ink::Primary,
                )));
                blocks.push(Block::line(Text::of(
                    format!("read as  {}", alert.source.word()),
                    Ink::Primary,
                )));
                if !alert.summary.is_empty() {
                    blocks.push(Block::line(Text::new()));
                    blocks.push(Block::line(Text::of(alert.summary.clone(), Ink::Primary)));
                }
                if let Some(url) = &alert.link {
                    blocks.push(Block::line(Text::new()));
                    blocks.push(Block::line(Text::of("CONSOLE", Ink::Muted)));
                    blocks.push(Block::line(Text::from(vec![
                        Seg::new(url.clone(), Ink::Accent).linked(url.clone()),
                    ])));
                }
                blocks.push(Block::line(Text::new()));
                blocks.push(Block::line(Text::of("BELONGS TO", Ink::Muted)));
                if alert.place.mapped() {
                    let mut location = format!("service {}", alert.place.service);
                    if let Some(repo) = &alert.place.repo {
                        location.push_str(&format!(" · repo {repo}"));
                    }
                    blocks.push(Block::line(Text::of(location, Ink::Primary)));
                } else {
                    blocks.push(Block::line(Text::of("unmapped: no rule in settings places it on a repo or service; the x menu maps it.", Ink::Warn)));
                }
                if let Some(thread) = p.data.thread(&channel, &ts) {
                    blocks.push(Block::line(Text::new()));
                    let n = thread.reply_count();
                    blocks.push(Block::line(Text::of(
                        if n == 0 {
                            "no replies in its thread".into()
                        } else {
                            format!("{} in its thread", crate::view::replies(n))
                        },
                        Ink::Muted,
                    )));
                }
            }
        }
        Some(item) => {
            if let Some(message) = super::detail::resolve(p, &item) {
                return super::detail::pane(p, cx, title, &message);
            }
        }
    }
    PaneView::new(
        PaneId::Detail,
        title,
        Node::Rich {
            blocks,
            pictures: Some("slack".into()),
        },
    )
}
