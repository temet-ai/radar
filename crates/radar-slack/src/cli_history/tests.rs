use super::*;
use crate::client::{Recorded, Reply, Request};
use radar_core::Plugin;
use std::sync::Arc;

const NOW: i64 = 1_791_271_000;

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn ok(body: Value) -> anyhow::Result<Reply> {
    Ok(Reply {
        status: 200,
        body: body.to_string(),
        ..Default::default()
    })
}

fn auth() -> Value {
    json!({"ok":true,"team_id":"TTEST","user_id":"UAUTHSECRET","url":"https://private-workspace.example/","team":"Workspace secret","user":"Auth name secret"})
}

fn settings(n: usize) -> Settings {
    let mut cfg = Settings {
        token: "xoxp-cli-test-secret".into(),
        ..Default::default()
    };
    cfg.channels = (0..n)
        .map(|i| config::Channel {
            alias: format!("alias-secret-{i}"),
            id: format!("CTEST{i:05}"),
            name: format!("name-secret-{i}"),
            purpose: "purpose secret".into(),
            kind: config::Kind::Team,
            color: "blue".into(),
            watched: true,
            reader: None,
        })
        .collect();
    cfg
}

fn execute(
    command: &str,
    cfg: Settings,
    data: cache::AccountCache,
    selection: Option<String>,
    json: bool,
    replies: Vec<anyhow::Result<Reply>>,
) -> (CliExecution, Arc<Recorded>) {
    let rec = Recorded::new(replies);
    let c = Client::new(rec.clone()).with_wait(Arc::new(|_| Box::pin(async {})));
    let mut evidence = Evidence::default();
    let result = rt().block_on(async {
        if command == "list" {
            list_inner(&c, cfg, data, selection, None, json, NOW, &mut evidence).await
        } else {
            catchup_inner(&c, cfg, data, selection, json, NOW, &mut evidence).await
        }
    });
    (CliExecution::new(result, evidence.observations), rec)
}

fn fact<'a>(execution: &'a CliExecution, kind: &str) -> &'a radar_core::plugin::CliObservation {
    execution
        .observations
        .iter()
        .find(|fact| fact.action().kind.as_str() == kind)
        .unwrap_or_else(|| panic!("missing {kind}"))
}

fn facts(execution: &CliExecution, kind: &str) -> usize {
    execution
        .observations
        .iter()
        .filter(|fact| fact.action().kind.as_str() == kind)
        .count()
}

fn request(method: &'static str, params: &[(&str, &str)]) -> Request {
    Request {
        method,
        params: params
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        write: false,
        body: None,
    }
}

#[test]
fn empty_complete_reads_are_successful_and_requests_keep_exact_parameters() {
    for command in ["list", "catchup"] {
        let cfg = settings(1);
        let since = format!("{}.000000", NOW - cfg.days as i64 * 86_400);
        let (got, rec) = execute(
            command,
            cfg,
            Default::default(),
            None,
            true,
            vec![ok(auth()), ok(json!({"ok":true,"messages":[]}))],
        );
        assert_eq!(got.result.as_ref().unwrap(), "[]\n");
        let read = fact(&got, "slack.cli.history.messages");
        assert_eq!(read.outcome(), Outcome::Succeeded);
        assert_eq!(read.action().metadata.count, Some(0));
        assert!(!read.action().metadata.truncated);
        assert_eq!(read.action().subjects[0].key.instance, "TTEST");
        assert_eq!(read.action().subjects[0].key.id, "CTEST00000");
        assert!(fact(&got, "slack.cli.auth").action().subjects.is_empty());
        assert_eq!(
            rec.requests(),
            vec![
                request("auth.test", &[]),
                request(
                    "conversations.history",
                    &[
                        ("channel", "CTEST00000"),
                        ("oldest", &since),
                        ("inclusive", "false"),
                        ("limit", "200")
                    ]
                )
            ]
        );
    }
}

#[test]
fn refused_selection_and_empty_configuration_do_not_authenticate() {
    for command in ["list", "catchup"] {
        for (cfg, selection, expected) in [
            (settings(0), None, "no watched channels"),
            (
                settings(1),
                Some("private-unknown-selection".into()),
                "no channel called",
            ),
        ] {
            let (got, rec) = execute(command, cfg, Default::default(), selection, false, vec![]);
            assert!(
                got.result
                    .as_ref()
                    .unwrap_err()
                    .to_string()
                    .contains(expected)
            );
            assert!(rec.requests().is_empty());
            assert_eq!(got.observations.len(), 1);
            assert_eq!(fact(&got, "slack.cli.refused").outcome(), Outcome::Blocked);
            assert!(fact(&got, "slack.cli.refused").action().subjects.is_empty());
        }
    }
}

#[test]
fn auth_failure_and_incomplete_identity_never_adopt_cached_scope() {
    for command in ["list", "catchup"] {
        for response in [
            json!({"ok":false,"error":"AUTH_ERROR_SECRET"}),
            json!({"ok":true,"team_id":"TTEST","user_id":""}),
        ] {
            let (got, rec) = execute(
                command,
                settings(1),
                crate::demo::sample().cache_snapshot(),
                None,
                false,
                vec![ok(response.clone())],
            );
            assert!(got.result.is_err());
            assert_eq!(rec.requests().len(), 1);
            assert!(
                got.observations
                    .iter()
                    .all(|fact| fact.action().subjects.is_empty())
            );
            assert_eq!(facts(&got, "slack.cli.history.messages"), 0);
            assert_eq!(
                fact(&got, "slack.cli.auth").outcome(),
                if response["ok"] == true {
                    Outcome::Succeeded
                } else {
                    Outcome::Failed
                }
            );
            if response["ok"] == true {
                assert_eq!(fact(&got, "slack.cli.identity").outcome(), Outcome::Partial);
            }
        }
    }
}

#[test]
fn local_token_refusal_is_observed_without_resolving_or_reading_provider() {
    for command in ["list", "catchup"] {
        let mut plugin = SlackPlugin::new();
        plugin.cfg = settings(1);
        plugin.cfg.token.clear();
        let rec = Recorded::new(vec![]);
        plugin.via = crate::Via(Some(rec.clone()));
        let matches = plugin
            .cli()
            .unwrap()
            .try_get_matches_from(["slack", command])
            .unwrap();
        let got = rt().block_on(plugin.run_cli_observed(&matches));
        assert!(
            got.result
                .as_ref()
                .unwrap_err()
                .to_string()
                .contains("run radar, press , for settings")
        );
        assert_eq!(fact(&got, "slack.cli.refused").outcome(), Outcome::Blocked);
        assert!(rec.requests().is_empty());
    }
}

#[test]
fn real_cutoff_and_malformed_pages_are_distinct_partial_facts() {
    for command in ["list", "catchup"] {
        let messages: Vec<_> = (0..301).map(|i| json!({"ts":format!("1791271{i:03}.000000"),"text":"private message","username":"private bot"})).collect();
        let cases = [
            (json!({"ok":true,"messages":messages}), true, 0, 300),
            (
                json!({"ok":true,"messages":[],"has_more":true}),
                false,
                1,
                0,
            ),
            (json!({"ok":true,"messages":{}}), false, 1, 0),
            (
                json!({"ok":true,"messages":[],"response_metadata":{"next_cursor":99}}),
                false,
                1,
                0,
            ),
        ];
        for (page, truncated, incomplete, count) in cases {
            let (got, _) = execute(
                command,
                settings(1),
                Default::default(),
                None,
                true,
                vec![ok(auth()), ok(page)],
            );
            assert!(got.result.is_ok());
            let read = fact(&got, "slack.cli.history.messages");
            assert_eq!(read.outcome(), Outcome::Partial);
            assert_eq!(read.action().metadata.truncated, truncated);
            assert_eq!(read.action().metadata.count, Some(count));
            assert_eq!(
                facts(&got, "slack.cli.history.lists.truncated"),
                usize::from(truncated)
            );
            assert_eq!(
                facts(&got, "slack.cli.history.pages.incomplete"),
                incomplete
            );
        }
    }
}

#[test]
fn earlier_channel_evidence_survives_later_page_error_without_partial_output() {
    for command in ["list", "catchup"] {
        let replies = vec![
            ok(auth()),
            ok(json!({"ok":true,"messages":[]})),
            ok(
                json!({"ok":true,"messages":[],"response_metadata":{"next_cursor":"CURSOR_SECRET"}}),
            ),
            ok(json!({"ok":false,"error":"PAGE_ERROR_SECRET"})),
        ];
        let (got, rec) = execute(
            command,
            settings(2),
            Default::default(),
            None,
            true,
            replies,
        );
        assert!(got.result.is_err());
        assert_eq!(
            fact(&got, "slack.cli.history.messages").action().subjects[0]
                .key
                .id,
            "CTEST00000"
        );
        assert_eq!(
            fact(&got, "slack.cli.history.messages").outcome(),
            Outcome::Succeeded
        );
        assert_eq!(
            fact(&got, "slack.cli.history.errors").action().subjects[0]
                .key
                .id,
            "CTEST00001"
        );
        assert_eq!(
            fact(&got, "slack.cli.history.errors").outcome(),
            Outcome::Failed
        );
        assert_eq!(rec.requests().len(), 4);
        assert_eq!(
            rec.requests()[3].params.last().unwrap(),
            &("cursor".into(), "CURSOR_SECRET".into())
        );
    }
}

#[test]
fn optional_name_failure_keeps_rows_and_has_separate_fallback_evidence() {
    for command in ["list", "catchup"] {
        let replies = vec![
            ok(auth()),
            ok(
                json!({"ok":true,"messages":[{"ts":"1791270000.000001","user":"UAUTHORSECRET","text":"MESSAGE_SECRET"}]}),
            ),
            ok(json!({"ok":false,"error":"NAME_ERROR_SECRET"})),
        ];
        let (got, rec) = execute(
            command,
            settings(1),
            Default::default(),
            None,
            true,
            replies,
        );
        let output: Value = serde_json::from_str(got.result.as_ref().unwrap()).unwrap();
        assert_eq!(output[0]["text"], "MESSAGE_SECRET");
        assert_eq!(output[0]["author"]["name"], "UAUTHORSECRET");
        assert_eq!(
            fact(&got, "slack.cli.history.messages").outcome(),
            Outcome::Succeeded
        );
        assert_eq!(
            fact(&got, "slack.cli.names.fallback").outcome(),
            Outcome::Partial
        );
        assert_eq!(
            fact(&got, "slack.cli.names.fallback")
                .action()
                .metadata
                .count,
            Some(1)
        );
        assert_eq!(
            fact(&got, "slack.cli.names.returned")
                .action()
                .metadata
                .count,
            Some(0)
        );
        assert_eq!(
            rec.requests()[2],
            request("users.info", &[("user", "UAUTHORSECRET")])
        );
        let serialized = got
            .observations
            .iter()
            .map(|fact| json!({"kind": fact.action().kind, "origin": fact.action().origin, "subjects": fact.action().subjects, "metadata": fact.action().metadata}).to_string())
            .collect::<String>();
        for sentinel in [
            "UAUTHORSECRET",
            "UAUTHSECRET",
            "MESSAGE_SECRET",
            "NAME_ERROR_SECRET",
            "alias-secret",
            "name-secret",
            "Workspace secret",
            "Auth name secret",
            "private-workspace",
            "xoxp-cli-test-secret",
        ] {
            assert!(
                !serialized.contains(sentinel),
                "leaked {sentinel}: {serialized}"
            );
        }
    }
}

#[test]
fn observation_limit_does_not_truncate_configured_channel_processing_or_output() {
    for command in ["list", "catchup"] {
        let n = radar_core::plugin::MAX_CLI_OBSERVATIONS + 5;
        let mut replies = vec![ok(auth())];
        replies.extend((0..n).map(|_| ok(json!({"ok":true,"messages":[]}))));
        let (got, rec) = execute(
            command,
            settings(n),
            Default::default(),
            None,
            false,
            replies,
        );
        assert!(got.result.is_ok());
        assert_eq!(rec.requests().len(), n + 1);
        assert_eq!(
            got.observations.len(),
            radar_core::plugin::MAX_CLI_OBSERVATIONS
        );
        assert_eq!(got.observations.omitted(), 7);
        if command == "catchup" {
            assert!(
                got.result
                    .as_ref()
                    .unwrap()
                    .contains(&format!("#alias-secret-{}", n - 1))
            );
        }
    }
}

#[test]
fn successful_paging_keeps_request_order_and_does_not_claim_a_cutoff() {
    for command in ["list", "catchup"] {
        let (got, rec) = execute(
            command,
            settings(1),
            Default::default(),
            None,
            true,
            vec![
                ok(auth()),
                ok(
                    json!({"ok":true,"messages":[],"response_metadata":{"next_cursor":"CURSOR_SECRET"}}),
                ),
                ok(json!({"ok":true,"messages":[]})),
            ],
        );
        assert_eq!(got.result.as_ref().unwrap(), "[]\n");
        assert_eq!(
            fact(&got, "slack.cli.history.messages").outcome(),
            Outcome::Succeeded
        );
        assert_eq!(rec.requests().len(), 3);
        let mut expected = rec.requests()[1].clone();
        expected
            .params
            .push(("cursor".into(), "CURSOR_SECRET".into()));
        assert_eq!(rec.requests()[2], expected);
    }
}

#[test]
fn repeated_failed_name_lookups_keep_each_commands_existing_retry_behavior() {
    for (command, attempts) in [("list", 1), ("catchup", 2)] {
        let mut replies = vec![
            ok(auth()),
            ok(json!({"ok":true,"messages":[
                {"ts":"1791270000.000001","user":"UREPEATSECRET","text":"first"},
                {"ts":"1791270000.000002","user":"UREPEATSECRET","text":"second"}
            ]})),
        ];
        replies.extend((0..attempts).map(|_| ok(json!({"ok":false,"error":"NAME_ERROR_SECRET"}))));
        let (got, rec) = execute(
            command,
            settings(1),
            Default::default(),
            None,
            true,
            replies,
        );
        let output: Value = serde_json::from_str(got.result.as_ref().unwrap()).unwrap();
        assert_eq!(output.as_array().unwrap().len(), 2);
        assert!(
            output
                .as_array()
                .unwrap()
                .iter()
                .all(|row| row["author"]["name"] == "UREPEATSECRET")
        );
        assert_eq!(rec.requests().len(), attempts + 2);
        assert!(
            rec.requests()[2..]
                .iter()
                .all(|got| *got == request("users.info", &[("user", "UREPEATSECRET")]))
        );
        assert_eq!(
            fact(&got, "slack.cli.names.attempted")
                .action()
                .metadata
                .count,
            Some(attempts as u32)
        );
        assert_eq!(
            fact(&got, "slack.cli.names.fallback")
                .action()
                .metadata
                .count,
            Some(attempts as u32)
        );
        assert_eq!(
            fact(&got, "slack.cli.history.messages").outcome(),
            Outcome::Succeeded
        );
    }
}

#[test]
fn provider_rate_limit_retry_is_not_changed_or_misreported_as_a_read_failure() {
    for command in ["list", "catchup"] {
        let (got, rec) = execute(
            command,
            settings(1),
            Default::default(),
            None,
            true,
            vec![
                ok(auth()),
                Ok(Reply {
                    status: 429,
                    retry_after: Some(0),
                    body: String::new(),
                    ..Default::default()
                }),
                ok(json!({"ok":true,"messages":[]})),
            ],
        );
        assert_eq!(got.result.as_ref().unwrap(), "[]\n");
        assert_eq!(rec.requests().len(), 3);
        assert_eq!(rec.requests()[1], rec.requests()[2]);
        assert_eq!(
            fact(&got, "slack.cli.history.messages").outcome(),
            Outcome::Succeeded
        );
        assert_eq!(facts(&got, "slack.cli.history.errors"), 0);
    }
}
