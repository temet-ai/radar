use super::*;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

#[tokio::test]
async fn real_socket_lists_tools_routes_calls_and_keeps_partial_input_while_answering() {
    let dir = std::env::temp_dir().join(format!("radar-context-{}", new_ticket().unwrap()));
    let mut server = ContextServer::bind(dir.join("context.sock"), Arc::new(|| {})).unwrap();
    let stream = UnixStream::connect(&server.listener.path).await.unwrap();
    let (rd, mut wr) = stream.into_split();
    let mut rd = BufReader::new(rd);
    wr.write_all(format!("{}\n", json!({"radar_ticket":server.listener.ticket})).as_bytes())
        .await
        .unwrap();
    wr.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n")
        .await
        .unwrap();
    let mut line = String::new();
    rd.read_line(&mut line).await.unwrap();
    let listed: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 4);
    wr.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"context\",\"arguments\":{\"roots\":[\"WATCH-12\"]}}}\n").await.unwrap();
    let calls = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let calls = server.calls();
            if !calls.is_empty() {
                break calls;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(calls[0].tool, "context");
    // Half the next frame must survive a concurrent response.
    wr.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":3,")
        .await
        .unwrap();
    server.answer(calls[0].id, Ok(json!({"items":[{"title":"WATCH-12"}]})));
    line.clear();
    rd.read_line(&mut line).await.unwrap();
    let answer: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(
        answer["result"]["structuredContent"]["items"][0]["title"],
        "WATCH-12"
    );
    wr.write_all(b"\"method\":\"ping\"}\n").await.unwrap();
    line.clear();
    rd.read_line(&mut line).await.unwrap();
    assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["id"], 3);
    drop(server);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn wrong_ticket_never_reaches_the_host() {
    let dir = std::env::temp_dir().join(format!("radar-context-{}", new_ticket().unwrap()));
    let mut server = ContextServer::bind(dir.join("context.sock"), Arc::new(|| {})).unwrap();
    let stream = UnixStream::connect(&server.listener.path).await.unwrap();
    let (rd, mut wr) = stream.into_split();
    wr.write_all(b"{\"radar_ticket\":\"wrong\"}\n")
        .await
        .unwrap();
    let mut reader = BufReader::new(rd);
    let mut line = String::new();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), reader.read_line(&mut line))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    assert!(server.calls().is_empty());
    drop(server);
    let _ = std::fs::remove_dir_all(dir);
}
