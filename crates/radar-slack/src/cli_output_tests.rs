//! Byte-for-byte compatibility fixtures captured on the original PR24 CLI.
use crate::client::{Client, Recorded, Reply};
use crate::demo::{ALICE, TEAM};
use serde_json::json;

// Already in a prior year: catchup labels consult the real clock.
const NOW: i64 = 1_600_100_000;

#[test]
fn both_cli_commands_keep_their_text_and_json_bytes() {
    for command in ["list", "catchup"] {
        for as_json in [false, true] {
            let mut plugin = crate::demo::sample();
            plugin
                .data
                .marks
                .insert(TEAM.into(), "1600079000.000001".into());
            let bodies = [
                json!({"ok":true,"team_id":"T000ACME1","user_id":ALICE,"url":"https://acme.slack.com/"}),
                json!({"ok":true,"messages":[
                    {"ts":"1600099000.000100","username":"Build Bot","text":"status *green* <https://example.test/report|report>","reply_count":2},
                    {"ts":"1600089000.000200","username":"Other Bot","text":"morning\nsecond line"}
                ]}),
            ];
            let replies = bodies
                .into_iter()
                .map(|body| {
                    Ok(Reply {
                        status: 200,
                        body: body.to_string(),
                        ..Default::default()
                    })
                })
                .collect();
            let recorded = Recorded::new(replies);
            let client = Client::new(recorded.clone());
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let output = runtime
                .block_on(async {
                    if command == "list" {
                        crate::list(
                            &client,
                            plugin.cfg.clone(),
                            plugin.cache_snapshot(),
                            Some("team".into()),
                            Some("status bot".into()),
                            as_json,
                            NOW,
                        )
                        .await
                    } else {
                        crate::catchup(
                            &client,
                            plugin.cfg.clone(),
                            plugin.cache_snapshot(),
                            Some("team".into()),
                            as_json,
                            NOW,
                        )
                        .await
                    }
                })
                .unwrap();
            assert_eq!(recorded.requests().len(), 2);
            // Insta normalizes the final newline, so pin the CLI terminator too.
            assert_eq!(
                output.bytes().rev().take_while(|b| *b == b'\n').count(),
                if command == "catchup" && !as_json {
                    2
                } else {
                    1
                }
            );
            insta::assert_snapshot!(
                format!("{command}_{}", if as_json { "json" } else { "text" }),
                output
            );
        }
    }
}
