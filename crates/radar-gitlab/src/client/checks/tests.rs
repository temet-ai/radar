use super::*;
use crate::client::recorded::{Replay, block};
use crate::client::{Method, Req, Resp};

fn read(body: Value, path: Option<&str>) -> CheckRead {
    let client = Client::over(Replay::new(vec![(200, body)]));
    block(async {
        match path {
            Some(path) => client.check_path_observed(path).await,
            None => client.check_user_observed().await,
        }
    })
    .unwrap()
}

#[test]
fn public_wrappers_keep_exact_queries_variables_and_display_sentences() {
    for (path, body, expected, projects, mrs) in [
        (
            None,
            json!({"currentUser":{"username":"alice", "name":"Alice Example"}}),
            "signed in as alice (Alice Example)",
            None,
            None,
        ),
        (
            Some("acme/team"),
            json!({"group":{"fullName":"Acme / Team", "projects":{"count":12}, "mergeRequests":{"count":"5"}}, "project":null}),
            "found the group Acme / Team: 12 repos, 5 open merge requests",
            Some(12),
            Some(5),
        ),
        (
            Some("acme/team/api"),
            json!({"group":null, "project":{"nameWithNamespace":"Acme / API", "mergeRequests":{"count":0}}}),
            "found the repo Acme / API: 0 open merge requests",
            None,
            Some(0),
        ),
    ] {
        let plain = Replay::new(vec![(200, json!({"data":body}))]);
        let recorded = Replay::new(vec![(200, json!({"data":body}))]);
        let client = Client::over(plain.clone());
        let observed = Client::over(recorded.clone());
        let (value, got) = block(async {
            match path {
                Some(path) => (
                    client.check_path(path).await.unwrap(),
                    observed.check_path_observed(path).await.unwrap(),
                ),
                None => (
                    client.check_user().await.unwrap(),
                    observed.check_user_observed().await.unwrap(),
                ),
            }
        });
        assert_eq!(value, expected);
        assert_eq!(got.value, expected);
        assert_eq!(
            got.evidence,
            CheckEvidence {
                reported_projects: projects,
                reported_open_merge_requests: mrs,
                ..Default::default()
            }
        );
        assert_eq!(plain.asked(), recorded.asked());
        assert_eq!(
            recorded.asked(),
            [Req::new(
                Method::Post,
                "/api/graphql",
                Some(json!({
                    "query": if path.is_some() { PATH_QUERY } else { "query { currentUser { username name } }" },
                    "variables": path.map_or_else(|| json!({}), |path| json!({"path":path}))
                }))
            )]
        );
    }
}

#[test]
fn nonnull_auth_values_still_pass_with_bounded_default_evidence() {
    for user in [json!(false), json!(0), json!("user"), json!([]), json!({})] {
        let got = read(json!({"data":{"currentUser":user}}), None);
        assert_eq!(got.value, "signed in as  ()");
        assert_eq!(
            got.evidence,
            CheckEvidence {
                incomplete_objects: u32::from(!user.is_object()),
                incomplete_fields: 2,
                defaulted_fields: 2,
                ..Default::default()
            }
        );
    }
    for (user, expected, incomplete) in [
        (json!({"username":"", "name":""}), "signed in as  ()", 0),
        (
            json!({"username":" ", "name":"\t"}),
            "signed in as   (\t)",
            0,
        ),
        (
            json!({"username":"alice", "name":null}),
            "signed in as alice ()",
            1,
        ),
        (
            json!({"username":5, "name":"Alice"}),
            "signed in as  (Alice)",
            1,
        ),
    ] {
        let got = read(json!({"data":{"currentUser":user}}), None);
        assert_eq!(got.value, expected);
        assert_eq!(got.evidence.incomplete_fields, incomplete);
        assert_eq!(got.evidence.defaulted_fields, incomplete);
    }
}

#[test]
fn malformed_selected_path_objects_preserve_group_precedence_and_defaults() {
    for branch in ["group", "project"] {
        for selected in [json!(false), json!(0), json!("path"), json!([]), json!({})] {
            let got = read(json!({"data":{branch:selected}}), Some("raw-path"));
            let group = branch == "group";
            assert_eq!(
                got.value,
                if group {
                    "found the group : 0 repos, 0 open merge requests"
                } else {
                    "found the repo : 0 open merge requests"
                }
            );
            assert_eq!(
                got.evidence,
                CheckEvidence {
                    incomplete_objects: u32::from(!selected.is_object())
                        + if group { 2 } else { 1 },
                    incomplete_fields: if group { 3 } else { 2 },
                    defaulted_fields: if group { 3 } else { 2 },
                    ..Default::default()
                }
            );
        }
    }
}

#[test]
fn only_selected_fields_are_read_and_dual_nonnull_paths_are_ambiguous() {
    for project in [
        Value::Null,
        json!(false),
        json!({"nameWithNamespace":"SECRET_OTHER", "mergeRequests":{"count":999}}),
    ] {
        let got = read(
            json!({"data":{"group":{"fullName":"G", "projects":{"count":0}, "mergeRequests":{"count":4}}, "project":project}}),
            Some("acme"),
        );
        assert_eq!(
            got.value,
            "found the group G: 0 repos, 4 open merge requests"
        );
        assert_eq!(
            got.evidence,
            CheckEvidence {
                ambiguous_path: !project.is_null(),
                reported_projects: Some(0),
                reported_open_merge_requests: Some(4),
                ..Default::default()
            }
        );
    }
    let got = read(
        json!({"data":{"group":{"fullName":null, "projects":false, "mergeRequests":{"count":4}}, "project":{"nameWithNamespace":"not selected"}}}),
        Some("acme"),
    );
    assert_eq!(got.evidence.incomplete_objects, 1);
    assert_eq!(got.evidence.incomplete_fields, 2);
    assert_eq!(got.evidence.defaulted_fields, 2);
    assert_eq!(got.evidence.reported_open_merge_requests, Some(4));
}

#[test]
fn numbers_preserve_legacy_casts_but_never_report_wrapped_or_defaulted_counts() {
    for (number, display, reported, defaulted, lossy) in [
        (json!(0), 0, Some(0), 0, 0),
        (json!(u32::MAX), u32::MAX, Some(u32::MAX), 0, 0),
        (json!("42"), 42, Some(42), 0, 0),
        (json!("+42"), 42, Some(42), 0, 0),
        (json!(u64::from(u32::MAX) + 2), 1, None, 0, 1),
        (json!(u64::MAX), u32::MAX, None, 0, 1),
        (json!("4294967296"), 0, None, 0, 1),
        (json!("18446744073709551616"), 0, None, 1, 0),
        (json!(-1), 0, None, 1, 0),
        (json!(1.0), 0, None, 1, 0),
        (json!(true), 0, None, 1, 0),
        (Value::Null, 0, None, 1, 0),
        (json!(" 42"), 0, None, 1, 0),
        (json!(""), 0, None, 1, 0),
    ] {
        let got = read(
            json!({"data":{"project":{"nameWithNamespace":"P", "mergeRequests":{"count":number}}}}),
            Some("acme/api"),
        );
        assert_eq!(
            got.value,
            format!("found the repo P: {display} open merge requests"),
            "{number}"
        );
        assert_eq!(
            got.evidence,
            CheckEvidence {
                incomplete_fields: defaulted + lossy,
                defaulted_fields: defaulted,
                lossy_numbers: lossy,
                reported_open_merge_requests: reported,
                ..Default::default()
            },
            "{number}"
        );
    }
}

#[test]
fn missing_null_and_scalar_roots_keep_existing_errors() {
    for data in [
        Value::Null,
        json!(false),
        json!(0),
        json!("root"),
        json!([]),
        json!({}),
        json!({"currentUser":null,"group":null,"project":null}),
    ] {
        for path in [None, Some("raw requested path")] {
            let plain = Replay::new(vec![(200, json!({"data":data}))]);
            let recorded = Replay::new(vec![(200, json!({"data":data}))]);
            let client = Client::over(plain.clone());
            let observed = Client::over(recorded.clone());
            let (value, got) = block(async {
                match path {
                    Some(path) => (
                        client.check_path(path).await.unwrap_err(),
                        observed.check_path_observed(path).await.unwrap_err(),
                    ),
                    None => (
                        client.check_user().await.unwrap_err(),
                        observed.check_user_observed().await.unwrap_err(),
                    ),
                }
            });
            assert_eq!(format!("{value:#}"), format!("{got:#}"));
            assert_eq!(
                value.to_string(),
                if path.is_some() {
                    "no group or repo raw requested path, or this token cannot see it"
                } else {
                    "GitLab answered, but not as anyone: the token is missing or has no read_api scope"
                }
            );
            assert_eq!(plain.asked(), recorded.asked());
            assert_eq!(plain.asked().len(), 1);
        }
    }
}

#[test]
fn graphql_status_error_and_invalid_json_behavior_is_unchanged() {
    for (status, body, succeeds) in [
        (403, json!({"data":{"currentUser":{"username":"a","name":"A"}}}).to_string(), true),
        (500, json!({"data":{"currentUser":false}}).to_string(), true),
        (401, json!({"data":{"currentUser":{"username":"a","name":"A"}}}).to_string(), false),
        (200, json!({"errors":[{"message":"SECRET_ERROR"}], "data":{"currentUser":{"username":"a","name":"A"}}}).to_string(), false),
        (200, "SECRET_INVALID_JSON".into(), false),
        (200, String::new(), false),
    ] {
        let replies = vec![Resp {status, body: body.into_bytes(), ..Default::default()}];
        let plain = Replay::raw(replies.clone());
        let recorded = Replay::raw(replies);
        let value = block(Client::over(plain.clone()).check_user()).map_err(|e| format!("{e:#}"));
        let got = block(Client::over(recorded.clone()).check_user_observed()).map(|got| got.value).map_err(|e| format!("{e:#}"));
        assert_eq!(value, got);
        assert_eq!(got.is_ok(), succeeds);
        assert_eq!(plain.asked(), recorded.asked());
        assert_eq!(plain.asked().len(), 1);
    }
}

#[test]
fn raw_paths_are_requested_without_new_validation_or_normalization() {
    for path in ["", "  ", " acme/team ", "acme/../team", "group:acme/team"] {
        let recorded = Replay::new(vec![(
            200,
            json!({"data":{"project":{"nameWithNamespace":"P", "mergeRequests":{"count":1}}}}),
        )]);
        let got = block(Client::over(recorded.clone()).check_path_observed(path)).unwrap();
        assert_eq!(got.value, "found the repo P: 1 open merge requests");
        assert_eq!(
            recorded.asked()[0].body.as_ref().unwrap()["variables"],
            json!({"path":path})
        );
    }
}

#[test]
fn empty_and_whitespace_path_names_are_type_valid_without_claiming_identity() {
    for name in ["", " ", "\t\n"] {
        for group in [true, false] {
            let data = if group {
                json!({"group":{"fullName":name,"projects":{"count":0},"mergeRequests":{"count":0}}})
            } else {
                json!({"project":{"nameWithNamespace":name,"mergeRequests":{"count":0}}})
            };
            let got = read(json!({"data":data}), Some("acme"));
            assert!(!got.evidence.incomplete());
            assert_eq!(
                got.value,
                if group {
                    format!("found the group {name}: 0 repos, 0 open merge requests")
                } else {
                    format!("found the repo {name}: 0 open merge requests")
                }
            );
        }
    }
}
