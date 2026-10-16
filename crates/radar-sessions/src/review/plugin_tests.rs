//! The review journey through the plugin, the way the host drives it: ticks,
//! the questions it asks of other parts, and dock intents. The fake provider
//! stands in for Copilot; nothing waits on a clock.

use std::path::PathBuf;

use radar_core::agent_context::{ChangeResolution, DiffStats, Resolve, WorktreeError, WorktreeReady};
use radar_core::dock::{DockIntent, DockItem, PermissionAction};
use radar_core::journal::SubjectKey;
use radar_core::plugin::{ReviewAnswer, ReviewAsk};
use radar_core::{Effect, Outcome, Plugin};

use crate::SessionsPlugin;
use crate::fake::{FakeProvider, Step};

fn key() -> SubjectKey {
    SubjectKey::new("sample", "https://one.example.test", "merge_request", "acme/api!263").unwrap()
}

fn resolution() -> ChangeResolution {
    ChangeResolution {
        subject: key(),
        title: "Fix the retry loop".into(),
        link: None,
        repository: "acme/api".into(),
        source_branch: "fix/retry".into(),
        target_branch: "main".into(),
        head_sha: "abc1234567890".into(),
        diff_stats: DiffStats { files: 1, additions: 2, deletions: 1 },
        description: "retry".into(),
        threads: vec![],
    }
}

/// What the other parts answer, and what the host showed.
struct World {
    resolve: Resolve,
    worktree: Result<WorktreeReady, WorktreeError>,
    notices: Vec<String>,
    asked: Vec<String>,
    now: i64,
}

impl World {
    fn ok(dir: &std::path::Path) -> World {
        World {
            resolve: Resolve::Ready(resolution()),
            worktree: Ok(WorktreeReady { path: dir.to_path_buf(), reused: false }),
            notices: vec![],
            asked: vec![],
            now: 1_800_000_000,
        }
    }

    fn take(&mut self, fx: Vec<Effect>) {
        for e in fx {
            if let Effect::Notify(m) = e {
                self.notices.push(m.to_string());
            }
        }
    }

    /// One host tick: the part ticks, then each question is answered.
    fn step(&mut self, p: &mut SessionsPlugin) {
        self.now += 1;
        let fx = p.tick(self.now);
        self.take(fx);
        for (run, ask) in p.review_asks() {
            let answer = match ask {
                ReviewAsk::Resolve(_) => {
                    self.asked.push("resolve".into());
                    ReviewAnswer::Resolved(self.resolve.clone())
                }
                ReviewAsk::Worktree(w) => {
                    self.asked.push(format!("worktree {} {}", w.branch, w.sha));
                    ReviewAnswer::Worktree(self.worktree.clone())
                }
                ReviewAsk::SaveNote(n) => {
                    self.asked.push(format!("note {}", n.file_name));
                    ReviewAnswer::NoteSaved(None)
                }
            };
            if let Outcome::Handled(fx) = p.review_answer(run, answer) {
                self.take(fx);
            }
        }
    }

    fn until(&mut self, p: &mut SessionsPlugin, what: &str, done: impl Fn(&World, &SessionsPlugin) -> bool) {
        // The plugin's tasks run on this thread, only here: a bound on steps, not on time.
        for _ in 0..256u64 {
            self.step(p);
            if done(self, p) {
                return;
            }
            p.run_tasks();
        }
        panic!("never reached: {what}; notices {:?}; asked {:?}; dock {:?}", self.notices, self.asked, p.dock());
    }

    fn said(&self, text: &str) -> bool {
        self.notices.iter().any(|n| n.contains(text))
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("radar-review-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn plugin(provider: &FakeProvider, notes: &std::path::Path) -> SessionsPlugin {
    let mut p = SessionsPlugin::on_this_thread().with_review_dir(notes.to_path_buf());
    p.add_adapter(Box::new(provider.adapter()));
    p
}

fn begin(p: &mut SessionsPlugin, w: &mut World) {
    // The host has ticked before anyone can ask: the part knows the time.
    let fx = p.tick(w.now);
    w.take(fx);
    let Outcome::Handled(fx) = p.review_start(None, key()) else { panic!("not handled") };
    w.take(fx);
}

fn permission(p: &SessionsPlugin) -> Option<(String, Option<PermissionAction>)> {
    p.dock()?.transcript.iter().find_map(|i| match i {
        DockItem::Permission(r) if r.answered.is_none() && r.target == "cargo test" => Some((r.id.clone(), r.answered)),
        _ => None,
    })
}

fn notes(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|d| d.flatten().map(|e| std::fs::read_to_string(e.path()).unwrap()).collect())
        .unwrap_or_default()
}

#[test]
fn a_review_runs_in_the_worktree_asks_before_running_code_and_is_saved_as_a_local_note() {
    let (wt, saved) = (scratch("wt"), scratch("notes"));
    let provider = FakeProvider::new();
    provider.script(vec![
        Step::Say("Verdict: request changes.".into()),
        // Reads and git diff run without asking.
        Step::Shell("git diff main...HEAD".into()),
        // Running the change's code waits for the operator.
        Step::Shell("cargo test".into()),
        Step::Say("- src/retry.rs:12 [major] the loop never ends.".into()),
    ]);
    let mut p = plugin(&provider, &saved);
    let mut w = World::ok(&wt);
    begin(&mut p, &mut w);
    w.until(&mut p, "a request for cargo test", |_, p| permission(p).is_some());
    // The git diff before it was answered by the policy; this one is the operator's.
    let (id, _) = permission(&p).unwrap();
    for _ in 0..100 {
        w.step(&mut p);
    }
    assert_eq!(permission(&p).map(|r| r.0), Some(id.clone()), "nothing answers cargo test but the operator");
    let session = p.active().cloned().unwrap();
    assert_eq!(p.store().get(&session).unwrap().meta.cwd, wt.display().to_string());
    assert!(w.asked.contains(&"worktree fix/retry abc1234567890".to_string()));
    assert!(notes(&saved).is_empty(), "nothing is saved before the review ends");

    p.dock_intent(&DockIntent::Approve { id });
    w.until(&mut p, "the note", |w, _| w.said("saved at"));
    let all = notes(&saved);
    assert_eq!(all.len(), 1);
    let note = &all[0];
    assert!(note.contains("reviewed_sha: \"abc1234567890\""), "{note}");
    assert!(note.contains("recipe: \"review-mr\""), "{note}");
    assert!(note.contains("posted: false"), "{note}");
    assert!(note.contains("Verdict: request changes.") && note.contains("src/retry.rs:12"), "{note}");
    assert!(w.said("nothing was posted"));
    // The row's word, and what a moved head does to it.
    assert_eq!(p.review_chip("acme/api!263", Some("abc1234567890")).as_deref(), Some("reviewed abc1234"));
    assert_eq!(p.review_chip("acme/api!263", Some("fff0000")).as_deref(), Some("review stale (abc1234)"));
    // Its x menu opens the note, or copies where it is.
    let mr = radar_core::Subject::new("merge-request", "acme/api!263", "api !263");
    let Some(radar_core::Contribution::Actions(rows)) = p.contribute(radar_core::Point::Actions, &mr) else {
        panic!("no actions on a reviewed merge request")
    };
    let path = std::fs::read_dir(&saved).unwrap().flatten().next().unwrap().path().display().to_string();
    let open = rows.iter().find(|r| r.what == "open the local review").unwrap();
    let url = format!("file://{path}");
    assert!(matches!(&p.act(&mr, &open.id)[..], [Effect::Open(u)] if *u == url));
    let copy = rows.iter().find(|r| r.what.starts_with("copy")).unwrap();
    assert!(matches!(&p.act(&mr, &copy.id)[..], [Effect::Copy(c)] if *c == path));
    let other = radar_core::Subject::new("merge-request", "acme/api!9", "api !9");
    assert!(p.contribute(radar_core::Point::Actions, &other).is_none(), "nothing on one never reviewed");
}

#[test]
fn a_change_request_that_is_gone_ends_the_review_with_the_reason_and_starts_no_session() {
    let (wt, saved) = (scratch("gone-wt"), scratch("gone-notes"));
    let mut p = plugin(&FakeProvider::new(), &saved);
    let mut w = World::ok(&wt);
    w.resolve = Resolve::Failed("the merge request was not found (deleted?)".into());
    begin(&mut p, &mut w);
    w.until(&mut p, "the failure", |w, _| w.said("not found"));
    assert!(p.dock().unwrap().tabs.is_empty());
    assert!(notes(&saved).is_empty());
}

#[test]
fn a_resolution_that_is_still_pending_is_asked_again() {
    let (wt, saved) = (scratch("pending-wt"), scratch("pending-notes"));
    let mut p = plugin(&FakeProvider::new(), &saved);
    let mut w = World::ok(&wt);
    w.resolve = Resolve::Pending;
    begin(&mut p, &mut w);
    for _ in 0..5 {
        w.step(&mut p);
    }
    assert!(w.asked.iter().filter(|a| *a == "resolve").count() >= 3);
    assert!(w.asked.iter().all(|a| a == "resolve"), "no worktree before the change request is known");
}

#[test]
fn a_worktree_that_cannot_be_made_ends_the_review_and_starts_no_session() {
    let (wt, saved) = (scratch("nowt-wt"), scratch("nowt-notes"));
    let mut p = plugin(&FakeProvider::new(), &saved);
    let mut w = World::ok(&wt);
    w.worktree = Err(WorktreeError::ShaNotFound);
    begin(&mut p, &mut w);
    w.until(&mut p, "the failure", |w, _| w.said("head commit is not on the remote branch"));
    assert!(p.dock().unwrap().tabs.is_empty());
}

#[test]
fn a_provider_error_ends_the_review_and_saves_nothing() {
    let (wt, saved) = (scratch("err-wt"), scratch("err-notes"));
    let provider = FakeProvider::new();
    provider.script(vec![Step::Say("Looking.".into()), Step::Fail("rate limited".into())]);
    let mut p = plugin(&provider, &saved);
    let mut w = World::ok(&wt);
    begin(&mut p, &mut w);
    w.until(&mut p, "the failure", |w, _| w.said("the reviewer failed: rate limited"));
    for _ in 0..50 {
        w.step(&mut p);
    }
    assert!(notes(&saved).is_empty());
}

#[test]
fn stopping_the_turn_ends_the_review_and_saves_nothing() {
    let (wt, saved) = (scratch("stop-wt"), scratch("stop-notes"));
    let provider = FakeProvider::new();
    provider.script(vec![Step::Say("Working on it".into()), Step::Hang]);
    let mut p = plugin(&provider, &saved);
    let mut w = World::ok(&wt);
    begin(&mut p, &mut w);
    w.until(&mut p, "the reviewer working", |_, p| {
        p.dock().is_some_and(|d| d.transcript.iter().any(|i| matches!(i, DockItem::Assistant { text, .. } if text.contains("Working on it"))))
    });
    p.dock_intent(&DockIntent::Stop);
    w.until(&mut p, "the stop", |w, _| w.said("stopped; nothing was saved"));
    for _ in 0..50 {
        w.step(&mut p);
    }
    assert!(notes(&saved).is_empty());
}

#[test]
fn the_same_change_request_is_not_reviewed_twice_at_once() {
    let (wt, saved) = (scratch("dup-wt"), scratch("dup-notes"));
    let mut p = plugin(&FakeProvider::new(), &saved);
    let mut w = World::ok(&wt);
    w.resolve = Resolve::Pending;
    begin(&mut p, &mut w);
    begin(&mut p, &mut w);
    assert!(w.said("already running"));
}

#[test]
fn saved_reviews_are_read_back_after_a_restart() {
    let (wt, saved) = (scratch("again-wt"), scratch("again-notes"));
    let provider = FakeProvider::new();
    provider.script(vec![Step::Say("Verdict: approve.".into())]);
    let mut p = plugin(&provider, &saved);
    let mut w = World::ok(&wt);
    begin(&mut p, &mut w);
    w.until(&mut p, "the note", |w, _| w.said("saved at"));
    let again = plugin(&FakeProvider::new(), &saved);
    assert_eq!(again.review_chip("acme/api!263", Some("abc1234567890")).as_deref(), Some("reviewed abc1234"));
}

#[test]
fn the_reviewers_tab_keeps_the_policy_after_the_review_is_saved() {
    let (wt, saved) = (scratch("after-wt"), scratch("after-notes"));
    let provider = FakeProvider::new();
    provider.script(vec![Step::Say("Verdict: approve.".into())]);
    let mut p = plugin(&provider, &saved);
    let mut w = World::ok(&wt);
    begin(&mut p, &mut w);
    w.until(&mut p, "the note", |w, _| w.said("saved at"));
    // A follow-up in the same tab: a git read is still answered by the policy, not left to the dock.
    provider.script(vec![Step::Shell("git log --oneline".into()), Step::Say("Done.".into())]);
    p.dock_intent(&DockIntent::Send { text: "one more thing".into() });
    w.until(&mut p, "the follow-up answered", |_, p| {
        p.dock().is_some_and(|d| d.transcript.iter().any(|i| matches!(i, DockItem::Assistant { text, .. } if text == "Done.")))
    });
    assert!(permission(&p).is_none(), "no approval box for a read the policy allows");
}
