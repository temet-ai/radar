//! What a Slack message tells the host so an agent can be briefed on it
//! (send to agent), and the merge requests it links (review locally).
//!
//! Everything here is read from what the plugin already holds; nothing is
//! fetched and nothing is posted.

use radar_core::agent_context::{
    AgentContext, AgentContextRequest, ContextField, REVIEW_LOCALLY_ACTION, ReviewRequest,
    SEND_TO_AGENT_ACTION,
};
use radar_core::journal::SubjectKey;
use radar_core::plugin::{Binding, Effect, Outcome, Pane};
use radar_core::view::{Chord, Key};
use radar_core::ui::icon::Icon;

use crate::model::{Namer, Thread};
use crate::page::Page;
use crate::screen::{self, Item};
use crate::{Row, SlackPlugin, act, head, mrkdwn};

/// The kind of a Slack message in a [`SubjectKey`] (the journal's, too).
pub(crate) const MESSAGE: &str = "message";
/// The word the brief heads the quoted material with.
pub(crate) const KIND: &str = "Slack message";
/// Replies a context carries, the newest ones; the rest are counted.
pub(crate) const MAX_REPLIES: usize = 10;
/// Merge request links a message may ask a review of.
pub(crate) const MAX_LINKS: usize = 5;

/// The scoped identity of a message: this workspace, `<channel>/<ts>`.
pub(crate) fn key(p: &SlackPlugin, channel: &str, ts: &str) -> Option<SubjectKey> {
    let team = p.data.me.as_ref().map(|m| m.team_id.as_str()).filter(|t| !t.is_empty())?;
    SubjectKey::new("slack", team, MESSAGE, &format!("{channel}/{ts}")).ok()
}

/// Links in `links` that go to a merge request, cut back to the request
/// itself (`…/-/merge_requests/263/diffs#note_1` → `…/-/merge_requests/263`),
/// each once, in order.
pub(crate) fn merge_request_links(links: &[String]) -> Vec<String> {
    const MARK: &str = "/-/merge_requests/";
    let mut out: Vec<String> = Vec::new();
    for l in links {
        let mark = if l.contains(MARK) { MARK } else { "/pull/" };
        let Some(at) = l.find(mark) else { continue };
        if !(l.starts_with("https://") || l.starts_with("http://")) {
            continue;
        }
        let digits: String = l[at + mark.len()..].chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() {
            continue;
        }
        let url = format!("{}{mark}{digits}", &l[..at]);
        if !out.contains(&url) {
            out.push(url);
        }
    }
    out
}

/// The thread a message is in, and the message (the parent, or one reply).
fn held<'a>(p: &'a SlackPlugin, channel: &'a str, ts: &'a str) -> Option<(Thread<'a>, &'a crate::client::Message)> {
    let root = p.known_thread_of(channel, ts)?;
    let thread = p.data.thread(channel, root)?;
    let message = if thread.parent.ts == ts {
        thread.parent
    } else {
        thread.replies.iter().find(|m| m.ts == ts)?
    };
    Some((thread, message))
}

/// The merge requests the thread of `channel`/`ts` links.
pub(crate) fn thread_merge_requests(p: &SlackPlugin, channel: &str, ts: &str) -> Vec<String> {
    let Some((thread, _)) = held(p, channel, ts) else { return vec![] };
    merge_request_links(&thread.links())
}

/// The context for one message of a held thread.
pub(crate) fn context(p: &SlackPlugin, channel: &str, ts: &str) -> Option<AgentContext> {
    let key = key(p, channel, ts)?;
    let (thread, m) = held(p, channel, ts)?;
    let names = Namer { data: &p.data, cfg: &p.cfg };
    let text = |m: &crate::client::Message| mrkdwn::plain(&m.text, &names).trim().to_string();
    let who = |m: &crate::client::Message| p.data.author(m);
    let root = thread.parent.ts.clone();
    let link = p.permalink(thread.channel, &m.ts, (m.ts != root).then_some(root.as_str()));
    let mut c = AgentContext::new(KIND, key, &format!("message in #{}", p.alias(thread.channel)))
        .link(link)
        .field(ContextField::new("channel", format!("#{}", p.alias(thread.channel))))
        .field(ContextField::new("author", who(m)))
        .field(ContextField::new("text", text(m)));
    if m.ts != root {
        c = c
            .field(ContextField::new("thread_started_by", who(thread.parent)))
            .field(ContextField::new("thread_text", text(thread.parent)));
    }
    if let Some(link) = p.permalink(thread.channel, &root, None) {
        c = c.field(ContextField::new("thread_link", link));
    }
    let others: Vec<&crate::client::Message> =
        thread.replies.iter().filter(|r| r.ts != m.ts).collect();
    let skipped = others.len().saturating_sub(MAX_REPLIES);
    if !others.is_empty() {
        let lines: Vec<String> = others[skipped..]
            .iter()
            .map(|r| format!("{}: {}", who(r), text(r)))
            .collect();
        c = c.field(ContextField::new("replies", lines.join("\n")));
        if skipped > 0 {
            c = c.field(ContextField::new("older_replies_left_out", skipped.to_string()));
        }
    }
    let mrs = merge_request_links(&thread.links());
    if !mrs.is_empty() {
        let shown: Vec<String> = mrs.iter().take(MAX_LINKS).cloned().collect();
        c = c.field(ContextField::new("linked_merge_requests", shown.join("\n")));
    }
    Some(c)
}

pub(crate) fn document(p: &SlackPlugin, request: &AgentContextRequest) -> Option<radar_core::context::Document> {
    use radar_core::Plugin;
    p.subject_from_journal(&request.subject)?;
    let (channel, ts) = request.subject.id.split_once('/')?;
    let (thread, _) = held(p, channel, ts)?;
    let mut ctx = context(p, channel, ts)?;
    ctx.subject = request.subject.clone();
    let names = Namer { data: &p.data, cfg: &p.cfg };
    let messages = std::iter::once(thread.parent).chain(thread.replies.iter());
    let text = messages.map(|m| format!("{} {}: {}", m.ts, p.data.author(m), mrkdwn::plain(&m.text, &names))).collect::<Vec<_>>().join("\n\n");
    let mut doc = radar_core::context::Document::from_context(ctx).section("thread", text);
    doc.observed_at = p.data.channels.get(channel).map(|c| c.read_at);
    if thread.parent.reply_count as usize > thread.replies.len() {
        doc.sections.last_mut()?.truncated = true;
        doc.missing.push("some thread replies have not been read".into());
    }
    Some(doc)
}

/// The message the keys act on: the selected one on the feed, a thread
/// page's first message. None on the nav pane, an alert or a heading.
fn target(p: &SlackPlugin, pane: Pane) -> Option<(String, String)> {
    if pane == Pane::Nav {
        return None;
    }
    match p.pages.last() {
        Some(Page::Thread { channel, ts, .. }) => Some((channel.clone(), ts.clone())),
        Some(_) => None,
        None => {
            let item = screen::selected(p)?;
            if matches!(item, Item::Alert { .. }) {
                return None;
            }
            let (channel, ts, _) = item.message(p)?;
            Some((channel, ts))
        }
    }
}

/// The key of both agent actions on a row: the same everywhere a row names
/// a subject (`A`).
pub(crate) const SEND_KEY: char = 'A';
/// Review the merge request a message links, locally.
pub(crate) const REVIEW_KEY: char = 'v';

/// The rows the agent actions add to the actions menu of `pane`, each with
/// its id written out. Nothing when no message is selected.
pub(crate) fn rows(p: &SlackPlugin, pane: Pane) -> Vec<Row> {
    let Some((channel, ts)) = target(p, pane) else { return vec![] };
    // The ids are the host's shared ones (every part's row says the same).
    let mut out = vec![
        head(Binding::heading("with an agent").icon(Some(Icon::Bot))),
        act(SEND_TO_AGENT_ACTION, Binding::new(SEND_KEY.to_string(), "ask an agent about it…")),
    ];
    if !thread_merge_requests(p, &channel, &ts).is_empty() {
        out.push(act(
            REVIEW_LOCALLY_ACTION,
            Binding::new(REVIEW_KEY.to_string(), "review its merge request locally…"),
        ));
    }
    out
}

/// `A` asks the host to send the message to an agent; `v` asks it to
/// start the local review of the merge request(s) the thread links. Nothing
/// is posted from here: the host asks for the context and hands it to the
/// agent dock.
pub(crate) fn key_press(p: &SlackPlugin, pane: Pane, pressed: Chord) -> Option<Outcome> {
    if !pressed.plain() {
        return None;
    }
    let Key::Char(c) = pressed.key else { return None };
    if c != SEND_KEY && c != REVIEW_KEY {
        return None;
    }
    let (channel, ts) = target(p, pane)?;
    let Some(subject) = key(p, &channel, &ts) else {
        return Some(Outcome::Handled(vec![Effect::Notify(
            "this workspace is not known yet: nothing to send".into(),
        )]));
    };
    Some(Outcome::Handled(if c == SEND_KEY {
        if context(p, &channel, &ts).is_none() {
            vec![Effect::Notify("that message is not held here: open its thread first".into())]
        } else {
            vec![Effect::SendToAgent(AgentContextRequest { subject })]
        }
    } else {
        let links = thread_merge_requests(p, &channel, &ts);
        if links.is_empty() {
            vec![Effect::Notify("no merge request linked in this thread".into())]
        } else {
            vec![Effect::StartReview(ReviewRequest { automatic: false, origin: subject, links, changes: vec![] })]
        }
    }))
}

/// Asked of the plugin by the host for a message key of this workspace.
pub(crate) fn agent_context(p: &SlackPlugin, request: &AgentContextRequest) -> Option<AgentContext> {
    let s = &request.subject;
    if s.provider != "slack" || s.kind != MESSAGE {
        return None;
    }
    let team = p.data.me.as_ref().map(|m| m.team_id.as_str())?;
    if s.instance != team {
        return None;
    }
    let (channel, ts) = s.id.split_once('/')?;
    context(p, channel, ts)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::demo::{self, TEAM};

    fn ask_ts() -> String {
        demo::ago(2 * 86_400 + 3600)
    }

    fn field<'a>(c: &'a AgentContext, name: &str) -> Option<&'a ContextField> {
        c.fields.iter().find(|f| f.name == name)
    }

    #[test]
    fn a_message_is_keyed_by_workspace_channel_and_ts() {
        let p = demo::sample();
        let ts = ask_ts();
        let k = key(&p, TEAM, &ts).unwrap();
        assert_eq!((k.provider.as_str(), k.instance.as_str(), k.kind.as_str()), ("slack", "T000ACME1", "message"));
        assert_eq!(k.id, format!("{TEAM}/{ts}"));
    }

    #[test]
    fn a_thread_carries_its_words_and_lists_its_merge_request() {
        let p = demo::sample();
        let ts = ask_ts();
        let c = context(&p, TEAM, &ts).unwrap();
        assert_eq!(field(&c, "author").unwrap().value, "Bob Builder");
        assert!(field(&c, "text").unwrap().value.contains("Add a stats endpoint"));
        assert_eq!(field(&c, "linked_merge_requests").unwrap().value, demo::MR_263);
        assert!(field(&c, "replies").unwrap().value.contains("Carol Coder: approved, nice one"));
        assert_eq!(c.link.as_deref(), field(&c, "thread_link").map(|f| f.value.as_str()));
        assert!(c.link.as_deref().unwrap().starts_with("https://acme.slack.com/archives/"));
    }

    #[test]
    fn a_reply_carries_its_thread_start_and_not_itself_twice() {
        let p = demo::sample();
        let parent = ask_ts();
        let reply = p.data.thread(TEAM, &parent).unwrap().replies[1].ts.clone();
        let c = context(&p, TEAM, &reply).unwrap();
        assert_eq!(field(&c, "text").unwrap().value, "approved, nice one");
        assert_eq!(field(&c, "thread_started_by").unwrap().value, "Bob Builder");
        assert!(field(&c, "thread_text").unwrap().value.contains("stats endpoint"));
        let others = &field(&c, "replies").unwrap().value;
        assert!(others.contains("looking") && !others.contains("approved"));
        assert!(c.link.as_deref().unwrap().contains("thread_ts="));
    }

    #[test]
    fn the_host_asks_by_key_and_only_this_workspace_answers() {
        let p = demo::sample();
        let ts = ask_ts();
        let ask = |k| agent_context(&p, &AgentContextRequest { subject: k });
        assert!(ask(key(&p, TEAM, &ts).unwrap()).is_some());
        let other = SubjectKey::new("slack", "T000OTHER", MESSAGE, &format!("{TEAM}/{ts}")).unwrap();
        assert!(ask(other).is_none());
        let wrong_kind = SubjectKey::new("slack", "T000ACME1", "thread", &format!("{TEAM}/{ts}")).unwrap();
        assert!(ask(wrong_kind).is_none());
        let unknown = SubjectKey::new("slack", "T000ACME1", MESSAGE, &format!("{TEAM}/1.000001")).unwrap();
        assert!(ask(unknown).is_none());
    }

    #[test]
    fn replies_are_bounded_to_the_newest() {
        let mut p = demo::sample();
        let parent = ask_ts();
        let many: Vec<_> = (0..MAX_REPLIES + 4)
            .map(|i| demo::msg(&format!("{}.{:06}", demo::NOW - 100 + i as i64, 1), demo::BOB, &format!("r{i}")))
            .collect();
        p.data.channels.get_mut(TEAM).unwrap().replies.insert(parent.clone(), many);
        let c = context(&p, TEAM, &parent).unwrap();
        let replies = &field(&c, "replies").unwrap().value;
        assert_eq!(replies.lines().count(), MAX_REPLIES);
        assert!(replies.contains("r13") && !replies.contains("r3\n") && !replies.starts_with("Bob Builder: r0"));
        assert_eq!(field(&c, "older_replies_left_out").unwrap().value, "4");
    }

    fn links(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn open_thread(p: &mut SlackPlugin, ts: &str) {
        p.pages.push(Page::Thread { channel: TEAM.into(), ts: ts.into(), tab: 0, row: 0 });
    }

    fn press(p: &mut SlackPlugin, pane: Pane, c: char) -> Option<Vec<Effect>> {
        match key_press(p, pane, crate::keys::chord(Key::Char(c), crate::keys::Mods::SHIFT)) {
            Some(Outcome::Handled(fx)) => Some(fx),
            Some(_) => panic!("only handled outcomes"),
            None => None,
        }
    }

    fn ids(p: &SlackPlugin) -> Vec<String> {
        rows(p, Pane::Middle).into_iter().map(|(id, _)| id).collect()
    }

    #[test]
    fn a_enter_on_a_thread_asks_the_host_for_the_brief_of_that_message() {
        let mut p = demo::sample();
        let ts = ask_ts();
        open_thread(&mut p, &ts);
        let fx = press(&mut p, Pane::Middle, 'A').unwrap();
        let [Effect::SendToAgent(req)] = fx.as_slice() else { panic!("{fx:?}") };
        assert_eq!(req.subject, key(&p, TEAM, &ts).unwrap());
        // The host's question is answered by the same plugin, from what it holds.
        assert!(agent_context(&p, req).is_some());
    }

    #[test]
    fn v_starts_a_review_of_the_linked_merge_request_with_the_message_as_origin() {
        let mut p = demo::sample();
        let ts = ask_ts();
        open_thread(&mut p, &ts);
        let fx = press(&mut p, Pane::Middle, 'v').unwrap();
        let [Effect::StartReview(r)] = fx.as_slice() else { panic!("{fx:?}") };
        assert_eq!(r.origin, key(&p, TEAM, &ts).unwrap());
        assert_eq!(r.links, [demo::MR_263]);
        assert!(r.changes.is_empty(), "the host maps links to the owning part's keys");
    }

    #[test]
    fn a_thread_without_a_merge_request_offers_no_review_and_says_so_on_v() {
        let mut p = demo::sample();
        let lunch = p.data.channels[TEAM].messages[2].ts.clone();
        open_thread(&mut p, &lunch);
        assert_eq!(ids(&p), ["", "agent.send"].map(String::from));
        let fx = press(&mut p, Pane::Middle, 'v').unwrap();
        assert!(matches!(fx.as_slice(), [Effect::Notify(m)] if m.contains("no merge request")));
        let mut p = demo::sample();
        open_thread(&mut p, &ask_ts());
        assert_eq!(ids(&p), ["", "agent.send", "agent.review_locally"].map(String::from));
    }

    #[test]
    fn the_nav_pane_and_other_keys_are_not_taken() {
        let mut p = demo::sample();
        open_thread(&mut p, &ask_ts());
        assert!(press(&mut p, Pane::Nav, 'A').is_none());
        assert!(rows(&p, Pane::Nav).is_empty());
        assert!(press(&mut p, Pane::Middle, 'a').is_none() || key_press(&p, Pane::Middle, crate::keys::chord(Key::Char('a'), crate::keys::Mods::NONE)).is_none());
        assert!(key_press(&p, Pane::Middle, crate::keys::chord(Key::Char('A'), crate::keys::Mods::CONTROL)).is_none());
    }

    #[test]
    fn the_shared_ids_run_their_keys_and_an_unlisted_id_runs_nothing() {
        use radar_core::view::Intent;
        let mut p = demo::sample();
        open_thread(&mut p, &ask_ts());
        let resolve = |p: &SlackPlugin, id: &str| {
            p.resolve_action(crate::SCREEN.id, Pane::Middle, &Intent::Invoke { action: id.into() })
        };
        let key_of = |i: Option<Intent>| match i {
            Some(Intent::Key { chord }) => Some(chord.key),
            _ => None,
        };
        assert_eq!(key_of(resolve(&p, SEND_TO_AGENT_ACTION)), Some(radar_core::view::Key::Char('A')));
        assert_eq!(key_of(resolve(&p, REVIEW_LOCALLY_ACTION)), Some(radar_core::view::Key::Char('v')));
        assert!(resolve(&p, "agent.nothing").is_none());
        // Without a merge request in the thread, review is not offered, so its id runs nothing.
        let mut q = demo::sample();
        let lunch = q.data.channels[TEAM].messages[2].ts.clone();
        open_thread(&mut q, &lunch);
        assert!(resolve(&q, REVIEW_LOCALLY_ACTION).is_none());
        assert!(resolve(&q, SEND_TO_AGENT_ACTION).is_some());
    }

    #[test]
    fn a_message_that_is_not_held_says_so_and_sends_nothing() {
        let mut p = demo::sample();
        open_thread(&mut p, "1.000001");
        let fx = press(&mut p, Pane::Middle, 'A').unwrap();
        assert!(matches!(fx.as_slice(), [Effect::Notify(m)] if m.contains("not held")));
    }

    #[test]
    fn a_merge_request_link_is_cut_back_to_the_request() {
        let got = merge_request_links(&links(&[
            "https://git.example/acme/team-a/api/-/merge_requests/263/diffs#note_1",
            "https://git.example/acme/team-a/api/-/merge_requests/263",
            "https://git.example/acme/team-a/api/-/merge_requests/270?x=1",
            "https://git.example/acme/team-a/api/-/issues/5",
            "https://git.example/acme/team-a/api/-/merge_requests/",
            "not-a-url/-/merge_requests/9",
        ]));
        assert_eq!(
            got,
            [
                "https://git.example/acme/team-a/api/-/merge_requests/263",
                "https://git.example/acme/team-a/api/-/merge_requests/270"
            ]
        );
    }
}
