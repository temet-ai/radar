use super::*;
use crate::demo::{BOB, GROUP, TEAM, ago, msg};

const URL: &str = "https://code.example/acme/api/pull/7";

fn pull() -> Subject {
    Subject::new(PULL_REQUEST, URL, "api #7")
        .with_url(URL)
        .with("title", "ABC-12: A change")
}

fn held(url: &str) -> SlackPlugin {
    let mut p = crate::tests::sample();
    p.data.channels.clear();
    p.data.channels.insert(
        TEAM.into(),
        crate::model::ChannelData {
            info: None,
            messages: vec![msg(
                &ago(100),
                BOB,
                &format!("<!subteam^{GROUP}> <{url}|review> ABC-12"),
            )],
            ..Default::default()
        },
    );
    p.changed();
    p
}

fn actions(p: &SlackPlugin, s: &Subject) -> Vec<Action> {
    let Some(Contribution::Actions(actions)) = contribute(p, Point::Actions, s) else {
        return vec![];
    };
    actions
}

#[test]
fn pull_request_exact_discussions_fill_read_only_places_without_review_actions() {
    let mut p = held(URL);
    let s = pull();
    assert_eq!(threads_about(&p, &s).len(), 1);
    let Some(Contribution::Cell(cell)) = contribute(&p, Point::Column, &s) else {
        panic!()
    };
    assert_eq!(cell.text, "discussed · 1");
    let thread = cell.show.unwrap();
    assert_eq!(thread.field("channel"), Some(TEAM));
    let Some(Contribution::Section { lines, .. }) = contribute(&p, Point::Section, &s) else {
        panic!()
    };
    assert!(
        !lines
            .iter()
            .any(|line| line.to_string().contains("review asked"))
    );
    let Some(Contribution::Tab { rows, .. }) = contribute(&p, Point::Tab, &s) else {
        panic!()
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].subject, Some(thread.clone()));
    let actions = actions(&p, &s);
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].what, "open discussion in #team");
    assert!(actions[0].change.is_none());
    assert!(
        matches!(&act(&mut p, &s, &actions[0].id)[..], [Effect::Show(shown)] if shown == &thread)
    );
    for action in ["ask", "thread", "reask", "unknown"] {
        assert!(act(&mut p, &s, action).is_empty(), "{action}");
    }
    assert!(p.draft.is_none());
    assert!(p.replying.is_none());
}

#[test]
fn pull_request_discussions_never_match_neighbor_host_path_query_or_bare_key() {
    let s = pull();
    for url in [
        "https://other.example/acme/api/pull/7",
        "https://code.example/acme/api/pull/70",
        "https://code.example/acme/api/pull/7/files",
        "https://code.example/acme/api/pull/7#comment",
        "https://code.example/acme/api/pull/7?view=1",
        "https://code.example/acme/api/pull/7/",
        "ABC-12",
    ] {
        let p = held(url);
        assert!(threads_about(&p, &s).is_empty(), "{url}");
        for point in [Point::Column, Point::Section, Point::Tab, Point::Actions] {
            assert!(contribute(&p, point, &s).is_none(), "{url}: {point:?}");
        }
    }
    let p = held(URL);
    let mut missing = s;
    missing.url = None;
    assert!(threads_about(&p, &missing).is_empty());
}

#[test]
fn pull_request_discussion_action_keeps_original_thread_after_sort_order_changes() {
    let mut p = held(URL);
    let s = pull();
    let action = actions(&p, &s).remove(0);
    p.data
        .channels
        .get_mut(TEAM)
        .unwrap()
        .messages
        .push(msg(&ago(10), BOB, URL));
    p.changed();
    assert!(
        matches!(&act(&mut p, &s, &action.id)[..], [Effect::Show(thread)]
        if thread.field("ts") == Some(ago(100).as_str()))
    );
}

#[test]
fn pull_request_discussion_actions_refuse_changed_workspace_or_missing_relationship() {
    for change in ["team", "host", "url", "removed"] {
        let mut p = held(URL);
        let s = pull();
        let action = actions(&p, &s).remove(0);
        match change {
            "team" => p.data.me.as_mut().unwrap().team_id = "T00OTHER1".into(),
            "host" => p.data.me.as_mut().unwrap().url = "https://other.slack.com/".into(),
            "url" => {
                p.data.channels.get_mut(TEAM).unwrap().messages[0].text =
                    "https://other.example/acme/api/pull/7".into()
            }
            "removed" => p.data.channels.clear(),
            _ => unreachable!(),
        }
        p.changed();
        assert!(act(&mut p, &s, &action.id).is_empty(), "{change}");
    }
}

#[test]
fn pull_request_does_not_inherit_path_only_repository_alerts_or_change_legacy_review() {
    let p = held(URL);
    let repo = Subject::new("repository", "https://code.example/acme/api", "api");
    assert!(contribute(&p, Point::Chips, &repo).is_none());
    assert!(contribute(&p, Point::Tab, &repo).is_none());
    let mut legacy = pull();
    legacy.kind = MERGE_REQUEST.into();
    assert!(actions(&p, &legacy).iter().any(|a| a.id == "ask"));
    let p = held("https://code.example/acme/api/pull/7/files");
    assert_eq!(
        threads_about(&p, &legacy).len(),
        1,
        "legacy subtree matching stays unchanged"
    );
}
