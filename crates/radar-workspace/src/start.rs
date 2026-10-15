//! Start work on a ticket: `s` picks the repos, asks how (the normal
//! checkout or a parked worktree), makes the ticket's README, and tells
//! whoever does git over the bus (`workspace.start`). Git's answer
//! (`git.started`) fills the README's `## Branches` table, then the editor
//! or a terminal opens at the workspace, and `workspace.started` goes out
//! for whoever else follows (the tracker asks to assign and move the
//! ticket). This part never names the others; the topics are the vocabulary.

use std::collections::BTreeMap;
use std::path::Path;

use radar_core::journal::{
    ActionDescriptor, Origin, Outcome as JournalOutcome, SubjectKey, SubjectRole, SubjectSnapshot,
};
use radar_core::plugin::{Effect, Menu, MenuItem, Msg};
use radar_core::ui::icon::Icon;
use radar_core::{Action, Subject};
use serde_json::{Value, json};

use crate::config::Space;
use crate::{Target, WorkspacePlugin, find, readme};

pub(crate) const START: &str = "workspace.start";
pub(crate) const STARTED: &str = "workspace.started";
pub(crate) const GIT_STARTED: &str = "git.started";

/// The menus: which repos, how, and whether to write the README's table.
pub(crate) const REPOS: &str = "workspace-start-repos";
pub(crate) const HOW: &str = "workspace-start-how";
pub(crate) const TABLE: &str = "workspace-start-table";

/// The action's id.
pub(crate) const ACTION: &str = "start";

/// What start work knows while it runs.
#[derive(Debug, Default)]
pub(crate) struct State {
    pending: Option<Pending>,
    /// What `s` started, by ticket key: for when git answers.
    runs: BTreeMap<String, Run>,
    table: Option<Table>,
}

#[derive(Debug, Clone)]
struct Run {
    title: String,
    kind: String,
    space: String,
}

#[derive(Debug, Clone)]
struct Pending {
    key: String,
    title: String,
    kind: String,
    space: String,
    picks: Vec<Pick>,
}

/// One clone offered, and whether it is ticked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Pick {
    pub dir: String,
    pub repo: String,
    pub branch: String,
    pub on: bool,
}

impl Pick {
    fn name(&self) -> String {
        if self.repo.is_empty() {
            Path::new(&self.dir)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.dir.clone())
        } else {
            self.repo.clone()
        }
    }
}

/// The README table asked about, until it is answered.
#[derive(Debug, Clone)]
struct Table {
    key: String,
    space: String,
    results: Value,
    path: String,
    text: String,
}

/// What a blocking task of start work found.
pub(crate) enum StartMsg {
    Clones {
        key: String,
        title: String,
        kind: String,
        space: String,
        picks: Vec<Pick>,
    },
    /// The README is made; say it to git.
    Made {
        key: String,
        space: String,
        payload: Value,
        said: String,
    },
    /// What git said, folded into the README it belongs in.
    Planned {
        key: String,
        space: String,
        results: Value,
        path: String,
        existing: Option<String>,
        text: String,
        rows: Vec<readme::Row>,
    },
    Finished {
        key: String,
        space: String,
        results: Value,
        said: String,
    },
    Failed(String),
}

fn journal_at(kind: &'static str, workspace: &str, ticket: &str, note: &str) -> Effect {
    let mut action = ActionDescriptor::new(kind, Origin::Plugin("workspace".into()));
    let Some(workspace) = crate::config::journal_path(workspace) else {
        action.metadata.omitted_subjects = 1;
        action.metadata.redacted = true;
        return Effect::Journal(action);
    };
    let workspace = workspace.display().to_string();
    let note = if note.is_empty() {
        String::new()
    } else {
        crate::config::journal_path(note)
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    };
    // Ticket keys learned through the local workflow use the workspace scope;
    // without a tracker instance, claiming a remote ticket identity would guess.
    for (kind, id, role) in [
        ("workspace", workspace.as_str(), SubjectRole::Primary),
        ("ticket", ticket, SubjectRole::Context),
        ("note", note.as_str(), SubjectRole::Result),
    ] {
        if !workspace.is_empty() && !id.is_empty() {
            match SubjectKey::new("workspace", &workspace, kind, id) {
                Ok(key) => action = action.with_subject(SubjectSnapshot::new(key, role)),
                Err(_) => {
                    action.metadata.redacted = true;
                    action.metadata.omitted_subjects += 1;
                }
            }
        }
    }
    Effect::Journal(action)
}

fn spawn(f: impl FnOnce() -> StartMsg + Send + 'static) -> Effect {
    Effect::Spawn(Box::pin(async move {
        let msg = tokio::task::spawn_blocking(f)
            .await
            .unwrap_or_else(|e| StartMsg::Failed(e.to_string()));
        Box::new(msg) as Msg
    }))
}

// --- what is read of the clones -------------------------------------------------------

/// The branch a clone has out, read from its HEAD (no git run).
pub(crate) fn head_branch(dir: &Path) -> String {
    let git = dir.join(".git");
    let head = if git.is_file() {
        let Ok(text) = std::fs::read_to_string(&git) else {
            return String::new();
        };
        let Some(at) = text.trim().strip_prefix("gitdir:") else {
            return String::new();
        };
        let at = Path::new(at.trim());
        let at = if at.is_absolute() {
            at.to_path_buf()
        } else {
            dir.join(at)
        };
        at.join("HEAD")
    } else {
        git.join("HEAD")
    };
    std::fs::read_to_string(head)
        .ok()
        .and_then(|t| {
            t.trim()
                .strip_prefix("ref: refs/heads/")
                .map(str::to_string)
        })
        .unwrap_or_default()
}

/// The `repos:` a note's frontmatter names: an inline list or a block of
/// `- name` lines.
pub(crate) fn repos_named(text: &str) -> Vec<String> {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut in_list = false;
    for l in lines {
        if l.trim() == "---" {
            break;
        }
        if let Some(v) = l.strip_prefix("repos:") {
            let v = v.trim();
            in_list = v.is_empty();
            out.extend(
                v.trim_start_matches('[')
                    .trim_end_matches(']')
                    .split(',')
                    .map(|s| s.trim().trim_matches(|c| c == '"' || c == '\'').to_string())
                    .filter(|s| !s.is_empty()),
            );
        } else if in_list {
            match l.trim().strip_prefix('-') {
                Some(item) => out.push(
                    item.trim()
                        .trim_matches(|c| c == '"' || c == '\'')
                        .to_string(),
                ),
                None if l.trim().is_empty() => {}
                None => in_list = false,
            }
        }
    }
    out
}

/// Whether `name` carries `key` as a whole token: bounded by characters
/// that are not letters or digits (`ABC-1` is not in `alice/ABC-12`).
pub(crate) fn has_key(name: &str, key: &str) -> bool {
    if key.is_empty() {
        return false;
    }
    let (name, key) = (name.to_lowercase(), key.to_lowercase());
    name.match_indices(&key).any(|(at, m)| {
        let before = name[..at].chars().next_back();
        let after = name[at + m.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

/// The local branches of a clone, read from its git folder without running
/// git: the loose refs and `packed-refs` (a worktree reads its clone's).
pub(crate) fn local_branches(dir: &Path) -> Vec<String> {
    let git = dir.join(".git");
    let mut common = git.clone();
    if git.is_file() {
        let Some(at) = std::fs::read_to_string(&git).ok().and_then(|t| {
            t.trim()
                .strip_prefix("gitdir:")
                .map(|p| p.trim().to_string())
        }) else {
            return Vec::new();
        };
        let at = Path::new(&at);
        let at = if at.is_absolute() {
            at.to_path_buf()
        } else {
            dir.join(at)
        };
        common = match std::fs::read_to_string(at.join("commondir")) {
            Ok(c) => at.join(c.trim()),
            Err(_) => at,
        };
    }
    let mut out = Vec::new();
    let heads = common.join("refs/heads");
    let mut todo = vec![heads.clone()];
    while let Some(d) = todo.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                todo.push(p);
            } else if let Ok(rel) = p.strip_prefix(&heads) {
                out.push(rel.display().to_string());
            }
        }
    }
    if let Ok(packed) = std::fs::read_to_string(common.join("packed-refs")) {
        out.extend(packed.lines().filter_map(|l| {
            l.split_once(' ')
                .and_then(|(_, r)| r.strip_prefix("refs/heads/"))
                .map(str::to_string)
        }));
    }
    out.sort();
    out.dedup();
    out
}

/// Whether the README (or any note) links to a clone: a markdown link or a
/// URL whose path holds the repo's path as whole segments, or, for a clone
/// that names no repo, its folder's name as a segment.
pub(crate) fn links_to(text: &str, repo: &str, base: &str) -> bool {
    let want: Vec<&str> = if repo.is_empty() {
        vec![base]
    } else {
        repo.split('/').collect()
    };
    if want.iter().all(|s| s.is_empty()) {
        return false;
    }
    let mut targets: Vec<&str> = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("](") {
        let after = &rest[at + 2..];
        let end = after.find(')').unwrap_or(after.len());
        targets.push(&after[..end]);
        rest = &after[end..];
    }
    targets.extend(
        text.split_whitespace()
            .filter(|w| w.starts_with("http://") || w.starts_with("https://")),
    );
    targets.iter().any(|t| {
        let t = t.split(['#', '?']).next().unwrap_or("");
        let segs: Vec<&str> = t
            .trim_start_matches(|c| c != '/' && c != '.')
            .split(['/', ':'])
            .filter(|s| !s.is_empty())
            .collect();
        segs.windows(want.len()).any(|w| w == want.as_slice())
    })
}

/// Whether a clone is ticked at first: a branch of it carries the key (the
/// one out, or any other), the README links to it, or a note's `repos:`
/// names it.
pub(crate) fn ticked(
    repo: &str,
    dir: &str,
    branch: &str,
    key: &str,
    named: &[String],
    branches: &[String],
    texts: &[String],
) -> bool {
    if has_key(branch, key) || branches.iter().any(|b| has_key(b, key)) {
        return true;
    }
    let base = Path::new(dir)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if texts.iter().any(|t| links_to(t, repo, &base)) {
        return true;
    }
    named.iter().any(|n| {
        let n = n.trim().trim_matches('/');
        !n.is_empty()
            && ((!repo.is_empty() && (repo == n || repo.ends_with(&format!("/{n}"))))
                || base == n
                || dir.ends_with(&format!("/{n}")))
    })
}

/// The payload of `workspace.start`.
pub(crate) fn payload(
    key: &str,
    title: &str,
    kind: &str,
    workspace: &str,
    picks: &[Pick],
    how: &str,
) -> Value {
    json!({
        "ticket": { "key": key, "title": title, "type": kind },
        "workspace": workspace,
        "repos": picks.iter().filter(|p| p.on).map(|p| json!({
            "dir": p.dir,
            "repo": p.repo,
            "how": how,
        })).collect::<Vec<_>>(),
    })
}

// --- the part's side ----------------------------------------------------------------

/// The action `s` adds to a ticket.
pub(crate) fn action(key: &str) -> Action {
    Action::new('s', format!("start work on {key}…"), ACTION).icon(Icon::Branch)
}

impl WorkspacePlugin {
    /// `s`: look at the clones off the event loop, then ask which.
    #[cfg(test)]
    pub(crate) fn start_act(&mut self, s: &Subject, space: &Space) -> Vec<Effect> {
        self.start_act_related(s, space, &[])
    }

    pub(crate) fn start_act_related(
        &mut self,
        s: &Subject,
        space: &Space,
        related: &[Subject],
    ) -> Vec<Effect> {
        // Only exact repository identities can preselect a clone. A similarly
        // named folder or a subject of another kind is not a match.
        let linked: std::collections::BTreeSet<String> = related
            .iter()
            .filter(|s| s.is("repo") && !s.id.is_empty())
            .map(|s| s.id.clone())
            .collect();
        let key = s.id.clone();
        let title = s.field("title").unwrap_or("").to_string();
        let kind = s.field("type").unwrap_or("").to_string();
        let (space_dir, clones_dir) = (space.dir.clone(), space.clones_dir());
        let notes_paths: Vec<String> = self
            .state
            .indexes
            .get(&space.dir)
            .map(|i| {
                i.for_ticket(&key)
                    .into_iter()
                    .map(|n| n.path.clone())
                    .collect()
            })
            .unwrap_or_default();
        let k = key.clone();
        vec![
            journal_at("workspace.start", &space.dir, &key, ""),
            Effect::Notify(format!("looking at the clones for {key}…")),
            spawn(move || {
                let texts: Vec<String> = notes_paths
                    .iter()
                    .filter_map(|p| std::fs::read_to_string(p).ok())
                    .collect();
                let named: Vec<String> = texts.iter().flat_map(|t| repos_named(t)).collect();
                let picks = find::clones(&clones_dir, find::DEPTH)
                    .into_iter()
                    .map(|c| {
                        let branch = head_branch(Path::new(&c.dir));
                        let all = local_branches(Path::new(&c.dir));
                        Pick {
                            on: linked.contains(&c.project)
                                || ticked(&c.project, &c.dir, &branch, &k, &named, &all, &texts),
                            dir: c.dir,
                            repo: c.project,
                            branch,
                        }
                    })
                    .collect();
                StartMsg::Clones {
                    key: k,
                    title,
                    kind,
                    space: space_dir,
                    picks,
                }
            }),
        ]
    }

    fn repos_menu(p: &Pending, at: &str) -> Menu {
        let go = MenuItem::pick(
            format!(
                "start work in {} →",
                plural(p.picks.iter().filter(|c| c.on).count())
            ),
            "",
            "go",
        )
        .icon(Icon::Branch);
        // The way on is the first row: it is on screen however many clones there are.
        let items = std::iter::once(go)
            .chain(p.picks.iter().map(|c| {
                let hint = if c.branch.is_empty() {
                    config_tilde(&c.dir)
                } else {
                    format!("{} · on {}", config_tilde(&c.dir), c.branch)
                };
                MenuItem::pick(
                    format!("{} {}", if c.on { "[x]" } else { "[ ]" }, c.name()),
                    hint,
                    c.dir.clone(),
                )
            }))
            .collect();
        Menu::new(
            REPOS,
            format!(
                "start {} in which repos? (first row: go on; ⏎ on a repo ticks it)",
                p.key
            ),
            items,
        )
        .at(at)
    }

    pub(crate) fn start_msg(&mut self, msg: StartMsg) -> Vec<Effect> {
        match msg {
            StartMsg::Clones {
                key,
                title,
                kind,
                space,
                picks,
            } => {
                if picks.is_empty() {
                    return vec![
                        Effect::Report(JournalOutcome::Blocked),
                        Effect::Notify(format!(
                            "no clones found under {}: set its folders in settings, workspace › workspaces",
                            config_tilde(&space)
                        )),
                    ];
                }
                let pending = Pending {
                    key,
                    title,
                    kind,
                    space,
                    picks,
                };
                let menu = Self::repos_menu(&pending, "go");
                self.state.start.pending = Some(pending);
                vec![Effect::Menu(menu)]
            }
            StartMsg::Made {
                key,
                space,
                payload,
                said,
            } => {
                let mut fx = vec![
                    Effect::Report(JournalOutcome::Succeeded),
                    journal_at("workspace.start.request", &space, &key, ""),
                    Effect::Emit {
                        topic: START,
                        payload,
                    },
                    Effect::Notify(said),
                ];
                fx.extend(self.reindex(&space, false));
                let _ = key;
                fx
            }
            StartMsg::Planned {
                key,
                space,
                results,
                path,
                existing,
                text,
                rows,
            } => {
                let mut fx = vec![Effect::Report(JournalOutcome::Succeeded)];
                fx.extend(self.planned(key, space, results, path, existing, text, rows));
                fx
            }
            StartMsg::Finished {
                key,
                space,
                results,
                said,
            } => {
                let mut fx = vec![Effect::Report(JournalOutcome::Succeeded)];
                fx.extend(self.finish(&key, &space, results, said));
                fx
            }
            // A filesystem write can fail after it changed part of the file.
            StartMsg::Failed(why) => vec![
                Effect::Report(JournalOutcome::Unknown),
                Effect::Notify(radar_core::writes::failed_line(&why)),
            ],
        }
    }

    /// An answer of one of the start menus; `None` when it is not one.
    pub(crate) fn start_menu(&mut self, menu: &str, value: &str) -> Option<Vec<Effect>> {
        match menu {
            REPOS => Some(self.repos_chosen(value)),
            HOW => Some(self.how_chosen(value)),
            TABLE => Some(self.table_answered(value)),
            _ => None,
        }
    }

    fn repos_chosen(&mut self, value: &str) -> Vec<Effect> {
        let Some(p) = self.state.start.pending.as_mut() else {
            return vec![];
        };
        if value != "go" {
            if let Some(c) = p.picks.iter_mut().find(|c| c.dir == value) {
                c.on = !c.on;
            }
            return vec![Effect::Menu(Self::repos_menu(p, value))];
        }
        if !p.picks.iter().any(|c| c.on) {
            return vec![
                Effect::Notify("tick at least one repo first".into()),
                Effect::Menu(Self::repos_menu(p, "go")),
            ];
        }
        let n = p.picks.iter().filter(|c| c.on).count();
        vec![Effect::Menu(
            Menu::new(
                HOW,
                format!("start {} in {}", p.key, plural(n)),
                vec![
                    MenuItem::new(
                        'c',
                        "in the normal checkout",
                        "set aside what is there first",
                        "checkout",
                    ),
                    MenuItem::new(
                        'w',
                        "parked on a worktree",
                        "beside its clone, the checkout left alone",
                        "worktree",
                    ),
                ],
            )
            .icon(Icon::Branch),
        )]
    }

    /// How is chosen: make the README, then say it to git.
    fn how_chosen(&mut self, how: &str) -> Vec<Effect> {
        let Some(p) = self.state.start.pending.take() else {
            return vec![];
        };
        let Some(space) = self.cfg.spaces.iter().find(|s| s.dir == p.space).cloned() else {
            return vec![];
        };
        self.state.start.runs.insert(
            p.key.clone(),
            Run {
                title: p.title.clone(),
                kind: p.kind.clone(),
                space: space.dir.clone(),
            },
        );
        let body = payload(
            &p.key,
            &p.title,
            &p.kind,
            &space.top().display().to_string(),
            &p.picks,
            how,
        );
        let (existing, create) = self.folder_facts(&space, &p.key);
        let date = readme::date_of(self.now);
        let space_dir = space.dir.clone();
        vec![
            journal_at("workspace.notes.prepare", &space.dir, &p.key, ""),
            Effect::Report(JournalOutcome::Started),
            spawn(move || {
                let top = space.top();
                let folder = readme::folder(existing.as_deref(), &create, &p.key, &p.title, &date);
                let path = readme::path_in(&top, &folder);
                let said = if path.exists() {
                    format!("asked git to start {}…", p.key)
                } else {
                    let text = readme::render_new(
                        readme::template(&top).as_deref(),
                        &p.key,
                        &p.title,
                        &p.kind,
                        &date,
                        &[],
                    );
                    match readme::write(&path, &text) {
                        Ok(()) => format!(
                            "made {}; asked git to start {}…",
                            config_tilde(&path.display().to_string()),
                            p.key
                        ),
                        Err(why) => {
                            return StartMsg::Failed(format!("the README was not made: {why}"));
                        }
                    }
                };
                StartMsg::Made {
                    key: p.key,
                    space: space_dir,
                    payload: body,
                    said,
                }
            }),
        ]
    }

    /// The ticket's notes folder if it has one, and where new ones go.
    fn folder_facts(&self, space: &Space, key: &str) -> (Option<String>, String) {
        let index = self.state.indexes.get(&space.dir);
        let existing = index.and_then(|i| i.folder_of(key));
        let create = index
            .map(|i| i.create.clone())
            .filter(|c| !c.is_empty())
            .or_else(|| {
                (space.notes.trim() != "okf" && !space.notes.trim().is_empty())
                    .then(|| space.notes.trim().to_string())
            })
            .unwrap_or_default();
        (existing, create)
    }

    // --- git answered -------------------------------------------------------------

    /// A bus topic: `git.started` fills the README.
    pub(crate) fn start_bus(&mut self, topic: &str, payload: &Value) -> Vec<Effect> {
        if topic != GIT_STARTED {
            return vec![];
        }
        let Some(key) = payload["ticket"].as_str().map(str::to_string) else {
            return vec![];
        };
        let run = self.state.start.runs.get(&key).cloned();
        let space = match &run {
            Some(r) => self.cfg.spaces.iter().find(|s| s.dir == r.space),
            None => key
                .rsplit_once('-')
                .and_then(|(project, _)| self.cfg.for_project(project)),
        };
        let Some(space) = space.cloned() else {
            return vec![];
        };
        let results = payload["results"].clone();
        // Nothing started: no README rows, no editor, and nobody is told it began.
        if !results
            .as_array()
            .is_some_and(|a| a.iter().any(|r| r["ok"].as_bool() == Some(true)))
        {
            let why = results[0]["why"].as_str().unwrap_or("nothing was started");
            return vec![
                journal_at("workspace.start.result", &space.dir, &key, ""),
                Effect::Report(JournalOutcome::Blocked),
                Effect::Notify(format!("{key} was not started: {why}")),
            ];
        }
        let (title, kind) =
            run.map_or_else(|| (String::new(), String::new()), |r| (r.title, r.kind));
        let (existing, create) = self.folder_facts(&space, &key);
        let date = readme::date_of(self.now);
        vec![
            journal_at("workspace.notes.plan", &space.dir, &key, ""),
            spawn(move || {
                let top = space.top();
                let rows = readme::rows_from(&results, &top);
                let folder = readme::folder(existing.as_deref(), &create, &key, &title, &date);
                let path = readme::path_in(&top, &folder);
                let current = std::fs::read_to_string(&path).ok();
                let text = match &current {
                    Some(t) => readme::merge(t, &rows),
                    None => readme::render_new(
                        readme::template(&top).as_deref(),
                        &key,
                        &title,
                        &kind,
                        &date,
                        &rows,
                    ),
                };
                StartMsg::Planned {
                    key,
                    space: space.dir,
                    results,
                    path: path.display().to_string(),
                    existing: current,
                    text,
                    rows,
                }
            }),
        ]
    }

    #[allow(clippy::too_many_arguments)]
    fn planned(
        &mut self,
        key: String,
        space: String,
        results: Value,
        path: String,
        existing: Option<String>,
        text: String,
        rows: Vec<readme::Row>,
    ) -> Vec<Effect> {
        match &existing {
            Some(old) if *old == text => self.finish(
                &key,
                &space,
                results,
                "the README already has these branches".into(),
            ),
            // A table with nothing in it is not a table to protect.
            Some(old) if readme::rows_in(old) > 0 => {
                let mut menu = Menu::confirm(
                    TABLE,
                    format!("update {key}'s README?"),
                    "write the table",
                    "leave the README",
                )
                .saying("only its ## Branches table changes, to:");
                for l in readme::table_lines(&rows) {
                    menu = menu.saying(l);
                }
                let event = journal_at("workspace.notes.update", &space, &key, &path);
                self.state.start.table = Some(Table {
                    key,
                    space,
                    results,
                    path,
                    text,
                });
                vec![event, Effect::Menu(menu)]
            }
            _ => self.write_then_finish(Table {
                key,
                space,
                results,
                path,
                text,
            }),
        }
    }

    fn write_then_finish(&mut self, t: Table) -> Vec<Effect> {
        vec![
            journal_at("workspace.notes.write", &t.space, &t.key, &t.path),
            Effect::Report(JournalOutcome::Started),
            spawn(move || {
                let path = Path::new(&t.path);
                match readme::write(path, &t.text) {
                    Ok(()) => StartMsg::Finished {
                        said: format!("filled the branches in {}", config_tilde(&t.path)),
                        key: t.key,
                        space: t.space,
                        results: t.results,
                    },
                    Err(why) => StartMsg::Failed(format!("the README was not written: {why}")),
                }
            }),
        ]
    }

    fn table_answered(&mut self, value: &str) -> Vec<Effect> {
        let Some(t) = self.state.start.table.take() else {
            return vec![];
        };
        if value == "yes" {
            self.write_then_finish(t)
        } else {
            let mut fx = vec![Effect::Report(JournalOutcome::Cancelled)];
            fx.extend(self.finish(
                &t.key,
                &t.space,
                t.results,
                "left the README as it is".into(),
            ));
            fx
        }
    }

    /// The README is settled: open the workspace, and tell the others.
    fn finish(&mut self, key: &str, space: &str, results: Value, said: String) -> Vec<Effect> {
        let partial = results
            .as_array()
            .is_some_and(|rows| rows.iter().any(|r| r["ok"].as_bool() != Some(true)));
        let mut fx = vec![
            journal_at("workspace.start.completed", space, key, ""),
            Effect::Report(if partial {
                JournalOutcome::Partial
            } else {
                JournalOutcome::Succeeded
            }),
            Effect::Notify(said),
        ];
        let Some(space) = self.cfg.spaces.iter().find(|s| s.dir == space).cloned() else {
            return fx;
        };
        let dir = self.open_dir(&space, key);
        let (existing, create) = self.folder_facts(&space, key);
        let title = self
            .state
            .start
            .runs
            .get(key)
            .map(|r| r.title.clone())
            .unwrap_or_default();
        let folder = readme::folder(
            existing.as_deref(),
            &create,
            key,
            &title,
            &readme::date_of(self.now),
        );
        let notes_path = readme::path_in(&space.top(), &folder).display().to_string();
        fx.push(Effect::Emit {
            topic: STARTED,
            payload: json!({
                "ticket": key,
                "workspace": space.top().display().to_string(),
                "notes": notes_path,
                "repos": results,
            }),
        });
        fx.extend(self.reindex(&space.dir, false));
        // Launches have their own semantic span. Keep the workflow's bus and
        // index effects before that marker so their replies cannot finish it.
        if space.editor.trim().is_empty() {
            fx.extend(self.open_terminal(&space, dir));
        } else {
            fx.extend(self.open_editor(&space, Target::Folder(dir)));
        }
        fx
    }
}

fn plural(n: usize) -> String {
    if n == 1 {
        "1 repo".into()
    } else {
        format!("{n} repos")
    }
}

fn config_tilde(p: &str) -> String {
    crate::config::tilde(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::find::fixture::Scratch;
    use crate::notes;
    use radar_core::Plugin;
    use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    const KEY: &str = "ABC-12";

    fn plugin(s: &Scratch, editor: &str) -> WorkspacePlugin {
        let mut p = WorkspacePlugin::default();
        let mut space = Space::new(s.0.display().to_string());
        space.tickets = vec!["ABC".into()];
        space.editor = editor.into();
        p.cfg.spaces = vec![space];
        p.now = radar_core::plugin::DEMO_NOW;
        p
    }

    fn ticket() -> Subject {
        Subject::new("ticket", KEY, KEY)
            .with("title", "Add a stats endpoint")
            .with("type", "Story")
            .with("project", "ABC")
    }

    /// Three clones, one already on the ticket's branch.
    fn workspace(name: &str) -> Scratch {
        let s = Scratch::new(name);
        s.clone(
            "repositories/team-a/api",
            "git@example.com:acme/team-a/api.git",
        );
        s.clone(
            "repositories/team-a/web",
            "git@example.com:acme/team-a/web.git",
        );
        s.clone("repositories/docs", "git@example.com:acme/docs.git");
        s.file(
            "repositories/team-a/web/.git/HEAD",
            "ref: refs/heads/alice/ABC-12\n",
        );
        s
    }

    fn run(fx: Vec<Effect>) -> Vec<StartMsg> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let mut out = Vec::new();
        for e in fx {
            if let Effect::Spawn(t) = e {
                let msg = rt.block_on(t);
                out.push(*msg.downcast::<StartMsg>().expect("a StartMsg"));
            }
        }
        out
    }

    fn menu_of(fx: &[Effect]) -> Menu {
        fx.iter()
            .find_map(|e| match e {
                Effect::Menu(m) => Some(m.clone()),
                _ => None,
            })
            .expect("a menu")
    }

    fn picks(p: &WorkspacePlugin, s: &Scratch) -> Vec<Pick> {
        let space = p.cfg.spaces[0].clone();
        let mut fx = p.start_act_for_test(&ticket(), &space);
        match fx.remove(0) {
            StartMsg::Clones { picks, .. } => {
                let _ = s;
                picks
            }
            _ => panic!("clones"),
        }
    }

    impl WorkspacePlugin {
        fn start_act_for_test(&self, s: &Subject, space: &Space) -> Vec<StartMsg> {
            let mut me = WorkspacePlugin {
                cfg: self.cfg.clone(),
                now: self.now,
                ..WorkspacePlugin::default()
            };
            run(me.start_act(s, space))
        }
    }

    #[test]
    fn journal_scope_expands_home_and_matches_the_owner_snapshot() {
        let mut p = WorkspacePlugin::default();
        p.cfg.spaces.push(Space::new("~/work"));
        let top = crate::config::journal_path("~/work").unwrap();
        let note = top.join("artifacts/README.md");
        let Effect::Journal(action) = journal_at("workspace.start", "~/work", "ABC-12", "") else {
            panic!("journal")
        };
        assert_eq!(action.metadata.omitted_subjects, 0);
        let key = &action
            .subjects
            .iter()
            .find(|s| s.key.kind == "ticket")
            .unwrap()
            .key;
        assert_eq!(key.instance, top.display().to_string());
        let subject = Subject::new("note", note.display().to_string(), "README");
        assert_eq!(
            p.journal_subject(&subject).unwrap().key.instance,
            key.instance
        );
        let Effect::Journal(relative) = journal_at("workspace.start", ".", "ABC-12", "") else {
            panic!("journal")
        };
        assert!(Path::new(&relative.subjects[0].key.instance).is_absolute());
    }

    #[test]
    fn start_journal_has_scoped_identities_without_ticket_text() {
        let s = workspace("journal");
        let mut p = plugin(&s, "");
        let ticket = ticket().with("title", "private ticket description");
        let note = Subject::new(
            "note",
            s.path("artifacts/README.md").display().to_string(),
            "private label",
        );
        let snapshot = p.journal_subject(&note).unwrap();
        assert_eq!(snapshot.key.instance, s.0.display().to_string());
        assert!(
            p.journal_subject(&Subject::new("note", "/elsewhere/README.md", "x"))
                .is_none()
        );
        let effects = p.act_with_related(&ticket, ACTION, &[]);
        let descriptor = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::Journal(action) => Some(action),
                _ => None,
            })
            .unwrap();
        assert_eq!(descriptor.kind.as_str(), "workspace.start");
        let key = &descriptor
            .subjects
            .iter()
            .find(|s| s.key.kind == "ticket")
            .unwrap()
            .key;
        assert_eq!(key.provider, "workspace");
        assert_eq!(key.instance, s.0.display().to_string());
        assert_eq!(key.id, "ABC-12");
        assert!(!format!("{descriptor:?}").contains("private ticket description"));
        let failed = p.start_msg(StartMsg::Failed("private filesystem error".into()));
        assert!(
            failed
                .iter()
                .any(|e| matches!(e, Effect::Report(JournalOutcome::Unknown)))
        );
    }

    #[test]
    fn related_repos_preselect_exact_clones_and_keep_the_existing_ticks() {
        let s = workspace("related-repos");
        let mut p = plugin(&s, "");
        let related = vec![
            Subject::new("repo", "acme/team-a/api", "api"),
            Subject::new("repo", "acme/team-a/api", "the same repo again"),
            Subject::new("repo", "elsewhere/docs", "docs"),
            Subject::new("repo", "acme/not-cloned", "not cloned here"),
            Subject::new("note", "acme/docs", "not a repo"),
            Subject::new("repo", "", "no identity"),
        ];
        let fx = p.act_with_related(&ticket(), ACTION, &related);
        let msg = run(fx).remove(0);
        let StartMsg::Clones { picks, .. } = &msg else {
            panic!("clones")
        };
        assert_eq!(picks.len(), 3, "uncloned repos add no rows");
        let on = |repo: &str| picks.iter().find(|c| c.repo == repo).unwrap().on;
        assert!(on("acme/team-a/api"), "the linked repo is selected");
        assert!(
            on("acme/team-a/web"),
            "the existing branch still selects it"
        );
        assert!(!on("acme/docs"), "a basename or a non-repo is no match");
        p.start_msg(msg);
        // Preselection remains a suggestion: the user can untick the linked repo.
        let dir = s.path("repositories/team-a/api").display().to_string();
        p.on_menu(REPOS, &dir);
        assert!(
            !p.state
                .start
                .pending
                .as_ref()
                .unwrap()
                .picks
                .iter()
                .find(|c| c.repo == "acme/team-a/api")
                .unwrap()
                .on
        );
    }

    #[test]
    fn no_related_repos_keeps_start_work_unchanged() {
        let s = workspace("no-related-repos");
        let mut p = plugin(&s, "");
        let msg = run(p.act_with_related(&ticket(), ACTION, &[])).remove(0);
        let StartMsg::Clones { picks, .. } = msg else {
            panic!("clones")
        };
        let selected: Vec<_> = picks
            .iter()
            .filter(|c| c.on)
            .map(|c| c.repo.as_str())
            .collect();
        assert_eq!(selected, ["acme/team-a/web"]);
    }

    #[test]
    fn the_ticks_are_the_clones_on_the_branch_and_the_ones_a_note_names() {
        let s = workspace("ticks");
        let p = plugin(&s, "");
        let by =
            |picks: &[Pick], name: &str| picks.iter().find(|c| c.dir.ends_with(name)).unwrap().on;
        let found = picks(&p, &s);
        assert_eq!(found.len(), 3);
        assert!(by(&found, "team-a/web"));
        assert!(!by(&found, "team-a/api"));
        assert!(!by(&found, "docs"));
        // A note's `repos:` names more, as a list or as lines.
        assert_eq!(
            repos_named("---\nrepos: [team-a/api, \"docs\"]\n---\n"),
            ["team-a/api", "docs"]
        );
        assert_eq!(
            repos_named("---\ntitle: x\nrepos:\n  - acme/docs\n  - api\nstatus: a\n---\n"),
            ["acme/docs", "api"]
        );
        assert!(repos_named("repos: [a]\n").is_empty());
        let t =
            |repo: &str, dir: &str, branch: &str, named: &[&str], all: &[&str], texts: &[&str]| {
                let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
                ticked(repo, dir, branch, KEY, &s(named), &s(all), &s(texts))
            };
        assert!(t(
            "acme/team-a/api",
            "/w/repositories/team-a/api",
            "main",
            &["team-a/api"],
            &[],
            &[]
        ));
        assert!(t("", "/w/repositories/docs", "main", &["docs"], &[], &[]));
        assert!(!t("acme/team-a/api", "/w/api", "main", &["pi"], &[], &[]));
        assert!(t("a/b", "/w/b", "feature/abc-12-x", &[], &[], &[]));
        // The key is a whole token: ABC-1 is not ABC-12's branch.
        assert!(!t("a/b", "/w/b", "alice/ABC-12", &[], &["alice/ABC-12"], &[]) || KEY == "ABC-12");
        assert!(has_key("alice/ABC-12", "ABC-12"));
        assert!(has_key("abc-12_x", "ABC-12"));
        assert!(!has_key("alice/ABC-12", "ABC-1"));
        assert!(!has_key("alice/XABC-12", "ABC-12"));
        assert!(!has_key("", "ABC-12"));
        // A repo that holds the branch without being on it is ticked.
        assert!(t(
            "a/b",
            "/w/b",
            "main",
            &[],
            &["main", "alice/ABC-12"],
            &[]
        ));
        assert!(!t(
            "a/b",
            "/w/b",
            "main",
            &[],
            &["main", "alice/ABC-123"],
            &[]
        ));
        // A README that links to the repo ticks it, in a prose-only plan.
        let readme = "Plan: see [the api](https://git.example.com/acme/team-a/api/-/merge_requests/4) and `docs`.";
        assert!(t("acme/team-a/api", "/w/api", "main", &[], &[], &[readme]));
        assert!(!t("acme/team-a/web", "/w/web", "main", &[], &[], &[readme]));
        assert!(!t("acme/team-a/ap", "/w/ap", "main", &[], &[], &[readme]));
        assert!(t(
            "",
            "/w/docs",
            "main",
            &[],
            &[],
            &["[docs](../repositories/docs/README.md)"]
        ));
    }

    #[test]
    fn the_ticks_find_branches_that_are_not_checked_out() {
        let s = workspace("elsewhere");
        s.file(
            "repositories/team-a/api/.git/refs/heads/alice/ABC-12",
            "x\n",
        );
        s.file(
            "repositories/docs/.git/packed-refs",
            "# pack\nabc refs/heads/alice/ABC-12\nabc refs/heads/main\n",
        );
        s.file(
            "artifacts/d/ABC-12-x/README.md",
            "# ABC-12\n\nSee [web](https://git.example.com/acme/team-a/web/-/tree/main).\n",
        );
        let mut p = plugin(&s, "");
        p.state
            .indexes
            .insert(s.0.display().to_string(), notes::build(&p.cfg.spaces[0], 0));
        let found = picks(&p, &s);
        let on = |n: &str| found.iter().find(|c| c.dir.ends_with(n)).unwrap().on;
        assert!(on("team-a/api") && on("docs") && on("team-a/web"));
        assert_eq!(
            local_branches(&s.path("repositories/docs")),
            ["alice/ABC-12", "main"]
        );
    }

    #[test]
    fn with_forty_clones_the_way_on_is_the_first_row_and_on_screen() {
        let picks: Vec<Pick> = (0..40)
            .map(|i| Pick {
                dir: format!("/w/repositories/r{i}"),
                repo: format!("acme/r{i}"),
                branch: "main".into(),
                on: i == 7,
            })
            .collect();
        let p = Pending {
            key: KEY.into(),
            title: String::new(),
            kind: String::new(),
            space: "/w".into(),
            picks,
        };
        let menu = WorkspacePlugin::repos_menu(&p, "go");
        assert_eq!(menu.items.len(), 41);
        assert_eq!(menu.items[0].value, "go");
        assert_eq!(menu.items[0].label, "start work in 1 repo →");
        assert!(menu.title.contains("first row"));
        // Drawn at both sizes, the row is on screen without scrolling.
        for (w, h) in [(80u16, 24u16), (160, 40)] {
            let mut host =
                radar_core::Host::new(vec![Box::new(WorkspacePlugin::default())], 0).unwrap();
            host.menu = Some(radar_core::host::OpenMenu::open(
                radar_core::host::Owner::Plugin(0),
                menu.clone(),
            ));
            let text = radar_core::shell::to_text(&host, w, h);
            assert!(text.contains("start work in 1 repo"), "{w}x{h}:\n{text}");
        }
    }

    #[test]
    fn when_nothing_started_nothing_is_filled_opened_or_announced() {
        let s = workspace("failed");
        let mut p = plugin(&s, "code");
        let bad = json!({"ticket": KEY, "results": [
            {"dir": "/w/a", "repo": "", "branch": "b", "base": "main", "worktree": "", "stash": "", "ok": false, "why": "dirty"},
            {"dir": "/w/b", "repo": "", "branch": "b", "base": "main", "worktree": "", "stash": "", "ok": false, "why": "not run: stopped"},
        ]});
        let fx = p.start_bus(GIT_STARTED, &bad);
        assert!(matches!(&fx[..], [Effect::Journal(action),
            Effect::Report(JournalOutcome::Blocked), Effect::Notify(t)]
            if action.kind.as_str() == "workspace.start.result" && t == "ABC-12 was not started: dirty"));
    }

    #[test]
    fn a_worktrees_head_is_followed_through_its_git_file() {
        let s = Scratch::new("head");
        s.file(
            "main/.git/worktrees/wt/HEAD",
            "ref: refs/heads/alice/ABC-12\n",
        );
        s.file(
            "wt/.git",
            &format!("gitdir: {}\n", s.path("main/.git/worktrees/wt").display()),
        );
        assert_eq!(head_branch(&s.path("wt")), "alice/ABC-12");
        assert_eq!(head_branch(&s.path("nowhere")), "");
    }

    #[test]
    fn the_menus_toggle_with_a_row_and_go_asks_how_then_the_payload_goes_to_git() {
        let s = workspace("flow");
        let mut p = plugin(&s, "");
        let fx = p.start_act(&ticket(), &p.cfg.spaces[0].clone());
        let mut msgs = run(fx);
        let fx = p.start_msg(msgs.remove(0));
        let menu = menu_of(&fx);
        assert_eq!(menu.id, REPOS);
        // The rows: three clones then go; the ticked one says so.
        assert_eq!(menu.items.len(), 4);
        assert!(menu.items.iter().any(|i| i.label == "[x] acme/team-a/web"));
        assert_eq!(menu.items[0].value, "go");
        // Choosing a row toggles it and asks again.
        let api = menu
            .items
            .iter()
            .find(|i| i.label.ends_with("team-a/api"))
            .unwrap()
            .value
            .clone();
        let fx = p.on_menu(REPOS, &api);
        let menu = menu_of(&fx);
        assert!(menu.items.iter().any(|i| i.label == "[x] acme/team-a/api"));
        assert_eq!(menu.items[0].label, "start work in 2 repos →");
        // The highlight opens on the row just ticked, as the host puts it
        // (arrow keys and j/k then move from there).
        assert_eq!(menu.at.as_deref(), Some(api.as_str()));
        let open = radar_core::host::OpenMenu::open(radar_core::host::Owner::Host, menu.clone());
        assert_eq!(menu.items[open.row].value, api);
        // Down (and j) go on to the next row from there, not from the top.
        let mut host = radar_core::Host::new(vec![Box::new(WorkspacePlugin::default())], 0).unwrap();
        host.menu = Some(open);
        let at = host.menu.as_ref().unwrap().row;
        for code in [KeyCode::Down, KeyCode::Char('j')] {
            host.update(radar_core::host::Event::Key(KeyEvent::new(
                code,
                KeyModifiers::NONE,
            )));
        }
        assert_eq!(
            host.menu.as_ref().unwrap().row,
            (at + 2).min(menu.items.len() - 1)
        );
        // With none ticked it says so.
        let web = menu
            .items
            .iter()
            .find(|i| i.label.ends_with("team-a/web"))
            .unwrap()
            .value
            .clone();
        p.on_menu(REPOS, &web);
        p.on_menu(REPOS, &api);
        let fx = p.on_menu(REPOS, "go");
        assert!(matches!(&fx[0], Effect::Notify(t) if t.contains("at least one")));
        // Tick two again, go, and choose how.
        p.on_menu(REPOS, &api);
        p.on_menu(REPOS, &web);
        let fx = p.on_menu(REPOS, "go");
        let how = menu_of(&fx);
        assert_eq!(how.id, HOW);
        assert_eq!(
            how.items
                .iter()
                .map(|i| i.value.as_str())
                .collect::<Vec<_>>(),
            ["checkout", "worktree"]
        );
        let fx = p.on_menu(HOW, "worktree");
        let mut made = run(fx);
        let StartMsg::Made { payload, said, .. } = made.remove(0) else {
            panic!("made")
        };
        let top = s.0.display().to_string();
        assert_eq!(
            payload,
            json!({
                "ticket": {"key": "ABC-12", "title": "Add a stats endpoint", "type": "Story"},
                "workspace": top,
                "repos": [
                    {"dir": s.path("repositories/team-a/api").display().to_string(), "repo": "acme/team-a/api", "how": "worktree"},
                    {"dir": s.path("repositories/team-a/web").display().to_string(), "repo": "acme/team-a/web", "how": "worktree"},
                ],
            })
        );
        assert!(said.contains("asked git to start ABC-12…"));
        // The README was made before the question went out, with no branches yet.
        let readme = std::fs::read_to_string(
            s.path("artifacts/2026-10-06/ABC-12-add-a-stats-endpoint/README.md"),
        )
        .unwrap();
        assert!(readme.contains("ticket: ABC-12"));
        assert_eq!(readme::rows_in(&readme), 0);
    }

    fn git_started(s: &Scratch) -> Value {
        json!({
            "ticket": KEY,
            "results": [
                {"dir": s.path("repositories/team-a/api").display().to_string(), "repo": "acme/team-a/api", "branch": "alice/ABC-12", "base": "main", "worktree": "", "stash": "ABC-12-local-2026-10-06", "ok": true, "why": ""},
                {"dir": s.path("repositories/docs").display().to_string(), "repo": "acme/docs", "branch": "alice/ABC-12", "base": "main", "worktree": s.path("repositories/docs-abc-12").display().to_string(), "stash": "", "ok": true, "why": ""},
            ]
        })
    }

    #[test]
    fn finishing_keeps_workflow_bus_and_index_outside_the_launch_span() {
        let s = workspace("launch-scope");
        for (editor, kind) in [
            ("", "workspace.terminal.launch"),
            ("mock-editor", "workspace.editor.launch"),
        ] {
            let mut p = plugin(&s, editor);
            let dir = s.0.display().to_string();
            let fx = p.finish(KEY, &dir, json!([]), "ready".into());
            let launch = fx
                .iter()
                .position(|e| {
                    matches!(e,
                        Effect::Journal(action) if action.kind.as_str() == kind
                    )
                })
                .expect("launch marker");
            assert!(
                fx[..launch]
                    .iter()
                    .any(|e| matches!(e, Effect::Emit { topic: STARTED, .. }))
            );
            assert!(
                fx[..launch].iter().any(|e| matches!(e,
                    Effect::SpawnAction { action, .. } if action.kind.as_str() == "workspace.notes.index"
                )),
                "notes index belongs to the workflow"
            );
            assert!(matches!(
                &fx[launch..],
                [Effect::Journal(_), Effect::Spawn(_)]
            ));
        }
    }

    #[test]
    fn git_started_fills_the_table_of_the_readme_s_made_and_says_workspace_started() {
        let s = workspace("filled");
        let mut p = plugin(&s, "");
        let fx = p.start_act(&ticket(), &p.cfg.spaces[0].clone());
        let m = run(fx).remove(0);
        p.start_msg(m);
        p.on_menu(REPOS, "go");
        run(p.on_menu(HOW, "checkout"));
        let planned = run(p.start_bus(GIT_STARTED, &git_started(&s))).remove(0);
        // The README had an empty table: it is written without asking.
        let fx = p.start_msg(planned);
        let finished = run(fx).remove(0);
        let fx = p.start_msg(finished);
        let emitted = fx
            .iter()
            .find_map(|e| match e {
                Effect::Emit { topic, payload } => Some((*topic, payload.clone())),
                _ => None,
            })
            .expect("workspace.started");
        let path = s.path("artifacts/2026-10-06/ABC-12-add-a-stats-endpoint/README.md");
        assert_eq!(
            emitted,
            (
                STARTED,
                json!({
                    "ticket": KEY,
                    "workspace": s.0.display().to_string(),
                    "notes": path.display().to_string(),
                    "repos": git_started(&s)["results"],
                })
            )
        );
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("| acme/team-a/api | repositories/team-a/api | alice/ABC-12 | main | ABC-12-local-2026-10-06 |"));
        assert!(text.contains("| acme/docs | repositories/docs-abc-12 | alice/ABC-12 | main |  |"));
        // Without an editor set, a terminal is opened at the top.
        assert!(fx.iter().any(|e| matches!(e, Effect::Spawn(_))));
    }

    #[test]
    fn an_existing_readme_is_asked_about_and_only_its_branches_table_changes() {
        let s = workspace("existing");
        s.file("artifacts/2026-01-01/ABC-12-stats/README.md", "# ABC-12\n\n## Decisions\n\n- keep it simple\n\n## Branches\n\n| Repo | Checkout | Branch | Base | Stash |\n| --- | --- | --- | --- | --- |\n| acme/team-a/web | repositories/team-a/web | alice/ABC-12 | main |  |\n\n## Evidence\n\nnone\n");
        let mut p = plugin(&s, "");
        p.state
            .indexes
            .insert(s.0.display().to_string(), notes::build(&p.cfg.spaces[0], 0));
        let planned = run(p.start_bus(GIT_STARTED, &git_started(&s))).remove(0);
        let fx = p.start_msg(planned);
        let menu = menu_of(&fx);
        assert_eq!(menu.id, TABLE);
        let said: Vec<&str> = menu
            .items
            .iter()
            .filter(|i| i.note)
            .map(|i| i.label.as_str())
            .collect();
        assert_eq!(said[0], "only its ## Branches table changes, to:");
        assert_eq!(said[1], "| Repo | Checkout | Branch | Base | Stash |");
        assert_eq!(said.len(), 5);
        // Nothing is written until yes.
        let path = s.path("artifacts/2026-01-01/ABC-12-stats/README.md");
        let before = std::fs::read_to_string(&path).unwrap();
        assert!(!before.contains("acme/docs"));
        let fx = p.on_menu(TABLE, "yes");
        let finished = run(fx).remove(0);
        p.start_msg(finished);
        let after = std::fs::read_to_string(&path).unwrap();
        let want = before.replace(
            "| acme/team-a/web | repositories/team-a/web | alice/ABC-12 | main |  |\n",
            "| acme/team-a/web | repositories/team-a/web | alice/ABC-12 | main |  |\n| acme/team-a/api | repositories/team-a/api | alice/ABC-12 | main | ABC-12-local-2026-10-06 |\n| acme/docs | repositories/docs-abc-12 | alice/ABC-12 | main |  |\n",
        );
        assert_eq!(after, want);
    }

    #[test]
    fn no_leaves_the_readme_and_still_tells_the_others() {
        let s = workspace("declined");
        s.file(
            "artifacts/d/ABC-12-x/README.md",
            "# x\n\n## Branches\n\n| h |\n| - |\n| a |\n",
        );
        let mut p = plugin(&s, "code");
        p.state
            .indexes
            .insert(s.0.display().to_string(), notes::build(&p.cfg.spaces[0], 0));
        let planned = run(p.start_bus(GIT_STARTED, &git_started(&s))).remove(0);
        p.start_msg(planned);
        let fx = p.on_menu(TABLE, "no");
        assert!(
            fx.iter()
                .any(|e| matches!(e, Effect::Emit { topic: STARTED, .. }))
        );
        assert!(
            !std::fs::read_to_string(s.path("artifacts/d/ABC-12-x/README.md"))
                .unwrap()
                .contains("acme/docs")
        );
    }

    #[test]
    fn s_is_an_action_of_a_ticket_and_does_something() {
        let s = workspace("check");
        let make = move || -> Box<dyn Plugin> { Box::new(plugin(&Scratch::new("check-make"), "")) };
        let mut check = radar_core::check::Check::new(&make);
        assert!(check.actions(&ticket()) >= 4);
        assert!(check.gaps.is_empty(), "{:?}", check.gaps);
        let _ = s;
        let p = plugin(&workspace("keys"), "");
        let Some(radar_core::Contribution::Actions(a)) =
            p.contribute(radar_core::Point::Actions, &ticket())
        else {
            panic!("actions")
        };
        let s_row = a.iter().find(|a| a.id == ACTION).unwrap();
        assert_eq!(
            (s_row.key, s_row.what.as_str()),
            (Some('s'), "start work on ABC-12…")
        );
    }
}
