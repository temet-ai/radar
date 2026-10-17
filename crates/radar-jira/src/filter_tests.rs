//! Filter semantics at owner-unit speed, plus one real-host typing boundary.
//! The legacy keys and two-site topology are intentional: these replace the
//! corresponding demo-backed tests without changing what their data exercises.

use crate::config::{Project, Site, View};
use crate::model::{Category, Parent};
use crate::test_support::TicketBuilder;
use crate::{JiraPlugin, Settings, screen};
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use radar_core::{Effect, Outcome, Pane, Plugin};

const NOW: i64 = 1_791_273_600;

fn site(name: &str, projects: &[&str], color: &str) -> Site {
    Site {
        name: name.into(),
        url: format!("https://{name}.invalid"),
        email: "env:RADAR_JIRA_FIXTURE_EMAIL_NEVER_RESOLVE".into(),
        token: "env:RADAR_JIRA_FIXTURE_TOKEN_NEVER_RESOLVE".into(),
        color: color.into(),
        projects: projects
            .iter()
            .map(|key| Project {
                key: (*key).into(),
                ac_field: String::new(),
            })
            .collect(),
        from: String::new(),
    }
}

/// Only the current list, with no demo, wall clock, environment discovery,
/// logging, network or page data. TicketBuilder owns every ticket separately.
fn fixture() -> JiraPlugin {
    let mut p = JiraPlugin {
        cfg: Settings {
            sites: vec![
                site("acme", &["ABC", "DEF"], "blue"),
                site("beta", &["XYZ"], "green"),
            ],
            views: vec![View {
                name: "All tickets".into(),
                when: vec![],
                jql: "statusCategory != Done ORDER BY updated DESC".into(),
                site: String::new(),
            }],
            ..Default::default()
        },
        now: NOW,
        env: Some(Vec::new()),
        mcp: std::sync::OnceLock::from(Vec::new()),
        ..Default::default()
    };
    let parent = Parent {
        key: "ABC-1".into(),
        summary: "Dashboard for the stats".into(),
        status: "In Progress".into(),
        category: Category::Doing,
    };
    let mut epic = TicketBuilder::new()
        .key("ABC-1")
        .id("10001")
        .summary(&parent.summary)
        .status("In Progress", Category::Doing)
        .build();
    epic.kind = "Epic".into();
    let mut story = TicketBuilder::new()
        .key("ABC-12")
        .summary("Add a stats endpoint for the dashboard")
        .status("In Progress", Category::Doing)
        .build();
    story.kind = "Story".into();
    story.parent = Some(parent.clone());
    let mut sub = TicketBuilder::new()
        .key("ABC-13")
        .id("10013")
        .summary("Count requests per route")
        .status("In Progress", Category::Doing)
        .build();
    sub.kind = "Sub-task".into();
    sub.subtask = true;
    sub.parent = Some(parent);
    let mut bug = TicketBuilder::new()
        .key("ABC-20")
        .id("10020")
        .summary("Stats page shows yesterday twice")
        .build();
    bug.kind = "Bug".into();
    let review = TicketBuilder::new()
        .key("DEF-3")
        .id("20003")
        .summary("Rename the export job")
        .status("In Review", Category::Doing)
        .build();
    let other = TicketBuilder::new()
        .key("XYZ-7")
        .id("30007")
        .summary("Move the nightly build")
        .build();
    for (index, tickets) in [vec![story, sub, epic, bug, review], vec![other]]
        .into_iter()
        .enumerate()
    {
        let name = p.cfg.sites[index].name.clone();
        let jql = p.jql_for(index, &p.cfg.views[0]).unwrap();
        p.on_loaded(name, jql, (None, tickets));
    }
    p
}

fn child_path() -> Vec<String> {
    [
        "s:acme",
        "g:acme:In Progress",
        "t:acme:ABC-1",
        "t:acme:ABC-12",
    ]
    .map(String::from)
    .to_vec()
}

fn visible_keys(p: &JiraPlugin) -> Vec<&str> {
    p.visible()
        .into_iter()
        .map(|it| it.t.key.as_str())
        .collect()
}

fn assert_export_selected(p: &JiraPlugin) {
    assert_eq!(visible_keys(p), ["DEF-3"]);
    assert!(
        matches!(screen::selected(p), Some(screen::Selected::Ticket(it)) if it.t.key == "DEF-3")
    );
}

mod unit {
    use super::*;

    fn key(p: &mut JiraPlugin, pane: Pane, code: KeyCode) -> Vec<Effect> {
        match screen::key(p, pane, KeyEvent::new(code, KeyModifiers::NONE)) {
            Outcome::Handled(effects) => effects,
            Outcome::Ignored => panic!("filter ignored {code:?}"),
        }
    }

    fn typed(p: &mut JiraPlugin, pane: Pane, text: &str) {
        for c in text.chars() {
            assert!(key(p, pane, KeyCode::Char(c)).is_empty());
        }
    }

    fn assert_focus_middle(effects: &[Effect]) {
        assert!(matches!(effects, [Effect::Focus(Pane::Middle)]));
    }

    #[test]
    fn slash_filters_by_words_and_esc_clears() {
        let mut p = fixture();
        assert_eq!(
            visible_keys(&p),
            ["ABC-12", "ABC-13", "ABC-1", "ABC-20", "DEF-3", "XYZ-7"]
        );
        assert!(key(&mut p, Pane::Middle, KeyCode::Char('/')).is_empty());
        assert!(p.capturing());
        typed(&mut p, Pane::Middle, "export");
        assert_focus_middle(&key(&mut p, Pane::Middle, KeyCode::Enter));
        assert_export_selected(&p);
        assert!(key(&mut p, Pane::Middle, KeyCode::Esc).is_empty());
        assert!(p.filter.is_empty());
        assert_eq!(p.visible().len(), 6);
        assert_eq!(screen::nodes(&p).len(), 2);
    }

    #[test]
    fn down_from_the_filter_goes_to_the_first_result() {
        let mut p = fixture();
        assert!(key(&mut p, Pane::Nav, KeyCode::Char('/')).is_empty());
        typed(&mut p, Pane::Nav, "export");
        assert_focus_middle(&key(&mut p, Pane::Nav, KeyCode::Down));
        assert!(p.filter_input.is_none(), "↓ closes the field");
        assert_eq!(p.filter, "export", "↓ keeps the words");
        assert_export_selected(&p);
    }

    #[test]
    fn enter_from_the_filter_goes_to_the_results_keeping_the_selection_if_shown() {
        let mut p = fixture();
        let abc12 = child_path();
        // Prove the fixture still puts ABC-12 under ABC-1 in the same status.
        let nodes = screen::nodes(&p);
        let (epic, children) = &nodes[0].groups[0].tops[0];
        assert_eq!(epic.key, "ABC-1");
        assert_eq!(
            children.iter().map(|t| t.key.as_str()).collect::<Vec<_>>(),
            ["ABC-12", "ABC-13"]
        );
        p.tree.borrow_mut().select(abc12.clone());
        assert!(key(&mut p, Pane::Nav, KeyCode::Char('/')).is_empty());
        typed(&mut p, Pane::Nav, "endpoint");
        assert_focus_middle(&key(&mut p, Pane::Nav, KeyCode::Enter));
        assert!(p.filter_input.is_none());
        assert_eq!(visible_keys(&p), ["ABC-12"]);
        assert_eq!(
            p.tree.borrow().selected(),
            abc12.as_slice(),
            "still shown: it stays"
        );
        // A second edit hides the selected child, so Enter chooses the first match.
        assert!(key(&mut p, Pane::Middle, KeyCode::Char('/')).is_empty());
        for _ in 0..8 {
            assert!(key(&mut p, Pane::Middle, KeyCode::Backspace).is_empty());
        }
        typed(&mut p, Pane::Middle, "export");
        assert_focus_middle(&key(&mut p, Pane::Middle, KeyCode::Enter));
        assert!(p.filter_input.is_none());
        assert_export_selected(&p);
    }
}

mod integration {
    use super::*;
    use radar_core::host::Event;
    use radar_core::scenario::Scenario;

    fn press(scenario: &mut Scenario, code: KeyCode) {
        scenario
            .dispatch(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
            .unwrap();
        assert!(
            scenario.is_idle(),
            "filter keys must never start provider work"
        );
    }

    fn typed(scenario: &mut Scenario, text: &str) {
        for c in text.chars() {
            press(scenario, KeyCode::Char(c));
        }
    }

    #[test]
    fn captured_filter_typing_routes_enter_down_and_escape_through_the_host() {
        let mut p = fixture();
        // A regression that starts I/O must fail closed, even before credentials.
        let (offline, provider) = crate::test_support::hosted_plugin(vec![]).unwrap();
        p.provider = offline.provider;
        let abc12 = child_path();
        p.tree.borrow_mut().select(abc12.clone());
        let mut host = radar_core::Host::new(vec![Box::new(p)], NOW).unwrap();
        host.focus = Pane::Nav;
        host.set_view_room(160, 40);
        let mut scenario = Scenario::new(host);

        press(&mut scenario, KeyCode::Char('/'));
        assert!(scenario.host().capturing());
        typed(&mut scenario, "endpoint");
        assert_eq!(super::super::jira(scenario.host()).filter, "endpoint");
        press(&mut scenario, KeyCode::Enter);
        assert_eq!(scenario.host().focus, Pane::Middle);
        assert!(!scenario.host().capturing());
        assert_eq!(
            super::super::jira(scenario.host()).tree.borrow().selected(),
            abc12.as_slice()
        );

        press(&mut scenario, KeyCode::Esc);
        let p = super::super::jira(scenario.host());
        assert!(p.filter.is_empty());
        assert_eq!(p.visible().len(), 6);
        press(&mut scenario, KeyCode::Char('1'));
        assert_eq!(scenario.host().focus, Pane::Nav);
        press(&mut scenario, KeyCode::Char('/'));
        assert!(scenario.host().capturing());
        // In particular, x/o/r must be captured as text rather than actions.
        typed(&mut scenario, "export");
        assert_eq!(super::super::jira(scenario.host()).filter, "export");
        assert!(scenario.host().menu.is_none());
        press(&mut scenario, KeyCode::Down);
        assert_eq!(scenario.host().focus, Pane::Middle);
        assert!(!scenario.host().capturing());
        let p = super::super::jira(scenario.host());
        assert_eq!(p.filter, "export");
        assert_export_selected(p);

        // The only full-frame checkpoint: keep the existing visible wording.
        let shown = scenario.inspect(|host| radar_core::shell::to_text(host, 160, 40));
        assert!(shown.contains("tickets mentioning \"export\""), "{shown}");
        assert!(scenario.desktop_effects().is_empty());
        assert!(provider.ledger().is_empty());
        provider.assert_finished();
    }
}
