//! Regression coverage for delayed, already-completed Slack reads. All provider
//! replies are synthetic or recorded; task stamps come from the actual plugin.

use radar_core::Plugin;
use radar_core::journal::{Outcome, Reason};
use radar_core::plugin::{Effect, Msg};
use serde_json::{Value, json};

use crate::client::{
    ChannelInfo, Found, GroupsRead, ListEvidence, Message, MessagesRead, Reaction, Recorded, Reply,
    UserGroup,
};
use crate::demo::{self, ALICE, BOB, NOW, TEAM};
use crate::read::Read;
use crate::sync_history::{
    self, ChannelRead, FollowedBatch, FollowedResponse, FollowedTarget, ForYouEvidence,
    GroupsEvidence, NamesEvidence, SearchKind, SearchList, SearchObservation, ThreadsRead,
};
use crate::{Check, Checked, Fetched, SlackMsg, SlackPlugin, SyncErrorKey, Via, thread_key};

const OLD_TEAM: &str = "T000ACME1";
const NEXT_TEAM: &str = "T000NEXT1";
const ROOT: &str = "1791270000.000100";

fn sample() -> SlackPlugin {
    let mut p = demo::sample();
    // Even accidentally spawned work can only reach this empty recording.
    p.via = Via(Some(Recorded::new(vec![])));
    p.data.asked.push(crate::model::Asked {
        channel: TEAM.into(),
        ts: ROOT.into(),
        replies: 3,
        ..Default::default()
    });
    p.data.marks.insert(TEAM.into(), demo::ago(500));
    p.data.read_to(TEAM, ROOT, &demo::ago(400));
    p.begin_reply(TEAM, ROOT, Some("kept reply draft"));
    let preview = p.preview_reply();
    let id = preview
        .iter()
        .find_map(|fx| match fx {
            Effect::Menu(m) => Some(m.id.clone()),
            _ => None,
        })
        .unwrap();
    p.answer_reply(&id, "no");
    p.draft = Some(crate::ask::Draft {
        channel: "team".into(),
        items: vec![crate::ask::Item {
            id: "api!263".into(),
            url: demo::MR_263.into(),
            label: "api !263".into(),
            words: "kept review words".into(),
            tickets: vec![],
        }],
        ..Default::default()
    });
    p.begin_reply(TEAM, ROOT, Some("unfinished\nreply"));
    p.pages.push(crate::page::Page::Thread {
        channel: TEAM.into(),
        ts: ROOT.into(),
        tab: 1,
        row: 0,
    });
    p.threads.insert(
        thread_key(TEAM, ROOT),
        Fetched {
            value: Some(vec![demo::msg(ROOT, BOB, "kept page value")]),
            loading: false,
            error: None,
        },
    );
    p
}

/// State the late answer must not touch, including local-only user work.
fn held(p: &SlackPlugin) -> Value {
    let pages: Vec<_> = p
        .threads
        .iter()
        .map(|(key, value)| json!([key, value.value, value.loading, value.error]))
        .collect();
    json!({
        "data": p.data,
        "settings": p.cfg,
        "reading": p.sync.reading,
        "started": p.sync.started,
        "last_ok": p.sync.last_ok,
        "errors": format!("{:?}", p.sync.errors),
        "groups_at": p.sync.groups_at,
        "checks": format!("{:?}", p.checks),
        "thread_values": pages,
        "pages": format!("{:?}", p.pages),
        "drafts": format!("{:?}", p.reply_context.kept),
        "draft": format!("{:?}", p.draft),
        "replying": format!("{:?}", p.replying),
    })
}

fn history(read: &Read) -> Vec<Effect> {
    match read {
        Read::Me { action, result } => sync_history::bootstrap(action, result),
        Read::Channel {
            action,
            result,
            names_evidence,
            ..
        } => sync_history::channel(action, result, *names_evidence),
        Read::Threads {
            action,
            result,
            names_evidence,
            ..
        } => sync_history::threads(action, result, *names_evidence),
        Read::ForYou {
            action,
            result,
            evidence,
            ..
        } => sync_history::for_you(action, evidence, result.is_ok()),
        Read::Followed { action, batch } => sync_history::followed(action, batch),
    }
}

fn assert_skipped(p: &mut SlackPlugin, message: SlackMsg, evidence: Vec<Effect>) {
    let before = held(p);
    let effects = p.on_msg(Box::new(message));
    assert_eq!(held(p), before, "a rejected answer changed current state");
    assert_eq!(effects.len(), evidence.len() + 1, "{effects:?}");
    // Preserve the provider's original outcomes, counts, redactions and subject
    // identities exactly. Only application to current state was blocked.
    assert_eq!(
        format!("{:?}", &effects[..evidence.len()]),
        format!("{evidence:?}")
    );
    assert!(
        matches!(effects.last(), Some(Effect::Observe { action, outcome: Outcome::Blocked })
        if action.kind.as_str() == "slack.read.apply.skipped.stale"
            && action.metadata.reason == Some(Reason::Unspecified)),
        "{effects:?}"
    );
    assert!(
        effects
            .iter()
            .all(|effect| matches!(effect, Effect::Report(_) | Effect::Observe { .. })),
        "{effects:?}"
    );
    let journal = format!("{effects:?}");
    for payload in [
        "OLD_PRIVATE_BODY",
        "OLD_PRIVATE_NAME",
        "OLD_PRIVATE_ERROR",
        "SLACK_NEXT_TOKEN",
    ] {
        assert!(
            !journal.contains(payload),
            "payload entered history: {journal}"
        );
    }
}

fn rotate_token(p: &mut SlackPlugin, through_configure: bool) {
    if through_configure {
        let mut cfg = p.cfg.clone();
        cfg.token = "env:SLACK_NEXT_TOKEN".into();
        p.configure(Some(&toml::Value::try_from(cfg).unwrap()))
            .unwrap();
    } else {
        p.replace_token("env:SLACK_NEXT_TOKEN".into());
    }
}

fn start_fresh_work(p: &mut SlackPlugin) {
    p.now = NOW + 20;
    let effects = p.read();
    assert!(!effects.is_empty());
    assert!(p.sync.reading > 0);
    // A current failure and a prior current success must survive old completions.
    p.sync.last_ok = Some(NOW + 10);
    p.sync
        .errors
        .insert(SyncErrorKey::ForYou, "current search failure".into());
}

fn ok(body: Value) -> anyhow::Result<Reply> {
    Ok(Reply {
        status: 200,
        body: body.to_string(),
        ..Default::default()
    })
}

fn run_only_task(effects: Vec<Effect>) -> Msg {
    let mut tasks = effects.into_iter().filter_map(|effect| match effect {
        Effect::Spawn(task) => Some(task),
        Effect::SpawnAction { task, .. } => Some(task),
        _ => None,
    });
    let task = tasks.next().expect("one read task");
    assert!(tasks.next().is_none(), "only one task was expected");
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(task)
}

#[test]
fn token_reference_changes_preserve_cache_marks_asks_and_unfinished_user_work() {
    for through_configure in [false, true] {
        let mut p = sample();
        let data = p.data.clone();
        let draft = format!("{:?}", p.draft);
        let replying = format!("{:?}", p.replying);
        let drafts = p.reply_context.kept.clone();

        p.sync.reading = 4;
        p.sync
            .errors
            .insert(SyncErrorKey::Bootstrap, "old failure".into());
        p.checks.insert("workspace".into(), (Check::Running, NOW));
        let f = p.threads.get_mut(&thread_key(TEAM, ROOT)).unwrap();
        f.loading = true;
        f.error = Some("old page failure".into());

        rotate_token(&mut p, through_configure);

        assert_eq!(p.data.me, None);
        assert_eq!(p.data, crate::model::Data::default());
        assert_eq!(
            p.snapshot().unwrap()["accounts"][0]["data"],
            serde_json::to_value(&data).unwrap()
        );
        assert_eq!(format!("{:?}", p.draft), draft);
        assert_eq!(format!("{:?}", p.replying), replying);
        assert_eq!(p.reply_context.kept, drafts);
        assert!(p.pages.is_empty());
        assert!(p.threads.is_empty());
        assert_eq!(p.sync.reading, 0);
        assert_eq!(p.sync.started, None);
        assert_eq!(p.sync.last_ok, None);
        assert!(p.sync.errors.is_empty());
        assert!(p.checks.is_empty());
    }
}

#[test]
fn recorded_old_bootstrap_cannot_install_identity_or_start_children_after_token_change() {
    for through_configure in [false, true] {
        let mut p = sample();
        p.data.me = None;
        let recorded = Recorded::new(vec![
            ok(json!({"ok": true, "team_id": OLD_TEAM, "user_id": ALICE,
                "team": "old team", "user": "old user", "url": "https://old.example/"})),
            ok(json!({"ok": true, "user": {"id": ALICE, "tz_offset": 3600}})),
        ]);
        p.via = Via(Some(recorded.clone()));
        // Run the old task, then delay delivery across a real configuration change.
        let message = run_only_task(p.read());
        let message = *message.downcast::<SlackMsg>().ok().unwrap();
        let evidence = match &message {
            SlackMsg::Read { read, .. } => history(read),
            _ => unreachable!(),
        };
        assert_eq!(recorded.requests().len(), 2);
        rotate_token(&mut p, through_configure);
        start_fresh_work(&mut p);

        assert_skipped(&mut p, message, evidence);

        assert_eq!(p.data.me, None);
        assert_eq!(
            recorded.requests().len(),
            2,
            "late bootstrap started provider work"
        );
    }
}

#[test]
fn old_bootstrap_failure_does_not_poison_the_new_read() {
    let mut p = sample();
    let stamp = p.read_stamp();
    rotate_token(&mut p, false);
    start_fresh_work(&mut p);
    let read = Read::Me {
        action: sync_history::bootstrap_action(),
        result: Err("OLD_PRIVATE_ERROR".into()),
    };
    let evidence = history(&read);
    assert_skipped(
        &mut p,
        SlackMsg::Read {
            stamp,
            read: Box::new(read),
        },
        evidence,
    );
}

fn semantic_answers() -> Vec<Read> {
    let names = || vec![(BOB.into(), "OLD_PRIVATE_NAME".into())];
    let mut out = Vec::new();
    for failed in [false, true] {
        out.push(Read::Channel {
            id: TEAM.into(),
            result: if failed {
                Err("OLD_PRIVATE_ERROR".into())
            } else {
                Ok(ChannelRead {
                    info: Some(ChannelInfo {
                        id: TEAM.into(),
                        name: "old-channel".into(),
                        ..Default::default()
                    }),
                    list: MessagesRead {
                        messages: vec![Message {
                            ts: ROOT.into(),
                            text: "OLD_PRIVATE_BODY".into(),
                            reply_count: 1,
                            latest_reply: Some("1791270001.000100".into()),
                            ..Default::default()
                        }],
                        evidence: ListEvidence {
                            truncated: true,
                            incomplete_pages: 1,
                        },
                    },
                })
            },
            names: names(),
            names_evidence: NamesEvidence {
                attempted: 2,
                errors: 1,
                deferred: 3,
            },
            action: sync_history::action("slack.sync.channel", OLD_TEAM, TEAM),
        });
        out.push(Read::Threads {
            id: TEAM.into(),
            result: if failed {
                Err("OLD_PRIVATE_ERROR".into())
            } else {
                Ok(ThreadsRead {
                    threads: [(
                        ROOT.into(),
                        vec![demo::msg("1791270001.000100", BOB, "OLD_PRIVATE_BODY")],
                    )]
                    .into(),
                    evidence: ListEvidence {
                        truncated: true,
                        incomplete_pages: 1,
                    },
                    cutoffs: 1,
                    deferred: 2,
                    ..Default::default()
                })
            },
            names: names(),
            names_evidence: NamesEvidence {
                attempted: 1,
                errors: 0,
                deferred: 0,
            },
            action: sync_history::action("slack.sync.threads", OLD_TEAM, TEAM),
        });
        let found = Found {
            channel_id: TEAM.into(),
            channel_name: "old-channel".into(),
            message: demo::msg(ROOT, BOB, "OLD_PRIVATE_BODY"),
            ..Default::default()
        };
        let observed = sync_history::for_you_message(OLD_TEAM, &found);
        out.push(Read::ForYou {
            groups: Some(vec![UserGroup {
                id: "S0000OLD1".into(),
                handle: "old-group".into(),
                ..Default::default()
            }]),
            result: if failed {
                Err("OLD_PRIVATE_ERROR".into())
            } else {
                Ok(vec![found])
            },
            names: names(),
            action: sync_history::for_you_action(OLD_TEAM),
            evidence: ForYouEvidence {
                groups: GroupsEvidence::Returned {
                    returned: 1,
                    selected: 1,
                    incomplete_collection: false,
                    incomplete_entries: 0,
                },
                planned: 2,
                queries: vec![
                    SearchObservation {
                        kind: SearchKind::Mentions,
                        list: Some(SearchList {
                            returned: 1,
                            evidence: ListEvidence::default(),
                        }),
                    },
                    SearchObservation {
                        kind: SearchKind::Direct,
                        list: if failed {
                            None
                        } else {
                            Some(SearchList {
                                returned: 0,
                                evidence: ListEvidence::default(),
                            })
                        },
                    },
                ],
                unique: 1,
                retained: vec![observed],
                ..Default::default()
            },
        });
        out.push(Read::Followed {
            action: sync_history::followed_action(OLD_TEAM),
            batch: FollowedBatch {
                updates: if failed {
                    vec![]
                } else {
                    vec![(
                        TEAM.into(),
                        ROOT.into(),
                        99,
                        vec![Reaction {
                            name: "eyes".into(),
                            count: 99,
                            users: vec![BOB.into()],
                        }],
                    )]
                },
                targets: vec![FollowedTarget {
                    action: sync_history::followed_target(OLD_TEAM, TEAM, ROOT),
                    response: if failed {
                        None
                    } else {
                        Some(FollowedResponse {
                            parent_found: true,
                            messages: 100,
                            evidence: ListEvidence::default(),
                        })
                    },
                }],
            },
        });
    }
    out
}

#[test]
fn stale_channel_threads_search_and_followed_keep_original_history_without_applying() {
    for read in semantic_answers() {
        let mut p = sample();
        let stamp = p.read_stamp();
        let evidence = history(&read);
        // A different workspace makes it impossible to accidentally regenerate
        // the old provider evidence from the current cache identity.
        let mut me = p.data.me.clone().unwrap();
        me.team_id = NEXT_TEAM.into();
        me.user_id = "U000NEXT1".into();
        p.install_me(me);
        start_fresh_work(&mut p);
        assert_skipped(
            &mut p,
            SlackMsg::Read {
                stamp,
                read: Box::new(read),
            },
            evidence,
        );
    }
}

#[test]
fn a_late_channel_success_cannot_finish_an_idle_new_generation_or_start_reply_reads() {
    let mut p = sample();
    let stamp = p.read_stamp();
    let read = semantic_answers().remove(0);
    let evidence = history(&read);
    rotate_token(&mut p, false);
    assert_eq!(p.sync.reading, 0);
    assert_skipped(
        &mut p,
        SlackMsg::Read {
            stamp,
            read: Box::new(read),
        },
        evidence,
    );
    assert_eq!(p.sync.last_ok, None, "old success is not fresh sync health");
}

#[test]
fn recorded_page_answer_cannot_clear_new_loading_overwrite_values_or_advance_read_marks() {
    let mut p = sample();
    let recorded = Recorded::new(vec![ok(json!({"ok": true, "messages": [
        {"ts": ROOT, "user": BOB, "text": "OLD_PRIVATE_BODY", "reply_count": 1},
        {"ts": "1791270999.000100", "user": BOB, "thread_ts": ROOT, "text": "old reply"}
    ]}))]);
    p.via = Via(Some(recorded.clone()));
    let message = run_only_task(crate::page::fetch_thread(&mut p, TEAM, ROOT));
    let message = *message.downcast::<SlackMsg>().ok().unwrap();
    let SlackMsg::Thread { action, result, .. } = &message else {
        panic!("page result")
    };
    let evidence = sync_history::page_thread(action, result);
    let mut next = p.data.me.clone().unwrap();
    next.team_id = NEXT_TEAM.into();
    rotate_token(&mut p, false);
    assert!(p.install_me(next));
    let new = crate::page::fetch_thread(&mut p, TEAM, ROOT);
    assert!(!new.is_empty());
    assert!(p.threads[&thread_key(TEAM, ROOT)].loading);
    assert_skipped(&mut p, message, evidence);
    assert_eq!(recorded.requests().len(), 1);
}

#[test]
fn old_page_failure_cannot_replace_new_page_error_or_clear_its_loading() {
    let mut p = sample();
    let stamp = p.read_stamp();
    let action = sync_history::page_thread_action(OLD_TEAM, TEAM, ROOT);
    let result = Err("OLD_PRIVATE_ERROR".into());
    let evidence = sync_history::page_thread(&action, &result);
    rotate_token(&mut p, false);
    let mut next = demo::sample().data.me.unwrap();
    next.team_id = NEXT_TEAM.into();
    assert!(p.install_me(next));
    let f = p.threads.entry(thread_key(TEAM, ROOT)).or_default();
    f.loading = true;
    f.error = Some("current page failure".into());
    assert_skipped(
        &mut p,
        SlackMsg::Thread {
            stamp,
            channel: TEAM.into(),
            ts: ROOT.into(),
            action,
            result,
        },
        evidence,
    );
}

fn channel_evidence(failed: bool) -> crate::client::ChannelEvidence {
    use crate::client::{ChannelEnd, ChannelEvidence, ChannelKind, ChannelPhase};
    ChannelEvidence {
        phases: [
            Some(ChannelPhase {
                kind: ChannelKind::Info,
                requests: usize::from(!failed),
                returned: usize::from(!failed),
                defaulted_fields: 0,
                identity_gaps: 0,
                incomplete_pages: 0,
                end: if failed {
                    ChannelEnd::Request
                } else {
                    ChannelEnd::Found
                },
                membership: None,
            }),
            None,
        ],
    }
}

fn channel_history(failed: bool) -> Vec<Effect> {
    crate::settings_history::channel(
        &crate::settings_history::channel_action(),
        &channel_evidence(failed),
    )
}

#[test]
fn older_and_duplicate_setting_checks_cannot_overwrite_the_latest_same_epoch_answer() {
    let mut p = sample();
    let key = format!("channel:{TEAM}");
    let older = p.begin_check(&key);
    let latest = p.begin_check(&key);
    // A different key starting later does not invalidate this key's newest check.
    let other = p.begin_check("channel:C000OTHER");
    let effects = p.on_msg(Box::new(SlackMsg::Checked {
        stamp: latest,
        key: key.clone(),
        requested_id: Some(TEAM.into()),
        action: crate::settings_history::channel_action(),
        evidence: channel_evidence(false),
        result: Ok(Checked::Channel(ChannelInfo {
            id: TEAM.into(),
            name: "newest-name".into(),
            ..Default::default()
        })),
    }));
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::Notify(_)))
    );
    assert_eq!(
        p.data.channels[TEAM].info.as_ref().unwrap().name,
        "newest-name"
    );
    for stamp in [older, latest] {
        assert_skipped(
            &mut p,
            SlackMsg::Checked {
                stamp,
                key: key.clone(),
                requested_id: Some(TEAM.into()),
                action: crate::settings_history::channel_action(),
                evidence: channel_evidence(true),
                result: Err("OLD_PRIVATE_ERROR".into()),
            },
            channel_history(true),
        );
    }
    assert!(p.complete_check("channel:C000OTHER", other));
    assert!(
        !p.complete_check("channel:C000OTHER", other),
        "completion is single-use"
    );
}

#[test]
fn newest_check_is_still_accepted_after_an_older_completion_is_rejected() {
    let mut p = sample();
    let first = p.begin_check("workspace");
    let latest = p.begin_check("workspace");
    assert!(!p.complete_check("workspace", first));
    assert!(p.complete_check("workspace", latest));
    assert!(!p.complete_check("workspace", latest));
}

#[test]
fn stale_workspace_channel_add_and_group_checks_neither_mutate_nor_notify() {
    for failed in [false, true] {
        let mut p = sample();
        let workspace = p.begin_check("workspace");
        let add = p.begin_check("add");
        let groups = p.begin_check("groups");
        let mut old_me = p.data.me.clone().unwrap();
        old_me.team_id = "T000OLD99".into();
        rotate_token(&mut p, false);
        let fresh_workspace = p.begin_check("workspace");
        let fresh_add = p.begin_check("add");
        let fresh_groups = p.begin_check("groups");
        for key in ["workspace", "add", "groups"] {
            p.checks.insert(key.into(), (Check::Running, NOW + 1));
        }
        let workspace_action = crate::settings_history::workspace_action();
        let workspace_result = if failed {
            Err("OLD_PRIVATE_ERROR".into())
        } else {
            Ok(old_me)
        };
        let workspace_evidence =
            crate::settings_history::workspace(&workspace_action, &workspace_result);
        assert_skipped(
            &mut p,
            SlackMsg::WorkspaceChecked {
                stamp: workspace,
                action: workspace_action,
                result: workspace_result,
            },
            workspace_evidence,
        );
        assert_skipped(
            &mut p,
            SlackMsg::Checked {
                stamp: add,
                key: "add".into(),
                requested_id: Some("C0000OLD1".into()),
                action: crate::settings_history::channel_action(),
                evidence: channel_evidence(failed),
                result: if failed {
                    Err("OLD_PRIVATE_ERROR".into())
                } else {
                    Ok(Checked::Channel(ChannelInfo {
                        id: "C0000OLD1".into(),
                        name: "old-channel".into(),
                        ..Default::default()
                    }))
                },
            },
            channel_history(failed),
        );
        let action = crate::settings_history::groups_action();
        let result = if failed {
            Err("OLD_PRIVATE_ERROR".into())
        } else {
            Ok(GroupsRead {
                groups: vec![UserGroup {
                    id: "S0000OLD1".into(),
                    handle: "old-group".into(),
                    ..Default::default()
                }],
                incomplete_collection: false,
                incomplete_entries: 0,
            })
        };
        let evidence = crate::settings_history::groups(&action, &result);
        assert_skipped(
            &mut p,
            SlackMsg::Groups {
                stamp: groups,
                action,
                result,
                pick: true,
            },
            evidence,
        );
        assert!(p.complete_check("workspace", fresh_workspace));
        assert!(p.complete_check("add", fresh_add));
        assert!(p.complete_check("groups", fresh_groups));
    }
}

#[test]
fn groups_use_the_latest_single_use_check_even_within_one_epoch() {
    fn empty() -> Result<GroupsRead, String> {
        Ok(GroupsRead {
            groups: vec![],
            incomplete_collection: false,
            incomplete_entries: 0,
        })
    }
    let mut p = sample();
    let old = p.begin_check("groups");
    let latest = p.begin_check("groups");
    assert_skipped(
        &mut p,
        SlackMsg::Groups {
            stamp: old,
            action: crate::settings_history::groups_action(),
            result: empty(),
            pick: true,
        },
        crate::settings_history::groups(&crate::settings_history::groups_action(), &empty()),
    );
    let groups = vec![UserGroup {
        id: "S0000NEW1".into(),
        handle: "new-group".into(),
        ..Default::default()
    }];
    let effects = p.on_msg(Box::new(SlackMsg::Groups {
        stamp: latest,
        action: crate::settings_history::groups_action(),
        result: Ok(GroupsRead {
            groups: groups.clone(),
            incomplete_collection: false,
            incomplete_entries: 0,
        }),
        pick: false,
    }));
    assert_eq!(p.data.groups, groups);
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::Notify(_)))
    );
    assert_skipped(
        &mut p,
        SlackMsg::Groups {
            stamp: latest,
            action: crate::settings_history::groups_action(),
            result: empty(),
            pick: true,
        },
        crate::settings_history::groups(&crate::settings_history::groups_action(), &empty()),
    );
}

enum HostCommand {
    Start,
    ChangeIdentity,
    SyntheticReaction { failed: bool },
}

struct GuardedHost(std::sync::Arc<std::sync::Mutex<SlackPlugin>>);

impl Plugin for GuardedHost {
    fn id(&self) -> &'static str {
        "slack"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }

    fn on_msg(&mut self, message: Msg) -> Vec<Effect> {
        let mut p = self.0.lock().unwrap();
        match message.downcast::<HostCommand>() {
            Ok(command) => match *command {
                HostCommand::Start => p.read(),
                HostCommand::ChangeIdentity => {
                    let mut me = p.data.me.clone().unwrap();
                    me.team_id = NEXT_TEAM.into();
                    p.install_me(me);
                    p.read()
                }
                HostCommand::SyntheticReaction { failed } => {
                    p.via = Via(Some(Recorded::new(vec![ok(if failed {
                        json!({"ok": false, "error": "OLD_PRIVATE_ERROR"})
                    } else {
                        json!({"ok": true})
                    })])));
                    let pick = p
                        .pick_reaction(TEAM, ROOT, "this message".into())
                        .into_iter()
                        .find_map(|effect| match effect {
                            Effect::Menu(menu) => Some(menu.id),
                            _ => None,
                        })
                        .unwrap();
                    let mut effects = p.answer_reaction(&pick, "eyes").unwrap();
                    let confirm = effects
                        .iter()
                        .find_map(|effect| match effect {
                            Effect::Menu(menu) => Some(menu.id.clone()),
                            _ => None,
                        })
                        .unwrap();
                    effects.retain(|effect| !matches!(effect, Effect::Menu(_)));
                    let mode = radar_core::writes::mode();
                    radar_core::writes::set_mode(radar_core::writes::Mode::Edit, None);
                    effects.extend(p.answer_reaction(&confirm, "yes").unwrap());
                    radar_core::writes::set_mode(mode.0, mode.1);
                    effects
                }
            },
            Err(message) => p.on_msg(message),
        }
    }
}

#[test]
fn current_bootstrap_identity_change_invalidates_peers_and_dispatches_applicable_new_children() {
    for change_user in [false, true] {
        let mut p = sample();
        p.cfg.channels.truncate(1);
        p.data.asked.clear();
        p.data.channels.get_mut(TEAM).unwrap().info = Some(ChannelInfo {
            id: TEAM.into(),
            name: "known-channel".into(),
            ..Default::default()
        });
        let (team, user) = if change_user {
            (OLD_TEAM, "U000NEXT1")
        } else {
            (NEXT_TEAM, ALICE)
        };
        let empty_search =
            || ok(json!({"ok": true, "messages": {"matches": [], "paging": {"pages": 1}}}));
        let recorded = Recorded::new(vec![
            ok(json!({"ok": true, "team_id": team, "user_id": user,
                "team": "fresh team", "user": "fresh user", "url": "https://fresh.example/"})),
            ok(json!({"ok": true, "user": {"id": user, "tz_offset": 7200}})),
            ok(json!({"ok": true, "channel": {"id": TEAM, "name": "fresh-channel"}})),
            ok(
                json!({"ok": true, "messages": [{"ts": ROOT, "user": BOB, "text": "fresh channel body"}], "has_more": false}),
            ),
            ok(json!({"ok": true, "user": {"id": BOB, "name":"fresh-name"}})),
            ok(json!({"ok":true, "usergroups":[]})),
            empty_search(),
            empty_search(),
        ]);
        p.via = Via(Some(recorded));
        let old_stamp = p.read_stamp();
        let _old_peers = p.read();
        assert_eq!(p.sync.reading, 2);
        // Older cache identity lacks a time zone, so a current bootstrap may
        // discover a changed team or user while same-epoch peers still exist.
        p.data.me.as_mut().unwrap().tz_offset = None;
        let bootstrap = run_only_task(p.read());
        assert_eq!(p.sync.reading, 3);
        p.now = NOW + 7;
        let children = p.on_msg(bootstrap);
        assert_ne!(p.read_stamp(), old_stamp);
        assert_eq!(
            p.sync.started,
            Some(NOW + 7),
            "replacement children start a fresh cycle"
        );
        let current_stamp = p.read_stamp();
        assert_eq!(p.data.me.as_ref().unwrap().team_id, team);
        assert_eq!(p.data.me.as_ref().unwrap().user_id, user);
        let children: Vec<_> = children
            .into_iter()
            .filter(|effect| matches!(effect, Effect::SpawnAction { .. }))
            .collect();
        assert_eq!(children.len(), 2);
        assert_eq!(
            p.sync.reading,
            children.len(),
            "old bootstrap and peer count must be discarded"
        );
        let stale = semantic_answers().remove(0);
        let evidence = history(&stale);
        assert_skipped(
            &mut p,
            SlackMsg::Read {
                stamp: old_stamp,
                read: Box::new(stale),
            },
            evidence,
        );
        for (index, child) in children.into_iter().enumerate() {
            if let Effect::SpawnAction { action, .. } = &child {
                assert_eq!(action.subjects[0].key.instance, team);
            }
            let message = run_only_task(vec![child]);
            let message = message.downcast::<SlackMsg>().ok().unwrap();
            assert!(matches!(&*message, SlackMsg::Read { stamp, .. } if *stamp == current_stamp));
            let applied = p.on_msg(message);
            assert!(
                applied
                    .iter()
                    .any(|effect| matches!(effect, Effect::Report(Outcome::Succeeded))),
                "{applied:?}"
            );
            assert!(
                !applied
                    .iter()
                    .any(|effect| matches!(effect, Effect::Observe { action, .. }
                if action.kind.as_str() == "slack.read.apply.skipped.stale"))
            );
            assert_eq!(p.sync.reading, 1 - index);
        }
        assert_eq!(p.data.channels[TEAM].messages[0].text, "fresh channel body");
        assert!(p.data.for_you.is_empty());
        assert_eq!(p.sync.last_ok, Some(NOW + 7));
    }
}

#[test]
fn same_identity_bootstrap_retains_its_epoch_and_original_cycle_start() {
    let mut p = sample();
    p.cfg.channels.truncate(1);
    p.data.asked.clear();
    p.data.me.as_mut().unwrap().tz_offset = None;
    let stamp = p.read_stamp();
    let _pending = p.read();
    assert_eq!(p.sync.started, Some(NOW));
    assert_eq!(p.sync.reading, 1);
    let mut me = p.data.me.clone().unwrap();
    me.tz_offset = Some(7200);
    p.now = NOW + 7;
    let effects = p.on_msg(Box::new(SlackMsg::Read {
        stamp,
        read: Box::new(Read::Me {
            action: sync_history::bootstrap_action(),
            result: Ok(crate::client::BootstrapRead {
                me,
                timezone: crate::client::TimezoneEvidence::Explicit,
            }),
        }),
    }));
    assert_eq!(p.read_stamp(), stamp);
    assert_eq!(p.sync.started, Some(NOW));
    assert_eq!(p.sync.reading, 2);
    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, Effect::SpawnAction { .. }))
            .count(),
        2
    );
}

#[test]
fn delayed_write_results_after_token_change_still_complete_the_original_write_scope() {
    use radar_core::Host;
    use radar_core::host::{Event, HostEffect, JournalRecord};

    for failed in [false, true] {
        let shared = std::sync::Arc::new(std::sync::Mutex::new(sample()));
        let mut host = Host::new(vec![Box::new(GuardedHost(shared.clone()))], NOW).unwrap();
        host.enable_journal();
        let dispatched = host.update(Event::Msg {
            plugin: 0,
            msg: Box::new(HostCommand::SyntheticReaction { failed }),
        });
        let semantic = dispatched
            .iter()
            .find_map(|effect| match effect {
                HostEffect::Journal(JournalRecord::Begin { id, descriptor, .. })
                    if descriptor.kind.as_str() == "slack.reaction.add" =>
                {
                    assert_eq!(descriptor.subjects[0].key.instance, OLD_TEAM);
                    Some(id.clone())
                }
                _ => None,
            })
            .unwrap();
        let (task_id, task) = dispatched
            .into_iter()
            .find_map(|effect| match effect {
                HostEffect::Tracked {
                    action,
                    semantic: Some(id),
                    effect,
                } if id == semantic => match *effect {
                    HostEffect::Spawn { task, .. } => Some((action, task)),
                    _ => None,
                },
                _ => None,
            })
            .unwrap();
        let message = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(task);
        {
            let mut p = shared.lock().unwrap();
            let mut me = p.data.me.clone().unwrap();
            rotate_token(&mut p, false);
            me.team_id = NEXT_TEAM.into();
            p.install_me(me);
        }
        let effects = host.update(Event::Tracked {
            action: task_id.clone(),
            semantic: Some(semantic.clone()),
            event: Box::new(Event::Msg {
                plugin: 0,
                msg: message,
            }),
        });
        let records: Vec<_> = effects
            .iter()
            .filter_map(|effect| match effect {
                HostEffect::Journal(record) => Some(record),
                _ => None,
            })
            .collect();
        let expected = if failed {
            Outcome::Unknown
        } else {
            Outcome::Succeeded
        };
        for target in [&task_id, &semantic] {
            let terminal: Vec<_> = records
                .iter()
                .filter_map(|record| match record {
                    JournalRecord::Outcome { id, outcome, .. }
                        if id == target && outcome.is_terminal() =>
                    {
                        Some(*outcome)
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(
                terminal,
                [expected],
                "an old write must still report its real result"
            );
        }
        assert!(!records.iter().any(|record| matches!(record,
            JournalRecord::Begin { descriptor, .. } if descriptor.kind.as_str() == "slack.read.apply.skipped.stale")));
        assert!(!format!("{records:?}").contains("OLD_PRIVATE_ERROR"));
        assert!(
            !effects.iter().any(|effect| matches!(
                effect,
                HostEffect::Spawn { .. } | HostEffect::Tracked { .. }
            )),
            "a stale reaction completion must not refresh through the new client"
        );
    }
}

#[test]
fn tracked_late_results_keep_original_terminal_outcome_and_old_scope_with_a_blocked_skip_child() {
    use radar_core::Host;
    use radar_core::host::{Event, HostEffect, JournalRecord};
    use radar_core::journal::ActionId;

    fn records(effects: &[HostEffect]) -> Vec<JournalRecord> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                HostEffect::Journal(record) => Some(record.clone()),
                _ => None,
            })
            .collect()
    }

    fn terminal(records: &[JournalRecord], target: &ActionId) -> Vec<Outcome> {
        records
            .iter()
            .filter_map(|record| match record {
                JournalRecord::Outcome { id, outcome, .. }
                    if id == target && outcome.is_terminal() =>
                {
                    Some(*outcome)
                }
                _ => None,
            })
            .collect()
    }

    for failed in [false, true] {
        let mut p = sample();
        p.cfg.channels.truncate(1);
        p.data.channels.get_mut(TEAM).unwrap().info = Some(ChannelInfo {
            id: TEAM.into(),
            name: "known-channel".into(),
            ..Default::default()
        });
        let response = if failed {
            json!({"ok": false, "error": "OLD_PRIVATE_ERROR"})
        } else {
            json!({"ok": true, "messages": [{"ts": ROOT, "user": BOB,
                "text": "OLD_PRIVATE_BODY", "reply_count": 1,
                "latest_reply": "1791270001.000100"}], "has_more": false})
        };
        let recorded = Recorded::new(vec![ok(response)]);
        p.via = Via(Some(recorded.clone()));
        let shared = std::sync::Arc::new(std::sync::Mutex::new(p));
        let mut host = Host::new(vec![Box::new(GuardedHost(shared.clone()))], NOW).unwrap();
        host.enable_journal();
        let dispatched = host.update(Event::Msg {
            plugin: 0,
            msg: Box::new(HostCommand::Start),
        });
        let initial = records(&dispatched);
        let (semantic, scope) = initial
            .iter()
            .find_map(|record| match record {
                JournalRecord::Begin { id, descriptor, .. }
                    if descriptor.kind.as_str() == "slack.sync.channel" =>
                {
                    Some((id.clone(), descriptor.clone()))
                }
                _ => None,
            })
            .unwrap();
        let (task_id, task) = dispatched
            .into_iter()
            .find_map(|effect| match effect {
                HostEffect::Tracked {
                    action,
                    semantic: Some(id),
                    effect,
                } if id == semantic => match *effect {
                    HostEffect::Spawn { task, .. } => Some((action, task)),
                    _ => None,
                },
                _ => None,
            })
            .unwrap();
        let message = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(task);
        assert_eq!(recorded.requests().len(), 1);
        let _fresh = host.update(Event::Msg {
            plugin: 0,
            msg: Box::new(HostCommand::ChangeIdentity),
        });
        let before = held(&shared.lock().unwrap());
        let effects = host.update(Event::Tracked {
            action: task_id.clone(),
            semantic: Some(semantic.clone()),
            event: Box::new(Event::Msg {
                plugin: 0,
                msg: message,
            }),
        });
        assert_eq!(held(&shared.lock().unwrap()), before);
        assert!(
            effects
                .iter()
                .all(|effect| matches!(effect, HostEffect::Journal(_))),
            "rejected completion generated follow-on work: {effects:?}"
        );
        let completed = records(&effects);
        let expected = if failed {
            Outcome::Failed
        } else {
            Outcome::Succeeded
        };
        assert_eq!(terminal(&completed, &task_id), [expected]);
        assert_eq!(terminal(&completed, &semantic), [expected]);
        let mut skip = None;
        let mut provider_observations = 0;
        for record in &completed {
            if let JournalRecord::Begin {
                id,
                descriptor,
                parent,
            } = record
            {
                assert_eq!(parent.as_ref(), Some(&task_id));
                assert_eq!(
                    descriptor.subjects, scope.subjects,
                    "must retain the dispatch workspace"
                );
                assert_eq!(descriptor.subjects[0].key.instance, OLD_TEAM);
                if descriptor.kind.as_str() == "slack.read.apply.skipped.stale" {
                    assert_eq!(descriptor.metadata.reason, Some(Reason::Unspecified));
                    assert!(
                        skip.replace(id.clone()).is_none(),
                        "exactly one skip observation"
                    );
                } else {
                    assert!(descriptor.kind.as_str().starts_with("slack.sync."));
                    provider_observations += 1;
                }
            }
        }
        assert!(provider_observations > 0, "provider evidence was lost");
        assert_eq!(terminal(&completed, &skip.unwrap()), [Outcome::Blocked]);
        assert!(!format!("{completed:?}").contains("OLD_PRIVATE_"));
        assert_eq!(
            recorded.requests().len(),
            1,
            "no stale-dependent read was dispatched"
        );
    }
}
