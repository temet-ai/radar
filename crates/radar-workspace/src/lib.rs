//! radar-workspace: how you work, as a radar plugin. A workspace is a top folder
//! of clones you and your agents work across; a ticket is planned in notes
//! before any code.
//!
//! - `config.rs`: the `[workspace]` table, `[[workspace.space]]` entries.
//! - `find.rs`: clones and workspaces on this machine, read without git.
//! - `okf.rs`: the workspace's `.okf.yaml`, which says where knowledge is.
//! - `notes.rs`: a ticket's notes, indexed off the event loop and cached.
//! - `settings.rs`: the `workspaces` settings section.
//!
//! It owns `workspace` and `note` (shown by opening them in an editor) and
//! adds to `ticket` subjects, whoever tracks them: a Notes tab, a Notes
//! section, and the actions `e` the workspace in an editor, a terminal in
//! the chosen folder, `r` look for notes again (GUIDE 1.11b).

mod config;
#[cfg(any(test, feature = "demo"))]
mod demo;
mod find;
mod notes;
mod okf;
mod readme;
mod screen;
mod settings;
mod start;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use radar_core::design::help_bar::KeyHint;
use radar_core::input::TextInput;
use radar_core::journal::{
    ActionDescriptor, Origin, Outcome as JournalOutcome, Reason, SubjectKey, SubjectRole,
    SubjectSnapshot,
};
use radar_core::plugin::{Binding, Effect, Menu, MenuItem, Msg, Outcome, Pane, ScreenSpec, SectionSpec};
use radar_core::system::{self, Env, FolderLaunch, InFolder};
use radar_core::ui::icon::Icon;
use radar_core::{Action, Adds, Contribution, Owned, Plugin, Point, Row, Setup, Showing, Subject};
use serde_json::Value;

use config::{Space, Workspaces};

/// The workspace part: its `[workspace]` table, and what it knows of the
/// notes and clones in each workspace.
#[derive(Debug, Default)]
pub struct WorkspacePlugin {
    pub(crate) cfg: Workspaces,
    pub(crate) state: State,
    pub(crate) now: i64,
}

/// The workspace part while it runs.
#[derive(Debug, Default)]
pub(crate) struct State {
    /// The notes index of each workspace, by its configured folder.
    pub indexes: BTreeMap<String, notes::Index>,
    /// Workspaces being indexed now.
    pub indexing: BTreeSet<String>,
    /// Say how the running index went (the user asked for it).
    pub say: BTreeSet<String>,
    /// Workspaces found on this machine; `None` until looked.
    pub offers: Option<Vec<find::Offer>>,
    pub finding: bool,
    /// Offers the user said not now to, by folder.
    pub passed: BTreeSet<String>,
    /// Editors found installed by the last look.
    pub installed: Vec<(String, String)>,
    /// The selected row of the workspaces section.
    pub row: usize,
    pub edit: Option<(settings::Field, TextInput)>,
    pub screen_selected: Vec<String>,
    pub screen_folded: BTreeSet<String>,
    /// Start work on a ticket, while it runs.
    pub(crate) start: start::State,
}

impl WorkspacePlugin {
    pub fn new() -> Self {
        Self::default()
    }

    /// The workspace a ticket subject is worked in, and its notes.
    fn space_of(&self, s: &Subject) -> Option<&Space> {
        if !s.is(TICKET) {
            return None;
        }
        let project = s
            .field("project")
            .map(str::to_string)
            .or_else(|| s.id.split_once('-').map(|(p, _)| p.to_string()))?;
        self.cfg.for_project(&project)
    }

    /// The workspace and ticket folder (relative to its top) a note about
    /// `subject` is kept in: a ticket's own folder, or the folder of the
    /// ticket whose notes name the merge request. None for any other subject.
    fn note_home(&self, subject: &radar_core::journal::SubjectKey) -> Option<(&Space, String)> {
        let kind = subject.kind.replace('_', "-");
        if kind == TICKET {
            let (project, _) = subject.id.split_once('-')?;
            let space = self.cfg.for_project(project)?;
            let key = subject.id.to_ascii_uppercase();
            let index = self.state.indexes.get(&space.dir);
            let existing = index.and_then(|i| i.folder_of(&key));
            let create = index.map(|i| i.create.as_str()).unwrap_or_default();
            let date = readme::date_of(self.now);
            return Some((space, readme::folder(existing.as_deref(), create, &key, "", &date)));
        }
        if kind != "merge-request" {
            return None;
        }
        for space in &self.cfg.spaces {
            let Some(index) = self.state.indexes.get(&space.dir) else {
                continue;
            };
            let top = space.top();
            let found = index.notes.iter().filter(|n| !n.folder.is_empty()).find(|n| {
                std::fs::read_to_string(top.join(&n.rel))
                    .is_ok_and(|text| readme::names_change(&text, &subject.id))
            });
            if let Some(note) = found {
                return Some((space, note.folder.clone()));
            }
        }
        None
    }

    /// The workspace that holds a file.
    fn space_with(&self, path: &str) -> Option<&Space> {
        self.cfg
            .spaces
            .iter()
            .filter(|s| Path::new(path).starts_with(s.top()))
            .max_by_key(|s| s.top().as_os_str().len())
            .or(self.cfg.spaces.first())
    }
}

const TICKET: &str = "ticket";
const NOTE: &str = "note";
const WORKSPACE: &str = "workspace";

pub(crate) const ADDS: [Adds; 1] = [Adds::new(
    TICKET,
    "its notes (a tab and a section), the workspace in an editor and a terminal",
)];

/// The menu that asks which editor to open with:
/// `workspace-editor:<workspace folder>\t<folder|file>\t<path>`.
pub(crate) const PICK_EDITOR: &str = "workspace-editor";

// --- background work -------------------------------------------------------------------

/// What a task of this part found.
pub(crate) enum WorkMsg {
    Indexed {
        dir: String,
        index: notes::Index,
        observed: find::Observation,
        action: ActionDescriptor,
    },
    Found {
        offers: Vec<find::Offer>,
        observed: find::Observation,
        action: ActionDescriptor,
    },
    /// A read task failed before it could return evidence. Keep its own scope
    /// and state key even if settings changed while it ran.
    ReadFailed {
        dir: Option<String>,
        action: ActionDescriptor,
        why: String,
    },
    /// Typed launch-helper evidence, with UI-only display context.
    Launched { result: FolderLaunch, what: String },
    /// A blocking task did not return an observation.
    Failed(String),
    /// No editor set, or it is not installed: ask which.
    PickEditor {
        menu: String,
        why: String,
        installed: Vec<(String, String)>,
    },
}

/// Blocking work off the event loop, its answer back as a [`WorkMsg`].
fn blocking(f: impl FnOnce() -> WorkMsg + Send + 'static) -> Effect {
    Effect::Spawn(Box::pin(async move {
        let msg = tokio::task::spawn_blocking(f)
            .await
            .unwrap_or_else(|e| WorkMsg::Failed(e.to_string()));
        Box::new(msg) as Msg
    }))
}

/// Read tasks keep their semantic scope out of later sibling effects and retain
/// enough state to release an in-flight guard when spawn_blocking panics.
fn blocking_read(
    action: ActionDescriptor,
    dir: Option<String>,
    f: impl FnOnce(ActionDescriptor) -> WorkMsg + Send + 'static,
) -> Effect {
    let captured = action.clone();
    Effect::SpawnAction {
        action,
        task: Box::pin(async move {
            let completed = captured.clone();
            let msg = tokio::task::spawn_blocking(move || f(completed))
                .await
                .unwrap_or_else(|error| WorkMsg::ReadFailed {
                    dir,
                    action: captured,
                    why: error.to_string(),
                });
            Box::new(msg) as Msg
        }),
    }
}

fn read_action(kind: &'static str, dir: Option<&str>) -> ActionDescriptor {
    let mut action = ActionDescriptor::new(kind, Origin::Plugin("workspace".into()));
    // Discovery deliberately has no subject list: inferred private paths were
    // not requested as journal targets. Its fixed kind defines the search scope.
    if let Some(dir) = dir {
        let subject = config::journal_path(dir).and_then(|path| {
            let top = path.display().to_string();
            SubjectKey::new("workspace", &top, "workspace", &top).ok()
        });
        match subject {
            Some(key) => action
                .subjects
                .push(SubjectSnapshot::new(key, SubjectRole::Primary)),
            None => {
                action.metadata.redacted = true;
                action.metadata.omitted_subjects = 1;
            }
        }
    }
    action
}

fn read_outcome(observed: find::Observation, missing: bool) -> JournalOutcome {
    if observed.incomplete() {
        JournalOutcome::Partial
    } else if missing {
        JournalOutcome::Blocked
    } else {
        JournalOutcome::Succeeded
    }
}

/// Counts are terminal observations at task return, not backfilled execution
/// events. The fixed kind identifies each unit; all file payloads stay UI-only.
fn read_count(
    scope: &ActionDescriptor,
    kind: &'static str,
    count: usize,
    observed: find::Observation,
    outcome: JournalOutcome,
) -> Effect {
    let mut action = scope.clone();
    action.kind = radar_core::journal::StableCode::new(kind);
    action.metadata.count = Some(u32::try_from(count).unwrap_or(u32::MAX));
    action.metadata.truncated = observed.cutoffs != 0 || count > u32::MAX as usize;
    action.metadata.reason = if outcome == JournalOutcome::Blocked {
        Some(Reason::Unavailable)
    } else if observed.errors != 0 {
        Some(Reason::Storage)
    } else {
        None
    };
    Effect::Observe { action, outcome }
}

fn read_gaps(scope: &ActionDescriptor, observed: find::Observation) -> Vec<Effect> {
    let mut effects = Vec::new();
    let index = scope.kind.as_str() == "workspace.notes.index";
    for (kind, count) in [
        (
            if index {
                "workspace.notes.index.errors"
            } else {
                "workspace.discovery.errors"
            },
            observed.errors,
        ),
        (
            if index {
                "workspace.notes.index.limits"
            } else {
                "workspace.discovery.limits"
            },
            observed.cutoffs,
        ),
    ] {
        if count != 0 {
            effects.push(read_count(
                scope,
                kind,
                count as usize,
                observed,
                JournalOutcome::Partial,
            ));
        }
    }
    effects
}

impl WorkspacePlugin {
    /// Index the workspace in `dir` off the event loop.
    pub(crate) fn reindex(&mut self, dir: &str, say: bool) -> Vec<Effect> {
        let Some(space) = self.cfg.spaces.iter().find(|s| s.dir == dir).cloned() else {
            return vec![];
        };
        let action = read_action("workspace.notes.index", Some(&space.dir));
        if !self.state.indexing.insert(dir.to_string()) {
            return vec![
                read_count(
                    &action,
                    "workspace.notes.index.suppressed",
                    1,
                    find::Observation::default(),
                    JournalOutcome::Blocked,
                ),
                Effect::Notify("already looking for notes".into()),
            ];
        }
        if say {
            self.state.say.insert(dir.to_string());
        }
        let (key, now) = (dir.to_string(), self.now);
        let mut fx = vec![blocking_read(action, Some(key.clone()), move |action| {
            let (index, observed) = notes::build_observed(&space, now);
            WorkMsg::Indexed {
                index,
                observed,
                action,
                dir: key,
            }
        })];
        if say {
            fx.push(Effect::Notify("looking for notes…".into()));
        }
        fx
    }

    /// Forget what was known of a workspace's folder (it changed).
    pub(crate) fn forget(&mut self, dir: &str) {
        self.state.indexes.remove(dir);
    }

    /// Look for workspaces on this machine, once.
    fn find_now(&mut self) -> Vec<Effect> {
        let action = read_action("workspace.discovery", None);
        if self.state.finding {
            return vec![read_count(
                &action,
                "workspace.discovery.suppressed",
                1,
                find::Observation::default(),
                JournalOutcome::Blocked,
            )];
        }
        self.state.finding = true;
        let known: Vec<_> = self.cfg.spaces.iter().map(Space::top).collect();
        vec![blocking_read(action, None, move |action| {
            let home = dirs::home_dir();
            let cwd = std::env::current_dir().ok();
            let (offers, observed) = find::discover(cwd.as_deref(), home.as_deref(), &known);
            WorkMsg::Found {
                offers,
                observed,
                action,
            }
        })]
    }

    /// The editor on `target`, or a menu asking which when none is set.
    pub(crate) fn open_editor(&self, space: &Space, target: Target) -> Vec<Effect> {
        vec![
            journal_launch("workspace.editor.launch", space, &target),
            self.editor_task(space, target),
        ]
    }

    /// Keep the original semantic identity when an editor chooser continues.
    fn editor_task(&self, space: &Space, target: Target) -> Effect {
        let (editor, top) = (space.editor.clone(), space.dir.clone());
        blocking(move || {
            let env = Env::detect();
            let prog = system::split_command(&editor)
                .into_iter()
                .next()
                .unwrap_or_default();
            if prog.is_empty() || !system::on_path(&prog) {
                let why = if editor.is_empty() {
                    "no editor is set for this workspace".to_string()
                } else {
                    format!("{prog} is not installed here")
                };
                return WorkMsg::PickEditor {
                    menu: format!("{PICK_EDITOR}:{top}\t{}", target.encode()),
                    why,
                    installed: system::installed_editors(&env),
                };
            }
            start(&env, &editor, &target)
        })
    }

    /// A terminal at `dir`, scoped to the workspace captured at invocation.
    pub(crate) fn open_terminal(&self, space: &Space, dir: String) -> Vec<Effect> {
        vec![
            journal_launch(
                "workspace.terminal.launch",
                space,
                &Target::Folder(dir.clone()),
            ),
            blocking(move || {
                let env = Env::detect();
                let name = Path::new(&dir)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let ways = system::folder_ways(&env, &dir, InFolder::Terminal, "", &name);
                hand_over(ways, &dir, "a terminal")
            }),
        ]
    }
}

/// What to open: a folder in an editor, or one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    Folder(String),
    File(String),
}

impl Target {
    fn encode(&self) -> String {
        match self {
            Target::Folder(p) => format!("folder\t{p}"),
            Target::File(p) => format!("file\t{p}"),
        }
    }

    fn decode(kind: &str, path: &str) -> Option<Self> {
        match kind {
            "folder" => Some(Target::Folder(path.to_string())),
            "file" => Some(Target::File(path.to_string())),
            _ => None,
        }
    }
}

/// Open `target` in `editor`.
fn start(env: &Env, editor: &str, target: &Target) -> WorkMsg {
    let (dir, command) = match target {
        Target::Folder(d) => (d.clone(), editor.to_string()),
        // The editor is given the file as well as its folder.
        Target::File(f) => {
            let parent = Path::new(f)
                .parent()
                .map_or_else(|| ".".to_string(), |p| p.display().to_string());
            let quoted = if f.contains(' ') {
                format!("\"{f}\"")
            } else {
                f.clone()
            };
            (parent, format!("{editor} {quoted}"))
        }
    };
    let name = Path::new(&dir)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ways = system::folder_ways(env, &dir, InFolder::Editor(&command), "", &name);
    hand_over(ways, &dir, editor)
}

/// Start the first way that works; when none can or does, hand the user the
/// command line on their clipboard and say why.
fn hand_over(ways: system::FolderWays, dir: &str, what: &str) -> WorkMsg {
    WorkMsg::Launched {
        result: system::start_folder(ways, Path::new(dir)),
        what: what.into(),
    }
}

/// Capture the configured workspace and exact launch target before dispatch.
/// The target may lie outside the workspace (the existing UI permits that), but
/// its scope still records which workspace supplied the editor configuration.
fn journal_launch(kind: &'static str, space: &Space, target: &Target) -> Effect {
    let mut action = ActionDescriptor::new(kind, Origin::Plugin("workspace".into()));
    let Some(top) = config::journal_path(&space.dir) else {
        action.metadata.redacted = true;
        action.metadata.omitted_subjects = 1;
        return Effect::Journal(action);
    };
    let top = top.display().to_string();
    let (target_kind, path) = match target {
        Target::Folder(path) => ("folder", path),
        Target::File(path) => ("note", path),
    };
    let target = config::journal_path(path).map(|path| path.display().to_string());
    for (kind, id, role) in [
        ("workspace", Some(top.as_str()), SubjectRole::Primary),
        (target_kind, target.as_deref(), SubjectRole::Destination),
    ] {
        match id.and_then(|id| SubjectKey::new("workspace", &top, kind, id).ok()) {
            Some(key) => action.subjects.push(SubjectSnapshot::new(key, role)),
            None => {
                action.metadata.redacted = true;
                action.metadata.omitted_subjects += 1;
            }
        }
    }
    Effect::Journal(action)
}

impl WorkspacePlugin {
    fn on_work(&mut self, msg: WorkMsg) -> Vec<Effect> {
        match msg {
            WorkMsg::Indexed {
                dir,
                mut index,
                observed,
                action,
            } => {
                index.at = self.now;
                self.state.indexing.remove(&dir);
                let say = self.state.say.remove(&dir);
                let outcome = read_outcome(observed, index.missing);
                let mut fx = vec![Effect::Report(outcome)];
                // Keep the UI/cache's existing encountered-entry count: it
                // includes failed reads and the first file past a cutoff.
                for (kind, count) in [
                    ("workspace.notes.index.files.encountered", index.files),
                    ("workspace.notes.index.notes", index.notes.len()),
                    ("workspace.notes.index.clones", index.clones),
                ] {
                    fx.push(read_count(&action, kind, count, observed, outcome));
                }
                fx.extend(read_gaps(&action, observed));
                fx.push(Effect::PersistCache);
                if say {
                    fx.push(if index.missing {
                        Effect::error(None, format!("{} is not a folder here", config::tilde(&dir)))
                    } else {
                        Effect::Notify(format!(
                            "found {} note{} in {} files",
                            index.notes.len(),
                            if index.notes.len() == 1 { "" } else { "s" },
                            index.files
                        ))
                    });
                }
                self.state.indexes.insert(dir, index);
                fx
            }
            WorkMsg::Found {
                offers,
                observed,
                action,
            } => {
                self.state.finding = false;
                let outcome = read_outcome(observed, false);
                let mut fx = vec![
                    Effect::Report(outcome),
                    read_count(
                        &action,
                        "workspace.discovery.results",
                        offers.len(),
                        observed,
                        outcome,
                    ),
                ];
                fx.extend(read_gaps(&action, observed));
                self.state.offers = Some(offers);
                fx.push(Effect::PersistCache);
                fx
            }
            WorkMsg::ReadFailed {
                dir,
                mut action,
                why,
            } => {
                if let Some(dir) = dir {
                    self.state.indexing.remove(&dir);
                    self.state.say.remove(&dir);
                    action.kind =
                        radar_core::journal::StableCode::new("workspace.notes.index.interrupted");
                } else {
                    self.state.finding = false;
                    action.kind =
                        radar_core::journal::StableCode::new("workspace.discovery.interrupted");
                }
                action.metadata.reason = Some(Reason::Interrupted);
                vec![
                    Effect::Report(JournalOutcome::Unknown),
                    Effect::Observe {
                        action,
                        outcome: JournalOutcome::Unknown,
                    },
                    Effect::error(None, radar_core::writes::failed_line(&why)),
                ]
            }
            WorkMsg::Launched { result, what } => {
                let (outcome, said, copy) = match result {
                    FolderLaunch::Dispatched(started) => {
                        (JournalOutcome::Succeeded, format!("opened {started}"), None)
                    }
                    FolderLaunch::Unavailable { why, by_hand } => (
                        JournalOutcome::Blocked,
                        format!("{why}: `{by_hand}` is on your clipboard"),
                        Some(by_hand),
                    ),
                    FolderLaunch::Unconfirmed { why, by_hand } => (
                        JournalOutcome::Unknown,
                        format!("could not open {what} ({why}): `{by_hand}` is on your clipboard"),
                        Some(by_hand),
                    ),
                };
                let mut fx = vec![Effect::Report(outcome)];
                fx.extend(copy.map(Effect::Copy));
                fx.push(if outcome == JournalOutcome::Succeeded { Effect::Notify(said) } else { Effect::error(None, said) });
                fx
            }
            WorkMsg::Failed(why) => vec![
                Effect::Report(JournalOutcome::Unknown),
                Effect::error(None, radar_core::writes::failed_line(&why)),
            ],
            WorkMsg::PickEditor {
                menu,
                why,
                installed,
            } => {
                self.state.installed = installed.clone();
                if installed.is_empty() {
                    return vec![
                        Effect::Report(JournalOutcome::Blocked),
                        Effect::error(None, format!(
                            "{why}, and radar found no editor installed: set one in settings, workspace › workspaces"
                        )),
                    ];
                }
                let items = installed
                    .iter()
                    .map(|(cmd, name)| {
                        MenuItem::pick(
                            name.clone(),
                            format!("{cmd} · and for this workspace from now on"),
                            cmd.clone(),
                        )
                    })
                    .collect();
                vec![
                    Effect::Notify(why),
                    Effect::Menu(Menu::new(menu, "open with…", items)),
                ]
            }
        }
    }

    /// The user picked an editor: keep it for the workspace, and open.
    fn picked(&mut self, arg: &str, editor: &str) -> Vec<Effect> {
        let mut parts = arg.splitn(3, '\t');
        let (dir, kind, path) = (
            parts.next().unwrap_or(""),
            parts.next().unwrap_or(""),
            parts.next().unwrap_or(""),
        );
        let Some(target) = Target::decode(kind, path) else {
            return vec![];
        };
        let Some(space) = self.cfg.spaces.iter_mut().find(|s| s.dir == dir) else {
            return vec![];
        };
        space.editor = editor.to_string();
        let space = space.clone();
        let mut fx = vec![Effect::Notify(format!(
            "{} opens in {editor}",
            space.label()
        ))];
        fx.push(self.editor_task(&space, target));
        fx
    }

    /// What `e` and the terminal open at for a ticket: the workspace's top,
    /// or (`open_at = "repo"`) the folder its notes are in.
    fn open_dir(&self, space: &Space, key: &str) -> String {
        let top = space.top();
        if space.open_at == "repo"
            && let Some(folder) = self
                .state
                .indexes
                .get(&space.dir)
                .and_then(|i| i.folder_of(key))
            && top.join(&folder).is_dir()
        {
            return top.join(folder).display().to_string();
        }
        top.display().to_string()
    }

    // --- joining the ticket's owner ------------------------------------------------

    fn tab(&self, space: &Space, key: &str) -> Option<Contribution> {
        let index = self.state.indexes.get(&space.dir)?;
        let found = index.for_ticket(key);
        if found.is_empty() {
            return None;
        }
        let rows = found
            .into_iter()
            .map(|n| Row {
                line: radar_core::view::Text::plain(notes::line(n)),
                subject: Some(note_subject(n)),
            })
            .collect();
        Some(Contribution::Tab {
            title: "Notes".into(),
            rows,
        })
    }

    fn section(&self, space: &Space, key: &str) -> Option<Contribution> {
        let index = self.state.indexes.get(&space.dir)?;
        let readme = index
            .for_ticket(key)
            .into_iter()
            .find(|n| n.readme && !n.lead.is_empty())?;
        Some(Contribution::Section {
            icon: Some(radar_core::ui::icon::Icon::Folder),
            title: "Notes".into(),
            lines: readme.lead.iter().map(|l| radar_core::view::Text::plain(l.clone())).collect(),
        })
    }

    fn actions(&self, space: &Space, key: &str) -> Contribution {
        let editor = if space.editor.is_empty() {
            format!("open {} in an editor…", space.label())
        } else {
            format!("open {} in {}", space.label(), space.editor)
        };
        // Contributions use cached data only. Whether this folder still exists
        // is checked when acting; either it or the workspace top may be opened.
        let terminal = if space.open_at == "repo"
            && self
                .state
                .indexes
                .get(&space.dir)
                .and_then(|index| index.folder_of(key))
                .is_some()
        {
            format!("open a terminal for {key} in {}", space.label())
        } else {
            format!("open a terminal at the top of {}", space.label())
        };
        let mut terminal = Action::new('t', terminal, "terminal").icon(Icon::Terminal);
        // Other parts use `t`; this row is chosen from the menu then.
        terminal.key = None;
        Contribution::Actions(vec![
            start::action(key),
            Action::new('e', editor, "editor").icon(Icon::Editor),
            terminal,
            Action::new('r', "look for notes again", "reindex").icon(Icon::Search),
        ])
    }
}

/// A note as a subject: its path is its id, its file its link.
pub(crate) fn note_subject(n: &notes::Note) -> Subject {
    Subject::new(NOTE, &n.path, &n.name)
        .with_url(format!("file://{}", n.path))
        .with("ticket", &n.key)
        .with("type", &n.kind)
        .with("status", &n.status)
        .with("title", &n.title)
}

impl Plugin for WorkspacePlugin {
    fn id(&self) -> &'static str {
        "workspace"
    }

    fn name(&self) -> &'static str {
        "Workspace"
    }

    fn screens(&self) -> Vec<ScreenSpec> {
        vec![screen::SCREEN]
    }

    fn save_note(
        &mut self,
        request: &radar_core::plugin::NoteRequest,
    ) -> Option<Result<std::path::PathBuf, String>> {
        let (space, folder) = self.note_home(&request.subject)?;
        if !readme::plain_file_name(&request.file_name) {
            return Some(Err(format!("{:?} is not a file name", request.file_name)));
        }
        let dir = space.top().join(folder);
        let path = dir.join(&request.file_name);
        // A name that is taken keeps its file: the same text is already
        // saved; other text (a second review of the same sha) goes under
        // the next free name, so no finished review is lost.
        let mut path = path;
        let (stem, ext) = match request.file_name.rsplit_once('.') {
            Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
            _ => (request.file_name.clone(), String::new()),
        };
        let mut n = 1;
        while path.is_file() {
            if std::fs::read_to_string(&path).is_ok_and(|t| t == request.text) {
                return Some(Ok(path));
            }
            n += 1;
            path = dir.join(format!("{stem}-{n}{ext}"));
        }
        let written = std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, &request.text));
        Some(
            written
                .map(|()| path)
                .map_err(|e| format!("workspace: could not save the note in {}: {e}", dir.display())),
        )
    }

    fn screen_group(&self, _: &str) -> Option<radar_core::ScreenGroup> {
        Some(radar_core::ScreenGroup {
            id: "repositories",
            name: "repositories",
            key: 'R',
            source: "Local workspaces",
        })
    }

    fn panes(&self, _: &str) -> Vec<Pane> {
        vec![Pane::Middle, Pane::Detail]
    }

    fn view(&self, screen: &str, cx: &radar_core::view::ViewCx<'_>) -> Option<radar_core::view::ScreenView> {
        (screen == screen::SCREEN.id).then(|| screen::view(self, cx))
    }

    fn folds(&self, scope: &str, pane: Pane) -> bool {
        scope == screen::SCREEN.id && pane == Pane::Middle
    }

    fn keys(&self, _: &str, pane: Pane) -> Vec<Binding> {
        screen::keys(self, pane)
    }

    fn top_right(&self, _: &str) -> Option<String> {
        Some(format!("{} local workspaces", self.cfg.spaces.len()))
    }

    fn configure(&mut self, table: Option<&toml::Value>) -> anyhow::Result<()> {
        self.cfg = match table {
            Some(t) => t.clone().try_into()?,
            None => Workspaces::default(),
        };
        self.cfg.tidy();
        Ok(())
    }

    /// The table, left out while it is the default.
    fn config(&self) -> Option<toml::Value> {
        (!self.cfg.is_default())
            .then(|| toml::Value::try_from(&self.cfg).ok())
            .flatten()
    }

    #[cfg(feature = "demo")]
    fn demo(&mut self) {
        *self = demo::sample();
    }

    fn restore(&mut self, cache: Value) {
        if let Ok(indexes) = serde_json::from_value(cache["indexes"].clone()) {
            self.state.indexes = indexes;
        }
        if let Ok(offers) = serde_json::from_value(cache["offers"].clone()) {
            self.state.offers = Some(offers);
        }
    }

    fn snapshot(&self) -> Option<Value> {
        Some(serde_json::json!({ "indexes": self.state.indexes, "offers": self.state.offers }))
    }

    /// Without a workspace it has nothing to add anywhere.
    fn set_up(&self) -> Setup {
        if self.cfg.spaces.is_empty() {
            Setup::Needed("no workspace yet: add one in settings, workspace › workspaces".into())
        } else {
            Setup::Ready
        }
    }

    fn journal_subject(&self, subject: &Subject) -> Option<radar_core::journal::SubjectSnapshot> {
        if !subject.is(WORKSPACE) && !subject.is(NOTE) {
            return None;
        }
        let target = config::journal_path(&subject.id)?;
        let top = self
            .cfg
            .spaces
            .iter()
            .filter_map(|space| config::journal_path(&space.dir))
            .filter(|top| target.starts_with(top))
            .max_by_key(|top| top.as_os_str().len())?;
        let key = radar_core::journal::SubjectKey::new(
            "workspace",
            &top.display().to_string(),
            &subject.kind,
            &target.display().to_string(),
        )
        .ok()?;
        Some(radar_core::journal::SubjectSnapshot::new(
            key,
            radar_core::journal::SubjectRole::Primary,
        ))
    }

    fn owns(&self) -> Vec<Owned> {
        vec![
            Owned::link(WORKSPACE, "workspaces"),
            Owned::link(NOTE, "notes"),
        ]
    }

    /// A note opens its file in the workspace's editor; a workspace opens
    /// its folder.
    fn show(&mut self, s: &Subject) -> Showing {
        let target = if s.is(NOTE) {
            Target::File(s.id.clone())
        } else if s.is(WORKSPACE) {
            Target::Folder(s.id.clone())
        } else {
            return Showing::Cannot;
        };
        let Some(space) = self.space_with(&s.id).cloned() else {
            return Showing::Cannot;
        };
        Showing::Done(self.open_editor(&space, target))
    }

    fn tick(&mut self, now: i64) -> Vec<Effect> {
        self.now = now;
        let mut fx = Vec::new();
        let todo: Vec<String> = self
            .cfg
            .spaces
            .iter()
            .filter(|s| {
                self.state
                    .indexes
                    .get(&s.dir)
                    .is_none_or(|index| index.clones > 0 && index.repositories.is_empty())
                    && !self.state.indexing.contains(&s.dir)
            })
            .map(|s| s.dir.clone())
            .collect();
        for dir in todo {
            fx.extend(self.reindex(&dir, false));
        }
        if self.state.offers.is_none() && !self.state.finding {
            fx.extend(self.find_now());
        }
        fx
    }

    fn on_msg(&mut self, msg: Msg) -> Vec<Effect> {
        match msg.downcast::<WorkMsg>() {
            Ok(m) => self.on_work(*m),
            Err(m) => match m.downcast::<start::StartMsg>() {
                Ok(m) => self.start_msg(*m),
                Err(_) => vec![],
            },
        }
    }

    fn capturing(&self) -> bool {
        self.state.edit.is_some()
    }

    fn paste(&mut self, text: &str) -> Vec<Effect> {
        if let Some((_, input)) = self.state.edit.as_mut() {
            input.paste(text);
        }
        vec![]
    }

    fn adds(&self) -> Vec<Adds> {
        ADDS.to_vec()
    }

    fn contribute(&self, point: Point, s: &Subject) -> Option<Contribution> {
        let space = self.space_of(s)?;
        match point {
            Point::Tab => self.tab(space, &s.id),
            Point::Section => self.section(space, &s.id),
            Point::Actions => Some(self.actions(space, &s.id)),
            _ => None,
        }
    }

    fn act(&mut self, s: &Subject, action: &str) -> Vec<Effect> {
        self.act_with_related(s, action, &[])
    }

    fn act_with_related(&mut self, s: &Subject, action: &str, related: &[Subject]) -> Vec<Effect> {
        let Some(space) = self.space_of(s).cloned() else {
            return vec![];
        };
        let dir = self.open_dir(&space, &s.id);
        match action {
            "editor" => self.open_editor(&space, Target::Folder(dir)),
            "terminal" => self.open_terminal(&space, dir),
            "reindex" => self.reindex(&space.dir, true),
            start::ACTION => self.start_act_related(s, &space, related),
            _ => vec![],
        }
    }

    fn on_bus(&mut self, topic: &str, payload: &Value) -> Vec<Effect> {
        self.start_bus(topic, payload)
    }

    fn typing(&self, menu: &str, text: &str) -> radar_core::Typing {
        settings::typing(self, menu, text)
    }

    fn on_menu(&mut self, menu: &str, value: &str) -> Vec<Effect> {
        if let Some(fx) = self.start_menu(menu, value) {
            return fx;
        }
        if let Some(arg) = menu.strip_prefix(&format!("{PICK_EDITOR}:")) {
            return self.picked(arg, value);
        }
        settings::on_menu(self, menu, value).unwrap_or_default()
    }

    // --- settings ------------------------------------------------------------------

    fn sections(&self) -> Vec<SectionSpec> {
        settings::SECTIONS.to_vec()
    }

    fn section_icon(&self, section: &str) -> Option<Icon> {
        (section == "workspaces").then_some(Icon::Local)
    }

    fn section_view(&self, section: &str, cx: &radar_core::view::ViewCx<'_>) -> Option<radar_core::view::SectionView> {
        settings::section_view(self, section, cx)
    }

    fn intent(&mut self, scope: &str, pane: Pane, intent: &radar_core::view::Intent) -> Outcome {
        if scope == screen::SCREEN.id { screen::intent(self, pane, intent) }
        else { settings::intent(self, scope, intent) }
    }

    fn section_hints(&self, _: &str) -> Vec<KeyHint<'static>> {
        settings::hints(self)
    }

    fn bulk(&self, scope: &str) -> Vec<MenuItem> {
        settings::bulk(self, scope)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod launch_tests;

#[cfg(test)]
mod read_tests;

#[cfg(test)]
mod action_labels_tests;

#[cfg(test)]
mod save_note_tests {
    use super::*;
    use crate::find::fixture::Scratch;
    use radar_core::journal::SubjectKey;
    use radar_core::plugin::NoteRequest;

    const OKF: &str = "sources:\n  artifacts:\n    - name: art\n      include:\n        - artifacts/**/*.md\ncreate:\n  locations:\n    artifacts:\n      path: artifacts/{date}\n";
    const README: &str = "---\ntype: ticket\nticket: ABC-12\n---\n# ABC-12\n\nReview: https://gitlab.example.test/acme/api/-/merge_requests/263\n";

    fn plugin(s: &Scratch) -> WorkspacePlugin {
        let mut space = Space::new(s.0.display().to_string());
        space.tickets = vec!["ABC".into()];
        let mut p = WorkspacePlugin::new();
        p.cfg.spaces.push(space.clone());
        p.state.indexes.insert(space.dir.clone(), notes::build(&space, 0));
        p
    }

    fn mr(id: &str) -> SubjectKey {
        SubjectKey::new("forge", "https://forge.example.test", "merge_request", id).unwrap()
    }

    fn note(subject: SubjectKey, file_name: &str) -> NoteRequest {
        NoteRequest { subject, file_name: file_name.into(), text: "reviewed abc123\n".into() }
    }

    #[test]
    fn a_review_is_saved_in_the_folder_of_the_ticket_whose_notes_name_the_mr() {
        let s = Scratch::new("save-note");
        s.file(".okf.yaml", OKF);
        s.file("artifacts/2026-01-01/ABC-12-stats/README.md", README);
        let mut p = plugin(&s);
        let path = p.save_note(&note(mr("acme/api!263"), "review-abc123.md")).unwrap().unwrap();
        assert_eq!(path, s.path("artifacts/2026-01-01/ABC-12-stats/review-abc123.md"));
        assert_eq!(std::fs::read_to_string(path).unwrap(), "reviewed abc123\n");
    }

    #[test]
    fn a_ticket_without_notes_gets_the_create_location() {
        let s = Scratch::new("save-note-new");
        s.file(".okf.yaml", OKF);
        s.file("artifacts/2026-01-01/ABC-12-stats/README.md", README);
        let mut p = plugin(&s);
        p.now = 1_700_000_000;
        let ticket = SubjectKey::new("tracker", "https://tracker.example.test", "ticket", "ABC-99").unwrap();
        let path = p.save_note(&note(ticket, "review.md")).unwrap().unwrap();
        assert!(path.starts_with(s.path("artifacts/2023-11-14")), "{path:?}");
        assert!(path.ends_with("review.md") && path.is_file());
    }

    #[test]
    fn a_note_already_saved_under_that_name_is_kept() {
        let s = Scratch::new("save-note-keep");
        s.file(".okf.yaml", OKF);
        s.file("artifacts/2026-01-01/ABC-12-stats/README.md", README);
        let mut p = plugin(&s);
        let first = p.save_note(&note(mr("acme/api!263"), "r.md")).unwrap().unwrap();
        let mut again = note(mr("acme/api!263"), "r.md");
        again.text = "other\n".into();
        // The same text again is already saved.
        let same = note(mr("acme/api!263"), "r.md");
        assert_eq!(p.save_note(&same).unwrap().unwrap(), first);
        // A second review of the same sha is kept beside the first.
        let second = p.save_note(&again).unwrap().unwrap();
        assert_eq!(second, s.path("artifacts/2026-01-01/ABC-12-stats/r-2.md"));
        assert_eq!(std::fs::read_to_string(first).unwrap(), "reviewed abc123\n");
        assert_eq!(std::fs::read_to_string(second).unwrap(), "other\n");
        again.text = "third\n".into();
        let third = p.save_note(&again).unwrap().unwrap();
        assert!(third.ends_with("r-3.md") && third.is_file());
    }

    #[test]
    fn an_mr_reference_must_start_at_a_word_edge_or_its_hosts_slash() {
        let s = Scratch::new("save-note-edge");
        s.file(".okf.yaml", OKF);
        s.file(
            "artifacts/2026-01-01/ABC-12-stats/README.md",
            "---\nticket: ABC-12\n---\nsee bigacme/api!263 and other/acme/api!263 and https://x.test/other/acme/api/-/merge_requests/263\n",
        );
        let mut p = plugin(&s);
        assert!(p.save_note(&note(mr("acme/api!263"), "r.md")).is_none());
        s.file(
            "artifacts/2026-01-01/ABC-12-stats/README.md",
            "---\nticket: ABC-12\n---\nsee (acme/api!263)\n",
        );
        let mut p = plugin(&s);
        assert!(p.save_note(&note(mr("acme/api!263"), "r.md")).is_some());
    }

    #[test]
    fn unknown_subjects_and_other_merge_requests_are_left_to_the_host() {
        let s = Scratch::new("save-note-none");
        s.file(".okf.yaml", OKF);
        s.file("artifacts/2026-01-01/ABC-12-stats/README.md", README);
        let mut p = plugin(&s);
        // The number is taken whole: !26 is not !263.
        assert!(p.save_note(&note(mr("acme/api!26"), "r.md")).is_none());
        assert!(p.save_note(&note(mr("acme/web!263"), "r.md")).is_none());
        let message = SubjectKey::new("chat", "T1", "message", "C1/1.2").unwrap();
        assert!(p.save_note(&note(message, "r.md")).is_none());
    }

    #[test]
    fn a_file_name_with_a_folder_is_refused() {
        let s = Scratch::new("save-note-name");
        s.file(".okf.yaml", OKF);
        s.file("artifacts/2026-01-01/ABC-12-stats/README.md", README);
        let mut p = plugin(&s);
        let refused = p.save_note(&note(mr("acme/api!263"), "../x.md")).unwrap();
        assert!(refused.is_err());
        assert!(!s.path("artifacts/2026-01-01/x.md").exists());
    }
}
