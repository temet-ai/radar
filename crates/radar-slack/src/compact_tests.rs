//! The compact density through the real host: the thread page, the detail pane
//! and a direct message at three sizes, and the cozy layout kept exactly as
//! it was.

use radar_core::Host;
use radar_core::ratatui::crossterm::event::KeyCode;
use crate::tests::shown;

use crate::SlackPlugin;
use crate::client::{File, Reaction, Shared};
use crate::config::Density;
use crate::model::ChannelData;
use crate::tests::{ALICE, CAROL, MR_263, keys, msg, on_team, press, sample};

const DM: &str = "D0000CAROL";
/// 2026-10-05 00:00 UTC, a Monday.
const MON: i64 = 1_791_158_400;

fn at(day_secs: i64, h: i64, m: i64) -> String {
    format!("{}.{:06}", day_secs + h * 3600 + m * 60, 0)
}

fn cozy(mut p: SlackPlugin) -> SlackPlugin {
    p.cfg.density = Density::Cozy;
    p
}

fn host_of(p: SlackPlugin) -> Host {
    let mut host = Host::new(vec![Box::new(SlackPlugin::new())], crate::tests::NOW).unwrap();
    host.load_config(Some("")).unwrap();
    host.plugins[0] = Box::new(p);
    host
}

/// A direct message with Carol: she writes twice in a minute, Alice answers
/// (a long reply with a reaction), the next morning Carol sends two files and
/// Alice shares a message. The first is the thread's parent, the rest its
/// replies, so the page shows it as a conversation.
fn chatty() -> SlackPlugin {
    let mut p = sample();
    let first = at(MON, 17, 58);
    let mut parent = msg(&first, CAROL, &format!("hi <@{ALICE}>, quick question about the stats"));
    parent.reply_count = 5;
    // The same direct message For you holds, as it was said first.
    p.data.for_you[0].message = parent.clone();
    let second = msg(&at(MON, 17, 59), CAROL, "the dashboard shows 0 for the weekly numbers");
    let third = msg(&at(MON, 18, 10), ALICE, "on it, looking now :eyes:");
    let mut fourth = msg(
        &at(MON, 18, 12),
        ALICE,
        "found it: the query groups by ISO week but the cache keys by calendar week, so they disagree on the first of the year. I will fix the key and add a test that crosses a year boundary. Back in ten minutes",
    );
    fourth.reactions = vec![Reaction { name: "+1".into(), count: 1, users: vec![CAROL.into()] }];
    let mut fifth = msg(&at(MON + 86_400, 6, 30), CAROL, "thanks!! :tada: the numbers are right again, see the notes");
    fifth.edited = true;
    fifth.files = vec![
        File { name: "notes.csv".into(), url: "https://acme.slack.com/files/U1/F2/notes.csv".into(), ..Default::default() },
        File { name: "Weekly.png".into(), url: "https://acme.slack.com/files/U1/F3/weekly.png".into(), image: true, ..Default::default() },
    ];
    let mut sixth = msg(&at(MON + 86_400, 6, 31), ALICE, "great, Bob wrote the original");
    sixth.shared = vec![Shared {
        author: "Bob Builder".into(),
        text: format!("the weekly job is in <{MR_263}|the api>, it keys the cache by calendar week for now and we should change that soon"),
        url: "https://acme.slack.com/archives/C0000TEAM1/p1791000000000100".into(),
    }];
    p.data.channels.insert(
        DM.into(),
        ChannelData {
            info: None,
            messages: vec![parent],
            replies: [(first, vec![second, third, fourth, fifth, sixth])].into_iter().collect(),
            ..Default::default()
        },
    );
    p
}

/// To the direct message in For you, whose detail pane says its words.
fn on_dm(host: &mut Host) {
    keys(host, &[KeyCode::Char('2')]);
    for _ in 0..6 {
        if shown(host, 160, 40).contains("direct message") && shown(host, 160, 40).contains("Carol Coder") {
            return;
        }
        press(host, KeyCode::Down);
    }
    panic!("the direct message was not reached: {}", shown(host, 160, 40));
}

fn dm_page(p: SlackPlugin) -> Host {
    let mut host = host_of(p);
    on_dm(&mut host);
    press(&mut host, KeyCode::Enter);
    host
}

fn thread_page(p: SlackPlugin) -> Host {
    let mut host = host_of(p);
    on_team(&mut host, 4);
    press(&mut host, KeyCode::Enter);
    host
}

#[test]
fn the_thread_page_is_a_few_lines_in_compact() {
    let host = thread_page(sample());
    // The one line a message is wrapped to the room it is asked for.
    for (w, h) in [(80, 24), (160, 40)] {
        let text = shown(&host, w, h);
        assert!(text.contains("06:46  Alice Example  looking"), "one line a message: {text}");
        assert!(text.contains("── 2 replies ──"), "{text}");
        insta::assert_snapshot!(format!("thread_page_compact_{w}x{h}"), text);
    }
}

#[test]
fn the_thread_page_in_cozy_is_as_it_was() {
    let host = thread_page(cozy(sample()));
    let text = shown(&host, 160, 40);
    assert!(text.contains(":you: Alice Example  2d ago"), "{text}");
    insta::assert_snapshot!("thread_page_cozy", text);
}

#[test]
fn the_preview_of_a_message_is_compact_and_in_cozy_as_it_was() {
    let mut host = host_of(sample());
    // Focus on the detail pane, so it is the pane shown where the screen is narrow.
    on_team(&mut host, 4);
    keys(&mut host, &[KeyCode::Char('3')]);
    let wide = shown(&host, 160, 40);
    for word in ["2 replies", "looking", "approved, nice one", "LINKS", "requests/263"] {
        assert!(wide.contains(word), "compact detail pane keeps {word}: {wide}");
    }
    insta::assert_snapshot!("preview_compact", wide);
    let mut host = host_of(cozy(sample()));
    on_team(&mut host, 4);
    keys(&mut host, &[KeyCode::Char('3')]);
    insta::assert_snapshot!("preview_cozy_100x40", shown(&host, 100, 40));
}

#[test]
fn a_direct_message_is_a_conversation_of_single_lines() {
    let host = dm_page(chatty());
    for (w, h) in [(80, 24), (100, 40), (160, 40)] {
        let text = shown(&host, w, h);
        assert!(text.contains("── 5 replies ──"), "{text}");
        insta::assert_snapshot!(format!("dm_page_compact_{w}x{h}"), text);
    }
    let text = shown(&host, 160, 40);
    assert!(text.contains("17:58  Carol Coder    hi @Alice Example, quick question"), "{text}");
    // Alice's second message, two minutes after her first, leaves her name
    // blank; the time stays. Its reaction ends its last line.
    assert!(text.contains("18:10  Alice Example  on it, looking now"), "{text}");
    assert!(text.contains("18:12                 found it: the query groups"), "{text}");
    assert!(text.contains("Back in ten minutes  👍 1"), "{text}");
    // (edited) ends Carol's words, her files hang under them.
    assert!(text.contains("see the notes  (edited)"), "{text}");
    // A day divider where the date changes; files and the shared message
    // one hanging line each, by name and by author.
    assert!(text.contains("── yesterday ──") && text.contains("── today ──"), "{text}");
    assert!(text.contains("▣ Weekly.png") && text.contains("▤ notes.csv"), "{text}");
    assert!(text.contains("│ Bob Builder: the weekly job is in the api"), "{text}");
    assert!(!text.contains("slack.com"), "no raw link: {text}");
}

#[test]
fn a_threads_files_and_a_shared_message_in_cozy_are_as_they_were() {
    let mut host = host_of(cozy(crate::tests::with_files()));
    on_team(&mut host, 4);
    press(&mut host, KeyCode::Enter);
    let text = shown(&host, 160, 40);
    assert!(text.contains("> shared: Carol Coder's message") && text.contains("▣ image Screenshot.png https://"), "{text}");
    insta::assert_snapshot!("thread_files_cozy_160x40", text);
}

#[test]
fn a_direct_message_in_cozy_is_as_it_was() {
    let host = dm_page(cozy(chatty()));
    let text = shown(&host, 160, 40);
    assert!(text.contains("Carol Coder  13h ago"), "{text}");
    insta::assert_snapshot!("dm_page_cozy_160x40", text);
}

#[test]
fn the_preview_of_a_direct_message_is_compact() {
    let mut host = host_of(chatty());
    on_dm(&mut host);
    keys(&mut host, &[KeyCode::Char('3')]);
    insta::assert_snapshot!("dm_preview_compact", shown(&host, 160, 40));
}

#[test]
fn density_is_a_row_in_message_look_and_switches_the_layout() {
    let mut p = sample();
    assert_eq!(p.cfg.density, Density::Compact);
    let toml = toml::to_string(&p.cfg).unwrap();
    assert!(!toml.contains("density"), "compact is the default, so it is not written: {toml}");
    p.cfg.density = Density::Cozy;
    assert!(toml::to_string(&p.cfg).unwrap().contains("density = \"cozy\""));
    let back: crate::config::Settings = toml::from_str("density = \"cozy\"").unwrap();
    assert_eq!(back.density, Density::Cozy);
    assert_eq!(toml::from_str::<crate::config::Settings>("").unwrap().density, Density::Compact);
}
