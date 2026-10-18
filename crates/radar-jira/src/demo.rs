//! The sample data the Jira part's tests use, and `radar shot` draws with
//! (`Plugin::demo`): two sites, their tickets in every default view. Neutral
//! names; no live service. Compiled for tests and under the `demo` feature.

use super::*;
use crate::config::Project;
use crate::model::fixtures::ticket;
use crate::model::{Category, Parent};

pub(crate) const NOW: i64 = radar_core::plugin::DEMO_NOW;

pub(crate) fn site(name: &str, host: &str, color: &str, projects: &[(&str, &str)]) -> Site {
    Site {
        name: name.into(),
        url: format!("https://{host}"),
        email: "alice@example.com".into(),
        token: "env:RADAR_JIRA_TEST_TOKEN".into(),
        color: color.into(),
        projects: projects
            .iter()
            .map(|(k, ac)| Project {
                key: k.to_string(),
                ac_field: ac.to_string(),
            })
            .collect(),
        from: String::new(),
    }
}

/// A workspace's `.mcp.json` whose token is typed in.
pub(crate) const MCP_TYPED: &str = r#"{ "mcpServers": { "tools": { "env": {
    "JIRA_BASE_URL": "https://gamma.atlassian.net",
    "JIRA_EMAIL": "${JIRA_EMAIL}",
    "JIRA_TOKEN": "typed-secret-value",
    "JIRA_PROJECT_KEY": "GAM",
    "JIRA_ACCEPTANCE_CRITERIA_FIELD": "customfield_10044" } } } }"#;

pub(crate) fn mcp(text: &str) -> std::sync::OnceLock<Vec<config::Offer>> {
    std::sync::OnceLock::from(config::mcp_offers(text, "/work/acme/.mcp.json").unwrap())
}

/// Text written as radar's comments are, as Jira would hold it.
pub(crate) fn doc(text: &str) -> radar_core::ui::rich::Doc {
    adf::doc(&adf::from_text(text))
}

/// Two sites: acme watching ABC (with an acceptance-criteria field) and
/// DEF, beta watching XYZ. Each site's tickets for every default view.
pub(crate) fn sample() -> JiraPlugin {
    let mut p = JiraPlugin::new();
    p.now = NOW;
    p.env = Some(vec![
        ("JIRA_OTHER_URL".into(), "https://other.atlassian.net".into()),
        ("JIRA_OTHER_EMAIL".into(), "alice@example.com".into()),
        ("JIRA_OTHER_API_TOKEN".into(), "not-a-real-token".into()),
    ]);
    p.mcp = mcp(MCP_TYPED);
    p.cfg.sites = vec![
        site("acme", "acme.atlassian.net", "blue", &[("ABC", "customfield_10050"), ("DEF", "")]),
        site("beta", "beta.atlassian.net", "green", &[("XYZ", "")]),
    ];
    // The demo has provider identities even when tests skip media preloading.
    p.connections.reconcile(&p.cfg.sites);
    let mut epic = ticket("ABC-1", "Dashboard for the stats", "In Progress", Category::Doing);
    epic.id = "10001".into();
    epic.kind = "Epic".into();
    epic.updated = "2026-09-20T09:00:00.000+0000".into();
    let mut story = ticket("ABC-12", "Add a stats endpoint for the dashboard", "In Progress", Category::Doing);
    story.id = "10012".into();
    story.parent = Some(Parent {
        key: "ABC-1".into(),
        summary: "Dashboard for the stats".into(),
        status: "In Progress".into(),
        category: Category::Doing,
    });
    story.labels = vec!["backend".into()];
    story.ac = doc("- GET /stats answers within 200 ms\n- counts are per day");
    let mut sub = ticket("ABC-13", "Count requests per route", "In Progress", Category::Doing);
    sub.id = "10013".into();
    sub.kind = "Sub-task".into();
    sub.subtask = true;
    sub.assignee = "Bob Example".into();
    sub.assignee_id = "acc-bob".into();
    sub.parent = story.parent.clone();
    let mut bug = ticket("ABC-20", "Stats page shows yesterday twice", "To Do", Category::Todo);
    bug.id = "10020".into();
    bug.kind = "Bug".into();
    bug.assignee.clear();
    bug.assignee_id.clear();
    bug.updated = "2026-08-01T09:00:00.000+0000".into();
    let mut review = ticket("DEF-3", "Rename the export job", "In Review", Category::Doing);
    review.id = "20003".into();
    let mut other = ticket("XYZ-7", "Move the nightly build", "To Do", Category::Todo);
    other.id = "30007".into();
    let acme: Vec<Ticket> = vec![story, sub, epic, bug, review];
    let beta: Vec<Ticket> = vec![other];
    for v in p.cfg.views.clone() {
        for (i, ts) in [(0, &acme), (1, &beta)] {
            if let Some(jql) = p.jql_for(i, &v) {
                let name = p.cfg.sites[i].name.clone();
                p.on_loaded(name, jql, (None, ts.clone()));
            }
        }
    }
    p.data.get_mut("acme").unwrap().me = Some(Me {
        account_id: "acc-alice".into(),
        name: "Alice Example".into(),
    });
    // Unit tests count the pages read; the demo build opens ABC-13 offline.
    #[cfg(not(test))]
    preload_media_page(&mut p);
    p
}

/// This demo contract constructs no host or frame. `sample` still uses the
/// production constructor, so it is not a pure fixture-builder test.
#[cfg(test)]
mod identity_tests {
    use super::*;

    #[test]
    fn sample_tickets_have_stable_detail_identities_without_media_preload() {
        let p = sample();
        assert!(p.details.is_empty(), "tests do not preload a page");
        assert_eq!(p.next_page_read, 0);
        for (site, key, id) in [
            ("acme", "ABC-1", "10001"),
            ("acme", "ABC-12", "10012"),
            ("acme", "ABC-13", "10013"),
            ("acme", "ABC-20", "10020"),
            ("acme", "DEF-3", "20003"),
            ("beta", "XYZ-7", "30007"),
        ] {
            assert_eq!(p.data[site].tickets[key].id, id);
            p.tree.borrow_mut().select(vec![screen::ticket_id(site, key)]);
            let identity = p
                .detail_identity(SCREEN.id, Pane::Detail)
                .expect("demo ticket identity");
            assert_eq!(identity.subject, [site, key, id]);
            assert_eq!(Some(identity.connection), p.connections.generation(site));
        }
    }
}

/// ABC-13's page already read, so it opens offline (`radar shot`, the demo
/// build): the one ticket whose description holds media.
fn preload_media_page(p: &mut JiraPlugin) {
    let _ = page::fetch(p, "acme", "ABC-13", true);
    let read = p.details[&detail_key("acme", "ABC-13")].request.expect("pending page read");
    let detail = media_detail(p);
    let done = page::on_detail(p, "acme", "ABC-13", read, Ok(client::DetailRead { detail, ..Default::default() }));
    assert!(done);
}

/// A ticket description holding media: a screenshot Jira keeps (an image by
/// name, with its size), a design drawing on the web (an external image) and
/// two attached files, one of them gone from the ticket's attachments.
pub(crate) fn media_description() -> radar_core::ui::rich::Doc {
    adf::doc(&serde_json::json!({"type": "doc", "version": 1, "content": [
        {"type": "paragraph", "content": [{"type": "text", "text": "The count is wrong on the second row:"}]},
        {"type": "mediaSingle", "attrs": {"layout": "center"}, "content": [
            {"type": "media", "attrs": {"type": "file", "id": "m-1", "alt": "dashboard.png", "width": 640, "height": 480}}
        ]},
        {"type": "mediaSingle", "content": [
            {"type": "media", "attrs": {"type": "external", "url": "https://example.com/design/stats.png", "alt": "stats design"}}
        ]},
        {"type": "mediaGroup", "content": [
            {"type": "media", "attrs": {"type": "file", "id": "m-2", "alt": "requests.csv"}},
            {"type": "media", "attrs": {"type": "file", "id": "m-3", "alt": "trace.log"}}
        ]}
    ]}))
}

/// The page of ABC-13 as Jira would answer it: its description holds media.
pub(crate) fn media_detail(p: &JiraPlugin) -> Detail {
    Detail {
        ticket: Ticket {
            description: media_description(),
            ..p.data["acme"].tickets["ABC-13"].clone()
        },
        sprint: Some("Sprint 41".into()),
        ..Default::default()
    }
}

/// The ticket page of ABC-13 (its description holds media) as the view data a
/// renderer is sent, at 80x24 and 160x40. With `RADAR_MEDIA_FIXTURES_DIR` set the
/// JSON is written there, `jira_ticket-media_<w>x<h>.json`:
///
/// RADAR_MEDIA_FIXTURES_DIR=$PWD/ink/test/fixtures/media \
///   CARGO_TARGET_DIR=target/fleet cargo nextest run -p radar-jira -E 'test(media_page_views)'
#[cfg(test)]
mod media_fixtures {
    use super::*;
    use radar_core::host::Event;

    fn press(host: &mut radar_core::Host, code: KeyCode) {
        let _ = radar_core::shell::to_text(host, 160, 40);
        host.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    }

    /// The sample's host with ABC-13's page open, reached by keys alone.
    fn opened() -> (radar_core::Host, usize) {
        let mut p = sample();
        preload_media_page(&mut p);
        let mut host = radar_core::Host::new(vec![Box::new(p)], NOW).unwrap();
        for downs in 0..30 {
            let shown = radar_core::shell::to_text(&host, 160, 40);
            if shown.lines().nth(1).is_some_and(|l| l.contains("╭ 3 ABC-13 ")) {
                press(&mut host, KeyCode::Enter);
                return (host, downs);
            }
            press(&mut host, KeyCode::Down);
        }
        panic!("ABC-13 never selected");
    }

    #[test]
    fn media_page_views() {
        let (host, downs) = opened();
        let shown = radar_core::shell::to_text(&host, 160, 40);
        assert!(shown.contains("dashboard.png"), "{shown}");
        assert!(shown.contains("requests.csv"), "{shown}");
        eprintln!("ABC-13 is {downs} Down presses from the start; then Enter");
        drop(host);
        for (w, h) in [(80u16, 24u16), (160, 40)] {
            let head = radar_core::headless::Headless::new(opened().0, w, h);
            let view = head.screen().expect("the ticket screen has a view");
            let json = serde_json::to_string_pretty(&view).unwrap();
            assert!(json.contains("\"media\"") && json.contains("dashboard.png") && json.contains("stats.png"), "{json}");
            if let Ok(dir) = std::env::var("RADAR_MEDIA_FIXTURES_DIR") {
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(format!("{dir}/jira_ticket-media_{w}x{h}.json"), json + "\n").unwrap();
            }
        }
    }
}
