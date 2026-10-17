use super::*;
use radar_core::{Plugin, Subject, plugin::Effect};
use std::{
    collections::VecDeque,
    sync::atomic::{AtomicU64, Ordering},
};

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "radar-watch-fixture-{}-{stamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn installed_git() -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join(if cfg!(windows) { "git.exe" } else { "git" }))
        .find(|path| path.is_file())
        .expect("these real local Git tests require installed Git")
        .canonicalize()
        .unwrap()
}

fn fixture() -> (Scratch, GitPlugin, LocalCheckoutFixture) {
    let scratch = Scratch::new();
    let (plugin, fixture) = local_checkout_fixture(&scratch.0, &installed_git()).unwrap();
    (scratch, plugin, fixture)
}

async fn finish(plugin: &mut GitPlugin, effects: Vec<Effect>) {
    let mut queue: VecDeque<_> = effects.into();
    while let Some(effect) = queue.pop_front() {
        let task = match effect {
            Effect::Spawn(task) | Effect::SpawnAction { task, .. } => task,
            _ => continue,
        };
        queue.extend(plugin.on_msg(task.await));
    }
}

#[tokio::test]
async fn actual_scanner_and_branches_read_the_dirty_local_clone_without_changes() {
    let (_scratch, mut plugin, fixture) = fixture();
    assert_eq!(fixture.commands().len(), 5);
    let effects = plugin.tick(radar_core::plugin::DEMO_NOW);
    assert!(
        fixture.check_finished().is_err(),
        "queued scans count as unfinished"
    );
    finish(&mut plugin, effects).await;
    let index = plugin.local.index.as_ref().unwrap();
    assert_eq!(index.folders, 1);
    assert_eq!(index.checkouts.len(), 1);
    assert_eq!(index.checkouts[0].dir, fixture.root().display().to_string());
    assert_eq!(
        index.checkouts[0].remote,
        fixture.0.origin.display().to_string()
    );
    assert!(
        index.checkouts[0].project.is_empty(),
        "the origin is an actual local path"
    );
    assert!(plugin.local.installed.is_empty());
    assert_eq!(
        plugin.local.terminals,
        Some(crate::terminal::Terminals::default())
    );
    assert_eq!(plugin.local.home, local::HomeWalk::Off);
    let effects = screen::refresh(&mut plugin);
    assert!(
        fixture.check_finished().is_err(),
        "queued Branches reads count as unfinished"
    );
    finish(&mut plugin, effects).await;
    let detail = plugin.branches.details.values().next().unwrap();
    assert!(detail.error.is_empty());
    assert_eq!(detail.branches.len(), 1);
    assert_eq!(detail.branches[0].name, "main");
    assert_eq!(detail.branches[0].upstream, "origin/main");
    assert_eq!(detail.trees.len(), 1);
    assert_eq!(detail.trees[0].dirty, 1);
    assert!(detail.stashes.is_empty());
    assert!(
        fixture
            .commands()
            .iter()
            .any(|c| c.phase == GitCommandPhase::Read && c.args[0] == "status")
    );
    for command in fixture.commands() {
        let helper = command
            .args
            .iter()
            .any(|arg| arg.starts_with("--upload-pack="));
        assert_eq!(
            helper,
            command.phase == GitCommandPhase::Setup && command.args[0] == "clone"
        );
        if helper {
            assert!(command.args.iter().any(|arg| arg == "--local"));
            assert!(command.args.iter().any(|arg| arg == "--no-hardlinks"));
        }
    }
    fixture.assert_finished();
    fixture.assert_unchanged();
}

#[tokio::test]
async fn configuration_cache_and_demo_keep_the_explicit_fixture_attached() {
    let (_scratch, mut plugin, fixture) = fixture();
    let effects = plugin.tick(radar_core::plugin::DEMO_NOW);
    finish(&mut plugin, effects).await;
    let cache = plugin.snapshot().unwrap();
    plugin.configure(None).unwrap();
    plugin.configure(Some(&fixture.config())).unwrap();
    plugin.restore(cache.clone());
    plugin.demo();
    assert_eq!(plugin.snapshot().unwrap(), cache);
    assert_eq!(plugin.config(), Some(fixture.config()));
    assert!(plugin.fixture.is_some());
    let effects = screen::refresh(&mut plugin);
    finish(&mut plugin, effects).await;
    fixture.assert_finished();
    fixture.assert_unchanged();
    let mut ordinary = GitPlugin::default();
    ordinary.configure(Some(&fixture.config())).unwrap();
    assert!(
        ordinary.fixture.is_none(),
        "configuration never opts into test support"
    );
}

#[tokio::test]
async fn foreign_config_cache_actions_discovery_and_cli_are_sticky_refusals() {
    let (_scratch, mut plugin, fixture) = fixture();
    let original = plugin.config();
    let mut foreign = fixture.config();
    foreign["roots"] = toml::Value::Array(vec![toml::Value::String("/outside-watch".into())]);
    assert!(plugin.configure(Some(&foreign)).is_err());
    assert_eq!(plugin.config(), original);
    plugin.restore(serde_json::json!({ "index": {
        "checkouts": [{ "dir": "/outside-watch", "remote": "https://watch.example.test/watch.git",
            "project": "ankh-morpork/watch", "branch": "main", "files": [], "main": "" }],
        "at": 0, "folders": 1, "cut": false, "missing": []
    }, "use": {} }));
    assert!(plugin.local.index.is_none());
    let subject = Subject::new("repo", "ankh-morpork/watch", "Watch");
    for action in ["push", "fetch", "editor", "terminal", "scan", "diff"] {
        assert!(plugin.act(&subject, action).is_empty());
    }
    assert!(plugin.on_menu("branches-delete", "yes").is_empty());
    assert!(plugin.on_menu("bulk", "prune").is_empty());
    assert!(
        plugin
            .on_bus(crate::act::START, &serde_json::json!({}))
            .is_empty()
    );
    plugin.setup_looked();
    let _ = plugin.typing("local-link:watch", "/outside-watch");
    let _ = plugin.intent(
        "checkouts",
        radar_core::Pane::Middle,
        &Intent::Key {
            chord: radar_core::view::Chord::char('s'),
        },
    );
    let _ = plugin.intent(
        screen::SCREEN.id,
        radar_core::Pane::Middle,
        &Intent::Invoke {
            action: "git.branch.delete".into(),
        },
    );
    let request = radar_core::agent_context::WorktreeRequest {
        repository: "ankh-morpork/watch".into(),
        branch: "main".into(),
        sha: "0".repeat(40),
        for_subject: None,
    };
    assert!(plugin.ensure_worktree(&request).unwrap().0.is_err());
    let matches = clap::Command::new("watch").get_matches_from(["watch"]);
    assert!(plugin.run_cli(&matches).await.is_err());
    assert_eq!(
        fixture.commands().len(),
        5,
        "none of the rejected entrypoints launches Git"
    );
    assert!(fixture.violations().len() >= 15);
    plugin.configure(None).unwrap();
    assert!(
        fixture.check_finished().is_err(),
        "reconfigure cannot erase violations"
    );
    fixture.assert_unchanged();
}

#[test]
fn unknown_commands_refs_and_roots_never_launch_a_child() {
    let (_scratch, _plugin, fixture) = fixture();
    for args in [
        vec!["fetch", "origin"],
        vec!["push", "origin", "main"],
        vec!["diff"],
        vec!["status", "--porcelain=2", "--branch", "--ignored"],
        vec!["-c", "alias.status=!false", "status"],
        vec!["rev-list", "--count", "elsewhere", "--not", "--remotes"],
        vec!["worktree", "add", "/outside-watch"],
    ] {
        assert!(fixture.git(fixture.root(), &args).is_err());
    }
    assert!(
        fixture
            .git(
                Path::new("/outside-watch"),
                &["status", "--porcelain=2", "--branch"]
            )
            .is_err()
    );
    assert!(fixture.scan(&["/outside-watch".into()], 1).is_err());
    assert_eq!(fixture.commands().len(), 5);
    assert_eq!(fixture.violations().len(), 9);
    assert!(fixture.check_finished().is_err());
    fixture.assert_unchanged();
}

#[test]
fn edited_config_external_gitdir_and_alternates_are_rejected_before_git() {
    let (scratch, _plugin, fixture) = fixture();
    let outside = scratch.0.join("outside-config");
    fs::write(&outside, "[alias]\nwatch = !false\n").unwrap();
    let dot_git = fixture.root().join(".git");
    let config = dot_git.join("config");
    let original = fs::read(&config).unwrap();
    let mut changed = original.clone();
    changed.extend_from_slice(format!("\n[include]\npath = {}\n", outside.display()).as_bytes());
    fs::write(&config, changed).unwrap();
    assert!(
        fixture
            .git(fixture.root(), &["status", "--porcelain=2", "--branch"])
            .is_err()
    );
    fs::write(&config, original).unwrap();
    let saved = fixture.root().join("saved-git");
    fs::rename(&dot_git, &saved).unwrap();
    fs::write(&dot_git, format!("gitdir: {}\n", outside.display())).unwrap();
    assert!(
        fixture
            .git(fixture.root(), &["status", "--porcelain=2", "--branch"])
            .is_err()
    );
    fs::remove_file(&dot_git).unwrap();
    fs::rename(&saved, &dot_git).unwrap();
    let alternates = dot_git.join("objects/info/alternates");
    fs::write(&alternates, format!("{}\n", outside.display())).unwrap();
    assert!(
        fixture
            .git(fixture.root(), &["status", "--porcelain=2", "--branch"])
            .is_err()
    );
    fs::remove_file(alternates).unwrap();
    assert_eq!(fixture.commands().len(), 5);
    assert_eq!(fixture.violations().len(), 3);
    assert!(fixture.check_finished().is_err());
    fixture.assert_unchanged();
}

#[cfg(unix)]
#[test]
fn symlinks_and_hard_links_are_refused_without_following_them() {
    use std::os::unix::fs::symlink;
    let (scratch, _plugin, fixture) = fixture();
    let outside = scratch.0.join("outside-config");
    fs::write(&outside, "outside fixture; never a Git config source\n").unwrap();
    let config = fixture.root().join(".git/config");
    let original = fs::read(&config).unwrap();
    fs::remove_file(&config).unwrap();
    symlink(&outside, &config).unwrap();
    assert!(
        fixture
            .git(fixture.root(), &["status", "--porcelain=2", "--branch"])
            .is_err()
    );
    fs::remove_file(&config).unwrap();
    fs::hard_link(&outside, &config).unwrap();
    assert!(
        fixture
            .git(fixture.root(), &["status", "--porcelain=2", "--branch"])
            .is_err()
    );
    fs::remove_file(&config).unwrap();
    fs::write(config, original).unwrap();
    assert_eq!(fixture.commands().len(), 5);
    assert_eq!(fixture.violations().len(), 2);
    fixture.assert_unchanged();
}

#[test]
fn child_environment_and_protocol_are_explicit_and_scratch_scoped() {
    let (scratch, _plugin, fixture) = fixture();
    let command = fixture.0.isolated.command(
        fixture.root(),
        &["status", "--porcelain=2", "--branch"],
        false,
    );
    assert!(Path::new(command.get_program()).is_absolute());
    let env: BTreeMap<_, _> = command
        .get_envs()
        .map(|(key, value)| {
            (
                key.to_string_lossy().into_owned(),
                value.unwrap().to_string_lossy().into_owned(),
            )
        })
        .collect();
    for name in [
        "HOME",
        "XDG_CONFIG_HOME",
        "XDG_CACHE_HOME",
        "GIT_CONFIG_GLOBAL",
        "GIT_CONFIG_SYSTEM",
        "GIT_EXEC_PATH",
        "PATH",
        "TMPDIR",
    ] {
        assert!(
            Path::new(&env[name]).starts_with(&fixture.0.owned.0),
            "{name}"
        );
    }
    assert_eq!(env["GIT_CONFIG_NOSYSTEM"], "1");
    assert_eq!(env["GIT_CONFIG_COUNT"], "0");
    assert_eq!(env["GIT_TERMINAL_PROMPT"], "0");
    assert_eq!(env["GIT_OPTIONAL_LOCKS"], "0");
    for name in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_CONFIG_PARAMETERS",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_OBJECT_DIRECTORY",
        "GIT_CONFIG_KEY_0",
    ] {
        assert!(!env.contains_key(name));
    }
    let args: Vec<_> = command.get_args().collect();
    assert!(
        args.iter()
            .any(|arg| *arg == std::ffi::OsStr::new("protocol.allow=never"))
    );
    assert!(
        args.iter()
            .any(|arg| *arg == std::ffi::OsStr::new("protocol.file.allow=never"))
    );
    assert!(
        !args
            .iter()
            .any(|arg| *arg == std::ffi::OsStr::new("protocol.file.allow=always"))
    );
    // Only inspect identities here: neither synthetic file is executed.
    let helper_check = scratch.0.join("helper-check");
    fs::create_dir(&helper_check).unwrap();
    let candidate = helper_check.join("git");
    fs::write(&candidate, "not an executable").unwrap();
    assert!(
        verified_upload_pack(&candidate).is_err(),
        "missing helper fails closed"
    );
    fs::write(
        helper_check.join(if cfg!(windows) {
            "git-upload-pack.exe"
        } else {
            "git-upload-pack"
        }),
        "a different file",
    )
    .unwrap();
    assert!(
        verified_upload_pack(&candidate).is_err(),
        "different helper identity fails closed"
    );
    fixture.assert_finished();
    fixture.assert_unchanged();
}

#[test]
fn cleanup_owns_only_its_fresh_subtree_and_waits_for_queued_tasks() {
    let (scratch, mut plugin, fixture) = fixture();
    let keep = scratch.0.join("caller-owned");
    fs::write(&keep, "Watch journal lives beside the fixture").unwrap();
    assert!(
        local_checkout_fixture(&scratch.0, &installed_git()).is_err(),
        "existing subtree is never adopted"
    );
    let root = fixture.root().to_path_buf();
    let effects = plugin.tick(radar_core::plugin::DEMO_NOW);
    drop(plugin);
    drop(fixture);
    assert!(root.exists(), "queued tasks retain the owned fixture");
    drop(effects);
    assert!(!root.exists());
    assert!(keep.exists(), "cleanup must not remove caller-owned state");
}

#[test]
fn bounded_child_reader_kills_a_stalled_installed_git_and_caps_output() {
    let (_scratch, _plugin, fixture) = fixture();
    // Installed Git waits for EOF on this deliberately held pipe. No shell,
    // alias, hook, arbitrary executable or repository write is involved.
    let mut waiting =
        fixture
            .0
            .isolated
            .command(fixture.root(), &["hash-object", "--stdin"], false);
    waiting.stdin(Stdio::piped());
    let started = Instant::now();
    let error = command_output_bounded(waiting, Duration::from_millis(25), 1024).unwrap_err();
    assert!(error.contains("time limit"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(2));
    let output = fixture.0.isolated.command(
        fixture.root(),
        &["for-each-ref", "--format=%(refname)", "refs/heads"],
        false,
    );
    let error = command_output_bounded(output, Duration::from_secs(1), 1).unwrap_err();
    assert!(error.contains("output exceeded"), "{error}");
    fixture.assert_finished();
    fixture.assert_unchanged();
}
