//! Small status checks on the view text; transport/cache behavior is tested separately.

use radar_core::Host;
use crate::tests::shown;

use super::{FollowedBatch, FollowedResponse, FollowedTarget, followed_action, followed_target};
use crate::client::ListEvidence;
use crate::demo::{NOW, TEAM, sample};
use crate::{SlackPlugin, SyncErrorKey};

fn response(parent_found: bool, truncated: bool) -> Option<FollowedResponse> {
    Some(FollowedResponse {
        parent_found,
        messages: usize::from(parent_found),
        evidence: ListEvidence {
            truncated,
            ..Default::default()
        },
    })
}

fn rendered(p: SlackPlugin) -> String {
    let mut host = Host::new(vec![Box::new(SlackPlugin::new())], NOW).unwrap();
    host.load_config(Some("")).unwrap();
    host.plugins[0] = Box::new(p);
    shown(&host, 240, 40)
}

fn error_lines(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| line.split_once("✗ ").map(|(_, error)| error))
        .map(|error| error.split('│').next().unwrap().trim().to_string())
        .collect()
}

#[test]
fn followed_failures_and_partial_results_show_safe_sync_health() {
    let cases = [
        ("all failed", vec![None, None]),
        ("missing parent", vec![response(false, false)]),
        ("mixed", vec![response(true, false), None]),
        ("paging gap", vec![response(true, true)]),
    ];
    let mut summaries = Vec::new();
    for (label, responses) in cases {
        let mut p = sample();
        p.sync.reading = 1;
        let batch = FollowedBatch {
            updates: vec![],
            targets: responses
                .into_iter()
                .enumerate()
                .map(|(i, response)| FollowedTarget {
                    action: followed_target("T000ACME1", TEAM, &format!("1791000000.{i:06}")),
                    response,
                })
                .collect(),
        };
        let _ = p.on_read(crate::read::Read::Followed {
            action: followed_action("T000ACME1"),
            batch,
        });
        let top = crate::screen::top_right(&p).unwrap();
        let status: Vec<_> = crate::screen::status(&p)
            .into_iter()
            .filter_map(|(text, _)| text.contains("read").then_some(text))
            .collect();
        let text = rendered(p);
        let errors = error_lines(&text);
        assert_eq!(errors.len(), 1, "{text}");
        assert!(!errors[0].contains(TEAM));
        assert!(!errors[0].contains("1791000000"));
        summaries.push(format!(
            "{label}\n{top}\n{}\n{}",
            status.join("\n"),
            errors.join("\n")
        ));
    }
    insta::assert_snapshot!(summaries.join("\n\n"), @r"
    all failed
    synced 40s ago · every 2m
    1 read failed
    Followed review asks could not sync: followed review threads could not be refreshed

    missing parent
    synced 40s ago · every 2m
    1 read failed
    Followed review asks could not sync: followed review threads could not be refreshed

    mixed
    synced 40s ago · every 2m
    1 read failed
    Followed review asks could not sync: some followed review threads could not be fully refreshed

    paging gap
    synced 40s ago · every 2m
    1 read failed
    Followed review asks could not sync: some followed review threads could not be fully refreshed
    ");
}

#[test]
fn typed_error_scopes_keep_existing_rendered_labels_and_order() {
    let mut p = sample();
    p.sync.errors.extend([
        (SyncErrorKey::ForYou, "mentions unavailable".into()),
        (
            SyncErrorKey::Channel("aaa".into()),
            "channel unavailable".into(),
        ),
        (SyncErrorKey::Bootstrap, "workspace unavailable".into()),
    ]);
    let text = rendered(p);
    insta::assert_snapshot!(error_lines(&text).join("\n"), @r"
    Slack could not sync: workspace unavailable
    #aaa could not sync: channel unavailable
    For you could not sync: mentions unavailable
    ");
}
