//! Slack's pages: a thread (its messages, and what it links to) and a review
//! ask being drafted (exactly as it will post, and where). Both are drawn
//! with the kit's page frame; `esc` comes back. Also the reply bar, the
//! reaction menu and the confirms every write goes through.

use radar_core::input::{Edit, TextInput};
use radar_core::plugin::{Binding, Effect, Msg, Outcome, Pane};
use radar_core::view::{ActionGroup, Chord, Key};
use radar_core::ui::icon::Icon;
use radar_core::{Subject, log, ui::rows::step};

#[cfg(test)]
use crate::Write;
use crate::ask::{self, Draft};
use crate::client::Message;
use crate::model::Thread;
use crate::{Fetched, Row, SlackMsg, SlackPlugin, act, head, thread_key};
use radar_core::ui::composer::Typed;
#[cfg(test)]
use radar_core::writes;

mod neutral;
pub(crate) use neutral::{intent, view};

/// A page on the Slack screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Page {
    Thread {
        channel: String,
        ts: String,
        tab: usize,
        /// The cursor on the Links tab.
        row: usize,
    },
    /// The review ask being drafted.
    Draft,
}

impl Page {
    /// A thread's root or the existing lifetime of a review draft. Draft
    /// revisions, selected items and retained/blocked state are not identity.
    pub(crate) fn identity(&self, draft: Option<&Draft>) -> Option<String> {
        match self {
            Self::Thread { channel, ts, .. } => {
                Some(serde_json::json!(["thread", channel, ts]).to_string())
            }
            Self::Draft => {
                let incarnation = draft?.stamp.incarnation;
                // Older unstamped drafts have no established lifetime yet.
                (incarnation != 0)
                    .then(|| serde_json::json!(["review-draft", incarnation]).to_string())
            }
        }
    }
}

#[cfg(test)]
mod page_identity_tests {
    use super::*;

    #[test]
    fn page_identity_uses_thread_root_not_tab_or_child() {
        let mut page = Page::Thread {
            channel: "CWATCH".into(), ts: "12.000001".into(), tab: 0, row: 0,
        };
        let identity = page.identity(None);
        assert_eq!(identity.as_deref(), Some(r#"["thread","CWATCH","12.000001"]"#));
        if let Page::Thread { tab, row, .. } = &mut page {
            *tab = 1;
            *row = 3;
        }
        assert_eq!(page.identity(None), identity);
        for other in [
            Page::Thread { channel: "CLANCRE".into(), ts: "12.000001".into(), tab: 1, row: 3 },
            Page::Thread { channel: "CWATCH".into(), ts: "13.000001".into(), tab: 1, row: 3 },
        ] {
            assert_ne!(other.identity(None), identity);
        }
    }

    #[test]
    fn page_identity_keeps_draft_incarnation_across_edits_and_retention() {
        let page = Page::Draft;
        let mut draft = Draft {
            stamp: crate::ask_context::DraftStamp { incarnation: 12, revision: 0 },
            ..Draft::default()
        };
        let identity = page.identity(Some(&draft));
        assert_eq!(identity.as_deref(), Some(r#"["review-draft",12]"#));
        draft.items.push(ask::Item {
            id: "ankh-morpork/watch!12".into(),
            url: "https://gitlab.example/ankh-morpork/watch/-/merge_requests/12".into(),
            label: "Watch patrol".into(), words: "Please review".into(), tickets: vec![],
        });
        draft.items[0].label = "Night watch patrol".into();
        draft.items[0].words = "Please review the patrol".into();
        draft.row = 2;
        draft.channel = "watch-review".into();
        draft.error = Some("retained after a connection change".into());
        draft.changed();
        assert_eq!(page.identity(Some(&draft)), identity);
        draft.stamp.incarnation = 13;
        assert_ne!(page.identity(Some(&draft)), identity);
    }

    #[test]
    fn page_identity_does_not_guess_for_missing_or_unstamped_drafts() {
        assert_eq!(Page::Draft.identity(None), None);
        assert_eq!(Page::Draft.identity(Some(&Draft::default())), None);
    }
}

const THREAD_TABS: [&str; 2] = ["Thread", "Links"];

/// Rendered links are only meaningful for this connection and thread.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LinkScope {
    read: crate::read_context::ReadStamp,
    channel: String,
    ts: String,
}

pub(crate) fn invalidate_links(p: &SlackPlugin) {
    p.page_links.borrow_mut().clear();
    p.page_links_scope.replace(None);
}

/// Every reader uses the same identity check, including keys before a redraw.
fn page_links(p: &SlackPlugin) -> std::cell::Ref<'_, [Subject]> {
    let current = match (p.pages.last(), p.page_links_scope.borrow().as_ref()) {
        (Some(Page::Thread { channel, ts, .. }), Some(scope)) => {
            scope.read == p.read_stamp() && scope.channel == *channel && scope.ts == *ts
        }
        _ => false,
    };
    std::cell::Ref::map(p.page_links.borrow(), |links| {
        if current { links.as_slice() } else { &[] }
    })
}

fn link_at(p: &SlackPlugin, row: usize) -> Option<Subject> {
    let links = page_links(p);
    links.get(row.min(links.len().saturating_sub(1))).cloned()
}

/// Only link actions need this lease. Other rows in the same menu (reply,
/// refresh, back) keep working when a background read changes the links.
pub(crate) fn menu_key_context(p: &SlackPlugin, action: &str) -> Option<String> {
    if !matches!(action, "⏎" | "enter" | "o" | "y") {
        return None;
    }
    let Page::Thread {
        channel,
        ts,
        tab: 1,
        row,
    } = p.pages.last()?
    else {
        return None;
    };
    let link = link_at(p, *row)?;
    // Subject's public equality deliberately compares only kind/id. These
    // keys also use URL and routing fields, which may change independently.
    fn target(s: &Subject) -> serde_json::Value {
        serde_json::json!([
            s.kind,
            s.id,
            s.url,
            s.fields,
            s.related.iter().map(target).collect::<Vec<_>>()
        ])
    }
    Some(serde_json::json!([p.read_stamp(), channel, ts, 1, target(&link)]).to_string())
}

/// Reactions offered: shortcut, exact provider identifier, and human action words.
pub(crate) const REACTIONS: [(char, &str, &str); 5] = [
    ('e', "eyes", "eyes on it"),
    ('d', "white_check_mark", "done"),
    ('p', "+1", "plus one"),
    ('t', "pray", "thanks"),
    ('r', "raised_hands", "raise hands"),
];

// --- opening and fetching -------------------------------------------------------------

/// Open a thread's page; fetch it when it is not held whole.
pub(crate) fn open_thread(p: &mut SlackPlugin, channel: &str, ts: &str) -> Vec<Effect> {
    if !p.authenticated() {
        return vec![Effect::Notify(
            "check the Slack connection before opening this thread".into(),
        )];
    }
    invalidate_links(p);
    p.pages.push(Page::Thread {
        channel: channel.into(),
        ts: ts.into(),
        tab: 0,
        row: 0,
    });
    let mut fx = vec![crate::sync_history::page_opened(
        p.data.me.as_ref().map_or("", |me| me.team_id.as_str()),
        channel,
        ts,
    )];
    p.page_scroll.top();
    // Opened is read: in radar only, kept in the cache, never sent to Slack.
    let newest = p
        .data
        .thread(channel, ts)
        .map_or_else(|| ts.to_string(), |th| th.last().to_string());
    p.data.read_to(channel, ts, &newest);
    let whole = p
        .data
        .thread(channel, ts)
        .is_some_and(|th| th.replies.len() as u32 >= th.parent.reply_count);
    fx.extend([Effect::Focus(Pane::Middle), Effect::PersistCache]);
    if !whole {
        fx.extend(fetch_thread(p, channel, ts));
    }
    fx
}

/// Fetch a thread (again).
pub(crate) fn fetch_thread(p: &mut SlackPlugin, channel: &str, ts: &str) -> Vec<Effect> {
    if !p.authenticated() {
        return vec![Effect::Notify(
            "check the Slack connection before opening this thread".into(),
        )];
    }
    let action = crate::sync_history::page_thread_action(
        p.data.me.as_ref().map_or("", |me| me.team_id.as_str()),
        channel,
        ts,
    );
    let Ok(c) = p.client() else {
        return vec![crate::sync_history::page_thread_blocked(&action, true)];
    };
    let f = p.threads.entry(thread_key(channel, ts)).or_default();
    if f.loading {
        return vec![
            crate::sync_history::page_thread_blocked(&action, false),
            Effect::Notify("already reading this thread".into()),
        ];
    }
    f.loading = true;
    f.error = None;
    let stamp = p.read_stamp();
    let (channel, ts) = (channel.to_string(), ts.to_string());
    let captured = action.clone();
    vec![Effect::SpawnAction {
        action,
        task: Box::pin(async move {
            let result = c
                .replies_observed(&channel, &ts, 500)
                .await
                .map_err(|e| format!("{e:#}"));
            if let Err(e) = &result {
                log::warn(format!("slack thread {channel} {ts}: {e}"));
            }
            Box::new(SlackMsg::Thread {
                stamp,
                channel,
                ts,
                action: captured,
                result,
            }) as Msg
        }),
    }]
}

pub(crate) fn on_thread(
    p: &mut SlackPlugin,
    channel: &str,
    ts: &str,
    result: Result<Vec<Message>, String>,
) {
    invalidate_links(p);
    let f = p.threads.entry(thread_key(channel, ts)).or_default();
    f.loading = false;
    match result {
        Ok(msgs) => {
            f.error = None;
            // Being read now: what came is read too.
            let open = matches!(p.pages.last(), Some(Page::Thread { channel: c, ts: t, .. }) if c == channel && t == ts);
            if open
                && let Some(last) = msgs
                    .iter()
                    .map(|m| m.ts.as_str())
                    .max_by_key(|t| crate::model::ts_key(t))
            {
                p.data.read_to(channel, ts, last);
            }
            let f = p.threads.entry(thread_key(channel, ts)).or_default();
            // A held thread takes its replies too.
            if let Some(c) = p.data.channels.get_mut(channel)
                && let Some(parent) = c.messages.iter_mut().find(|m| m.ts == ts)
            {
                if let Some(fresh) = msgs.iter().find(|m| m.ts == ts) {
                    *parent = fresh.clone();
                }
                c.replies.insert(
                    ts.to_string(),
                    msgs.iter().filter(|m| m.ts != ts).cloned().collect(),
                );
            }
            f.value = Some(msgs);
        }
        Err(e) => f.error = Some(e),
    }
}

pub(crate) fn loading(p: &SlackPlugin) -> bool {
    p.threads.values().any(|f| f.loading)
}

/// The thread a page shows: held, else fetched.
fn page_thread<'a>(
    p: &'a SlackPlugin,
    channel: &'a str,
    ts: &str,
) -> Option<(Thread<'a>, &'a Fetched<Vec<Message>>)> {
    static NONE: std::sync::OnceLock<Fetched<Vec<Message>>> = std::sync::OnceLock::new();
    let fetched = p
        .threads
        .get(&thread_key(channel, ts))
        .unwrap_or_else(|| NONE.get_or_init(Fetched::default));
    if let Some(th) = p.data.thread(channel, ts) {
        return Some((th, fetched));
    }
    let msgs = fetched.value.as_ref()?;
    let parent = msgs.iter().find(|m| m.ts == ts).or_else(|| msgs.first())?;
    let start = usize::from(msgs.first().is_some_and(|m| m.ts == parent.ts));
    Some((
        Thread {
            channel,
            parent,
            replies: &msgs[start..],
        },
        fetched,
    ))
}

// --- drawing ----------------------------------------------------------------------

fn retry_words() -> &'static str {
    "Press r to try again. Use , for connection settings or Esc to go back."
}

/// The review group's handle: as kept in settings, else as Slack gave it,
/// else its id.
pub(crate) fn review_handle(p: &SlackPlugin) -> String {
    use crate::mrkdwn::Names;
    crate::model::Namer {
        data: &p.data,
        cfg: &p.cfg,
    }
    .group(&p.cfg.review.group)
    .unwrap_or_else(|| p.cfg.review.group.clone())
}

// --- writing: replies, reactions, the review ask ---------------------------------------

/// Start writing a reply, offering any separately retained text first.
pub(crate) fn start_reply(p: &mut SlackPlugin, channel: &str, thread: &str) -> Vec<Effect> {
    p.start_reply(channel, thread)
}

/// Keys while a reply is written: the kit's composer's.
pub(crate) fn reply_key(p: &mut SlackPlugin, key: Chord) -> Outcome {
    let Some(r) = p.replying.as_mut() else {
        return Outcome::Ignored;
    };
    let before = r.composer.text();
    let typed = r.composer.key(key);
    if r.composer.text() != before {
        r.stamp.changed();
    }
    match typed {
        Typed::Review => Outcome::Handled(p.preview_reply()),
        Typed::Stop => {
            p.replying = None;
            Outcome::done()
        }
        Typed::AskStop => Outcome::Handled(p.preview_reply_drop()),
        Typed::Typing => Outcome::done(),
    }
}

/// `Bob's message`, or `this message` when its author is not held.
fn whose_message(p: &SlackPlugin, channel: &str, ts: &str) -> String {
    p.data
        .thread(channel, &p.thread_of(channel, ts))
        .and_then(|th| {
            std::iter::once(th.parent)
                .chain(th.replies)
                .find(|m| m.ts == ts)
                .map(|m| p.data.author(m))
        })
        .map_or_else(|| "this message".into(), |who| format!("{who}'s message"))
}

pub(crate) fn react_menu(p: &mut SlackPlugin, channel: &str, ts: &str) -> Vec<Effect> {
    p.pick_reaction(channel, ts, whose_message(p, channel, ts))
}

/// Answers to the menus the pages and the screen opened. `None` when the
/// menu is not one of theirs.
pub(crate) fn on_menu(p: &mut SlackPlugin, menu: &str, value: &str) -> Option<Vec<Effect>> {
    if let Some(fx) = p.answer_reply(menu, value) {
        return Some(fx);
    }
    if let Some(fx) = p.answer_ask(menu, value) {
        return Some(fx);
    }
    if let Some(fx) = p.answer_reaction(menu, value) {
        return Some(fx);
    }
    None
}

// --- keys ---------------------------------------------------------------------------

fn draft_key(p: &mut SlackPlugin, key: Chord) -> Outcome {
    let Some(d) = p.draft.as_mut() else {
        return Outcome::Ignored;
    };
    if let Some(input) = d.edit.as_mut() {
        let before = input.value();
        let edit = input.key(key);
        let mut changed = input.value() != before;
        match edit {
            Edit::Cancel => d.edit = None,
            Edit::Submit(text) => {
                let words = text.trim();
                if let Some(it) = d.items.get_mut(d.row)
                    && it.words != words
                {
                    it.words = words.to_string();
                    changed = true;
                }
                d.edit = None;
            }
            Edit::Typing => {}
        }
        if changed {
            d.changed();
        }
        return Outcome::done();
    }
    match key.key {
        Key::Char('j') | Key::Down => {
            d.row = step(d.row, d.items.len(), true);
            Outcome::done()
        }
        Key::Char('k') | Key::Up => {
            d.row = step(d.row, d.items.len(), false);
            Outcome::done()
        }
        Key::Char('e') | Key::Enter => {
            if let Some(it) = d.items.get(d.row) {
                d.edit = Some(TextInput::new(&it.words));
                d.changed();
            }
            Outcome::done()
        }
        Key::Char('d') if !d.items.is_empty() => {
            let gone = d.items.remove(d.row);
            d.row = d.row.min(d.items.len().saturating_sub(1));
            d.changed();
            Outcome::Handled(vec![Effect::Notify(format!(
                "{} taken out of the ask",
                gone.label
            ))])
        }
        Key::Char('c') => Outcome::Handled(p.preview_ask_channel()),
        Key::Char('p') => Outcome::Handled(p.preview_ask()),
        Key::Char('y') => Outcome::Handled(p.copy_ask_draft()),
        Key::Char('n') => Outcome::Handled(p.preview_ask_drop()),
        // Not handled when there is nothing more to see: the host says so.
        Key::PageDown | Key::PageUp => {
            if p.page_scroll.page(key.key == Key::PageDown) {
                Outcome::done()
            } else {
                Outcome::Ignored
            }
        }
        Key::Esc => {
            p.pages.pop();
            Outcome::done()
        }
        _ => Outcome::Ignored,
    }
}

/// A thread page's tabs, for the host to move across (GUIDE 1.5).
pub(crate) fn page_tabs(p: &SlackPlugin) -> Option<radar_core::plugin::PageTabs> {
    let Some(Page::Thread { tab, .. }) = p.pages.last() else {
        return None;
    };
    Some(radar_core::plugin::PageTabs {
        names: THREAD_TABS.iter().map(|name| (*name).to_string()).collect(),
        on: (*tab).min(THREAD_TABS.len() - 1),
    })
}

/// Show tab `i` from its top. The host asks on `← → h l`, the page on
/// `[` `]`.
pub(crate) fn show_tab(p: &mut SlackPlugin, i: usize) -> Vec<Effect> {
    let i = i.min(THREAD_TABS.len() - 1);
    if let Some(Page::Thread { tab, row, .. }) = p.pages.last_mut()
        && *tab != i
    {
        *tab = i;
        *row = 0;
        p.page_scroll.top();
    }
    vec![Effect::Focus(Pane::Middle)]
}

pub fn key(p: &mut SlackPlugin, key: Chord) -> Outcome {
    let Some(page) = p.pages.last().cloned() else {
        return Outcome::Ignored;
    };
    let Page::Thread {
        channel,
        ts,
        tab,
        row,
    } = page
    else {
        return draft_key(p, key);
    };
    let n_links = page_links(p).len();
    let row = row.min(n_links.saturating_sub(1));
    if let Some(Page::Thread { row: selected, .. }) = p.pages.last_mut() {
        *selected = row;
    }
    let set = |p: &mut SlackPlugin, tab: usize, row: usize| {
        if let Some(Page::Thread { tab: t, row: r, .. }) = p.pages.last_mut() {
            *t = tab;
            *r = row;
        }
    };
    match key.key {
        Key::Esc => {
            p.pages.pop();
            invalidate_links(p);
            p.page_scroll.top();
            return Outcome::done();
        }
        Key::Char(']') => return Outcome::Handled(show_tab(p, (tab + 1) % THREAD_TABS.len())),
        Key::Char('[') => {
            return Outcome::Handled(show_tab(
                p,
                (tab + THREAD_TABS.len() - 1) % THREAD_TABS.len(),
            ));
        }
        Key::Char('o') | Key::Char('y') if tab == 1 => {
            let s = link_at(p, row);
            let Some(url) = s.and_then(|s| s.url) else {
                return Outcome::Handled(vec![Effect::Notify("no link selected here".into())]);
            };
            return Outcome::Handled(if key.key == Key::Char('o') {
                vec![Effect::Open(url)]
            } else {
                vec![Effect::Copy(url), Effect::Notify("link copied".into())]
            });
        }
        Key::Char('o') => {
            return Outcome::Handled(match p.permalink(&channel, &ts, None) {
                Some(web) => p.open_in_slack(&channel, &ts, None, web),
                None => vec![],
            });
        }
        Key::Char('y') => {
            return Outcome::Handled(match p.permalink(&channel, &ts, None) {
                Some(url) => vec![Effect::Copy(url), Effect::Notify("link copied".into())],
                None => vec![],
            });
        }
        Key::Char('m') if p.cfg.by_id(&channel).is_some() => {
            let last = page_thread(p, &channel, &ts).map(|(th, _)| th.last().to_string());
            return Outcome::Handled(match last {
                Some(l) => p.catch_up(&channel, &l),
                None => vec![],
            });
        }
        Key::Char('c') => return Outcome::Handled(start_reply(p, &channel, &ts)),
        Key::Char('e') => return Outcome::Handled(react_menu(p, &channel, &ts)),
        _ => {}
    }
    if tab == 0 {
        if p.page_scroll.key(&key) {
            return Outcome::done();
        }
        return Outcome::Ignored;
    }
    match key.key {
        Key::Enter => match link_at(p, row) {
            Some(s) => Outcome::Handled(vec![Effect::Show(s)]),
            None => Outcome::Handled(vec![Effect::Notify("no link selected here".into())]),
        },
        _ => match key.vertical() {
            // ↑ at the top is not the list's: the host takes it up.
            Some(down) if down || row > 0 => {
                set(p, tab, step(row, n_links, down));
                Outcome::done()
            }
            _ => Outcome::Ignored,
        },
    }
}

/// What the next page pop actually reveals; the host supplies cross-part returns.
fn back_to(p: &SlackPlugin) -> String {
    match p.pages.iter().rev().nth(1) {
        Some(Page::Draft) => "the review ask".into(),
        Some(Page::Thread { channel, ts, .. }) => match page_thread(p, channel, ts) {
            Some((th, _)) => format!(
                "{}'s thread in #{}",
                p.data.author(th.parent),
                p.alias(channel)
            ),
            None => format!("the previous thread in #{}", p.alias(channel)),
        },
        None => match crate::screen::nav_row(p) {
            crate::screen::NavRow::ForYou => "For you".into(),
            crate::screen::NavRow::Discussions => "Discussions".into(),
            crate::screen::NavRow::Alerts => "Alerts".into(),
            crate::screen::NavRow::Channel(i) => format!("#{}", p.cfg.channels[i].alias),
        },
    }
}

#[cfg(test)]
#[path = "page_polish_tests.rs"]
mod polish_tests;

#[cfg(test)]
#[path = "page_menu_context_tests.rs"]
mod menu_context_tests;

/// The breadcrumb's last crumb: the item, an author and its first words.
fn crumb(who: &str, first: &str) -> String {
    let cut = first.chars().count() > 24;
    let head: String = first.chars().take(24).collect();
    format!("{who}: {}{}", head.trim_end(), if cut { "…" } else { "" })
}

/// The page's `x` menu.
pub fn rows(p: &SlackPlugin) -> Vec<Row> {
    match p.pages.last() {
        Some(Page::Thread {
            channel,
            ts,
            tab,
            row,
        }) => {
            let held = page_thread(p, channel, ts);
            // The thread's icon leads; the channel is named as it is written.
            let mut b = vec![head(
                Binding::heading(match &held {
                    Some((th, _)) => format!(
                        "{}'s thread in #{}",
                        p.data.author(th.parent),
                        p.alias(channel)
                    ),
                    None => format!("a thread in #{}", p.alias(channel)),
                })
                .icon(Icon::Thread),
            )];
            if *tab == 1 {
                if let Some(s) = link_at(p, *row) {
                    if s.is(radar_core::compose::LINK) {
                        b.push(head(Binding::heading("a link")));
                        b.push(act("slack.page.open_link_in_browser", Binding::new("⏎", "open it in the browser")));
                        b.push(act("slack.page.copy_link_url", Binding::new("y", "copy it")));
                    } else {
                        b.push(head(Binding::heading(s.label.clone())));
                        b.push(act("slack.page.open_subject", Binding::new("⏎", format!("open {}", s.label))));
                        if s.url.is_some() {
                            b.push(act("slack.page.open_subject_link", Binding::new("o", "open its link")));
                            b.push(act("slack.page.copy_subject_link", Binding::new("y", "copy its link")));
                        }
                    }
                }
                b.push(head(Binding::heading("the thread")));
            } else if p.workspace().is_some() {
                b.push(act("slack.page.open_in_slack", Binding::new("o", "open it in Slack")));
                b.push(act("slack.page.copy_thread_link", Binding::new("y", "copy its link")));
            }
            if p.cfg.by_id(channel).is_some() && held.is_some() {
                b.push(act("slack.page.mark_read", Binding::new(
                    "m",
                    "mark read up to its newest reply, in radar only",
                )));
            }
            b.push(act("slack.page.reply", Binding::new("c", "comment in its thread…").adds().group(ActionGroup::Change)));
            b.push(act("slack.page.react", Binding::new("e", "react to its first message…").adds().group(ActionGroup::Change)));
            b.push(act("slack.page.next_tab", Binding::new(
                "]",
                format!("next tab: {}", THREAD_TABS[(tab + 1) % THREAD_TABS.len()]),
            ).pane()));
            b.push(act("slack.page.previous_tab", Binding::new(
                "[",
                format!(
                    "previous tab: {}",
                    THREAD_TABS[(tab + THREAD_TABS.len() - 1) % THREAD_TABS.len()]
                ),
            ).pane()));
            if *tab == 0 && p.page_scroll.more() {
                b.push(act("slack.page.scroll_down", Binding::new("pgdn", "scroll down a page")));
            }
            b.push(act("slack.page.back", Binding::new("esc", format!("back to {}", back_to(p)))));
            b
        }
        Some(Page::Draft) => {
            let mut b = vec![head(Binding::heading("review ask").icon(Icon::NeedsApproval))];
            if let Some(d) = &p.draft {
                match p
                    .ask_draft_blocked()
                    .map(str::to_string)
                    .or_else(|| d.blocked(&p.cfg, &p.data.groups))
                {
                    None => b.push(act("slack.draft.post", Binding::new("p", format!("post it in #{}…", d.channel)).adds())),
                    Some(why) => b.push(act("slack.draft.post", Binding::new("p", format!("post it · {why}")))),
                }
                if let Some(it) = d.items.get(d.row) {
                    b.push(act("slack.draft.edit_words", Binding::new(
                        "e",
                        format!("edit the words for {}", it.label),
                    )));
                    b.push(act("slack.draft.take_out", Binding::new("d", format!("drop {}", it.label))));
                }
                b.push(act("slack.draft.choose_channel", Binding::new("c", "post it in another channel…")));
                b.push(act("slack.draft.copy_text", Binding::new("y", "copy the review ask text")));
                b.push(act("slack.draft.drop", Binding::new("n", "cancel the review ask…")));
            }
            b.push(act("slack.draft.back", Binding::new("esc", "back, keeping the draft")));
            b
        }
        None => vec![],
    }
}

/// The page's menu rows without their ids, for the tests that read keys.
#[cfg(test)]
pub fn keys(p: &SlackPlugin) -> Vec<Binding> {
    rows(p).into_iter().map(|(_, b)| b).collect()
}

/// Start a review ask with a merge request, or add it to the one being
/// written; then show the draft.
pub(crate) fn ask_for(p: &mut SlackPlugin, item: ask::Item) -> Vec<Effect> {
    if p.draft.is_some()
        && let Some(why) = p.ask_draft_blocked()
    {
        return vec![Effect::Notify(format!(
            "{why}; copy or drop the retained ask first"
        ))];
    }
    let label = item.label.clone();
    let note = match p.draft.as_mut() {
        Some(d) => {
            if d.add(item) {
                format!("{label} added to the review ask")
            } else {
                format!("{label} is in the review ask already")
            }
        }
        None => {
            let draft = Draft::new(&p.cfg, item);
            p.begin_ask_draft(draft);
            format!("review ask for {label}: nothing is sent until you post it")
        }
    };
    vec![
        Effect::Notify(note),
        Effect::Show(crate::join::draft_subject(p)),
    ]
}

#[cfg(test)]
mod ask_revision_tests {
    use super::*;
    use crate::keys::Mods;

    fn draft() -> SlackPlugin {
        let mut p = crate::demo::sample();
        p.cfg.review.channel = "team".into();
        let item = ask::Item::of(
            &Subject::new("merge-request", "mr-1", "api !1")
                .with_url("https://git.example/mr/1")
                .with("title", "first words"),
        )
        .unwrap();
        ask_for(&mut p, item);
        p
    }

    fn press(p: &mut SlackPlugin, code: Key) {
        draft_key(p, crate::keys::chord(code, Mods::NONE));
    }

    fn stamp(p: &SlackPlugin) -> crate::ask_context::DraftStamp {
        p.draft.as_ref().unwrap().stamp
    }

    fn choose_channel(p: &mut SlackPlugin, channel: &str) {
        let Outcome::Handled(fx) =
            draft_key(p, crate::keys::chord(Key::Char('c'), Mods::NONE))
        else {
            panic!("channel choice not handled")
        };
        let id = fx
            .into_iter()
            .find_map(|effect| match effect {
                Effect::Menu(menu) => Some(menu.id),
                _ => None,
            })
            .expect("channel choice menu");
        assert!(id.starts_with("ask-channel:"));
        let _ = on_menu(p, &id, channel).expect("channel choice handled");
    }

    #[test]
    fn retained_ask_is_copyable_from_page_and_settings_without_credentials() {
        let mut p = draft();
        p.pages.push(Page::Draft);
        p.replace_token(String::new());
        p.revoke_session();
        let before = p.draft.as_ref().unwrap().items.clone();
        let extra = before[0].clone();
        let refused = ask_for(&mut p, extra);
        assert!(matches!(refused.as_slice(), [Effect::Notify(_)]));
        assert_eq!(p.draft.as_ref().unwrap().items, before);
        assert!(keys(&p).iter().any(|binding| binding.keys == "y"));
        let copy = crate::keys::chord(Key::Char('y'), Mods::NONE);
        for result in [
            draft_key(&mut p, copy),
            crate::settings::key(&mut p, "connection", copy),
        ] {
            let Outcome::Handled(effects) = result else {
                panic!("copy is available")
            };
            assert!(effects.iter().any(|effect| matches!(effect, Effect::Copy(text) if text.contains("first words") && text.contains("https://git.example/mr/1"))));
            assert!(
                !effects
                    .iter()
                    .any(|effect| matches!(effect, Effect::Spawn(_) | Effect::SpawnAction { .. }))
            );
        }
        assert_eq!(p.draft.as_ref().unwrap().items, before);
    }

    #[test]
    fn editing_revises_the_draft_but_cursor_and_unchanged_input_do_not() {
        let mut p = draft();
        let initial = stamp(&p);
        press(&mut p, Key::Char('e'));
        let opened = stamp(&p);
        assert_ne!(opened, initial);

        press(&mut p, Key::Home);
        press(&mut p, Key::Backspace);
        assert_eq!(stamp(&p), opened);
        press(&mut p, Key::Char('!'));
        let typed = stamp(&p);
        assert_ne!(typed, opened);
        assert_eq!(p.draft.as_ref().unwrap().items[0].words, "first words");

        press(&mut p, Key::Enter);
        let committed = stamp(&p);
        assert_ne!(committed, typed);
        assert_eq!(p.draft.as_ref().unwrap().items[0].words, "!first words");

        press(&mut p, Key::Char('e'));
        let reopened = stamp(&p);
        assert_ne!(reopened, committed);
        press(&mut p, Key::Enter);
        assert_eq!(stamp(&p), reopened, "committing unchanged words is a no-op");
        press(&mut p, Key::Char('e'));
        let cancelling = stamp(&p);
        press(&mut p, Key::Esc);
        assert_eq!(
            stamp(&p),
            cancelling,
            "opening already invalidated the preview"
        );
        assert_eq!(p.draft.as_ref().unwrap().items[0].words, "!first words");
    }

    #[test]
    fn channel_changes_and_removal_revise_but_selection_does_not() {
        let mut p = draft();
        let initial = stamp(&p);
        press(&mut p, Key::Down);
        press(&mut p, Key::Up);
        radar_core::Plugin::intent(
            &mut p,
            crate::SCREEN.id,
            Pane::Middle,
            &radar_core::view::Intent::Select {
                target: radar_core::view::Target::new("item:0"),
            },
        );
        choose_channel(&mut p, "team");
        assert_eq!(stamp(&p), initial);

        choose_channel(&mut p, "try");
        let moved = stamp(&p);
        assert_ne!(moved, initial);
        assert_eq!(p.draft.as_ref().unwrap().channel, "try");

        press(&mut p, Key::Char('d'));
        let removed = stamp(&p);
        assert_ne!(removed, moved);
        assert!(p.draft.as_ref().unwrap().items.is_empty());
        press(&mut p, Key::Char('d'));
        assert_eq!(stamp(&p), removed);
    }
}

#[cfg(test)]
mod journal_tests {
    use super::*;
    use radar_core::journal::Outcome as JournalOutcome;
    use crate::keys::Mods;

    #[test]
    fn journal_thread_requires_its_known_workspace_and_discards_message_fields() {
        let p = crate::demo::sample();
        let subject = crate::join::thread_subject(&p, crate::demo::TEAM, "1700000000.123456")
            .with("text", "private message");
        let snapshot = radar_core::Plugin::journal_subject(&p, &subject).unwrap();
        assert_eq!(snapshot.key.instance, "T000ACME1");
        assert_eq!(
            snapshot.key.id,
            format!("{}/1700000000.123456", crate::demo::TEAM)
        );
        assert!(!format!("{snapshot:?}").contains("private"));
        let foreign = subject.with_url(format!(
            "https://other.slack.com/archives/{}/p1700000000123456",
            crate::demo::TEAM
        ));
        assert!(radar_core::Plugin::journal_subject(&p, &foreign).is_none());
        assert!(
            radar_core::Plugin::journal_subject(&p, &radar_core::Subject::new("ticket", "ABC-12", "x"))
                .is_none()
        );
    }

    #[test]
    fn reply_journal_has_workspace_message_and_outcome_but_no_message_text() {
        let mut p = crate::demo::sample();
        start_reply(&mut p, "C1", "1700000000.123456");
        p.replying
            .as_mut()
            .unwrap()
            .composer
            .paste("private reply and token");
        let fx = reply_key(
            &mut p,
            crate::keys::chord(Key::Char('s'), Mods::CONTROL),
        );
        let Outcome::Handled(fx) = fx else {
            panic!("review not handled")
        };
        let Effect::Journal(action) = &fx[0] else {
            panic!("missing journal intent")
        };
        assert_eq!(action.kind.as_str(), "slack.reply.post");
        assert_eq!(action.subjects[0].key.instance, "T000ACME1");
        assert_eq!(action.subjects[0].key.id, "C1/1700000000.123456");
        assert!(action.subjects.iter().any(|s| s.key.kind == "thread"
            && s.key.id == "C1/1700000000.123456"
            && s.role == radar_core::journal::SubjectRole::Context));
        let stored = format!("{action:?}");
        assert!(!stored.contains("private reply") && !stored.contains("token"));
        let menu = fx
            .iter()
            .find_map(|fx| match fx {
                Effect::Menu(m) => Some(m.id.clone()),
                _ => None,
            })
            .unwrap();
        assert!(
            on_menu(&mut p, &menu, "no")
                .unwrap()
                .iter()
                .any(|e| matches!(e, Effect::Report(JournalOutcome::Cancelled)))
        );
        let what = Write::Reply {
            operation: crate::reply_context::ReplyOperation {
                id: 0,
                draft: Default::default(),
                read: p.read_stamp(),
                payload: crate::reply_context::ReplyPayload {
                    channel: "C1".into(),
                    thread: "1700000000.123456".into(),
                    text: "private reply".into(),
                },
                original: "private reply".into(),
                team: "T000ACME1".into(),
                status: crate::reply_context::ReplyStatus::Unsent,
            },
        };
        let result = p.on_wrote(
            what.clone(),
            Ok(crate::client::Posted {
                channel: "C1".into(),
                ts: "1700000001.123456".into(),
            }),
            "T000ACME1",
        );
        assert!(matches!(
            result[0],
            Effect::Report(JournalOutcome::Succeeded)
        ));
        let Effect::Journal(action) = &result[1] else {
            panic!("missing posted message")
        };
        assert_eq!(
            action.subjects[0].role,
            radar_core::journal::SubjectRole::Result
        );
        assert_eq!(action.subjects[0].key.id, "C1/1700000001.123456");
        assert!(action.subjects.iter().any(|s| s.key.kind == "thread"
            && s.key.id == "C1/1700000000.123456"
            && s.role == radar_core::journal::SubjectRole::Context));
        assert!(matches!(
            p.on_wrote(what.clone(), Err("raw secret error".into()), "T000ACME1")[0],
            Effect::Report(JournalOutcome::Unknown)
        ));
        let old = writes::mode();
        writes::set_mode(writes::Mode::ReadOnly, None);
        let blocked = p.write(what);
        writes::set_mode(old.0, old.1);
        assert!(
            blocked
                .iter()
                .any(|e| matches!(e, Effect::Report(JournalOutcome::Blocked)))
        );
        assert!(!blocked.iter().any(|e| matches!(e, Effect::Spawn(_))));
    }

    #[test]
    fn reaction_journal_links_the_observed_thread_without_relabelling_its_message() {
        let mut p = crate::demo::sample();
        let (root, reply) = ("1700000000.123456", "1700000001.123456");
        p.data
            .channels
            .get_mut(crate::demo::TEAM)
            .unwrap()
            .replies
            .insert(
                root.into(),
                vec![crate::client::Message {
                    ts: reply.into(),
                    ..Default::default()
                }],
            );
        let Effect::Journal(action) =
            p.journal_write("slack.reaction.add", crate::demo::TEAM, Some(reply))
        else {
            panic!("missing reaction intent")
        };
        assert_eq!(action.subjects[0].key.kind, "message");
        assert_eq!(
            action.subjects[0].key.id,
            format!("{}/{reply}", crate::demo::TEAM)
        );
        let thread = action
            .subjects
            .iter()
            .find(|s| s.key.kind == "thread")
            .unwrap();
        assert_eq!(thread.key.instance, "T000ACME1");
        assert_eq!(thread.key.id, format!("{}/{root}", crate::demo::TEAM));
        assert_eq!(thread.role, radar_core::journal::SubjectRole::Context);
        let Effect::Journal(unknown) =
            p.journal_write("slack.reaction.add", crate::demo::TEAM, Some("999.000001"))
        else {
            panic!("missing reaction intent")
        };
        assert!(!unknown.subjects.iter().any(|s| s.key.kind == "thread"));
    }
}

#[cfg(test)]
mod mnemonic_tests {
    use super::*;

    #[test]
    fn draft_action_producer_does_not_depend_on_the_items_name() {
        let mut p = SlackPlugin {
                cfg: crate::demo::settings(),
                now: radar_core::plugin::DEMO_NOW,
                env: Some(vec![]),
                app_running: Some(false),
                ..Default::default()
            };
        let item = crate::ask::Item {
            id: "ankh-morpork/watch!44".into(),
            url: "https://gitlab.example/ankh-morpork/watch/-/merge_requests/44".into(),
            label: "zzz".into(), words: "Keep these exact words".into(), tickets: vec![],
        };
        p.draft = Some(crate::ask::Draft::new(&p.cfg, item));
        p.pages.push(Page::Draft);
        for (_, binding) in rows(&p) {
            if radar_core::mnemonic::letter(&binding.keys).is_some() {
                assert!(radar_core::mnemonic::at(&binding.keys, &binding.what).is_some(),
                    "{} / {}", binding.keys, binding.what);
            }
        }
        let bindings = rows(&p);
        let (_, drop) = bindings.iter().find(|(id, _)| id == "slack.draft.take_out").unwrap();
        assert_eq!((&*drop.keys, &*drop.what), ("d", "drop zzz"));
        let (_, cancel) = bindings.iter().find(|(id, _)| id == "slack.draft.drop").unwrap();
        assert_eq!((&*cancel.keys, &*cancel.what), ("n", "cancel the review ask…"));
        assert_eq!(p.draft.as_ref().unwrap().items[0].words, "Keep these exact words");
    }
}
