//! Behavioral controls also runnable unchanged against the pre-isolation code.
//! All responses are recorded; no provider client is exercised live.
use crate::{
    SlackPlugin, Via,
    client::{Client, Recorded, Reply},
    demo,
};
use radar_core::{Plugin, plugin::Effect};
use serde_json::{Value, json};

fn ok(body: Value) -> anyhow::Result<Reply> {
    Ok(Reply {
        status: 200,
        body: body.to_string(),
        ..Default::default()
    })
}
fn run(task: radar_core::plugin::Task) -> radar_core::plugin::Msg {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(task)
}

#[test]
fn boundary_control_legacy_identity_is_never_active_on_restore() {
    let mut p = SlackPlugin::new();
    p.configure(demo::sample().config().as_ref()).unwrap();
    p.restore(json!({"data":demo::sample().data}));
    assert!(p.data.me.is_none());
    assert!(p.data.users.is_empty());
    assert!(p.workspace().is_none());
}

#[test]
fn boundary_control_account_change_does_not_reuse_overlapping_id_data() {
    let mut p = demo::sample();
    p.data.marks.insert(demo::TEAM.into(), "123.001".into());
    let mut other = p.data.me.clone().unwrap();
    other.user_id = "UOTHER".into();
    p.replace_token("env:DIFFERENT_REFERENCE".into());
    p.install_me(other);
    assert!(p.data.users.is_empty());
    assert!(p.data.groups.is_empty());
    assert!(p.data.marks.is_empty());
}

#[test]
fn boundary_control_unknown_envelope_survives_roundtrip() {
    let raw = json!({"version":99,"future":{"draft":"private authored words","marks":[1,2]}});
    let mut p = SlackPlugin::new();
    p.restore(raw.clone());
    assert_eq!(p.snapshot().unwrap()["quarantine"][0], raw);
}

#[test]
fn boundary_control_pre_auth_page_cannot_dispatch() {
    let mut p = SlackPlugin::new();
    p.cfg = demo::settings();
    let recording = Recorded::new(vec![]);
    p.via = Via(Some(recording.clone()));
    let fx = crate::page::fetch_thread(&mut p, demo::TEAM, "123.001");
    assert!(
        !fx.iter()
            .any(|e| matches!(e, Effect::Spawn(_) | Effect::SpawnAction { .. }))
    );
    assert!(recording.requests().is_empty());
}

#[test]
fn boundary_control_incomplete_auth_cannot_dispatch_children() {
    let mut p = SlackPlugin::new();
    p.cfg = demo::settings();
    let recording = Recorded::new(vec![
        ok(json!({"ok":true})),
        ok(json!({"ok":true,"user":{"tz_offset":0}})),
    ]);
    p.via = Via(Some(recording.clone()));
    let task = p
        .read()
        .into_iter()
        .find_map(|e| match e {
            Effect::Spawn(t) | Effect::SpawnAction { task: t, .. } => Some(t),
            _ => None,
        })
        .unwrap();
    let fx = p.on_msg(run(task));
    assert!(p.data.me.is_none());
    assert!(
        !fx.iter()
            .any(|e| matches!(e, Effect::Spawn(_) | Effect::SpawnAction { .. }))
    );
    assert_eq!(recording.requests().len(), 1);
}

#[test]
fn boundary_control_cli_authenticates_despite_cached_identity_and_marks() {
    for catchup in [false, true] {
        let mut p = demo::sample();
        p.cfg.channels.truncate(1);
        p.data.marks.insert(demo::TEAM.into(), "123.001".into());
        let recording = Recorded::new(vec![
            ok(
                json!({"ok":true,"team_id":"TOTHER","user_id":"UOTHER","url":"https://other.slack.com/"}),
            ),
            ok(
                json!({"ok":true,"messages":[{"ts":"500.001","username":"a bot","text":"new message"}]}),
            ),
        ]);
        let client = Client::new(recording.clone());
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = if catchup {
            rt.block_on(crate::catchup(
                &client,
                p.cfg.clone(),
                p.data.clone(),
                None,
                true,
                demo::NOW,
            ))
        } else {
            rt.block_on(crate::list(
                &client,
                p.cfg.clone(),
                p.data.clone(),
                None,
                None,
                true,
                demo::NOW,
            ))
        }
        .unwrap();
        let requests = recording.requests();
        assert_eq!(requests[0].method, "auth.test");
        let rows: Value = serde_json::from_str(&result).unwrap();
        assert!(
            rows[0]["url"]
                .as_str()
                .unwrap()
                .starts_with("https://other.slack.com/")
        );
        assert_eq!(
            requests[1]
                .params
                .iter()
                .find(|(key, _)| key == "oldest")
                .unwrap()
                .1,
            format!("{}.000000", demo::NOW - p.cfg.days as i64 * 86400)
        );
    }
}
