//! How Slack joins the other parts (radar-core's `compose`): the subjects it
//! owns (threads, and the review ask being written), recognising its
//! permalinks, showing them, and what it adds to the others' subjects, from
//! what it holds and never fetching:
//!
//! - a merge request: a "Discussed in Slack" section on its page, a chip on
//!   its row (`asked 3d · 2 replies`, or `discussed · 2`), and "ask for
//!   review…" (with the ask's thread once asked);
//! - a ticket: a Slack tab of the threads naming it, and the same chip.
//! - a pull request: existing exact-link discussions and navigation only.
//!
//! A thread is about a subject when it links to the subject's link (or
//! under it), or names its id as a word (a key). Its subjects, in the
//! vocabulary the parts share (GUIDE.md):
//! - `thread`: id `<channel>/<ts>`; fields `channel`, `ts`, optional `team_id`;
//! - `review-ask`: the ask being written; id `draft`.

mod permalink;
#[cfg(test)]
mod routing_tests;
#[cfg(test)]
mod pull_request_tests;

use std::ops::Range;

use radar_core::plugin::{Effect, Pane, Tone};
use radar_core::view::{ActionGroup, Ink, Seg, Text};
use radar_core::ui::icon::Icon;
use radar_core::{Action, Adds, Cell, Chip, Contribution, Owned, Point, Row, Showing, Subject};

use crate::model::{Thread, reask_due, ts_key, ts_secs};
use crate::page::{self, Page};
use crate::screen::{self, Item};
use crate::{SlackPlugin, ask, view};

pub(crate) const THREAD: &str = "thread";
pub(crate) const REVIEW_ASK: &str = "review-ask";
const MERGE_REQUEST: &str = "merge-request";
const PULL_REQUEST: &str = "pull-request";
const TICKET: &str = "ticket";
const REPO: &str = "repo";

pub(crate) const OWNS: [Owned; 2] = [
    Owned::page(THREAD, "threads"),
    Owned::page(REVIEW_ASK, "review asks"),
];

pub(crate) const ADDS: [Adds; 4] = [
    Adds::new(
        REPO,
        "a firing marker on its row and page, and an Alerts tab of what fires for it",
    ),
    Adds::new(
        MERGE_REQUEST,
        "a slack column (its review ask or threads; a click opens it), a Discussed in Slack section and tab, and ask for review",
    ),
    Adds::new(TICKET, "a Slack tab of the threads naming it, and a chip"),
    Adds::new(
        PULL_REQUEST,
        "a discussion column, section and tab, and open an existing discussion",
    ),
];

/// The key a thread's row goes by for motion: its subject's.
pub(crate) fn motion_key(channel: &str, ts: &str) -> String {
    radar_core::motion::key(THREAD, &format!("{channel}/{ts}"))
}

/// A thread, as a subject.
pub(crate) fn thread_subject(p: &SlackPlugin, channel: &str, ts: &str) -> Subject {
    let mut s = Subject::new(
        THREAD,
        format!("{channel}/{ts}"),
        format!("#{} thread", p.alias(channel)),
    )
    .with("channel", channel)
    .with("ts", ts);
    if let Some(me) = p.data.me.as_ref().filter(|me| !me.team_id.is_empty()) {
        s = s.with("team_id", &me.team_id);
    }
    if let Some(url) = p.permalink(channel, ts, None) {
        s = s.with_url(url);
    }
    s
}

/// The review ask being written, as a subject.
pub(crate) fn draft_subject(p: &SlackPlugin) -> Subject {
    let n = p.draft.as_ref().map_or(0, |d| d.items.len());
    Subject::new(REVIEW_ASK, "draft", format!("review ask ({n})"))
}

/// What the focused pane is about: a thread.
pub(crate) fn subject(p: &SlackPlugin, pane: Pane) -> Option<Subject> {
    match p.pages.last() {
        Some(Page::Thread { channel, ts, .. }) => return Some(thread_subject(p, channel, ts)),
        Some(Page::Draft) => return Some(draft_subject(p)),
        None => {}
    }
    if pane == Pane::Nav {
        return None;
    }
    match screen::selected(p)? {
        Item::Subject(_) | Item::Heading(_) => None,
        Item::Alert { channel, ts } => Some(thread_subject(p, &channel, &ts)),
        item => {
            let (channel, _, thread) = item.message(p)?;
            Some(thread_subject(p, &channel, &thread))
        }
    }
}

/// Slack permalinks in `text`: `https://<workspace>.slack.com/archives/<channel>/p<ts>`,
/// to the thread (a reply's link names its thread).
pub(crate) fn recognise(p: &SlackPlugin, text: &str) -> Vec<(Range<usize>, Subject)> {
    let Some(me) = &p.data.me else {
        return vec![];
    };
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = text[from..].find("/archives/") {
        let mid = from + at;
        let start = text[..mid]
            .char_indices()
            .rev()
            .find(|(_, c)| c.is_whitespace() || "<>|\"'(".contains(*c))
            .map_or(0, |(i, c)| i + c.len_utf8());
        let end = text[mid..]
            .find(|c: char| c.is_whitespace() || "<>|\"')".contains(c))
            .map_or(text.len(), |n| mid + n);
        // Examine the complete candidate once, including query and fragment.
        from = end;
        let original = &text[start..end];
        if let Some(link) = permalink::parse(&me.url, original) {
            out.push((
                start..end,
                thread_subject(p, &link.channel, &link.root).with_url(original),
            ));
        }
    }
    out
}

/// A direct local reference may omit URL/team; supplied context must agree.
pub(crate) fn thread_target<'a>(p: &SlackPlugin, s: &'a Subject) -> Option<(&'a str, &'a str)> {
    if !p.authenticated() {
        return None;
    }
    let (channel, ts) = (s.field("channel")?, s.field("ts")?);
    if !crate::config::is_channel_id(channel)
        || !permalink::timestamp(ts)
        || s.id != format!("{channel}/{ts}")
    {
        return None;
    }
    if let Some(team) = s.field("team_id")
        && (team.is_empty() || p.data.me.as_ref()?.team_id != team)
    {
        return None;
    }
    if let Some(url) = &s.url {
        let link = permalink::parse(&p.data.me.as_ref()?.url, url)?;
        if link.channel != channel || link.root != ts {
            return None;
        }
    }
    Some((channel, ts))
}

/// Another part asked to show one of ours: the thread's page, or the
/// review ask being written.
pub(crate) fn show(p: &mut SlackPlugin, s: &Subject) -> Showing {
    match s.kind.as_str() {
        THREAD => {
            let Some((channel, ts)) = thread_target(p, s) else {
                return Showing::Cannot;
            };
            let (channel, ts) = (channel.to_string(), ts.to_string());
            Showing::Page {
                screen: crate::SCREEN.id,
                effects: page::open_thread(p, &channel, &ts),
            }
        }
        REVIEW_ASK if p.draft.is_some() => {
            if p.pages.last() != Some(&Page::Draft) {
                p.pages.push(Page::Draft);
            }
            p.page_scroll.top();
            Showing::Page {
                screen: crate::SCREEN.id,
                effects: vec![Effect::Focus(Pane::Middle)],
            }
        }
        _ => Showing::Cannot,
    }
}

/// What a thread links to and says, worked out once per change of what is
/// held: `contribute` runs on every row another part draws.
#[derive(Debug)]
pub(crate) struct Indexed {
    channel: String,
    ts: String,
    links: Vec<String>,
    text: String,
}

/// The index of the watched channels' threads, the most recently active
/// first; built on first use after [`SlackPlugin::changed`].
pub(crate) fn index(p: &SlackPlugin) -> std::cell::Ref<'_, Vec<Indexed>> {
    if p.index.borrow().is_none() {
        let built = p
            .data
            .all_threads(&p.cfg)
            .iter()
            // A bot's alerts are not discussions.
            .filter(|t| {
                p.cfg
                    .by_id(t.channel)
                    .is_some_and(|c| c.kind != crate::config::Kind::Alerts)
            })
            .map(|t| Indexed {
                channel: t.channel.to_string(),
                ts: t.parent.ts.clone(),
                links: t.links(),
                text: t.texts().collect::<Vec<_>>().join("\n"),
            })
            .collect();
        *p.index.borrow_mut() = Some(built);
    }
    std::cell::Ref::map(p.index.borrow(), |i| i.as_ref().expect("built above"))
}

/// The threads in watched channels about `s`, the most recently active
/// first: linking to its link (or under it), or naming a ticket's key as a
/// word. Pull requests require an exact complete URL, without subtree or
/// bare-key matching.
pub(crate) fn threads_about<'a>(p: &'a SlackPlugin, s: &Subject) -> Vec<Thread<'a>> {
    let url = s.url.as_deref().unwrap_or("");
    let found: Vec<(String, String)> = index(p)
        .iter()
        .filter(|t| {
            if s.is(PULL_REQUEST) {
                !url.is_empty() && t.links.iter().any(|link| link == url)
            } else {
                crate::model::links_to(&t.links, url)
                    || (s.is(TICKET) && crate::model::has_word(&t.text, &s.id))
            }
        })
        .map(|t| (t.channel.clone(), t.ts.clone()))
        .collect();
    found
        .iter()
        .filter_map(|(c, ts)| {
            let c = p.cfg.by_id(c)?;
            p.data.thread(&c.id, ts)
        })
        .collect()
}

/// A review ask about a merge request: one radar posted, or a thread in a
/// watched channel that mentions the review group and links it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AskSeen {
    pub channel: String,
    pub ts: String,
    pub replies: u32,
    pub reactions: Vec<crate::client::Reaction>,
    pub test: bool,
}

pub(crate) fn ask_about(p: &SlackPlugin, s: &Subject) -> Option<AskSeen> {
    let posted = p.data.asked_for(&s.id).map(|a| AskSeen {
        channel: a.channel.clone(),
        ts: a.ts.clone(),
        replies: a.replies,
        reactions: a.reactions.clone(),
        test: a.test,
    });
    let group = &p.cfg.review.group;
    let seen = (!group.is_empty())
        .then(|| {
            threads_about(p, s)
                .into_iter()
                .filter(|t| {
                    crate::mrkdwn::mentions(&t.parent.text)
                        .groups
                        .contains(group)
                })
                .max_by_key(|t| ts_key(&t.parent.ts))
                .map(|t| AskSeen {
                    channel: t.channel.to_string(),
                    ts: t.parent.ts.clone(),
                    replies: t.reply_count(),
                    reactions: t.parent.reactions.clone(),
                    test: false,
                })
        })
        .flatten();
    match (posted, seen) {
        (Some(a), Some(b)) => Some(if ts_key(&b.ts) > ts_key(&a.ts) { b } else { a }),
        (a, b) => a.or(b),
    }
}

/// Whether an ask has waited long enough with no reply to ask again, as
/// set under review, counted in your Slack time zone.
pub(crate) fn due(p: &SlackPlugin, a: &AskSeen) -> bool {
    let offset = p.data.me.as_ref().and_then(|m| m.tz_offset).unwrap_or(0);
    a.replies == 0 && reask_due(p.cfg.review.reask(), ts_secs(&a.ts), p.now, offset)
}

/// The words of an ask again: the group mentioned (not in a test channel),
/// and a link to the ask it follows up.
pub(crate) fn reask_words(p: &SlackPlugin, a: &AskSeen) -> String {
    let groups: Vec<String> = if a.test {
        vec![]
    } else {
        let mut ids: Vec<String> = Vec::new();
        if !p.cfg.review.group.is_empty() {
            ids.push(p.cfg.review.group.clone());
        }
        for h in ask::placeholders(&p.cfg.review.template) {
            if let Some(g) = h
                .strip_prefix('@')
                .and_then(|h| ask::group_by_handle(&p.data.groups, h))
                && !ids.contains(&g.id)
            {
                ids.push(g.id.clone());
            }
        }
        ids.into_iter()
            .map(|id| format!("<!subteam^{id}> "))
            .collect()
    };
    let ask = match p.permalink(&a.channel, &a.ts, None) {
        Some(url) => format!("<{url}|this review ask>"),
        None => "this review ask".into(),
    };
    let lead = groups.concat();
    if lead.is_empty() {
        format!("Still looking for a review on {ask}, please.")
    } else {
        format!("{lead}still looking for a review on {ask}, please.")
    }
}

fn ask_chip(p: &SlackPlugin, a: &AskSeen) -> Chip {
    let when = view::age(&a.ts, p.now);
    let replies = match a.replies {
        0 => "no reply".to_string(),
        n => view::replies(n),
    };
    let test = if a.test { " (test)" } else { "" };
    Chip::new(
        format!("asked{test} {when} · {replies}"),
        if due(p, a) { Tone::Warn } else { Tone::Info },
    )
    .icon(Icon::NeedsApproval)
}

/// One line about a thread: where, who, when, how many replied.
/// How many threads a merge request's Slack section names; its tab has all.
const SECTION_THREADS: usize = 3;

/// `words` cut to `most` characters at a word, with `…` when cut.
fn clip(words: &str, most: usize) -> String {
    if words.chars().count() <= most {
        return words.to_string();
    }
    let cut: String = words.chars().take(most).collect();
    let cut = cut.rsplit_once(' ').map_or(cut.as_str(), |(head, _)| head);
    format!("{} …", cut.trim_end())
}

fn thread_line(p: &SlackPlugin, t: &Thread<'_>) -> String {
    format!(
        "#{} · {} · {} ago · {}",
        p.alias(t.channel),
        p.data.author(t.parent),
        view::age(t.last(), p.now),
        view::replies(t.reply_count())
    )
}

/// What fires for a repo, and what resolved or failed lately: the alerts
/// the rules in settings place on it, firing first. Held, never fetched.
fn alerts_of_repo(p: &SlackPlugin, s: &Subject) -> Vec<crate::alerts::Alert> {
    let hours = p.cfg.alerts.recent_hours;
    p.alerts()
        .alerts
        .iter()
        .filter(|a| a.recent(p.now, hours))
        .filter(|a| {
            a.place
                .repo
                .as_deref()
                .is_some_and(|r| crate::alerts::repo_is(r, &s.id))
        })
        .cloned()
        .collect()
}

/// A repo's firing marker and Alerts tab.
fn repo_alerts(p: &SlackPlugin, point: Point, s: &Subject) -> Option<Contribution> {
    use crate::alerts::State;
    let mine = alerts_of_repo(p, s);
    if mine.is_empty() {
        return None;
    }
    match point {
        Point::Chips => {
            let n = |state| mine.iter().filter(|a| a.state == state).count();
            let mut chips = Vec::new();
            if n(State::Firing) > 0 {
                chips.push(
                    Chip::new(format!("firing · {}", n(State::Firing)), Tone::Bad)
                        .icon(Icon::AlertFiring),
                );
            }
            if n(State::Failed) > 0 {
                chips.push(
                    Chip::new(format!("job failed · {}", n(State::Failed)), Tone::Bad)
                        .icon(Icon::RunFailed),
                );
            }
            (!chips.is_empty()).then_some(Contribution::Chips(chips))
        }
        Point::Tab => {
            let rows = mine
                .iter()
                .map(|a| {
                    let how = match a.lasted(p.now) {
                        Some(d) if a.state == State::Firing => {
                            format!("firing {}", crate::alert_view::lasted(d))
                        }
                        Some(d) => format!("resolved after {}", crate::alert_view::lasted(d)),
                        None => format!(
                            "{} {} ago",
                            a.state.word(),
                            crate::alert_view::lasted(p.now - a.changed())
                        ),
                    };
                    let mut parts = vec![a.name.clone()];
                    parts.extend(
                        [a.environment.clone(), a.priority.clone()]
                            .into_iter()
                            .filter(|x| !x.is_empty()),
                    );
                    parts.push(how);
                    parts.push(format!("#{}", p.alias(&a.channel)));
                    Row {
                        // The state is all the mark says: with icons off its symbol stays.
                        line: Text::new()
                            .push(Seg::icon_or_lead(a.state.icon(), format!("{} ", a.state.mark()), Ink::Plain))
                            .push(Seg::new(parts.join(" · "), Ink::Plain)),
                        subject: Some(thread_subject(p, &a.channel, &a.ts)),
                    }
                })
                .collect();
            Some(Contribution::Tab {
                title: "Alerts".into(),
                rows,
            })
        }
        _ => None,
    }
}

pub(crate) fn contribute(p: &SlackPlugin, point: Point, s: &Subject) -> Option<Contribution> {
    if s.is(REPO) {
        return repo_alerts(p, point, s);
    }
    if !(s.is(MERGE_REQUEST) || s.is(PULL_REQUEST) || s.is(TICKET)) {
        return None;
    }
    match point {
        // A merge request's row has a Slack column, which a click opens: its
        // review ask, else how many threads are about it (a click opens the
        // first).
        Point::Column if s.is(MERGE_REQUEST) || s.is(PULL_REQUEST) => {
            if s.is(MERGE_REQUEST)
                && let Some(a) = ask_about(p, s)
            {
                let chip = ask_chip(p, &a);
                let cell = Cell::new("slack", chip.text).tone(chip.tone);
                return Some(Contribution::Cell(
                    cell.show(thread_subject(p, &a.channel, &a.ts)),
                ));
            }
            let threads = threads_about(p, s);
            let first = threads.first()?;
            let cell =
                Cell::new("slack", format!("discussed · {}", threads.len())).tone(Tone::Muted);
            Some(Contribution::Cell(cell.show(thread_subject(
                p,
                first.channel,
                &first.parent.ts,
            ))))
        }
        Point::Chips if s.is(TICKET) => {
            let threads = threads_about(p, s);
            let first = threads.first()?;
            let mut chip =
                Chip::new(format!("discussed · {}", threads.len()), Tone::Muted).icon(Icon::Thread);
            // A click opens the first thread in Slack.
            if let Some(url) = p.permalink(first.channel, &first.parent.ts, None) {
                chip = chip.link(url);
            }
            Some(Contribution::Chips(vec![chip]))
        }
        Point::Section if s.is(MERGE_REQUEST) || s.is(PULL_REQUEST) => {
            let threads = threads_about(p, s);
            let ask = s.is(MERGE_REQUEST).then(|| ask_about(p, s)).flatten();
            if threads.is_empty() && ask.is_none() {
                return None;
            }
            let mut lines = Vec::new();
            // The channel's name, a link to the message in Slack: no
            // address spelled out under it.
            let place = |channel: &str, ts: &str| {
                let seg = Seg::new(format!("#{}", p.alias(channel)), Ink::AccentSecondary);
                match p.permalink(channel, ts, None) {
                    Some(url) => seg.linked(url),
                    None => seg,
                }
            };
            if let Some(a) = &ask {
                let mut line = Text::from(vec![
                    Seg::new("review asked in ", Ink::Muted),
                    place(&a.channel, &a.ts),
                    Seg::new(format!(" {} ago · ", view::age(&a.ts, p.now)), Ink::Muted),
                    if a.replies == 0 {
                        Seg::new("no reply yet", Ink::Warn)
                    } else {
                        Seg::new(view::replies(a.replies), Ink::Secondary)
                    },
                ]);
                for r in &a.reactions {
                    let glyph = radar_core::ui::rich::emoji(&r.name).unwrap_or_else(|| format!(":{}:", r.name));
                    line.add(Seg::new(format!(" · {glyph} {}", r.count), Ink::Muted));
                }
                if due(p, a) {
                    line.add(Seg::new(" · ask again from its x menu", Ink::Muted));
                }
                lines.push(line);
            }
            // The ask's own thread is said once, above.
            let others: Vec<&Thread<'_>> = threads
                .iter()
                .filter(|t| {
                    ask.as_ref()
                        .is_none_or(|a| !(a.channel == t.channel && a.ts == t.parent.ts))
                })
                .collect();
            // Each thread: where and when on one short line, then who said
            // what, cut to about two lines of a narrow pane.
            for t in others.iter().take(SECTION_THREADS) {
                let me = p.data.me.as_ref().map(|m| m.user_id.as_str());
                // The author in full, and the message words linking to the
                // message in Slack (#36).
                lines.push(Text::from(vec![
                    place(t.channel, &t.parent.ts),
                    Seg::new(format!(" · {}", view::who_full(&p.data, t.parent)), Ink::Secondary).strong(),
                    Seg::new(
                        format!(" · {} ago · {}", view::age(t.last(), p.now), view::replies(t.reply_count())),
                        Ink::Muted,
                    ),
                ]));
                let words = Seg::new(clip(&view::first_line(&p.data, &p.cfg, t.parent), 72), Ink::Primary);
                lines.push(Text::from(vec![
                    Seg::icon(view::author_icon(me, t.parent)),
                    match p.permalink(t.channel, &t.parent.ts, None) {
                        Some(url) => words.linked(url),
                        None => words,
                    },
                ]));
            }
            if others.len() > SECTION_THREADS {
                lines.push(Text::of(
                    format!("⋯ {} more threads in the Slack tab", others.len() - SECTION_THREADS),
                    Ink::Muted,
                ));
            }
            Some(Contribution::Section {
                icon: Some(Icon::Channel),
                title: "Discussed in Slack".into(),
                lines,
            })
        }
        Point::Tab => {
            let mut threads = threads_about(p, s);
            // A merge request's review ask first, even when its thread is
            // not in a watched channel's days.
            let mut rows = Vec::new();
            if s.is(MERGE_REQUEST)
                && let Some(a) = ask_about(p, s)
            {
                threads.retain(|t| !(t.channel == a.channel && t.parent.ts == a.ts));
                rows.push(Row {
                    line: Text::plain(format!(
                        "review asked in #{} {} ago · {}",
                        p.alias(&a.channel),
                        view::age(&a.ts, p.now),
                        if a.replies == 0 {
                            "no reply yet".into()
                        } else {
                            view::replies(a.replies)
                        },
                    )),
                    subject: Some(thread_subject(p, &a.channel, &a.ts)),
                });
            }
            if threads.is_empty() && rows.is_empty() {
                return None;
            }
            rows.extend(threads.iter().map(|t| Row {
                line: Text::plain(format!(
                    "{}  {}",
                    thread_line(p, t),
                    view::first_line(&p.data, &p.cfg, t.parent)
                )),
                subject: Some(thread_subject(p, t.channel, &t.parent.ts)),
            }));
            Some(Contribution::Tab {
                title: "Slack".into(),
                rows,
            })
        }
        Point::Actions if s.is(MERGE_REQUEST) => {
            let mut out = Vec::new();
            let asked = ask_about(p, s);
            // Asked already (and no ask being written): the thread is what
            // `s` opens; the fact about it is the row's hint.
            let thread_key = if asked.is_some() && p.draft.is_none() { 's' } else { 't' };
            if s.url.is_some() && (asked.is_none() || p.draft.is_some()) {
                out.push(
                    Action::new(
                        's',
                        match &p.draft {
                            Some(d) if d.items.iter().any(|i| i.id == s.id) => {
                                "show the review ask being written".to_string()
                            }
                            Some(d) => format!("add to the review ask ({} in it)…", d.items.len()),
                            None => "ask for review in Slack…".to_string(),
                        },
                        "ask",
                    )
                    .icon(Icon::NeedsApproval)
                    .group(ActionGroup::Ask),
                );
            }
            if let Some(a) = asked {
                let fact = if a.replies == 0 { "no reply yet".to_string() } else { view::replies(a.replies) };
                out.push(
                    Action::new(thread_key, "open its Slack review thread", "thread")
                        .icon(Icon::Thread)
                        .hint(format!("asked {} ago · {fact}", view::age(&a.ts, p.now)))
                        .group(ActionGroup::Ask),
                );
                if due(p, &a) {
                    out.push(
                        Action::new('n', "ask again in the thread…", "reask")
                            .icon(Icon::Reply)
                            .adds()
                            .group(ActionGroup::Ask),
                    );
                }
            }
            (!out.is_empty()).then_some(Contribution::Actions(out))
        }
        Point::Actions if s.is(PULL_REQUEST) => {
            let threads = threads_about(p, s);
            let first = threads.first()?;
            Some(Contribution::Actions(vec![
                Action::new(
                    't',
                    format!("open discussion thread in #{}", p.alias(first.channel)),
                    discussion_action(&thread_subject(p, first.channel, &first.parent.ts)),
                ).icon(Icon::Thread),
            ]))
        }
        _ => None,
    }
}

/// Preserve the exact discussion target and current workspace identity when
/// a contributed menu stays open across sync or account changes.
fn discussion_action(subject: &Subject) -> String {
    format!(
        "discussion\0{}\0{}\0{}",
        subject.id,
        subject.field("team_id").unwrap_or_default(),
        subject.url.as_deref().unwrap_or_default(),
    )
}

pub(crate) fn act(p: &mut SlackPlugin, s: &Subject, action: &str) -> Vec<Effect> {
    if s.is(PULL_REQUEST) {
        return threads_about(p, s)
            .into_iter()
            .map(|thread| thread_subject(p, thread.channel, &thread.parent.ts))
            .find(|subject| discussion_action(subject) == action)
            .map_or_else(Vec::new, |subject| vec![Effect::Show(subject)]);
    }
    match action {
        "ask" => match ask::Item::of(s) {
            Some(item) => page::ask_for(p, item),
            None => vec![Effect::Notify(format!("{}: no link to ask about", s.label))],
        },
        "thread" => match ask_about(p, s) {
            Some(a) => vec![Effect::Show(thread_subject(p, &a.channel, &a.ts))],
            None => vec![],
        },
        "reask" => match ask_about(p, s) {
            Some(a) => {
                let words = reask_words(p, &a);
                let before = p.replying.as_ref().map(|r| r.stamp);
                let mut fx = p.begin_reply(&a.channel, &a.ts, Some(&words));
                if p.replying.as_ref().map(|r| r.stamp) != before {
                    fx.insert(0, Effect::Show(thread_subject(p, &a.channel, &a.ts)));
                }
                fx
            }
            None => vec![],
        },
        _ => vec![],
    }
}
