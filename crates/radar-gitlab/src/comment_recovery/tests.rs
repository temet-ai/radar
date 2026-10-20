//! Reducer and real-Host coverage. Every Spawn is left unpolled; no providers.
use super::*;
use crate::write::{self, LineAt};
use radar_core::host::{Event, HostEffect, JournalRecord, View};
use radar_core::plugin::Pane;
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use radar_core::{Host, Plugin, writes};

const PROJECT: &str = "acme/team-a/api";
const IID: u64 = 263;
const FIRST: &str = "PRIVATE_α  \n日本語\n  \n";
const SECOND: &str = "PRIVATE_β  \n🙂 tail \t\n  ";

struct ModeGuard((writes::Mode, Option<String>));
impl ModeGuard {
    fn new(mode: writes::Mode) -> Self {
        let old = writes::mode();
        writes::set_mode(mode, None);
        Self(old)
    }
}
impl Drop for ModeGuard {
    fn drop(&mut self) {
        writes::set_mode(self.0.0, self.0.1.take());
    }
}
fn key(c: char, control: bool) -> KeyEvent {
    KeyEvent::new(
        KeyCode::Char(c),
        if control {
            KeyModifiers::CONTROL
        } else {
            KeyModifiers::NONE
        },
    )
}
fn menu(fx: &[Effect]) -> &Menu {
    fx.iter()
        .find_map(|e| {
            if let Effect::Menu(m) = e {
                Some(m)
            } else {
                None
            }
        })
        .expect("menu")
}
fn choose(p: &mut GitlabPlugin, fx: &[Effect], value: &str) -> Vec<Effect> {
    p.on_menu(&menu(fx).id, value)
}
fn spawned(fx: &[Effect]) -> bool {
    fx.iter()
        .any(|e| matches!(e, Effect::Spawn(_) | Effect::SpawnAction { .. }))
}
fn begin(p: &mut GitlabPlugin, target: Target, text: &str) {
    let fx = write::start(p, PROJECT, IID, target);
    if !fx.is_empty() {
        choose(p, &fx, "new");
    }
    assert!(p.compose.is_some());
    p.paste(text);
    write::compose_key(p, key('s', true));
}
fn sent(p: &mut GitlabPlugin) -> u64 {
    let change = p.change.clone().unwrap();
    let fx = p.on_menu("gl-write", "yes");
    assert!(spawned(&fx));
    p.recovery.test_operation(&change)
}
fn answer(p: &GitlabPlugin, operation: u64, success: bool) -> WroteMsg {
    let (connection, change) = p.recovery.active[&operation].clone();
    WroteMsg {
        operation,
        connection,
        change,
        result: if success {
            Ok("PRIVATE_POSTED".into())
        } else {
            Err("PRIVATE_ERROR".into())
        },
    }
}
fn decline(p: &mut GitlabPlugin) -> u64 {
    let fx = p.on_menu("gl-write", "no");
    assert!(!spawned(&fx));
    assert!(p.compose.is_none());
    *p.recovery.records.last_key_value().unwrap().0
}
fn copy(p: &mut GitlabPlugin, id: u64) -> String {
    let fx = list(p, None);
    let fx = choose(p, &fx, &format!("entry:{id}"));
    let fx = choose(p, &fx, "copy");
    fx.into_iter()
        .find_map(|e| {
            if let Effect::Copy(t) = e {
                Some(t)
            } else {
                None
            }
        })
        .expect("copy")
}
fn restore(p: &mut GitlabPlugin, id: u64, target: Target) -> Vec<Effect> {
    let fx = write::start(p, PROJECT, IID, target);
    let fx = choose(p, &fx, &format!("entry:{id}"));
    choose(p, &fx, "edit")
}
fn targets() -> Vec<Target> {
    vec![
        Target::Note,
        Target::Line(LineAt {
            new_path: "PRIVATE_new.rs".into(),
            old_path: "PRIVATE_old.rs".into(),
            old: Some(12),
            new: Some(14),
            refs: crate::model::DiffRefs {
                base: "base".into(),
                head: "head".into(),
                start: "start".into(),
            },
        }),
        Target::Reply {
            discussion: "ab12".into(),
            place: "PRIVATE_file:12".into(),
        },
    ]
}

#[test]
fn failure_and_decline_in_both_orders_keep_distinct_exact_occurrences() {
    let _mode = ModeGuard::new(writes::Mode::Edit);
    for target in targets() {
        for failure_first in [true, false] {
            let mut p = crate::tests::sample();
            begin(&mut p, target.clone(), FIRST);
            let first = sent(&mut p);
            let completion = answer(&p, first, false);
            begin(&mut p, target.clone(), SECOND);
            if failure_first {
                write::on_wrote(&mut p, completion);
            }
            let second = decline(&mut p);
            if !failure_first {
                let completion = answer(&p, first, false);
                write::on_wrote(&mut p, completion);
            }
            assert_ne!(first, second);
            assert_eq!(p.recovery.usage(), (2, FIRST.len() + SECOND.len()));
            assert_eq!(copy(&mut p, first), FIRST);
            assert_eq!(copy(&mut p, second), SECOND);
            assert_eq!(p.recovery.records[&first].status, Status::Uncertain);
            assert_eq!(p.recovery.records[&second].status, Status::Unsent);
            let fx = write::start(&mut p, PROJECT, IID, target.clone());
            let values: Vec<_> = menu(&fx)
                .items
                .iter()
                .map(|item| item.value.as_str())
                .collect();
            assert!(
                values.contains(&"entry:1")
                    && values.contains(&"entry:2")
                    && values.contains(&"new")
            );
            assert!(p.compose.is_none(), "c offers choices first");
            let fx = choose(&mut p, &fx, &format!("entry:{first}"));
            assert!(
                menu(&fx)
                    .items
                    .iter()
                    .any(|i| i.label.contains("may already exist"))
            );
            choose(&mut p, &fx, "edit");
            assert_eq!(p.compose.as_ref().unwrap().text(), FIRST);
            assert_eq!(p.compose.as_ref().unwrap().target, target);
            assert_eq!(p.compose.as_ref().unwrap().status, Status::Uncertain);
            assert!(p.recovery.records.contains_key(&second));
            assert_eq!(p.recovery.usage(), (1, SECOND.len()));
        }
    }
}

#[test]
fn identical_text_is_not_deduplicated_and_explicit_drop_is_one_occurrence_once() {
    let mut p = crate::tests::sample();
    begin(&mut p, Target::Note, FIRST);
    let first = decline(&mut p);
    begin(&mut p, Target::Note, FIRST);
    let second = decline(&mut p);
    assert_eq!(p.recovery.usage(), (2, FIRST.len() * 2));
    let fx = list(&mut p, None);
    let fx = choose(&mut p, &fx, &format!("entry:{first}"));
    let fx = choose(&mut p, &fx, "drop");
    let drop_menu = menu(&fx).id.clone();
    p.on_menu(&drop_menu, "yes");
    assert_eq!(p.recovery.usage(), (1, FIRST.len()));
    assert_eq!(copy(&mut p, second), FIRST);
    let before = p.recovery.records.clone();
    p.on_menu(&drop_menu, "yes");
    assert_eq!(p.recovery.records, before);
}

#[test]
fn pending_reservation_remains_accessible_and_old_dialog_cannot_drop_after_completion() {
    let _mode = ModeGuard::new(writes::Mode::Edit);
    let mut p = crate::tests::sample();
    begin(&mut p, Target::Note, FIRST);
    let id = sent(&mut p);
    assert_eq!(copy(&mut p, id), FIRST);
    let fx = list(&mut p, None);
    let fx = choose(&mut p, &fx, &format!("entry:{id}"));
    let pending = menu(&fx).clone();
    assert!(pending.items.iter().any(|i| i.label.contains("pending")));
    assert!(
        !pending
            .items
            .iter()
            .any(|i| matches!(i.value.as_str(), "drop" | "edit"))
    );
    let completion = answer(&p, id, false);
    write::on_wrote(&mut p, completion);
    p.on_menu(&pending.id, "drop");
    assert_eq!(p.recovery.records[&id].status, Status::Uncertain);
    assert_eq!(copy(&mut p, id), FIRST);
}

#[test]
fn duplicate_and_unmatched_completions_preserve_other_operations_and_original_outcomes() {
    let _mode = ModeGuard::new(writes::Mode::Edit);
    for success in [true, false] {
        let mut p = crate::tests::sample();
        begin(&mut p, Target::Note, FIRST);
        let a = sent(&mut p);
        let duplicate = answer(&p, a, success);
        let completion = answer(&p, a, success);
        p.on_msg(Box::new(completion));
        begin(&mut p, Target::Note, SECOND);
        let b = sent(&mut p);
        let before = p.recovery.records.clone();
        let acting = format!("{:?}", p.acting);
        let mut mismatch = answer(&p, b, false);
        mismatch.change = Change::Note {
            project: PROJECT.into(),
            iid: IID,
            text: "different".into(),
        };
        for message in [duplicate, mismatch] {
            let outcome = if message.result.is_ok() {
                Outcome::Succeeded
            } else {
                Outcome::Unknown
            };
            let fx = p.on_msg(Box::new(message));
            assert!(matches!(fx[0], Effect::Report(got) if got == outcome));
            assert!(fx.iter().any(|e| matches!(e, Effect::Observe { action, outcome: Outcome::Blocked } if action.kind.as_str() == "gitlab.write.completion.skipped")));
            assert!(!spawned(&fx));
            assert_eq!(p.recovery.records, before);
            assert_eq!(format!("{:?}", p.acting), acting);
            assert!(p.recovery.active.contains_key(&b));
        }
    }
}

#[test]
fn entry_and_utf8_byte_capacity_refuse_atomically_including_pending_slots() {
    let _mode = ModeGuard::new(writes::Mode::Edit);
    for by_bytes in [false, true] {
        let mut p = crate::tests::sample();
        if !by_bytes {
            for _ in 0..MAX_RECORDS - 1 {
                p.recovery.test_keep("saved");
            }
        }
        let first = if by_bytes {
            "é".repeat(MAX_BYTES / 2)
        } else {
            FIRST.into()
        };
        begin(&mut p, Target::Note, &first);
        let id = sent(&mut p);
        begin(&mut p, Target::Note, SECOND);
        let before = p.recovery.records.clone();
        let change = p.change.clone();
        for decision in ["no", "yes"] {
            let fx = p.on_menu("gl-write", decision);
            if decision == "no" {
                assert!(matches!(fx[0], Effect::Report(Outcome::Cancelled)));
            }
            assert!(!spawned(&fx));
            assert_eq!(p.compose.as_ref().unwrap().text(), SECOND);
            assert_eq!(p.change, change);
            assert_eq!(p.recovery.records, before);
        }
        let completion = answer(&p, id, false);
        write::on_wrote(&mut p, completion);
        assert_eq!(p.recovery.records[&id].original, first);
        assert_eq!(p.recovery.records[&id].status, Status::Uncertain);
        let usage = p.recovery.usage();
        let fx = p.on_menu("gl-write", "yes");
        assert!(!spawned(&fx));
        assert!(fx.iter().any(|e| matches!(e, Effect::Observe { action, .. } if action.kind.as_str() == "gitlab.comment.recovery.full")));
        assert_eq!(p.recovery.usage(), usage);
        assert_eq!(p.compose.as_ref().unwrap().text(), SECOND);
        assert_eq!(p.change, change);
        let fx = list(&mut p, None);
        let fx = choose(&mut p, &fx, &format!("entry:{id}"));
        let fx = choose(&mut p, &fx, "drop");
        choose(&mut p, &fx, "yes");
        let fx = p.on_menu("gl-write", "no");
        assert!(!spawned(&fx));
        assert!(p.compose.is_none());
        assert!(p.recovery.records.values().any(|r| r.original == SECOND));
    }
}

#[test]
fn oversized_editor_and_restore_at_capacity_never_lose_text() {
    let _mode = ModeGuard::new(writes::Mode::Edit);
    let mut p = crate::tests::sample();
    let oversized = format!("{}a", "🙂".repeat(MAX_BYTES / 4));
    begin(&mut p, Target::Note, &oversized);
    for value in ["yes", "no"] {
        assert!(!spawned(&p.on_menu("gl-write", value)));
        assert_eq!(p.compose.as_ref().unwrap().text(), oversized);
        assert_eq!(p.recovery.usage(), (0, 0));
    }
    p.compose = None;
    p.change = None;
    p.review = None;
    begin(&mut p, Target::Note, FIRST);
    let id = decline(&mut p);
    for _ in 1..MAX_RECORDS {
        p.recovery.test_keep("saved");
    }
    assert_eq!(p.recovery.usage().0, MAX_RECORDS);
    restore(&mut p, id, Target::Note);
    assert_eq!(p.compose.as_ref().unwrap().text(), FIRST);
    assert_eq!(p.recovery.usage().0, MAX_RECORDS - 1);
    write::compose_key(&mut p, key('s', true));
    decline(&mut p);
    assert_eq!(p.recovery.usage().0, MAX_RECORDS);
}

#[test]
fn uncertain_status_survives_edit_decline_and_mode_refusal_without_retry() {
    let _mode = ModeGuard::new(writes::Mode::Edit);
    let mut p = crate::tests::sample();
    begin(&mut p, Target::Note, FIRST);
    let id = sent(&mut p);
    let completion = answer(&p, id, false);
    write::on_wrote(&mut p, completion);
    restore(&mut p, id, Target::Note);
    p.paste("edited ");
    let radar_core::plugin::Outcome::Handled(fx) = write::compose_key(&mut p, key('s', true)) else {
        panic!()
    };
    assert!(
        menu(&fx)
            .items
            .iter()
            .any(|i| i.label.contains("may already exist"))
    );
    let id = decline(&mut p);
    assert_eq!(p.recovery.records[&id].status, Status::Uncertain);
    restore(&mut p, id, Target::Note);
    write::compose_key(&mut p, key('s', true));
    writes::set_mode(writes::Mode::ReadOnly, None);
    let fx = p.on_menu("gl-write", "yes");
    assert!(!spawned(&fx));
    assert!(matches!(fx.last(), Some(Effect::Notice(message)) if message.tone == radar_core::view::chrome::NoticeTone::Error));
    let r = p.recovery.records.values().next().unwrap();
    assert_eq!(r.status, Status::Uncertain);
    assert_eq!(r.original, format!("{FIRST}edited "));
    assert!(p.recovery.active.is_empty());
}

#[test]
fn frozen_full_target_and_original_connection_prevent_silent_rebinding() {
    let mut p = crate::tests::sample();
    p.cfg.url = "https://user:PRIVATE_PASSWORD@git.example:8443/PRIVATE_PATH?PRIVATE_QUERY#PRIVATE_FRAGMENT".into();
    let original_target = targets().remove(1);
    begin(&mut p, original_target.clone(), FIRST);
    let id = decline(&mut p);
    let record = p.recovery.records[&id].clone();
    assert_eq!(record.source, "https://git.example:8443");
    let Target::Line(mut different) = original_target.clone() else {
        panic!()
    };
    different.refs.head = "new-head".into();
    write::start(&mut p, PROJECT, IID, Target::Line(different));
    assert_eq!(
        p.compose.as_ref().unwrap().text(),
        "",
        "new diff head is a distinct target"
    );
    assert_eq!(p.recovery.records[&id], record);
    p.compose = None;
    let fx = write::start(&mut p, PROJECT, IID, original_target.clone());
    let fx = choose(&mut p, &fx, &format!("entry:{id}"));
    p.replace_connection("https://other.example".into(), p.cfg.token.clone());
    choose(&mut p, &fx, "edit");
    assert!(p.compose.is_none());
    assert_eq!(p.recovery.records[&id], record);
    let fx = write::start(&mut p, PROJECT, IID, original_target);
    let fx = choose(&mut p, &fx, &format!("entry:{id}"));
    assert!(!menu(&fx).items.iter().any(|i| i.value == "edit"));
    assert!(
        menu(&fx)
            .items
            .iter()
            .any(|i| i.label.contains("earlier connection"))
    );
    assert_eq!(copy(&mut p, id), FIRST);
    let fx = list(&mut p, None);
    let fx = choose(&mut p, &fx, &format!("entry:{id}"));
    let fx = choose(&mut p, &fx, "drop");
    choose(&mut p, &fx, "yes");
    assert!(
        p.recovery.records.is_empty(),
        "old-connection copies can be explicitly dropped"
    );
}

#[test]
fn other_editors_block_restore_and_start_without_changing_keyboard_or_paste_target() {
    let mut p = crate::tests::sample();
    begin(&mut p, Target::Note, FIRST);
    let id = decline(&mut p);
    for editor in 0..4 {
        let fx = write::start(&mut p, PROJECT, IID, Target::Note);
        let fx = choose(&mut p, &fx, &format!("entry:{id}"));
        let input = radar_core::input::TextInput::new("before");
        match editor {
            0 => p.filter_input = Some(input),
            1 => p.edit = Some((crate::Target::Conn(crate::settings::Field::Base), input)),
            2 => p.log_search = Some(input),
            _ => {
                crate::goto::start(&mut p);
            }
        }
        choose(&mut p, &fx, "edit");
        assert!(p.compose.is_none());
        write::start(&mut p, PROJECT, IID, Target::Note);
        assert!(p.compose.is_none());
        p.paste("_paste");
        assert_eq!(p.recovery.records[&id].original, FIRST);
        match editor {
            0 => assert_eq!(p.filter_input.as_ref().unwrap().value(), "before_paste"),
            1 => assert_eq!(p.edit.as_ref().unwrap().1.value(), "before_paste"),
            2 => assert_eq!(p.log_search.as_ref().unwrap().value(), "before_paste"),
            _ => assert!(p.goto.is_some()),
        }
        p.filter_input = None;
        p.edit = None;
        p.log_search = None;
        p.goto = None;
    }
}

fn host_key(host: &mut Host, c: char, control: bool) -> Vec<HostEffect> {
    host.update(Event::Key(key(c, control)))
}
fn host_copy(fx: &[HostEffect]) -> Option<&str> {
    fx.iter().find_map(|e| match e {
        HostEffect::Copy(t) => Some(t.as_str()),
        HostEffect::Tracked { effect, .. } => host_copy(std::slice::from_ref(effect)),
        _ => None,
    })
}
fn journal(fx: &[HostEffect]) -> String {
    let records: Vec<&JournalRecord> = fx
        .iter()
        .filter_map(|e| {
            if let HostEffect::Journal(r) = e {
                Some(r)
            } else {
                None
            }
        })
        .collect();
    format!("{records:?}")
}
fn host(p: GitlabPlugin, settings: bool) -> Host {
    let mut host = Host::new(vec![Box::new(p)], 1).unwrap();
    host.view = if settings {
        View::Settings
    } else {
        View::Screen(0)
    };
    host.focus = Pane::Middle;
    host.section = host
        .sections
        .iter()
        .position(|(_, s)| s.group == "gitlab" && s.id == "connection")
        .unwrap();
    host.enable_journal();
    host
}

#[test]
fn real_host_exposes_offline_recovery_with_no_hidden_composer_or_journal_text() {
    let mut p = crate::tests::sample();
    begin(&mut p, Target::Note, FIRST);
    decline(&mut p);
    p.replace_connection("https://other.example".into(), String::new());
    p.cfg.sources.clear();
    p.pages.clear();
    p.mr_pages.clear();
    p.data.clear();
    let cached = p.snapshot();
    assert!(!format!("{cached:?}").contains("PRIVATE_"));
    let mut host = host(p, false);
    host_key(&mut host, ',', false);
    assert_eq!(host.view, View::Settings);
    host.focus = Pane::Middle;
    let mut trace = journal(&host_key(&mut host, 'r', true));
    assert!(
        host.menu
            .as_ref()
            .unwrap()
            .menu
            .title
            .contains("kept comments")
    );
    trace.push_str(&journal(&host.update(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )))));
    assert!(
        !host
            .menu
            .as_ref()
            .unwrap()
            .menu
            .items
            .iter()
            .any(|i| i.value == "edit")
    );
    let text = crate::tests::shown(&host, 160, 40);
    assert!(text.contains("PRIVATE_α"), "{text}");
    assert!(text.contains("earlier connection"), "{text}");
    let fx = host_key(&mut host, 'y', false);
    assert_eq!(host_copy(&fx), Some(FIRST));
    trace.push_str(&journal(&fx));
    assert!(!host.capturing());
    assert!(!trace.contains("PRIVATE_"));
    assert!(!trace.contains("entry:1"));
    host_key(&mut host, 'r', true);
    host.update(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    host_key(&mut host, 'd', false);
    let fx = host_key(&mut host, 'y', false);
    assert!(journal(&fx).contains("gitlab.comment.recovery.dropped"));
    assert!(!host.capturing());
}

#[test]
fn real_host_recovery_from_typing_and_settings_edit_preserves_text_and_paste() {
    for settings in [false, true] {
        let mut p = crate::tests::sample();
        begin(&mut p, Target::Note, FIRST);
        decline(&mut p);
        if settings {
            p.edit = Some((
                crate::Target::Conn(crate::settings::Field::Base),
                radar_core::input::TextInput::new("base"),
            ));
        } else {
            begin(&mut p, Target::Note, SECOND);
            p.on_menu("gl-write", "edit");
        }
        let mut host = host(p, settings);
        host_key(&mut host, 'r', true);
        host.update(Event::Paste("PRIVATE_IGNORED".into()));
        host.update(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert!(
            !host
                .menu
                .as_ref()
                .unwrap()
                .menu
                .items
                .iter()
                .any(|i| i.value == "edit")
        );
        let fx = host_key(&mut host, 'y', false);
        assert_eq!(host_copy(&fx), Some(FIRST));
        assert!(host.capturing());
        host_key(&mut host, if settings { 'a' } else { '!' }, false);
        host.update(Event::Paste("_paste".into()));
        if settings {
            host.update(Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )));
            assert_eq!(
                host.plugins[0].config().unwrap()["base"].as_str(),
                Some("basea_paste")
            );
        } else {
            host_key(&mut host, 's', true);
            let notes: Vec<_> = host
                .menu
                .as_ref()
                .unwrap()
                .menu
                .items
                .iter()
                .filter(|i| i.note)
                .map(|i| i.label.as_str())
                .collect();
            assert_eq!(notes.join("\n"), format!("{SECOND}!_paste"));
        }
    }
}

#[test]
fn stale_recovery_choice_cannot_replace_newer_composer_or_drop_another_record() {
    let mut p = crate::tests::sample();
    begin(&mut p, Target::Note, FIRST);
    let id = decline(&mut p);
    let fx = write::start(&mut p, PROJECT, IID, Target::Note);
    let fx = choose(&mut p, &fx, &format!("entry:{id}"));
    let entry_menu = menu(&fx).id.clone();
    begin(&mut p, Target::Note, SECOND);
    p.on_menu(&entry_menu, "edit");
    assert_eq!(p.compose.as_ref().unwrap().text(), SECOND);
    assert_eq!(p.recovery.records[&id].original, FIRST);
    let fx = list(&mut p, None);
    let stale = menu(&fx).id.clone();
    p.recovery.records.clear();
    list(&mut p, None); // current draft remains a copy option and replaces intent
    p.on_menu(&stale, &format!("entry:{id}"));
    assert_eq!(p.compose.as_ref().unwrap().text(), SECOND);
}
