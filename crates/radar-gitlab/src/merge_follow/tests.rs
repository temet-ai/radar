//! The follow-up of an accepted merge over held data: tasks are created and
//! dropped without polling, so no client, credential or network is used.

use super::*;
use crate::demo::sample;
use crate::list_actions::{StateClick, state_click};
use crate::model::Mr;
use crate::mr_actions::{self, ActMsg};
use crate::ready::blocks;
use radar_core::Plugin;

const PROJECT: &str = "acme/team-a/web";
const KEY: &str = "acme/team-a/web!187";

/// !187 is ready to merge; the merge is asked for and GitLab accepts it.
fn taken(p: &mut GitlabPlugin) -> Vec<Effect> {
    let connection = p.connection_stamp();
    drop(mr_actions::start(p, PROJECT, 187, "merge"));
    mr_actions::on_msg(
        p,
        ActMsg {
            connection,
            key: KEY.into(),
            what: "merge",
            result: Ok("GitLab took it; it is opened for now".into()),
            outcome: JournalOutcome::Unknown,
        },
    )
}

fn group(p: &GitlabPlugin) -> String {
    group_of(p, KEY).expect("the sample holds !187")
}

fn held(p: &GitlabPlugin) -> &Mr {
    p.data.values().flat_map(|d| d.mrs.iter()).find(|m| m.iid == 187 && m.project_path == PROJECT).unwrap()
}

fn held_mut(p: &mut GitlabPlugin) -> &mut Mr {
    p.data.values_mut().flat_map(|d| d.mrs.iter_mut()).find(|m| m.iid == 187 && m.project_path == PROJECT).unwrap()
}

fn first_words(p: &GitlabPlugin) -> String {
    blocks(held(p)).first().map(|b| b.words()).unwrap_or_default()
}

/// The `gitlab.merge.confirm` record among the effects, by its outcome.
fn confirmed(fx: &[Effect]) -> Option<JournalOutcome> {
    fx.iter().find_map(|e| match e {
        Effect::Observe { action, outcome } if action.kind.as_str() == "gitlab.merge.confirm" => Some(*outcome),
        _ => None,
    })
}

fn notice(fx: &[Effect]) -> String {
    fx.iter().map(|e| format!("{e:?}")).collect::<Vec<_>>().join(" | ")
}

#[test]
fn a_merge_gitlab_accepted_is_checked_until_it_merges() {
    let mut p = sample();
    let fx = taken(&mut p);
    assert!(
        fx.iter().any(|e| matches!(e, Effect::Notify(w) if w == "GitLab accepted merge !187; checking…")),
        "{}",
        notice(&fx)
    );
    assert_eq!(held(&p).merge_watch, MergeWatch::Checking);
    assert_eq!(first_words(&p), "merging · checking");
    assert_eq!(mr_actions::blocker(held(&p)).as_deref(), Some("merging · checking"));
    assert!(mr_actions::running(&p), "the part is busy while it checks");
    assert!(following(&p, PROJECT, 187));
    // Every few seconds, not at the refresh interval.
    p.sync.last_started = Some(p.now);
    assert!(!check_due(&p));
    p.now += CHECK_EVERY;
    assert!(check_due(&p));
    let g = group(&p);
    let fx = settle(&mut p, &g, &[(KEY.into(), MrState::Merged)]);
    assert_eq!(p.acting[KEY].state, Acting::Done("merged !187".into()));
    assert_eq!(confirmed(&fx), Some(JournalOutcome::Succeeded));
    assert!(p.merging.is_empty());
    assert_eq!(held(&p).merge_watch, MergeWatch::None);
}

#[test]
fn a_merge_gitlab_refuses_later_says_its_reason_and_can_be_asked_again() {
    let mut p = sample();
    drop(taken(&mut p));
    held_mut(&mut p).merge_error = "Branch cannot be merged".into();
    let g = group(&p);
    let fx = settle(&mut p, &g, &[]);
    assert_eq!(
        p.acting[KEY].state,
        Acting::Failed("GitLab did not merge !187: Branch cannot be merged".into())
    );
    assert_eq!(confirmed(&fx), Some(JournalOutcome::Failed));
    assert!(p.merging.is_empty());
    assert_eq!(first_words(&p), "merge failed: Branch cannot be merged");
    assert_eq!(mr_actions::blocker(held(&p)), None, "merge is offered again");
    assert_eq!(state_click(held(&p)), Some(StateClick::Merge));
}

#[test]
fn a_merge_gitlab_is_not_doing_is_said_after_15_seconds() {
    let mut p = sample();
    drop(taken(&mut p));
    let g = group(&p);
    p.now += QUIET_AFTER - 5;
    assert!(settle(&mut p, &g, &[]).is_empty(), "still within the wait");
    assert_eq!(held(&p).merge_watch, MergeWatch::Checking);
    p.now += 5;
    let fx = settle(&mut p, &g, &[]);
    assert_eq!(confirmed(&fx), Some(JournalOutcome::Failed));
    assert!(
        matches!(&p.acting[KEY].state, Acting::Failed(w) if w.contains("is not merging it; merge again")),
        "{:?}",
        p.acting[KEY]
    );
    assert_eq!(first_words(&p), "not merging · merge again");
    assert_eq!(mr_actions::blocker(held(&p)), None);
    assert_eq!(state_click(held(&p)), Some(StateClick::Merge));
    // The row's menu leads with merge, as it does for one ready to merge.
    let mr = held(&p).clone();
    let rows = crate::list_actions::bindings(&p, &mr);
    assert!(rows.iter().any(|(_, b)| b.keys == "m" && b.lead), "{rows:?}");
    // A new merge ends this follow-up.
    drop(mr_actions::start(&mut p, PROJECT, 187, "merge"));
    assert!(p.merging.is_empty());
    assert_eq!(held(&p).merge_watch, MergeWatch::None);
}

#[test]
fn no_answer_is_said_only_when_the_tenancy_fails() {
    let mut p = sample();
    drop(taken(&mut p));
    let g = group(&p);
    p.now += NO_ANSWER_AFTER;
    assert!(unanswered(&mut p).is_empty(), "a clock alone concludes nothing");
    assert!(!check_due(&p), "it stops asking every few seconds");
    p.sync.errors.insert(g.clone(), "boom".into());
    let fx = unanswered(&mut p);
    assert!(
        fx.iter().any(|e| matches!(e, Effect::Notify(w) if w == "no answer from GitLab about merge !187; the next sync will show it")),
        "{}",
        notice(&fx)
    );
    assert_eq!(confirmed(&fx), Some(JournalOutcome::Unknown));
    assert_eq!(first_words(&p), "merge unconfirmed");
    // An answer at last: GitLab's own fields say the rest.
    p.sync.errors.clear();
    assert!(settle(&mut p, &g, &[]).is_empty());
    assert!(p.merging.is_empty());
    assert_eq!(held(&p).merge_watch, MergeWatch::None);
}

#[test]
fn a_check_from_an_old_connection_is_dropped() {
    let mut p = sample();
    drop(taken(&mut p));
    let g = group(&p);
    p.replace_connection("https://gitlab.example".into(), "another-token".into());
    assert!(settle(&mut p, &g, &[(KEY.into(), MrState::Merged)]).is_empty());
    assert!(p.merging.is_empty());
}

/// The tick asks for a sync while a merge is checked, not only at the
/// refresh interval.
#[test]
fn the_tick_syncs_while_a_merge_is_checked() {
    let mut p = sample();
    drop(taken(&mut p));
    p.sync.last_started = Some(p.now);
    let now = p.now + CHECK_EVERY;
    assert!(!Plugin::tick(&mut p, now).is_empty(), "a sync starts");
}
