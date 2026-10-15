use super::*;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::client::recorded::block;
use crate::client::{CheckEvidence, Method, Req, Resp, Sent, Transport};
use crate::{JiraPlugin, demo, model::Me, settings};
use radar_core::Plugin;
use serde_json::{Value, json};

#[derive(Clone)]
enum Reply {
    Json(u16, Value),
    Body(String),
    Network,
}

struct Recorded {
    replies: Mutex<VecDeque<Reply>>,
    asked: Mutex<Vec<Req>>,
}

impl Recorded {
    fn new(replies: Vec<Reply>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.into()),
            asked: Mutex::default(),
        })
    }
    fn body(body: Value) -> Arc<Self> {
        Self::new(vec![Reply::Json(200, body)])
    }
    fn requests(&self) -> Vec<Req> {
        self.asked.lock().unwrap().clone()
    }
}
impl Transport for Recorded {
    fn send(&self, req: &Req) -> Sent<'_> {
        self.asked.lock().unwrap().push(req.clone());
        let reply = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("recorded reply");
        Box::pin(async move {
            match reply {
                Reply::Json(status, body) => Ok(Resp {
                    status,
                    retry_after: None,
                    rate: Default::default(),
                    body: body.to_string(),
                }),
                Reply::Body(body) => Ok(Resp {
                    status: 200,
                    retry_after: None,
                    rate: Default::default(),
                    body,
                }),
                Reply::Network => Err("SECRET_NETWORK_ERROR".into()),
            }
        })
    }
}

fn site() -> Site {
    let mut site = demo::sample().cfg.sites.remove(0);
    site.name = "SECRET_ALIAS".into();
    site.url =
        "https://SECRET_USER:SECRET_PASSWORD@jira.example/?token=SECRET_TOKEN#SECRET_FRAGMENT"
            .into();
    site.email = "SECRET_EMAIL".into();
    site.token = "SECRET_TOKEN".into();
    site
}
fn read(kind: Kind, transport: Arc<Recorded>) -> Result<CheckRead<String>, CheckError> {
    block(read_async(kind, transport))
}
async fn read_async(kind: Kind, transport: Arc<Recorded>) -> Result<CheckRead<String>, CheckError> {
    let client = Ok(Client::over(transport));
    match kind {
        Kind::Site => read_site(client).await,
        Kind::Project => read_project(client, "ABC").await,
    }
}
fn good(kind: Kind) -> Value {
    match kind {
        Kind::Site => {
            json!({"accountId": "SECRET_ACCOUNT", "displayName": "SECRET_NAME", "emailAddress": "SECRET_EMAIL", "body": "SECRET_BODY", "scopes": ["SECRET_SCOPE"]})
        }
        Kind::Project => {
            json!({"name": "SECRET_NAME", "id": "SECRET_REMOTE_ID", "description": "SECRET_BODY"})
        }
    }
}
fn reported(effects: &[Effect]) -> Outcome {
    let Effect::Report(outcome) = effects[0] else {
        panic!("original outcome first")
    };
    outcome
}
fn observations(effects: &[Effect]) -> Vec<(&str, Option<u32>, Outcome, Option<Reason>)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Observe { action, outcome } => Some((
                action.kind.as_str(),
                action.metadata.count,
                *outcome,
                action.metadata.reason,
            )),
            _ => None,
        })
        .collect()
}
fn notify(effects: &[Effect]) -> Vec<&str> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Notify(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}
fn request(kind: Kind, p: &mut JiraPlugin, i: usize) -> Vec<Effect> {
    match kind {
        Kind::Site => settings::check_site(p, i),
        Kind::Project => settings::check_project(p, i, "ABC"),
    }
}
fn receive(
    p: &mut JiraPlugin,
    target: crate::settings_checks::Target,
    request: crate::settings_checks::Request,
    scope: &ActionDescriptor,
    result: Result<CheckRead<String>, CheckError>,
) -> Vec<Effect> {
    p.on_msg(Box::new(JiraMsg::Checked {
        target,
        request,
        action: scope.clone(),
        result,
    }))
}

#[test]
fn site_fields_preserve_permissive_parser_and_exact_request_with_typed_gaps() {
    for (body, incomplete, defaulted) in [
        (good(Kind::Site), 0, 0),
        (Value::Null, 2, 2),
        (json!({}), 2, 2),
        (json!({"accountId": null, "displayName": false}), 2, 2),
        (json!({"accountId": [], "displayName": {}}), 2, 2),
        (json!({"accountId": "", "displayName": " \n\t"}), 2, 0),
        (json!({"accountId": "SECRET_ACCOUNT"}), 1, 1),
        (json!({"displayName": "SECRET_NAME"}), 1, 1),
        (
            json!({"accountId": " SECRET_ACCOUNT ", "displayName": " SECRET_NAME "}),
            0,
            0,
        ),
    ] {
        let plain = Recorded::body(body.clone());
        let expected = block(Client::over(plain.clone()).myself()).unwrap();
        let legacy = Me::from_json(&body);
        assert_eq!(expected, legacy);
        let recorded = Recorded::body(body);
        let got = block(Client::over(recorded.clone()).myself_observed()).unwrap();
        assert_eq!(got.value, legacy);
        assert_eq!(
            got.evidence,
            CheckEvidence {
                incomplete_fields: incomplete,
                defaulted_fields: defaulted
            }
        );
        assert_eq!(recorded.requests(), plain.requests());
        assert_eq!(
            recorded.requests(),
            [Req {
                method: Method::Get,
                path: "/rest/api/3/myself".into(),
                body: None
            }]
        );
        let display = CheckRead {
            value: got.value.name,
            evidence: got.evidence,
        };
        let scope = action(Kind::Site, Some(&site()), "");
        let fx = observed(Kind::Site, &scope, &Ok(display));
        assert_eq!(
            reported(&fx),
            if incomplete == 0 {
                Outcome::Succeeded
            } else {
                Outcome::Partial
            }
        );
        let rows = observations(&fx);
        assert_eq!(rows[0].1, Some(1));
        assert_eq!(
            rows.iter()
                .find(|row| row.0.ends_with("fields.incomplete"))
                .map(|row| row.1),
            (incomplete != 0).then_some(Some(incomplete))
        );
        assert_eq!(
            rows.iter()
                .find(|row| row.0.ends_with("fields.defaulted"))
                .map(|row| row.1),
            (defaulted != 0).then_some(Some(defaulted))
        );
    }
}

#[test]
fn project_default_is_not_inferred_from_equal_names_and_blanks_are_retained() {
    for (body, name, incomplete, defaulted) in [
        (good(Kind::Project), "SECRET_NAME", 0, 0),
        (json!({"name": "ABC"}), "ABC", 0, 0),
        (Value::Null, "ABC", 1, 1),
        (json!({}), "ABC", 1, 1),
        (json!({"name": null}), "ABC", 1, 1),
        (json!({"name": false}), "ABC", 1, 1),
        (json!({"name": []}), "ABC", 1, 1),
        (json!({"name": {}}), "ABC", 1, 1),
        (json!({"name": ""}), "", 1, 0),
        (json!({"name": " \n\t"}), " \n\t", 1, 0),
        (json!({"name": " SECRET_NAME "}), " SECRET_NAME ", 0, 0),
    ] {
        let plain = Recorded::body(body.clone());
        let expected = block(Client::over(plain.clone()).project("ABC")).unwrap();
        assert_eq!(expected, name);
        let recorded = Recorded::body(body);
        let got = read(Kind::Project, recorded.clone()).unwrap();
        assert_eq!(got.value, format!("found {name}"));
        assert_eq!(
            got.evidence,
            CheckEvidence {
                incomplete_fields: incomplete,
                defaulted_fields: defaulted
            }
        );
        assert_eq!(recorded.requests(), plain.requests());
        assert_eq!(
            recorded.requests(),
            [Req {
                method: Method::Get,
                path: "/rest/api/3/project/ABC".into(),
                body: None
            }]
        );
        let fx = observed(
            Kind::Project,
            &action(Kind::Project, Some(&site()), "ABC"),
            &Ok(got),
        );
        assert_eq!(
            reported(&fx),
            if incomplete == 0 {
                Outcome::Succeeded
            } else {
                Outcome::Partial
            }
        );
        assert_eq!(
            observations(&fx)
                .iter()
                .any(|row| row.0.ends_with("fields.defaulted")),
            defaulted != 0
        );
    }
}

#[test]
fn empty_successful_bodies_keep_default_display_values_and_report_partial() {
    for kind in [Kind::Site, Kind::Project] {
        let recorded = Recorded::new(vec![Reply::Body(String::new())]);
        let got = read(kind, recorded.clone()).unwrap();
        assert_eq!(
            got.value,
            match kind {
                Kind::Site => "",
                Kind::Project => "found ABC",
            }
        );
        assert_eq!(recorded.requests().len(), 1);
        let expected = match kind {
            Kind::Site => 2,
            Kind::Project => 1,
        };
        assert_eq!(
            got.evidence,
            CheckEvidence {
                incomplete_fields: expected,
                defaulted_fields: expected
            }
        );
        assert_eq!(
            reported(&observed(
                kind,
                &action(kind, Some(&site()), "ABC"),
                &Ok(got)
            )),
            Outcome::Partial
        );
    }
}

#[test]
fn read_failures_and_retries_keep_wrapper_errors_requests_and_one_logical_error() {
    for kind in [Kind::Site, Kind::Project] {
        for (replies, attempts) in [
            (vec![Reply::Network], 1),
            (vec![Reply::Body("SECRET_INVALID_JSON".into())], 1),
            (
                vec![Reply::Json(403, json!({"errorMessages": ["SECRET_ERROR"]}))],
                1,
            ),
            (
                vec![
                    Reply::Json(503, json!({})),
                    Reply::Json(429, json!({})),
                    Reply::Json(503, json!({})),
                ],
                3,
            ),
        ] {
            let plain = Recorded::new(replies.clone());
            let client = Client::over(plain.clone());
            let expected = match kind {
                Kind::Site => block(client.myself()).unwrap_err(),
                Kind::Project => block(client.project("ABC")).unwrap_err(),
            };
            let recorded = Recorded::new(replies);
            let result = read(kind, recorded.clone());
            assert!(matches!(&result, Err(CheckError::Read(_))));
            assert_eq!(
                format!("{:#}", result.as_ref().unwrap_err().error()),
                format!("{expected:#}")
            );
            assert_eq!(recorded.requests(), plain.requests());
            assert_eq!(recorded.requests().len(), attempts);
            assert!(
                recorded
                    .requests()
                    .windows(2)
                    .all(|pair| pair[0] == pair[1])
            );
            let fx = observed(kind, &action(kind, Some(&site()), "ABC"), &result);
            assert_eq!(reported(&fx), Outcome::Failed);
            assert_eq!(
                observations(&fx),
                [(
                    kind.codes()[4],
                    Some(1),
                    Outcome::Failed,
                    Some(Reason::Unspecified)
                )]
            );
        }
    }
}

#[test]
fn successful_retries_report_only_returned_evidence_without_attempt_counts() {
    for kind in [Kind::Site, Kind::Project] {
        let replies = vec![
            Reply::Json(429, json!({})),
            Reply::Json(502, json!({})),
            Reply::Json(200, good(kind)),
        ];
        let plain = Recorded::new(replies.clone());
        let client = Client::over(plain.clone());
        match kind {
            Kind::Site => {
                block(client.myself()).unwrap();
            }
            Kind::Project => {
                block(client.project("ABC")).unwrap();
            }
        }
        let recorded = Recorded::new(replies);
        let result = read(kind, recorded.clone());
        assert_eq!(recorded.requests(), plain.requests());
        assert_eq!(recorded.requests().len(), 3);
        let fx = observed(kind, &action(kind, Some(&site()), "ABC"), &result);
        assert_eq!(reported(&fx), Outcome::Succeeded);
        assert_eq!(
            observations(&fx),
            [(kind.codes()[1], Some(1), Outcome::Succeeded, None)]
        );
    }
}

#[test]
fn invalid_project_and_setup_failures_are_typed_local_refusals_with_zero_http() {
    let recorded = Recorded::new(vec![]);
    let client = Client::over(recorded.clone());
    let invalid = "SECRET invalid / key";
    let original = block(client.project(invalid)).unwrap_err();
    let result = block(read_project(Ok(client), invalid));
    assert!(matches!(&result, Err(CheckError::InvalidProject(_))));
    assert_eq!(
        format!("{:#}", result.as_ref().unwrap_err().error()),
        format!("{original:#}")
    );
    assert!(recorded.requests().is_empty());
    let fx = observed(
        Kind::Project,
        &action(Kind::Project, Some(&site()), invalid),
        &result,
    );
    assert_eq!(reported(&fx), Outcome::Blocked);
    assert_eq!(
        observations(&fx),
        [(
            Kind::Project.codes()[5],
            Some(1),
            Outcome::Blocked,
            Some(Reason::InvalidInput)
        )]
    );
    // Identical error text cannot turn setup/read failures into key classification.
    let setup = block(read_project(Err(anyhow::anyhow!("{original:#}")), invalid));
    assert!(matches!(&setup, Err(CheckError::Setup(_))));
    let fx = observed(
        Kind::Project,
        &action(Kind::Project, Some(&site()), invalid),
        &setup,
    );
    assert_eq!(observations(&fx)[0].3, Some(Reason::Unavailable));
    assert!(recorded.requests().is_empty());
    let same_text = Err(CheckError::Read(anyhow::anyhow!("{original:#}")));
    assert_eq!(
        reported(&observed(
            Kind::Project,
            &action(Kind::Project, Some(&site()), invalid),
            &same_text
        )),
        Outcome::Failed
    );
}

#[test]
fn production_dispatch_captures_scope_and_defers_setup_before_key_validation() {
    for kind in [Kind::Site, Kind::Project] {
        let mut p = demo::sample();
        p.cfg.sites = vec![site()];
        p.cfg.sites[0].email.clear();
        let fx = match kind {
            Kind::Site => settings::check_site(&mut p, 0),
            Kind::Project => settings::check_project(&mut p, 0, "SECRET invalid key"),
        };
        let [Effect::SpawnAction { action: scope, .. }] = &fx[..] else {
            panic!("independent settings task")
        };
        let scope = scope.clone();
        assert_eq!(scope.kind.as_str(), kind.codes()[0]);
        p.cfg.sites[0].url = "https://replacement.example".into();
        p.cfg.sites[0].email = "changed@example.com".into();
        let Effect::SpawnAction { task, .. } = fx.into_iter().next().unwrap() else {
            unreachable!()
        };
        let msg = block(task);
        let JiraMsg::Checked {
            action: captured,
            result,
            ..
        } = msg.downcast_ref::<JiraMsg>().unwrap()
        else {
            panic!("settings reply")
        };
        assert_eq!(captured.subjects, scope.subjects);
        assert_eq!(captured.metadata, scope.metadata);
        assert!(matches!(result, Err(CheckError::Setup(_))));
        assert!(
            result
                .as_ref()
                .unwrap_err()
                .error()
                .to_string()
                .contains("no email set")
        );
        let fx = p.on_msg(msg);
        assert_eq!(reported(&fx), Outcome::Blocked);
        assert_eq!(observations(&fx)[0].3, Some(Reason::Unavailable));
        assert!(notify(&fx).is_empty());
        assert!(
            observations(&fx)
                .last()
                .unwrap()
                .0
                .ends_with("application.skipped")
        );
    }
}

#[test]
fn captured_subjects_only_name_safe_configured_server_and_requested_project() {
    for kind in [Kind::Site, Kind::Project] {
        let mut requested = site();
        let scope = action(kind, Some(&requested), "ABC");
        requested.url = "https://replacement.example".into();
        assert_eq!(scope.subjects.len(), 1);
        let subject = &scope.subjects[0];
        assert_eq!(subject.key.instance, "https://jira.example/");
        assert_eq!(
            (subject.key.kind.as_str(), subject.key.id.as_str()),
            match kind {
                Kind::Site => ("site", "site"),
                Kind::Project => ("project", "ABC"),
            }
        );
        assert_eq!(subject.role, SubjectRole::Primary);
        assert!(subject.url.is_none());
        assert!(!format!("{scope:?}").contains("SECRET"));
        let changed = action(kind, Some(&requested), "ABC");
        assert_ne!(scope.subjects, changed.subjects);
        for url in ["https://[", "file:///SECRET", ""] {
            requested.url = url.into();
            let scope = action(kind, Some(&requested), "ABC");
            assert!(scope.subjects.is_empty());
            assert!(scope.metadata.redacted);
            assert_eq!(scope.metadata.omitted_subjects, 1);
            assert!(!format!("{scope:?}").contains("SECRET"));
        }
    }
    let invalid = action(Kind::Project, Some(&site()), "SECRET invalid key");
    assert!(invalid.subjects.is_empty());
    assert!(invalid.metadata.redacted);
    assert_eq!(invalid.metadata.omitted_subjects, 1);
}

#[test]
fn missing_config_rows_add_only_independent_blocked_history_without_ui_or_state_change() {
    for kind in [Kind::Site, Kind::Project] {
        let mut p = demo::sample();
        let before_checks = format!("{:?}", p.checks);
        let before_cache = p.snapshot();
        let fx = request(kind, &mut p, usize::MAX);
        let [
            Effect::Observe {
                action,
                outcome: Outcome::Blocked,
            },
        ] = &fx[..]
        else {
            panic!("observation only")
        };
        assert_eq!(action.kind.as_str(), kind.codes()[6]);
        assert_eq!(action.metadata.reason, Some(Reason::Unavailable));
        assert!(action.subjects.is_empty());
        assert!(action.metadata.redacted);
        assert_eq!(format!("{:?}", p.checks), before_checks);
        assert_eq!(p.snapshot(), before_cache);
    }
}

#[test]
fn original_outcomes_precede_unchanged_application_notifications_and_retained_account_id() {
    for kind in [Kind::Site, Kind::Project] {
        for case in ["complete", "defaulted", "blank", "error", "setup"] {
            for cached in [false, true] {
                let mut p = demo::sample();
                let mut legacy = demo::sample();
                for plugin in [&mut p, &mut legacy] {
                    plugin.data.get_mut("acme").unwrap().me = cached.then(|| Me {
                        account_id: "SECRET_CACHED_ACCOUNT".into(),
                        name: "SECRET_OLD_NAME".into(),
                    });
                }
                let result = match case {
                    "complete" => read(kind, Recorded::body(good(kind))),
                    "defaulted" => read(kind, Recorded::body(Value::Null)),
                    "blank" => read(
                        kind,
                        Recorded::body(json!({"accountId": "", "displayName": "", "name": ""})),
                    ),
                    "error" => Err(CheckError::Read(anyhow::anyhow!("SECRET_ERROR"))),
                    _ => Err(CheckError::Setup(anyhow::anyhow!("SECRET_SETUP"))),
                };
                let target = match kind {
                    Kind::Site => crate::settings_checks::Target::Site("acme".into()),
                    Kind::Project => crate::settings_checks::Target::project("acme", "ABC"),
                };
                drop(request(kind, &mut p, 0));
                let owner = p.check_owners.pending[&target];
                let legacy_result = result
                    .as_ref()
                    .map(|got| got.value.clone())
                    .map_err(|e| format!("{:#}", e.error()));
                let expected = settings::on_checked(&mut legacy, target.clone(), legacy_result);
                let fx = receive(
                    &mut p,
                    target,
                    owner,
                    &action(kind, Some(&site()), "ABC"),
                    result,
                );
                assert_eq!(
                    reported(&fx),
                    match case {
                        "complete" => Outcome::Succeeded,
                        "defaulted" | "blank" => Outcome::Partial,
                        "error" => Outcome::Failed,
                        _ => Outcome::Blocked,
                    }
                );
                assert_eq!(notify(&fx), notify(&expected));
                assert!(matches!(fx.last().unwrap(), Effect::Notify(_)));
                assert!(fx.iter().all(|effect| matches!(
                    effect,
                    Effect::Report(_) | Effect::Observe { .. } | Effect::Notify(_)
                )));
                assert_eq!(format!("{:?}", p.checks), format!("{:?}", legacy.checks));
                assert_eq!(p.snapshot(), legacy.snapshot());
                if matches!(kind, Kind::Site) && matches!(case, "complete" | "defaulted" | "blank")
                {
                    assert_eq!(
                        p.data["acme"].me.as_ref().unwrap().account_id,
                        if cached { "SECRET_CACHED_ACCOUNT" } else { "" }
                    );
                }
                let cache = p.snapshot().unwrap().to_string();
                assert!(
                    !cache.contains("incomplete_fields")
                        && !cache.contains("defaulted_fields")
                        && !cache.contains("jira.settings.check")
                );
            }
        }
    }
}

#[test]
fn removed_config_preserves_original_history_but_blocks_late_and_duplicate_application() {
    let mut p = demo::sample();
    drop(request(Kind::Site, &mut p, 0));
    let target = crate::settings_checks::Target::Site("acme".into());
    let owner = p.check_owners.pending[&target];
    let scope = action(Kind::Site, p.cfg.site("acme"), "");
    p.configure(None).unwrap();
    p.data.clear();
    for name in ["SECRET_NEWER_NAME", "SECRET_LATE_NAME"] {
        let fx = receive(
            &mut p,
            target.clone(),
            owner,
            &scope,
            Ok(CheckRead {
                value: name.into(),
                evidence: CheckEvidence::default(),
            }),
        );
        assert_eq!(reported(&fx), Outcome::Succeeded);
        assert!(p.data.is_empty());
        assert!(p.checks.is_empty());
        assert!(notify(&fx).is_empty());
        assert_eq!(
            observations(&fx).last().unwrap().0,
            "jira.settings.check.site.application.skipped"
        );
    }
}

mod host_tests;
