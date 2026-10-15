//! The sample the workspace part's tests use, and `radar shot` draws with
//! (`Plugin::demo`): one workspace, `acme`, with its clones and the notes of
//! a ticket indexed, and a second workspace found on this machine. Neutral
//! names; nothing on this machine is read. Compiled for tests and under the
//! `demo` feature.

use crate::config::Space;
use crate::find::Offer;
use crate::notes::{Index, Note};
use crate::{State, WorkspacePlugin};

pub const KEY: &str = "ABC-12";

fn note(rel: &str, kind: &str, status: &str, title: &str, readme: bool, lead: &[&str]) -> Note {
    let name = rel.rsplit('/').next().unwrap_or(rel).to_string();
    Note {
        key: KEY.into(),
        path: format!("/work/acme/{rel}"),
        rel: rel.into(),
        name,
        folder: "artifacts/2026-01-01/ABC-12-stats".into(),
        title: title.into(),
        kind: kind.into(),
        status: status.into(),
        date: "2026-01-01".into(),
        readme,
        lead: lead.iter().map(|l| (*l).to_string()).collect(),
    }
}

pub fn sample() -> WorkspacePlugin {
    let mut p = WorkspacePlugin {
        now: radar_core::plugin::DEMO_NOW,
        ..WorkspacePlugin::default()
    };
    let mut acme = Space::new("/work/acme");
    acme.tickets = vec!["ABC".into()];
    acme.after_worktree = "scripts/sync.sh".into();
    p.cfg.spaces = vec![acme];
    let folder = "artifacts/2026-01-01/ABC-12-stats";
    let mut state = State::default();
    state.indexes.insert(
        "/work/acme".into(),
        Index {
            at: p.now - 120,
            okf: true,
            roots: vec!["artifacts".into(), "docs".into()],
            files: 212,
            clones: 14,
            create: "artifacts/{date}".into(),
            notes: vec![
                note(
                    &format!("{folder}/README.md"),
                    "ticket",
                    "planning",
                    "ABC-12: Add stats",
                    true,
                    &[
                        "# ABC-12: Add stats",
                        "Ticket: a story. Planned from 2026-01-01.",
                        "## What we know",
                    ],
                ),
                note(
                    &format!("{folder}/spec-ui.md"),
                    "spec",
                    "approved",
                    "UI",
                    false,
                    &[],
                ),
                note(
                    &format!("{folder}/spec-backend.md"),
                    "spec",
                    "draft",
                    "Backend",
                    false,
                    &[],
                ),
            ],
            ..Index::default()
        },
    );
    state.offers = Some(vec![Offer {
        dir: "/work/other".into(),
        name: "other".into(),
        clones: 3,
        repos: String::new(),
    }]);
    p.state = state;
    p
}
