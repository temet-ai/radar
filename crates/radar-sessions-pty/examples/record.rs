//! Records what a program writes to its terminal, for the Ink renderer's tests.
//!
//!     cargo run -p radar-sessions-pty --example record -- \
//!         --size 80x24 --out screen.json [--send 'q'] [--after-ms 600] [--wait-ms 1500] -- less -R file
//!
//! The file is JSON: the command, the size and the raw bytes in base64, exactly as the host's ring would replay
//! them.

use std::time::Duration;

use base64::Engine;
use radar_sessions_pty::{Terminal, TerminalSpec};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let (mut size, mut out, mut send, mut after, mut wait) = ((80u16, 24u16), String::new(), None, 600u64, 1500u64);
    let mut command = Vec::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--size" => {
                let v = args.next().unwrap_or_default();
                let (c, r) = v.split_once('x').unwrap_or(("80", "24"));
                size = (c.parse()?, r.parse()?);
            }
            "--out" => out = args.next().unwrap_or_default(),
            "--send" => send = args.next(),
            "--after-ms" => after = args.next().unwrap_or_default().parse()?,
            "--wait-ms" => wait = args.next().unwrap_or_default().parse()?,
            "--" => {
                command = args.by_ref().collect();
                break;
            }
            other => anyhow::bail!("unknown argument {other}"),
        }
    }
    anyhow::ensure!(!out.is_empty() && !command.is_empty(), "need --out and a command after --");
    let spec = TerminalSpec::new(command[0].clone(), std::env::current_dir()?).args(command[1..].to_vec()).size(size.0, size.1);
    let term = Terminal::spawn(spec)?;
    std::thread::sleep(Duration::from_millis(after));
    if let Some(s) = send {
        term.write(s.replace("\\r", "\r").replace("\\n", "\n").as_bytes())?;
    }
    term.wait_exit(Duration::from_millis(wait));
    let bytes = term.replay();
    let doc = serde_json::json!({
        "command": command,
        "columns": size.0,
        "rows": size.1,
        "state": term.frame(),
        "base64": base64::engine::general_purpose::STANDARD.encode(&bytes),
    });
    std::fs::write(&out, serde_json::to_string_pretty(&doc)? + "\n")?;
    eprintln!("{} bytes, {:?}", bytes.len(), term.state());
    term.kill();
    Ok(())
}
