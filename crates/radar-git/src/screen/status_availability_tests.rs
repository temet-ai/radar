//! Fake-only read, reducer and real host rendering checks. Spawned reads are never run.
use super::*;
use radar_core::host::{Event, Hit, Host, HostEffect, MouseKind, View as HostView};
use radar_core::ratatui::crossterm::event::KeyModifiers;

const CLONE: &str = "/work/acme/api";
const WORKTREE: &str = "/work/acme/api-hotfix";

fn fixture(failed: bool) -> (Detail, ReadObservation) {
    read_with(CLONE, |path, args| {
        if failed && args[0] == "status" && path == Path::new(WORKTREE) {
            return Err("SECRET_STATUS_ERROR".into());
        }
        Ok(match args[0] {
            "for-each-ref" => "main\torigin/main\t\t10\taaa1111\tmain\nhotfix\torigin/hotfix\t[gone]\t20\tbbb2222\tfix".into(),
            "symbolic-ref" => "origin/main".into(),
            "rev-parse" => "aaa1111".into(),
            "branch" => "main\nhotfix".into(),
            "rev-list" => "2".into(),
            "worktree" => format!("worktree {CLONE}\nHEAD aaa1111\nbranch refs/heads/main\n\nworktree {WORKTREE}\nHEAD bbb2222\nbranch refs/heads/hotfix"),
            "status" if path == Path::new(WORKTREE) => "# branch.ab +2 -1\n? one\n? two".into(),
            "status" | "stash" => String::new(),
            _ => panic!("unexpected fake read: {args:?}"),
        })
    })
}

fn sample() -> GitPlugin {
    let mut p = crate::demo::branches();
    p.local
        .index
        .as_mut()
        .unwrap()
        .checkouts
        .retain(|c| c.dir != "/work/acme/web");
    p.branches.details.clear();
    p.branches.last_ok = None;
    p.branches.read_at = 0;
    p
}

fn reply(failed: bool) -> BranchMsg {
    let (detail, observed) = fixture(failed);
    BranchMsg::Read {
        dir: CLONE.into(),
        detail: Box::new(detail),
        observed,
        action: read_action(CLONE),
    }
}

fn receive(p: &mut GitPlugin, failed: bool) -> Vec<Effect> {
    let fx = refresh(p);
    assert!(fx.iter().any(|fx| matches!(fx, Effect::SpawnAction { .. })));
    on_msg(p, reply(failed))
}

fn select_worktree(p: &GitPlugin) {
    p.branches
        .tree
        .borrow_mut()
        .select(vec![clone_id(CLONE), tree_id(WORKTREE)]);
}

fn press(host: &mut Host, code: KeyCode) -> Vec<HostEffect> {
    host.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
}

// Optional official full-colour evidence: run these tests with radar-core/png
// and RADAR_STATUS_SHOTS pointing to a scratch output folder.
fn export(host: &Host, width: u16, height: u16, name: &str) {
    let Some(dir) = std::env::var_os("RADAR_STATUS_SHOTS") else {
        return;
    };
    let dir = Path::new(&dir);
    std::fs::create_dir_all(dir).unwrap();
    let _ = radar_core::shell::to_svg(host, width, height);
    let svg = radar_core::shell::to_svg(host, width, height);
    let name = format!("{name}-{width}x{height}");
    std::fs::write(dir.join(format!("{name}.svg")), &svg).unwrap();
    std::fs::write(
        dir.join(format!("{name}.png")),
        radar_core::shot::to_png(&svg).unwrap(),
    )
    .unwrap();
}

fn host(p: GitPlugin) -> Host {
    let now = p.now;
    let mut h = Host::new(vec![Box::new(GitPlugin::default())], now).unwrap();
    h.load_config(Some("")).unwrap();
    h.plugins[0] = Box::new(p);
    h.view = HostView::Screen(0);
    h.focus = Pane::Middle;
    h
}

#[test]
fn failed_worktree_status_is_unknown_without_changing_partial_journal_evidence() {
    let mut p = sample();
    let fx = receive(&mut p, true);
    let d = &p.branches.details[CLONE];
    assert_eq!(d.trees[0].status, StatusRead::Current);
    assert_eq!(d.trees[1].status, StatusRead::Unavailable);
    assert_eq!(d.trees[1].dirty, 0, "zero is no longer evidence of clean");
    assert!(
        visible(View::Dirty, Some(d)),
        "unknown status stays discoverable"
    );
    assert_eq!(
        counts(&p)[1],
        0,
        "unknown is not a counted dirty status either"
    );
    assert_eq!(p.branches.last_ok, None);
    assert!(top_right(&p).unwrap().contains("read incomplete"));
    assert!(matches!(fx[0], Effect::Report(JournalOutcome::Partial)));
    assert_eq!(fx.len(), 4, "the existing bounded observations stay intact");
    assert!(!format!("{fx:?}").contains("SECRET_"));
    assert!(
        !lost_tree(&d.trees[1])
            .iter()
            .any(|s| s.contains("nothing uncommitted"))
    );
}

#[test]
fn cached_failed_status_keeps_only_same_revision_last_known_values() {
    for changed_revision in [false, true] {
        let mut p = sample();
        receive(&mut p, false);
        let successful = p.branches.last_ok;
        if changed_revision {
            p.branches.details.get_mut(CLONE).unwrap().trees[1].head = "older".into();
        }
        p.now += 60;
        receive(&mut p, true);
        let tree = &p.branches.details[CLONE].trees[1];
        assert_eq!(
            tree.status,
            if changed_revision {
                StatusRead::Unavailable
            } else {
                StatusRead::LastKnown
            }
        );
        assert_eq!(tree.dirty, if changed_revision { 0 } else { 2 });
        assert_eq!(
            p.branches.last_ok, successful,
            "an attempt cannot advance success freshness"
        );
        assert!(top_right(&p).unwrap().contains("last complete"));
        if !changed_revision {
            p.now += 60;
            receive(&mut p, true);
            assert_eq!(
                p.branches.details[CLONE].trees[1].status,
                StatusRead::LastKnown
            );
            assert_eq!(p.branches.details[CLONE].trees[1].dirty, 2);
        }
    }
}

#[test]
fn unknown_or_stale_dirty_worktree_cannot_enable_forced_removal_or_prune() {
    for cached in [false, true] {
        let mut p = sample();
        if cached {
            receive(&mut p, false);
        }
        receive(&mut p, true);
        select_worktree(&p);
        let Outcome::Handled(fx) = key(
            &mut p,
            Pane::Middle,
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
        ) else {
            panic!("handled refusal")
        };
        assert!(matches!(&fx[..], [Effect::Notify(why)] if why.contains("read the clones again")));
        let question = format!("{DELETE}:tree\t{CLONE}\t{WORKTREE}");
        p.branches.tree.borrow_mut().select(vec![clone_id(CLONE)]);
        let fx = on_menu(&mut p, &question, "yes").unwrap();
        assert!(matches!(
            &fx[..],
            [Effect::Report(JournalOutcome::Blocked), Effect::Notify(_)]
        ));
        assert!(p.branches.changing.is_empty());
        assert!(
            !candidates(&p)
                .iter()
                .any(|c| c.tree == WORKTREE || c.branch == "hotfix")
        );
        assert!(bulk(&p).iter().any(|item| {
            item.note
                && item
                    .label
                    .contains("1 worktree skipped: status unavailable")
        }));
    }
}

#[test]
fn successful_retry_recovers_status_and_freshness_and_duplicate_reads_stay_suppressed() {
    let mut p = sample();
    receive(&mut p, true);
    let fx = refresh_now(&mut p);
    assert_eq!(
        fx.iter()
            .filter(|e| matches!(e, Effect::SpawnAction { .. }))
            .count(),
        1
    );
    let again = refresh_now(&mut p);
    assert!(
        matches!(&again[0], Effect::Observe { action, outcome: JournalOutcome::Blocked } if action.kind.as_str() == "git.branches.read.suppressed")
    );
    assert!(
        !again
            .iter()
            .any(|e| matches!(e, Effect::SpawnAction { .. }))
    );
    p.now += 2;
    on_msg(&mut p, reply(false));
    assert_eq!(
        p.branches.details[CLONE].trees[1].status,
        StatusRead::Current
    );
    assert!(p.branches.incomplete.is_empty() && !busy(&p));
    assert_eq!(p.branches.last_ok, Some(p.now));
    assert_eq!(counts(&p)[1], 1);
    assert!(top_right(&p).unwrap().starts_with("synced"));
    select_worktree(&p);
    let Outcome::Handled(fx) = key(
        &mut p,
        Pane::Middle,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
    ) else {
        panic!("question")
    };
    assert!(
        matches!(&fx[..], [Effect::Menu(menu)] if menu.items.iter().any(|item| item.label == "2 files uncommitted in it: lost"))
    );
}

#[test]
fn host_shows_unavailable_and_last_known_without_false_clean_at_both_sizes() {
    for (width, height) in [(80, 24), (160, 40)] {
        for cached in [false, true] {
            let mut p = sample();
            if cached {
                receive(&mut p, false);
            }
            p.now += 60;
            receive(&mut p, true);
            p.branches.view = 1;
            select_worktree(&p);
            let mut h = host(p);
            let tree = radar_core::shell::to_text(&h, width, height);
            assert!(
                tree.contains(if cached { "last known" } else { "unknown" }),
                "{tree}"
            );
            assert!(!tree.contains("No uncommitted changes shown"));
            insta::assert_snapshot!(
                format!(
                    "status_{}_tree_{width}x{height}",
                    if cached { "cached" } else { "cold" }
                ),
                tree
            );
            export(
                &h,
                width,
                height,
                if cached { "cached-tree" } else { "cold-tree" },
            );
            h.focus = Pane::Detail;
            let preview = radar_core::shell::to_text(&h, width, height);
            assert!(
                preview.contains(if cached {
                    "last known: 2 changed"
                } else {
                    "unavailable"
                }),
                "{preview}"
            );
            assert!(!preview.contains("nothing uncommitted"));
            insta::assert_snapshot!(
                format!(
                    "status_{}_preview_{width}x{height}",
                    if cached { "cached" } else { "cold" }
                ),
                preview
            );
            export(
                &h,
                width,
                height,
                if cached {
                    "cached-preview"
                } else {
                    "cold-preview"
                },
            );
            let fx = press(&mut h, KeyCode::Char('d'));
            assert!(fx.is_empty() && h.menu.is_none());
            let refused = radar_core::shell::to_text(&h, width, height);
            assert!(
                refused.contains("worktree status is unavailable"),
                "{refused}"
            );
        }
    }
}

#[test]
fn host_confirmation_rechecks_original_worktree_after_status_failure() {
    for (width, height) in [(80, 24), (160, 40)] {
        let mut p = sample();
        receive(&mut p, false);
        select_worktree(&p);
        let mut h = host(p);
        radar_core::shell::to_text(&h, width, height);
        press(&mut h, KeyCode::Char('d'));
        let question = radar_core::shell::to_text(&h, width, height);
        assert!(
            question.contains("2 files uncommitted in it: lost"),
            "{question}"
        );
        insta::assert_snapshot!(
            format!("status_known_remove_confirm_{width}x{height}"),
            question
        );
        export(&h, width, height, "known-remove-confirm");
        let original = h.menu.as_ref().unwrap().menu.id.clone();
        h.update(Event::Msg {
            plugin: 0,
            msg: Box::new(reply(true)),
        });
        assert_eq!(h.menu.as_ref().unwrap().menu.id, original);
        let fx = press(&mut h, KeyCode::Char('y'));
        assert!(
            !fx.iter().any(|fx| matches!(fx, HostEffect::Spawn { .. })),
            "unknown stale dirty must never force remove"
        );
        assert!(h.menu.is_none());
        assert!(
            radar_core::shell::to_text(&h, width, height).contains("worktree status is unavailable")
        );
    }
}

#[test]
fn host_mouse_menu_retry_and_keyboard_repeat_dispatch_once_then_recover() {
    let mut p = sample();
    receive(&mut p, true);
    select_worktree(&p);
    let mut h = host(p);
    press(&mut h, KeyCode::Char('x'));
    radar_core::shell::to_text(&h, 80, 24);
    let menu = &h.menu.as_ref().unwrap().menu;
    let index = menu
        .items
        .iter()
        .position(|item| item.label == "read the clones again")
        .unwrap();
    let (rect, _) = h
        .hits
        .borrow()
        .iter()
        .find(|(_, hit)| *hit == Hit::MenuItem(index))
        .unwrap()
        .clone();
    let fx = h.update(Event::Mouse {
        kind: MouseKind::Press,
        column: rect.x,
        row: rect.y,
        at_ms: 1000,
    });
    assert_eq!(
        fx.iter()
            .filter(|fx| matches!(fx, HostEffect::Spawn { .. }))
            .count(),
        1
    );
    assert!(h.menu.is_none());
    let again = press(&mut h, KeyCode::Char('r'));
    assert!(
        !again
            .iter()
            .any(|fx| matches!(fx, HostEffect::Spawn { .. }))
    );
    h.update(Event::Msg {
        plugin: 0,
        msg: Box::new(reply(false)),
    });
    h.focus = Pane::Detail;
    let text = radar_core::shell::to_text(&h, 80, 24);
    assert!(text.contains("2 changed"), "{text}");
    assert!(!text.contains("unavailable") && !text.contains("last known"));
}

#[test]
fn cached_clean_status_is_last_known_zero_and_bulk_question_names_skipped_statuses() {
    let mut p = sample();
    receive(&mut p, false);
    p.branches.details.get_mut(CLONE).unwrap().trees[1].dirty = 0;
    receive(&mut p, true);
    let d = p.branches.details.get_mut(CLONE).unwrap();
    assert_eq!(d.trees[1].status, StatusRead::LastKnown);
    assert_eq!(d.trees[1].dirty, 0);
    d.branches.push(Branch {
        name: "old".into(),
        sha: "ccc3333".into(),
        merged: true,
        ..Branch::default()
    });
    let words = tree_changes(&d.trees[1]).words();
    assert_eq!(words, "last known: 0 changed");
    assert!(!words.contains("clean"));
    let fx = ask_prune(&p);
    let [Effect::Menu(menu)] = &fx[..] else {
        panic!("prune question")
    };
    assert!(menu.items.iter().any(|item| {
        item.label
            .contains("1 worktree skipped: status unavailable")
    }));
    assert!(
        menu.items
            .iter()
            .any(|item| item.label.contains("branch old"))
    );
    assert!(!menu.items.iter().any(|item| item.label.contains(WORKTREE)));
}

#[test]
fn cold_interrupted_read_does_not_render_perpetual_loading_or_successful_freshness() {
    let mut p = sample();
    refresh(&mut p);
    on_msg(
        &mut p,
        BranchMsg::ReadInterrupted {
            dir: CLONE.into(),
            why: "fake interrupted read".into(),
        },
    );
    assert!(!busy(&p));
    assert_eq!(p.branches.last_ok, None);
    for (width, height) in [(80, 24), (160, 40)] {
        let text = radar_core::shell::to_text(&host_for_interruption(), width, height);
        assert!(text.contains("Could not read the clones"), "{text}");
        assert!(!text.contains("Reading the clones…"));
        assert!(text.contains("read incomplete"), "{text}");
    }
}

fn host_for_interruption() -> Host {
    let mut p = sample();
    refresh(&mut p);
    on_msg(
        &mut p,
        BranchMsg::ReadInterrupted {
            dir: CLONE.into(),
            why: "fake interrupted read".into(),
        },
    );
    host(p)
}

#[test]
fn one_post_action_reread_does_not_advance_whole_screen_sync_freshness() {
    let mut p = sample();
    receive(&mut p, false);
    let last_complete = p.branches.last_ok;
    p.now += 30;
    request_read(&mut p, CLONE.into());
    on_msg(&mut p, reply(false));
    assert_eq!(p.branches.last_ok, last_complete);
    p.now += 30;
    receive(&mut p, false);
    assert_eq!(p.branches.last_ok, Some(p.now));
}

#[test]
fn interrupted_cached_dirty_status_cannot_force_remove_and_preserves_unknowns() {
    for dirty in [0, 2] {
        let mut p = sample();
        receive(&mut p, false);
        p.branches.details.get_mut(CLONE).unwrap().trees[1].dirty = dirty;
        let known = p.branches.details[CLONE].trees[1].clone();
        p.branches.details.get_mut(CLONE).unwrap().trees[0].status = StatusRead::Unavailable;
        refresh(&mut p);
        let fx = on_msg(
            &mut p,
            BranchMsg::ReadInterrupted {
                dir: CLONE.into(),
                why: "SECRET_INTERRUPTED_STATUS".into(),
            },
        );
        assert!(matches!(fx[0], Effect::Report(JournalOutcome::Unknown)));
        let tree = &p.branches.details[CLONE].trees[1];
        assert_eq!(tree.status, StatusRead::LastKnown);
        assert!(visible(View::Dirty, Some(&p.branches.details[CLONE])));
        let shown = tree_changes(tree).words();
        assert!(shown.contains("last known") && !shown.contains("clean"));
        assert_eq!(
            (tree.dirty, tree.ahead, tree.behind),
            (known.dirty, known.ahead, known.behind)
        );
        assert_eq!(
            p.branches.details[CLONE].trees[0].status,
            StatusRead::Unavailable
        );
        select_worktree(&p);
        let Outcome::Handled(fx) = key(
            &mut p,
            Pane::Middle,
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
        ) else {
            panic!("handled refusal")
        };
        assert!(matches!(&fx[..], [Effect::Notify(why)] if why.contains("read the clones again")));
        let fx = on_menu(
            &mut p,
            &format!("{DELETE}:tree\t{CLONE}\t{WORKTREE}"),
            "yes",
        )
        .unwrap();
        assert!(matches!(
            &fx[..],
            [Effect::Report(JournalOutcome::Blocked), Effect::Notify(_)]
        ));
        assert!(!candidates(&p).iter().any(|c| c.tree == WORKTREE));
        receive(&mut p, false);
        assert_eq!(
            p.branches.details[CLONE].trees[1].status,
            StatusRead::Current
        );
    }
}

#[test]
fn successful_reread_cannot_upgrade_clean_removal_confirmation_to_force() {
    let mut p = sample();
    let (mut detail, observed) = fixture(false);
    detail.trees[1].dirty = 0;
    on_msg(
        &mut p,
        BranchMsg::Read {
            dir: CLONE.into(),
            detail: Box::new(detail),
            observed,
            action: read_action(CLONE),
        },
    );
    select_worktree(&p);
    let Outcome::Handled(fx) = key(
        &mut p,
        Pane::Middle,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
    ) else {
        panic!("removal question")
    };
    let [Effect::Menu(question)] = &fx[..] else {
        panic!("one removal question")
    };
    assert!(
        question
            .items
            .iter()
            .any(|i| i.label == "nothing uncommitted in it")
    );
    on_msg(&mut p, reply(false));
    let tree = &p.branches.details[CLONE].trees[1];
    assert_eq!((tree.status, tree.dirty), (StatusRead::Current, 2));
    // Never poll a production write future, even on the counterfactual baseline.
    let fx = on_menu(&mut p, &question.id, "yes").unwrap();
    assert!(
        !fx.iter().any(|effect| matches!(effect, Effect::Spawn(_))),
        "a clean approval must not dispatch removal after a dirty successful read"
    );
    assert!(p.branches.changing.is_empty());
}
