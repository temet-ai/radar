use super::*;
use crate::adapter::{AuthStatus, Capabilities};
use radar_core::view::{Chord, Node, Target};

fn ready(user: &str) -> AdapterStatus {
    AdapterStatus {
        installed: true,
        version: Some("1.0.95".into()),
        protocol: Some("3".into()),
        auth: AuthStatus::SignedIn { as_user: user.into() },
        error: None,
        capabilities: Capabilities { resume: true, stop: true, permissions: true, structured_events: true, ..Default::default() },
    }
}

fn out() -> AdapterStatus {
    AdapterStatus { auth: AuthStatus::SignedOut, ..ready("") }
}

fn missing() -> AdapterStatus {
    AdapterStatus { installed: false, version: None, protocol: None, ..out() }
}

fn select(s: &mut AgentsSettings, target: &str) {
    s.intent(SECTION, &Intent::Select { target: Target::new(target) });
}

fn effects(o: Outcome) -> Vec<Effect> {
    match o {
        Outcome::Handled(e) => e,
        Outcome::Ignored => panic!("ignored"),
    }
}

#[test]
fn the_defaults_are_local_and_reviewers_may_run_commands() {
    let s = AgentsSettings::default();
    assert!(s.defaults().local_only);
    assert!(s.defaults().reviewer_may_run_commands);
    assert_eq!(s.defaults().provider, "copilot");
    assert!(s.config().is_none());
}

#[test]
fn a_clis_paths_agent_and_folders_are_set_from_its_menu_and_saved() {
    let mut s = AgentsSettings::default();
    select(&mut s, "cli:claude");
    let fx = effects(s.intent(SECTION, &Intent::Key { chord: Chord::char('p') }));
    let [Effect::Menu(m)] = &fx[..] else { panic!("{fx:?}") };
    assert_eq!(m.id, "agents.cli");
    assert_eq!(m.items.len(), CLI_FIELDS.len());
    assert_eq!(s.requests.last(), Some(&Request::ReadCatalog("claude".into())), "its agents are read for the agent field");
    // Choosing a field asks for its value; the value is checked and saved.
    let fx = s.on_menu("agents.cli", "claude\texecutable").unwrap();
    let [Effect::Menu(typed)] = &fx[..] else { panic!("{fx:?}") };
    assert_eq!(typed.id, "agents.cli:claude:executable");
    s.on_menu("agents.cli:claude:executable", "/opt/claude/bin/claude").unwrap();
    s.on_menu("agents.cli:claude:config_dir", "~/claude-work").unwrap();
    s.on_menu("agents.cli:claude:agent", "code-reviewer").unwrap();
    s.on_menu("agents.cli:claude:plugins", "/srv/plugins/review").unwrap();
    assert!(matches!(&s.on_menu("agents.cli:claude:skills", "relative/skills").unwrap()[..], [Effect::Notify(w)] if w.contains("whole path")));
    s.on_menu("agents.skills", "/srv/skills").unwrap();
    s.on_menu("agents.work_dir", "~/code").unwrap();
    effects(s.intent(SECTION, &Intent::Activate { target: Target::new("def:project_config") }));

    let mut fresh = AgentsSettings::default();
    fresh.configure(Some(&s.config().unwrap())).unwrap();
    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap());
    let p = fresh.provider_setup("claude");
    assert_eq!(p.executable.as_deref(), Some(std::path::Path::new("/opt/claude/bin/claude")));
    assert_eq!(p.config_dir, Some(home.join("claude-work")));
    assert_eq!(fresh.provider_setup("pty:claude"), p, "the terminal tab runs the same CLI");
    let setup = fresh.session_setup("claude");
    assert_eq!(setup.agent.as_deref(), Some("code-reviewer"));
    assert_eq!(setup.plugin_dirs, [std::path::PathBuf::from("/srv/plugins/review")]);
    assert_eq!(setup.skill_dirs, [std::path::PathBuf::from("/srv/skills")]);
    assert!(!setup.project_config);
    assert_eq!(fresh.work_dir("/tmp".into()), home.join("code"));
    assert!(fresh.session_setup("codex").agent.is_none(), "an agent is per CLI");
}

#[test]
fn a_recipes_agent_and_skills_are_edited_in_settings_on_its_own_copy() {
    let dir = std::env::temp_dir().join(format!("radar-recipes-settings-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut s = AgentsSettings::default();
    s.on_menu("agents.recipes_dir", &dir.display().to_string()).unwrap();
    assert!(s.rows().contains(&Row::Recipe("review-mr".into())));
    select(&mut s, "rec:review-mr");
    let fx = effects(s.intent(SECTION, &Intent::Activate { target: Target::new("rec:review-mr") }));
    let [Effect::Menu(m)] = &fx[..] else { panic!("{fx:?}") };
    assert_eq!(m.id, "agents.recipe");
    s.on_menu("agents.recipe:review-mr:agent", "copilot=reviewer").unwrap();
    s.on_menu("agents.recipe:review-mr:skills", "security-review, notes").unwrap();
    // The built-in was copied to the folder first, then changed there; its instructions are as shipped.
    let (r, source, broken) = crate::brief::Recipe::load("review-mr", Some(&dir));
    let r = r.unwrap();
    assert!(matches!(source, crate::brief::recipe::Source::File(_)) && broken.is_none());
    assert_eq!(r.agent_for("copilot").as_deref(), Some("reviewer"));
    assert_eq!(r.skills, ["security-review", "notes"]);
    assert_eq!(r.instructions, crate::review::journey::recipe().instructions);
    // Back to the built-in: the operator's file is kept as .bak.
    let fx = s.on_menu("agents.recipe", "review-mr\tbuilt-in").unwrap();
    assert!(matches!(&fx[..], [Effect::Notify(w)] if w.contains(".bak")));
    assert!(dir.join("review-mr.md.bak").exists());
    assert_eq!(crate::brief::Recipe::load("review-mr", Some(&dir)).0.unwrap().agent, None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn desktop_notifications_are_off_until_asked_for_and_the_choice_is_saved() {
    let mut s = AgentsSettings::default();
    assert!(!s.cfg.notify);
    assert!(s.config().is_none(), "the default writes nothing");
    let listed = |s: &AgentsSettings| match s.section_view(SECTION, &cx()).map(|v| v.body) {
        Some(Node::List { rows, .. }) => rows.iter().any(|r| r.target.as_str() == "dock:notify"),
        _ => false,
    };
    assert!(listed(&s), "Settings → Agents lists the row");
    select(&mut s, "dock:notify");
    effects(s.intent(SECTION, &Intent::Activate { target: Target::new("dock:notify") }));
    assert!(s.cfg.notify);
    let mut fresh = AgentsSettings::default();
    fresh.configure(Some(&s.config().unwrap())).unwrap();
    assert!(fresh.cfg.notify);
}

#[test]
fn the_settings_are_saved_in_the_table_and_applied_to_a_new_session() {
    let mut s = AgentsSettings::default();
    effects(s.intent(SECTION, &Intent::Activate { target: Target::new("def:local_only") }));
    assert!(s.on_menu("agents.provider", "codex").is_some());
    s.on_menu("agents.model", "gpt-5").unwrap();
    s.on_menu("agents.allow", "shell(git diff)").unwrap();
    s.on_menu("agents.deny", "url(example.com)").unwrap();
    s.on_menu("agents.roots", "/srv/shared").unwrap();
    effects(s.intent(SECTION, &Intent::Activate { target: Target::new("def:reviewer_commands") }));

    let table = s.config().unwrap();
    let mut fresh = AgentsSettings::default();
    fresh.configure(Some(&table)).unwrap();
    let d = fresh.defaults();
    assert_eq!((d.provider.as_str(), d.model.as_deref()), ("codex", Some("gpt-5")));
    assert!(!d.local_only && !d.reviewer_may_run_commands);
    assert_eq!(d.allow[0].to_string(), "shell(git diff)");
    assert_eq!(d.deny[0].to_string(), "url(example.com)");
    assert_eq!(d.permitted_roots, ["/srv/shared"]);
}

#[test]
fn typing_a_listed_entry_removes_it_and_a_bad_rule_is_refused() {
    let mut s = AgentsSettings::default();
    s.on_menu("agents.allow", "read(src/**)").unwrap();
    assert!(matches!(s.typing("agents.allow", "read(src/**)").verdict, Some(Verdict::Unsure(_))));
    assert!(matches!(s.typing("agents.allow", "nonsense").verdict, Some(Verdict::Bad(_))));
    assert!(matches!(s.typing("agents.allow", "shell(ls)").verdict, Some(Verdict::Good(_))));
    s.on_menu("agents.allow", "read(src/**)").unwrap();
    assert!(s.defaults().allow.is_empty());
    s.on_menu("agents.allow", "nonsense").unwrap();
    assert!(s.defaults().allow.is_empty());
}

#[test]
fn a_token_pasted_in_a_field_is_refused_and_never_reaches_the_config() {
    let token = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
    let mut s = AgentsSettings::default();
    assert!(matches!(s.typing("agents.model", token).verdict, Some(Verdict::Bad(_))));
    assert!(matches!(s.typing("agents.roots", token).verdict, Some(Verdict::Bad(_))));
    s.on_menu("agents.model", token).unwrap();
    s.on_menu("agents.roots", token).unwrap();
    s.on_menu("agents.allow", &format!("shell({token})")).unwrap();
    let text = s.config().map(|t| t.to_string()).unwrap_or_default();
    assert!(!text.contains("ghp_"), "{text}");
    assert!(s.defaults().allow.is_empty() && s.defaults().model.is_none());
}

#[test]
fn a_workspace_has_its_own_defaults() {
    let mut s = AgentsSettings::default();
    s.set_workspace(Some("work".into()));
    s.on_menu("agents.provider", "claude").unwrap();
    assert_eq!(s.defaults().provider, "claude");
    s.set_workspace(Some("other".into()));
    assert_eq!(s.defaults().provider, "copilot");
    s.set_workspace(None);
    assert_eq!(s.defaults().provider, "copilot");
    let table = s.config().unwrap();
    let mut fresh = AgentsSettings::default();
    fresh.configure(Some(&table)).unwrap();
    fresh.set_workspace(Some("work".into()));
    assert_eq!(fresh.defaults().provider, "claude");
}

#[test]
fn a_missing_cli_is_told_from_a_signed_out_one_and_only_the_second_starts_a_sign_in() {
    let mut s = AgentsSettings::default();
    s.set_status("codex", missing());
    s.set_status("copilot", out());
    assert_ne!(s.state("codex").word(), s.state("copilot").word());

    select(&mut s, "cli:codex");
    effects(s.intent(SECTION, &Intent::Key { chord: Chord::char('i') }));
    assert!(s.take_requests().is_empty(), "nothing to sign in to");

    select(&mut s, "cli:copilot");
    effects(s.intent(SECTION, &Intent::Key { chord: Chord::char('i') }));
    assert_eq!(s.take_requests(), [Request::StartLogin("copilot".into())]);
    // A second start while it runs is refused.
    effects(s.intent(SECTION, &Intent::Key { chord: Chord::char('i') }));
    assert!(s.take_requests().is_empty());

    s.login_event("copilot", LoginEvent::DeviceCode { url: "https://github.com/login/device".into(), code: "ABCD-1234".into() });
    let words = s.section_view(SECTION, &cx()).unwrap().detail.iter().map(text_of).collect::<Vec<_>>().join("\n");
    assert!(words.contains("ABCD-1234") && words.contains("github.com/login/device"), "{words}");

    effects(s.intent(SECTION, &Intent::Key { chord: Chord::new(Key::Esc) }));
    assert_eq!(s.take_requests(), [Request::CancelLogin("copilot".into())]);
    s.login_event("copilot", LoginEvent::Completed);
    assert!(s.take_requests().is_empty(), "a cancelled flow reads nothing more");
}

#[test]
fn codex_and_claude_sign_in_with_their_own_command_in_a_tab_then_the_status_is_read() {
    let mut s = AgentsSettings::default();
    s.set_status("claude", out());
    select(&mut s, "cli:claude");
    effects(s.intent(SECTION, &Intent::Activate { target: Target::new("cli:claude") }));
    assert_eq!(
        s.take_requests(),
        [Request::RunInTab { provider: "claude".into(), argv: vec!["claude".into(), "auth".into(), "login".into()] }]
    );
    s.tab_opened("claude", "t1");
    s.tab_exited("claude", Some(0));
    assert_eq!(s.take_requests(), [Request::ReadStatus("claude".into())]);
}

#[test]
fn a_pty_cli_is_installed_when_its_command_is_on_the_path() {
    let dir = std::env::temp_dir().join(format!("radar-agents-path-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("aider");
    std::fs::write(&exe, "#!/bin/sh\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();

    let mut s = AgentsSettings::default();
    s.add_pty(PtyCli { id: "aider".into(), name: "Aider".into(), command: "aider".into(), args: vec![], login: None });
    s.set_path(Some(dir.to_string_lossy().into()));
    assert_eq!(s.state("aider"), status::State::Installed);
    s.set_path(Some(String::new()));
    assert_eq!(s.state("aider"), status::State::Missing);
    std::fs::remove_dir_all(&dir).unwrap();
}

fn cx() -> ViewCx<'static> {
    cx_at(80, 24)
}

fn cx_at(width: u16, height: u16) -> ViewCx<'static> {
    ViewCx {
        now: 0,
        focus: radar_core::view::PaneId::Middle,
        width,
        height,
        parts: radar_core::Parts::none(),
        icon_lead: 0,
    }
}

fn text_of(t: &radar_core::view::Text) -> String {
    t.0.iter().map(|s| s.text.as_str()).collect()
}

fn rows_of(v: &SectionView) -> Vec<String> {
    match &v.body {
        Node::List { rows, .. } => rows.iter().map(|r| text_of(&r.text)).collect(),
        _ => panic!("a list"),
    }
}

#[test]
fn the_section_lists_each_cli_with_its_state_and_the_defaults() {
    let mut s = AgentsSettings::default();
    s.set_status("copilot", ready("octo@github.com"));
    s.set_status("codex", missing());
    s.set_status("claude", out());
    let v = s.section_view(SECTION, &cx()).unwrap();
    let rows = rows_of(&v).join("\n");
    for want in ["Copilot", "signed in", "octo@github.com", "not installed", "signed out", "local only", "reviewer commands"] {
        assert!(rows.contains(want), "{want} in\n{rows}");
    }
    assert_eq!(v.scope, SECTION);
    assert!(sections().iter().any(|x| x.id == SECTION));
}

// --- drawn by the real host ------------------------------------------------

struct Hosted(AgentsSettings);

impl radar_core::Plugin for Hosted {
    fn id(&self) -> &'static str {
        "sessions"
    }
    fn name(&self) -> &'static str {
        "Agents"
    }
    fn configure(&mut self, t: Option<&toml::Value>) -> anyhow::Result<()> {
        self.0.configure(t)
    }
    fn config(&self) -> Option<toml::Value> {
        self.0.config()
    }
    fn sections(&self) -> Vec<radar_core::SectionSpec> {
        sections()
    }
    fn section_view(&self, section: &str, cx: &ViewCx<'_>) -> Option<SectionView> {
        self.0.section_view(section, cx)
    }
    fn intent(&mut self, scope: &str, _pane: radar_core::Pane, intent: &Intent) -> Outcome {
        self.0.intent(scope, intent)
    }
}

fn shot(s: AgentsSettings, target: &str, w: u16, h: u16) -> String {
    let mut s = s;
    select(&mut s, target);
    let mut host = radar_core::Host::new(vec![Box::new(Hosted(s))], radar_core::plugin::DEMO_NOW).unwrap();
    host.view = radar_core::host::View::Settings;
    host.section = host.sections.iter().position(|(_, x)| x.name == "Tools and defaults").unwrap();
    radar_core::shell::to_text(&host, w, h)
}

fn every_state() -> AgentsSettings {
    let mut s = AgentsSettings::default();
    s.set_status("copilot", ready("octo@github.com"));
    s.set_status("codex", missing());
    s.set_status("claude", out());
    s
}

#[test]
fn a_recipe_row_says_where_it_comes_from_and_what_it_runs_as() {
    let dir = std::env::temp_dir().join(format!("radar-recipe-shot-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut s = every_state();
    s.on_menu("agents.recipes_dir", &dir.display().to_string()).unwrap();
    s.on_menu("agents.recipe:review-mr:agent", "copilot=reviewer").unwrap();
    let text = shot(s, "rec:review-mr", 160, 40);
    if std::env::var_os("RADAR_SHOW").is_some() {
        println!("{text}");
    }
    for want in ["recipe review-mr", "v2 · yours · agent copilot=reviewer", "recipe ask", "built-in", "review locally"] {
        assert!(text.contains(want), "{want}:\n{text}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_section_is_drawn_at_both_sizes_with_each_cli_in_its_own_state() {
    for (w, h) in [(80, 24), (160, 40)] {
        let text = shot(every_state(), "cli:copilot", w, h);
        for want in ["Copilot", "Codex", "Claude Agent", "signed in", "not installed", "signed out", "octo@github.com"] {
            assert!(text.contains(want), "{want} at {w}x{h}:\n{text}");
        }
    }
}

#[test]
fn the_other_states_and_the_defaults_are_drawn_at_both_sizes() {
    let mut s = every_state();
    s.set_status("claude", AdapterStatus { auth: AuthStatus::PolicyBlocked { message: "agent mode is off".into() }, ..ready("") });
    s.set_status("codex", AdapterStatus { error: Some("app-server did not answer".into()), ..ready("") });
    for (w, h) in [(80, 24), (160, 40)] {
        let text = shot(s.clone_for_test(), "cli:claude", w, h);
        assert!(text.contains("blocked by policy") && text.contains("cannot connect"), "{w}x{h}:\n{text}");
        let text = shot(s.clone_for_test(), "def:reviewer_commands", w, h);
        assert!(text.contains("local only") && text.contains("reviewer"), "{w}x{h}:\n{text}");
    }
}

#[test]
fn a_device_code_is_drawn_while_the_sign_in_waits() {
    let mut s = every_state();
    select(&mut s, "cli:claude");
    s.set_status("copilot", out());
    select(&mut s, "cli:copilot");
    s.intent(SECTION, &Intent::Key { chord: Chord::char('i') });
    s.login_event("copilot", LoginEvent::DeviceCode { url: "https://github.com/login/device".into(), code: "ABCD-1234".into() });
    for (w, h) in [(80, 24), (160, 40)] {
        let text = shot(s.clone_for_test(), "cli:copilot", w, h);
        assert!(text.contains("ABCD-1234"), "{w}x{h}:\n{text}");
    }
}

/// Whatever is typed, nothing shaped like a credential is in what is saved.
#[test]
fn no_token_shaped_value_is_in_the_saved_config() {
    let tokens = [
        "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
        "github_pat_11AAAAAAA0abcdefghijklmnop",
        "sk-ant-api03-abcdefghijklmnopqrstuvwxyz",
        "xoxp-1234-5678-abcd",
        "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.abcdefghij",
        "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8",
    ];
    let mut s = AgentsSettings::default();
    for t in tokens {
        for menu in ["agents.model", "agents.roots", "agents.allow", "agents.deny", "agents.provider"] {
            s.on_menu(menu, t);
            s.on_menu(menu, &format!("shell({t})"));
        }
    }
    s.on_menu("agents.model", "gpt-5");
    let saved = s.config().map(|t| t.to_string()).unwrap_or_default();
    for t in tokens {
        assert!(!saved.contains(t), "{t} in\n{saved}");
    }
    assert!(saved.contains("gpt-5"));
}

#[test]
fn a_provider_with_no_adapter_in_this_build_is_not_left_checking() {
    let mut s = AgentsSettings::default();
    assert_eq!(s.state("codex"), status::State::Unread);
    s.set_available(vec!["copilot".into()]);
    assert_eq!(s.state("codex"), status::State::NoAdapter);
    assert_eq!(s.state("copilot"), status::State::Unread);
    s.set_status("codex", missing());
    assert_eq!(s.state("codex"), status::State::Missing);
}
