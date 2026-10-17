//! Downstream tests begin at an explicitly initialized selection. The one
//! equivalence contract still executes the real refresh, parser, host completion
//! path and selection, so this shortcut cannot stand in for their coverage.

use super::*;
use crate::{SCREEN, screen};
use radar_core::host::{Event, JournalRecord};
use radar_core::scenario::Scenario;
use radar_core::view::{Intent, TreeRow};
use radar_core::{Effect, Host, Outcome, Pane, Plugin};
use serde_json::json;

fn get(path: &str) -> Req {
    Req {
        method: Method::Get,
        path: path.into(),
        body: None,
    }
}

fn identity() -> ExpectedRequest {
    ExpectedRequest::json(
        "identity",
        "watch",
        "https://watch.invalid",
        get("/rest/api/3/myself"),
        json!({"accountId": "vimes", "displayName": "Sam Vimes"}),
    )
}

fn scenario(plugin: JiraPlugin) -> Scenario {
    let mut host = Host::new(vec![Box::new(plugin)], radar_core::plugin::DEMO_NOW).unwrap();
    host.focus = Pane::Middle;
    host.set_view_room(160, 40);
    Scenario::new(host)
}

fn invoke(plugin: &mut JiraPlugin, action: &str) -> Vec<Effect> {
    match plugin.intent(
        SCREEN.id,
        Pane::Middle,
        &Intent::Invoke {
            action: action.into(),
        },
    ) {
        Outcome::Handled(effects) => effects,
        Outcome::Ignored => panic!("selected ticket ignored {action}"),
    }
}

#[test]
fn selected_summary_and_key_drive_the_real_subject_and_copy_action() {
    let (mut plugin, fixture) = TicketSelected::new(
        TicketBuilder::new()
            .key("WATCH-99")
            .id("10099")
            .summary("Find the missing dragon"),
    )
    .build(vec![])
    .unwrap();

    let subject = plugin.subject(SCREEN.id, Pane::Detail).unwrap();
    assert_eq!(subject.id, "WATCH-99");
    assert_eq!(subject.field("title"), Some("Find the missing dragon"));
    assert!(matches!(
        invoke(&mut plugin, "jira.tickets.copy_link").as_slice(),
        [Effect::Copy(link), Effect::Notify(_)] if link == "https://watch.invalid/browse/WATCH-99"
    ));
    assert!(fixture.ledger().is_empty());
    fixture.assert_finished();
}

#[test]
fn selected_status_and_assignee_drive_the_real_domain_rules() {
    let (plugin, fixture) = TicketSelected::new(
        TicketBuilder::new()
            .status("In Review", Category::Doing)
            .assignee("vimes", "Sam Vimes"),
    )
    .build(vec![])
    .unwrap();

    let Some(screen::Selected::Ticket(item)) = screen::selected(&plugin) else {
        panic!("checkpoint must select a ticket");
    };
    assert!(plugin.is_mine(&item.site.name, item.t));
    assert_eq!(plugin.assignees(), ["Sam Vimes"]);
    assert_eq!(
        plugin.tree.borrow().selected(),
        ["s:watch", "g:watch:In Review", "t:watch:WATCH-12"]
    );
    assert!(
        screen::status(&plugin)
            .iter()
            .any(|(text, _)| text == "1 doing")
    );
    assert_eq!(
        plugin
            .subject(SCREEN.id, Pane::Middle)
            .unwrap()
            .field("status"),
        Some("In Review")
    );
    assert!(fixture.ledger().is_empty());
    fixture.assert_finished();
}

#[test]
fn overrides_replace_stable_keys_without_sharing_or_reselecting() {
    let starting = TicketSelected::new(TicketBuilder::new()).with_ticket(
        TicketBuilder::new()
            .key("WATCH-13")
            .id("10013")
            .summary("Check the gates"),
    );
    let (original, original_fixture) = starting.clone().build(vec![]).unwrap();
    let (changed, fixture) = starting
        .with_ticket(
            TicketBuilder::new()
                .summary("Count the dragons")
                .status("Done", Category::Done),
        )
        .build(vec![])
        .unwrap();
    let jql = changed.jql_for(0, &changed.cfg.views[0]).unwrap();
    assert_eq!(changed.data["watch"].found[&jql], ["WATCH-12", "WATCH-13"]);
    assert_eq!(changed.data["watch"].tickets.len(), 2);
    assert_eq!(
        original.data["watch"].tickets["WATCH-12"].summary,
        "Patrol the Shades"
    );
    assert_eq!(
        changed.data["watch"].tickets["WATCH-12"].summary,
        "Count the dragons"
    );
    assert!(
        matches!(screen::selected(&changed), Some(screen::Selected::Ticket(item)) if item.t.key == "WATCH-12")
    );
    let path = changed.tree.borrow().selected().to_vec();
    assert_eq!(path, ["s:watch", "g:watch:Done", "t:watch:WATCH-12"]);
    assert!(
        changed.tree.borrow().is_open(&path[..2]),
        "Done must be opened to show the selected row"
    );
    original_fixture.assert_finished();
    fixture.assert_finished();
}

#[test]
fn explicit_selection_uses_the_exact_identity_and_rejects_missing_targets() {
    let starting = TicketSelected::new(TicketBuilder::new())
        .with_ticket(TicketBuilder::new().key("WATCH-13").id("10013"));
    let (plugin, fixture) = starting
        .clone()
        .select("watch", "WATCH-13")
        .build(vec![])
        .unwrap();
    assert!(
        matches!(screen::selected(&plugin), Some(screen::Selected::Ticket(item)) if item.t.key == "WATCH-13")
    );
    fixture.assert_finished();
    for (site, key, expected) in [
        ("witches", "WATCH-12", "site does not exist: witches"),
        (
            "watch",
            "WATCH-404",
            "ticket does not exist: watch/WATCH-404",
        ),
        ("watch", "s:watch", "ticket does not exist: watch/s:watch"),
    ] {
        let error = starting
            .clone()
            .select(site, key)
            .build(vec![])
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "{error}");
    }
}

#[tokio::test]
async fn selected_checkpoint_keeps_strict_injection_across_configure_restore_and_reset() {
    let (mut plugin, fixture) = TicketSelected::new(TicketBuilder::new())
        .build(vec![identity()])
        .unwrap();
    let cache = plugin.snapshot().unwrap();
    plugin.configure(None).unwrap();
    plugin.restore(json!({"data": {}}));
    plugin
        .configure(Some(&toml::Value::try_from(fixture_settings()).unwrap()))
        .unwrap();
    plugin.restore(cache);
    assert!(plugin.vars().is_empty());
    assert!(plugin.offers().is_empty());
    let client = plugin.provider.client(&plugin.cfg.sites[0]).unwrap();
    assert_eq!(client.myself().await.unwrap().account_id, "vimes");
    fixture.assert_finished();
    assert!(client.myself().await.is_err());
    assert_eq!(fixture.ledger().len(), 2);
    assert!(fixture.ledger()[1].key.is_none());
    assert!(
        fixture
            .check_finished()
            .unwrap_err()
            .to_string()
            .contains("unexpected request")
    );
}

#[tokio::test]
async fn selected_checkpoint_matches_executed_host_refresh_and_selection_without_faking_history() {
    let ticket = TicketBuilder::new()
        .summary("Find the missing dragon")
        .status("Done", Category::Done)
        .assignee("vimes", "Sam Vimes");
    let (checkpoint, checkpoint_fixture) =
        TicketSelected::new(ticket.clone()).build(vec![]).unwrap();
    assert_eq!(checkpoint.now, radar_core::plugin::DEMO_NOW);
    assert!(checkpoint.sync.last_ok.is_none());
    assert!(checkpoint.sync.in_flight.is_empty());
    assert!(checkpoint.data["watch"].synced.is_empty());
    assert!(checkpoint.data["watch"].code.is_empty());
    assert!(checkpoint.details.is_empty() && checkpoint.pages.is_empty());
    assert!(checkpoint.sent.is_empty());
    let checkpoint = scenario(checkpoint);
    assert!(checkpoint.records().is_empty());
    assert!(checkpoint.cache_writes().is_empty());

    // A complete Jira response matching the owned model. This path still runs
    // the production parser; no JiraMsg success or journal result is injected.
    let ticket = ticket.build();
    let issue = json!({
        "id": ticket.id, "key": ticket.key,
        "fields": {
            "summary": ticket.summary,
            "issuetype": {"name": ticket.kind, "subtask": false},
            "status": {"name": ticket.status, "statusCategory": {"key": "done"}},
            "assignee": {"accountId": ticket.assignee_id, "displayName": ticket.assignee},
            "project": {"key": ticket.project, "name": ticket.project_name},
            "created": ticket.created, "updated": ticket.updated
        }
    });
    let fields = crate::model::list_fields(&[]).join(",");
    let search = format!(
        "/rest/api/3/search/jql?jql={}&fields={}&maxResults=100",
        crate::client::enc("project = WATCH ORDER BY updated DESC"),
        crate::client::enc(&fields)
    );
    let (empty, executed_fixture) = hosted_plugin(vec![
        identity(),
        ExpectedRequest::json(
            "tickets",
            "watch",
            "https://watch.invalid",
            get(&search),
            json!({"issues": [issue], "isLast": true}),
        ),
        ExpectedRequest::json(
            "development",
            "watch",
            "https://watch.invalid",
            get("/rest/dev-status/latest/issue/summary?issueId=10012"),
            json!({"summary": {"pullrequest": {"byInstanceType": {}}}}),
        ),
    ])
    .unwrap();
    let mut executed = scenario(empty);
    executed
        .dispatch(Event::Intent(Intent::Invoke {
            action: "jira.tickets.refresh".into(),
        }))
        .unwrap();
    executed.settle(100).await.unwrap();
    let requests = executed_fixture.ledger();
    assert_eq!(
        requests
            .iter()
            .map(|request| request.key.as_deref())
            .collect::<Vec<_>>(),
        [Some("identity"), Some("tickets"), Some("development")],
        "exact request ledger: {requests:#?}"
    );
    executed_fixture.assert_finished();

    // Select by the frontend's real intents, with the independently expected
    // path. Done begins folded, so it must first be made visible.
    let path = ["s:watch", "g:watch:Done", "t:watch:WATCH-12"].map(String::from);
    executed.inspect(|host| host.screen_view(0, SCREEN.id).unwrap());
    executed
        .dispatch(Event::Intent(Intent::Fold {
            target: TreeRow::target_of(&path[..2]),
            open: true,
        }))
        .unwrap();
    executed
        .dispatch(Event::Intent(Intent::Select {
            target: TreeRow::target_of(&path),
        }))
        .unwrap();
    assert!(executed.is_idle());
    let checkpoint_view = checkpoint.inspect(|host| host.screen_view(0, SCREEN.id).unwrap());
    let executed_view = executed.inspect(|host| host.screen_view(0, SCREEN.id).unwrap());
    assert_eq!(
        checkpoint_view, executed_view,
        "selection, rows, preview and actions must agree"
    );

    let checkpoint_data = checkpoint.host().plugins[0].snapshot().unwrap();
    let executed_data = executed.host().plugins[0].snapshot().unwrap();
    for field in ["tickets", "found", "me"] {
        assert_eq!(
            checkpoint_data["data"]["watch"][field], executed_data["data"]["watch"][field],
            "{field}"
        );
    }
    for field in ["synced", "code"] {
        assert!(
            checkpoint_data["data"]["watch"]
                .get(field)
                .is_none_or(|value| value == &json!({}))
        );
        assert!(
            !executed_data["data"]["watch"][field]
                .as_object()
                .unwrap()
                .is_empty()
        );
    }
    let refresh_id = executed
        .records()
        .iter()
        .find_map(|record| match record {
            JournalRecord::Begin { id, descriptor, .. }
                if descriptor.kind.as_str() == "jira.sync.view" =>
            {
                Some(id)
            }
            _ => None,
        })
        .expect("executed refresh creates a real action record");
    assert!(executed.records().iter().any(|record| matches!(record,
        JournalRecord::Outcome { id, outcome: radar_core::journal::Outcome::Succeeded, .. } if id == refresh_id)));
    assert!(
        checkpoint.records().is_empty(),
        "direct initialization creates no history"
    );
    assert!(
        checkpoint_fixture.ledger().is_empty(),
        "direct initialization sends no requests"
    );
    assert!(checkpoint.desktop_effects().is_empty() && executed.desktop_effects().is_empty());
    checkpoint_fixture.assert_finished();
    executed_fixture.assert_finished();
}
