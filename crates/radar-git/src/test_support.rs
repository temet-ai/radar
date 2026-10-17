//! An explicit, disposable local Git boundary for hosted integration scenarios.
//! The real scanner, Git output parsers and reducers are unchanged. Only their
//! I/O boundary is confined: no ambient discovery, user configuration or network.

use std::{
    any::Any,
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use anyhow::{Context, bail, ensure};
use radar_core::view::{Intent, Key};

use crate::{GitPlugin, local, screen};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitCommandPhase {
    Setup,
    Read,
}

/// One actual child-process invocation. `args` is the requested Git operation;
/// `argv` also includes every isolation option passed to the absolute executable.
/// Rejected attempts never launch a child and remain in `violations()` instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitCommand {
    pub phase: GitCommandPhase,
    pub executable: PathBuf,
    pub dir: PathBuf,
    pub args: Vec<String>,
    pub argv: Vec<String>,
    pub completed: bool,
    pub success: Option<bool>,
}

#[derive(Debug, Default)]
struct Ledger {
    commands: Vec<GitCommand>,
    violations: Vec<String>,
    pending: usize,
}

#[derive(Debug)]
struct OwnedDirectory(PathBuf);

impl Drop for OwnedDirectory {
    fn drop(&mut self) {
        // Created with create_dir, never adopted from an existing directory.
        // remove_dir_all does not follow symlinks, including a replaced root.
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Debug)]
struct Inner {
    owned: OwnedDirectory,
    root: PathBuf,
    origin: PathBuf,
    isolated: IsolatedGit,
    config: local::Local,
    baseline: BTreeMap<PathBuf, Option<Vec<u8>>>,
    ledger: Mutex<Ledger>,
}

/// Keeping either the plugin, handle, or a queued task alive keeps its owned
/// subtree alive. The caller continues to own the scratch parent and journal.
#[derive(Clone, Debug)]
pub struct LocalCheckoutFixture(Arc<Inner>);

/// Create seed -> local bare origin -> working clone, then dirty the tracked
/// README. Both arguments must be explicit absolute paths. `scratch` must
/// already exist and be canonical; its `git-fixture` child must not exist.
/// `git_executable` is the caller's installed Git, never a PATH lookup here.
/// Setup also requires its sibling `git-upload-pack` to resolve to that same
/// installed binary. Local clone ref negotiation gets this one explicit helper.
/// The caller must retain exclusive ownership of this scratch parent until all
/// plugin tasks and handles are dropped. This is a deterministic test boundary,
/// not an OS sandbox for an untrusted executable or concurrent filesystem edits.
pub fn local_checkout_fixture(
    scratch: &Path,
    git_executable: &Path,
) -> anyhow::Result<(GitPlugin, LocalCheckoutFixture)> {
    plain_directory(scratch)?;
    ensure!(
        scratch.canonicalize()? == scratch,
        "scratch must be canonical"
    );
    ensure!(
        git_executable.is_absolute(),
        "Git executable must be absolute"
    );
    let git_executable = git_executable
        .canonicalize()
        .context("installed Git executable")?;
    ensure!(git_executable.is_file(), "installed Git must be a file");
    let owned = scratch.join("git-fixture");
    fs::create_dir(&owned).context("fixture subtree must be fresh")?;
    let owned = OwnedDirectory(owned);
    let isolated = IsolatedGit::new(&owned.0, git_executable)?;
    let seed = owned.0.join("seed");
    let origin = owned.0.join("origin.git");
    let root = owned.0.join("watch");
    fs::create_dir(&seed)?;
    let mut ledger = Ledger::default();
    let mut setup = |dir: &Path, args: &[&str]| -> anyhow::Result<()> {
        let command = isolated.command(dir, args, true);
        let mut record = isolated.record(GitCommandPhase::Setup, dir, args, &command);
        let output = command_output(command);
        record.completed = true;
        record.success = Some(output.is_ok());
        ledger.commands.push(record);
        output.map(|_| ()).map_err(anyhow::Error::msg)
    };
    setup(&seed, &["init", "--initial-branch=main"])?;
    fs::write(
        seed.join("README.md"),
        "# Watch\nA local checkout fixture.\n",
    )?;
    setup(&seed, &["add", "--", "README.md"])?;
    setup(&seed, &["commit", "-m", "Begin the Watch fixture"])?;
    let upload_pack = format!("--upload-pack={}", path_text(&isolated.upload_pack)?);
    setup(
        &owned.0,
        &[
            "clone",
            "--local",
            "--bare",
            "--no-hardlinks",
            &upload_pack,
            path_text(&seed)?,
            path_text(&origin)?,
        ],
    )?;
    setup(
        &owned.0,
        &[
            "clone",
            "--local",
            "--no-hardlinks",
            &upload_pack,
            path_text(&origin)?,
            path_text(&root)?,
        ],
    )?;
    fs::write(
        root.join("README.md"),
        "# Watch\nA local checkout fixture, with uncommitted work.\n",
    )?;
    let config = local::Local {
        roots: vec![path_text(&root)?.into()],
        depth: 1,
        types: vec![],
        ..Default::default()
    };
    let baseline = tree_snapshot(&owned.0)?;
    let fixture = LocalCheckoutFixture(Arc::new(Inner {
        owned,
        root,
        origin,
        isolated,
        config,
        baseline,
        ledger: Mutex::new(ledger),
    }));
    fixture.storage()?;
    let plugin = GitPlugin {
        cfg: fixture.0.config.clone(),
        fixture: Some(fixture.clone()),
        ..Default::default()
    };
    Ok((plugin, fixture))
}

impl LocalCheckoutFixture {
    pub fn root(&self) -> &Path {
        &self.0.root
    }

    /// The `[git]` table, not a whole host configuration.
    pub fn config(&self) -> toml::Value {
        toml::Value::try_from(&self.0.config).expect("fixture config serializes")
    }

    pub fn commands(&self) -> Vec<GitCommand> {
        self.0.ledger.lock().unwrap().commands.clone()
    }

    pub fn violations(&self) -> Vec<String> {
        self.0.ledger.lock().unwrap().violations.clone()
    }

    pub fn check_finished(&self) -> anyhow::Result<()> {
        let ledger = self.0.ledger.lock().unwrap();
        ensure!(
            ledger.violations.is_empty(),
            "local Git fixture violations: {:?}",
            ledger.violations
        );
        ensure!(
            ledger.pending == 0,
            "local Git fixture still has {} queued/running operations",
            ledger.pending
        );
        ensure!(
            ledger
                .commands
                .iter()
                .all(|c| c.completed && c.success == Some(true)),
            "local Git child commands are incomplete or failed"
        );
        Ok(())
    }

    pub fn assert_finished(&self) {
        self.check_finished().unwrap();
    }

    /// Compare real file contents, including the index, refs and dirty README,
    /// with the post-setup baseline. Does not run an extra status subprocess.
    pub fn assert_unchanged(&self) {
        self.storage().unwrap();
    }

    pub(crate) fn blocked(&self, operation: &str) -> String {
        let why = format!("blocked local Git fixture operation: {operation}");
        self.0.ledger.lock().unwrap().violations.push(why.clone());
        why
    }

    fn unviolated(&self) -> Result<(), String> {
        if self.0.ledger.lock().unwrap().violations.is_empty() {
            Ok(())
        } else {
            Err("local Git fixture already has a sticky violation".into())
        }
    }

    pub(crate) fn pending(&self) -> Pending {
        self.0.ledger.lock().unwrap().pending += 1;
        Pending(self.clone())
    }

    pub(crate) fn configure(&self, table: Option<&toml::Value>) -> anyhow::Result<()> {
        if let Some(table) = table {
            let parsed: Result<local::Local, _> = table.clone().try_into();
            if !parsed.is_ok_and(|config| config == self.0.config) {
                bail!(self.blocked("configuration outside immutable fixture scope"));
            }
        }
        Ok(())
    }

    fn valid_index(&self, index: &local::Index) -> bool {
        !index.cut
            && index.missing.is_empty()
            && index.folders == 1
            && index.checkouts.len() == 1
            && index.checkouts.iter().all(|c| {
                Path::new(&c.dir) == self.root()
                    && c.dir == self.root().to_string_lossy()
                    && c.remote == self.0.origin.to_string_lossy()
                    && c.branch == "main"
                    && c.project.is_empty()
                    && c.main.is_empty()
                    && c.files == ["README.md"]
            })
    }

    pub(crate) fn accept_cache(&self, cache: &serde_json::Value) -> bool {
        let index = &cache["index"];
        let used = &cache["use"];
        let valid = (index.is_null()
            || serde_json::from_value::<local::Index>(index.clone())
                .is_ok_and(|index| self.valid_index(&index)))
            && (used.is_null() || used.as_object().is_some_and(|used| used.is_empty()));
        if !valid {
            self.blocked("cache outside immutable fixture scope");
        }
        valid
    }

    pub(crate) fn accept_message(&self, message: &(dyn Any + Send)) -> bool {
        let valid = if let Some(message) = message.downcast_ref::<local::LocalMsg>() {
            match message {
                local::LocalMsg::Scanned { index, installed } => {
                    self.valid_index(index) && installed.is_empty()
                }
                local::LocalMsg::Failed(_) => true,
                _ => false,
            }
        } else if let Some(message) = message.downcast_ref::<screen::BranchMsg>() {
            match message {
                screen::BranchMsg::Read { dir, detail, .. } => {
                    dir == &self.root().to_string_lossy()
                        && detail.dir == *dir
                        && detail.trees.iter().all(|tree| tree.dir == *dir)
                }
                screen::BranchMsg::ReadInterrupted { dir, .. } => {
                    dir == &self.root().to_string_lossy()
                }
                _ => false,
            }
        } else {
            false
        };
        if !valid {
            self.blocked("message outside checkout / Branches read boundary");
        }
        valid
    }

    pub(crate) fn accept_intent(&self, scope: &str, intent: &Intent) -> bool {
        let valid = scope == screen::SCREEN.id
            && match intent {
                Intent::Invoke { action } => matches!(
                    action.as_str(),
                    "git.branch.read_again" | "git.branch.use" | "r" | "⏎"
                ),
                Intent::Key { chord } => {
                    !chord.ctrl
                        && !chord.alt
                        && matches!(
                            chord.key,
                            Key::Char('r' | 'j' | 'k' | 'h' | 'l')
                                | Key::Enter
                                | Key::Esc
                                | Key::Up
                                | Key::Down
                                | Key::Left
                                | Key::Right
                                | Key::Tab
                                | Key::BackTab
                                | Key::Home
                                | Key::End
                                | Key::PageUp
                                | Key::PageDown
                        )
                }
                Intent::Select { .. }
                | Intent::Context { .. }
                | Intent::Fold { .. }
                | Intent::Activate { .. }
                | Intent::Show { .. }
                | Intent::End { .. } => true,
                _ => false,
            };
        if !valid {
            self.blocked("intent outside checkout / Branches read boundary");
        }
        valid
    }

    pub(crate) fn scan(&self, roots: &[String], depth: usize) -> Result<local::Index, String> {
        if roots != self.0.config.roots || depth != self.0.config.depth {
            return Err(self.blocked("scan outside exact fixture root"));
        }
        self.storage()
            .map_err(|why| self.blocked(&why.to_string()))?;
        self.unviolated()?;
        // Use the production scanner and read_checkout, with no traversal below
        // the explicit clone. No checkout or parsed model is synthesized here.
        Ok(local::scan(roots, 0))
    }

    pub(crate) fn git(&self, dir: &Path, args: &[&str]) -> Result<String, String> {
        if dir != self.root() || dir.as_os_str() != self.root().as_os_str() {
            return Err(self.blocked("Git command outside exact fixture root"));
        }
        if !allowed_read(args) {
            return Err(self.blocked(&format!("unapproved Git arguments: {args:?}")));
        }
        self.storage()
            .map_err(|why| self.blocked(&why.to_string()))?;
        self.unviolated()?;
        let _pending = self.pending();
        let command = self.0.isolated.command(dir, args, false);
        let record = self
            .0
            .isolated
            .record(GitCommandPhase::Read, dir, args, &command);
        let index = {
            let mut ledger = self.0.ledger.lock().unwrap();
            let index = ledger.commands.len();
            ledger.commands.push(record);
            index
        };
        let output = command_output(command);
        {
            let mut ledger = self.0.ledger.lock().unwrap();
            ledger.commands[index].completed = true;
            ledger.commands[index].success = Some(output.is_ok());
        }
        if let Err(why) = &output {
            self.blocked(&format!("allowed Git read failed: {why}"));
        }
        output
    }

    fn storage(&self) -> anyhow::Result<()> {
        plain_directory(&self.0.owned.0)?;
        plain_directory(self.root())?;
        let dot_git = self.root().join(".git");
        plain_directory(&dot_git)?;
        for forbidden in [
            "commondir",
            "worktrees",
            "config.worktree",
            "objects/info/alternates",
            "objects/info/http-alternates",
        ] {
            ensure!(
                fs::symlink_metadata(dot_git.join(forbidden))
                    .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
                "external Git administration is not allowed: {forbidden}"
            );
        }
        ensure!(
            tree_snapshot(&self.0.owned.0)? == self.0.baseline,
            "fixture files changed outside the read-only contract"
        );
        Ok(())
    }
}

impl GitPlugin {
    pub(crate) fn fixture_denied(&self, operation: &str) -> bool {
        if let Some(fixture) = &self.fixture {
            fixture.blocked(operation);
            true
        } else {
            false
        }
    }
}

pub(crate) struct Pending(LocalCheckoutFixture);

impl Drop for Pending {
    fn drop(&mut self) {
        self.0.0.ledger.lock().unwrap().pending -= 1;
    }
}

fn allowed_read(args: &[&str]) -> bool {
    const FORMAT: &str = "--format=%(refname:short)%09%(upstream:short)%09%(upstream:track)%09%(committerdate:unix)%09%(objectname:short)%09%(subject)";
    matches!(
        args,
        ["for-each-ref", FORMAT, "refs/heads"]
            | ["symbolic-ref", "--short", "refs/remotes/origin/HEAD"]
            | [
                "rev-parse",
                "--verify",
                "--quiet",
                "refs/remotes/origin/main"
            ]
            | [
                "branch",
                "--merged",
                "origin/main" | "main",
                "--format=%(refname:short)"
            ]
            | ["rev-list", "--count", "main", "--not", "--remotes"]
            | ["worktree", "list", "--porcelain"]
            | ["status", "--porcelain=2", "--branch"]
            | ["stash", "list", "--format=%gs"]
    )
}

#[derive(Debug)]
struct IsolatedGit {
    executable: PathBuf,
    upload_pack: PathBuf,
    owned: PathBuf,
    home: PathBuf,
    xdg: PathBuf,
    empty: PathBuf,
    hooks: PathBuf,
    templates: PathBuf,
    helpers: PathBuf,
    temporary: PathBuf,
}

impl IsolatedGit {
    fn new(owned: &Path, executable: PathBuf) -> anyhow::Result<Self> {
        let upload_pack = verified_upload_pack(&executable)?;
        let this = Self {
            executable,
            upload_pack,
            owned: owned.into(),
            home: owned.join("home"),
            xdg: owned.join("xdg"),
            empty: owned.join("empty-config"),
            hooks: owned.join("hooks"),
            templates: owned.join("templates"),
            helpers: owned.join("helpers"),
            temporary: owned.join("tmp"),
        };
        for path in [
            &this.home,
            &this.xdg,
            &this.hooks,
            &this.templates,
            &this.helpers,
            &this.temporary,
        ] {
            fs::create_dir(path)?;
        }
        fs::write(&this.empty, b"")?;
        Ok(this)
    }

    fn command(&self, dir: &Path, args: &[&str], setup: bool) -> Command {
        let mut command = Command::new(&self.executable);
        command
            .env_clear()
            .current_dir(dir)
            .stdin(Stdio::null())
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", &self.xdg)
            .env("XDG_CACHE_HOME", &self.xdg)
            .env("PATH", &self.helpers)
            .env("GIT_EXEC_PATH", &self.helpers)
            .env("TMPDIR", &self.temporary)
            .env("TMP", &self.temporary)
            .env("TEMP", &self.temporary)
            .env("LC_ALL", "C")
            .env("TZ", "UTC")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_SYSTEM", &self.empty)
            .env("GIT_CONFIG_GLOBAL", &self.empty)
            .env("GIT_CONFIG_COUNT", "0")
            .env("GIT_ATTR_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_ASKPASS", "false")
            .env("SSH_ASKPASS", "false")
            .env("SSH_ASKPASS_REQUIRE", "never")
            .env("GIT_PAGER", "")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_NO_REPLACE_OBJECTS", "1")
            .env("GIT_CEILING_DIRECTORIES", &self.owned)
            .env("GIT_AUTHOR_DATE", "2026-10-01T12:00:00+00:00")
            .env("GIT_COMMITTER_DATE", "2026-10-01T12:00:00+00:00")
            .arg("--no-pager")
            .arg("-C")
            .arg(dir);
        for setting in [
            "user.name=Sam Vimes",
            "user.email=vimes@watch.example.test",
            "commit.gpgSign=false",
            "tag.gpgSign=false",
            "core.fsmonitor=false",
            "core.untrackedCache=false",
            "credential.helper=",
            "credential.interactive=false",
            "core.sshCommand=false",
            "core.gitProxy=false",
            "diff.external=",
            "submodule.recurse=false",
            "fetch.recurseSubmodules=false",
            "maintenance.auto=false",
            "gc.auto=0",
            "protocol.allow=never",
        ] {
            command.args(["-c", setting]);
        }
        for (key, path) in [
            ("core.hooksPath", &self.hooks),
            ("init.templateDir", &self.templates),
            ("core.attributesFile", &self.empty),
            ("core.excludesFile", &self.empty),
        ] {
            command.arg("-c").arg(format!("{key}={}", path.display()));
        }
        // These exact setup operations use only our fresh seed and bare origin.
        // Runtime reads have no protocol exception, including the file protocol.
        command.args([
            "-c",
            if setup {
                "protocol.file.allow=always"
            } else {
                "protocol.file.allow=never"
            },
        ]);
        command.args(args);
        command
    }

    fn record(
        &self,
        phase: GitCommandPhase,
        dir: &Path,
        args: &[&str],
        command: &Command,
    ) -> GitCommand {
        GitCommand {
            phase,
            executable: self.executable.clone(),
            dir: dir.into(),
            args: args.iter().map(|arg| (*arg).into()).collect(),
            argv: command
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect(),
            completed: false,
            success: None,
        }
    }
}

fn command_output(command: Command) -> Result<String, String> {
    command_output_bounded(command, Duration::from_secs(10), 128 * 1024)
}

/// Kill and reap a stalled child; stdout and stderr are independently bounded.
/// Reader results use a bounded wait rather than an unconditional thread join,
/// so even an inherited output pipe cannot keep a blocking task alive forever.
fn command_output_bounded(
    mut command: Command,
    timeout: Duration,
    output_limit: usize,
) -> Result<String, String> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("isolated Git: {e}"))?;
    let exceeded = Arc::new(AtomicBool::new(false));
    fn capture(
        reader: impl Read + Send + 'static,
        limit: usize,
        exceeded: Arc<AtomicBool>,
    ) -> mpsc::Receiver<Result<Vec<u8>, String>> {
        let (send, receive) = mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = reader
                .take(limit as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| format!("isolated Git output: {e}"))
                .and_then(|_| {
                    if bytes.len() > limit {
                        exceeded.store(true, Ordering::Release);
                        Err("isolated Git output exceeded its bound".into())
                    } else {
                        Ok(bytes)
                    }
                });
            let _ = send.send(result);
        });
        receive
    }
    let stdout = capture(child.stdout.take().unwrap(), output_limit, exceeded.clone());
    let stderr = capture(child.stderr.take().unwrap(), output_limit, exceeded.clone());
    let started = Instant::now();
    let status = loop {
        if exceeded.load(Ordering::Acquire) || started.elapsed() >= timeout {
            let why = if exceeded.load(Ordering::Acquire) {
                "isolated Git output exceeded its bound"
            } else {
                "isolated Git exceeded its time limit"
            };
            let _ = child.kill();
            let reaped = child.wait();
            break Err(match reaped {
                Ok(_) => why.to_owned(),
                Err(e) => format!("{why}; could not reap child: {e}"),
            });
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => std::thread::sleep(Duration::from_millis(2)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(format!("isolated Git wait: {error}"));
            }
        }
    };
    // No hooks/helpers are permitted, and Git's local built-ins do not leave
    // descendants behind. Still bound capture completion defensively.
    let captured = |receiver: mpsc::Receiver<Result<Vec<u8>, String>>| {
        receiver
            .recv_timeout(Duration::from_millis(250))
            .map_err(|_| "isolated Git output did not close".to_owned())?
    };
    let out = captured(stdout);
    let err = captured(stderr);
    let status = status?;
    let (out, err) = (out?, err?);
    if status.success() {
        Ok(String::from_utf8_lossy(&out).trim().into())
    } else {
        Err(format!(
            "isolated Git {status}: {}",
            String::from_utf8_lossy(&err).trim()
        ))
    }
}

fn path_text(path: &Path) -> anyhow::Result<&str> {
    let text = path.to_str().context("fixture path must be UTF-8")?;
    ensure!(
        !text.chars().any(char::is_control),
        "fixture path contains control characters"
    );
    Ok(text)
}

/// Git's local clone still negotiates refs through upload-pack. Permit only the
/// selected installed Git's own alternate entrypoint, passed as an absolute
/// clone option. Do not expose an installed helper directory through PATH.
fn verified_upload_pack(executable: &Path) -> anyhow::Result<PathBuf> {
    let helper = executable.with_file_name(if cfg!(windows) {
        "git-upload-pack.exe"
    } else {
        "git-upload-pack"
    });
    let same = helper
        .canonicalize()
        .context("installed Git upload-pack entrypoint")?
        == executable;
    #[cfg(unix)]
    let same = {
        use std::os::unix::fs::MetadataExt;
        let git = fs::metadata(executable)?;
        let upload_pack = fs::metadata(&helper)?;
        same || (git.dev() == upload_pack.dev() && git.ino() == upload_pack.ino())
    };
    ensure!(
        same,
        "upload-pack must be an entrypoint of the selected installed Git binary"
    );
    ensure!(
        path_text(&helper)?
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/._-+:".contains(&b)),
        "installed upload-pack path must contain no shell syntax"
    );
    Ok(helper)
}

/// Inspect each component before descending; never canonicalize a replacement
/// symlink or read through a `.git` file supplied by another worktree.
fn plain_directory(path: &Path) -> anyhow::Result<()> {
    ensure!(path.is_absolute(), "fixture directory must be absolute");
    path_text(path)?;
    let mut prefix = PathBuf::new();
    for component in path.components() {
        ensure!(
            !matches!(component, Component::CurDir | Component::ParentDir),
            "fixture path must be canonical"
        );
        prefix.push(component.as_os_str());
        let kind = fs::symlink_metadata(&prefix)?.file_type();
        ensure!(
            kind.is_dir() && !kind.is_symlink(),
            "fixture directory is not a plain directory: {}",
            prefix.display()
        );
    }
    Ok(())
}

fn tree_snapshot(root: &Path) -> anyhow::Result<BTreeMap<PathBuf, Option<Vec<u8>>>> {
    plain_directory(root)?;
    let mut entries = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        ensure!(
            entries.len() < 256,
            "fixture tree grew beyond its bounded snapshot"
        );
        let metadata = fs::symlink_metadata(&path)?;
        let kind = metadata.file_type();
        ensure!(
            !kind.is_symlink(),
            "fixture symlink is forbidden: {}",
            path.display()
        );
        let relative = path.strip_prefix(root)?.to_path_buf();
        if kind.is_dir() {
            entries.insert(relative, None);
            for child in fs::read_dir(path)? {
                pending.push(child?.path());
            }
        } else {
            ensure!(kind.is_file(), "fixture special file is forbidden");
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                ensure!(metadata.nlink() == 1, "fixture hard links are forbidden");
            }
            ensure!(
                metadata.len() <= 1_048_576,
                "fixture file grew beyond its bounded snapshot"
            );
            entries.insert(relative, Some(fs::read(path)?));
        }
    }
    Ok(entries)
}

#[cfg(test)]
mod tests;
