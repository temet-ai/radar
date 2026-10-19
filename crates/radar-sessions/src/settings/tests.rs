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
fn repository_locations_round_trip_and_inherit_each_omitted_field() {
    let config: toml::Value = toml::from_str(r#"
        work_dir = "/work/default"
        reviews_dir = "/notes/default"
        [repositories."ankh-morpork/watch"]
        checkout = "/work/watch"
        work_dir = "/work/watch-agent"
        reviews_dir = "/notes/watch"
        [repositories."ankh-morpork/library"]
        checkout = "/work/watch/library"
        reviews_dir = "/notes/library"
        [repositories."https://gitlab.example.test/ankh-morpork/watch"]
        reviews_dir = "/notes/private-watch"
    "#).unwrap();
    let mut settings = AgentsSettings::default();
    settings.configure(Some(&config)).unwrap();
    let mut restored = AgentsSettings::default();
    restored.configure(settings.config().as_ref()).unwrap();
    assert_eq!(restored.cfg, settings.cfg);
    assert_eq!(restored.work_dir("/work/watch/src".into()), std::path::PathBuf::from("/work/watch-agent"));
    assert_eq!(restored.work_dir("/work/watch/library/src".into()), std::path::PathBuf::from("/work/default"));
    assert_eq!(restored.work_dir("/work/watch-other".into()), std::path::PathBuf::from("/work/default"));
    let dirs = restored.repository_review_dirs();
    assert_eq!(locations::repository_entry(&dirs, "https://gitlab.example.test", "ankh-morpork/watch"), Some(&std::path::PathBuf::from("/notes/private-watch")));
    assert_eq!(locations::repository_entry(&dirs, "https://github.example.test", "ankh-morpork/watch"), Some(&std::path::PathBuf::from("/notes/watch")));
    // A workspace which changes only other defaults still inherits the global working folder.
    restored.cfg.workspaces.insert("watch".into(), Default::default());
    restored.set_workspace(Some("watch".into()));
    assert_eq!(restored.work_dir("/elsewhere".into()), std::path::PathBuf::from("/work/default"));
}

#[test]
fn repository_location_paths_are_validated_instead_of_silently_ignored() {
    for field in ["checkout", "work_dir", "reviews_dir"] {
        let table: toml::Value = toml::from_str(&format!("[repositories.\"ankh-morpork/watch\"]\n{field} = 'relative/path'")).unwrap();
        let error = AgentsConfig::from_table(Some(&table)).unwrap_err().to_string();
        assert!(error.contains(field) && error.contains("absolute path"), "{error}");
    }
}

#[test]
fn sending_a_repository_item_uses_its_working_folder_without_picking_a_related_repository() {
    use radar_core::agent_context::{AgentContext, ContextField};
    use radar_core::journal::SubjectKey;
    let mut settings = AgentsSettings::default();
    settings.cfg.repositories.insert("ankh-morpork/watch".into(), locations::RepositoryLocations {
        checkout: Some("/work/watch".into()), work_dir: Some("/work/watch-agent".into()), ..Default::default()
    });
    for kind in ["merge-request", "pull-request"] {
        let root = AgentContext::new("change", SubjectKey::new("sample", "https://gitlab.example.test", kind, "ankh-morpork/watch!12").unwrap(), "Watch change")
            .field(ContextField::new("project", "ankh-morpork/watch"));
        assert_eq!(settings.context_work_dir(std::slice::from_ref(&root)), Some("/work/watch-agent".into()));
        let ticket = AgentContext::new("ticket", SubjectKey::new("sample", "https://jira.example.test", "ticket", "WATCH-12").unwrap(), "Ticket");
        assert_eq!(settings.context_work_dir(&[ticket, root]), None);
    }
    let local = AgentContext::new("branch", SubjectKey::new("local", "/work/watch", "branch", "vimes/WATCH-12").unwrap(), "Branch");
    assert_eq!(settings.context_work_dir(&[local]), Some("/work/watch-agent".into()));
}

#[test]
fn repository_locations_are_editable_from_the_folder_rows_with_inheritance_and_removal() {
    let mut s = AgentsSettings::default();
    for row in ["def:work_dir", "rec:reviews"] {
        select(&mut s, row);
        let fx = effects(s.intent(SECTION, &Intent::Key { chord: Chord::char('p') }));
        assert!(matches!(&fx[..], [Effect::Menu(m)] if m.id == "agents.repositories"));
    }
    let repo = "https://gitlab.example.test/ankh-morpork/watch";
    s.on_menu("agents.repository_add", repo).unwrap();
    for field in ["checkout", "work_dir", "reviews_dir"] {
        let fx = s.on_menu("agents.repository_fields", &format!("{field}\t{repo}")).unwrap();
        let [Effect::Menu(m)] = &fx[..] else { panic!("no field editor") };
        assert_eq!(m.typed.as_deref(), Some(""));
        let bad = s.on_menu(&m.id, "relative/path").unwrap();
        assert!(matches!(&bad[..], [Effect::Notice(n)] if n.tone == radar_core::view::chrome::NoticeTone::Error));
        s.on_menu(&m.id, "/work/watch").unwrap();
    }
    let mut restored = AgentsSettings::default();
    restored.configure(s.config().as_ref()).unwrap();
    assert_eq!(restored.cfg.repositories[repo].reviews_dir.as_deref(), Some("/work/watch"));
    restored.on_menu(&format!("agents.repository_path:reviews_dir:{repo}"), "").unwrap();
    assert!(restored.cfg.repositories[repo].reviews_dir.is_none());
    restored.on_menu("agents.repository_fields", &format!("remove\t{repo}")).unwrap();
    assert!(!restored.cfg.repositories.contains_key(repo));
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
    assert!(matches!(&s.on_menu("agents.cli:claude:skills", "relative/skills").unwrap()[..], [Effect::Notice(w)] if w.tone == radar_core::view::chrome::NoticeTone::Error && w.contains("whole path")));
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
    assert_eq!(m.items.iter().map(|i| (i.key, i.value.as_str(), radar_core::mnemonic::at(&i.keys, &i.label))).collect::<Vec<_>>(), [
        (Some('e'), "review-mr\tedit", Some(0)), (Some('a'), "review-mr\tagent", Some(0)), (Some('s'), "review-mr\tskills", Some(0)),
    ]);
    s.on_menu("agents.recipe:review-mr:agent", "copilot=reviewer").unwrap();
    s.on_menu("agents.recipe:review-mr:skills", "security-review, notes").unwrap();
    // The built-in was copied to the folder first, then changed there; its instructions are as shipped.
    let (r, source, broken) = crate::brief::Recipe::load("review-mr", Some(&dir));
    let r = r.unwrap();
    assert!(matches!(source, crate::brief::recipe::Source::File(_)) && broken.is_none());
    assert_eq!(r.agent_for("copilot").as_deref(), Some("reviewer"));
    assert_eq!(r.skills, ["security-review", "notes"]);
    assert_eq!(r.instructions, crate::review::journey::recipe().instructions);
    let own = menu_of(effects(s.intent(SECTION, &Intent::Activate { target: Target::new("rec:review-mr") })));
    assert_eq!(own.items.iter().map(|i| (i.key, i.value.as_str(), radar_core::mnemonic::at(&i.keys, &i.label))).collect::<Vec<_>>(), [
        (Some('e'), "review-mr\tedit", Some(0)), (Some('a'), "review-mr\tagent", Some(0)),
        (Some('s'), "review-mr\tskills", Some(0)), (Some('b'), "review-mr\tbuilt-in", Some(8)),
    ]);
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

fn menu_of(fx: Vec<Effect>) -> radar_core::Menu {
    fx.into_iter().find_map(|e| if let Effect::Menu(m) = e { Some(m) } else { None }).expect("a menu")
}

mod mnemonic_tests {
    use super::*;

    fn owned_settings() -> AgentsSettings {
        let mut s = AgentsSettings::default();
        // Full settings views enumerate recipes. Pin their fallback to an
        // owned regular file, never the operator's HOME/XDG recipe directory.
        // The provider/action/help cases retain that small filesystem boundary;
        // rule-menu and setup-card cases do not call recipe discovery at all.
        s.cfg.recipes_dir = Some(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs").into());
        s
    }

    #[test]
    fn provider_mnemonics_keep_native_keys_and_number_arbitrary_custom_names_without_changing_dispatch() {
        let mut s = owned_settings();
        let custom = [
            ("rincewind", "Rincewind"), ("ook", "Ook"), ("death", "Death"),
            ("mort", "Mort"), ("uberwald", "Überwald"), ("great-atuin", "𐐷 Great A’Tuin"),
            ("turtle", "🐢"), ("tiffany", "Tiffany Aching"),
        ];
        // Set the data directly: this is a chooser/dispatch test, not a PATH probe.
        s.cfg.pty = custom.iter().map(|(id, name)| PtyCli {
            id: (*id).into(), name: (*name).into(), command: (*id).into(), args: vec![], login: None,
        }).collect();
        s.on_menu("agents.provider", "great-atuin").unwrap();
        let menu = menu_of(effects(s.intent(SECTION, &Intent::Activate { target: Target::new("def:provider") })));
        assert_eq!((menu.id.as_str(), menu.at.as_deref(), menu.question), ("agents.provider", Some("great-atuin"), false));
        let offered: Vec<_> = menu.items.iter().map(|i| (i.key, i.label.as_str(), i.value.as_str())).collect();
        assert_eq!(offered, [
            (Some('c'), "Copilot", "copilot"), (Some('x'), "Codex", "codex"), (Some('l'), "Claude Agent", "claude"),
            (Some('1'), "Rincewind", "rincewind"), (Some('2'), "Ook", "ook"), (Some('3'), "Death", "death"),
            (Some('4'), "Mort", "mort"), (Some('5'), "Überwald", "uberwald"),
            (Some('6'), "𐐷 Great A’Tuin", "great-atuin"), (Some('7'), "🐢", "turtle"),
        ], "only the seven old custom letter keys change; names, ids, order and offer limit stay fixed");
        assert_eq!(menu.items.iter().map(|i| radar_core::mnemonic::at(&i.keys, &i.label)).collect::<Vec<_>>(),
            [Some(0), Some(4), Some(1), None, None, None, None, None, None, None]);
        assert_eq!(menu.items.iter().filter(|i| i.hint == "now").map(|i| i.value.as_str()).collect::<Vec<_>>(), ["great-atuin"]);
        for item in &menu.items {
            assert_eq!(item.keys, item.key.unwrap().to_string());
            assert!(!item.heading && !item.note && !item.muted);
            let chosen = menu.items.iter().find(|i| i.key == item.key).unwrap();
            let fx = s.on_menu(&menu.id, &chosen.value).unwrap();
            assert!(matches!(&fx[..], [Effect::Notify(words)] if words == &format!("new sessions start with {}", chosen.label)));
            assert_eq!(s.defaults().provider, chosen.value, "the numeric/letter choice reaches the same provider id");
            let again = menu_of(effects(s.intent(SECTION, &Intent::Activate { target: Target::new("def:provider") })));
            assert_eq!(again.at.as_deref(), Some(chosen.value.as_str()));
            assert_eq!(again.items.iter().filter(|i| i.hint == "now").count(), 1);
        }
        assert!(s.take_requests().is_empty(), "choosing a default does not start or reconfigure a provider");
    }

    #[test]
    fn rule_kind_mnemonics_use_actual_offers_and_keep_permission_values_and_dispatch() {
        let expected = [
            ('c', "a command", "shell", 2, "git diff"),
            ('r', "reading files", "read", 0, "/srv/watch/*"),
            ('w', "writing files", "write", 0, "/srv/watch/*"),
            ('u', "URL access", "url", 0, "ankh-morpork.example"),
            ('m', "an MCP tool", "mcp", 3, "watch/search"),
        ];
        for (list, verb) in [("agents.allow", "Always allow"), ("agents.deny", "Always deny")] {
            let mut s = owned_settings();
            let menu = menu_of(s.rules_menu(list));
            assert_eq!(menu.id, list);
            let ways: Vec<_> = menu.items.iter().filter(|i| i.key.is_some()).collect();
            assert_eq!(ways.len(), expected.len());
            for (item, (key, words, kind, at, argument)) in ways.into_iter().zip(expected) {
                assert_eq!((item.key, item.keys.as_str(), item.label.as_str(), item.hint.as_str()), (Some(key), key.to_string().as_str(), words, ""));
                assert_eq!(radar_core::mnemonic::at(&item.keys, &item.label), Some(at));
                assert_eq!(item.value, format!("type\t{kind}"));
                let field = menu_of(s.on_menu(&menu.id, &item.value).unwrap());
                assert_eq!(field.id, format!("{list}:{kind}"));
                assert_eq!(field.title, format!("{verb} {words}"));
                assert_eq!(field.typed.as_deref(), Some(""));
                s.on_menu(&field.id, argument).unwrap();
                assert!(s.rules(list).iter().any(|r| r.kind == kind && r.argument == argument), "{list}: {kind} keeps the original rule payload");
            }
            let other = if list == "agents.allow" { "agents.deny" } else { "agents.allow" };
            assert!(s.rules(other).is_empty());
        }
    }

    #[test]
    fn cli_field_mnemonics_keep_field_names_values_order_and_typed_menu_dispatch() {
        let mut s = owned_settings();
        // Setup has the same CLI menu without enumerating the recipes folder.
        s.setup_card.set(true);
        select(&mut s, "cli:claude");
        let menu = menu_of(effects(s.intent(SECTION, &Intent::Key { chord: Chord::char('p') })));
        assert_eq!(menu.id, "agents.cli");
        let expected = [
            ('e', "executable", "executable", 0), ('f', "config dir", "config_dir", 3),
            ('a', "agent", "agent", 0), ('s', "skills", "skills", 0),
            ('n', "instructions", "instructions", 1), ('p', "plugins", "plugins", 0),
        ];
        assert_eq!(menu.items.len(), expected.len());
        for (item, (key, label, field, at)) in menu.items.iter().zip(expected) {
            assert_eq!((item.key, item.label.as_str()), (Some(key), label));
            assert_eq!(item.value, format!("claude\t{field}"));
            assert_eq!(radar_core::mnemonic::at(&item.keys, &item.label), Some(at));
            let typed = menu_of(s.on_menu(&menu.id, &item.value).unwrap());
            assert_eq!(typed.id, format!("agents.cli:claude:{field}"));
        }
    }

    #[test]
    fn settings_action_mnemonics_preserve_stable_ids_and_chords_for_actual_row_states() {
        let mut s = owned_settings();
        s.cfg.pty.push(PtyCli { id: "ook".into(), name: "Ook 🐢".into(), command: "ook".into(), args: vec![], login: None });
        let cli = [
            ("sessions.agents.sign_in", "i", Some(1)), ("sessions.agents.copy", "y", Some(3)),
            ("sessions.agents.open", "o", Some(0)), ("sessions.agents.add", "a", Some(0)),
            ("sessions.agents.refresh", "r", Some(0)), ("sessions.agents.paths", "p", Some(0)),
        ];
        let check = |s: &AgentsSettings, expected: &[(&str, &str, Option<usize>)]| {
            let view = s.section_view(SECTION, &cx()).unwrap();
            for heading in view.actions.iter().filter(|a| a.heading) {
                assert!(heading.id.is_empty() && heading.key.is_none() && heading.mnemonic_at.is_none() && !heading.mnemonic_shift);
            }
            let actual: Vec<_> = view.actions.iter().filter(|a| !a.heading)
                .map(|a| (a.id.clone(), a.key, a.mnemonic_at, a.mnemonic_shift)).collect();
            let expected: Vec<_> = expected.iter().map(|(id, key, at)| ((*id).to_string(), Chord::parse(key), *at, false)).collect();
            assert_eq!(actual, expected, "{}", s.selected().target());
        };
        for target in ["cli:copilot", "cli:codex", "cli:claude", "cli:ook"] {
            select(&mut s, target);
            check(&s, &cli);
        }
        s.login = Some(("copilot".into(), Login::Starting));
        select(&mut s, "cli:copilot");
        let mut active = cli.to_vec();
        active.push(("sessions.agents.cancel", "esc", None));
        check(&s, &active);
        s.login = None;
        select(&mut s, "cli:add");
        check(&s, &[("sessions.agents.add", "a", Some(0))]);
        for target in ["rec:folder", "rec:ask", "rec:review-mr"] {
            select(&mut s, target);
            check(&s, &[("sessions.agents.edit", "⏎", None), ("sessions.agents.recipe.edit", "e", Some(0))]);
        }
        for target in ["def:work_dir", "rec:reviews"] {
            select(&mut s, target);
            check(&s, &[("sessions.agents.edit", "⏎", None), ("sessions.agents.repositories", "p", Some(2))]);
        }
        for target in ["def:provider", "def:model", "def:roots", "def:skills", "def:project_config", "def:allow", "def:deny", "def:local_only", "def:reviewer_commands", "dock:notify"] {
            select(&mut s, target);
            check(&s, &[("sessions.agents.edit", "⏎", None)]);
        }
    }

    #[test]
    fn recipe_help_uses_the_same_styled_words_and_keeps_shift_visible_for_ask() {
        let mut s = owned_settings();
        let underlined = |text: &radar_core::view::Text| text.0.iter()
            .filter(|seg| seg.look.as_ref().is_some_and(|look| look.split_whitespace().any(|word| word == "underline")))
            .map(|seg| seg.text.clone()).collect::<Vec<_>>();
        select(&mut s, "rec:folder");
        let section = s.section_view(SECTION, &cx()).unwrap();
        let edit = section.detail.last().unwrap();
        assert!(edit.words().ends_with("Choose edit recipes to copy the built-ins there and open the folder in your editor."));
        assert_eq!(underlined(edit), ["e"]);
        assert_eq!(edit.0.iter().filter_map(|seg| seg.key_hint.as_deref()).collect::<Vec<_>>(), ["e"]);
        select(&mut s, "rec:ask");
        let section = s.section_view(SECTION, &cx()).unwrap();
        let ask = section.detail.last().unwrap();
        assert_eq!(ask.words(), "Used by shift ask an agent about it on any row: the brief's task and limits.");
        assert_eq!(underlined(ask), ["a"]);
        assert_eq!(ask.0.iter().filter_map(|seg| seg.key_hint.as_deref()).collect::<Vec<_>>(), ["A"]);
    }
}

fn asked(command: &str) -> crate::event::PermissionRequest {
    use crate::event::*;
    PermissionRequest {
        request_id: command.into(),
        kind: PermissionKind::Shell,
        target: PermissionTarget::Command { command: command.into(), segments: vec![] },
        tool_call_id: None,
        can_offer_session_approval: true,
    }
}

#[test]
fn always_allow_offers_what_the_agents_asked_for_one_key_each() {
    let mut s = AgentsSettings::default();
    let url = crate::event::PermissionRequest {
        kind: crate::event::PermissionKind::Url,
        target: crate::event::PermissionTarget::Url { url: "https://gitlab.com/a/b/-/merge_requests/1".into() },
        ..asked("u")
    };
    s.set_asked(&[asked("git status --short && git branch -avv && git remote -v"), asked("git status"), url]);
    let m = menu_of(effects(s.intent(SECTION, &Intent::Activate { target: Target::new("def:allow") })));
    let offered: Vec<(Option<char>, &str, &str)> = m.items.iter().filter(|i| i.key.is_some_and(|k| k.is_ascii_digit())).map(|i| (i.key, i.label.as_str(), i.hint.as_str())).collect();
    assert_eq!(
        offered,
        [
            (Some('1'), "command git status", "asked 2 times"),
            (Some('2'), "command git branch", "asked once"),
            (Some('3'), "command git remote", "asked once"),
            (Some('4'), "website gitlab.com", "asked once"),
        ]
    );
    // 1 adds it; the list comes back with it allowed and no longer offered.
    let back = menu_of(s.on_menu("agents.allow", &m.items.iter().find(|i| i.key == Some('1')).unwrap().value.clone()).unwrap());
    assert_eq!(s.defaults().allow[0].to_string(), "shell(git status)");
    assert!(back.items.iter().any(|i| i.label == "command git status" && i.key.is_none()));
    assert!(!back.items.iter().any(|i| i.label == "command git status" && i.key.is_some()));
    // Typing a command offers the asked ones that match.
    let t = s.typing("agents.allow:shell", "git b");
    assert_eq!(t.offers.iter().map(|o| o.label.as_str()).collect::<Vec<_>>(), ["git branch"]);
}

#[test]
fn always_allow_lists_its_rules_in_words_and_adds_one_by_kind_without_any_syntax() {
    let mut s = AgentsSettings::default();
    // ⏎ on the row: nothing listed, and one way to add each kind of request.
    let m = menu_of(effects(s.intent(SECTION, &Intent::Activate { target: Target::new("def:allow") })));
    assert_eq!(m.id, "agents.allow");
    let labels: Vec<&str> = m.items.iter().map(|i| i.label.as_str()).collect();
    assert!(labels.contains(&"  nothing yet"), "{labels:?}");
    for want in ["a command", "reading files", "writing files", "URL access", "an MCP tool"] {
        assert!(labels.contains(&want), "{want} in {labels:?}");
    }
    // c: a command. A short title; the example is under the field before anything is typed.
    let add = m.items.iter().find(|i| i.label == "a command").unwrap();
    let field = menu_of(s.on_menu("agents.allow", &add.value).unwrap());
    assert_eq!((field.id.as_str(), field.title.as_str()), ("agents.allow:shell", "Always allow a command"));
    assert!(matches!(s.typing(&field.id, "").verdict, Some(Verdict::Unsure(w)) if w.contains("git diff")));
    assert!(matches!(s.typing(&field.id, "cargo test").verdict, Some(Verdict::Good(_))));
    // ⏎ adds it and goes back to the list, where it is shown in words.
    let back = menu_of(s.on_menu(&field.id, "cargo test").unwrap());
    assert_eq!(s.defaults().allow[0].to_string(), "shell(cargo test)");
    let listed = back.items.iter().find(|i| i.label == "command cargo test").expect("listed in words");
    assert!(matches!(s.typing(&field.id, "cargo test").verdict, Some(Verdict::Unsure(_))));
    // Choosing a listed one removes it.
    s.on_menu("agents.allow", &listed.value.clone()).unwrap();
    assert!(s.defaults().allow.is_empty());
    // The same for always deny, a website.
    let d = menu_of(effects(s.intent(SECTION, &Intent::Activate { target: Target::new("def:deny") })));
    let site = d.items.iter().find(|i| i.label == "URL access").unwrap();
    let field = menu_of(s.on_menu("agents.deny", &site.value).unwrap());
    s.on_menu(&field.id, "example.com").unwrap();
    assert_eq!(s.defaults().deny[0].to_string(), "url(example.com)");
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
        setup: false,
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
    fn typing(&self, menu: &str, text: &str) -> radar_core::plugin::Typing {
        self.0.typing(menu, text)
    }
    fn on_menu(&mut self, menu: &str, value: &str) -> Vec<Effect> {
        self.0.on_menu(menu, value).unwrap_or_default()
    }
}

/// The section on Settings in the real host, the nav focused, driven as a
/// renderer that lays the section out itself (Ink, Headless) drives it: by
/// intents.
fn driven() -> radar_core::headless::Headless {
    let mut host = radar_core::Host::new(vec![Box::new(Hosted(every_state()))], radar_core::plugin::DEMO_NOW).unwrap();
    host.view = radar_core::host::View::Settings;
    host.section = host.sections.iter().position(|(_, x)| x.name == "Tools and defaults").unwrap();
    host.focus = radar_core::Pane::Nav;
    radar_core::headless::Headless::new(host, 160, 40)
}

fn press(h: &mut radar_core::headless::Headless, key: radar_core::view::Key) {
    h.send(Intent::Key { chord: Chord::new(key) });
}

fn type_in(h: &mut radar_core::headless::Headless, text: &str) {
    for c in text.chars() {
        press(h, radar_core::view::Key::Char(c));
    }
}

fn row_text(h: &radar_core::headless::Headless, target: &str) -> String {
    match h.section().map(|v| v.body) {
        Some(Node::List { rows, .. }) => rows.iter().find(|r| r.target.as_str() == target).map(|r| text_of(&r.text)).unwrap_or_default(),
        _ => panic!("a list"),
    }
}

fn hints(h: &radar_core::headless::Headless) -> Vec<String> {
    h.host().chrome_view(160, 40).status.hints.iter().map(|k| format!("{} {}", k.keys, k.label)).collect()
}

fn saved(h: &radar_core::headless::Headless) -> String {
    h.host().plugins[0].config().map(|t| t.to_string()).unwrap_or_default()
}

/// Every row that asks for text, from a press on the row to the saved
/// value, as a renderer sends it: the field opens with its caret and says
/// how to finish it, typing shows, ⏎ keeps the text in the row and in the
/// config, esc leaves it and keeps nothing.
#[test]
fn every_typed_row_opens_a_field_that_types_saves_and_cancels_through_the_host() {
    // (row, a key that chooses "type your own" first, what is typed, what the row then reads, what the config holds)
    let rows: [(&str, Option<char>, &str, &str, &str); 8] = [
        ("def:model", None, "gpt-5", "gpt-5", "gpt-5"),
        ("def:work_dir", None, "/tmp/gg-work", "/tmp/gg-work", "/tmp/gg-work"),
        ("def:roots", None, "/tmp/gg-root", "/tmp/gg-root", "/tmp/gg-root"),
        ("def:skills", None, "/tmp/gg-skills", "/tmp/gg-skills", "/tmp/gg-skills"),
        ("rec:folder", None, "/tmp/gg-recipes", "/tmp/gg-recipes", "/tmp/gg-recipes"),
        ("rec:reviews", None, "/tmp/gg-reviews", "/tmp/gg-reviews", "/tmp/gg-reviews"),
        ("def:allow", Some('c'), "ls", "command ls", "ls"),
        ("def:deny", Some('c'), "rm", "command rm", "rm"),
    ];
    use radar_core::view::Key;
    for (target, choose, typed, shown, config) in rows {
        let mut h = driven();
        // A press on the row from the nav selects it and gives the section the keys.
        h.send(Intent::Select { target: Target::new(target) });
        for round in ["esc", "⏎"] {
            press(&mut h, Key::Enter);
            if let Some(c) = choose {
                assert!(h.menu().is_some_and(|m| m.typed.is_none()), "{target}: ⏎ offers the ways first");
                press(&mut h, Key::Char(c));
            }
            let menu = h.menu().unwrap_or_else(|| panic!("{target}: ⏎ opened nothing"));
            assert!(menu.typed.is_some(), "{target}: a field");
            let hints = hints(&h);
            assert!(hints.ends_with(&["⏎ save".to_string(), "esc cancel".to_string()]), "{target}: {hints:?}");

            type_in(&mut h, typed);
            let now = h.menu().and_then(|m| m.typed).unwrap_or_default();
            assert!(now.ends_with(typed), "{target}: typing shows, {now:?}");
            if round == "esc" {
                press(&mut h, Key::Esc);
                assert!(h.menu().is_none(), "{target}: esc closes it");
                assert!(!saved(&h).contains(config), "{target}: esc keeps nothing:\n{}", saved(&h));
                assert!(!row_text(&h, target).contains(shown), "{target}: {}", row_text(&h, target));
            } else {
                press(&mut h, Key::Enter);
                if choose.is_some() {
                    // The rules come back, the new one listed.
                    assert!(h.menu().is_some_and(|m| m.typed.is_none()), "{target}: ⏎ is back at the list");
                    press(&mut h, Key::Esc);
                }
                assert!(h.menu().is_none(), "{target}: ⏎ closes it");
                assert!(row_text(&h, target).contains(shown), "{target}: the row reads it: {}", row_text(&h, target));
                assert!(saved(&h).contains(config), "{target}: saved:\n{}", saved(&h));
            }
        }
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
    let yours = format!("v{} · yours · agent copilot=reviewer", crate::brief::Recipe::built_in("review-mr").unwrap().version);
    for want in ["recipe review-mr", yours.as_str(), "recipe ask", "built-in", "review locally"] {
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
        for menu in ["agents.model", "agents.roots", "agents.allow", "agents.deny", "agents.provider", "agents.allow:shell", "agents.deny:url"] {
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

#[test]
fn a_chain_is_allowed_when_every_command_is_and_denied_when_any_is() {
    let mut s = AgentsSettings::default();
    s.on_menu("agents.allow", "shell(git status)").unwrap();
    s.on_menu("agents.allow", "shell(git branch)").unwrap();
    let decide = |s: &AgentsSettings, c: &str| s.defaults().standing(&asked(c)).map(|(allow, _)| allow);
    assert_eq!(decide(&s, "git status --short && git branch -avv"), Some(true));
    assert_eq!(decide(&s, "git status && git remote -v"), None, "git remote is not allowed");
    assert_eq!(decide(&s, "git status > out.txt"), None, "a redirection is never a chain");
    s.on_menu("agents.deny", "shell(git branch)").unwrap();
    assert_eq!(decide(&s, "git status; git branch -D x"), Some(false));
    // What the dock's always allow adds for a chain: each command by its program and subcommand.
    let rules: Vec<String> = config::Rule::suggest(&asked("zsh -lc 'git status --short && git branch -avv | head'")).iter().map(ToString::to_string).collect();
    assert_eq!(rules, ["shell(git status)", "shell(git branch)", "shell(head)"]);
    assert!(config::Rule::suggest(&asked("cat $(whoami)")).is_empty());
}
