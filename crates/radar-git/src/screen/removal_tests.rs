//! No service or filesystem mutations: every read is fake, and production
//! removal futures are dropped. Only the explicitly injected fake is polled.
use super::*;
use radar_core::host::{Event, Host, HostEffect, JournalRecord, View as HostView};
use radar_core::ratatui::crossterm::event::KeyModifiers;
use std::sync::{Arc, Mutex};

const CLONE: &str = "/work/acme/api";
const TREE: &str = "/work/acme/api-hotfix";

fn reply(dirty: usize) -> BranchMsg {
    let (detail, observed) = read_with(CLONE, |path, args| {
        Ok(match args[0] {
            "for-each-ref" => {
                "main\torigin/main\t\t10\taaa1111\tmain\nhotfix\torigin/hotfix\t\t20\tbbb2222\tfix"
                    .into()
            }
            "symbolic-ref" => "origin/main".into(),
            "rev-parse" => "aaa1111".into(),
            "branch" => "main\nhotfix".into(),
            "rev-list" => "0".into(),
            "worktree" => format!(
                "worktree {CLONE}\nHEAD aaa1111\nbranch refs/heads/main\n\nworktree {TREE}\nHEAD bbb2222\nbranch refs/heads/hotfix"
            ),
            "status" if path == Path::new(TREE) => {
                (0..dirty).map(|i| format!("? file-{i}\n")).collect()
            }
            "status" | "stash" => String::new(),
            _ => panic!("unexpected fake read: {args:?}"),
        })
    });
    assert_eq!(observed.outcome(), JournalOutcome::Succeeded);
    BranchMsg::Read {
        dir: CLONE.into(),
        detail: Box::new(detail),
        observed,
        action: read_action(CLONE),
    }
}

fn sample(dirty: usize) -> GitPlugin {
    let mut p = crate::demo::branches();
    on_msg(&mut p, reply(dirty));
    p.branches
        .tree
        .borrow_mut()
        .select(vec![clone_id(CLONE), tree_id(TREE)]);
    p
}

fn question(p: &mut GitPlugin) -> Menu {
    let fx: [Effect; 1] = delete(p).try_into().expect("one removal question");
    let [Effect::Menu(menu)] = fx else {
        panic!("expected removal question")
    };
    menu
}

fn blocked(fx: &[Effect]) {
    assert!(
        matches!(
            fx,
            [Effect::Report(JournalOutcome::Blocked), Effect::Notify(_)]
        ),
        "a stale confirmation must not create a removal task: {fx:?}"
    );
}

fn dispatched(fx: &[Effect]) {
    assert!(matches!(
        fx,
        [
            Effect::Journal(_),
            Effect::Report(JournalOutcome::Confirmed),
            Effect::Report(JournalOutcome::Started),
            Effect::Spawn(_)
        ]
    ));
}

#[test]
fn successful_refresh_from_clean_to_dirty_requires_fresh_removal_confirmation() {
    let mut p = sample(0);
    let original = question(&mut p);
    assert!(
        original
            .items
            .iter()
            .any(|i| i.label == "nothing uncommitted in it")
    );
    on_msg(&mut p, reply(2));
    assert_eq!(
        p.branches.details[CLONE].trees[1].status,
        StatusRead::Current
    );
    let fx = on_menu(&mut p, &original.id, "yes").unwrap();
    blocked(&fx);
    assert!(matches!(&fx[1], Effect::Notify(why) if why == CHANGED));
    assert!(p.branches.changing.is_empty());
    assert!(p.branches.removal.pending.is_none());
    let updated = question(&mut p);
    assert_ne!(updated.id, original.id);
    assert!(
        updated
            .items
            .iter()
            .any(|i| i.label == "2 files uncommitted in it: lost")
    );
    // Inspect and drop the real write future, never poll it.
    dispatched(&on_menu(&mut p, &updated.id, "yes").unwrap());
}

#[test]
fn changed_identity_scope_or_observed_risk_blocks_and_consumes_only_its_intent() {
    type Change = fn(&mut GitPlugin);
    let changes: &[(&str, Change)] = &[
        ("more dirty files", |p| {
            p.branches.details.get_mut(CLONE).unwrap().trees[1].dirty = 3
        }),
        ("fewer dirty files", |p| {
            p.branches.details.get_mut(CLONE).unwrap().trees[1].dirty = 1
        }),
        ("became clean", |p| {
            p.branches.details.get_mut(CLONE).unwrap().trees[1].dirty = 0
        }),
        ("branch", |p| {
            p.branches.details.get_mut(CLONE).unwrap().trees[1].branch = "other".into()
        }),
        ("observed HEAD", |p| {
            p.branches.details.get_mut(CLONE).unwrap().trees[1].head = "ccc3333".into()
        }),
        ("worktree path", |p| {
            p.branches.details.get_mut(CLONE).unwrap().trees[1].dir = "/work/other".into()
        }),
        ("worktree missing", |p| {
            p.branches.details.get_mut(CLONE).unwrap().trees.pop();
        }),
        ("clone detail identity", |p| {
            p.branches.details.get_mut(CLONE).unwrap().dir = "/work/other".into()
        }),
        ("clone missing", |p| {
            p.local
                .index
                .as_mut()
                .unwrap()
                .checkouts
                .retain(|c| c.dir != CLONE)
        }),
        ("clone no longer a clone", |p| {
            p.local.index.as_mut().unwrap().checkouts[0].main = "/work/other".into()
        }),
        ("target became a clone", |p| {
            p.local.index.as_mut().unwrap().checkouts[1].main.clear()
        }),
        ("status unavailable", |p| {
            p.branches.details.get_mut(CLONE).unwrap().trees[1].status = StatusRead::Unavailable
        }),
        ("status last known", |p| {
            p.branches.details.get_mut(CLONE).unwrap().trees[1].status = StatusRead::LastKnown
        }),
        ("remote account", |p| {
            p.local.index.as_mut().unwrap().checkouts[0].remote =
                "git@other.example:else/api.git".into()
        }),
        ("project identity", |p| {
            p.local.index.as_mut().unwrap().checkouts[0].project = "other/api".into()
        }),
        ("root scope", |p| p.cfg.roots = vec!["/other-root".into()]),
        ("scan depth", |p| p.cfg.depth += 1),
        ("linked scope", |p| {
            p.cfg.links.push(local::Link {
                repo: "other/api".into(),
                dir: "/other".into(),
                ..Default::default()
            })
        }),
        ("duplicate target", |p| {
            let d = p.branches.details.get_mut(CLONE).unwrap();
            d.trees.push(d.trees[1].clone());
        }),
    ];
    for (name, change) in changes {
        let mut p = sample(2);
        let original = question(&mut p);
        change(&mut p);
        let fx = on_menu(&mut p, &original.id, "yes").unwrap();
        blocked(&fx);
        assert!(p.branches.changing.is_empty(), "{name}");
        assert!(p.branches.removal.pending.is_none(), "{name}");
        blocked(&on_menu(&mut p, &original.id, "yes").unwrap());
    }
}

#[test]
fn pending_removal_cannot_remove_the_clone_itself_or_change_an_inflight_clone() {
    let mut p = sample(0);
    p.branches
        .tree
        .borrow_mut()
        .select(vec![clone_id(CLONE), tree_id(CLONE)]);
    assert!(
        matches!(&delete(&mut p)[..], [Effect::Notify(why)] if why == "that is the clone itself")
    );
    assert!(p.branches.removal.pending.is_none());
    let mut p = sample(0);
    let original = question(&mut p);
    p.branches.changing.insert(CLONE.into());
    blocked(&on_menu(&mut p, &original.id, "yes").unwrap());
    assert_eq!(p.branches.changing.len(), 1);
}

#[test]
fn cancel_reopen_and_outdated_answers_never_authorize_or_consume_a_new_question() {
    let mut p = sample(0);
    let original = question(&mut p);
    assert!(matches!(
        &on_menu(&mut p, &original.id, "no").unwrap()[..],
        [Effect::Report(JournalOutcome::Cancelled)]
    ));
    blocked(&on_menu(&mut p, &original.id, "yes").unwrap());
    let reopened = question(&mut p);
    assert_ne!(original.id, reopened.id);
    for value in ["yes", "no"] {
        blocked(&on_menu(&mut p, &original.id, value).unwrap());
        assert_eq!(
            p.branches.removal.pending.as_ref().unwrap().menu,
            reopened.id
        );
    }
    let newest = question(&mut p);
    blocked(&on_menu(&mut p, &reopened.id, "yes").unwrap());
    dispatched(&on_menu(&mut p, &newest.id, "yes").unwrap());
    blocked(&on_menu(&mut p, &newest.id, "yes").unwrap());
}

#[test]
fn legacy_or_forged_path_menus_cannot_create_removal_intents() {
    let mut p = sample(2);
    for id in [
        format!("{DELETE}:tree\t{CLONE}\t{TREE}"),
        format!("{DELETE}:tree\t1"),
    ] {
        blocked(&on_menu(&mut p, &id, "yes").unwrap());
    }
    assert!(p.branches.changing.is_empty());
}

#[test]
fn target_scope_is_transient_redacted_and_menu_nonce_does_not_encode_it() {
    let mut p = sample(2);
    p.cfg.roots.push("SECRET_ROOT_SCOPE".into());
    p.local.index.as_mut().unwrap().checkouts[0].remote = "SECRET_ACCOUNT_REMOTE".into();
    let original = question(&mut p);
    let pending = format!("{:?}", p.branches.removal);
    assert!(!pending.contains("SECRET") && !pending.contains(CLONE) && !pending.contains(TREE));
    assert!(!original.id.contains(CLONE) && !original.id.contains(TREE));
    assert!(
        original
            .id
            .split('\t')
            .next_back()
            .unwrap()
            .parse::<u64>()
            .is_ok()
    );
}

#[tokio::test]
async fn unchanged_clean_and_approved_dirty_dispatch_original_target_and_force_once() {
    for dirty in [0, 2] {
        let mut p = sample(dirty);
        let original = question(&mut p);
        on_msg(&mut p, reply(dirty));
        // Changed selection is irrelevant: the old confirmation keeps its target.
        p.branches
            .tree
            .borrow_mut()
            .select(vec![clone_id("/work/acme/web")]);
        let calls = Arc::new(Mutex::new(Vec::new()));
        let recorded = calls.clone();
        let fx = answer_with(&mut p, &original.id, "yes", move |clone, tree, force| {
            recorded
                .lock()
                .unwrap()
                .push((clone.to_path_buf(), tree.to_string(), force));
            Ok("fake removal only".into())
        });
        dispatched(&fx);
        // This future calls only the fake closure above, never git or a service.
        for effect in fx {
            if let Effect::Spawn(task) = effect {
                let msg = *task
                    .await
                    .downcast::<BranchMsg>()
                    .unwrap_or_else(|_| panic!("fake reply"));
                let BranchMsg::Done { said, reread } = msg else {
                    panic!("done")
                };
                assert_eq!(said.unwrap(), "fake removal only");
                assert_eq!(reread, [CLONE]);
            }
        }
        assert_eq!(
            &*calls.lock().unwrap(),
            &[(Path::new(CLONE).to_path_buf(), TREE.into(), dirty > 0)]
        );
        blocked(&on_menu(&mut p, &original.id, "yes").unwrap());
        assert_eq!(calls.lock().unwrap().len(), 1);
    }
}

fn hosted(p: GitPlugin) -> Host {
    let now = p.now;
    let mut h = Host::new(vec![Box::new(GitPlugin::default())], now).unwrap();
    h.load_config(Some("")).unwrap();
    h.plugins[0] = Box::new(p);
    h.view = HostView::Screen(0);
    h.focus = Pane::Middle;
    h
}

fn press(h: &mut Host, code: KeyCode) -> Vec<HostEffect> {
    h.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
}

fn spawned(effect: &HostEffect) -> bool {
    match effect {
        HostEffect::Spawn { .. } => true,
        HostEffect::Tracked { effect, .. } => spawned(effect),
        _ => false,
    }
}

fn export(h: &Host, width: u16, height: u16, name: &str) {
    let Some(dir) = std::env::var_os("RADAR_REMOVAL_SHOTS") else {
        return;
    };
    let dir = Path::new(&dir);
    std::fs::create_dir_all(dir).unwrap();
    let _ = radar_core::shell::to_svg(h, width, height);
    let svg = radar_core::shell::to_svg(h, width, height);
    let name = format!("{name}-{width}x{height}");
    std::fs::write(dir.join(format!("{name}.svg")), &svg).unwrap();
    std::fs::write(
        dir.join(format!("{name}.png")),
        radar_core::shot::to_png(&svg).unwrap(),
    )
    .unwrap();
}

#[test]
fn host_stale_clean_question_blocks_successful_dirty_refresh_then_reopens_honestly() {
    for (width, height) in [(80, 24), (160, 40)] {
        let mut h = hosted(sample(0));
        h.enable_journal();
        radar_core::shell::to_text(&h, width, height);
        press(&mut h, KeyCode::Char('d'));
        let before = radar_core::shell::to_text(&h, width, height);
        assert!(before.contains("nothing uncommitted in it"));
        let original = h.menu.as_ref().unwrap().menu.id.clone();
        h.update(Event::Msg {
            plugin: 0,
            msg: Box::new(reply(2)),
        });
        assert_eq!(h.menu.as_ref().unwrap().menu.id, original);
        let fx = press(&mut h, KeyCode::Char('y'));
        assert!(!fx.iter().any(spawned));
        assert!(fx.iter().any(|e| matches!(
            e,
            HostEffect::Journal(JournalRecord::Outcome {
                outcome: JournalOutcome::Blocked,
                ..
            })
        )));
        assert!(h.menu.is_none());
        let stale = radar_core::shell::to_text(&h, width, height);
        assert!(stale.contains(CHANGED), "{stale}");
        insta::assert_snapshot!(format!("stale_removal_{width}x{height}"), stale);
        export(&h, width, height, "stale-removal");
        let fx = press(&mut h, KeyCode::Char('y'));
        assert!(!fx.iter().any(spawned));
        press(&mut h, KeyCode::Char('d'));
        assert_ne!(h.menu.as_ref().unwrap().menu.id, original);
        let updated = radar_core::shell::to_text(&h, width, height);
        assert!(updated.contains("2 files uncommitted in it: lost"));
        insta::assert_snapshot!(format!("reviewed_removal_{width}x{height}"), updated);
        export(&h, width, height, "reviewed-removal");
        // A fresh, explicit confirmation is allowed. Drop its production future.
        let fx = press(&mut h, KeyCode::Char('y'));
        assert_eq!(fx.iter().filter(|e| spawned(e)).count(), 1);
    }
}

#[test]
fn host_escape_then_repeated_yes_does_nothing_and_reopening_uses_new_intent() {
    let mut h = hosted(sample(0));
    radar_core::shell::to_text(&h, 80, 24);
    press(&mut h, KeyCode::Char('d'));
    radar_core::shell::to_text(&h, 80, 24);
    let original = h.menu.as_ref().unwrap().menu.id.clone();
    press(&mut h, KeyCode::Esc);
    assert!(h.menu.is_none());
    for _ in 0..2 {
        let fx = press(&mut h, KeyCode::Char('y'));
        assert!(!fx.iter().any(spawned));
    }
    press(&mut h, KeyCode::Char('d'));
    assert_ne!(h.menu.as_ref().unwrap().menu.id, original);
    radar_core::shell::to_text(&h, 80, 24);
    press(&mut h, KeyCode::Char('n'));
    assert!(h.menu.is_none());
}
