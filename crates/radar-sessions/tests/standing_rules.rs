//! Standing allow and deny rules from Settings → Agents decide a matching
//! permission request; nothing else is answered for the operator.

use radar_core::Plugin;
use radar_core::dock::{DockIntent, DockItem, DockView, PermissionAction, ToolState};
use radar_sessions::SessionsPlugin;
use radar_sessions::fake::FakeProvider;

fn plugin(config: &str) -> SessionsPlugin {
    let provider = FakeProvider::new();
    let mut p = SessionsPlugin::new();
    p.add_adapter(Box::new(provider.adapter()));
    let table: toml::Value = toml::from_str(config).unwrap();
    p.configure(Some(&table)).unwrap();
    p
}

fn until(p: &mut SessionsPlugin, what: &str, done: impl Fn(&DockView) -> bool) -> DockView {
    for n in 0..2_000_000u64 {
        p.tick(n as i64);
        if let Some(dock) = p.dock()
            && done(&dock)
        {
            return dock;
        }
        std::thread::yield_now();
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
    assert!(format!("{done:?}").contains("shell(ls)"), "a line names the rule");
}

#[test]
fn an_allow_rule_approves_the_request_once_and_names_the_rule() {
    let mut p = plugin("provider = \"fake\"\nallow = [\"shell(ls)\"]\n");
    run(&mut p);
    let done = until(&mut p, "the approval", |d| matches!(permission(d), Some(Some(PermissionAction::ApproveOnce))));
    assert!(format!("{done:?}").contains("shell(ls)"));
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
    for n in 0..300 {
        p.tick(n);
    }
    assert_eq!(permission(&p.dock().unwrap()), Some(None));
}
