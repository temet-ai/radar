//! radar's own MCP server for an agent session, as far as
//! step S1, the spike, needs it: one listener per session on a unix socket, the MCP methods
//! `initialize`, `ping`, `tools/list` and `tools/call`, and one tool, `echo`.
//!
//! The CLI starts `radar mcp --socket <path> --ticket <t>` as a stdio MCP server; that is
//! [`relay_main`], which connects to the session's socket, sends the ticket as its first line
//! and then copies bytes both ways. It holds no state and speaks no MCP. The protocol is
//! spoken here, so a test drives the whole server through a socket with no CLI.
//!
//! Wire: newline-delimited JSON-RPC 2.0, each line at most [`MAX_LINE`] bytes. The first line
//! a connection sends is `{"radar_ticket":"<t>"}`; a wrong ticket, or a peer of another user,
//! is closed without an answer.

use std::io;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tokio::task::{JoinHandle, JoinSet};

use crate::adapter::ToolServer;
use crate::event::RadarSessionId;

/// The server's name in every CLI: Claude Code calls its tools `mcp__radar__<tool>`.
pub const SERVER_NAME: &str = "radar";
/// The MCP revisions this server answers, newest first. A client asking for another gets
/// the newest, as the specification says.
pub const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];
/// The longest line read; a longer one closes the connection.
pub const MAX_LINE: usize = 1 << 20;
/// The tools this server offers.
pub const TOOLS: &[&str] = &["echo"];

/// A `tools/call` that reached the session's listener.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolCallSeen {
    pub session: RadarSessionId,
    pub tool: String,
    pub arguments: Value,
}

fn tool_specs() -> Value {
    json!([{
        "name": "echo",
        "title": "Echo",
        "description": "Returns the text it is given. Proves the session reaches radar.",
        "inputSchema": {
            "type": "object",
            "properties": { "text": { "type": "string", "description": "The text to return" } },
            "required": ["text"],
        },
        "annotations": { "readOnlyHint": true, "openWorldHint": false },
    }])
}

fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn result(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// The answer to one message. None for a notification (no `id`) and for a response the
/// client sends (no `method`): neither is answered.
pub fn answer(msg: &Value) -> Option<Value> {
    let id = msg.get("id")?;
    let method = msg.get("method").and_then(Value::as_str)?;
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    Some(match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str);
            let version = asked.filter(|v| PROTOCOL_VERSIONS.contains(v)).unwrap_or(PROTOCOL_VERSIONS[0]);
            result(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": SERVER_NAME, "title": "radar", "version": env!("CARGO_PKG_VERSION") },
                }),
            )
        }
        "ping" => result(id, json!({})),
        "tools/list" => result(id, json!({ "tools": tool_specs() })),
        "tools/call" => match params.get("name").and_then(Value::as_str) {
            Some("echo") => match params.pointer("/arguments/text").and_then(Value::as_str) {
                Some(text) => result(id, json!({ "content": [{ "type": "text", "text": text }], "isError": false })),
                // A tool's own failure is a result the model reads, not a protocol error.
                None => result(id, json!({ "content": [{ "type": "text", "text": "echo needs `text`, a string" }], "isError": true })),
            },
            Some(other) => error(id, -32602, &format!("unknown tool `{other}`")),
            None => error(id, -32602, "tools/call needs `name`"),
        },
        _ => error(id, -32601, &format!("method `{method}` is not offered")),
    })
}

/// A line of at most [`MAX_LINE`] bytes, without its newline. None at the end of input;
/// an error for a longer line.
async fn read_line<R: AsyncBufRead + Unpin>(r: &mut R, buf: &mut Vec<u8>) -> io::Result<Option<()>> {
    buf.clear();
    let n = (&mut *r).take(MAX_LINE as u64 + 1).read_until(b'\n', buf).await?;
    if n == 0 {
        return Ok(None);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
    } else if buf.len() > MAX_LINE {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "line too long"));
    }
    Ok(Some(()))
}

/// Equal in time that does not depend on where the two differ.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn serve(stream: UnixStream, ticket: String, session: RadarSessionId, seen: mpsc::UnboundedSender<ToolCallSeen>) -> io::Result<()> {
    let (rd, mut wr) = stream.into_split();
    let mut rd = BufReader::new(rd);
    let mut line = Vec::new();
    if read_line(&mut rd, &mut line).await?.is_none() {
        return Ok(());
    }
    let first: Value = serde_json::from_slice(&line).unwrap_or(Value::Null);
    if !first.get("radar_ticket").and_then(Value::as_str).is_some_and(|t| same(t, &ticket)) {
        return Ok(());
    }
    while read_line(&mut rd, &mut line).await?.is_some() {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let reply = match serde_json::from_slice::<Value>(&line) {
            Ok(msg) => {
                if msg.get("method").and_then(Value::as_str) == Some("tools/call") && msg.get("id").is_some() {
                    let _ = seen.send(ToolCallSeen {
                        session: session.clone(),
                        tool: msg.pointer("/params/name").and_then(Value::as_str).unwrap_or_default().to_string(),
                        arguments: msg.pointer("/params/arguments").cloned().unwrap_or(Value::Null),
                    });
                }
                answer(&msg)
            }
            Err(_) => Some(error(&Value::Null, -32700, "not JSON")),
        };
        if let Some(reply) = reply {
            let mut out = serde_json::to_vec(&reply).map_err(io::Error::other)?;
            out.push(b'\n');
            wr.write_all(&out).await?;
        }
    }
    Ok(())
}

async fn accept(listener: UnixListener, owner: u32, ticket: String, session: RadarSessionId, seen: mpsc::UnboundedSender<ToolCallSeen>) {
    // Owned here, so dropping this task (the listener's drop aborts it) ends every connection.
    let mut conns = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else { continue };
                // The socket's mode already keeps other users out; this is the second check.
                if stream.peer_cred().map(|c| c.uid()).ok() != Some(owner) {
                    continue;
                }
                conns.spawn(serve(stream, ticket.clone(), session.clone(), seen.clone()));
            }
            Some(_) = conns.join_next(), if !conns.is_empty() => {}
        }
    }
}

/// A random ticket, 128 bits as hex.
pub fn new_ticket() -> io::Result<String> {
    let mut bytes = [0u8; 16];
    std::io::Read::read_exact(&mut std::fs::File::open("/dev/urandom")?, &mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// One session's listener. Dropping it closes every connection and removes the socket.
pub struct McpListener {
    path: PathBuf,
    ticket: String,
    task: JoinHandle<()>,
}

impl McpListener {
    /// Listens on `path` (mode 0600; a missing folder is made 0700) for `session`, with a new
    /// ticket. Each `tools/call` is sent to `seen`. Needs a tokio runtime.
    pub fn bind(path: impl Into<PathBuf>, session: RadarSessionId, seen: mpsc::UnboundedSender<ToolCallSeen>) -> io::Result<Self> {
        let path = path.into();
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty() && !d.exists()) {
            std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        }
        // A socket left by a radar that died; anything else at the path is not ours to remove.
        if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_socket()) {
            std::fs::remove_file(&path)?;
        }
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        let owner = std::fs::metadata(&path)?.uid();
        let ticket = new_ticket()?;
        let task = tokio::spawn(accept(listener, owner, ticket.clone(), session, seen));
        Ok(Self { path, ticket, task })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn ticket(&self) -> &str {
        &self.ticket
    }

    /// The server a session is set up with: `radar mcp --socket <path> --ticket <t>`, with `radar`
    /// the running binary (`std::env::current_exe`).
    pub fn tool_server(&self, radar: &Path) -> ToolServer {
        ToolServer {
            name: SERVER_NAME.into(),
            command: radar.display().to_string(),
            args: vec![
                "mcp".into(),
                "--socket".into(),
                self.path.display().to_string(),
                "--ticket".into(),
                self.ticket.clone(),
            ],
            tools: TOOLS.iter().map(|t| t.to_string()).collect(),
        }
    }
}

impl Drop for McpListener {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_file(&self.path);
    }
}

/// The relay: connects to `socket`, sends the ticket, then copies `input` to the socket and
/// the socket to `output`, until the socket closes (or `input` ends and then the socket).
pub async fn relay<R, W>(socket: &Path, ticket: &str, mut input: R, mut output: W) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let stream = UnixStream::connect(socket).await?;
    let (mut rd, mut wr) = stream.into_split();
    let mut hello = serde_json::to_vec(&json!({ "radar_ticket": ticket })).map_err(io::Error::other)?;
    hello.push(b'\n');
    wr.write_all(&hello).await?;
    {
        let up = async {
            tokio::io::copy(&mut input, &mut wr).await?;
            wr.shutdown().await
        };
        let down = tokio::io::copy(&mut rd, &mut output);
        tokio::pin!(up, down);
        tokio::select! {
            r = &mut down => { r?; }
            r = &mut up => {
                // The listener closing (a wrong ticket, a line over the limit) ends the relay
                // however it is seen first: as the end of what it sends, or as a write it refuses.
                match r {
                    Err(e) if matches!(e.kind(), io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset) => {}
                    r => r?,
                }
                (&mut down).await?;
            }
        }
    }
    output.flush().await
}

const USAGE: &str = "usage: radar mcp --socket <path> --ticket <ticket>\n\
radar's MCP server for one agent session, on stdin and stdout. Started by the agent's CLI.";

/// `radar mcp …`: `args` are what follows `mcp`. Returns the exit code; the caller exits with
/// it (a blocked read of stdin would otherwise hold the runtime open).
pub async fn relay_main(args: impl IntoIterator<Item = String>) -> i32 {
    let (mut socket, mut ticket) = (None, None);
    let mut args = args.into_iter();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--socket" => socket = args.next(),
            "--ticket" => ticket = args.next(),
            "-h" | "--help" => {
                println!("{USAGE}");
                return 0;
            }
            _ => {
                eprintln!("{USAGE}");
                return 2;
            }
        }
    }
    let (Some(socket), Some(ticket)) = (socket, ticket) else {
        eprintln!("{USAGE}");
        return 2;
    };
    match relay(Path::new(&socket), &ticket, tokio::io::stdin(), tokio::io::stdout()).await {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("radar mcp: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn scratch() -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!("radar-mcp-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn call(id: u64, method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
    }

    #[test]
    fn initialize_agrees_a_revision_and_says_tools() {
        let r = answer(&call(1, "initialize", json!({ "protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": { "name": "t", "version": "1" } }))).unwrap();
        assert_eq!(r["id"], 1);
        assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(r["result"]["capabilities"], json!({ "tools": { "listChanged": false } }));
        assert_eq!(r["result"]["serverInfo"]["name"], "radar");
        let newest = answer(&call(2, "initialize", json!({ "protocolVersion": "2099-01-01" }))).unwrap();
        assert_eq!(newest["result"]["protocolVersion"], PROTOCOL_VERSIONS[0]);
    }

    #[test]
    fn echo_is_listed_and_returns_its_text() {
        let list = answer(&call(1, "tools/list", json!({}))).unwrap();
        let tools = list["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.iter().map(|t| t["name"].as_str().unwrap()).collect::<Vec<_>>(), TOOLS);
        assert_eq!(tools[0]["inputSchema"]["required"], json!(["text"]));
        let r = answer(&call(2, "tools/call", json!({ "name": "echo", "arguments": { "text": "ping" } }))).unwrap();
        assert_eq!(r["result"], json!({ "content": [{ "type": "text", "text": "ping" }], "isError": false }));
        let bad = answer(&call(3, "tools/call", json!({ "name": "echo", "arguments": {} }))).unwrap();
        assert_eq!(bad["result"]["isError"], true);
    }

    #[test]
    fn unknown_methods_and_tools_are_errors_and_notifications_are_not_answered() {
        assert_eq!(answer(&call(1, "resources/list", json!({}))).unwrap()["error"]["code"], -32601);
        assert_eq!(answer(&call(2, "tools/call", json!({ "name": "rm" }))).unwrap()["error"]["code"], -32602);
        assert_eq!(answer(&call(3, "ping", Value::Null)).unwrap()["result"], json!({}));
        assert_eq!(answer(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })), None);
        assert_eq!(answer(&json!({ "jsonrpc": "2.0", "id": 9, "result": {} })), None);
    }

    /// The relay over in-memory stdin and stdout, against a real listener.
    struct Client {
        stdin: tokio::io::DuplexStream,
        stdout: BufReader<tokio::io::DuplexStream>,
        relay: JoinHandle<io::Result<()>>,
    }

    impl Client {
        fn start(path: &Path, ticket: &str) -> Self {
            let (stdin, relay_in) = tokio::io::duplex(1 << 16);
            let (relay_out, stdout) = tokio::io::duplex(1 << 16);
            let (path, ticket) = (path.to_path_buf(), ticket.to_string());
            let relay = tokio::spawn(async move { relay(&path, &ticket, relay_in, relay_out).await });
            Self { stdin, stdout: BufReader::new(stdout), relay }
        }

        async fn send(&mut self, msg: Value) {
            self.stdin.write_all(format!("{msg}\n").as_bytes()).await.unwrap();
        }

        async fn recv(&mut self) -> Option<Value> {
            let mut line = String::new();
            (self.stdout.read_line(&mut line).await.unwrap() > 0).then(|| serde_json::from_str(&line).unwrap())
        }
    }

    #[tokio::test]
    async fn a_session_round_trips_through_the_relay_and_its_calls_are_seen() {
        let dir = scratch();
        let (tx, mut seen) = mpsc::unbounded_channel();
        let l = McpListener::bind(dir.join("s1.sock"), RadarSessionId::new("s1"), tx).unwrap();
        assert_eq!(std::fs::metadata(l.path()).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);

        let mut c = Client::start(l.path(), l.ticket());
        c.send(call(1, "initialize", json!({ "protocolVersion": "2025-06-18" }))).await;
        assert_eq!(c.recv().await.unwrap()["result"]["serverInfo"]["name"], "radar");
        c.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).await;
        c.send(call(2, "tools/list", json!({}))).await;
        assert_eq!(c.recv().await.unwrap()["result"]["tools"][0]["name"], "echo");
        c.send(call(3, "tools/call", json!({ "name": "echo", "arguments": { "text": "ping" } }))).await;
        assert_eq!(c.recv().await.unwrap()["result"]["content"][0]["text"], "ping");
        assert_eq!(
            seen.recv().await.unwrap(),
            ToolCallSeen { session: RadarSessionId::new("s1"), tool: "echo".into(), arguments: json!({ "text": "ping" }) }
        );
        // The CLI closing its end ends the relay cleanly.
        drop(c.stdin);
        assert!(c.relay.await.unwrap().is_ok());
        assert_eq!(c.stdout.read_line(&mut String::new()).await.unwrap(), 0);

        let path = l.path().to_path_buf();
        drop(l);
        assert!(!path.exists(), "the socket goes with its listener");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_wrong_ticket_gets_no_answer_and_reaches_nothing() {
        let dir = scratch();
        let (tx, mut seen) = mpsc::unbounded_channel();
        let l = McpListener::bind(dir.join("s2.sock"), RadarSessionId::new("s2"), tx).unwrap();
        let mut c = Client::start(l.path(), "not-the-ticket");
        // The relay may already have stopped (the listener closes at the ticket): a failed write is fine.
        let msg = call(1, "tools/call", json!({ "name": "echo", "arguments": { "text": "x" } }));
        let _ = c.stdin.write_all(format!("{msg}\n").as_bytes()).await;
        // The listener closes the connection; the relay sees the end and stops.
        assert_eq!(c.recv().await, None);
        assert!(c.relay.await.unwrap().is_ok());
        drop(l);
        assert!(seen.recv().await.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_listener_that_closes_while_the_cli_still_writes_ends_the_relay_cleanly() {
        // A server that reads the hello and hangs up, as the listener does at a wrong ticket;
        // the CLI's line is already waiting, so the relay may meet the close as a refused write.
        let dir = scratch();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s5.sock");
        let server = UnixListener::bind(&path).unwrap();
        let mut c = Client::start(&path, "not-the-ticket");
        c.send(call(1, "tools/call", json!({ "name": "echo", "arguments": { "text": "x" } }))).await;
        let (conn, _) = server.accept().await.unwrap();
        let mut conn = BufReader::new(conn);
        conn.read_line(&mut String::new()).await.unwrap();
        drop(conn);
        c.send(call(2, "ping", Value::Null)).await;
        assert_eq!(c.recv().await, None);
        assert!(c.relay.await.unwrap().is_ok(), "a closed socket is the end, not a failure");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_line_over_the_limit_closes_the_connection() {
        let dir = scratch();
        let (tx, _seen) = mpsc::unbounded_channel();
        let l = McpListener::bind(dir.join("s3.sock"), RadarSessionId::new("s3"), tx).unwrap();
        let mut c = Client::start(l.path(), l.ticket());
        let long = format!("{{\"x\":\"{}\"}}\n", "a".repeat(MAX_LINE + 1));
        // Cut off part way: the write may fail once the relay has stopped.
        let _ = c.stdin.write_all(long.as_bytes()).await;
        assert_eq!(c.recv().await, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_server_entry_runs_radar_mcp_with_the_socket_and_ticket() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let _g = rt.enter();
        let dir = scratch();
        let (tx, _seen) = mpsc::unbounded_channel();
        let l = McpListener::bind(dir.join("s4.sock"), RadarSessionId::new("s4"), tx).unwrap();
        let s = l.tool_server(Path::new("/opt/radar"));
        assert_eq!(s.name, "radar");
        assert_eq!(s.command, "/opt/radar");
        assert_eq!(s.args, vec!["mcp".to_string(), "--socket".into(), l.path().display().to_string(), "--ticket".into(), l.ticket().into()]);
        assert_eq!(s.tools, vec!["echo".to_string()]);
        assert_eq!(l.ticket().len(), 32);
        drop(l);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
