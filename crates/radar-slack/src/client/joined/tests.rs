use super::*;
use crate::client::{Recorded, Reply, Request};
use serde_json::json;

fn ok(body: Value) -> Result<Reply> {
    Ok(Reply { status: 200, body: body.to_string(), ..Reply::default() })
}

fn client(replies: Vec<Result<Reply>>) -> (Client, std::sync::Arc<Recorded>) {
    let recorded = Recorded::new(replies);
    let client = Client::new(recorded.clone()).with_wait(std::sync::Arc::new(|_| Box::pin(async {})));
    (client, recorded)
}

fn row(id: &str, name: &str, private: bool) -> Value {
    json!({"id": id, "name": name, "is_private": private, "is_archived": false,
        "topic": {"value": ""}, "purpose": {"value": "keeping the watch"}})
}

fn list(rows: Vec<Value>, next: &str) -> Result<Reply> {
    ok(json!({"ok": true, "channels": rows, "response_metadata": {"next_cursor": next}}))
}

fn request(types: &str, cursor: &str) -> Request {
    let mut params = vec![
        ("types".to_string(), types.to_string()),
        ("exclude_archived".into(), "true".into()),
        ("limit".into(), "999".into()),
    ];
    if !cursor.is_empty() {
        params.push(("cursor".into(), cursor.into()));
    }
    Client::read("users.conversations", params)
}

#[tokio::test]
async fn every_page_is_read_and_the_channels_come_back_by_name_as_members() {
    let (c, recorded) = client(vec![
        list(vec![row("C0WATCH01", "watch-house", false), row("C0DUTY001", "Night-duty", true)], "next"),
        list(vec![row("C0GUILD01", "assassins-guild", false), row("C0WATCH01", "watch-house", false)], ""),
    ]);
    let got = c.joined_channels(true).await.unwrap();
    let names: Vec<_> = got.channels.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["assassins-guild", "Night-duty", "watch-house"], "by name, once each");
    assert!(got.channels.iter().all(|c| c.is_member));
    assert!(got.channels.iter().any(|c| c.is_private && c.name == "Night-duty"));
    assert!(!got.private_skipped && !got.truncated);
    assert_eq!(
        recorded.requests(),
        [request("public_channel,private_channel", ""), request("public_channel,private_channel", "next")]
    );
}

#[tokio::test]
async fn archived_and_nameless_rows_are_left_out() {
    let mut archived = row("C0OLD0001", "old-watch", false);
    archived["is_archived"] = json!(true);
    let (c, _) = client(vec![list(vec![archived, row("C0NONAME1", "", false), row("", "x", false)], "")]);
    assert!(c.joined_channels(true).await.unwrap().channels.is_empty());
}

#[tokio::test]
async fn a_token_without_groups_read_gets_the_public_channels_and_the_reason() {
    let refused = ok(json!({"ok": false, "error": "missing_scope", "needed": "groups:read"}));
    let (c, recorded) = client(vec![refused, list(vec![row("C0WATCH01", "watch-house", false)], "")]);
    let got = c.joined_channels(true).await.unwrap();
    assert_eq!(got.channels.len(), 1);
    assert!(got.private_skipped, "the list says private channels were left out");
    assert_eq!(
        recorded.requests(),
        [request("public_channel,private_channel", ""), request("public_channel", "")]
    );
}

#[tokio::test]
async fn asking_for_public_channels_only_asks_once_and_says_the_rest_were_left_out() {
    let (c, recorded) = client(vec![list(vec![row("C0WATCH01", "watch-house", false)], "")]);
    let got = c.joined_channels(false).await.unwrap();
    assert!(got.private_skipped);
    assert_eq!(recorded.requests(), [request("public_channel", "")]);
}

#[tokio::test]
async fn another_refusal_is_the_error_and_nothing_is_retried() {
    let (c, recorded) = client(vec![ok(json!({"ok": false, "error": "invalid_auth"}))]);
    let error = c.joined_channels(true).await.unwrap_err();
    assert!(format!("{error:#}").contains("refused"), "{error:#}");
    assert_eq!(recorded.requests().len(), 1);
}

#[tokio::test]
async fn the_page_limit_stops_the_list_and_says_so() {
    let pages = (0..MAX_LIST_PAGES)
        .map(|n| list(vec![row(&format!("C{n:08}"), &format!("watch-{n:03}"), false)], "more"))
        .collect();
    let (c, recorded) = client(pages);
    let got = c.joined_channels(true).await.unwrap();
    assert!(got.truncated);
    assert_eq!(got.channels.len(), MAX_LIST_PAGES);
    assert_eq!(recorded.requests().len(), MAX_LIST_PAGES);
}
