//! What starts when, what the pickers show at once, and the session kept ready, on the fake
//! provider. Nothing waits on a clock: ticks carry the host's time, so freshness is a number
//! the test chooses, and a wait is a bounded spin on what the fake counted.

use radar_core::Plugin;
use radar_core::dock::{DockIntent, DockView, PickerKind};
use radar_sessions::fake::FakeProvider;
use radar_sessions::warm::{CATALOG_FRESH, MODELS_FRESH};
use radar_sessions::{SessionsPlugin, SlashCommand};

/// The plugin's tasks run on the test's thread, only when a step lets them: nothing races the test.
fn plugin(providers: &[&FakeProvider]) -> SessionsPlugin {
    let mut p = SessionsPlugin::on_this_thread();
    for f in providers {
        p.add_adapter(Box::new(f.adapter()));
    }
    p
}

/// Tick at host time `now` and run the plugin's tasks until `done`; fail after a bounded number of steps.
fn until(p: &mut SessionsPlugin, now: i64, what: &str, done: impl Fn(&SessionsPlugin) -> bool) {
    for _ in 0..256 {
        p.tick(now);
        if done(p) {
            return;
        }
        p.run_tasks();
    }
    panic!("never reached: {what}; dock = {:?}", p.dock());
}

/// Let the background tasks that were going to run do so: each step runs every task to its end.
fn quiet(p: &mut SessionsPlugin, now: i64) {
    for _ in 0..8 {
        p.tick(now);
        p.run_tasks();
    }
}

fn dock(p: &SessionsPlugin) -> DockView {
    p.dock().expect("a dock")
}

fn picker(p: &mut SessionsPlugin, kind: PickerKind) -> radar_core::dock::DockPicker {
    p.dock_intent(&DockIntent::OpenPicker { kind });
    dock(p).picker.expect("an open picker")
}

/// The saved state of a first run, without its sessions: what a restart finds besides them.
fn cache_only(p: &SessionsPlugin) -> serde_json::Value {
    let mut snap = p.snapshot().expect("something to save");
    snap.as_object_mut().unwrap().retain(|k, _| k == "warm");
    snap
}

/// A plugin whose default provider has started, signed in, and whose models are in (they are
/// in the saved state once the part has them).
fn started(f: &FakeProvider, now: i64) -> SessionsPlugin {
    let mut p = plugin(&[f]);
    until(&mut p, now, "the models", |p| {
        p.snapshot().is_some_and(|s| s["warm"]["models"]["fake"]["value"].as_array().is_some_and(|m| m.len() == 2))
    });
    p
}

#[test]
fn launch_starts_the_default_provider_only_and_the_others_wait_until_something_needs_them() {
    let (a, b) = (FakeProvider::new(), FakeProvider::named("second"));
    let mut p = plugin(&[&a, &b]);
    until(&mut p, 1000, "the default provider's models", |_| a.calls("list_models") == 1);
    quiet(&mut p, 1000);
    assert_eq!((a.calls("warm"), a.calls("status")), (1, 1), "the default provider's client started once");
    assert_eq!((b.calls("warm"), b.calls("status"), b.calls("list_models")), (0, 0, 0), "nothing started the other one");
    assert_eq!((a.calls("create"), a.calls("catalog"), a.calls("list_sessions")), (0, 0, 0), "the dock was never opened");

    // A new tab on the other provider starts it, and the tab comes up.
    p.dock_intent(&DockIntent::NewSession { provider: Some("second".into()) });
    until(&mut p, 1000, "the second provider's tab", |_| b.calls("warm") == 1 && b.calls("create") == 1 && b.calls("status") == 1);
    until(&mut p, 1000, "the tab takes text", |p| dock(p).composer.can_send);
}

#[test]
fn settings_agents_asks_the_clis_nobody_has_asked() {
    let (a, b) = (FakeProvider::new(), FakeProvider::named("second"));
    let mut p = plugin(&[&a, &b]);
    until(&mut p, 1000, "the default provider's models", |_| a.calls("list_models") == 1);
    quiet(&mut p, 1000);
    assert_eq!(b.calls("status"), 0);
    let cx = radar_core::view::ViewCx {
        now: 0,
        focus: radar_core::view::PaneId::Middle,
        width: 80,
        height: 24,
        parts: radar_core::Parts::none(),
        icon_lead: 0,
        setup: false,
    };
    p.section_view(radar_sessions::settings::SECTION, &cx);
    until(&mut p, 1000, "the other CLI's status", |_| b.calls("status") == 1);
    quiet(&mut p, 1000);
    assert_eq!((b.calls("status"), b.calls("warm"), b.calls("list_models")), (1, 0, 0), "its status, nothing more");
    assert_eq!(a.calls("status"), 1, "the default one is not asked twice");
}

#[test]
fn a_cli_that_is_signed_out_is_not_asked_for_models_catalogs_or_a_session() {
    let a = FakeProvider::new();
    a.set_signed_in(false);
    a.set_idle_free(true);
    let mut p = plugin(&[&a]);
    until(&mut p, 1000, "its status", |_| a.calls("status") == 1);
    p.dock_visible(true);
    quiet(&mut p, 1000);
    assert_eq!((a.calls("list_models"), a.calls("catalog"), a.calls("create")), (0, 0, 0));
}

#[test]
fn models_come_from_the_cache_at_once_and_are_read_again_only_when_they_are_old() {
    let a = FakeProvider::new();
    let mut first = started(&a, 1000);
    let saved = cache_only(&first);
    first.tick(1000);

    // A restart within the hour: the picker is full before anything ticked, and nothing is asked.
    let b = FakeProvider::new();
    let mut p = plugin(&[&b]);
    p.restore(saved.clone());
    let at = 1000 + MODELS_FRESH - 1;
    p.tick(at);
    let open = picker(&mut p, PickerKind::Model);
    assert_eq!(open.choices.len(), 2, "shown from the cache");
    assert!(!open.reading);
    quiet(&mut p, at);
    assert_eq!(b.calls("list_models"), 0, "an hour has not passed");

    // Later: still shown at once, and read again in the background.
    let c = FakeProvider::new();
    let mut p = plugin(&[&c]);
    p.restore(saved);
    let later = 1000 + MODELS_FRESH;
    p.tick(later);
    assert_eq!(picker(&mut p, PickerKind::Model).choices.len(), 2);
    until(&mut p, later, "the models read again", |_| c.calls("list_models") >= 1);
}

#[test]
fn the_agent_and_skill_lists_are_shown_from_what_was_read_and_read_again_when_old() {
    let a = FakeProvider::new();
    let mut p = plugin(&[&a]);
    p.tick(1000);
    // First time: nothing to show yet, so the picker says it is reading, then fills in.
    let open = picker(&mut p, PickerKind::Agent);
    assert!(open.reading && open.choices.is_empty());
    until(&mut p, 1000, "the catalog", |p| dock(p).picker.is_some_and(|k| !k.reading));
    assert_eq!(a.calls("catalog"), 1);
    p.dock_intent(&DockIntent::ClosePicker);

    // Soon after: shown at once, the CLI not asked again.
    p.tick(1000 + CATALOG_FRESH - 1);
    let open = picker(&mut p, PickerKind::Agent);
    assert!(!open.reading && open.choices.len() > 1, "{open:?}");
    quiet(&mut p, 1000 + CATALOG_FRESH - 1);
    assert_eq!(a.calls("catalog"), 1);
    p.dock_intent(&DockIntent::ClosePicker);

    // Later: shown at once and read again.
    let late = 1000 + CATALOG_FRESH;
    p.tick(late);
    assert!(!picker(&mut p, PickerKind::Skill).reading, "the old list is still shown");
    until(&mut p, late, "the catalog read again", |_| a.calls("catalog") == 2);

    // After a restart the first open is instant too.
    let saved = cache_only(&p);
    let b = FakeProvider::new();
    let mut p = plugin(&[&b]);
    p.restore(saved);
    p.tick(late + 1);
    let open = picker(&mut p, PickerKind::Agent);
    assert!(!open.reading && open.choices.len() > 1);
}

#[test]
fn a_catalog_read_for_other_folders_is_not_shown_for_this_one() {
    let a = FakeProvider::new();
    let mut p = plugin(&[&a]);
    p.tick(1000);
    picker(&mut p, PickerKind::Skill);
    until(&mut p, 1000, "the catalog kept", |p| p.snapshot().is_some_and(|s| s["warm"]["catalogs"].as_object().is_some_and(|c| !c.is_empty())));
    let saved = cache_only(&p);
    // The same cache, but this workspace works in another folder: its key differs, so the CLI is asked.
    let b = FakeProvider::new();
    let mut q = plugin(&[&b]);
    q.restore(saved);
    q.configure(Some(&toml::from_str::<toml::Value>("work_dir = \"/somewhere/else\"\n").unwrap())).unwrap();
    q.tick(1001);
    picker(&mut q, PickerKind::Skill);
    until(&mut q, 1001, "the catalog for the other folder", |_| b.calls("catalog") == 1);
}

#[test]
fn the_commands_a_session_offered_are_in_the_menu_of_the_next_one_before_it_has_said_them() {
    let a = FakeProvider::new();
    a.set_commands(vec![SlashCommand::new("plan", "make a plan")]);
    let mut p = plugin(&[&a]);
    p.dock_intent(&DockIntent::NewSession { provider: None });
    until(&mut p, 1000, "the session and its commands", |p| {
        let d = dock(p);
        d.composer.can_send && d.composer.commands.iter().any(|c| c.name == "plan")
    });
    let saved = cache_only(&p);

    // A new run, a provider that has not said its commands yet: the menu is the remembered one.
    let b = FakeProvider::new();
    let mut p = plugin(&[&b]);
    p.restore(saved);
    p.dock_intent(&DockIntent::NewSession { provider: None });
    let names: Vec<String> = dock(&p).composer.commands.iter().map(|c| c.name.clone()).collect();
    assert!(names.contains(&"plan".to_string()), "{names:?}");
}

/// The default provider started, signed in, with the dock open and its session made.
fn with_spare(f: &FakeProvider) -> SessionsPlugin {
    f.set_idle_free(true);
    let mut p = started(f, 1000);
    assert_eq!(f.calls("create"), 0, "a closed dock makes no session");
    p.dock_visible(true);
    until(&mut p, 1000, "a session made ahead", |_| f.calls("create") == 1);
    p
}

#[test]
fn a_session_is_made_ahead_when_the_dock_opens_and_stays_out_of_sight_and_out_of_the_saved_state() {
    let a = FakeProvider::new();
    a.set_commands(vec![SlashCommand::new("plan", "make a plan")]);
    let mut p = with_spare(&a);
    quiet(&mut p, 1000);
    assert_eq!(a.calls("create"), 1, "one, not one a tick");
    assert!(dock(&p).tabs.is_empty(), "no tab for it");
    assert_eq!(p.snapshot().unwrap()["sessions"].as_array().map(Vec::len), Some(0), "nothing about it is saved");
    assert!(p.store().sessions().next().is_none());
}

#[test]
fn the_new_tab_takes_the_session_made_ahead_and_attaches_at_once() {
    let a = FakeProvider::new();
    a.set_commands(vec![SlashCommand::new("plan", "make a plan")]);
    let mut p = with_spare(&a);
    until(&mut p, 1000, "its commands", |p| p.snapshot().is_some_and(|s| s["warm"]["commands"].as_object().is_some_and(|c| !c.is_empty())));
    quiet(&mut p, 1000);
    p.dock_intent(&DockIntent::NewSession { provider: None });
    // Before the next tick: the tab is there with its commands, and nothing was created for it.
    let d = dock(&p);
    assert_eq!(d.tabs.len(), 1);
    assert!(d.composer.commands.iter().any(|c| c.name == "plan"));
    until(&mut p, 1000, "the tab takes text", |p| dock(p).composer.can_send);
    // The next one is made so the tab after this is as quick; the tab itself made none.
    until(&mut p, 1000, "the next one", |_| a.calls("create") == 2);
    quiet(&mut p, 1000);
    assert_eq!(a.calls("create"), 2);
    assert!(dock(&p).tabs.len() == 1, "the new spare is not a tab");

    p.dock_intent(&DockIntent::Send { text: "hello".into() });
    until(&mut p, 1000, "the answer", |p| {
        dock(p).transcript.iter().any(|i| matches!(i, radar_core::dock::DockItem::Assistant { text, saved: true, .. } if text == "You said: hello"))
    });
    // A second tab takes the second session.
    p.dock_intent(&DockIntent::NewSession { provider: None });
    assert_eq!(dock(&p).tabs.len(), 2);
    until(&mut p, 1000, "the third", |_| a.calls("create") == 3);
}

#[test]
fn a_session_made_ahead_that_no_longer_fits_is_discarded_and_replaced() {
    let a = FakeProvider::new();
    let mut p = with_spare(&a);
    p.dock_intent(&DockIntent::PickModel { id: "fake-large".into() });
    until(&mut p, 1000, "the old one discarded and a new one made", |_| a.discarded().len() == 1 && a.calls("create") == 2);
    assert_eq!(a.session_count(), 1, "the provider holds only the new one");
    p.dock_intent(&DockIntent::NewSession { provider: None });
    until(&mut p, 1000, "the tab takes text", |p| dock(p).composer.can_send);
    assert_eq!(dock(&p).status.model, "fake-large");
    // The tab made no session of its own: the only one made after it is the next spare.
    until(&mut p, 1000, "the next spare", |_| a.calls("create") == 3);
    quiet(&mut p, 1000);
    assert_eq!(a.calls("create"), 3);
}

#[test]
fn a_tab_that_differs_from_the_session_made_ahead_makes_its_own() {
    let a = FakeProvider::new();
    let mut p = with_spare(&a);
    // The model is picked and a tab opened before any tick could notice the difference.
    p.dock_intent(&DockIntent::PickModel { id: "fake-large".into() });
    p.dock_intent(&DockIntent::NewSession { provider: None });
    until(&mut p, 1000, "the tab takes text", |p| dock(p).composer.can_send);
    assert_eq!(dock(&p).status.model, "fake-large");
    until(&mut p, 1000, "the stale one discarded", |_| a.discarded().len() == 1);
}

#[test]
fn a_provider_whose_idle_session_shows_in_its_list_keeps_none_ready() {
    let a = FakeProvider::new();
    let mut p = started(&a, 1000);
    p.dock_visible(true);
    quiet(&mut p, 1000);
    assert_eq!(a.calls("create"), 0, "its sessions are made when a tab asks");
    p.dock_intent(&DockIntent::NewSession { provider: None });
    until(&mut p, 1000, "the tab", |_| a.calls("create") == 1);
    quiet(&mut p, 1000);
    assert_eq!(a.calls("create"), 1);
}

#[test]
fn opening_the_dock_reads_the_default_providers_lists_in_the_background_once() {
    let a = FakeProvider::new();
    let mut p = started(&a, 1000);
    quiet(&mut p, 1000);
    assert_eq!((a.calls("catalog"), a.calls("list_sessions")), (0, 0));
    p.dock_visible(true);
    until(&mut p, 1000, "its lists", |_| a.calls("catalog") == 1 && a.calls("list_sessions") == 1);
    p.dock_visible(true);
    quiet(&mut p, 1000);
    assert_eq!((a.calls("catalog"), a.calls("list_sessions")), (1, 1), "once");
}

#[test]
fn the_session_picker_keeps_each_providers_sessions_apart() {
    let (a, b) = (FakeProvider::new(), FakeProvider::named("second"));
    let mut p = plugin(&[&a, &b]);
    p.dock_intent(&DockIntent::NewSession { provider: None });
    p.dock_intent(&DockIntent::NewSession { provider: Some("second".into()) });
    until(&mut p, 1000, "both sessions", |_| a.calls("create") == 1 && b.calls("create") == 1);
    p.dock_intent(&DockIntent::OpenPicker { kind: PickerKind::Session });
    until(&mut p, 1000, "both listed", |p| dock(p).picker.is_some_and(|k| k.choices.len() == 2));
    let ids: Vec<String> = dock(&p).picker.unwrap().choices.into_iter().map(|c| c.id).collect();
    assert!(ids.iter().any(|i| i.starts_with("fake:")) && ids.iter().any(|i| i.starts_with("second:")), "{ids:?}");
}
