//! The plugin drives the fake provider the way the host does: ticks and dock
//! intents, no renderer. Nothing waits on a clock or races the test: the
//! plugin's tasks run on the test's thread, only when a step lets them
//! ([`SessionsPlugin::on_this_thread`]).

use radar_core::Plugin;
use radar_core::dock::{AskState, DockIntent, DockItem, DockQuestion, DockView, PermissionAction, SessionState, ToolState};
use radar_sessions::SessionsPlugin;
use radar_sessions::fake::FakeProvider;

/// Steps a test gives anything before it fails: a bound on work, not on time.
const STEPS: u64 = 256;

fn plugin(provider: &FakeProvider) -> SessionsPlugin {
    let mut p = SessionsPlugin::on_this_thread();
    p.add_adapter(Box::new(provider.adapter()));
    // The plugin listens before anything is sent: a turn run before it would be told to nobody.
    for n in 0..STEPS as i64 {
        if provider.listening() {
            return p;
        }
        p.tick(n);
        p.run_tasks();
    }
    panic!("the plugin never subscribed");
}

/// Tick and run the plugin's tasks until `done` holds on the dock.
fn until(p: &mut SessionsPlugin, what: &str, done: impl Fn(&DockView) -> bool) -> DockView {
    for n in 0..STEPS {
        p.tick(n as i64);
        if let Some(dock) = p.dock()
            && done(&dock)
        {
            return dock;
        }
        p.run_tasks();
    }
    panic!("never reached: {what}; dock = {:?}", p.dock());
}

/// Tick and run every task to its end, a few times over: whatever the plugin and the provider
/// would still do has been done. Returns how many effects ran a task (a desktop notification).
fn quiet(p: &mut SessionsPlugin) -> usize {
    let mut spawned = 0;
    for n in 0..8 {
        spawned += p.tick(n).iter().filter(|e| matches!(e, radar_core::Effect::Spawn(_))).count();
        p.run_tasks();
    }
    spawned
}

fn permission(dock: &DockView) -> Option<(String, Option<PermissionAction>)> {
    dock.transcript.iter().find_map(|i| match i {
        DockItem::Permission(p) => Some((p.id.clone(), p.answered)),
        _ => None,
    })
}

fn say(dock: &DockView, text: &str) -> bool {
    dock.transcript.iter().any(|i| matches!(i, DockItem::Assistant { text: t, saved: true, .. } if t == text))
}

fn start(p: &mut SessionsPlugin) -> DockView {
    p.dock_intent(&DockIntent::NewSession { provider: None });
    until(p, "a session that can take text", |d| d.composer.can_send && d.tabs.len() == 1)
}

fn question(dock: &DockView) -> Option<&DockQuestion> {
    dock.transcript.iter().find_map(|i| match i {
        DockItem::Question(q) => Some(q),
        _ => None,
    })
}

#[test]
fn a_question_waits_for_the_operator_and_a_picked_choice_is_the_answer() {
    let provider = FakeProvider::new();
    let mut p = plugin(&provider);
    start(&mut p);
    p.dock_intent(&DockIntent::Send { text: "ask me which colour".into() });
    let waiting = until(&mut p, "a question", |d| question(d).is_some());
    assert_eq!(waiting.tabs[0].state, SessionState::Approval);
    let q = question(&waiting).unwrap().clone();
    assert_eq!(q.state, AskState::Waiting);
    assert_eq!(q.questions[0].choices.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(), ["red", "blue"]);
    // A label the question does not offer, and ticking on, answer nothing.
    p.dock_intent(&DockIntent::Answer { id: q.id.clone(), question: q.questions[0].id.clone(), choices: vec!["green".into()], text: None });
    quiet(&mut p);
    assert_eq!(question(&p.dock().unwrap()).unwrap().state, AskState::Waiting);

    p.dock_intent(&DockIntent::Answer { id: q.id.clone(), question: q.questions[0].id.clone(), choices: vec!["blue".into()], text: None });
    // The provider's events reach the inbox one by one: the answer can show a tick before the turn ends.
    let done = until(&mut p, "the answer and the end of the turn", |d| say(d, "You chose: blue") && d.tabs[0].state == SessionState::Idle);
    let q = question(&done).unwrap();
    assert_eq!(q.state, AskState::Answered);
    assert_eq!(q.questions[0].answer, vec!["blue".to_string()]);
}

#[test]
fn the_composer_text_answers_a_question_in_the_operators_own_words_and_decline_refuses() {
    let provider = FakeProvider::new();
    let mut p = plugin(&provider);
    start(&mut p);
    p.dock_intent(&DockIntent::Send { text: "ask me".into() });
    let q = question(&until(&mut p, "a question", |d| question(d).is_some())).unwrap().clone();
    p.dock_intent(&DockIntent::Answer { id: q.id.clone(), question: q.questions[0].id.clone(), choices: vec![], text: Some("  green  ".into()) });
    until(&mut p, "the typed answer", |d| say(d, "You chose: green"));

    p.dock_intent(&DockIntent::Send { text: "ask again".into() });
    let q = until(&mut p, "a second question", |d| d.transcript.iter().filter(|i| matches!(i, DockItem::Question(_))).count() == 2);
    let id = q.transcript.iter().rev().find_map(|i| match i { DockItem::Question(q) => Some(q.id.clone()), _ => None }).unwrap();
    p.dock_intent(&DockIntent::Decline { id });
    let done = until(&mut p, "the refusal", |d| say(d, "No answer"));
    assert!(done.transcript.iter().any(|i| matches!(i, DockItem::Question(q) if q.state == AskState::Declined)));
}

#[test]
fn a_request_waits_until_the_operator_approves_and_then_the_tool_runs() {
    let provider = FakeProvider::new();
    let mut p = plugin(&provider);
    start(&mut p);
    p.dock_intent(&DockIntent::Send { text: "run something".into() });
    let waiting = until(&mut p, "a pending request", |d| permission(d).is_some());
    assert_eq!(waiting.tabs[0].state, SessionState::Approval);
    // Ticking on, and answering an unknown id, change nothing: nothing auto-approves.
    quiet(&mut p);
    p.dock_intent(&DockIntent::Approve { id: "not-a-request".into() });
    quiet(&mut p);
    let still = p.dock().unwrap();
    let (id, answered) = permission(&still).unwrap();
    assert_eq!(answered, None);

    p.dock_intent(&DockIntent::Approve { id });
    let done = until(&mut p, "the turn to finish", |d| say(d, "The command finished."));
    assert!(matches!(permission(&done), Some((_, Some(PermissionAction::ApproveOnce)))));
    assert!(done.transcript.iter().any(|i| matches!(i, DockItem::Tool { state: ToolState::Done, title, .. } if title == "ls -la")));
    assert_eq!(done.tabs[0].state, SessionState::Idle);
}

#[test]
fn approving_for_the_session_answers_the_same_request_again_without_asking() {
    let provider = FakeProvider::new();
    let mut p = plugin(&provider);
    start(&mut p);
    p.dock_intent(&DockIntent::Send { text: "run something".into() });
    let waiting = until(&mut p, "a pending request", |d| permission(d).is_some());
    let offered = waiting.transcript.iter().find_map(|i| if let DockItem::Permission(p) = i { Some(p.actions.clone()) } else { None }).unwrap();
    assert!(offered.contains(&PermissionAction::ApproveForSession), "{offered:?}");
    let (id, _) = permission(&waiting).unwrap();
    p.dock_intent(&DockIntent::ApproveForSession { id });
    let done = until(&mut p, "the turn to finish", |d| say(d, "The command finished.") && d.tabs[0].state == SessionState::Idle);
    assert!(matches!(permission(&done), Some((_, Some(PermissionAction::ApproveForSession)))));

    // The same command again in the same session: radar approves it, and says so.
    p.dock_intent(&DockIntent::Send { text: "run it again".into() });
    let again = until(&mut p, "the second turn to finish", |d| {
        d.transcript.iter().filter(|i| matches!(i, DockItem::Tool { state: ToolState::Done, .. })).count() == 2 && d.tabs[0].state == SessionState::Idle
    });
    let answers: Vec<_> = again.transcript.iter().filter_map(|i| if let DockItem::Permission(p) = i { Some(p.answered) } else { None }).collect();
    assert_eq!(answers, [Some(PermissionAction::ApproveForSession), Some(PermissionAction::ApproveForSession)]);
    assert!(format!("{again:?}").contains("approved for this session: ls -la"), "{again:?}");
}

#[test]
fn denying_refuses_the_tool() {
    let provider = FakeProvider::new();
    let mut p = plugin(&provider);
    start(&mut p);
    p.dock_intent(&DockIntent::Send { text: "run it".into() });
    let waiting = until(&mut p, "a pending request", |d| permission(d).is_some());
    let (id, _) = permission(&waiting).unwrap();
    p.dock_intent(&DockIntent::Deny { id });
    let done = until(&mut p, "the denial", |d| matches!(permission(d), Some((_, Some(PermissionAction::Deny)))));
    assert!(done.transcript.iter().any(|i| matches!(i, DockItem::Tool { state: ToolState::Denied, .. })));
}

#[test]
fn stop_ends_a_running_turn() {
    let provider = FakeProvider::new();
    let mut p = plugin(&provider);
    start(&mut p);
    p.dock_intent(&DockIntent::Send { text: "keep going long".into() });
    let running = until(&mut p, "a running turn", |d| d.composer.can_stop);
    assert_eq!(running.tabs[0].state, SessionState::Streaming);
    p.dock_intent(&DockIntent::Stop);
    let stopped = until(&mut p, "stopped", |d| d.tabs[0].state == SessionState::Stopped);
    assert!(!stopped.composer.can_stop);
}

#[test]
fn a_restarted_plugin_restores_the_session_and_reads_its_history_back_from_the_provider() {
    let provider = FakeProvider::new();
    let mut first = plugin(&provider);
    start(&mut first);
    first.dock_intent(&DockIntent::Send { text: "hello there".into() });
    let before = until(&mut first, "the answer", |d| say(d, "You said: hello there"));
    let snapshot = first.snapshot().expect("a session to keep");
    // The snapshot holds identity and cursor, not the transcript.
    assert!(!snapshot.to_string().contains("hello there"), "{snapshot}");
    drop(first);

    let mut second = plugin(&provider);
    second.restore(snapshot);
    // Read back when its tab shows, not at start.
    second.dock_visible(true);
    let after = until(&mut second, "the history", |d| say(d, "You said: hello there"));
    assert_eq!(after.tabs.len(), 1);
    assert_eq!(after.transcript, before.transcript);
}

// ---- restored sessions: what waited lapses, and a message or the resume key carries on ----

/// The plugin of the next run: `provider`'s process ended with the last one (nothing runs, history stays), and
/// the new plugin holds what the last one kept.
fn next_run(provider: &FakeProvider, snapshot: serde_json::Value) -> SessionsPlugin {
    provider.restart();
    let mut p = plugin(provider);
    p.restore(snapshot);
    p
}

/// How many times radar created or resumed a session with the provider.
fn calls(provider: &FakeProvider) -> usize {
    provider.setups().len()
}

fn notices(dock: &DockView) -> Vec<String> {
    dock.transcript
        .iter()
        .filter_map(|i| match i {
            DockItem::Notice { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn lapsed_permission(dock: &DockView) -> Option<&radar_core::dock::DockPermission> {
    dock.transcript.iter().find_map(|i| match i {
        DockItem::Permission(p) if p.lapsed => Some(p),
        _ => None,
    })
}

const CARRY_ON: &str = "the turn that was waiting was ended when radar closed; send a message to carry on";

#[test]
fn a_request_and_a_question_that_waited_when_radar_closed_lapse_and_need_no_one() {
    let provider = FakeProvider::new();
    let mut first = plugin(&provider);
    // Two sessions: one waits for an approval, the other for an answer.
    two_tabs(&mut first);
    let tabs = first.dock().unwrap().tabs;
    first.dock_intent(&DockIntent::SwitchTab { id: tabs[0].id.clone() });
    first.dock_intent(&DockIntent::Send { text: "run something".into() });
    until(&mut first, "an approval waiting", |d| permission(d).is_some_and(|(_, a)| a.is_none()));
    first.dock_intent(&DockIntent::SwitchTab { id: tabs[1].id.clone() });
    first.dock_intent(&DockIntent::Send { text: "ask me".into() });
    until(&mut first, "a question waiting", |d| question(d).is_some_and(|q| q.state == AskState::Waiting));
    let snapshot = first.snapshot().unwrap();
    drop(first);

    let mut p = next_run(&provider, snapshot);
    let before = calls(&provider);
    quiet(&mut p);
    // Restored, nothing resumed: every tab is not attached, and none needs you.
    let (restored, active) = p.dock_tabs().unwrap();
    assert!(restored.iter().all(|t| t.state == SessionState::Detached), "{restored:?}");
    assert!(radar_core::dock::wanting(&restored, active.as_deref(), false).is_empty(), "no chip for a restored session");
    assert_eq!(calls(&provider), before, "no resume before a tab shows");
    let dock = p.dock().unwrap();
    assert!(dock.composer.can_send && dock.composer.disabled.is_none(), "a message attaches it: {:?}", dock.composer);
    assert!(dock.composer.commands.iter().any(|c| c.name == "resume"));
    assert_eq!(dock.activity.as_ref().map(|a| a.doing), Some(radar_core::dock::Doing::Detached));

    // The question's tab shows: it is read back, and the question lapses.
    p.dock_visible(true);
    let asked = until(&mut p, "the question read back", |d| question(d).is_some() && !notices(d).is_empty());
    let q = question(&asked).unwrap();
    assert_eq!(q.state, AskState::Lapsed);
    assert!(q.questions.iter().all(|a| a.answered), "nothing to answer: {q:?}");
    assert_eq!(notices(&asked), vec![CARRY_ON.to_string()]);
    assert_eq!(calls(&provider), before + 1, "only the tab that showed was resumed");
    assert_eq!(asked.tabs[1].state, SessionState::Idle);
    assert_eq!(asked.tabs[0].state, SessionState::Detached, "the other tab waits until it shows");
    assert!(asked.activity.is_none(), "a turn read back is not news: {:?}", asked.activity);
    assert!(!asked.composer.commands.iter().any(|c| c.name == "resume"), "attached: no resume command");

    // The request's tab: the same, without buttons, and an answer reaches nobody.
    p.dock_intent(&DockIntent::SwitchTab { id: tabs[0].id.clone() });
    let shown = until(&mut p, "the request read back", |d| lapsed_permission(d).is_some());
    let r = lapsed_permission(&shown).unwrap().clone();
    assert!(r.actions.is_empty() && r.answered.is_none(), "{r:?}");
    let (all, active) = p.dock_tabs().unwrap();
    assert!(radar_core::dock::wanting(&all, active.as_deref(), true).is_empty() && radar_core::dock::wanting(&all, active.as_deref(), false).is_empty());
    p.dock_intent(&DockIntent::Approve { id: r.id.clone() });
    p.dock_intent(&DockIntent::AlwaysAllow { id: r.id.clone() });
    quiet(&mut p);
    let after = p.dock().unwrap();
    assert!(!after.transcript.iter().any(|i| matches!(i, DockItem::Error { .. })), "{:?}", after.transcript);
    assert_eq!(lapsed_permission(&after).map(|p| p.id.clone()), Some(r.id));
    assert!(!notices(&after).iter().any(|n| n.starts_with("always allowed")), "no rule is made from a lapsed request");

    // A message carries on.
    p.dock_intent(&DockIntent::Send { text: "hello".into() });
    until(&mut p, "the answer", |d| say(d, "You said: hello"));
}

#[test]
fn a_restored_request_sends_no_desktop_notification_before_or_after_it_is_read_back() {
    let provider = FakeProvider::new();
    let dir = std::env::temp_dir().join(format!("radar-notify-restored-{}", std::process::id()));
    let table: toml::Value = toml::from_str(&format!("notify = true\nreviews_dir = {:?}\n", dir.display().to_string())).unwrap();
    let mut first = plugin(&provider);
    start(&mut first);
    first.dock_intent(&DockIntent::Send { text: "run something".into() });
    until(&mut first, "an approval waiting", |d| permission(d).is_some());
    let snapshot = first.snapshot().unwrap();
    drop(first);

    let mut p = next_run(&provider, snapshot);
    p.configure(Some(&table)).unwrap();
    let hidden = quiet(&mut p);
    assert_eq!(hidden, 0, "a restored session needs no one");
    // Read back with its tab out of sight (another tab shows): still nothing to tell.
    p.dock_intent(&DockIntent::Resume);
    let (back, spawned) = until_counting(&mut p, "the request read back", |d| lapsed_permission(d).is_some() && !notices(d).is_empty());
    let more = quiet(&mut p);
    assert_eq!(spawned + more, 0, "a lapsed request is not news");
    assert!(!back.tabs[0].unread, "a turn read back is not unread");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_message_typed_in_a_restored_session_resumes_it_and_then_goes() {
    use radar_core::dock::Queued;
    let provider = FakeProvider::new();
    let mut first = plugin(&provider);
    start(&mut first);
    first.dock_intent(&DockIntent::Send { text: "hello there".into() });
    until(&mut first, "the answer", |d| say(d, "You said: hello there"));
    let snapshot = first.snapshot().unwrap();
    drop(first);

    let mut p = next_run(&provider, snapshot);
    // The dock is hidden: nothing is read back until the message.
    p.dock_intent(&DockIntent::Send { text: "and now?".into() });
    let waiting = p.dock().unwrap();
    assert_eq!(users(&waiting), vec![("and now?".into(), Some(Queued::Here))]);
    let done = until(&mut p, "the history, then the answer", |d| say(d, "You said: hello there") && say(d, "You said: and now?"));
    assert_eq!(users(&done), vec![("hello there".into(), None), ("and now?".into(), None)]);
    assert!(notices(&done).is_empty(), "the last turn had ended: nothing was cut: {:?}", done.transcript);
    assert!(!done.composer.commands.iter().any(|c| c.name == "resume"), "attached: no resume command");
}

#[test]
fn the_resume_key_and_slash_resume_attach_without_sending() {
    let provider = FakeProvider::new();
    let mut first = plugin(&provider);
    start(&mut first);
    first.dock_intent(&DockIntent::Send { text: "hello there".into() });
    until(&mut first, "the answer", |d| say(d, "You said: hello there"));
    let snapshot = first.snapshot().unwrap();
    drop(first);

    for how in [DockIntent::Resume, DockIntent::Send { text: " /resume ".into() }] {
        let mut p = next_run(&provider, snapshot.clone());
        p.dock_intent(&how);
        // The history reaches the dock event by event before the resume's end does: until then the tab says
        // `resuming`. Waiting for the history alone left the state to how far the end had come (failed once in
        // ten runs of the crate when the tasks ran on their own threads).
        let back = until(&mut p, "the history, attached", |d| say(d, "You said: hello there") && d.tabs[0].state != SessionState::Resuming);
        assert_eq!(users(&back), vec![("hello there".into(), None)], "nothing was sent: {how:?}");
        assert_eq!(back.tabs[0].state, SessionState::Idle);
    }
}

#[test]
fn a_provider_that_asks_again_after_the_resume_gets_a_fresh_request_that_is_answered() {
    let provider = FakeProvider::new();
    let mut first = plugin(&provider);
    start(&mut first);
    first.dock_intent(&DockIntent::Send { text: "run something".into() });
    until(&mut first, "an approval waiting", |d| permission(d).is_some());
    let snapshot = first.snapshot().unwrap();
    drop(first);

    let mut p = next_run(&provider, snapshot);
    p.dock_visible(true);
    let old = until(&mut p, "the lapsed request", |d| lapsed_permission(d).is_some() && !notices(d).is_empty()).transcript.len();
    provider.reask();
    let fresh = until(&mut p, "the request asked again", |d| {
        d.transcript.iter().any(|i| matches!(i, DockItem::Permission(r) if !r.lapsed && r.answered.is_none() && !r.actions.is_empty()))
    });
    assert!(fresh.transcript.len() > old);
    assert_eq!(fresh.tabs[0].state, SessionState::Approval, "this one needs you");
    let id = fresh.transcript.iter().find_map(|i| match i {
        DockItem::Permission(r) if !r.lapsed => Some(r.id.clone()),
        _ => None,
    });
    p.dock_intent(&DockIntent::Approve { id: id.unwrap() });
    until(&mut p, "the tool ran", |d| say(d, "The command finished."));
}

#[test]
fn a_resume_the_provider_refuses_says_so_and_the_session_stays_to_try_again() {
    let provider = FakeProvider::new();
    let mut first = plugin(&provider);
    start(&mut first);
    let snapshot = first.snapshot().unwrap();
    drop(first);

    // Another provider that never held the session.
    let other = FakeProvider::new();
    let mut p = next_run(&other, snapshot);
    p.dock_visible(true);
    let failed = until(&mut p, "the refusal", |d| d.transcript.iter().any(|i| matches!(i, DockItem::Error { .. })));
    let errors = |d: &DockView| d.transcript.iter().filter(|i| matches!(i, DockItem::Error { text, .. } if text.starts_with("could not resume:"))).count();
    assert_eq!(errors(&failed), 1);
    assert_eq!(failed.tabs.len(), 1, "the tab stays");
    assert_eq!(failed.tabs[0].state, SessionState::Error);
    assert!(failed.composer.can_send && failed.composer.commands.iter().any(|c| c.name == "resume"));
    // Showing it again does not try again by itself; the resume key does.
    p.dock_visible(false);
    p.dock_visible(true);
    quiet(&mut p);
    assert_eq!(errors(&p.dock().unwrap()), 1);
    p.dock_intent(&DockIntent::Resume);
    until(&mut p, "a second try", |d| errors(d) == 2);
}

#[test]
fn a_restored_review_session_keeps_the_review_policy() {
    let provider = FakeProvider::new();
    let mut first = plugin(&provider);
    start(&mut first);
    let mut snapshot = first.snapshot().unwrap();
    drop(first);
    // As the review journey's session is kept.
    snapshot["sessions"][0]["review"] = serde_json::Value::Bool(true);

    let mut p = next_run(&provider, snapshot);
    let before = calls(&provider);
    p.dock_visible(true);
    until(&mut p, "attached", |d| d.tabs[0].state == SessionState::Idle);
    assert!(provider.setups()[before..].iter().all(|s| s.review), "resumed with the review policy: {:?}", provider.setups());
    assert_eq!(calls(&provider), before + 1);
    p.dock_intent(&DockIntent::Send { text: "run make".into() });
    let asked = until(&mut p, "a request", |d| permission(d).is_some());
    let r = asked.transcript.iter().find_map(|i| match i {
        DockItem::Permission(r) => Some(r.clone()),
        _ => None,
    });
    assert!(!r.unwrap().actions.contains(&PermissionAction::ApproveForSession), "a review session never offers approve for session");
}

#[test]
fn the_commands_a_session_reports_reach_the_composer_with_what_they_do() {
    use radar_sessions::SlashCommand;
    let provider = FakeProvider::new();
    provider.set_commands(vec![
        SlashCommand::new("compact", "summarise the conversation"),
        SlashCommand::new("/review", "review a change").with_hint("<pr>"),
    ]);
    let mut p = plugin(&provider);
    start(&mut p);
    let commands = until(&mut p, "the reported commands", |d| d.composer.commands.iter().any(|c| c.name == "compact")).composer.commands;
    assert!(commands.iter().any(|c| c.name == "compact" && c.detail == "summarise the conversation"));
    // The slash is the composer's; a hint comes ahead of what the command does.
    assert!(commands.iter().any(|c| c.name == "review" && c.detail == "<pr>  review a change"));
}

#[test]
fn a_provider_that_reports_none_falls_back_to_the_adapters_own_commands() {
    let provider = FakeProvider::new();
    let mut p = plugin(&provider);
    let dock = start(&mut p);
    assert_eq!(dock.composer.commands.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), vec!["model", "close"]);
}

fn menu_of(out: radar_core::Outcome) -> Option<radar_core::Menu> {
    match out {
        radar_core::Outcome::Handled(fx) => fx.into_iter().find_map(|e| match e {
            radar_core::Effect::Menu(m) => Some(m),
            _ => None,
        }),
        radar_core::Outcome::Ignored => None,
    }
}

fn hides_dock(out: radar_core::Outcome) -> bool {
    match out {
        radar_core::Outcome::Handled(fx) => fx.iter().any(|e| matches!(e, radar_core::Effect::HideDock)),
        radar_core::Outcome::Ignored => false,
    }
}

fn two_tabs(p: &mut SessionsPlugin) -> DockView {
    start(p);
    p.dock_intent(&DockIntent::NewSession { provider: None });
    until(p, "two sessions that can take text", |d| d.tabs.len() == 2 && d.composer.can_send)
}

#[test]
fn an_idle_session_closes_at_once_and_the_neighbour_shows() {
    let provider = FakeProvider::new();
    let mut p = plugin(&provider);
    let dock = two_tabs(&mut p);
    let (first, second) = (dock.tabs[0].id.clone(), dock.tabs[1].id.clone());
    assert_eq!(dock.active.as_deref(), Some(second.as_str()));
    // Closing the one showing, the last: the one before it takes over; no question for an idle one.
    let out = p.dock_intent(&DockIntent::CloseTab { id: second.clone() });
    assert!(menu_of(out).is_none());
    let after = p.dock().unwrap();
    assert_eq!(after.tabs.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), [first.as_str()]);
    assert_eq!(after.active.as_deref(), Some(first.as_str()));
    // The last tab closing leaves the dock with no session at all.
    p.dock_intent(&DockIntent::CloseTab { id: first });
    let empty = p.dock().unwrap();
    assert!(empty.tabs.is_empty());
    assert_eq!(empty.active, None);
}

#[test]
fn closing_the_last_tab_hides_the_dock_and_closing_another_does_not() {
    let provider = FakeProvider::new();
    let mut p = plugin(&provider);
    let dock = two_tabs(&mut p);
    let (first, second) = (dock.tabs[0].id.clone(), dock.tabs[1].id.clone());
    assert!(!hides_dock(p.dock_intent(&DockIntent::CloseTab { id: second })), "a tab is left to show");
    assert!(hides_dock(p.dock_intent(&DockIntent::CloseTab { id: first })), "the last one closing leaves no empty window");
    // `/close` is the same close.
    start(&mut p);
    assert!(hides_dock(p.dock_intent(&DockIntent::Send { text: "/close".into() })));
}

#[test]
fn stop_and_close_of_the_last_tab_hides_the_dock_and_the_provider_runs_nothing() {
    let provider = FakeProvider::new();
    let mut p = plugin(&provider);
    start(&mut p);
    p.dock_intent(&DockIntent::Send { text: "keep going long".into() });
    let running = until(&mut p, "a running turn", |d| d.composer.can_stop);
    assert_eq!(provider.running(), 1);
    let ask = menu_of(p.dock_intent(&DockIntent::CloseTab { id: running.tabs[0].id.clone() })).expect("a question");
    let fx = p.on_menu(&ask.id, "yes");
    assert!(fx.iter().any(|e| matches!(e, radar_core::Effect::HideDock)));
    for n in 0..STEPS {
        p.tick(n as i64);
        if provider.running() == 0 {
            return;
        }
        p.run_tasks();
    }
    panic!("the provider still runs the closed session's turn");
}

#[test]
fn closing_a_tab_that_is_not_showing_leaves_the_showing_one_and_a_middle_one_hands_over_to_the_next() {
    let provider = FakeProvider::new();
    let mut p = plugin(&provider);
    start(&mut p);
    for _ in 0..2 {
        p.dock_intent(&DockIntent::NewSession { provider: None });
    }
    let dock = until(&mut p, "three sessions", |d| d.tabs.len() == 3 && d.composer.can_send);
    let ids: Vec<String> = dock.tabs.iter().map(|t| t.id.clone()).collect();
    p.dock_intent(&DockIntent::CloseTab { id: ids[0].clone() });
    assert_eq!(p.dock().unwrap().active.as_deref(), Some(ids[2].as_str()), "closing another tab changes nothing");
    p.dock_intent(&DockIntent::SwitchTab { id: ids[1].clone() });
    p.dock_intent(&DockIntent::CloseTab { id: ids[1].clone() });
    let after = p.dock().unwrap();
    assert_eq!(after.tabs.len(), 1);
    assert_eq!(after.active.as_deref(), Some(ids[2].as_str()));
}

#[test]
fn a_new_tab_after_a_close_does_not_take_the_name_of_one_that_is_open() {
    let provider = FakeProvider::new();
    let mut p = plugin(&provider);
    let dock = two_tabs(&mut p);
    p.dock_intent(&DockIntent::CloseTab { id: dock.tabs[0].id.clone() });
    p.dock_intent(&DockIntent::NewSession { provider: None });
    let after = until(&mut p, "two sessions again", |d| d.tabs.len() == 2 && d.composer.can_send);
    let titles: Vec<&str> = after.tabs.iter().map(|t| t.title.as_str()).collect();
    assert_ne!(titles[0], titles[1], "{titles:?}");
}

#[test]
fn a_running_session_is_asked_about_and_keep_it_leaves_it_running() {
    let provider = FakeProvider::new();
    let mut p = plugin(&provider);
    start(&mut p);
    p.dock_intent(&DockIntent::Send { text: "keep going long".into() });
    let running = until(&mut p, "a running turn", |d| d.composer.can_stop);
    let id = running.tabs[0].id.clone();
    let ask = menu_of(p.dock_intent(&DockIntent::CloseTab { id: id.clone() })).expect("a question");
    assert!(ask.question, "a dialog, not a list");
    assert_eq!(p.dock().unwrap().tabs.len(), 1, "nothing closed before the answer");
    // keep it
    p.on_menu(&ask.id, "no");
    let kept = p.dock().unwrap();
    assert_eq!(kept.tabs.len(), 1);
    assert_eq!(kept.tabs[0].state, SessionState::Streaming);
}

#[test]
fn stop_and_close_ends_the_turn_and_the_session_stays_in_the_providers_list() {
    let provider = FakeProvider::new();
    let mut p = plugin(&provider);
    start(&mut p);
    p.dock_intent(&DockIntent::Send { text: "keep going long".into() });
    let running = until(&mut p, "a running turn", |d| d.composer.can_stop);
    let id = running.tabs[0].id.clone();
    let ask = menu_of(p.dock_intent(&DockIntent::CloseTab { id })).expect("a question");
    let fx = p.on_menu(&ask.id, "yes");
    assert!(fx.iter().any(|e| matches!(e, radar_core::Effect::PersistCache)), "the closed tab does not come back after a restart");
    let closed = p.dock().unwrap();
    assert!(closed.tabs.is_empty());
    // Not deleted: the session picker still lists it, so alt-s resumes it.
    p.dock_intent(&DockIntent::OpenPicker { kind: radar_core::dock::PickerKind::Session });
    let listed = until(&mut p, "the provider's sessions", |d| d.picker.as_ref().is_some_and(|k| !k.choices.is_empty()));
    assert_eq!(listed.picker.unwrap().choices.len(), 1);
}

/// The user rows: their text and how they are queued.
fn users(dock: &DockView) -> Vec<(String, Option<radar_core::dock::Queued>)> {
    dock.transcript
        .iter()
        .filter_map(|i| match i {
            DockItem::User { text, queued, .. } => Some((text.clone(), *queued)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_message_sent_while_a_turn_runs_is_shown_queued_and_sent_when_the_turn_ends() {
    use radar_core::dock::Queued;
    let provider = FakeProvider::new();
    let mut p = plugin(&provider);
    start(&mut p);
    // A turn that lasts until the provider is told to finish it.
    p.dock_intent(&DockIntent::Send { text: "a long job".into() });
    let running = until(&mut p, "the turn to run", |d| d.composer.can_stop && say(d, "Working on it"));
    assert!(running.composer.can_send, "Enter takes a message while the turn runs");

    // The fake takes nothing mid-turn: radar keeps both, in order, shown at once.
    p.dock_intent(&DockIntent::Send { text: "and?".into() });
    p.dock_intent(&DockIntent::Send { text: "one more".into() });
    let queued = p.dock().unwrap();
    assert_eq!(
        users(&queued),
        vec![("a long job".into(), None), ("and?".into(), Some(Queued::Here)), ("one more".into(), Some(Queued::Here))]
    );
    quiet(&mut p);
    assert_eq!(users(&p.dock().unwrap()).len(), 3, "nothing is sent while the turn runs");

    provider.release();
    let done = until(&mut p, "both answered", |d| say(d, "You said: and?") && say(d, "You said: one more"));
    assert_eq!(users(&done), vec![("a long job".into(), None), ("and?".into(), None), ("one more".into(), None)]);
    assert!(!done.transcript.iter().any(|i| matches!(i, DockItem::Error { .. })), "the provider was never sent text mid-turn: {:?}", done.transcript);
    assert!(!done.composer.can_stop);
}

#[test]
fn a_message_typed_while_disconnected_waits_and_goes_once_the_provider_is_back() {
    use radar_core::dock::Queued;
    let provider = FakeProvider::new();
    let mut p = plugin(&provider);
    start(&mut p);
    provider.disconnect();
    let off = until(&mut p, "the session to see the loss", |d| !d.composer.can_send);
    assert!(off.composer.disabled.is_some(), "the composer says why");
    // A send that arrives anyway (a renderer that had not heard yet) is kept, not dropped.
    p.dock_intent(&DockIntent::Send { text: "still there?".into() });
    assert_eq!(users(&p.dock().unwrap()), vec![("still there?".into(), Some(Queued::Here))]);
    provider.reconnect();
    let back = until(&mut p, "the answer after the reconnect", |d| say(d, "You said: still there?"));
    assert_eq!(users(&back), vec![("still there?".into(), None)]);
}

#[test]
fn slash_close_closes_the_session_showing_and_is_listed_among_the_commands() {
    let provider = FakeProvider::new();
    let mut p = plugin(&provider);
    let dock = two_tabs(&mut p);
    assert!(dock.composer.commands.iter().any(|c| c.name == "close"));
    let out = p.dock_intent(&DockIntent::Send { text: " /close ".into() });
    assert!(menu_of(out).is_none());
    assert_eq!(p.dock().unwrap().tabs.len(), 1);
}

/// Tick until `done` holds on the dock, counting the effects that run a task (a desktop notification is one).
fn until_counting(p: &mut SessionsPlugin, what: &str, done: impl Fn(&DockView) -> bool) -> (DockView, usize) {
    let mut spawned = 0;
    for n in 0..STEPS {
        spawned += p.tick(n as i64).iter().filter(|e| matches!(e, radar_core::Effect::Spawn(_))).count();
        if let Some(dock) = p.dock()
            && done(&dock)
        {
            return (dock, spawned);
        }
        p.run_tasks();
    }
    panic!("never reached: {what}; dock = {:?}", p.dock());
}

#[test]
fn a_turn_that_ends_out_of_sight_is_unread_until_its_tab_shows_and_the_activity_line_says_how_it_ended() {
    let provider = FakeProvider::new();
    let mut p = plugin(&provider);
    start(&mut p);
    // The host has not shown the dock: the turn ends out of sight.
    p.dock_intent(&DockIntent::Send { text: "hello".into() });
    let (done, spawned) = until_counting(&mut p, "the turn to end unread", |d| d.tabs[0].unread);
    assert_eq!(spawned, 0, "no desktop notification unless Settings → Agents asks for one");
    assert_eq!(done.activity.as_ref().map(|a| a.doing), Some(radar_core::dock::Doing::Done));
    assert_eq!(p.dock_tabs().unwrap().0, done.tabs, "the chip's cheap read agrees with the dock");
    // Shown on its tab: seen.
    p.dock_visible(true);
    assert!(!p.dock().unwrap().tabs[0].unread);
    // A turn that ends in sight is never unread; one that runs spins its tab and says what it does.
    p.dock_intent(&DockIntent::Send { text: "keep going long".into() });
    let running = until(&mut p, "a running turn", |d| d.composer.can_stop);
    assert!(running.tabs[0].running.is_some());
    assert!(running.activity.as_ref().is_some_and(|a| matches!(a.doing, radar_core::dock::Doing::Writing | radar_core::dock::Doing::Thinking)));
    p.dock_intent(&DockIntent::Stop);
    let stopped = until(&mut p, "the turn stopped", |d| d.activity.as_ref().is_some_and(|a| a.doing == radar_core::dock::Doing::Stopped));
    assert!(!stopped.tabs[0].unread && stopped.tabs[0].running.is_none());
}

#[test]
fn with_notifications_on_a_request_out_of_sight_goes_to_the_desktop_once() {
    let provider = FakeProvider::new();
    let mut p = plugin(&provider);
    let dir = std::env::temp_dir().join(format!("radar-notify-test-{}", std::process::id()));
    let table: toml::Value = toml::from_str(&format!("notify = true\nreviews_dir = {:?}\n", dir.display().to_string())).unwrap();
    p.configure(Some(&table)).unwrap();
    start(&mut p);
    p.dock_intent(&DockIntent::Send { text: "run something".into() });
    let (_, spawned) = until_counting(&mut p, "a pending request", |d| permission(d).is_some());
    let more = quiet(&mut p);
    assert_eq!(spawned + more, 1, "one notification for the request, not one a tick");
    let _ = std::fs::remove_dir_all(dir);
}
