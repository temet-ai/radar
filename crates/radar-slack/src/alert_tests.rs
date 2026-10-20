//! The Alerts view and what feeds it, through the real host: a sample of
//! two alerts channels (one read as Alertmanager's attachments, one as
//! failed jobs' blocks) with a firing alert mapped to a repo, one unmapped,
//! one resolved, a job that failed, a resolved one too old to show, a bot
//! message no reader can read and a person's message.

use radar_core::Host;
use radar_core::host::HostEffect;
use radar_core::ratatui::crossterm::event::KeyCode;


use crate::client::{Block, Card, Message};
use crate::config::{Kind, Reader, Rule};
use crate::model::ChannelData;
use crate::tests::{menu_shown, shown, ALICE, NOW, ago, chars, every_row, hosted_with_parts, keys, menu_labels, msg, press, slack_places_of};
use crate::{SlackPlugin, tests};

pub(crate) const ALRT: &str = "C0000ALRT1";
pub(crate) const JOBS: &str = "C00000JOB1";
pub(crate) const CONSOLE: &str = "https://console.acme.example/alerts/42";

fn card(color: &str, title: &str, text: &str) -> Card {
    Card { color: color.into(), title: title.into(), text: text.into(), ..Card::default() }
}

fn bot(ts: String, cards: Vec<Card>) -> Message {
    Message {
        ts,
        bot_name: Some("alertbot".into()),
        subtype: Some("bot_message".into()),
        cards,
        ..Message::default()
    }
}

fn firing(name: &str, env: &str, priority: &str) -> Card {
    card(
        "danger",
        &format!("[FIRING:1] {name}"),
        &format!(
            "*Summary:* Error rate above 5% for 10m\n*Environment:* {env}\n*Priority:* {priority}\n*Console:* <{CONSOLE}|console>"
        ),
    )
}

fn resolved(name: &str, env: &str) -> Card {
    card("good", &format!("[RESOLVED] {name}"), &format!("*Environment:* {env}"))
}

/// The sample with an alerts channel for Alertmanager's attachments and one
/// for jobs that failed.
pub(crate) fn sample() -> SlackPlugin {
    let mut p = tests::sample();
    for (alias, id, reader, color) in [("team-alerts", ALRT, Reader::Alertmanager, "magenta"), ("jobs", JOBS, Reader::JobFailed, "purple")] {
        p.cfg.channels.push(crate::config::Channel {
            alias: alias.into(),
            id: id.into(),
            name: format!("acme-{alias}"),
            purpose: String::new(),
            kind: Kind::Alerts,
            color: color.into(),
            watched: true,
            reader: Some(reader),
        });
    }
    p.cfg.alerts.rules.push(Rule {
        prefix: "api-".into(),
        repo: "acme/team-a/api".into(),
        service: "api".into(),
        ..Rule::default()
    });
    let first = ago(3 * 3600 + 7 * 60);
    let mut high = bot(first.clone(), vec![firing("api-HighErrorRate", "prod", "P1")]);
    high.reply_count = 1;
    high.reply_users = vec![ALICE.into()];
    high.latest_reply = Some(ago(3000));
    let alerts = ChannelData {
        info: None,
        messages: vec![
            bot(ago(3 * 86_400), vec![firing("api-OldOne", "prod", "P2")]),
            bot(ago(3 * 86_400 - 600), vec![resolved("api-OldOne", "prod")]),
            bot(ago(6 * 3600), vec![firing("api-LatencyHigh", "staging", "P3")]),
            bot(ago(5 * 3600 + 20 * 60), vec![resolved("api-LatencyHigh", "staging")]),
            high,
            bot(ago(3700), vec![card("danger", "[FIRING:1] DiskFull", "*Environment:* prod\n*Priority:* P2")]),
            bot(ago(3600), vec![firing("api-HighErrorRate", "prod", "P1")]),
            bot(ago(1200), vec![card("danger", "Maintenance window starts at noon", "")]),
            msg(&ago(900), ALICE, "who is on the disk alert?"),
        ],
        replies: [(first, vec![msg(&ago(3000), ALICE, "looking at it")])].into_iter().collect(),
        ..Default::default()
    };
    let job = Message {
        ts: ago(2 * 3600),
        bot_name: Some("jobbot".into()),
        subtype: Some("bot_message".into()),
        blocks: vec![
            Block { kind: "header".into(), texts: vec![":x: Nightly export failed".into()] },
            Block { kind: "context".into(), texts: vec!["Environment: prod | <https://ci.acme.example/runs/9|run 9>".into()] },
        ],
        ..Message::default()
    };
    p.data.channels.insert(ALRT.into(), alerts);
    p.data.channels.insert(JOBS.into(), ChannelData { info: None, messages: vec![job], ..Default::default() });
    p
}

pub(crate) fn hosted() -> Host {
    hosting(sample())
}

fn hosting(p: SlackPlugin) -> Host {
    let mut host = Host::new(vec![Box::new(SlackPlugin::new())], NOW).unwrap();
    host.load_config(Some("")).unwrap();
    host.plugins[0] = Box::new(p);
    host
}

/// On the Alerts nav rows, then its tree (the cursor on the first alert).
fn on_alerts(host: &mut Host) {
    keys(host, &[KeyCode::Char('1'), KeyCode::Down, KeyCode::Down, KeyCode::Char('2')]);
}

/// Rows down from the first alert (the tree's headings are rows too):
/// DiskFull, the job that failed, and the resolved api-LatencyHigh.
const DISK: usize = 2;
const JOB: usize = 5;
const LATENCY: usize = 8;

fn down(host: &mut Host, n: usize) {
    for _ in 0..n {
        press(host, KeyCode::Down);
    }
}

fn at(text: &str, s: &str) -> usize {
    text.find(s).unwrap_or_else(|| panic!("{s} not shown:\n{text}"))
}

#[test]
fn the_rail_has_alerts_after_discussions_only_with_an_alerts_channel() {
    let host = hosted();
    let text = shown(&host, 160, 40);
    assert!(at(&text, "Discussions") < at(&text, "Alerts"), "{text}");
    assert!(at(&text, "Alerts") < at(&text, "#team-alerts"), "{text}");
    // No alerts channel: no row.
    let plain = tests::hosted();
    assert!(!shown(&plain, 160, 40).contains("Alerts"));
}

#[test]
fn alerts_show_firing_first_then_failed_jobs_then_what_resolved_by_service_and_environment() {
    let mut host = hosted();
    on_alerts(&mut host);
    let text = shown(&host, 160, 40);
    let order = [
        "firing",
        "api  prod",
        ":alert_firing: ▲ api-HighErrorRate ×2",
        "unmapped  prod",
        ":alert_firing: ▲ DiskFull",
        "failed jobs",
        ":run_failed: ✗ Nightly export",
        "recently resolved",
        "api  staging",
        ":alert_resolved: ✓ api-LatencyHigh",
    ];
    let mut from = 0;
    for s in order {
        let i = text[from..].find(s).unwrap_or_else(|| panic!("{s} not after the one before it:\n{text}")) + from;
        from = i + s.len();
    }
    assert!(text.contains("3h 07m"), "how long it has fired: {text}");
    assert!(text.contains("40m"), "how long the resolved one fired: {text}");
    assert!(!text.contains("api-OldOne"), "resolved too long ago to show: {text}");
    // The nav pane counts what fires, and the status line too.
    assert!(text.contains("2 firing"), "{text}");
    insta::assert_snapshot!("alerts", text);
}

#[test]
fn the_preview_tells_the_alert_how_long_where_it_belongs_and_its_console_link() {
    let mut host = hosted();
    on_alerts(&mut host);
    let text = shown(&host, 160, 40);
    for s in [
        ":alert_firing: firing for 3h 07m",
        "environment",
        "prod",
        "priority",
        "P1",
        "Error rate above 5% for 10m",
        CONSOLE,
        "service api · repo acme/team-a/api",
        "1 reply in its thread",
        "notified 2 times",
    ] {
        assert!(text.contains(s), "{s}:\n{text}");
    }
    // An unmapped one says so, and a job that failed says when.
    down(&mut host, DISK);
    let text = shown(&host, 160, 40);
    assert!(text.contains("unmapped: no rule in settings places it"), "{text}");
    down(&mut host, JOB - DISK);
    let text = shown(&host, 160, 40);
    assert!(text.contains(":run_failed: failed 2h 00m ago") && text.contains("https://ci.acme.example/runs/9"), "{text}");
    // A resolved one: after how long, and when.
    down(&mut host, LATENCY - JOB);
    let text = shown(&host, 160, 40);
    assert!(text.contains(":alert_resolved: resolved after 40m"), "{text}");
}

#[test]
fn enter_opens_the_alerts_thread_o_its_console_link_and_y_copies_it() {
    let mut host = hosted();
    on_alerts(&mut host);
    let fx = press(&mut host, KeyCode::Char('o'));
    assert!(matches!(&fx[..], [HostEffect::Open(u)] if u == CONSOLE), "{fx:?}");
    let fx = press(&mut host, KeyCode::Char('y'));
    assert!(matches!(&fx[..], [HostEffect::Copy(u), ..] if u == CONSOLE), "{fx:?}");
    // The job's own link, from its context.
    down(&mut host, JOB);
    let fx = press(&mut host, KeyCode::Char('o'));
    assert!(matches!(&fx[..], [HostEffect::Open(u)] if u == "https://ci.acme.example/runs/9"), "{fx:?}");
    // An alert with no link says so and opens nothing.
    let mut p = sample();
    p.data.channels.get_mut(ALRT).unwrap().messages.push(bot(ago(100), vec![card("danger", "[FIRING] NoLink", "")]));
    let mut bare = hosting(p);
    on_alerts(&mut bare);
    down(&mut bare, 2);
    let fx = press(&mut bare, KeyCode::Char('o'));
    assert!(fx.is_empty(), "{fx:?}");
    assert!(bare.message.as_deref().is_some_and(|m| m.contains("no console link")), "{:?}", bare.message);
    // Enter: its thread's page, with the people who replied.
    let mut host = hosted();
    on_alerts(&mut host);
    press(&mut host, KeyCode::Enter);
    let text = shown(&host, 160, 40);
    assert!(text.contains("Slack / #team-alerts / alertbot: bot message") && text.contains("looking at it"), "{text}");
    press(&mut host, KeyCode::Esc);
    assert!(shown(&host, 160, 40).contains("api-HighErrorRate"), "back on the alerts");
}

#[test]
fn an_unmapped_alert_is_mapped_in_one_key_by_typing_a_repo_or_a_service() {
    let mut host = hosted();
    on_alerts(&mut host);
    down(&mut host, DISK);
    press(&mut host, KeyCode::Char('m'));
    assert!(host.plugins[0].capturing());
    let text = shown(&host, 160, 40);
    assert!(text.contains("DiskFull belongs to (a repo path, or a service)"), "{text}");
    // Nothing typed: refused with what it takes.
    press(&mut host, KeyCode::Enter);
    assert!(host.message.as_deref().is_some_and(|m| m.starts_with("not yet: a repo's path")), "{:?}", host.message);
    chars(&mut host, "acme/team-a/disk");
    press(&mut host, KeyCode::Enter);
    assert!(!host.plugins[0].capturing());
    let text = shown(&host, 160, 40);
    assert!(text.contains("disk  prod") && text.contains(":alert_firing: ▲ DiskFull"), "{text}");
    let saved = host.plugins[0].config().unwrap().to_string();
    assert!(saved.contains("prefix = \"DiskFull\"") && saved.contains("repo = \"acme/team-a/disk\""), "{saved}");
    // A mapped alert does not offer it.
    let _ = shown(&host, 160, 40);
    press(&mut host, KeyCode::Char('x'));
    assert!(!menu_labels(&host).iter().any(|l| l.contains("map it")), "{:?}", menu_labels(&host));
    // Escape drops it.
    let mut host = hosted();
    on_alerts(&mut host);
    down(&mut host, DISK);
    press(&mut host, KeyCode::Char('m'));
    chars(&mut host, "billing");
    press(&mut host, KeyCode::Esc);
    assert!(!host.plugins[0].capturing());
    assert!(shown(&host, 160, 40).contains("unmapped  prod"));
}

#[test]
fn what_no_reader_could_read_is_counted_in_the_rail_and_said_in_the_log_once() {
    let mut host = hosted();
    on_alerts(&mut host);
    let text = shown(&host, 160, 40);
    assert!(text.contains("1 bot message not read"), "the nav pane footer says so: {text}");
    let mut p = sample();
    assert_eq!(p.alerts().refused.len(), 1);
    assert!(p.alerts().refused[0].2.contains("no [FIRING] or [RESOLVED]"));
    // Said once a run, however often it is read.
    p.log_refusals();
    assert_eq!(p.logged_refusals.len(), 1);
    p.log_refusals();
    assert_eq!(p.logged_refusals.len(), 1);
    // A channel with no reader says so instead of reading nothing silently.
    let mut p = sample();
    let at = p.cfg.channels.iter().position(|c| c.alias == "team-alerts").unwrap();
    p.cfg.channels[at].reader = None;
    p.changed();
    assert_eq!(p.alerts().without_reader, ["team-alerts"]);
    let mut host = hosting(p);
    on_alerts(&mut host);
    assert!(shown(&host, 160, 40).contains("#team-alerts has no"));
}

#[test]
fn an_alerts_channel_is_not_a_discussion_and_the_x_menu_of_an_alert_lists_what_works() {
    let mut host = hosted();
    keys(&mut host, &[KeyCode::Char('1'), KeyCode::Down, KeyCode::Char('2')]);
    let text = shown(&host, 160, 40);
    assert!(!text.contains("HighErrorRate") && !text.contains("who is on the disk alert"), "{text}");
    let mut host = hosted();
    on_alerts(&mut host);
    let _ = shown(&host, 160, 40);
    press(&mut host, KeyCode::Char('x'));
    let labels = menu_labels(&host);
    assert_eq!(host.menu.as_ref().unwrap().menu.title, "api-HighErrorRate");
    for want in ["⏎ open its thread", "o open its console link", "y copy its console link"] {
        assert!(labels.iter().any(|l| l.starts_with(want)), "{want}: {labels:?}");
    }
    assert!(!labels.iter().any(|l| l.contains("map it")), "it is mapped already: {labels:?}");
    insta::assert_snapshot!("alert_keys", menu_shown(&host));
    press(&mut host, KeyCode::Esc);
    down(&mut host, DISK);
    press(&mut host, KeyCode::Char('x'));
    assert!(menu_labels(&host).iter().any(|l| l.starts_with("m map it to a repo or service")), "{:?}", menu_labels(&host));
}

#[test]
fn nothing_is_sent_to_slack_from_the_alerts_view() {
    let mut host = hosted();
    on_alerts(&mut host);
    for k in ['o', 'y', 'm', 'e', 'c'] {
        let fx = press(&mut host, KeyCode::Char(k));
        assert!(!fx.iter().any(|e| matches!(e, HostEffect::Spawn { .. })), "{k}: {fx:?}");
        if host.plugins[0].capturing() {
            press(&mut host, KeyCode::Esc);
        }
        host.menu = None;
    }
}

#[test]
fn every_row_and_target_is_taken_on_the_alerts_view_and_its_threads() {
    // The nav pane's Alerts row and those around it (the rest are the feed's
    // own check, with the settings sections).
    let checked = every_row(sample, &slack_places_of(3..6, 6, false));
    assert!(checked > 100, "only {checked} checked");
}

// --- settings: channels, the mapping ------------------------------------------------------------

fn settings_of(section: usize) -> Host {
    let mut host = hosted();
    press(&mut host, KeyCode::Char(radar_core::host::SETTINGS_KEY));
    let at = host.sections.iter().position(|(_, s)| s.group == "slack").unwrap();
    host.section = at + section;
    host.focus = radar_core::Pane::Middle;
    host
}

#[test]
fn the_alerts_section_lists_the_mapping_and_the_alerts_no_rule_places() {
    let host = settings_of(3);
    let text = shown(&host, 160, 40);
    for s in [
        "keep resolved",
        "+ add a mapping",
        "api-… → repo acme/team-a/api · service api",
        "name starts",
        "no rule places",
        "DiskFull",
        "Nightly export",
    ] {
        assert!(text.contains(s), "{s}:\n{text}");
    }
    insta::assert_snapshot!("settings_alerts", text);
}

#[test]
fn a_mapping_is_added_by_typing_a_name_then_where_it_belongs_and_is_kept() {
    let mut host = settings_of(3);
    // The first row is add a mapping.
    press(&mut host, KeyCode::Enter);
    assert!(host.plugins[0].capturing());
    // Nothing yet, and spaces, are refused with a reason.
    press(&mut host, KeyCode::Enter);
    assert!(host.message.as_deref().is_some_and(|m| m.starts_with("not yet: the start of the alert's name")), "{:?}", host.message);
    chars(&mut host, "db-");
    press(&mut host, KeyCode::Enter);
    // Where it belongs is asked straight away.
    assert!(host.plugins[0].capturing());
    chars(&mut host, "https://git.example/acme/team-a/db/-/merge_requests/1");
    press(&mut host, KeyCode::Enter);
    assert!(!host.plugins[0].capturing());
    let saved = host.plugins[0].config().unwrap().to_string();
    assert!(saved.contains("prefix = \"db-\"") && saved.contains("repo = \"acme/team-a/db\""), "{saved}");
    let text = shown(&host, 160, 40);
    assert!(text.contains("db-… → repo acme/team-a/db"), "{text}");
}

#[test]
fn an_unmapped_alert_listed_in_settings_is_mapped_from_its_row() {
    let mut host = settings_of(3);
    // Hours, add, the rule and its five fields: the unmapped names follow.
    for _ in 0..8 {
        press(&mut host, KeyCode::Down);
    }
    let text = shown(&host, 160, 40);
    assert!(text.contains("No rule places alerts named DiskFull"), "{text}");
    press(&mut host, KeyCode::Enter);
    assert!(host.plugins[0].capturing());
    chars(&mut host, "two words");
    press(&mut host, KeyCode::Enter);
    assert!(host.plugins[0].capturing(), "refused while it has spaces");
    let _ = shown(&host, 160, 40);
    for _ in 0..9 {
        press(&mut host, KeyCode::Backspace);
    }
    chars(&mut host, "disks");
    press(&mut host, KeyCode::Enter);
    assert!(!host.plugins[0].capturing());
    let saved = host.plugins[0].config().unwrap().to_string();
    assert!(saved.contains("prefix = \"DiskFull\"") && saved.contains("service = \"disks\""), "{saved}");
    assert!(!shown(&host, 160, 40).contains("No rule places alerts named DiskFull"));
}

#[test]
fn the_hours_resolved_alerts_stay_are_typed_and_checked() {
    let mut host = settings_of(3);
    press(&mut host, KeyCode::Down); // keep resolved, under add a mapping
    press(&mut host, KeyCode::Enter);
    assert!(host.plugins[0].capturing());
    for _ in 0..3 {
        press(&mut host, KeyCode::Backspace);
    }
    chars(&mut host, "0");
    press(&mut host, KeyCode::Enter);
    assert!(host.plugins[0].capturing(), "0 hours is refused");
    press(&mut host, KeyCode::Backspace);
    chars(&mut host, "6");
    press(&mut host, KeyCode::Enter);
    assert!(!host.plugins[0].capturing());
    assert!(host.plugins[0].config().unwrap().to_string().contains("recent_hours = 6"));
    // Six hours: the resolved one (5h 20m ago) is still there; at 5 it goes.
    let mut p = sample();
    p.cfg.alerts.recent_hours = 5;
    let mut host = hosting(p);
    on_alerts(&mut host);
    assert!(!shown(&host, 160, 40).contains("api-LatencyHigh"));
}

#[test]
fn a_channel_made_an_alerts_channel_gets_the_usual_reader_and_leaving_drops_it() {
    let mut host = hosted();
    let _ = shown(&host, 160, 40);
    // The team channel is the first in the registry.
    let fx = host.plugins[0].on_menu("kind:0", "alerts");
    assert!(fx.is_empty());
    let saved = host.plugins[0].config().unwrap().to_string();
    assert!(saved.contains("kind = \"alerts\"") && saved.contains("reader = \"alertmanager\""), "{saved}");
    host.plugins[0].on_menu("reader:0", "status");
    assert!(host.plugins[0].config().unwrap().to_string().contains("reader = \"status\""));
    host.plugins[0].on_menu("kind:0", "team");
    let saved = host.plugins[0].config().unwrap().to_string();
    assert!(!saved.contains("reader = \"status\""), "{saved}");
}

#[test]
fn an_alerts_channel_shows_its_reader_row_and_the_kind_menu_offers_alerts() {
    let mut host = settings_of(1);
    let text = shown(&host, 160, 40);
    assert!(text.contains("reader") && text.contains("alertmanager") && text.contains("job-failed"), "{text}");
    // Down from add: #team, its fields; the kind row offers alerts.
    for _ in 0..4 {
        press(&mut host, KeyCode::Down);
    }
    press(&mut host, KeyCode::Enter);
    let labels = menu_labels(&host);
    assert!(labels.iter().any(|l| l.contains("alerts")), "{labels:?}");
}

// --- what Slack adds to repos --------------------------------------------------------------------

fn repo(path: &str) -> radar_core::Subject {
    radar_core::Subject::new("repo", path, path.rsplit('/').next().unwrap()).with("project", path)
}

#[test]
fn a_repo_gets_a_firing_marker_and_an_alerts_tab_from_the_alerts_the_rules_place_on_it() {
    use radar_core::{Contribution, Plugin, Point};
    let p = sample();
    let Some(Contribution::Chips(chips)) = p.contribute(Point::Chips, &repo("acme/team-a/api")) else {
        panic!("a firing marker");
    };
    assert_eq!(chips.len(), 1);
    assert_eq!(chips[0].text, "firing · 1");
    assert_eq!(chips[0].tone, radar_core::Tone::Bad);
    // Firing first, then what resolved lately; each opens its thread.
    let Some(Contribution::Tab { title, rows }) = p.contribute(Point::Tab, &repo("acme/team-a/api")) else {
        panic!("an Alerts tab");
    };
    assert_eq!(title, "Alerts");
    let lines: Vec<String> = rows.iter().map(|r| r.line.to_string()).collect();
    assert_eq!(
        lines,
        [
            "▲ api-HighErrorRate · prod · P1 · firing 3h 07m · #team-alerts",
            "✓ api-LatencyHigh · staging · P3 · resolved after 40m 00s · #team-alerts",
        ]
    );
    assert!(rows.iter().all(|r| r.subject.as_ref().is_some_and(|s| s.is("thread"))));
    // A repo no rule names, and the places that are not a repo's.
    assert!(p.contribute(Point::Chips, &repo("acme/team-a/web")).is_none());
    assert!(p.contribute(Point::Tab, &repo("acme/team-a/web")).is_none());
    assert!(p.contribute(Point::Column, &repo("acme/team-a/api")).is_none());
    // A rule naming the repo by its last part is enough.
    let mut p = sample();
    p.cfg.alerts.rules[0].repo = "api".into();
    p.changed();
    assert!(p.contribute(Point::Chips, &repo("acme/team-a/api")).is_some());
    // Only resolved lately: a tab, and no marker.
    let mut p = sample();
    p.data.channels.get_mut(ALRT).unwrap().messages.retain(|m| m.ts != ago(3600) && m.ts != ago(3 * 3600 + 7 * 60));
    p.changed();
    assert!(p.contribute(Point::Chips, &repo("acme/team-a/api")).is_none());
    assert!(p.contribute(Point::Tab, &repo("acme/team-a/api")).is_some());
}

#[test]
fn a_job_that_failed_is_marked_on_its_repo_apart_from_what_fires() {
    use radar_core::{Contribution, Plugin, Point};
    let mut p = sample();
    p.cfg.alerts.rules.push(Rule { prefix: "Nightly".into(), repo: "acme/team-a/export".into(), ..Rule::default() });
    p.changed();
    let Some(Contribution::Chips(chips)) = p.contribute(Point::Chips, &repo("acme/team-a/export")) else {
        panic!("a marker");
    };
    assert_eq!(chips.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(), ["job failed · 1"]);
}

#[test]
fn not_set_up_it_marks_no_repo_and_says_what_it_adds_in_the_parts_section() {
    use radar_core::{Plugin, Point};
    let p = SlackPlugin::new();
    assert!(p.contribute(Point::Chips, &repo("acme/team-a/api")).is_none());
    assert!(p.contribute(Point::Tab, &repo("acme/team-a/api")).is_none());
    assert!(!Plugin::adds(&p).is_empty());
}

#[test]
fn alerts_add_nothing_to_other_parts_here() {
    // Next to a code host and a tracker: nothing about alerts on merge
    // requests or tickets.
    let mut host = hosted_with_parts();
    host.plugins[2] = Box::new(sample());
    let text = shown(&host, 160, 40);
    assert!(!text.to_lowercase().contains("firing"), "{text}");
}
