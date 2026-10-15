//! The build's version: the crate's, and the commit it was built from, so
//! `radar --version` and the log say which radar is running.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let dir = std::env::var("CARGO_MANIFEST_DIR").ok()?;
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

fn main() {
    let commit = git(&["rev-parse", "--short=7", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let date = git(&["log", "-1", "--format=%cs"]).unwrap_or_default();
    let version = env!("CARGO_PKG_VERSION");
    let build = if date.is_empty() {
        format!("{version} ({commit})")
    } else {
        format!("{version} ({commit}, {date})")
    };
    println!("cargo:rustc-env=RADAR_BUILD={build}");
    // A new commit is a new build.
    if let Some(head) = git(&["rev-parse", "--git-path", "HEAD"]) {
        println!("cargo:rerun-if-changed={head}");
    }
    if let Some(refs) = git(&["rev-parse", "--git-path", "refs/heads"]) {
        println!("cargo:rerun-if-changed={refs}");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
