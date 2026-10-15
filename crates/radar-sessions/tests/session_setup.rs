//! What Settings → Agents and the recipes say reaches the CLI: the provider's
//! place and state folder, the folders, the agent and the skills of every
//! session, and the agent chosen in the dock.

use std::path::PathBuf;

use radar_core::Plugin;
use radar_core::dock::{DockIntent, DockView, PickerKind};
use radar_sessions::SessionsPlugin;
use radar_sessions::adapter::{ProviderSetup, SessionSetup};
use radar_sessions::fake::FakeProvider;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("radar-setup-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn plugin(provider: &FakeProvider, config: &str) -> SessionsPlugin {
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

/// The setup of the `n`th session the fake was asked to create or resume.
fn setup(provider: &FakeProvider, p: &mut SessionsPlugin, n: usize) -> SessionSetup {
    for t in 0..2_000_000u64 {
        if let Some(s) = provider.setups().get(n) {
            return s.clone();
        }
        p.tick(t as i64);
        std::thread::yield_now();
    }
    panic!("no session {n} was created");
}

#[test]
fn the_settings_reach_the_cli_and_every_session() {
    let dir = scratch("settings");
    let (work, extra, skills, own, plugins) = (dir.join("work"), dir.join("extra"), dir.join("skills"), dir.join("own-skills"), dir.join("plugins"));
    for d in [&work, &extra, &skills, &own, &plugins] {
        std::fs::create_dir_all(d).unwrap();
    }
    let provider = FakeProvider::new();
    let config = format!(
        r#"
        provider = "fake"
        work_dir = "{work}"
        permitted_roots = ["{extra}"]
        skill_dirs = ["{skills}", "not/absolute"]
        project_config = false

        [providers.fake]
        executable = "/opt/fake/bin/fake"
        config_dir = "{dir}/fake-home"
        agent = "writer"
        skill_dirs = ["{own}"]
        plugin_dirs = ["{plugins}"]
        "#,
        work = work.display(),
        extra = extra.display(),
        skills = skills.display(),
        own = own.display(),
        plugins = plugins.display(),
        dir = dir.display(),
    );
    let mut p = plugin(&provider, &config);
    // The CLI's place and state folder, from the settings.
    assert_eq!(
        provider.provider_setups().last(),
        Some(&ProviderSetup { executable: Some("/opt/fake/bin/fake".into()), config_dir: Some(dir.join("fake-home")) })
    );
    p.dock_intent(&DockIntent::NewSession { provider: None });
    let s = setup(&provider, &mut p, 0);
    assert_eq!(s.extra_dirs, vec![extra]);
    assert_eq!(s.skill_dirs, vec![skills, own], "the shared skill folders, then the CLI's own; a relative path is left out");
    assert_eq!(s.plugin_dirs, vec![plugins]);
    assert_eq!(s.agent.as_deref(), Some("writer"));
    assert!(!s.project_config && s.local_only);
    let dock = until(&mut p, "the session", |d| d.composer.can_send);
    assert_eq!(dock.status.cwd, work.display().to_string(), "a dock session works in the working folder");
    assert_eq!(dock.status.agent, "writer");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_agent_chosen_in_the_dock_starts_the_next_session() {
    let provider = FakeProvider::new();
    let mut p = plugin(&provider, "provider = \"fake\"");
    p.dock_intent(&DockIntent::OpenPicker { kind: PickerKind::Agent });
    let dock = until(&mut p, "the agents the CLI offers", |d| d.picker.as_ref().is_some_and(|k| k.choices.len() == 3));
    let labels: Vec<_> = dock.picker.unwrap().choices.into_iter().map(|c| c.label).collect();
    assert_eq!(labels, vec!["the CLI's own agent", "reviewer", "writer"]);
    p.dock_intent(&DockIntent::PickAgent { id: "reviewer".into() });
    p.dock_intent(&DockIntent::NewSession { provider: None });
    assert_eq!(setup(&provider, &mut p, 0).agent.as_deref(), Some("reviewer"));
    // The skills picker lists what the CLI loaded.
    p.dock_intent(&DockIntent::OpenPicker { kind: PickerKind::Skill });
    let dock = until(&mut p, "the skills", |d| d.picker.as_ref().is_some_and(|k| k.kind == PickerKind::Skill && !k.choices.is_empty()));
    assert_eq!(dock.picker.unwrap().choices[0].id, "notes");
    // The CLI's own agent again.
    p.dock_intent(&DockIntent::PickAgent { id: String::new() });
    p.dock_intent(&DockIntent::NewSession { provider: None });
    assert_eq!(setup(&provider, &mut p, 1).agent, None);
}

#[test]
fn an_own_ask_recipe_names_the_agent_and_skills_of_the_session_it_starts() {
    let recipes = scratch("recipes");
    let ask = radar_sessions::brief::recipe::BUILT_IN[0].1;
    let own = radar_sessions::brief::Recipe::with_agent_and_skills(ask, Some("fake=reviewer, codex=other"), &["notes".into()]);
    std::fs::write(recipes.join("ask.md"), own).unwrap();
    let provider = FakeProvider::new();
    let mut p = plugin(&provider, &format!("provider = \"fake\"\nrecipes_dir = \"{}\"", recipes.display()));
    p.tick(0);
    let _ = p.agent_brief(vec![]);
    let s = setup(&provider, &mut p, 0);
    assert_eq!((s.agent.as_deref(), s.skills.clone()), (Some("reviewer"), vec!["notes".to_string()]));
    let _ = std::fs::remove_dir_all(&recipes);
}
