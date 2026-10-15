//! The shared contract kit, as far as a terminal goes: it declares what it lacks, reports a version,
//! never carries a token, and subscribes once. The kit skips the structured checks for an adapter
//! without structured events and says so.

use std::path::PathBuf;

use radar_sessions::contract::{run_all, Harness, Scenario};
use radar_sessions::{Adapter, BoxFuture, ProviderId};
use radar_sessions_pty::{PtyAdapter, PtyCommand};

/// A stand-in CLI: answers `--version`, otherwise reads a line and exits. Written here so the test
/// does not depend on which tools a machine has (BSD tools have no `--version`).
struct Harnessed {
    dir: PathBuf,
}

impl Harnessed {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("radar-pty-contract-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("fake-cli");
        std::fs::write(&script, "#!/bin/sh\n[ \"$1\" = --version ] && { echo 'fake-cli 1.2.3'; exit 0; }\nread line\n").unwrap();
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        Self { dir }
    }
    fn make(&self) -> Box<dyn Adapter> {
        let cmd = PtyCommand { id: ProviderId::new("pty:fake-cli"), command: self.dir.join("fake-cli").display().to_string(), args: vec![] };
        Box::new(PtyAdapter::new(cmd, self.dir.join("sockets")))
    }
}

impl Drop for Harnessed {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Harness for Harnessed {
    fn adapter(&mut self, _: Scenario) -> BoxFuture<'_, Box<dyn Adapter>> {
        Box::pin(async move { self.make() })
    }
    fn restart(&mut self) -> BoxFuture<'_, Box<dyn Adapter>> {
        Box::pin(async move { self.make() })
    }
    fn settle(&mut self) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
    fn disconnect(&mut self) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
    fn reconnect(&mut self) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
    fn release(&mut self) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

#[tokio::test]
async fn a_terminal_passes_the_checks_that_apply_to_it() {
    let report = run_all(&mut Harnessed::new()).await;
    assert_eq!(report.ran, vec!["capabilities", "lifecycle"], "{report:?}");
    assert!(report.skipped.iter().any(|(n, _)| *n == "approve"), "{report:?}");
}
