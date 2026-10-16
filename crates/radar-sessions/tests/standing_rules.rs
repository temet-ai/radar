//! Standing allow and deny rules from Settings → Agents decide a matching
//! permission request; nothing else is answered for the operator.

use radar_core::Plugin;
use radar_core::dock::{DockIntent, DockItem, DockView, PermissionAction, ToolState};
use radar_sessions::SessionsPlugin;
use radar_sessions::fake::FakeProvider;

fn plugin(config: &str) -> SessionsPlugin {
    let provider = FakeProvider::new();
    // The plugin's tasks run on the test's thread, only when a step lets them: nothing races the
    // test, so no load on the machine makes it late.
    let mut p = SessionsPlugin::on_this_thread();
    p.add_adapter(Box::new(provider.adapter()));
    let table: toml::Value = toml::from_str(config).unwrap();
    p.configure(Some(&table)).unwrap();
    p
}

/// Tick and run the plugin's tasks until `done`: a bound on steps, not on time.
fn until(p: &mut SessionsPlugin, what: &str, done: impl Fn(&DockView) -> bool) -> DockView {
    for n in 0..256u64 {
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

fn permission(dock: &DockView) -> Option<Option<PermissionAction>> {
    dock.transcript.iter().find_map(|i| match i {
        DockItem::Permission(p) => Some(p.answered),
        _ => None,
    })
}

fn run(p: &mut SessionsPlugin) {
    p.dock_intent(&DockIntent::NewSession { provider: None });
    until(p, "a session", |d| d.composer.can_send && d.tabs.len() == 1);
    p.dock_intent(&DockIntent::Send { text: "run it".into() });
}

#[test]
fn a_deny_rule_refuses_the_request_without_asking() {
    let mut p = plugin("provider = \"fake\"\ndeny = [\"shell(ls)\"]\n");
    run(&mut p);
    let done = until(&mut p, "the denial", |d| matches!(permission(d), Some(Some(PermissionAction::Deny))));
    assert!(done.transcript.iter().any(|i| matches!(i, DockItem::Tool { state: ToolState::Denied, .. })));
    assert!(format!("{done:?}").contains("command ls"), "a line names the rule");
}

#[test]
fn an_allow_rule_approves_the_request_once_and_names_the_rule() {
    let mut p = plugin("provider = \"fake\"\nallow = [\"shell(ls)\"]\n");
    run(&mut p);
    let done = until(&mut p, "the approval", |d| matches!(permission(d), Some(Some(PermissionAction::ApproveOnce))));
    assert!(format!("{done:?}").contains("command ls"));
}

#[test]
fn approving_for_the_session_is_offered_beside_the_rules_and_a_deny_rule_still_stands() {
    // No rule matches: the request waits and the dock offers approve for session.
    let mut p = plugin("provider = \"fake\"\nallow = [\"shell(git diff)\"]\n");
    run(&mut p);
    let waiting = until(&mut p, "a pending request", |d| permission(d).is_some());
    let actions = waiting.transcript.iter().find_map(|i| if let DockItem::Permission(p) = i { Some(p.actions.clone()) } else { None }).unwrap();
    assert!(actions.contains(&PermissionAction::ApproveForSession), "{actions:?}");
    // A deny rule answers before anything radar remembers for the session.
    let mut p = plugin("provider = \"fake\"\ndeny = [\"shell(ls)\"]\n");
    run(&mut p);
    until(&mut p, "the denial", |d| matches!(permission(d), Some(Some(PermissionAction::Deny))));
}

#[test]
fn a_rule_that_does_not_match_leaves_the_request_waiting() {
    let mut p = plugin("provider = \"fake\"\nallow = [\"shell(git diff)\"]\ndeny = [\"read(/etc/*)\"]\n");
    run(&mut p);
    until(&mut p, "a pending request", |d| permission(d).is_some());
    // Every task run to its end, a few times over: still nothing answered it.
    for n in 0..8 {
        p.tick(n);
        p.run_tasks();
    }
    assert_eq!(permission(&p.dock().unwrap()), Some(None));
}

#[test]
fn always_allow_in_the_dock_approves_saves_the_rule_and_the_same_request_never_asks_again() {
    let mut p = plugin("provider = \"fake\"\n");
    run(&mut p);
    let waiting = until(&mut p, "a pending request", |d| permission(d).is_some());
    let (id, actions) = waiting
        .transcript
        .iter()
        .find_map(|i| if let DockItem::Permission(p) = i { Some((p.id.clone(), p.actions.clone())) } else { None })
        .unwrap();
    assert!(actions.contains(&PermissionAction::AlwaysAllow), "{actions:?}");
    p.dock_intent(&DockIntent::AlwaysAllow { id });
    let done = until(&mut p, "the approval", |d| matches!(permission(d), Some(Some(PermissionAction::ApproveOnce))));
    assert!(format!("{done:?}").contains("always allowed from now on: command ls"), "{done:?}");
    // On the allow list, so it is saved with the settings.
    let saved = p.config().unwrap().to_string();
    assert!(saved.contains("shell(ls)"), "{saved}");
    // A new session's same request is answered by the rule.
    p.dock_intent(&DockIntent::NewSession { provider: None });
    until(&mut p, "a second session", |d| d.composer.can_send && d.tabs.len() == 2);
    p.dock_intent(&DockIntent::Send { text: "run it".into() });
    until(&mut p, "the rule's approval", |d| matches!(permission(d), Some(Some(PermissionAction::ApproveOnce))) && format!("{d:?}").contains("by your allow rule command ls"));
}
