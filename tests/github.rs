//! The new part remains independently routable in the real compiled part mix.
#![cfg(feature = "github")]

use radar_core::host::{Event, View};
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use radar_core::{Host, Plugin, Showing, Subject};

fn host() -> Host {
    let mut host = Host::new(radar::parts(), 1_791_271_000).unwrap();
    host.load_config(Some("[github]\nrepositories = ['acme/api']\ntoken = ''\n"))
        .unwrap();
    host
}

#[test]
fn feature_registers_a_public_read_part_without_stealing_history() {
    let mut host = host();
    assert_eq!(radar::mix().iter().filter(|&&id| id == "github").count(), 1);
    let index = host
        .plugins
        .iter()
        .position(|p| p.id() == "github")
        .unwrap();
    assert_eq!(host.plugins[index].set_up(), radar_core::Setup::Ready);
    host.update(Event::Key(KeyEvent::new(
        KeyCode::Char('G'),
        KeyModifiers::NONE,
    )));
    let View::Screen(screen) = host.view else {
        panic!("G should open the part");
    };
    assert_eq!(host.screens[screen].0, index);
    let screen_before = host.view;
    host.update(Event::Key(KeyEvent::new(
        KeyCode::Char('H'),
        KeyModifiers::NONE,
    )));
    // The overlay keeps the underlying screen; H is never a part screen key.
    assert_eq!(host.view, screen_before);
    assert!(host.screens.iter().all(|(_, s)| s.key != 'H'));
}

#[test]
fn recognised_host_qualified_subject_has_exact_history_identity() {
    let mut host = host();
    let text = "See https://github.com/Acme/API/pull/17";
    let subjects = radar_core::compose::recognise(&host.plugins, text);
    assert_eq!(subjects.len(), 1);
    let subject = &subjects[0].1;
    assert_eq!(subject.id, "https://github.com/acme/api/pull/17");
    let (owner, _) = radar_core::compose::owner_of(&host.plugins, &subject.kind).unwrap();
    assert_eq!(host.plugins[owner].id(), "github");
    let recorded = host.plugins[owner].journal_subject(subject).unwrap();
    assert_eq!(recorded.key.provider, "github");
    assert_eq!(recorded.key.instance, "https://github.com/");
    assert_eq!(recorded.key.kind, "pull-request");
    assert_eq!(recorded.key.id, "acme/api/pull/17");
    assert!(matches!(
        host.plugins[owner].show(subject),
        Showing::Page { .. }
    ));
    let legacy = Subject::new("merge-request", "acme/api!17", "same path elsewhere");
    assert_ne!(subject, &legacy);
    assert!(matches!(host.plugins[owner].show(&legacy), Showing::Cannot));
    // The configured allowlist is checked even for a structurally valid URL.
    let unconfigured = host.plugins[owner]
        .recognise("https://github.com/elsewhere/api/pull/17")
        .pop()
        .unwrap()
        .1;
    assert!(matches!(
        host.plugins[owner].show(&unconfigured),
        Showing::Cannot
    ));
}

#[test]
fn no_provider_cache_is_restored_from_an_unverified_payload() {
    let mut plugin = radar_github::GithubPlugin::default();
    let configured = host()
        .plugins
        .into_iter()
        .find(|p| p.id() == "github")
        .unwrap()
        .config()
        .unwrap();
    plugin.configure(Some(&configured)).unwrap();
    plugin.restore(serde_json::json!({"token":"never-a-credential","private":"cached contents"}));
    // The provider keeps a cache of what it read, but a payload it did not
    // write is discarded: nothing of it, least of all a credential, is kept.
    let kept = plugin.snapshot().map(|v| v.to_string()).unwrap_or_default();
    assert!(!kept.contains("never-a-credential"), "{kept}");
    assert!(!kept.contains("cached contents"), "{kept}");
}

#[test]
fn cli_help_and_unconfigured_list_are_offline_and_clear() {
    use std::process::Command;
    let root = std::env::temp_dir().join(format!("radar-github-cli-{}", std::process::id()));
    std::fs::create_dir_all(root.join("radar")).unwrap();
    let config = root.join("radar").join("config.toml");
    std::fs::write(&config, "[github]\ntoken = ''\nrepositories = []\n").unwrap();
    let command = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_radar"))
            .args(args)
            .env_clear()
            .env("RADAR_SANDBOX", "1")
            .env("RADAR_MODE", "read-only")
            .env("RADAR_NO_WRITES", "1")
            .env("XDG_CONFIG_HOME", &root)
            .env("XDG_STATE_HOME", &root)
            .env("HOME", &root)
            .env("APPDATA", &root)
            .env("LOCALAPPDATA", &root)
            .output()
            .unwrap()
    };
    let help = command(&["github", "list", "--help"]);
    assert!(
        help.status.success(),
        "{}",
        String::from_utf8_lossy(&help.stderr)
    );
    assert!(String::from_utf8_lossy(&help.stdout).contains("--json"));
    let empty = command(&["github", "list", "--json"]);
    assert!(!empty.status.success());
    assert!(String::from_utf8_lossy(&empty.stderr).contains("No GitHub repositories configured"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn workflow_attempts_have_native_scoped_ownership_without_legacy_collisions() {
    let mut host = host();
    let url = "https://github.com/acme/api/actions/runs/501/attempts/2";
    let recognised = radar_core::compose::recognise(&host.plugins, url);
    assert_eq!(recognised.len(), 1);
    let subject = &recognised[0].1;
    assert_eq!(subject.kind, "workflow-run");
    assert_eq!(subject.id, url);
    let (owner, _) = radar_core::compose::owner_of(&host.plugins, &subject.kind).unwrap();
    assert_eq!(host.plugins[owner].id(), "github");
    let recorded = host.plugins[owner].journal_subject(subject).unwrap();
    assert_eq!(recorded.key.provider, "github");
    assert_eq!(recorded.key.instance, "https://github.com/");
    assert_eq!(recorded.key.kind, "workflow-run");
    assert_eq!(recorded.key.id, "acme/api/actions/runs/501/attempts/2");
    assert!(matches!(
        host.plugins[owner].show(subject),
        Showing::Page { .. }
    ));
    for (index, part) in host.plugins.iter().enumerate() {
        if part.id() == "workbench" {
            let Some(radar_core::Contribution::Actions(actions)) =
                part.contribute(radar_core::Point::Actions, subject)
            else {
                panic!("explicit local annotation contribution")
            };
            assert_eq!(actions.len(), 1);
            assert_eq!(actions[0].id, "work.link");
            assert!(part.owns().iter().all(|owned| owned.kind != "workflow-run"));
        } else if index != owner {
            assert!(part.contribute(radar_core::Point::Actions, subject).is_none());
        }
    }
    let prior = host.plugins[owner]
        .recognise("https://github.com/acme/api/actions/runs/501/attempts/1")
        .pop()
        .unwrap()
        .1;
    assert_ne!(
        recorded.key,
        host.plugins[owner].journal_subject(&prior).unwrap().key
    );
    let mut forged = subject.clone();
    forged.fields.insert("attempt".into(), "1".into());
    assert!(matches!(host.plugins[owner].show(&forged), Showing::Cannot));
    assert!(host.plugins[owner].journal_subject(&forged).is_none());
    let elsewhere = host.plugins[owner]
        .recognise("https://github.com/elsewhere/api/actions/runs/501/attempts/2")
        .pop()
        .unwrap()
        .1;
    assert!(matches!(
        host.plugins[owner].show(&elsewhere),
        Showing::Cannot
    ));
}

#[cfg(feature = "demo")]
#[test]
fn native_actions_navigation_keeps_checks_and_runs_separate_in_the_real_mix() {
    use radar_core::{Pane, host::HostEffect};
    let mut host = host();
    let index = host
        .plugins
        .iter()
        .position(|p| p.id() == "github")
        .unwrap();
    host.plugins[index].demo();
    let press = |host: &mut Host, code| {
        let _ = radar_core::headless::host_text_of(host, 160, 40);
        host.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
    };
    press(&mut host, KeyCode::Char('G'));
    host.focus = Pane::Middle;
    press(&mut host, KeyCode::Up);
    press(&mut host, KeyCode::Enter);
    press(&mut host, KeyCode::Char(']'));
    press(&mut host, KeyCode::Char(']'));
    let shown = radar_core::headless::host_text_of(&host, 160, 40);
    assert!(shown.contains("Actions"), "{shown}");
    assert!(shown.contains("attempt"), "{shown}");
    host.focus = Pane::Middle;
    let selected = host.plugins[index].subject("github", Pane::Middle).unwrap();
    assert_eq!(selected.kind, "workflow-run");
    let before = selected.id.clone();
    press(&mut host, KeyCode::Enter);
    let opened = host.plugins[index].subject("github", Pane::Middle).unwrap();
    assert_eq!(opened.id, before);
    assert_eq!(opened.kind, "workflow-run");
    press(&mut host, KeyCode::Char('x'));
    let menu = &host.menu.as_ref().unwrap().menu;
    for item in &menu.items {
        let label = item.label.to_ascii_lowercase();
        assert!(
            !label.contains("rerun")
                && !label.contains("re-run")
                && !label.contains("cancel workflow")
                && !label.contains("dispatch")
        );
    }
    press(&mut host, KeyCode::Esc);
    let effects = press(&mut host, KeyCode::Char('o'));
    fn opened_url(effect: &HostEffect) -> Option<&str> {
        match effect {
            HostEffect::Open(url) => Some(url),
            HostEffect::Tracked { effect, .. } => opened_url(effect),
            _ => None,
        }
    }
    assert!(
        effects
            .iter()
            .any(|e| opened_url(e) == Some(before.as_str()))
    );
    press(&mut host, KeyCode::Esc);
    let restored = host.plugins[index].subject("github", Pane::Middle).unwrap();
    assert_eq!(restored.id, before);
    press(&mut host, KeyCode::Esc);
    assert!(matches!(host.view, View::Screen(_)));
}
