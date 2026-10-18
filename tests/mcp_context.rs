//! Connected source reads use real owner caches; no network or agent is started.
#![cfg(all(
    feature = "gitlab",
    feature = "jira",
    feature = "slack",
    feature = "git",
    feature = "demo"
))]
use radar_core::{
    Plugin,
    context::{Service, catalog, reference},
};
use serde_json::json;

#[test]
fn owners_join_change_ticket_discussion_and_checkout_without_losing_source_text() {
    let mut plugins: Vec<Box<dyn Plugin>> = vec![
        Box::new(radar_gitlab::GitlabPlugin::new()),
        Box::new(radar_jira::JiraPlugin::new()),
        Box::new(radar_slack::SlackPlugin::new()),
        Box::new(radar_git::GitPlugin::new()),
    ];
    for p in &mut plugins {
        p.demo();
    }
    let roots: Vec<_> = catalog(&plugins)
        .into_iter()
        .filter(|(_, key)| key.kind == "merge-request")
        .map(|(_, key)| reference(&key))
        .collect();
    assert!(!roots.is_empty());
    let mut service = Service::default();
    let packet = service
        .execute(
            &mut plugins,
            "context",
            json!({"roots":roots,"max_bytes":262144}),
            1,
        )
        .unwrap();
    let items = packet["items"].as_array().unwrap();
    let providers: std::collections::BTreeSet<_> = items
        .iter()
        .filter_map(|i| i["subject"]["provider"].as_str())
        .collect();
    assert!(providers.contains("gitlab"), "{providers:?}");
    assert!(providers.contains("jira"), "{providers:?}");
    assert!(providers.contains("slack"), "{providers:?}");
    let thread = items
        .iter()
        .find(|i| i["subject"]["provider"] == "slack")
        .unwrap();
    let read = service.execute(&mut plugins,"read",json!({"refs":[thread["ref"]],"snapshot_id":packet["snapshot_id"],"sections":["thread"]}),2).unwrap();
    assert!(read["text"].as_str().unwrap().contains("## thread"));
    assert!(read["total_bytes"].as_u64().unwrap() > 100);
}
