//! The sample the git part's tests use, and `radar shot` draws with
//! (`Plugin::demo`): one folder looked in, with a checkout of
//! `acme/team-a/api` on `main`. Neutral names; nothing on this machine is
//! read. Compiled for tests and under the `demo` feature.

use crate::GitPlugin;
use crate::local::{Checkout, Index};
use crate::screen::{Branch, Detail, Tree};

pub fn sample() -> GitPlugin {
    let mut p = GitPlugin {
        now: radar_core::plugin::DEMO_NOW,
        ..GitPlugin::default()
    };
    // api is checked out on this machine, and was looked for already.
    p.cfg.roots = vec!["/work".into()];
    p.local.started = true;
    p.local.index = Some(Index {
        checkouts: vec![Checkout {
            dir: "/work/acme/api".into(),
            remote: "git@example.com:acme/team-a/api.git".into(),
            project: "acme/team-a/api".into(),
            branch: "main".into(),
            files: vec!["build.gradle".into(), "src".into()],
            main: String::new(),
        }],
        at: p.now - 120,
        folders: 40,
        ..Default::default()
    });
    p
}

fn branch(
    name: &str,
    upstream: &str,
    (ahead, behind): (usize, usize),
    age: i64,
    now: i64,
) -> Branch {
    Branch {
        name: name.into(),
        upstream: upstream.into(),
        ahead,
        behind,
        updated: now - age,
        sha: format!("{:07x}", age * 7919 % 0xfff_ffff),
        subject: format!("work on {name}"),
        unpushed: if upstream.is_empty() { 1 } else { ahead },
        ..Branch::default()
    }
}

/// The sample with what the Branches screen shows: two clones, `api` and
/// `web`, and a worktree of `api`. `api` is on a ticket branch with changes,
/// two commits ahead, and a stash; its worktree holds another; one branch is
/// merged and one is gone from the remote. Nothing is read from this machine.
#[allow(dead_code)]
pub fn branches() -> GitPlugin {
    let mut p = sample();
    let now = p.now;
    let tree = |dir: &str, branch: &str, dirty, ahead, behind| Tree {
        dir: dir.into(),
        branch: branch.into(),
        dirty,
        ahead,
        behind,
        ..Tree::default()
    };
    let index = p.local.index.as_mut().expect("the sample has an index");
    index.checkouts[0].branch = "alice/ABC-12".into();
    index.checkouts.push(Checkout {
        dir: "/work/acme/api-hotfix".into(),
        remote: "git@example.com:acme/team-a/api.git".into(),
        project: "acme/team-a/api".into(),
        branch: "alice/ABC-14".into(),
        files: vec!["build.gradle".into()],
        main: "/work/acme/api".into(),
    });
    index.checkouts.push(Checkout {
        dir: "/work/acme/web".into(),
        remote: "git@example.com:acme/team-a/web.git".into(),
        project: "acme/team-a/web".into(),
        branch: "main".into(),
        files: vec!["package.json".into()],
        main: String::new(),
    });
    let mut main = branch("main", "origin/main", (0, 4), 86_400 * 2, now);
    main.merged = true;
    let mut done = branch("alice/ABC-9", "origin/alice/ABC-9", (0, 0), 86_400 * 6, now);
    done.merged = true;
    let mut old = branch(
        "alice/ABC-7",
        "origin/alice/ABC-7",
        (0, 0),
        86_400 * 20,
        now,
    );
    old.gone = true;
    p.branches.details.insert(
        "/work/acme/api".into(),
        Detail {
            dir: "/work/acme/api".into(),
            base: "main".into(),
            trees: vec![
                tree("/work/acme/api", "alice/ABC-12", 3, 2, 0),
                tree("/work/acme/api-hotfix", "alice/ABC-14", 0, 1, 3),
            ],
            branches: vec![
                main,
                branch(
                    "alice/ABC-12",
                    "origin/alice/ABC-12",
                    (2, 0),
                    3_600 * 3,
                    now,
                ),
                branch("alice/ABC-14", "origin/alice/ABC-14", (1, 3), 86_400, now),
                done,
                old,
                branch("spike", "", (0, 0), 86_400 * 9, now),
            ],
            stashes: vec!["On alice/ABC-12: ABC-12-local-2026-10-05".into()],
            error: String::new(),
        },
    );
    let mut web_main = branch("main", "origin/main", (0, 0), 86_400 * 4, now);
    web_main.merged = true;
    p.branches.details.insert(
        "/work/acme/web".into(),
        Detail {
            dir: "/work/acme/web".into(),
            base: "main".into(),
            trees: vec![tree("/work/acme/web", "main", 0, 0, 0)],
            branches: vec![web_main],
            stashes: vec![],
            error: String::new(),
        },
    );
    p.branches.read_at = now - 20;
    p.branches.last_ok = Some(now - 20);
    p
}
