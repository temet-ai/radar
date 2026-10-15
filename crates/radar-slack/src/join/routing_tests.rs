//! No live provider or opener: exact routing, recorded reads and real Host effects.

use super::*;
use crate::client::{Recorded, Reply};
use crate::{Via, demo};
use radar_core::host::{Event, HostEffect, JournalRecord};
use radar_core::journal::{ActionId, Journal, Query};
use radar_core::plugin::Msg;
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use radar_core::{Host, Plugin};
use std::sync::Arc;

const ROOT: &str = "1791200000.000100";
const REPLY: &str = "1791200001.000200";
const LINK: &str = "https://acme.slack.com/archives/C0000TEAM1/p1791200000000100";
const FOREIGN: &str = "https://other.slack.com/archives/C0000TEAM1/p1791200000000100?token=ROUTING_QUERY_CANARY#ROUTING_FRAGMENT_CANARY";
const EMBEDDED: &str =
    "https://foreign.example/?next=(https://acme.slack.com/archives/C0000TEAM1/p1791200000000100)";

fn sample() -> (SlackPlugin, Arc<Recorded>) {
    let mut p = demo::sample();
    let rec = Recorded::new(vec![]);
    p.via = Via(Some(rec.clone()));
    (p, rec)
}

fn local() -> Subject {
    Subject::new(
        THREAD,
        format!("{}/{ROOT}", demo::TEAM),
        "ROUTING_LABEL_CANARY",
    )
    .with("channel", demo::TEAM)
    .with("ts", ROOT)
}

fn one(p: &SlackPlugin, url: &str) -> Subject {
    let found = recognise(p, url);
    assert_eq!(found.len(), 1, "{url}");
    assert_eq!(found[0].0, 0..url.len());
    found.into_iter().next().unwrap().1
}

#[test]
fn same_workspace_parent_http_case_default_ports_and_reply_keep_the_original_url() {
    let (p, _) = sample();
    for (url, root) in [
        (LINK.to_string(), ROOT),
        (LINK.replace("https:", "http:"), ROOT),
        (LINK.replace("acme.slack.com", "ACME.SLACK.COM:443"), ROOT),
        (
            LINK.replace("https://acme.slack.com", "HTTP://ACME.SLACK.COM:80"),
            ROOT,
        ),
        (format!("{LINK}?unknown=retained#fragment"), ROOT),
        (
            format!(
                "https://acme.slack.com/archives/{}/p{}?thread_ts={ROOT}&cid={}",
                demo::TEAM,
                REPLY.replace('.', ""),
                demo::TEAM
            ),
            ROOT,
        ),
    ] {
        let s = one(&p, &url);
        assert_eq!(s.id, format!("{}/{root}", demo::TEAM));
        assert_eq!(s.field("channel"), Some(demo::TEAM));
        assert_eq!(s.field("ts"), Some(root));
        assert_eq!(s.field("team_id"), Some("T000ACME1"));
        assert_eq!(s.url.as_deref(), Some(url.as_str()));
    }
}

#[test]
fn known_workspace_origin_is_normalized_but_no_alias_is_inferred() {
    let (mut p, _) = sample();
    for workspace in ["HTTPS://ACME.SLACK.COM:443", "http://ACME.slack.com:80/"] {
        p.data.me.as_mut().unwrap().url = workspace.into();
        one(&p, LINK);
    }
    for workspace in ["https://renamed.slack.com/", "https://acme-old.slack.com/"] {
        p.data.me.as_mut().unwrap().url = workspace.into();
        assert!(recognise(&p, LINK).is_empty());
    }
}

#[test]
fn recognition_reports_exact_utf8_byte_ranges_and_several_whole_candidates() {
    let (p, _) = sample();
    let second = format!("{LINK}?thread_ts={REPLY}&extra=value#kept");
    let text = format!("先\u{2003}<{LINK}|parent> then ({second}) end");
    let found = recognise(&p, &text);
    assert_eq!(found.len(), 2);
    assert_eq!(&text[found[0].0.clone()], LINK);
    assert_eq!(&text[found[1].0.clone()], second);
    assert_eq!(found[1].1.url.as_deref(), Some(second.as_str()));
    assert_eq!(found[1].1.field("ts"), Some(REPLY));
}

#[test]
fn foreign_missing_or_unusable_workspace_is_not_recognized() {
    let (mut p, _) = sample();
    assert!(recognise(&p, FOREIGN).is_empty());
    p.data.me = None;
    assert!(recognise(&p, LINK).is_empty());
    for workspace in [
        "",
        "acme.slack.com",
        "https://slack.com/",
        "https://acme.slack.com.evil.example/",
        "https://nested.acme.slack.com/",
        "https://acme.slack.com./",
        "https://-acme.slack.com/",
        "https://acme-.slack.com/",
        "https://ac_me.slack.com/",
        "https://user@acme.slack.com/",
        "https://@acme.slack.com/",
        "https://acme.slack.com:444/",
        "ftp://acme.slack.com/",
        "https://acme.slack.com/path",
        "https://acme.slack.com/?x=1",
        "https://acme.slack.com/#fragment",
        "https://acme.slack.com/a/../",
        " https://acme.slack.com/",
    ] {
        let mut me = demo::sample().data.me.unwrap();
        me.url = workspace.into();
        p.data.me = Some(me);
        assert!(recognise(&p, LINK).is_empty(), "{workspace}");
    }
}

#[test]
fn deceptive_authorities_query_fragment_targets_and_nondefault_ports_stay_external() {
    let (p, _) = sample();
    for url in [
        LINK.replace("acme.slack.com", "other.slack.com"),
        LINK.replace("acme.slack.com", "acme.slack.com.evil.example"),
        LINK.replace("acme.slack.com", "acme.slack.com."),
        LINK.replace("acme.slack.com", "nested.acme.slack.com"),
        LINK.replace("acme.slack.com", "evil.example?next=acme.slack.com"),
        LINK.replace("acme.slack.com", "evil.example#next=acme.slack.com"),
        LINK.replace("acme.slack.com", "user:pass@acme.slack.com"),
        LINK.replace("acme.slack.com", "@acme.slack.com"),
        LINK.replace("acme.slack.com", "acme.slack.com@evil.example"),
        LINK.replace("acme.slack.com", "acme.slack.com:80"),
        LINK.replace("https://acme.slack.com", "http://acme.slack.com:443"),
        LINK.replace("acme.slack.com", "ac%6de.slack.com"),
        LINK.replace("https:", "ftp:"),
        format!("https://acme.slack.com/?next={LINK}"),
        format!("https://acme.slack.com/#{LINK}"),
    ] {
        assert!(recognise(&p, &url).is_empty(), "{url}");
    }
}

#[test]
fn path_must_be_exact_without_url_parser_repairs_or_encoded_separators() {
    let (p, _) = sample();
    for path in [
        "/archives/C0000TEAM1/p1791200000000100/",
        "/archives/C0000TEAM1/p1791200000000100/extra",
        "/prefix/archives/C0000TEAM1/p1791200000000100",
        "/x/../archives/C0000TEAM1/p1791200000000100",
        "/%2e/archives/C0000TEAM1/p1791200000000100",
        "/archives/C0000TEAM1/../C0000TEAM1/p1791200000000100",
        "/archives/C0000TEAM1%2fp1791200000000100",
        "/archives/C0000TEAM1/%701791200000000100",
        "/archives/C0000TEAM1/p1791200000000100%2f",
        "/archives/C0000TEAM1\\p1791200000000100",
        "/archives/C1/p1791200000000100",
        "/archives/c0000TEAM1/p1791200000000100",
        "/archives/C0000TEAM1/P1791200000000100",
        "/archives/C0000TEAM1/p123456",
        "/archives/C0000TEAM1/p1791200000.000100",
        "/archives/C0000TEAM1/p1791200000000100x",
    ] {
        let url = format!("https://acme.slack.com{path}");
        assert!(recognise(&p, &url).is_empty(), "{path}");
    }
}

#[test]
fn exact_query_key_is_the_only_optional_thread_root_and_fragments_never_route() {
    let (p, _) = sample();
    for suffix in [
        format!("?not_thread_ts={REPLY}"),
        format!("?next=thread_ts={REPLY}"),
        format!("#thread_ts={REPLY}"),
        format!("?thread_ts_extra={REPLY}"),
        format!(
            "?next=/archives/{}/p{}#thread_ts={REPLY}",
            demo::SVC,
            REPLY.replace('.', "")
        ),
    ] {
        let url = format!("{LINK}{suffix}");
        let s = one(&p, &url);
        assert_eq!(s.field("ts"), Some(ROOT));
        assert_eq!(s.url.as_deref(), Some(url.as_str()));
    }
    let s = one(&p, &format!("{LINK}?thread_ts={REPLY}#thread_ts=bad"));
    assert_eq!(s.field("ts"), Some(REPLY));
}

#[test]
fn thread_root_is_numeric_and_duplicate_keys_must_agree_exactly() {
    let (p, _) = sample();
    for query in [
        "thread_ts=",
        "thread_ts=12",
        "thread_ts=.1",
        "thread_ts=12.",
        "thread_ts=1.2.3",
        "thread_ts=-1.2",
        "thread_ts=1e2.3",
        "thread_ts=1.%20",
        "thread_ts=1.2x",
        "thread_ts=1.2&thread_ts=1.20",
        "thread_ts=1.2&thread_ts=01.2",
        "thread_ts=1.2&thread_ts=bad",
        "thread_ts=1.%FF",
        "thread_ts=1.%26thread_ts=2.3",
    ] {
        assert!(
            recognise(&p, &format!("{LINK}?{query}")).is_empty(),
            "{query}"
        );
    }
    for root in ["1.2", "0001.20", ROOT] {
        let s = one(&p, &format!("{LINK}?thread_ts={root}&thread_ts={root}"));
        assert_eq!(s.field("ts"), Some(root));
    }
}

#[test]
fn generated_subject_captures_only_a_known_team() {
    let (mut p, _) = sample();
    let s = thread_subject(&p, demo::TEAM, ROOT);
    assert_eq!(s.field("team_id"), Some("T000ACME1"));
    assert_eq!(s.url.as_deref(), Some(LINK));
    p.data.me.as_mut().unwrap().team_id.clear();
    assert_eq!(thread_subject(&p, demo::TEAM, ROOT).field("team_id"), None);
    p.data.me = None;
    let s = thread_subject(&p, demo::TEAM, ROOT);
    assert!(s.field("team_id").is_none() && s.url.is_none());
}

fn assert_refused(p: &mut SlackPlugin, s: &Subject) {
    let data = p.data.clone();
    let pages = p.pages.clone();
    let threads = format!("{:?}", p.threads);
    let scroll = p.page_scroll.at();
    assert!(matches!(show(p, s), Showing::Cannot), "{s:?}");
    assert_eq!(p.data, data, "no read mark/cache mutation");
    assert_eq!(p.pages, pages, "no page push");
    assert_eq!(
        format!("{:?}", p.threads),
        threads,
        "no loading/cache mutation"
    );
    assert_eq!(p.page_scroll.at(), scroll);
}

#[test]
fn show_rejects_inconsistent_id_fields_url_and_team_before_any_local_effect() {
    let (mut p, rec) = sample();
    p.page_scroll.set(41);
    let mut wrong_id = local();
    wrong_id.id = format!("{}/{REPLY}", demo::TEAM);
    let subjects = [
        wrong_id,
        local().with("channel", demo::SVC),
        local().with("ts", REPLY),
        local().with("channel", "C1"),
        local().with("ts", "1.bad"),
        Subject::new(THREAD, format!("{}/{ROOT}", demo::TEAM), "label"),
        local().with_url(FOREIGN),
        local().with_url(LINK.replace(demo::TEAM, demo::SVC)),
        local().with_url(format!("{LINK}?thread_ts={REPLY}")),
        local().with("team_id", "T000NEXT1"),
        local().with("team_id", ""),
        local().with_url(format!("{LINK}/extra")),
    ];
    for s in subjects {
        assert_refused(&mut p, &s);
    }
    p.data.me = None;
    assert_refused(&mut p, &local().with_url(LINK));
    assert_refused(&mut p, &local().with("team_id", "T000ACME1"));
    assert!(rec.requests().is_empty());
}

#[test]
fn captured_team_reference_is_refused_after_installing_another_team_even_at_equal_host() {
    let (mut p, rec) = sample();
    let captured = one(&p, LINK);
    let mut next = p.data.me.clone().unwrap();
    next.team_id = "T000NEXT1".into();
    p.install_me(next);
    assert_refused(&mut p, &captured);
    assert!(p.journal_subject(&captured).is_none());
    assert!(rec.requests().is_empty());
}

#[test]
fn valid_legacy_direct_local_and_generated_subjects_preserve_open_and_recorded_read() {
    for bare in [false, true] {
        let (mut p, _) = sample();
        p.data
            .channels
            .get_mut(demo::TEAM)
            .unwrap()
            .messages
            .clear();
        let rec = Recorded::new(vec![Ok(Reply {
            status: 200,
            body: serde_json::json!({"ok":true,"messages": [{"ts":ROOT,"text":"held"}]})
                .to_string(),
            ..Default::default()
        })]);
        p.via = Via(Some(rec.clone()));
        let s = if bare {
            local()
        } else {
            thread_subject(&p, demo::TEAM, ROOT)
        };
        let Showing::Page { effects, .. } = show(&mut p, &s) else {
            panic!("valid local show")
        };
        assert_eq!(effects.iter().filter(|e| matches!(e, Effect::Observe { action, .. } if action.kind.as_str() == "slack.page.open")).count(), 1);
        let task = effects
            .into_iter()
            .find_map(|e| match e {
                Effect::SpawnAction { task, .. } => Some(task),
                _ => None,
            })
            .unwrap();
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(task);
        p.on_msg(result);
        let requests = rec.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "conversations.replies");
        assert!(
            requests[0]
                .params
                .contains(&("channel".into(), demo::TEAM.into()))
        );
        assert!(requests[0].params.contains(&("ts".into(), ROOT.into())));
        assert_eq!(p.pages.len(), 1);
        assert_eq!(p.data.opened[&format!("{}/{ROOT}", demo::TEAM)], ROOT);
    }
}

#[test]
fn reply_permalink_show_opens_root_and_draft_show_is_unchanged() {
    let (mut p, rec) = sample();
    let s = one(&p, &format!("{LINK}?thread_ts={REPLY}"));
    assert!(matches!(show(&mut p, &s), Showing::Page { .. }));
    assert!(matches!(p.pages.last(), Some(Page::Thread { ts, .. }) if ts == REPLY));
    p.draft = Some(crate::ask::Draft::default());
    let draft = draft_subject(&p);
    let Showing::Page { effects, .. } = show(&mut p, &draft) else {
        panic!("draft show")
    };
    assert!(matches!(p.pages.last(), Some(Page::Draft)));
    assert!(!effects.iter().any(
        |e| matches!(e, Effect::Observe { action, .. } if action.kind.as_str() == "slack.page.open")
    ));
    assert!(
        rec.requests().is_empty(),
        "no returned task or opener is run"
    );
}

struct Sender;
impl Plugin for Sender {
    fn id(&self) -> &'static str {
        "sender"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn on_msg(&mut self, msg: Msg) -> Vec<Effect> {
        vec![Effect::Show(*msg.downcast::<Subject>().unwrap())]
    }
}

fn host(p: SlackPlugin) -> Host {
    let mut host = Host::new(vec![Box::new(demo::sample()), Box::new(Sender)], demo::NOW).unwrap();
    host.load_config(Some("")).unwrap();
    host.plugins[0] = Box::new(p);
    host.enable_journal();
    host
}

fn slack(host: &Host) -> &SlackPlugin {
    assert_eq!(host.plugins[0].id(), "slack");
    // SAFETY: host() always installs this exact type at index zero.
    unsafe { &*(host.plugins[0].as_ref() as *const dyn Plugin as *const SlackPlugin) }
}

fn inner(fx: &HostEffect) -> &HostEffect {
    match fx {
        HostEffect::Tracked { effect, .. } => inner(effect),
        _ => fx,
    }
}

fn send(host: &mut Host, s: Subject) -> Vec<HostEffect> {
    host.update(Event::Msg {
        plugin: 1,
        msg: Box::new(s),
    })
}

fn no_local_open(host: &Host, effects: &[HostEffect]) {
    assert!(slack(host).pages.is_empty());
    assert!(slack(host).threads.is_empty());
    assert!(!effects.iter().any(|e| matches!(
        inner(e),
        HostEffect::Spawn { .. } | HostEffect::WriteCache { .. }
    )));
    assert!(!effects.iter().any(|e| matches!(e, HostEffect::Journal(JournalRecord::Begin { descriptor, .. }) if descriptor.kind.as_str().starts_with("slack.page."))));
}

fn only_open(effects: &[HostEffect], expected: &str) {
    let opens: Vec<_> = effects
        .iter()
        .filter_map(|e| match inner(e) {
            HostEffect::Open(url) => Some(url.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(opens, [expected]);
}

#[test]
fn host_foreign_and_deceptive_links_fall_back_to_exact_original_without_slack_read() {
    for url in [
        FOREIGN.to_string(),
        EMBEDDED.to_string(),
        LINK.replace("acme.slack.com", "evil.example?next=acme.slack.com"),
    ] {
        let (p, rec) = sample();
        let before = p.data.clone();
        let mut host = host(p);
        let effects = send(&mut host, Subject::link(&url));
        no_local_open(&host, &effects);
        only_open(&effects, &url);
        assert_eq!(slack(&host).data, before);
        assert!(rec.requests().is_empty());
        // HostEffect::Open is the existing browser/SSH-clipboard path; never execute it here.
    }
}

#[test]
fn host_rejected_old_team_has_no_false_current_team_subject_or_local_open() {
    let (mut p, rec) = sample();
    let captured = one(&p, LINK);
    let mut next = p.data.me.clone().unwrap();
    next.team_id = "T000NEXT1".into();
    p.install_me(next);
    let before = p.data.clone();
    let mut host = host(p);
    let effects = send(&mut host, captured);
    only_open(&effects, LINK);
    no_local_open(&host, &effects);
    assert_eq!(slack(&host).data, before);
    let show = effects
        .iter()
        .find_map(|e| match e {
            HostEffect::Journal(JournalRecord::Begin { descriptor, .. })
                if descriptor.kind.as_str() == "ui.show" =>
            {
                Some(descriptor)
            }
            _ => None,
        })
        .unwrap();
    assert!(
        show.subjects.is_empty(),
        "refused stale team must not borrow current workspace"
    );
    assert!(rec.requests().is_empty());
}

#[test]
fn host_missing_url_refusal_says_cannot_open_and_has_no_opener_or_local_effect() {
    let (p, rec) = sample();
    let mut host = host(p);
    let effects = send(&mut host, local().with("ts", "bad"));
    no_local_open(&host, &effects);
    assert!(
        !effects
            .iter()
            .any(|e| matches!(inner(e), HostEffect::Open(_)))
    );
    assert_eq!(
        host.message.as_ref().unwrap().text,
        "cannot open ROUTING_LABEL_CANARY here"
    );
    assert!(rec.requests().is_empty());
}

#[test]
fn foreign_link_stays_visible_on_links_tab_and_internal_threads_do_not_group() {
    let (mut p, rec) = sample();
    let held = p.data.channels.get_mut(demo::TEAM).unwrap();
    held.messages = vec![demo::msg(
        ROOT,
        demo::BOB,
        &format!("<{LINK}|local thread> <{FOREIGN}|foreign thread>"),
    )];
    held.replies.clear();
    let mut host = host(p);
    send(&mut host, local());
    host.update(Event::Key(KeyEvent::new(
        KeyCode::Char(']'),
        KeyModifiers::NONE,
    )));
    let text = radar_core::shell::to_text(&host, 160, 40);
    let links = slack(&host).page_links.borrow();
    assert_eq!(links.len(), 1, "{text}");
    assert!(links[0].is(radar_core::compose::LINK));
    assert_eq!(links[0].url.as_deref(), Some(FOREIGN));
    assert!(text.contains("foreign thread"));
    assert!(rec.requests().is_empty());
}

#[test]
fn whole_foreign_link_with_embedded_local_reference_is_visible_and_stays_external() {
    let (mut p, rec) = sample();
    let recognised = recognise(&p, EMBEDDED);
    assert_eq!(
        recognised.len(),
        1,
        "ordinary embedded text recognition stays compatible"
    );
    assert_ne!(recognised[0].0, 0..EMBEDDED.len());
    let held = p.data.channels.get_mut(demo::TEAM).unwrap();
    held.messages = vec![demo::msg(
        ROOT,
        demo::BOB,
        &format!("<{EMBEDDED}|embedded foreign link>"),
    )];
    held.replies.clear();
    let mut host = host(p);
    send(&mut host, local());
    host.update(Event::Key(KeyEvent::new(
        KeyCode::Char(']'),
        KeyModifiers::NONE,
    )));
    radar_core::shell::to_text(&host, 160, 40);
    let link = {
        let links = slack(&host).page_links.borrow();
        assert_eq!(links.len(), 1);
        assert!(links[0].is(radar_core::compose::LINK));
        assert_eq!(links[0].url.as_deref(), Some(EMBEDDED));
        links[0].clone()
    };
    let effects = send(&mut host, link);
    only_open(&effects, EMBEDDED);
    assert_eq!(
        slack(&host).pages.len(),
        1,
        "no second local page for an embedded target"
    );
    assert!(rec.requests().is_empty());
}

#[test]
fn routing_journal_exports_no_raw_url_query_fragment_label_or_token_canaries() {
    let (p, _) = sample();
    let mut host = host(p);
    let mut effects = send(&mut host, Subject::link(FOREIGN));
    effects.extend(send(&mut host, local().with_url(FOREIGN)));
    let dir = std::env::temp_dir().join(format!("radar-slack-routing-{}", ActionId::new().as_str()));
    let mut db = Journal::open(&dir.join("actions.sqlite3")).unwrap();
    let session = db.start_session().unwrap();
    for effect in &effects {
        match effect {
            HostEffect::Journal(JournalRecord::Begin {
                id,
                descriptor,
                parent,
            }) => db
                .begin_with_id(&session, id, descriptor.clone(), parent.as_ref())
                .unwrap(),
            HostEffect::Journal(JournalRecord::Outcome {
                id,
                outcome,
                metadata,
            }) => {
                db.record(id, *outcome, metadata.clone()).unwrap();
            }
            HostEffect::Journal(JournalRecord::Gap { .. }) => panic!("unexpected gap"),
            _ => {}
        }
    }
    db.finish_session(&session).unwrap();
    let events = db
        .query(&Query {
            limit: 1000,
            ..Default::default()
        })
        .unwrap();
    let json = serde_json::to_string(&events).unwrap();
    assert!(
        !events
            .iter()
            .any(|e| e.kind.as_str().starts_with("slack.page."))
    );
    for forbidden in [
        FOREIGN,
        "ROUTING_QUERY_CANARY",
        "ROUTING_FRAGMENT_CANARY",
        "ROUTING_LABEL_CANARY",
        "xoxp-",
        "C0000TEAM1/p",
    ] {
        assert!(!json.contains(forbidden), "{forbidden}");
        for file in std::fs::read_dir(&dir).unwrap() {
            let bytes = std::fs::read(file.unwrap().path()).unwrap();
            assert!(
                !bytes
                    .windows(forbidden.len())
                    .any(|w| w == forbidden.as_bytes()),
                "{forbidden}"
            );
        }
    }
    drop(db);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn thread_routes_and_reads_require_current_authenticated_identity() {
    for missing_me in [false, true] {
        let (mut p, rec) = sample();
        if missing_me {
            p.data.me = None;
        } else {
            p.session_owner = None;
        }
        let before = p.data.clone();
        assert_refused(&mut p, &local());
        assert!(p.journal_subject(&local()).is_none());
        for effects in [
            page::open_thread(&mut p, demo::TEAM, ROOT),
            page::fetch_thread(&mut p, demo::TEAM, ROOT),
        ] {
            assert!(
                matches!(effects.as_slice(), [Effect::Notify(message)] if message.contains("check the Slack connection"))
            );
        }
        assert_eq!(p.data, before);
        assert!(p.pages.is_empty());
        assert!(p.threads.is_empty());
        assert!(rec.requests().is_empty());
    }
}
