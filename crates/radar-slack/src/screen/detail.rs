//! Message previews share one shell; density changes only their body.

use radar_core::plugin::{DetailIdentity, Pane};
use radar_core::ui::icon::Icon;
use radar_core::view::detail::{
    self, DetailContent, DetailFact, DetailHeader, FootAction, FootActions,
};
use radar_core::view::{self as v, Block, Ink, PaneView, Seg, Text};

use super::Item;
use crate::SlackPlugin;
use crate::client::{Found, Message};
use crate::model::Thread;

/// The selected message is independent of the thread shown in compact density.
/// Search results can hold it even when the thread has never been cached.
pub(super) struct Selection<'a> {
    channel: &'a str,
    thread: &'a str,
    pub(super) message: &'a Message,
    found: Option<&'a Found>,
}

pub(super) fn resolve<'a>(p: &'a SlackPlugin, item: &'a Item) -> Option<Selection<'a>> {
    match item {
        Item::Found(i) => {
            let found = p.data.for_you.get(*i)?;
            Some(Selection {
                channel: &found.channel_id,
                thread: super::found_thread(found),
                message: &found.message,
                found: Some(found),
            })
        }
        Item::Thread { channel, ts } => Some(Selection {
            channel,
            thread: ts,
            message: p.data.thread(channel, ts)?.parent,
            found: None,
        }),
        Item::Reply {
            channel,
            thread,
            ts,
        } => Some(Selection {
            channel,
            thread,
            message: p
                .data
                .thread(channel, thread)?
                .replies
                .iter()
                .find(|m| m.ts == *ts)?,
            found: None,
        }),
        _ => None,
    }
}

/// Only held, exact subject/account/destination data participates; no client,
/// credential resolution, row index or newly generated incarnation is involved.
pub(crate) fn identity(p: &SlackPlugin) -> Option<DetailIdentity> {
    let item = super::selected(p)?;
    let selected = resolve(p, &item)?;
    let me = p.data.me.as_ref()?;
    if [
        selected.channel,
        selected.thread,
        &selected.message.ts,
        &me.team_id,
        &me.user_id,
    ]
    .iter()
    .any(|s| s.is_empty())
    {
        return None;
    }
    Some(DetailIdentity {
        connection: p.read_stamp().generation(),
        subject: vec![
            me.team_id.clone(),
            me.user_id.clone(),
            p.session_owner
                .as_ref()
                .map(|owner| owner.team_id.clone())
                .unwrap_or_default(),
            p.session_owner
                .as_ref()
                .map(|owner| owner.user_id.clone())
                .unwrap_or_default(),
            selected.channel.into(),
            selected.message.ts.clone(),
            selected.thread.into(),
            super::link(p, &item).unwrap_or_default(),
        ],
    })
}

fn conversation(p: &SlackPlugin, selected: &Selection<'_>) -> Text {
    let info = p
        .data
        .channels
        .get(selected.channel)
        .and_then(|channel| channel.info.as_ref());
    let direct = selected.channel.starts_with('D')
        || selected
            .found
            .is_some_and(|found| found.is_im || found.is_mpim)
        || info.is_some_and(|info| info.is_im);
    let name = p
        .cfg
        .by_id(selected.channel)
        .map(|channel| channel.alias.as_str())
        .filter(|name| !name.trim().is_empty())
        .or_else(|| {
            selected
                .found
                .map(|found| found.channel_name.as_str())
                .filter(|name| !name.trim().is_empty())
        })
        .or_else(|| {
            info.map(|info| info.name.as_str())
                .filter(|name| !name.trim().is_empty())
        })
        .unwrap_or(selected.channel);
    Text::from(vec![
        Seg::icon(if direct {
            Icon::Direct
        } else {
            crate::view::channel_icon(selected.channel)
        }),
        Seg::new(
            if direct {
                name.into()
            } else {
                format!("#{name}")
            },
            Ink::Secondary,
        ),
    ])
}

pub(super) fn pane(
    p: &SlackPlugin,
    cx: &v::ViewCx<'_>,
    title: String,
    selected: &Selection<'_>,
) -> PaneView {
    let message = selected.message;
    let thread = p.data.thread(selected.channel, selected.thread);
    let mut path = Text::new();
    if let Some(team) = p
        .data
        .me
        .as_ref()
        .map(|me| me.team.as_str())
        .filter(|team| !team.trim().is_empty())
    {
        path.add(Seg::new(format!("{team} / "), Ink::Secondary));
    }
    path.0.extend(conversation(p, selected).0);
    let mut header = DetailHeader::new(
        path,
        Text::of(
            crate::view::first_line(&p.data, &p.cfg, message),
            Ink::Emphasis,
        ),
    );
    if message.user.as_ref().is_some_and(|user| !user.is_empty())
        || message
            .bot_name
            .as_ref()
            .is_some_and(|name| !name.is_empty())
    {
        header.author = Some(Text::from(vec![
            Seg::icon(crate::view::author_icon(
                p.data.me.as_ref().map(|me| me.user_id.as_str()),
                message,
            )),
            Seg::new(p.data.author(message), Ink::Emphasis),
        ]));
    }
    if !message.ts.is_empty() {
        header.age = Some(Text::of(
            format!("{} ago", crate::view::age(&message.ts, cx.now)),
            Ink::Muted,
        ));
    }
    let mut facts = vec![DetailFact::new(
        "thread",
        Text::of(
            if message.ts == selected.thread {
                "thread start".into()
            } else {
                thread.as_ref().map_or_else(
                    || "reply".into(),
                    |th| format!("reply in {}'s thread", p.data.author(th.parent)),
                )
            },
            Ink::Secondary,
        ),
    )];
    // A cached thread owns its count, including zero. An uncached search reply
    // does not know its parent's total; its default zero must not invent one.
    let replies = thread.as_ref().map(Thread::reply_count).or_else(|| {
        (message.ts == selected.thread && message.reply_count > 0).then_some(message.reply_count)
    });
    if let Some(count) = replies {
        facts.push(DetailFact::new(
            "replies",
            Text::of(count.to_string(), Ink::Secondary),
        ));
    }
    if message.edited {
        facts.push(DetailFact::new("edited", Text::of("yes", Ink::Secondary)));
    }
    if !message.files.is_empty() {
        facts.push(DetailFact::new(
            "files",
            Text::of(message.files.len().to_string(), Ink::Secondary),
        ));
    }
    if !message.shared.is_empty() {
        facts.push(DetailFact::new(
            "shared messages",
            Text::of(message.shared.len().to_string(), Ink::Secondary),
        ));
    }
    let mut sections = if p.cfg.density == crate::config::Density::Compact {
        compact(p, cx, selected, thread.as_ref())
    } else {
        cozy(p, cx, message, thread.as_ref())
    };
    // Found results still own their links/files/shared-message links without a
    // cached parent; use the same model collector as the regular thread.
    let standalone = Thread {
        channel: selected.channel,
        parent: message,
        replies: &[],
    };
    links(
        thread.as_ref().unwrap_or(&standalone),
        &standalone,
        &mut sections,
    );
    let mut content = DetailContent::new(header, facts, sections);
    content.pictures = Some("slack".into());
    detail::pane(title, content, foot(p))
}

fn foot(p: &SlackPlugin) -> FootActions {
    FootActions::new(
        crate::actions(&super::rows(p, Pane::Detail))
            .into_iter()
            .filter_map(|action| {
                let caption = match action.id.as_str() {
                    "slack.feed.open_thread" => "thread",
                    "slack.feed.open_in_slack" => "open",
                    "slack.feed.copy_message_link" => "copy",
                    "slack.feed.mark_read" => "mark read",
                    "slack.feed.reply" => "comment",
                    "slack.feed.react" => "react",
                    _ => return None,
                };
                Some(FootAction::new(action).caption(caption))
            })
            .collect(),
    )
}

fn cozy(
    p: &SlackPlugin,
    cx: &v::ViewCx<'_>,
    message: &Message,
    thread: Option<&Thread<'_>>,
) -> Vec<Block> {
    let mut body = vec![Block::Doc {
        doc: super::message_doc(p, message),
        bar: None,
        most: None,
        urls: None,
    }];
    let reactions = crate::view::reactions(message);
    if !reactions.is_empty() {
        body.push(Block::line(Text::of(reactions, Ink::Secondary)));
    }
    let mut sections = vec![Block::section(Some(Icon::Chat), "message", None, body)];
    if let Some(th) = thread.filter(|th| th.reply_count() > 0) {
        let last = th
            .replies
            .iter()
            .rev()
            .take(3)
            .rev()
            .map(|reply| {
                Text::from(vec![
                    Seg::new(crate::view::who(&p.data, reply), Ink::Emphasis),
                    Seg::new(
                        format!(": {}", crate::view::first_line(&p.data, &p.cfg, reply)),
                        Ink::Primary,
                    ),
                ])
            })
            .collect();
        sections.push(Block::section_of(
            Some(Icon::Reply),
            "replies",
            Some(format!(
                "{} · last {} ago",
                crate::view::replies(th.reply_count()),
                crate::view::age(th.last(), cx.now),
            )),
            last,
        ));
    }
    sections
}

fn compact(
    p: &SlackPlugin,
    cx: &v::ViewCx<'_>,
    selected: &Selection<'_>,
    thread: Option<&Thread<'_>>,
) -> Vec<Block> {
    let mut messages: Vec<_> = thread.map_or_else(
        || vec![selected.message],
        |th| std::iter::once(th.parent).chain(th.replies).collect(),
    );
    // Search may be fresher than the cached thread. Always keep the selected
    // reply in its body, and prefer its current payload over an older copy.
    if let Some(message) = messages
        .iter_mut()
        .find(|message| message.ts == selected.message.ts)
    {
        *message = selected.message;
    } else {
        messages.push(selected.message);
        messages.sort_by_key(|message| crate::model::ts_key(&message.ts));
    }
    let mut body = Vec::new();
    for message in messages {
        let mut author = Text::new();
        if p.data
            .is_new(selected.channel, selected.thread, &message.ts)
        {
            author.add(Seg::icon_or_lead(Icon::Unread, "● ", Ink::Accent));
        }
        author.add(Seg::new(crate::view::clock(&message.ts), Ink::Muted));
        author.add(Seg::new(
            format!("  {}", p.data.author(message)),
            Ink::Emphasis,
        ));
        body.push(Block::line(author));
        body.push(Block::line(Text::of(
            crate::view::first_line(&p.data, &p.cfg, message),
            Ink::Primary,
        )));
        for shared in &message.shared {
            let by = if shared.author.is_empty() {
                "a message"
            } else {
                &shared.author
            };
            body.push(Block::Wrapped {
                text: format!(
                    "{by}: {}",
                    crate::mrkdwn::plain(
                        &shared.text,
                        &crate::model::Namer {
                            data: &p.data,
                            cfg: &p.cfg
                        }
                    )
                ),
                prefix: Seg::new("│ ", Ink::Muted),
                ink: Ink::Secondary,
            });
        }
        for file in &message.files {
            body.push(Block::line(Text::from(vec![
                Seg::icon(if file.image {
                    Icon::FileImage
                } else {
                    Icon::File
                }),
                Seg::new(file.name.clone(), Ink::Secondary).linked(file.url.clone()),
            ])));
        }
        let reactions = crate::view::reactions(message);
        if !reactions.is_empty() {
            body.push(Block::line(Text::of(reactions, Ink::Secondary)));
        }
    }
    if let Some(th) = thread.filter(|th| th.reply_count() > 0) {
        body.push(Block::line(Text::new()));
        body.push(Block::line(Text::of(
            format!(
                "{} · last {} ago",
                crate::view::replies(th.reply_count()),
                crate::view::age(th.last(), cx.now)
            ),
            Ink::Secondary,
        )));
    }
    vec![Block::section(
        Some(Icon::Thread),
        "thread",
        Some(crate::view::day(&selected.message.ts, cx.now)),
        body,
    )]
}

fn links(thread: &Thread<'_>, selected: &Thread<'_>, sections: &mut Vec<Block>) {
    let mut links = thread.openable();
    for (label, url) in selected.openable() {
        if !links.iter().any(|(_, existing)| existing == &url) {
            links.push((label, url));
        }
    }
    if links.is_empty() {
        return;
    }
    let mut lines = Vec::new();
    for (label, url) in links {
        lines.push(Text::from(vec![
            Seg::icon(crate::view::link_icon(&label)),
            Seg::new(label.clone(), Ink::Emphasis).linked(url.clone()),
        ]));
        if label != url {
            lines.push(Text::from(vec![
                Seg::new(url.clone(), Ink::Accent).linked(url),
            ]));
        }
    }
    sections.push(Block::section_of(Some(Icon::Link), "links", None, lines));
}

#[cfg(test)]
mod tests;
