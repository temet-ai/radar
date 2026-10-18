//! Recorded completions and unpolled task effects only. No credential resolution.
use super::*;
use crate::{JiraMsg, detail_key, model::IssueType, tests, write};
use radar_core::Host;
use radar_core::host::{Event, HostEffect, JournalRecord, View};
use radar_core::plugin::Pane;
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use radar_core::writes::{self, Mode};
use serde_json::Value;

const BODY: &str = "  SECRET_世界\n\n尾  \n ";
const SUMMARY: &str = "  SECRET_ résumé  ";
fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}
fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}
fn kind(id: &str) -> IssueType {
    IssueType {
        id: id.into(),
        name: format!("Type {id}"),
        subtask: false,
    }
}
fn sample() -> JiraPlugin {
    writes::set_mode(Mode::Write, None);
    crate::demo::sample()
}
fn choose(p: &mut JiraPlugin, value: &str) -> Vec<Effect> {
    let id = p.dialogs.menu_id();
    p.on_menu(&id, value)
}
fn comment(p: &mut JiraPlugin, site: &str, target: &str, text: &str) {
    let fx = write::start_comment(p, site, target);
    if fx.iter().any(|e| matches!(e, Effect::Menu(_))) {
        choose(p, "new");
    }
    write::compose_paste(p, text);
}
fn create(
    p: &mut JiraPlugin,
    site: &str,
    project: &str,
    type_id: &str,
    summary: &str,
    description: &str,
) {
    let context = Context::capture(p, site);
    write::open_compose(
        p,
        site,
        Draft::Summary {
            project: project.into(),
            kind: kind(type_id),
        },
        summary,
        context,
        Status::Unsent,
        String::new(),
    );
    write::compose_key(p, key(KeyCode::Enter));
    write::compose_paste(p, description);
}
fn review(p: &mut JiraPlugin) -> Change {
    write::compose_key(p, ctrl('s'));
    p.change.clone().unwrap()
}
fn dispatch(p: &mut JiraPlugin) -> (Change, Dispatch) {
    let change = review(p);
    assert!(
        choose(p, "yes")
            .iter()
            .any(|e| matches!(e, Effect::Spawn(_)))
    );
    let dispatch = p.sent[&detail_key(change.site(), change.about())]
        .dispatch
        .clone()
        .unwrap();
    (change, dispatch)
}
fn fail(p: &mut JiraPlugin, change: Change, dispatch: &Dispatch) -> Vec<Effect> {
    write::on_wrote(p, change, Err("SECRET_transport".into()), dispatch)
}
fn restore(p: &mut JiraPlugin, id: u64, destination: Destination) {
    list(p, Some(destination));
    choose(p, &format!("entry:{id}"));
    choose(p, "edit");
}
fn first(p: &JiraPlugin) -> &Record {
    p.recovery.records.values().next().unwrap()
}
fn no_spawn(fx: &[Effect]) {
    assert!(
        !fx.iter()
            .any(|e| matches!(e, Effect::Spawn(_) | Effect::SpawnAction { .. }))
    );
}

#[test]
fn both_completion_decline_orders_preserve_distinct_comment_and_create_occurrences() {
    for is_create in [false, true] {
        for fail_first in [false, true] {
            let mut p = sample();
            if is_create {
                create(&mut p, "acme", "ABC", "1", SUMMARY, BODY);
            } else {
                comment(&mut p, "acme", "ABC-12", BODY);
            }
            let original = p.compose.as_ref().unwrap().original();
            let (change, sent) = dispatch(&mut p);
            assert_eq!(first(&p).status, Status::Pending);
            if is_create {
                create(&mut p, "acme", "ABC", "1", SUMMARY, "B  \n");
            } else {
                comment(&mut p, "acme", "ABC-12", "B  \n");
            }
            let second = p.compose.as_ref().unwrap().original();
            review(&mut p);
            if fail_first {
                no_spawn(&fail(&mut p, change.clone(), &sent));
            }
            no_spawn(&choose(&mut p, "no"));
            if !fail_first {
                no_spawn(&fail(&mut p, change, &sent));
            }
            let rows: Vec<_> = p.recovery.records.values().collect();
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0].original, original);
            assert_eq!(rows[0].status, Status::Uncertain);
            assert_eq!(rows[1].original, second);
            assert_eq!(rows[1].status, Status::Unsent);
        }
    }
}

#[test]
fn identical_text_is_two_occurrences_and_pending_capacity_is_reserved() {
    let mut p = sample();
    comment(&mut p, "acme", "ABC-12", BODY);
    let (change, sent) = dispatch(&mut p);
    comment(&mut p, "acme", "ABC-12", BODY);
    review(&mut p);
    choose(&mut p, "no");
    assert_eq!(p.recovery.usage(), (2, BODY.len() * 2));
    fail(&mut p, change, &sent);
    assert_eq!(p.recovery.usage(), (2, BODY.len() * 2));
    assert_eq!(
        p.recovery.records[&1].original,
        p.recovery.records[&2].original
    );
}

#[test]
fn create_preserves_separate_fields_and_raw_summary_across_both_stages_and_restore() {
    let mut p = sample();
    create(&mut p, "acme", "ABC", "original-type", SUMMARY, BODY);
    assert!(
        matches!(&p.compose.as_ref().unwrap().draft, Draft::Description { summary, .. } if summary == SUMMARY)
    );
    let outgoing = review(&mut p);
    assert!(
        matches!(outgoing, Change::Create { ref summary, ref description, .. } if summary == SUMMARY.trim() && description == BODY.trim_end())
    );
    choose(&mut p, "no");
    assert_eq!(
        first(&p).original,
        Original::Create {
            summary: SUMMARY.into(),
            description: BODY.into()
        }
    );
    let destination = Destination::create(&mut p, "acme", "ABC");
    restore(&mut p, 1, destination);
    assert_eq!(p.compose.as_ref().unwrap().text(), SUMMARY);
    assert_eq!(p.compose.as_ref().unwrap().saved_description, BODY);
    write::compose_paste(&mut p, " plus");
    write::compose_key(&mut p, key(KeyCode::Enter));
    assert_eq!(p.compose.as_ref().unwrap().text(), BODY);
    let outgoing = review(&mut p);
    assert!(
        matches!(outgoing, Change::Create { ref kind, ref summary, .. } if kind.id == "original-type" && *summary == format!("{} plus", SUMMARY.trim_start()))
    );
    choose(&mut p, "no");
    assert_eq!(
        first(&p).original,
        Original::Create {
            summary: format!("{SUMMARY} plus"),
            description: BODY.into()
        }
    );
}

#[test]
fn multiline_summary_recovery_is_typed_and_never_split_into_description() {
    let mut p = sample();
    let change = Change::Create {
        site: "acme".into(),
        project: "ABC".into(),
        kind: kind("1"),
        summary: "  首\nSECRET_second  ".into(),
        description: BODY.into(),
    };
    writes::set_mode(Mode::ReadOnly, None);
    no_spawn(&write::send(&mut p, change.clone()));
    assert_eq!(
        first(&p).original,
        Original::Create {
            summary: "  首\nSECRET_second  ".into(),
            description: BODY.into()
        }
    );
    let destination = Destination::create(&mut p, "acme", "ABC");
    restore(&mut p, 1, destination);
    assert_eq!(p.compose.as_ref().unwrap().text(), "  首\nSECRET_second  ");
    write::compose_key(&mut p, key(KeyCode::Enter));
    assert_eq!(p.compose.as_ref().unwrap().text(), BODY);
    assert!(
        matches!(p.compose.as_ref().unwrap().original(), Original::Create { summary, .. } if summary == "  首\nSECRET_second  ")
    );
}

#[test]
fn capacity_is_atomic_by_original_utf8_bytes_and_count_including_pending() {
    for limit_by_bytes in [false, true] {
        let mut p = sample();
        comment(&mut p, "acme", "ABC-12", BODY);
        let (change, pending) = dispatch(&mut p);
        let mut record = first(&p).clone();
        record.status = Status::Unsent;
        if limit_by_bytes {
            record.change = Change::Create {
                site: "acme".into(),
                project: "ABC".into(),
                kind: kind("1"),
                summary: String::new(),
                description: String::new(),
            };
            record.original = Original::Create {
                summary: "界".repeat((MAX_BYTES - BODY.len()) / 3),
                description: "x".repeat((MAX_BYTES - BODY.len()) % 3),
            };
            p.recovery.reserve(record).unwrap();
            assert_eq!(p.recovery.usage().1, MAX_BYTES);
        } else {
            for _ in 1..MAX_RECORDS {
                p.recovery.reserve(record.clone()).unwrap();
            }
            assert_eq!(p.recovery.usage().0, MAX_RECORDS);
        }
        comment(&mut p, "acme", "ABC-13", "PRIVATE_NEW  ");
        let reviewed = review(&mut p);
        let text = p.compose.as_ref().unwrap().text();
        let cursor = p.compose.as_ref().unwrap().composer.input.cursor();
        let before = p.recovery.records.clone();
        let declined = choose(&mut p, "no");
        assert!(matches!(declined[0], Effect::Report(Outcome::Cancelled)));
        assert_eq!(p.recovery.records, before);
        assert_eq!(p.change, Some(reviewed.clone()));
        assert_eq!(p.compose.as_ref().unwrap().text(), text);
        review(&mut p);
        no_spawn(&choose(&mut p, "yes"));
        assert_eq!(p.recovery.records, before);
        assert_eq!(p.change, Some(reviewed));
        assert_eq!(p.compose.as_ref().unwrap().composer.input.cursor(), cursor);
        fail(&mut p, change, &pending);
        assert_eq!(p.recovery.records.len(), before.len());
        assert_eq!(p.recovery.records[&1].status, Status::Uncertain);
    }
}

#[test]
fn uncertain_status_survives_restore_edits_decline_and_local_mode_refusal() {
    let mut p = sample();
    comment(&mut p, "acme", "ABC-12", BODY);
    let (change, sent) = dispatch(&mut p);
    fail(&mut p, change, &sent);
    for decline in [true, false] {
        let id = *p.recovery.records.keys().next().unwrap();
        let destination = Destination::comment(&mut p, "acme", "ABC-12");
        restore(&mut p, id, destination);
        assert_eq!(p.compose.as_ref().unwrap().status, Status::Uncertain);
        write::compose_paste(&mut p, "SECRET_edit  ");
        let fx = write::compose_key(&mut p, ctrl('s'));
        let radar_core::plugin::Outcome::Handled(fx) = fx else {
            panic!()
        };
        assert!(fx.iter().any(|e| matches!(e, Effect::Menu(m) if m.items.iter().any(|i| i.label.contains("outcome uncertain")))));
        writes::set_mode(Mode::ReadOnly, None);
        let refused = choose(&mut p, if decline { "no" } else { "yes" });
        no_spawn(&refused);
        let mut notices = refused.iter().filter(|e| matches!(e, Effect::Notify(_) | Effect::NotifyWith(_, _) | Effect::Notice(_)));
        let text = match (decline, notices.next(), notices.next()) {
            (true, Some(Effect::Notify(text)), None) => text.as_str(),
            (false, Some(Effect::Notice(message)), None)
                if message.tone == radar_core::view::chrome::NoticeTone::Error => message.text.as_str(),
            other => panic!("decline stays informational; mode refusal is an error: {other:?}"),
        };
        assert!(text.contains("current attempt not sent"), "{text}");
        assert!(text.contains("outcome uncertain"), "{text}");
        assert_eq!(first(&p).status, Status::Uncertain);
    }
}

#[test]
fn old_connection_and_aba_drafts_are_copy_only_and_stale_review_keeps_original_history_scope() {
    let mut p = sample();
    comment(&mut p, "acme", "ABC-12", BODY);
    let original = p.cfg.sites[0].clone();
    p.cfg.sites[0].url = "https://replacement.example".into();
    write_context::reconcile(&mut p);
    let change = review(&mut p);
    assert_eq!(
        change.descriptor(&p).subjects[0].key.instance,
        "https://acme.atlassian.net/"
    );
    let fx = choose(&mut p, "yes");
    no_spawn(&fx);
    assert_eq!(p.compose.as_ref().unwrap().text(), BODY);
    assert!(p.recovery.records.is_empty());
    review(&mut p);
    choose(&mut p, "no");
    p.cfg.sites[0] = original;
    write_context::reconcile(&mut p);
    let destination = Destination::comment(&mut p, "acme", "ABC-12");
    list(&mut p, Some(destination));
    let fx = choose(&mut p, "entry:1");
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Menu(m) if !m.items.iter().any(|i| i.value == "edit")))
    );
    assert_eq!(
        choose(&mut p, "copy-comment")
            .iter()
            .find_map(|e| if let Effect::Copy(t) = e {
                Some(t.as_str())
            } else {
                None
            }),
        Some(BODY)
    );
    assert_eq!(p.recovery.records.len(), 1);
}

#[test]
fn full_target_site_type_and_duplicate_completion_cannot_release_other_records() {
    let mut p = sample();
    let mut other = p.cfg.sites[0].clone();
    other.name = "other".into();
    other.url = "https://other.example".into();
    p.cfg.sites.push(other);
    create(&mut p, "acme", "ABC", "1", SUMMARY, BODY);
    let (change, sent) = dispatch(&mut p);
    let mut wrong = change.clone();
    if let Change::Create { kind, .. } = &mut wrong {
        kind.id = "2".into();
    }
    no_spawn(&write::on_wrote(&mut p, wrong, Ok(Value::Null), &sent));
    assert_eq!(first(&p).status, Status::Pending);
    let mut wrong_dispatch = sent.clone();
    wrong_dispatch.generation += 1;
    no_spawn(&write::on_wrote(
        &mut p,
        change.clone(),
        Ok(Value::Null),
        &wrong_dispatch,
    ));
    assert_eq!(first(&p).status, Status::Pending);
    fail(&mut p, change.clone(), &sent);
    let before = p.recovery.records.clone();
    no_spawn(&write::on_wrote(&mut p, change, Ok(Value::Null), &sent));
    assert_eq!(p.recovery.records, before);
    for destination in [
        Destination::create(&mut p, "other", "ABC"),
        Destination::create(&mut p, "acme", "DEF"),
        Destination::comment(&mut p, "acme", "ABC-12"),
    ] {
        assert!(!has_target(&p, &destination));
    }
}

#[test]
fn recovery_menus_are_stable_one_use_and_pending_is_view_copy_only() {
    let mut p = sample();
    comment(&mut p, "acme", "ABC-12", BODY);
    let (change, sent) = dispatch(&mut p);
    list(&mut p, None);
    let old = p.dialogs.menu_id();
    let fx = choose(&mut p, "entry:1");
    let current = p.dialogs.menu_id();
    assert!(fx.iter().any(|e| matches!(e, Effect::Menu(m) if !m.items.iter().any(|i| matches!(i.value.as_str(), "edit" | "drop")))));
    no_spawn(&p.on_menu(&old, "entry:1"));
    assert_eq!(p.dialogs.menu_id(), current);
    let copied = choose(&mut p, "copy-comment");
    assert!(matches!(&copied[..], [Effect::Copy(t)] if t == BODY));
    assert!(
        !p.on_menu(&current, "copy-comment")
            .iter()
            .any(|e| matches!(e, Effect::Copy(_)))
    );
    list(&mut p, None);
    choose(&mut p, "entry:1");
    fail(&mut p, change, &sent);
    no_spawn(&choose(&mut p, "drop"));
    assert_eq!(first(&p).status, Status::Uncertain);
    list(&mut p, None);
    choose(&mut p, "entry:1");
    choose(&mut p, "drop");
    let drop_id = p.dialogs.menu_id();
    choose(&mut p, "no");
    p.on_menu(&drop_id, "yes");
    assert_eq!(p.recovery.records.len(), 1);
    list(&mut p, None);
    choose(&mut p, "entry:1");
    choose(&mut p, "drop");
    choose(&mut p, "yes");
    assert!(p.recovery.records.is_empty());
}

struct Run {
    host: Host,
    history: Vec<JournalRecord>,
}
impl Run {
    fn new() -> Self {
        let mut host = tests::hosted();
        host.set_env_mode(Some((Mode::Write, "RADAR_MODE".into())));
        crate::page::open(tests::jira_mut(&mut host), "acme", "ABC-12");
        host.enable_journal();
        Self {
            host,
            history: Vec::new(),
        }
    }
    fn p(&mut self) -> &mut JiraPlugin {
        tests::jira_mut(&mut self.host)
    }
    fn event(&mut self, event: Event) -> Vec<HostEffect> {
        let fx = self.host.update(event);
        fn collect(fx: &[HostEffect], out: &mut Vec<JournalRecord>) {
            for e in fx {
                match e {
                    HostEffect::Journal(records) => out.push(records.clone()),
                    HostEffect::Tracked { effect, .. } => {
                        collect(std::slice::from_ref(effect.as_ref()), out)
                    }
                    _ => {}
                }
            }
        }
        collect(&fx, &mut self.history);
        fx
    }
    fn key(&mut self, code: KeyCode) {
        self.event(Event::Key(key(code)));
    }
    fn c(&mut self, c: char) {
        self.key(KeyCode::Char(c));
    }
    fn ctrl(&mut self, c: char) {
        self.event(Event::Key(ctrl(c)));
    }
    fn paste(&mut self, text: &str) {
        self.event(Event::Paste(text.into()));
    }
    fn kept(&mut self) {
        self.c('c');
        self.paste(BODY);
        self.ctrl('s');
        self.c('n');
    }
}

#[test]
fn host_recovery_escape_fresh_edit_and_review_never_send_without_new_confirmation() {
    let mut r = Run::new();
    r.kept();
    r.c('c');
    assert!(
        r.host
            .menu
            .as_ref()
            .unwrap()
            .menu
            .id
            .starts_with("j-recovery:")
    );
    r.key(KeyCode::Esc);
    r.c('c');
    r.key(KeyCode::Enter);
    r.c('e');
    assert_eq!(r.p().compose.as_ref().unwrap().text(), BODY);
    r.ctrl('r');
    r.key(KeyCode::Esc); // no records left; editor remains active
    // Escape on a nonempty editor asks to drop; cancel deliberately.
    if r.host.menu.is_some() {
        r.c('n');
    }
    r.paste("new  ");
    r.ctrl('s');
    r.key(KeyCode::Esc);
    r.c('y');
    assert!(r.p().sent.is_empty());
    assert!(r.p().compose.as_ref().unwrap().text().ends_with("new  y"));
    r.ctrl('s');
    r.c('e');
    r.ctrl('s');
    r.c('y');
    assert!(r.p().compose.is_none());
    assert_eq!(first(r.p()).status, Status::Pending);
    let stored = format!("{:?}", r.history);
    for private in ["SECRET", "世界", "résumé", "RADAR_JIRA_TEST_TOKEN"] {
        assert!(!stored.contains(private));
    }
}

#[test]
fn host_offline_settings_recovery_never_creates_hidden_editor_or_steals_paste() {
    let mut r = Run::new();
    r.kept();
    r.p().cfg.sites.clear();
    write_context::reconcile(r.p());
    r.host.view = View::Settings;
    r.host.focus = Pane::Middle;
    r.host.section = r
        .host
        .sections
        .iter()
        .position(|(_, s)| s.id == "connection")
        .unwrap();
    r.p().edit = Some((
        crate::Target::SiteAdd,
        radar_core::input::TextInput::new("SECRET_settings"),
    ));
    r.ctrl('r');
    r.key(KeyCode::Enter);
    let menu = &r.host.menu.as_ref().unwrap().menu;
    assert!(!menu.items.iter().any(|i| i.value == "edit"));
    r.c('y');
    assert!(r.p().compose.is_none());
    assert_eq!(r.p().edit.as_ref().unwrap().1.value(), "SECRET_settings");
    r.paste(" pasted");
    r.c('x');
    assert_eq!(
        r.p().edit.as_ref().unwrap().1.value(),
        "SECRET_settings pastedx"
    );
    r.ctrl('r');
    r.key(KeyCode::Enter);
    r.c('d');
    r.key(KeyCode::Esc);
    r.paste(" kept");
    assert_eq!(r.p().recovery.records.len(), 1);
    r.ctrl('r');
    r.key(KeyCode::Enter);
    r.c('d');
    r.c('y');
    assert!(r.p().recovery.records.is_empty());
    assert!(r.p().edit.as_ref().unwrap().1.value().ends_with(" kept"));
}

#[test]
fn host_pending_and_uncertain_recovery_remain_reachable_with_no_config() {
    let mut r = Run::new();
    r.c('c');
    r.paste(BODY);
    r.ctrl('s');
    let change = r.p().change.clone().unwrap();
    r.c('y');
    let sent = r.p().sent[&detail_key("acme", "ABC-12")]
        .dispatch
        .clone()
        .unwrap();
    r.p().cfg.sites.clear();
    write_context::reconcile(r.p());
    r.host.view = View::Settings;
    r.host.focus = Pane::Middle;
    r.host.section = r
        .host
        .sections
        .iter()
        .position(|(_, s)| s.id == "connection")
        .unwrap();
    r.ctrl('r');
    r.key(KeyCode::Enter);
    assert!(
        r.host
            .menu
            .as_ref()
            .unwrap()
            .menu
            .items
            .iter()
            .any(|i| i.label.contains("send pending"))
    );
    r.key(KeyCode::Esc);
    r.event(Event::Msg {
        plugin: 0,
        msg: Box::new(JiraMsg::Wrote {
            dispatch: sent,
            change,
            result: Err("SECRET_network".into()),
        }),
    });
    r.ctrl('r');
    r.key(KeyCode::Enter);
    assert!(
        r.host
            .menu
            .as_ref()
            .unwrap()
            .menu
            .items
            .iter()
            .any(|i| i.label.contains("outcome uncertain"))
    );
    assert!(r.p().compose.is_none());
}

#[test]
fn host_recovery_leaves_filter_and_goto_key_and_paste_ownership_unchanged() {
    for goto in [false, true] {
        let mut r = Run::new();
        r.kept();
        r.p().pages.clear();
        if goto {
            r.p().goto = Some(crate::Goto {
                input: radar_core::input::TextInput::new("SECRET_go"),
                before: vec![],
                found: None,
            });
        } else {
            r.p().filter_input = Some(radar_core::input::TextInput::new("SECRET_filter"));
        }
        r.ctrl('r');
        r.key(KeyCode::Enter);
        assert!(
            !r.host
                .menu
                .as_ref()
                .unwrap()
                .menu
                .items
                .iter()
                .any(|i| i.value == "edit")
        );
        r.c('y');
        r.paste(" pasted");
        r.c('x');
        assert!(r.p().compose.is_none());
        let text = if goto {
            r.p().goto.as_ref().unwrap().input.value()
        } else {
            r.p().filter_input.as_ref().unwrap().value()
        };
        assert!(text.ends_with(" pastedx"));
        assert_eq!(r.p().recovery.records.len(), 1);
    }
}

#[test]
fn current_create_copy_distinguishes_exact_fields_without_consuming_active_editor() {
    let mut p = sample();
    create(&mut p, "acme", "ABC", "1", SUMMARY, BODY);
    list(&mut p, None);
    assert!(
        matches!(&choose(&mut p, "copy-current-summary")[..], [Effect::Copy(text)] if text == SUMMARY)
    );
    list(&mut p, None);
    assert!(
        matches!(&choose(&mut p, "copy-current-description")[..], [Effect::Copy(text)] if text == BODY)
    );
    assert_eq!(p.compose.as_ref().unwrap().text(), BODY);
    assert!(p.recovery.records.is_empty());
}

#[test]
fn same_project_create_types_remain_distinct_and_restore_original_type_without_read() {
    let mut p = sample();
    for type_id in ["original-type", "other-type"] {
        create(&mut p, "acme", "ABC", type_id, SUMMARY, BODY);
        review(&mut p);
        choose(&mut p, "no");
    }
    assert_eq!(p.recovery.records.len(), 2);
    let effects = write::start_new(&mut p, "acme", Some("ABC"));
    no_spawn(&effects);
    choose(&mut p, "entry:1");
    no_spawn(&choose(&mut p, "edit"));
    assert!(
        matches!(&p.compose.as_ref().unwrap().draft, Draft::Summary { project, kind } if project == "ABC" && kind.id == "original-type")
    );
    assert_eq!(p.recovery.records.len(), 1);
    assert!(matches!(&first(&p).change, Change::Create { kind, .. } if kind.id == "other-type"));
    assert!(!format!("{:?} {:?}", p.snapshot(), p.config()).contains("SECRET_"));
}

#[test]
fn missing_site_refusal_preserves_editor_and_original_scope_without_reservation() {
    let mut p = sample();
    comment(&mut p, "acme", "ABC-12", BODY);
    review(&mut p);
    p.cfg.sites.clear();
    write_context::reconcile(&mut p);
    let before = p.compose.as_ref().unwrap().text();
    let fx = choose(&mut p, "yes");
    no_spawn(&fx);
    assert!(fx.iter().any(|e| matches!(e, Effect::Journal(a) if a.subjects[0].key.instance == "https://acme.atlassian.net/")));
    assert_eq!(p.compose.as_ref().unwrap().text(), before);
    assert!(p.change.is_some());
    assert!(p.recovery.records.is_empty());
}
