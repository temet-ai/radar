//! The Slack screen: a nav pane (For you · Discussions · each watched channel
//! with what is new) │ a middle pane (a banded tree) of messages with their threads folded
//! under them │ a detail pane of the selection. Rows are drawn by the kit.


use radar_core::plugin::{Binding, Effect, MenuItem, Outcome, Pane, Tone};
use radar_core::view::{Chord, Key};
use radar_core::ui::icon::Icon;
use radar_core::view::TreeNav;
use radar_core::ui::{self, rich};
use radar_core::Subject;

use crate::client::Found;
use crate::model::{Thread, ts_key};
use crate::{Row, SlackPlugin, act, head, page};

mod detail;
mod neutral;
pub use neutral::{intent, view};
pub(crate) use detail::identity as detail_identity;

/// A row of the nav pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NavRow {
    ForYou,
    Discussions,
    /// What the alerts channels' readers made of their messages; present
    /// when a watched channel is for alerts.
    Alerts,
    /// A watched channel, by its place in the registry.
    Channel(usize),
}

pub(crate) fn nav_rows(p: &SlackPlugin) -> Vec<NavRow> {
    let mut rows = vec![NavRow::ForYou, NavRow::Discussions];
    if crate::alert_view::has_channel(p) {
        rows.push(NavRow::Alerts);
    }
    rows.extend(
        p.cfg
            .channels
            .iter()
            .enumerate()
            .filter(|(_, c)| c.watched)
            .map(|(i, _)| NavRow::Channel(i)),
    );
    rows
}

pub(crate) fn nav_row(p: &SlackPlugin) -> NavRow {
    nav_rows(p).get(p.nav_at).copied().unwrap_or(NavRow::ForYou)
}

/// What a row of the tree is.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Item {
    /// A group with nothing of its own to open (a day, "general").
    Heading(String),
    /// What a group of discussions is about.
    Subject(Subject),
    Thread {
        channel: String,
        ts: String,
    },
    Reply {
        channel: String,
        thread: String,
        ts: String,
    },
    /// A message found for you, by its place in `for_you`.
    Found(usize),
    /// An alert, by its channel and the ts of the message that opened it
    /// (its thread).
    Alert {
        channel: String,
        ts: String,
    },
}

impl Item {
    /// The channel and ts of the message it is, and its thread.
    pub(crate) fn message(&self, p: &SlackPlugin) -> Option<(String, String, String)> {
        match self {
            Item::Thread { channel, ts } => Some((channel.clone(), ts.clone(), ts.clone())),
            Item::Reply {
                channel,
                thread,
                ts,
            } => Some((channel.clone(), ts.clone(), thread.clone())),
            Item::Found(i) => {
                let f = p.data.for_you.get(*i)?;
                let thread = f
                    .message
                    .thread_ts
                    .clone()
                    .unwrap_or_else(|| f.message.ts.clone());
                Some((f.channel_id.clone(), f.message.ts.clone(), thread))
            }
            _ => None,
        }
    }
}

/// When a found message last saw activity: the newest reply in its thread,
/// when the thread is held, else the message itself.
pub(crate) fn found_activity<'a>(p: &'a SlackPlugin, f: &'a Found) -> &'a str {
    p.data
        .thread(&f.channel_id, found_thread(f))
        .map(|th| th.last())
        .filter(|l| ts_key(l) > ts_key(&f.message.ts))
        .unwrap_or(&f.message.ts)
}

/// The For you groups: what each holds, the most recently active first.
fn for_you_groups(p: &SlackPlugin) -> Vec<(&'static str, &'static str, Vec<usize>)> {
    let me = p
        .data
        .me
        .as_ref()
        .map(|m| m.user_id.clone())
        .unwrap_or_default();
    let mine: Vec<String> = p.data.my_groups().iter().map(|g| g.id.clone()).collect();
    let (mut you, mut groups, mut dms) = (Vec::new(), Vec::new(), Vec::new());
    let mut order: Vec<usize> = (0..p.data.for_you.len()).collect();
    order.sort_by_key(|i| std::cmp::Reverse(ts_key(found_activity(p, &p.data.for_you[*i]))));
    for i in order {
        let f = &p.data.for_you[i];
        let m = crate::mrkdwn::mentions(&f.message.text);
        if f.is_im || f.is_mpim {
            dms.push(i);
        } else if m.users.contains(&me) {
            you.push(i);
        } else if m.groups.iter().any(|g| mine.contains(g)) {
            groups.push(i);
        } else {
            you.push(i);
        }
    }
    vec![
        ("mentions", "mentions you", you),
        ("groups", "mentions your groups", groups),
        ("dms", "direct messages", dms),
    ]
}

/// The thread a found message is in (its own ts when it starts one).
pub(crate) fn found_thread(f: &Found) -> &str {
    f.message.thread_ts.as_deref().unwrap_or(&f.message.ts)
}

/// How many things are for you: mentions, messages and threads you are in
/// past their marks.
pub(crate) fn for_you_count(p: &SlackPlugin) -> usize {
    if p.cfg.token.trim().is_empty() {
        return 0;
    }
    let found = p
        .data
        .for_you
        .iter()
        .filter(|f| p.data.is_new(&f.channel_id, found_thread(f), &f.message.ts))
        .count();
    found + p.data.yours(&p.cfg).len()
}

// --- top bar and status line ---------------------------------------------------

pub fn top_right(p: &SlackPlugin) -> Option<String> {
    if p.cfg.token.trim().is_empty() {
        return None;
    }
    let words = ui::status::sync_words(
        "Slack",
        p.sync.reading > 0,
        p.sync.last_ok,
        p.now,
        Some(p.cfg.refresh_secs),
    );
    // Saved data drawn before Slack has said who the token is.
    Some(if p.previewing { format!("cached · {words}") } else { words })
}

pub fn status(p: &SlackPlugin) -> Vec<(String, Tone)> {
    let mut out = Vec::new();
    if p.cfg.token.trim().is_empty() {
        return out;
    }
    // A bot's alerts are not news to catch up on: the Alerts view has them.
    let new: usize = p
        .cfg
        .watched()
        .filter(|c| c.kind != crate::config::Kind::Alerts)
        .map(|c| p.data.new_count(&c.id))
        .sum();
    if new > 0 {
        out.push((format!("{new} new"), Tone::Info));
    }
    let you = for_you_count(p);
    if you > 0 {
        out.push((format!("{you} for you"), Tone::Warn));
    }
    let firing = crate::alert_view::firing(p);
    if firing > 0 {
        out.push((format!("{firing} firing"), Tone::Bad));
    }
    if !p.sync.errors.is_empty() {
        let n = p.sync.errors.len();
        out.push((
            format!("{n} read{} failed", if n == 1 { "" } else { "s" }),
            Tone::Bad,
        ));
    }
    out
}

// --- layout -----------------------------------------------------------------------

// --- nav pane --------------------------------------------------------------------------

// --- the tree -----------------------------------------------------------------------

/// Whether a thread has every word of the `/` filter, in its channel's name,
/// who wrote in it or any of its words.
fn thread_passes(p: &SlackPlugin, th: &Thread<'_>) -> bool {
    let words: Vec<String> = p.filter.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return true;
    }
    let mut hay = p.alias(th.channel);
    for m in std::iter::once(th.parent).chain(th.replies) {
        hay.push(' ');
        hay.push_str(&p.data.author(m));
    }
    for text in th.texts() {
        hay.push(' ');
        hay.push_str(&crate::mrkdwn::plain(
            text,
            &crate::model::Namer {
                data: &p.data,
                cfg: &p.cfg,
            },
        ));
    }
    let hay = hay.to_lowercase();
    words.iter().all(|w| hay.contains(w))
}

/// Whether a For you message has every word of the `/` filter.
fn found_passes(p: &SlackPlugin, f: &Found) -> bool {
    let words: Vec<String> = p.filter.split_whitespace().map(str::to_lowercase).collect();
    let hay = format!(
        "{} {} {}",
        f.channel_name,
        p.data.author(&f.message),
        crate::mrkdwn::plain(
            &f.message.text,
            &crate::model::Namer {
                data: &p.data,
                cfg: &p.cfg
            }
        )
    )
    .to_lowercase();
    words.iter().all(|w| hay.contains(w))
}

/// `/` words and `:` go-to on the lists; the filter input takes its keys.
pub(crate) fn filter_key(p: &mut SlackPlugin, key: Chord) -> Outcome {
    use radar_core::input::Edit;
    let Some(mut input) = p.filter_input.take() else {
        return Outcome::Ignored;
    };
    match input.key(key) {
        Edit::Cancel => p.filter.clear(),
        Edit::Submit(text) => p.filter = text.trim().to_string(),
        Edit::Typing => {
            p.filter = input.value();
            p.filter_input = Some(input);
        }
    }
    *p.tree.borrow_mut() = TreeNav::default();
    p.opened_once.borrow_mut().clear();
    Outcome::done()
}

/// What is selected in the middle pane, as of the last frame.
pub(crate) fn selected(p: &SlackPlugin) -> Option<Item> {
    let path = p.tree.borrow().selected().to_vec();
    p.shown.borrow().get(&path).cloned()
}

// --- the detail pane --------------------------------------------------------------------

/// A message's text, what was shared into it (quoted, with its link) and
/// its files and images, as one document.
pub(crate) fn message_doc(p: &SlackPlugin, m: &crate::client::Message) -> rich::Doc {
    let mut doc = crate::view::doc(&p.data, &p.cfg, &m.text);
    for s in &m.shared {
        let who = if s.author.is_empty() {
            "a message".to_string()
        } else {
            format!("{}'s message", s.author)
        };
        let mut blocks = crate::view::doc(&p.data, &p.cfg, &s.text).blocks;
        let mut head = vec![rich::Inline::text(format!("shared: {who}"))];
        match blocks.first_mut() {
            Some(rich::Block::Para { text }) => {
                head.push(rich::Inline::line_break());
                head.append(text);
                *text = head;
            }
            _ => blocks.insert(0, rich::Block::para(head)),
        }
        // Its link on the line under its words.
        if !s.url.is_empty() {
            let link = rich::Inline::link(s.url.clone(), s.url.clone());
            match blocks.last_mut() {
                Some(rich::Block::Para { text }) => text.extend([rich::Inline::line_break(), link]),
                _ => blocks.push(rich::Block::para(vec![link])),
            }
        }
        doc.blocks.push(rich::Block::Quote { blocks });
    }
    for f in &m.files {
        doc.blocks.push(rich::Block::Media(rich::Media {
            image: f.image,
            name: f.name.clone(),
            url: f.url.clone(),
            src: f.src.clone(),
            size: f.size,
        }));
    }
    doc
}

// --- keys and clicks ------------------------------------------------------------------

/// The link `o` and `y` act on for what is selected.
fn link(p: &SlackPlugin, item: &Item) -> Option<String> {
    match item {
        Item::Subject(s) => s.url.clone(),
        Item::Found(i) => {
            let f = p.data.for_you.get(*i)?;
            if f.permalink.is_empty() {
                p.permalink(&f.channel_id, &f.message.ts, f.message.thread_ts.as_deref())
            } else {
                Some(f.permalink.clone())
            }
        }
        _ => {
            let (channel, ts, thread) = item.message(p)?;
            p.permalink(&channel, &ts, Some(&thread))
        }
    }
}

fn pick_rail(p: &mut SlackPlugin, i: usize) {
    if i != p.nav_at {
        p.nav_at = i;
        // A new list: its groups open again on first sight.
        *p.tree.borrow_mut() = TreeNav::default();
        p.opened_once.borrow_mut().clear();
        p.detail_scroll.top();
    }
}

fn moved(p: &SlackPlugin, before: &[String]) {
    if p.tree.borrow().selected() != before {
        p.detail_scroll.top();
    }
}

/// `⏎` on a row: a subject shows, a message opens its thread's page.
fn enter(p: &mut SlackPlugin, item: Item) -> Outcome {
    match item {
        Item::Subject(s) => Outcome::Handled(vec![Effect::Show(s)]),
        Item::Heading(_) => {
            p.tree.borrow_mut().toggle_selected();
            Outcome::done()
        }
        Item::Alert { channel, ts } => Outcome::Handled(crate::alert_view::open(p, &channel, &ts)),
        other => match other.message(p) {
            Some((channel, ts, thread)) => {
                let fx = page::open_thread(p, &channel, &thread);
                // The message itself is read, wherever it sits in its thread.
                p.data.read_to(&channel, &thread, &ts);
                Outcome::Handled(fx)
            }
            None => Outcome::Ignored,
        },
    }
}

/// The keys every row of messages has: open, copy, caught up, reply, react.
pub(crate) fn message_key(p: &mut SlackPlugin, item: &Item, key: Chord) -> Option<Outcome> {
    match key.key {
        Key::Char('o') => {
            let Some(web) = link(p, item) else {
                return Some(Outcome::done());
            };
            Some(Outcome::Handled(match item.message(p) {
                // A message: in the Slack app when it runs here.
                Some((channel, ts, thread)) if !matches!(item, Item::Subject(_)) => {
                    p.open_in_slack(&channel, &ts, Some(&thread), web)
                }
                _ => vec![Effect::Open(web)],
            }))
        }
        Key::Char('y') => Some(Outcome::Handled(match link(p, item) {
            Some(url) => vec![Effect::Copy(url), Effect::Notify("link copied".into())],
            None => vec![],
        })),
        Key::Char('m') => {
            let (channel, ts, _) = item.message(p)?;
            if p.cfg.by_id(&channel).is_none() {
                return Some(Outcome::Handled(vec![Effect::Notify(
                    "not a channel radar reads: nothing to mark read".into(),
                )]));
            }
            Some(Outcome::Handled(p.catch_up(&channel, &ts)))
        }
        Key::Char('c') => {
            let (channel, _, thread) = item.message(p)?;
            Some(Outcome::Handled(page::start_reply(p, &channel, &thread)))
        }
        Key::Char('e') => {
            let (channel, ts, _) = item.message(p)?;
            Some(Outcome::Handled(page::react_menu(p, &channel, &ts)))
        }
        _ => None,
    }
}

pub fn key(p: &mut SlackPlugin, pane: Pane, key: Chord) -> Outcome {
    match key.key {
        Key::Esc if pane == Pane::Middle && !p.filter.is_empty() => {
            p.filter.clear();
            *p.tree.borrow_mut() = TreeNav::default();
            p.opened_once.borrow_mut().clear();
            p.detail_scroll.top();
            return Outcome::done();
        }
        Key::Char('/') if key.bare() => {
            p.filter_input = Some(radar_core::input::TextInput::new(&p.filter));
            return Outcome::done();
        }
        Key::Char(':') if key.bare() => {
            return Outcome::Handled(vec![Effect::Notify(
                "go to a channel or thread: not on Slack yet; pick it in the nav pane, or / filters by words".into(),
            )]);
        }
        _ => {}
    }
    // Not handled when there is nothing more to see: the host says so.
    if matches!(key.key, Key::PageDown | Key::PageUp) {
        return if p.detail_scroll.page(key.key == Key::PageDown) {
            Outcome::done()
        } else {
            Outcome::Ignored
        };
    }
    if pane != Pane::Nav
        && let Some(item) = selected(p)
    {
        if let Item::Alert { channel, ts } = &item {
            if let Some(out) = crate::alert_view::key(p, channel, ts, key) {
                return out;
            }
        } else if let Some(out) = message_key(p, &item, key) {
            return out;
        }
    }
    match pane {
        Pane::Nav => {
            if let Some(down) = key.vertical() {
                // ↑ on the first row is the host's: up to the top bar.
                if !down && p.nav_at == 0 {
                    return Outcome::Ignored;
                }
                let n = nav_rows(p).len();
                let to = ui::rows::step(p.nav_at, n, down);
                pick_rail(p, to);
                return Outcome::done();
            }
            Outcome::Ignored
        }
        Pane::Detail => {
            if key.key == Key::Enter
                && let Some(item) = selected(p)
            {
                return enter(p, item);
            }
            if p.detail_scroll.key(&key) {
                return Outcome::done();
            }
            Outcome::Ignored
        }
        Pane::Middle => {
            if key.key == Key::Enter {
                return match selected(p) {
                    Some(item) => enter(p, item),
                    None => Outcome::Ignored,
                };
            }
            let before = p.tree.borrow().selected().to_vec();
            if !radar_core::view::tree_key(key) {
                return Outcome::Ignored;
            }
            let tree = radar_core::view::Tree {
                rows: p.rows_shown.borrow().clone(),
                open: p.tree.borrow().open.clone(),
                selected: before,
                ..Default::default()
            };
            if let Some(step) = radar_core::view::tree_step(&tree, key) {
                return neutral::intent(p, Pane::Middle, &step);
            }
            Outcome::done()
        }
    }
}

/// Bulk commands (`b`): done to everything the view shows.
pub fn bulk(p: &SlackPlugin) -> Vec<MenuItem> {
    if p.cfg.token.trim().is_empty() {
        return vec![];
    }
    let mut out = vec![
        MenuItem::new('e', "open everything", "every thread's replies", "s-open"),
        MenuItem::new('c', "close everything", "down to the groups", "s-close"),
    ];
    if let NavRow::Channel(i) = nav_row(p) {
        let c = &p.cfg.channels[i];
        if p.data.new_count(&c.id) > 0 {
            out.push(MenuItem::new(
                'm',
                format!("mark #{} caught up", c.alias),
                "to its newest message; local only",
                "s-mark",
            ));
        }
    }
    let behind = p
        .cfg
        .watched()
        .filter(|c| c.kind != crate::config::Kind::Alerts && p.data.new_count(&c.id) > 0)
        .count();
    if behind > 0 {
        out.push(MenuItem::new(
            'a',
            "mark every channel caught up",
            format!("{behind} with something new; local only"),
            "s-mark-all",
        ));
    }
    let linked = linked(p);
    if !linked.is_empty() {
        out.push(MenuItem::new(
            'o',
            "open every linked thing",
            format!("{} in this view, in your browser", linked.len()),
            "s-browse",
        ));
        out.push(MenuItem::new(
            'y',
            "copy every link",
            format!("{} in this view, one a line", linked.len()),
            "s-copy",
        ));
    }
    out.push(MenuItem::new(
        'r',
        "refresh everything",
        "every watched channel and For you",
        "s-read",
    ));
    out
}

/// The links of the subjects this view groups by, from the last frame.
fn linked(p: &SlackPlugin) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in p.shown.borrow().values() {
        if let Item::Subject(s) = item
            && let Some(u) = &s.url
            && !out.contains(u)
        {
            out.push(u.clone());
        }
    }
    out
}

pub fn on_bulk(p: &mut SlackPlugin, value: &str) -> Option<Vec<Effect>> {
    Some(match value {
        "s-open" => {
            let paths: Vec<Vec<String>> = p.shown.borrow().keys().cloned().collect();
            let mut state = p.tree.borrow_mut();
            for path in paths {
                state.open(path);
            }
            vec![]
        }
        "s-close" => {
            let groups: Vec<Vec<String>> = p
                .shown
                .borrow()
                .keys()
                .filter(|k| k.len() == 1)
                .cloned()
                .collect();
            let mut state = p.tree.borrow_mut();
            state.close_all();
            for g in groups {
                state.open(g);
            }
            vec![]
        }
        "s-mark" => {
            let NavRow::Channel(i) = nav_row(p) else {
                return Some(vec![]);
            };
            let id = p.cfg.channels[i].id.clone();
            match p.newest(&id) {
                Some(ts) => p.catch_up(&id, &ts),
                None => vec![],
            }
        }
        "s-mark-all" => {
            let ids: Vec<String> = p
                .cfg
                .watched()
                .filter(|c| c.kind != crate::config::Kind::Alerts)
                .map(|c| c.id.clone())
                .collect();
            let mut n = 0;
            for id in ids {
                if p.data.new_count(&id) > 0
                    && let Some(ts) = p.newest(&id)
                {
                    p.data.marks.insert(id, ts);
                    n += 1;
                }
            }
            vec![
                Effect::PersistCache,
                Effect::Notify(format!(
                    "{n} channel{} caught up",
                    if n == 1 { "" } else { "s" }
                )),
            ]
        }
        "s-browse" => linked(p).into_iter().map(Effect::Open).collect(),
        "s-copy" => {
            let links = linked(p);
            vec![
                Effect::Copy(links.join("\n")),
                Effect::Notify(format!("{} links copied", links.len())),
            ]
        }
        "s-read" => p.fetch_now(),
        _ => return None,
    })
}

/// A message's title in the detail pane and the menu: whose it is, `Bob's message`.
fn message_title(p: &SlackPlugin, item: &Item) -> Option<String> {
    let who = detail::resolve(p, item).map(|selected| p.data.author(selected.message))
        .unwrap_or_else(|| "message".into());
    item.message(p)?;
    Some(format!("{who}'s message"))
}

/// The `x` rows a message has, wherever it is shown.
pub(crate) fn message_keys(p: &SlackPlugin, item: &Item, b: &mut Vec<Row>) {
    if link(p, item).is_some() {
        b.push(act("slack.feed.open_in_slack", Binding::new("o", "open it in Slack")));
        b.push(act("slack.feed.copy_message_link", Binding::new("y", "copy its link")));
    }
    if let Some((channel, _, _)) = item.message(p) {
        if p.cfg.by_id(&channel).is_some() {
            b.push(act("slack.feed.mark_read", Binding::new("m", "mark read up to here, in radar only")));
        }
        b.push(head(Binding::heading("on Slack")));
        b.push(act("slack.feed.reply", Binding::new("c", "comment in its thread…").adds()));
        b.push(act("slack.feed.react", Binding::new("e", "react to it…").adds()));
    }
}

/// The actions menu (`x`) for a pane, named after what is selected.
pub fn rows(p: &SlackPlugin, pane: Pane) -> Vec<Row> {
    let sel = selected(p);
    // What the menu is about, and its icon (a message's).
    let mut heading_icon = None;
    let heading = match &sel {
        Some(Item::Subject(s)) => s.label.clone(),
        Some(Item::Heading(h)) => h.clone(),
        Some(Item::Alert { channel, ts }) => {
            crate::alert_view::find(p, channel, ts).map_or_else(|| "alert".into(), |a| a.name)
        }
        Some(item) => match message_title(p, item) {
            Some(title) => {
                heading_icon = Some(Icon::Chat);
                title
            }
            None => "message".into(),
        },
        None => "slack".into(),
    };
    match pane {
        Pane::Nav => {
            let (icon, name) = match nav_row(p) {
                NavRow::ForYou => (Some(Icon::Mention), "For you".to_string()),
                NavRow::Discussions => (Some(Icon::Thread), "Discussions".to_string()),
                NavRow::Alerts => (Some(Icon::Alert), "Alerts".to_string()),
                NavRow::Channel(i) => {
                    crate::view::channel_heading(&p.cfg.channels[i].id, &p.cfg.channels[i].alias)
                }
            };
            vec![
                head(Binding::heading(name).icon(icon)),
                act("slack.feed.go_to_messages", Binding::new("⏎", "go to its messages")),
            ]
        }
        Pane::Middle | Pane::Detail => {
            let mut b = vec![head(Binding::heading(heading).icon(heading_icon))];
            match &sel {
                Some(Item::Alert { channel, ts }) => {
                    b = crate::alert_view::rows(p, channel, ts);
                }
                Some(Item::Subject(s)) => {
                    b.push(act("slack.feed.open_subject", Binding::new(
                        "⏎",
                        format!(
                            "open {} {}",
                            s.label,
                            if pane == Pane::Middle {
                                "in radar, or its link"
                            } else {
                                ""
                            }
                        )
                        .trim_end()
                        .to_string(),
                    )));
                    if s.url.is_some() {
                        b.push(act("slack.feed.open_subject_link", Binding::new("o", "open its link")));
                        b.push(act("slack.feed.copy_subject_link", Binding::new("y", "copy its link")));
                    }
                }
                Some(Item::Heading(_)) if pane == Pane::Middle => {
                    let state = p.tree.borrow();
                    let open = state.is_open(state.selected());
                    b.push(act("slack.feed.toggle_group", Binding::new(
                        "⏎",
                        if open {
                            "hide what is under it"
                        } else {
                            "show what is under it"
                        },
                    )));
                }
                Some(item @ (Item::Thread { .. } | Item::Reply { .. } | Item::Found(_))) => {
                    b.push(act("slack.feed.open_thread", Binding::new("⏎", "open its thread")));
                    if pane == Pane::Middle && matches!(item, Item::Thread { .. }) {
                        let state = p.tree.borrow();
                        if let Some(Item::Thread { channel, ts }) = &sel
                            && p.data
                                .thread(channel, ts)
                                .is_some_and(|th| !th.replies.is_empty())
                        {
                            let open = state.is_open(state.selected());
                            b.push(act("slack.feed.toggle_replies", Binding::new(
                                "space",
                                if open {
                                    "hide its replies"
                                } else {
                                    "show its replies"
                                },
                            )));
                        }
                    }
                    message_keys(p, item, &mut b);
                }
                _ => {}
            }
            if pane == Pane::Detail {
                b.push(head(Binding::heading("detail pane")));
                if p.detail_scroll.at() < p.detail_scroll.max() {
                    b.push(act("slack.feed.scroll_preview_down", Binding::new("pgdn", "scroll down a page")));
                }
                if p.detail_scroll.at() > 0 {
                    b.push(act("slack.feed.scroll_preview_up", Binding::new("pgup", "scroll up a page")));
                }
                b.push(act("slack.feed.back_to_list", Binding::new("esc", "back to the middle pane")));
            }
            b
        }
    }
}
