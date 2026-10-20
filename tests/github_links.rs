//! Native composition with real parts and isolated local-folder fixtures.
#![cfg(all(
    feature = "github",
    feature = "git",
    feature = "jira",
    feature = "slack",
    feature = "demo"
))]
use radar_core::host::{Event, HostEffect};
use radar_core::view::{Chord, Key};
use radar_core::{Contribution, Host, Pane, Point};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

const PR: &str = "https://github.com/acme/api/pull/42";
/// What shows, as the view data's text form (`radar_core::headless`).
fn to_text(host: &Host, width: u16, height: u16) -> String {
    radar_core::headless::host_text_of(host, width, height)
}
fn press(host: &mut Host, code: Key) -> Vec<HostEffect> {
    let _ = to_text(host, 160, 40);
    host.update(Event::Intent(radar_core::view::Intent::Key { chord: Chord::new(code) }))
}
fn choose(host: &mut Host, label: &str) -> Vec<HostEffect> {
    let row = host
        .menu
        .as_ref()
        .unwrap()
        .menu
        .items
        .iter()
        .position(|r| r.label == label)
        .unwrap_or_else(|| panic!("missing menu row {label}"));
    host.menu.as_mut().unwrap().row = row;
    press(host, Key::Enter)
}
fn at(host: &Host, id: &str) -> usize {
    host.plugins.iter().position(|p| p.id() == id).unwrap()
}
fn fixture(config: &str) -> Host {
    let mut host = Host::new(
        vec![
            Box::<radar_github::GithubPlugin>::default(),
            Box::<radar_jira::JiraPlugin>::default(),
            Box::<radar_slack::SlackPlugin>::default(),
            Box::<radar_git::GitPlugin>::default(),
        ],
        radar_core::plugin::DEMO_NOW,
    )
    .unwrap();
    host.load_config(Some(config)).unwrap();
    let github = at(&host, "github");
    let jira = at(&host, "jira");
    let slack = at(&host, "slack");
    host.plugins[github].demo();
    host.plugins[jira].demo();
    let mut cache = host.plugins[jira].snapshot().unwrap();
    cache["data"]["acme"]["code"]["ABC-12"] = json!({
        "at":radar_core::plugin::DEMO_NOW,"updated":"", "links":[{
            "id":"#42","title":"Keep refreshes owned","url":PR,"status":"open",
            "repo":"acme/api","branch":"refresh-ownership"
        }]
    });
    host.plugins[jira].restore(cache);
    let cfg: radar_slack::Settings = serde_json::from_value(json!({
        "token":"env:NEVER_READ_SYNTHETIC", "channel":[{
            "alias":"team","id":"C0000TEAM1","name":"team","kind":"team","watched":true
        }]
    }))
    .unwrap();
    host.plugins[slack] = Box::new(radar_slack::test_support::offline_plugin(cfg, json!({
        "users":{"U000ALICE":"Alice Example"},
        "channels":{"C0000TEAM1":{"messages":[{"ts":"1791270000.000100","user":"U000ALICE","text":PR}]}}
    })).unwrap());
    press(&mut host, Key::Char('G'));
    host.focus = Pane::Middle;
    host
}
fn index(host: &mut Host, checkouts: serde_json::Value) {
    let git = at(host, "git");
    host.plugins[git].restore(json!({"index":{
        "checkouts":checkouts,"at":radar_core::plugin::DEMO_NOW,"folders":3,"cut":false,"missing":[]
    },"use":{}}));
}
fn checkout(dir: &str, remote: &str) -> serde_json::Value {
    json!({"dir":dir,"remote":remote,"project":"alice/api","branch":"main","files":[],"main":""})
}
fn subject(host: &Host) -> radar_core::Subject {
    host.plugins[at(host, "github")]
        .subject("github", Pane::Middle)
        .unwrap()
}
#[test]
fn real_parts_offer_only_source_folder_and_read_only_cached_relationships() {
    let mut host = fixture("");
    index(
        &mut host,
        json!([
            checkout("/work/wrong-host", "git@gitlab.com:alice/api.git"),
            checkout("/work/base", "https://github.com/acme/api.git"),
            checkout("/work/source", "git@GITHUB.COM:Alice/API.git"),
        ]),
    );
    let s = subject(&host);
    assert_eq!(s.id, PR);
    assert_eq!(
        s.field("source_repository_url"),
        Some("https://github.com/alice/api")
    );
    let git = at(&host, "git");
    let Some(Contribution::Line(line)) = host.plugins[git].contribute(Point::Line, &s) else {
        panic!()
    };
    let text = line.iter().map(|c| c.text.as_str()).collect::<String>();
    assert!(text.contains("/work/source"));
    assert!(!text.contains("wrong-host") && !text.contains("/work/base"));
    let text = to_text(&host, 160, 40);
    assert!(text.contains("local folder"), "{text}");
    assert!(text.contains("/work/source"), "{text}");
    assert!(text.contains("on main"), "{text}");
    press(&mut host, Key::Char('x'));
    let menu = &host.menu.as_ref().unwrap().menu;
    let labels: Vec<_> = menu.items.iter().map(|r| r.label.as_str()).collect();
    assert!(labels.contains(&"open current folder in an editor"));
    assert!(labels.contains(&"open terminal in current folder"));
    assert!(labels.contains(&"view ABC-12"));
    assert!(labels.contains(&"open discussion thread in #team"));
    assert!(!labels.iter().any(|s| s.contains("ask for review")
        || s.starts_with("move ABC")
        || s.starts_with("push ")
        || s.starts_with("check out ")));
    choose(&mut host, "open discussion thread in #team");
    assert_eq!(host.plugins[host.screen().unwrap().0].id(), "slack");
    press(&mut host, Key::Esc);
    assert_eq!(host.plugins[host.screen().unwrap().0].id(), "github");
    press(&mut host, Key::Char('x'));
    choose(&mut host, "view ABC-12");
    assert_eq!(host.plugins[host.screen().unwrap().0].id(), "jira");
    press(&mut host, Key::Esc);
    assert_eq!(host.plugins[host.screen().unwrap().0].id(), "github");
}

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "radar-github-links-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn repo(&self, name: &str, remote: &str) -> PathBuf {
        let dir = self.0.join(name);
        for args in [
            vec!["init", "-q", "-b", "main", dir.to_str().unwrap()],
            vec![
                "-C",
                dir.to_str().unwrap(),
                "remote",
                "add",
                "origin",
                remote,
            ],
        ] {
            let out = std::process::Command::new("git")
                .args(args)
                .env(
                    "GIT_CONFIG_GLOBAL",
                    if cfg!(windows) { "NUL" } else { "/dev/null" },
                )
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("HOME", &self.0)
                .env("XDG_CONFIG_HOME", &self.0)
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_CONFIG_COUNT")
                .env_remove("GIT_TEMPLATE_DIR")
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        dir
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn complete_git(host: &mut Host, effects: Vec<HostEffect>) -> Vec<HostEffect> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut out = vec![];
    for effect in effects {
        match effect {
            HostEffect::Spawn { plugin, task } => {
                assert_eq!(
                    host.plugins[plugin].id(),
                    "git",
                    "never execute provider tasks in this fixture"
                );
                out.extend(host.update(Event::Msg {
                    plugin,
                    msg: runtime.block_on(task),
                }));
            }
            other => out.push(other),
        }
    }
    out
}
fn link(host: &mut Host, dir: &Path) -> Vec<HostEffect> {
    press(host, Key::Char('x'));
    choose(host, "link a matching local folder…");
    host.update(Event::Paste(dir.display().to_string()));
    let fx = press(host, Key::Enter);
    complete_git(host, fx)
}
#[test]
fn manual_source_link_uses_real_git_folder_saves_reload_and_preserves_legacy_link() {
    let scratch = Scratch::new();
    let wrong = scratch.repo("wrong", "https://gitlab.com/alice/api.git");
    let source = scratch.repo("source", "ssh://git@github.com/Alice/API.git");
    let config = format!(
        "[[git.link]]\nrepo = 'legacy/team'\ndir = {}\n",
        serde_json::to_string(&scratch.0.join("legacy").display().to_string()).unwrap()
    );
    let mut host = fixture(&config);
    let fx = link(&mut host, &wrong);
    assert!(!fx.iter().any(|e| matches!(e, HostEffect::Save(_))));
    assert!(host.message.as_deref().unwrap().contains("origin"));
    let fx = link(&mut host, &source);
    let saved = fx
        .iter()
        .find_map(|e| {
            if let HostEffect::Save(s) = e {
                Some(s.clone())
            } else {
                None
            }
        })
        .expect("real Host saves the new folder link");
    assert!(saved.contains("https://github.com/alice/api"));
    assert!(saved.contains("legacy/team"));
    assert!(!saved.contains("https://github.com/acme/api\"\n"));
    let file = scratch.0.join("config.toml");
    std::fs::write(&file, &saved).unwrap();
    let mut reloaded = Host::new(
        vec![
            Box::<radar_github::GithubPlugin>::default(),
            Box::<radar_git::GitPlugin>::default(),
        ],
        radar_core::plugin::DEMO_NOW,
    )
    .unwrap();
    reloaded
        .load_config(Some(&std::fs::read_to_string(file).unwrap()))
        .unwrap();
    let cfg = reloaded.plugins[at(&reloaded, "git")]
        .config()
        .unwrap()
        .to_string();
    assert!(cfg.contains("https://github.com/alice/api") && cfg.contains("legacy/team"));
    let git = at(&host, "git");
    let Some(Contribution::Line(line)) = host.plugins[git].contribute(Point::Line, &subject(&host))
    else {
        panic!()
    };
    // The Git part shows a folder under the home directory as `~/…`, and a
    // scratch folder is under it when TMPDIR is.
    let full = source.to_str().unwrap().to_string();
    let shown = std::env::var("HOME")
        .ok()
        .and_then(|home| full.strip_prefix(&home).map(|rest| format!("~{rest}")))
        .unwrap_or_else(|| full.clone());
    assert!(
        line.iter().any(|c| c.text.contains(&shown)),
        "no chip shows {shown}: {:?}",
        line.iter().map(|c| c.text.as_str()).collect::<Vec<_>>()
    );
}

#[test]
fn integrated_preview_and_context_menu_render_at_both_widths() {
    for (w, h) in [(80, 24), (160, 40)] {
        let mut host = fixture("");
        index(
            &mut host,
            json!([checkout("/work/source", "git@github.com:alice/api.git")]),
        );
        for (name, menu) in [("integrated_preview", false), ("integrated_actions", true)] {
            if menu {
                press(&mut host, Key::Char('x'));
            }
            let text = to_text(&host, w, h);
            assert_eq!(host.plugins[host.screen().unwrap().0].id(), "github");
            if !menu {
                assert!(text.contains("── status · GITHUB ──"), "{text}");
            }
            if menu {
                assert!(text.contains("\n  # Git\n"), "git's heading in the menu:\n{text}");
            }
            insta::assert_snapshot!(format!("{name}_{w}x{h}"), text);
            if let Ok(dir) = std::env::var("RADAR_GITHUB_LINKED_RENDER_DIR") {
                let dir = PathBuf::from(dir);
                std::fs::create_dir_all(&dir).unwrap();
                let stem = format!("{name}_{w}x{h}");
                std::fs::write(dir.join(format!("{stem}.txt")), &text).unwrap();
            }
        }
    }
}
