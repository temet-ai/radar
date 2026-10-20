//! Slack's thread and review ask pages as renderer-neutral descriptions.

use radar_core::plugin::{Outcome, Pane};
use radar_core::ui::icon::Icon;
use radar_core::view::{
    self as v, Axis, Block, Ink, Node, PageLayout, PageView, PaneId, PaneView, Seg, Size, TabView,
    Text,
};

use super::{LinkScope, Page, THREAD_TABS, page_thread, retry_words};
use crate::model::Thread;
use crate::{SlackPlugin, thread_key};

pub fn view(p: &SlackPlugin, cx: &v::ViewCx<'_>) -> Option<v::ScreenView> {
    let page = p.pages.last()?;
    let frame = match page {
        Page::Thread {
            channel,
            ts,
            tab,
            row,
        } => thread_view(p, cx, channel, ts, *tab, *row),
        Page::Draft => draft_view(p, cx),
    };
    let mut screen = v::ScreenView::page(crate::SCREEN.id, frame);
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
    Some(screen)
}

pub fn intent(p: &mut SlackPlugin, pane: Pane, intent: &v::Intent) -> Outcome {
    match intent {
        v::Intent::Select { target }
        | v::Intent::Activate { target }
        | v::Intent::Context { target }
            if TabView::index(target).is_some() =>
        {
            Outcome::Handled(super::show_tab(p, TabView::index(target).unwrap_or(0)))
        }
        v::Intent::Select { target }
        | v::Intent::Context { target }
        | v::Intent::Activate { target }
            if target.as_str().starts_with("link:") =>
        {
            let Some(i) = target
                .as_str()
                .strip_prefix("link:")
                .and_then(|s| s.parse::<usize>().ok())
            else {
                return Outcome::Ignored;
            };
            let len = super::page_links(p).len();
            if i >= len {
                return Outcome::Ignored;
            }
            if let Some(Page::Thread { row, .. }) = p.pages.last_mut() {
                *row = i;
            }
            if matches!(intent, v::Intent::Activate { .. }) {
                return p.handle_key(pane,
                    v::Chord::new(v::Key::Enter),
                );
            }
            Outcome::done()
        }
        v::Intent::Select { target }
        | v::Intent::Context { target }
        | v::Intent::Activate { target }
            if target.as_str().starts_with("item:") =>
        {
            let Some(i) = target
                .as_str()
                .strip_prefix("item:")
                .and_then(|s| s.parse::<usize>().ok())
            else {
                return Outcome::Ignored;
            };
            let Some(draft) = p.draft.as_mut() else {
                return Outcome::Ignored;
            };
            if i >= draft.items.len() {
                return Outcome::Ignored;
            }
            draft.row = i;
            if matches!(intent, v::Intent::Activate { .. }) {
                return p.handle_key(pane,
                    v::Chord::new(v::Key::Enter),
                );
            }
            Outcome::done()
        }
        v::Intent::Key { .. } | v::Intent::Invoke { .. } => {
            intent.chord().map_or(Outcome::Ignored, |chord| {
                if !radar_core::Plugin::capturing(p)
                    && matches!(chord.key, v::Key::Up | v::Key::Char('k'))
                    && !chord.ctrl
                    && !chord.alt
                    && matches!(p.pages.last(), Some(Page::Thread { tab: 1, row: 0, .. }))
                {
                    return Outcome::Ignored;
                }
                p.handle_key(pane, chord)
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
        v::Intent::End { .. } | v::Intent::Dock(_) | v::Intent::Copy { .. } | v::Intent::NoticeInput { .. } | v::Intent::DismissNotice { .. } | v::Intent::OpenAll => Outcome::Ignored,
        _ => Outcome::Ignored,
    }
}

fn thread_view(
    p: &SlackPlugin,
    cx: &v::ViewCx<'_>,
    channel: &str,
    ts: &str,
    tab: usize,
    row: usize,
) -> PageView {
    let alias = p.alias(channel);
    let got = page_thread(p, channel, ts);
    let links = got
        .as_ref()
        .map(|(th, _)| thread_links(cx, th))
        .unwrap_or_default();
    p.page_links.replace(links.clone());
    p.page_links_scope.replace(Some(LinkScope {
        read: p.read_stamp(),
        channel: channel.into(),
        ts: ts.into(),
    }));
    let mut crumbs = vec![
        Seg::new("Slack", Ink::Secondary),
        Seg::styled(format!("#{alias}"), p.colour(channel)),
        Seg::new("thread", Ink::Secondary),
    ];
    let mut title = Text::of("thread", Ink::Emphasis);
    let mut details = Text::new();
    if let Some((th, _)) = &got {
        let first = crate::view::first_line(&p.data, &p.cfg, th.parent);
        crumbs[2] = Seg::new(
            super::crumb(&p.data.author(th.parent), &first),
            Ink::Secondary,
        );
        title = Text::of(first, Ink::Emphasis);
        details = Text::from(vec![
            Seg::icon(crate::view::author_icon(
                p.data.me.as_ref().map(|m| m.user_id.as_str()),
                th.parent,
            )),
            Seg::new(p.data.author(th.parent), Ink::Emphasis),
            Seg::new(
                format!(" · {} ago · ", crate::view::age(&th.parent.ts, cx.now)),
                Ink::Muted,
            ),
            Seg::icon(Icon::Reply),
            Seg::new(crate::view::replies(th.reply_count()), Ink::Secondary),
        ]);
    }
    let tabs = vec![
        TabView::new(THREAD_TABS[0], Some(Icon::Thread)),
        TabView::new(THREAD_TABS[1], Some(Icon::Link)),
    ];
    let fetched = p.threads.get(&thread_key(channel, ts));
    let status =
        fetched.and_then(|f| v::kit::fetching(f.loading, got.is_some(), f.error.as_deref()));
    let mut pane = if tab == 0 {
        let mut blocks = Vec::new();
        if let Some((th, _)) = &got {
            if p.cfg.density == crate::config::Density::Compact {
                compact_thread(p, cx, th, &mut blocks);
            } else {
                cozy_thread(p, th, &mut blocks);
            }
            if !links.is_empty() {
                blocks.push(Block::line(Text::of(
                    format!(
                        "{} to open: links, files and what it names, on the Links tab",
                        links.len()
                    ),
                    Ink::Muted,
                )));
            }
            if th.replies.len() < th.reply_count() as usize
                && fetched.is_none_or(|f| f.value.is_none())
            {
                let remaining = th.reply_count() as usize - th.replies.len();
                blocks.push(Block::line(Text::of(
                    if fetched.is_some_and(|f| f.loading) {
                        format!("{remaining} more replies are being read…")
                    } else {
                        format!("{remaining} replies aren't loaded. {}", retry_words())
                    },
                    Ink::Muted,
                )));
            }
        } else {
            blocks.push(Block::line(Text::of(
                if fetched.is_some_and(|f| f.value.is_some()) {
                    "Slack returned no messages for this thread."
                } else {
                    "This thread isn't loaded yet."
                },
                Ink::Muted,
            )));
            if !fetched.is_some_and(|f| f.loading) {
                blocks.push(Block::line(Text::of(retry_words(), Ink::Secondary)));
            }
        }
        PaneView::new(
            PaneId::Middle,
            "thread",
            Node::Rich {
                blocks,
                pictures: Some("slack".into()),
            },
        )
    } else {
        let rows = links
            .iter()
            .enumerate()
            .map(|(i, subject)| {
                let mut name = Text::new();
                if subject.is(radar_core::compose::LINK) {
                    name.add(Seg::icon(crate::view::link_icon(&subject.label)));
                }
                name.add(Seg::new(subject.label.clone(), Ink::Emphasis));
                if let Some(title) = subject.field("title") {
                    name.add(Seg::new(format!("  {title}"), Ink::Secondary));
                }
                if let Some(url) = subject
                    .url
                    .as_ref()
                    .filter(|url| subject.is(radar_core::compose::LINK) && **url != subject.label)
                {
                    name.add(Seg::new(format!("  {url}"), Ink::Muted));
                }
                for chip in cx.parts.chips(subject) {
                    name.add(Seg::new(" ", Ink::Plain));
                    name.add(Seg::chip(&chip));
                }
                let right = if subject.is(radar_core::compose::LINK) {
                    "link"
                } else if cx.parts.can_show(&subject.kind) {
                    "opens in radar"
                } else {
                    "opens its link"
                };
                v::ListRow::new(format!("link:{i}"), name)
                    .right(Text::of(right, Ink::Muted))
                    .linked(subject.url.clone())
            })
            .collect();
        PaneView::new(
            PaneId::Middle,
            "what it links to",
            Node::List {
                rows,
                selected: (!links.is_empty()).then_some(row.min(links.len().saturating_sub(1))),
                editing: None,
            },
        )
    };
    pane.head = status;
    if fetched.is_some_and(|f| !f.loading && f.error.is_some()) {
        pane.notes.push(Text::of(retry_words(), Ink::Secondary));
    }
    if tab == 1 && links.is_empty() {
        let words = if fetched.is_some_and(|f| !f.loading && f.error.is_none() && f.value.is_some())
        {
            "No links in these messages. Your browser gets a breather."
        } else {
            "No links in these messages."
        };
        pane.notes.push(Text::of(words, Ink::Muted));
    }
    PageView {
        crumbs,
        title,
        details,
        under: None,
        tabs,
        tab,
        notice: None,
        panes: vec![pane],
        layout: None,
    }
}

fn thread_links(cx: &v::ViewCx<'_>, th: &Thread<'_>) -> Vec<radar_core::Subject> {
    let mut out = Vec::new();
    for text in th.links().into_iter().chain(
        th.texts()
            .map(|t| crate::mrkdwn::plain(t, &crate::mrkdwn::NoNames)),
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
    for (label, url) in th.openable() {
        let known = out
            .iter()
            .any(|s: &radar_core::Subject| s.url.as_deref() == Some(url.as_str()))
            || cx.parts.recognise(&url).iter().any(|(range, _)| {
                range.start == 0 && range.end == url.len()
            });
        if !known {
            let mut subject = radar_core::Subject::link(url);
            subject.label = label;
            out.push(subject);
        }
    }
    out
}

fn compact_thread(p: &SlackPlugin, cx: &v::ViewCx<'_>, th: &Thread<'_>, blocks: &mut Vec<Block>) {
    let count = th.reply_count();
    let messages: Vec<_> = std::iter::once(th.parent)
        .chain(th.replies.iter())
        .collect();
    let names: Vec<_> = messages.iter().map(|m| p.data.author(m)).collect();
    let name_width = names
        .iter()
        .map(|name| radar_core::ui::width(name))
        .max()
        .unwrap_or(0)
        .min(16);
    let gutter = if messages
        .iter()
        .any(|m| p.data.is_new(th.channel, &th.parent.ts, &m.ts))
    {
        2
    } else {
        0
    };
    let indent = gutter + 5 + 2 + name_width + 2;
    let width = cx.width.saturating_sub(4);
    let mut day = None;
    let mut previous: Option<(String, i64)> = None;
    for (i, message) in messages.iter().enumerate() {
        let seconds = crate::model::ts_secs(&message.ts);
        let current_day = seconds.div_euclid(86_400);
        let changed_day = day != Some(current_day);
        if changed_day || i == 1 && count > 0 {
            let mut label = if i == 1 && count > 0 {
                Some(crate::view::replies(count))
            } else {
                None
            };
            if changed_day {
                let day_name = crate::view::day(&message.ts, cx.now);
                label =
                    Some(label.map_or(day_name.clone(), |prefix| format!("{prefix} · {day_name}")));
                day = Some(current_day);
            }
            blocks.push(Block::line(Text::of(
                format!("── {} ──", label.unwrap_or_default()),
                Ink::Muted,
            )));
            previous = None;
        }
        let mark = if p.data.is_new(th.channel, &th.parent.ts, &message.ts) {
            "● "
        } else {
            ""
        };
        let same = previous.as_ref().is_some_and(|(who, then)| {
            who == &names[i] && (0..=crate::compact::WITHIN).contains(&(seconds - *then))
        });
        previous = Some((names[i].clone(), seconds));
        let name = if same {
            " ".repeat(name_width)
        } else {
            radar_core::ui::pad(&names[i], name_width)
        };
        let body = crate::view::first_line(&p.data, &p.cfg, message);
        let mut lines = radar_core::ui::wrap_words(&body, width, indent);
        if lines.is_empty() {
            lines.push(String::new());
        }
        let reactions = crate::view::reactions(message);
        let mut tail = Vec::new();
        if message.edited {
            tail.push("(edited)".to_string());
        }
        if !reactions.is_empty() {
            tail.push(reactions);
        }
        if !tail.is_empty() {
            let end = format!("  {}", tail.join("  "));
            if radar_core::ui::width(lines.last().expect("one line")) + radar_core::ui::width(&end)
                <= (width as usize).saturating_sub(indent)
            {
                lines.last_mut().expect("one line").push_str(&end);
            } else {
                lines.push(tail.join("  "));
            }
        }
        for (line, content) in lines.into_iter().enumerate() {
            if line == 0 {
                blocks.push(Block::line(Text::from(vec![
                    Seg::new(
                        if gutter == 0 {
                            ""
                        } else if mark.is_empty() {
                            "  "
                        } else {
                            mark
                        },
                        Ink::Accent,
                    ),
                    Seg::new(crate::view::clock(&message.ts), Ink::Muted),
                    Seg::new(format!("  {name}  "), Ink::Emphasis),
                    Seg::new(content, Ink::Primary),
                ])));
            } else {
                blocks.push(Block::line(Text::of(
                    format!("{}{content}", " ".repeat(indent)),
                    Ink::Primary,
                )));
            }
        }
        for shared in &message.shared {
            let quoted = crate::client::Message {
                text: shared.text.clone(),
                ..Default::default()
            };
            let lead = if shared.author.is_empty() {
                "".into()
            } else {
                format!("{}: ", shared.author)
            };
            blocks.push(Block::Wrapped {
                text: format!(
                    "{lead}{}",
                    crate::view::first_line(&p.data, &p.cfg, &quoted)
                ),
                prefix: Seg::new(format!("{}│ ", " ".repeat(indent)), Ink::Muted),
                ink: Ink::Secondary,
            });
        }
        for file in &message.files {
            blocks.push(Block::line(Text::from(vec![
                Seg::new(" ".repeat(indent), Ink::Plain),
                Seg::icon_or_lead(
                    if file.image {
                        Icon::FileImage
                    } else {
                        Icon::File
                    },
                    if file.image { "▣ " } else { "▤ " },
                    Ink::Secondary,
                ),
                Seg::new(file.name.clone(), Ink::Secondary).linked(file.url.clone()),
            ])));
        }
    }
}

fn cozy_thread(p: &SlackPlugin, th: &Thread<'_>, blocks: &mut Vec<Block>) {
    for (i, message) in std::iter::once(th.parent)
        .chain(th.replies.iter())
        .enumerate()
    {
        if i > 0 {
            blocks.push(Block::line(Text::new()));
        }
        let mut header = Text::from(vec![
            Seg::new(if i > 0 { "  " } else { "" }, Ink::Muted),
            Seg::icon(crate::view::author_icon(
                p.data.me.as_ref().map(|me| me.user_id.as_str()),
                message,
            )),
            Seg::new(p.data.author(message), Ink::Emphasis),
            Seg::new(
                format!("  {} ago", crate::view::age(&message.ts, p.now)),
                Ink::Muted,
            ),
        ]);
        if p.data.is_new(th.channel, &th.parent.ts, &message.ts) {
            header.add(Seg::icon_or_lead(Icon::Unread, " ●", Ink::Accent));
        }
        blocks.push(Block::line(header));
        blocks.push(Block::Doc {
            doc: crate::screen::message_doc(p, message),
            bar: (i > 0).then(|| Seg::new("  ", Ink::Muted)),
            most: None,
            urls: Some(false),
        });
        let reactions = crate::view::reactions(message);
        if !reactions.is_empty() {
            blocks.push(Block::line(Text::of(reactions, Ink::Secondary)));
        }
    }
}

fn draft_view(p: &SlackPlugin, cx: &v::ViewCx<'_>) -> PageView {
    let crumbs = vec![
        Seg::new("Slack", Ink::Secondary),
        Seg::new("ask for review", Ink::Secondary),
    ];
    let Some(draft) = &p.draft else {
        return PageView { crumbs, title: Text::of("ask for review", Ink::Emphasis), details: Text::new(), under: None, tabs: vec![], tab: 0, notice: None,
            panes: vec![PaneView::new(PaneId::Middle, "ask for review", Node::State { state: v::State::Empty, text: "No review ask being written. Choose \"ask for review…\" on a merge request.".into() })], layout: None };
    };
    if let Some(why) = p.ask_draft_blocked() {
        let mut lines = vec![
            Text::of(why, Ink::Warn),
            Text::of(
                "Press y to copy the retained text, or n to drop the ask.",
                Ink::Secondary,
            ),
            Text::new(),
        ];
        for item in &draft.items {
            lines.extend([
                Text::of(&item.label, Ink::Emphasis),
                Text::of(&item.url, Ink::Accent),
                Text::of(&item.words, Ink::Primary),
                Text::new(),
            ]);
        }
        return PageView {
            crumbs,
            title: Text::of("retained review ask", Ink::Emphasis),
            details: Text::new(),
            under: None,
            tabs: vec![],
            tab: 0,
            notice: None,
            panes: vec![PaneView::new(
                PaneId::Middle,
                "retained review ask",
                Node::Document { lines, cut: false },
            )],
            layout: None,
        };
    }
    let to = p.cfg.channel(&draft.channel).map_or_else(
        || "no channel yet".into(),
        |channel| format!("#{}", channel.alias),
    );
    let handles = crate::ask::mentioned(&p.cfg, &p.data.groups);
    let under = if draft.mentions_group(&p.cfg) && !handles.is_empty() {
        format!(
            "mentions {} · the review channel",
            handles
                .iter()
                .map(|handle| format!("@{handle}"))
                .collect::<Vec<_>>()
                .join(" and ")
        )
    } else if handles.is_empty() {
        "no user group in the template, or none set under review: no one is mentioned".into()
    } else {
        "not the review channel: the user group is not mentioned here".into()
    };
    let text = crate::ask::text(&p.cfg, &p.data.groups, &draft.channel, &draft.items);
    let mut blocks = vec![Block::Doc {
        doc: crate::view::doc(&p.data, &p.cfg, &text),
        bar: None,
        most: None,
        urls: Some(false),
    }];
    if let Some(why) = draft
        .blocked(&p.cfg, &p.data.groups)
        .filter(|_| draft.sending.is_none())
    {
        blocks.push(Block::line(Text::of(
            format!("cannot post yet: {why}"),
            Ink::Warn,
        )));
    }
    if draft.sending.is_some() {
        blocks.push(Block::line(Text::of("sending to Slack…", Ink::Info)));
    }
    if let Some(error) = &draft.error {
        blocks.push(Block::line(Text::from(vec![
            Seg::icon(Icon::Error),
            Seg::new(format!("Slack did not take it: {error}"), Ink::Bad),
        ])));
    }
    blocks.push(Block::line(Text::new()));
    blocks.push(Block::line(Text::of("THE TEXT SENT", Ink::Muted)));
    blocks.extend(
        text.lines()
            .map(|line| Block::line(Text::of(line, Ink::Muted))),
    );
    let preview = PaneView::new(
        PaneId::Middle,
        format!("as it will post in {to}"),
        Node::Rich {
            blocks,
            pictures: Some("slack".into()),
        },
    );
    let label_width = draft
        .items
        .iter()
        .map(|item| radar_core::ui::width(&item.label))
        .max()
        .unwrap_or(0)
        .min(30)
        + 2;
    let rows = draft
        .items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let words = if draft.edit.is_some() && i == draft.row {
                ""
            } else if item.words.is_empty() {
                "(no words)"
            } else {
                &item.words
            };
            v::ListRow::new(
                format!("item:{i}"),
                Text::from(vec![
                    Seg::new(format!("{:<label_width$}", item.label), Ink::Emphasis),
                    Seg::new(words, Ink::Primary),
                ]),
            )
        })
        .collect();
    let editing = draft.edit.as_ref().map(|input| v::Editing {
        row: draft.row,
        at: label_width as u16,
        text: input.value(),
        caret: input.cursor(),
    });
    let items = PaneView::new(
        PaneId::Detail,
        "merge requests in it",
        Node::List {
            rows,
            selected: Some(draft.row),
            editing,
        },
    );
    let preview_height = (text.lines().count() as u16 + 6)
        .min(cx.height.saturating_sub(10))
        .max(5);
    PageView {
        crumbs,
        title: Text::from(vec![
            Seg::icon(Icon::NeedsApproval),
            Seg::new(format!("Ask for review in {to}"), Ink::Emphasis),
        ]),
        details: Text::of("posted as you", Ink::Secondary),
        under: Some(Text::of(under, Ink::Muted)),
        tabs: vec![],
        tab: 0,
        notice: None,
        panes: vec![preview, items],
        layout: Some(PageLayout::split(
            Axis::Vertical,
            [
                (Size::Length(preview_height), PageLayout::pane(0)),
                (Size::Fill(1), PageLayout::pane(1)),
            ],
        )),
    }
}
