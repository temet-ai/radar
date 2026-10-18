//! Host-qualified folder contributions. This path never uses legacy project-name
//! matching, guesses a fork, or changes a checkout's branch.

use std::path::Path;

use radar_core::plugin::{Effect, Menu, MenuItem, Msg, Tone};
use radar_core::system::{self, Env, FolderLaunch, InFolder};
use radar_core::ui::icon::Icon;
use radar_core::{Action, Adds, Chip, Contribution, Point, Subject, Typing, Verdict};
use serde::{Deserialize, Serialize};

use crate::GitPlugin;
use crate::local::{self, Checkout, Link};

pub(crate) const ADDS: [Adds; 2] = [
    Adds::new("repository", "matched local folders, editors and terminals"),
    Adds::new(
        "pull-request",
        "source-repository folders, on their current branch",
    ),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Identity {
    pub key: String,
    host: String,
    path: String,
}

fn clean(input: &str) -> bool {
    !input.is_empty()
        && input.len() <= 2048
        && !input.chars().any(|c| c.is_whitespace() || c.is_control())
        && !input.contains(['\\', '?', '#', '%'])
}

fn identity(host: &str, path: &str) -> Option<Identity> {
    if host.is_empty()
        || host.len() > 253
        || host.split('.').any(|label| {
            label.is_empty()
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        })
    {
        return None;
    }
    let segments: Vec<_> = path.split('/').collect();
    if segments.len() < 2
        || segments.iter().any(|s| {
            s.is_empty()
                || matches!(*s, "." | "..")
                || !s
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
        })
    {
        return None;
    }
    let host = host.to_ascii_lowercase();
    let path = path.to_ascii_lowercase();
    Some(Identity {
        key: format!("https://{host}/{path}"),
        host,
        path,
    })
}

/// Canonical web identity. Ports, aliases and path repairs are deliberately not
/// inferred; a transport endpoint alone cannot establish a different web origin.
pub(crate) fn repository(input: &str) -> Option<Identity> {
    if !clean(input) {
        return None;
    }
    let rest = input.strip_prefix("https://")?;
    let (host, path) = rest.split_once('/')?;
    let out = identity(host, path)?;
    (out.key == input).then_some(out)
}

/// Read-only normalization of ordinary HTTPS, SSH and scp-shaped origin values.
/// HTTPS credentials and non-default SSH ports cannot establish this identity.
pub(crate) fn remote(input: &str) -> Option<Identity> {
    if !clean(input) {
        return None;
    }
    let (host, path) = if let Some(rest) = input.strip_prefix("https://") {
        let (host, path) = rest.split_once('/')?;
        (host.strip_suffix(":443").unwrap_or(host), path)
    } else if let Some(rest) = input.strip_prefix("ssh://") {
        let (authority, path) = rest.split_once('/')?;
        let host = authority.strip_prefix("git@").unwrap_or(authority);
        (host.strip_suffix(":22").unwrap_or(host), path)
    } else {
        let (authority, path) = input.split_once(':')?;
        (authority.strip_prefix("git@").unwrap_or(authority), path)
    };
    let path = path.strip_suffix('/').unwrap_or(path);
    let path = path.strip_suffix(".git").unwrap_or(path);
    identity(host, path)
}

fn safe_text(input: &str) -> bool {
    !input.chars().any(char::is_control)
        && !input.contains(['?', '#', '%'])
        && !radar_core::log::contains_secret(input)
}

/// Credential safety is separate from qualification for scoped matching. Safe
/// legacy ports, transport schemes, aliases and local origins remain unchanged.
fn origin_is_safe_to_keep(input: &str) -> bool {
    if input.is_empty() || !safe_text(input) {
        return false;
    }
    let safe_user = |user: &str| {
        !user.is_empty()
            && user
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
    };
    if let Some((scheme, rest)) = input.split_once("://") {
        if rest.contains('\\') {
            return false;
        }
        let authority = rest.split('/').next().unwrap_or_default();
        return match authority.split_once('@') {
            Some((user, host)) => {
                scheme.eq_ignore_ascii_case("ssh") && safe_user(user) && !host.contains('@')
            }
            None => true,
        };
    }
    let local_path = input.starts_with(['/', '~', '.'])
        || input.as_bytes().get(1) == Some(&b':') && input.as_bytes()[0].is_ascii_alphabetic();
    if !local_path && let Some((user, host_path)) = input.split_once('@') {
        return safe_user(user) && !host_path.contains('@');
    }
    true
}

/// Drop all origin-derived fields together when an origin could contain secrets.
/// Safe checkouts keep the legacy scanner's exact display and matching metadata.
fn safe_checkout(checkout: Checkout) -> Option<Checkout> {
    (origin_is_safe_to_keep(&checkout.remote) && safe_text(&checkout.project)).then_some(checkout)
}

fn safe_checkouts(checkouts: &mut Vec<Checkout>) -> usize {
    let before = checkouts.len();
    *checkouts = std::mem::take(checkouts)
        .into_iter()
        .filter_map(safe_checkout)
        .collect();
    before - checkouts.len()
}

/// Scoped actions can be the next cache write after an older scanner or restore.
/// Sanitize the shared checkout state before that write, without changing the
/// legacy scanner's behavior or rewriting unrelated configuration.
fn persist(p: &mut GitPlugin) -> Effect {
    if let Some(index) = &mut p.local.index {
        safe_checkouts(&mut index.checkouts);
    }
    safe_checkouts(&mut p.local.linked);
    for checkouts in p.local.looked.values_mut() {
        safe_checkouts(checkouts);
    }
    let safe_key =
        |key: &str| safe_text(key) && (!key.contains("://") || origin_is_safe_to_keep(key));
    p.local.use_worktree.retain(|key, _| safe_key(key));
    p.local.looked.retain(|key, _| safe_key(key));
    Effect::PersistCache
}

pub(crate) fn is_scoped(key: &str) -> bool {
    key.contains("://")
}

#[derive(Clone, Debug)]
struct Target {
    identity: Option<Identity>,
}

fn target(s: &Subject) -> Option<Target> {
    if !matches!(s.kind.as_str(), "repository" | "pull-request") {
        return None;
    }
    let base = repository(s.field("repository_url")?)?;
    if s.url.as_ref().is_some_and(|u| u != &s.id)
        || s.field("instance")
            .is_some_and(|v| v != format!("https://{}", base.host))
        || s.field("repository").is_some_and(|v| v != base.path)
    {
        return None;
    }
    if s.is("repository") {
        return (s.id == base.key).then_some(Target {
            identity: Some(base),
        });
    }
    let number = s.id.strip_prefix(&format!("{}/pull/", base.key))?;
    if number
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0)
        .is_none_or(|n| n.to_string() != number)
        || s.field("number").is_some_and(|n| n != number)
    {
        return None;
    }
    let source = match s.field("source_repository_url") {
        Some(url) => {
            let source = repository(url)?;
            if source.host != base.host {
                return None;
            }
            Some(source)
        }
        None => None,
    };
    Some(Target {
        identity: source,
    })
}

pub(crate) fn related(p: &GitPlugin, subject: &Subject) -> Vec<Subject> {
    let Some(identity) = target(subject).and_then(|target| target.identity) else {
        return vec![];
    };
    checkouts(p, &identity.key)
        .into_iter()
        .filter_map(|checkout| {
            let key = radar_core::journal::SubjectKey::new(
                "local",
                &checkout.dir,
                "checkout",
                &checkout.dir,
            )
            .ok()?;
            let mut candidate = crate::source_navigation::from_journal(p, &key)?;
            candidate.label = format!("local folder · {}", checkout.dir);
            Some(candidate)
        })
        .collect()
}

pub(crate) fn checkouts<'a>(p: &'a GitPlugin, key: &str) -> Vec<&'a Checkout> {
    let Some(want) = repository(key) else {
        return vec![];
    };
    let matches = |c: &&Checkout| {
        Path::new(&c.dir).is_absolute()
            && !c.dir.chars().any(char::is_control)
            && remote(&c.remote).as_ref() == Some(&want)
    };
    let mut out = Vec::new();
    for link in p.cfg.links.iter().filter(|l| l.repo == key) {
        let dir = local::expand(&link.dir).display().to_string();
        out.extend(
            p.local
                .linked
                .iter()
                .filter(|c| c.dir == dir)
                .filter(matches),
        );
        out.extend(
            p.local
                .linked
                .iter()
                .filter(|c| c.main == dir)
                .filter(matches),
        );
    }
    let found = p
        .local
        .index
        .iter()
        .flat_map(|i| i.checkouts.iter())
        .filter(matches);
    let (trees, clones): (Vec<_>, Vec<_>) = found.partition(|c| !c.main.is_empty());
    for c in clones.into_iter().chain(trees) {
        if !out.iter().any(|old: &&Checkout| old.dir == c.dir) {
            out.push(c);
        }
    }
    out
}

fn chosen<'a>(p: &'a GitPlugin, key: &str) -> Option<&'a Checkout> {
    let all = checkouts(p, key);
    p.local
        .use_worktree
        .get(key)
        .and_then(|dir| all.iter().find(|c| &c.dir == dir))
        .or_else(|| all.first())
        .copied()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Selection {
    key: String,
    dir: Option<String>,
    branch: Option<String>,
}

fn selection(p: &GitPlugin, t: &Target) -> Option<Selection> {
    let key = t.identity.as_ref()?.key.clone();
    let c = chosen(p, &key);
    Some(Selection {
        key,
        dir: c.map(|c| c.dir.clone()),
        branch: c.map(|c| c.branch.clone()),
    })
}

fn action(s: &Selection, key: Option<char>, label: &str, verb: &str, icon: Icon) -> Action {
    let id = format!(
        "scoped:{verb}:{}",
        serde_json::to_string(s).expect("selection serializes")
    );
    let mut action = Action::new(key.unwrap_or(' '), label, id).icon(icon);
    action.key = key;
    action
}

pub(crate) fn contribute(p: &GitPlugin, point: Point, s: &Subject) -> Option<Contribution> {
    let t = target(s)?;
    if point == Point::Line {
        let mut words = vec![Chip::new("local folder  ", Tone::Muted).icon(Icon::Local)];
        let Some(sel) = selection(p, &t) else {
            words.push(Chip::new(
                "source repository unavailable; no local folder can be matched",
                Tone::Muted,
            ));
            return Some(Contribution::Line(words));
        };
        if let Some(c) = chosen(p, &sel.key) {
            // Keep this one-line contribution factual at narrow widths. Folder
            // actions remain in the context menu and never change its branch.
            words.push(Chip::new(local::display_path(p, &c.dir), Tone::Plain));
            words.push(Chip::new(format!(" · on {}", c.branch), Tone::Muted));
        } else {
            words.push(Chip::new(
                "no matching origin; link a folder or scan configured folders",
                Tone::Muted,
            ));
        }
        return Some(Contribution::Line(words));
    }
    let sel = selection(p, &t)?;
    if point == Point::Mark && s.is("repository") {
        let c = chosen(p, &sel.key)?;
        return Some(Contribution::Mark {
            icon: Icon::Local,
            words: vec![Chip::new(c.branch.clone(), Tone::Muted)],
        });
    }
    if point != Point::Actions {
        return None;
    }
    let mut actions = Vec::new();
    if sel.dir.is_some() {
        actions.push(action(
            &sel,
            Some('e'),
            "open current folder in an editor",
            "editor",
            Icon::Editor,
        ));
        actions.push(action(
            &sel,
            Some('t'),
            "open terminal in current folder",
            "terminal",
            Icon::Terminal,
        ));
        if checkouts(p, &sel.key).len() > 1 {
            actions.push(action(
                &sel,
                None,
                "use another matching local folder…",
                "use",
                Icon::Local,
            ));
        }
    }
    actions.push(action(
        &sel,
        Some('s'),
        "scan configured local folders",
        "scan",
        Icon::Search,
    ));
    actions.push(action(
        &sel,
        None,
        "link a matching local folder…",
        "link",
        Icon::Link,
    ));
    Some(Contribution::Actions(actions))
}

#[derive(Debug, Default)]
pub(crate) struct State {
    next: u64,
    menu: Option<MenuContext>,
    pending: Option<PendingLink>,
    scan: Option<(u64, local::Local)>,
}

impl State {
    pub(crate) fn invalidate(&mut self) -> bool {
        self.next = self.next.wrapping_add(1);
        self.menu = None;
        self.pending = None;
        self.scan.take().is_some()
    }
}

#[derive(Debug)]
struct PendingLink {
    nonce: u64,
    key: String,
    dir: String,
    previous: Option<String>,
    links: Vec<Link>,
}

#[derive(Clone, Debug)]
struct MenuContext {
    id: String,
    selection: Selection,
    links: Vec<Link>,
    editors: Option<Vec<String>>,
}

fn stale() -> Vec<Effect> {
    vec![Effect::Notify(
        "local folder changed; open its actions again".into(),
    )]
}

fn menu_context(p: &mut GitPlugin, selection: Selection, editors: Option<Vec<String>>) -> String {
    p.scoped.next = p.scoped.next.wrapping_add(1);
    let id = format!("scoped-menu:{}", p.scoped.next);
    p.scoped.menu = Some(MenuContext {
        id: id.clone(),
        selection,
        links: p.cfg.links.clone(),
        editors,
    });
    id
}

fn current(p: &GitPlugin, s: &Selection) -> bool {
    let c = chosen(p, &s.key);
    c.map(|c| &c.dir) == s.dir.as_ref() && c.map(|c| &c.branch) == s.branch.as_ref()
}

pub(crate) fn act(p: &mut GitPlugin, s: &Subject, id: &str) -> Option<Vec<Effect>> {
    let rest = id.strip_prefix("scoped:")?;
    let Some((verb, raw)) = rest.split_once(':') else {
        return Some(stale());
    };
    let Ok(sel) = serde_json::from_str::<Selection>(raw) else {
        return Some(stale());
    };
    if target(s).and_then(|t| selection(p, &t)).as_ref() != Some(&sel) {
        return Some(stale());
    }
    Some(match verb {
        "scan" => scan(p),
        "link" => {
            let id = menu_context(p, sel.clone(), None);
            vec![Effect::Menu(
                Menu::typed(
                    id,
                    format!("link {} to a matching local folder", sel.key),
                    sel.dir.as_deref().map(local::tilde).unwrap_or_default(),
                )
                .icon(Icon::Link),
            )]
        }
        "use" => {
            let id = menu_context(p, sel.clone(), None);
            let items = checkouts(p, &sel.key)
                .into_iter()
                .map(|c| {
                    MenuItem::pick(
                        local::tilde(&c.dir),
                        format!("on {} · keeps this branch", c.branch),
                        format!("use:{}", c.dir),
                    )
                })
                .collect();
            vec![Effect::Menu(
                Menu::new(id, format!("use which folder for {}?", sel.key), items)
                    .icon(Icon::Local),
            )]
        }
        "editor" | "terminal" => launch(p, &sel, verb, None),
        _ => vec![],
    })
}

/// Pure syntax validation; existence and origin are checked in a worker on save.
pub(crate) fn folder_problem(text: &str) -> Option<&'static str> {
    let cleaned = local::clean_folder(text);
    if cleaned.is_empty() {
        Some("type or paste a local folder")
    } else if cleaned.chars().any(char::is_control) {
        Some("folder must not contain control characters")
    } else if !local::expand(&cleaned).is_absolute() {
        Some("use an absolute folder or ~/ path")
    } else {
        None
    }
}

pub(crate) fn typing(p: &GitPlugin, id: &str, text: &str) -> Option<Typing> {
    let context = p
        .scoped
        .menu
        .as_ref()
        .filter(|c| c.id == id && c.editors.is_none())?;
    Some(Typing {
        verdict: Some(match folder_problem(text) {
            Some(why) => Verdict::Bad(why.into()),
            None => {
                Verdict::Unsure("Enter checks the folder's origin before saving the link".into())
            }
        }),
        offers: checkouts(p, &context.selection.key)
            .into_iter()
            .map(|c| {
                MenuItem::pick(
                    local::tilde(&c.dir),
                    format!("matching origin · on {}", c.branch),
                    c.dir.clone(),
                )
            })
            .collect(),
    })
}

pub(crate) fn on_menu(p: &mut GitPlugin, id: &str, value: &str) -> Option<Vec<Effect>> {
    if !id.starts_with("scoped-menu:") {
        return None;
    }
    let Some(context) = p.scoped.menu.take().filter(|c| c.id == id) else {
        return Some(stale());
    };
    if context.links != p.cfg.links || !current(p, &context.selection) {
        return Some(stale());
    }
    Some(if let Some(editors) = context.editors {
        if editors.iter().any(|e| e == value) {
            launch(p, &context.selection, "editor", Some(value.to_string()))
        } else {
            stale()
        }
    } else if let Some(dir) = value.strip_prefix("use:") {
        if checkouts(p, &context.selection.key)
            .iter()
            .any(|c| c.dir == dir)
        {
            p.local
                .use_worktree
                .insert(context.selection.key, dir.into());
            vec![
                persist(p),
                Effect::Notify("local folder selected; its branch is unchanged".into()),
            ]
        } else {
            stale()
        }
    } else {
        request_link(p, &context.selection.key, value)
    })
}

pub(crate) enum Message {
    ScanFailed {
        nonce: u64,
    },
    Scanned {
        nonce: u64,
        index: local::Index,
        linked: Vec<Checkout>,
        omitted: usize,
    },
    Linked {
        nonce: u64,
        key: String,
        dir: String,
        previous: Option<String>,
        result: Result<Checkout, &'static str>,
    },
    Editors {
        selection: Selection,
        links: Vec<Link>,
        installed: Vec<(String, String)>,
    },
    Launched {
        result: FolderLaunch,
    },
    Refused,
}

fn task(f: impl FnOnce() -> Message + Send + 'static) -> Effect {
    task_or(f, Message::Refused)
}

fn task_or(f: impl FnOnce() -> Message + Send + 'static, failure: Message) -> Effect {
    Effect::Spawn(Box::pin(async move {
        Box::new(tokio::task::spawn_blocking(f).await.unwrap_or(failure)) as Msg
    }))
}

fn checked(key: &str, dir: &str) -> Result<Checkout, &'static str> {
    let want = repository(key).ok_or("repository URL is not canonical")?;
    folder_problem(dir).map_or(Ok(()), Err)?;
    let path = std::fs::canonicalize(local::expand(&local::clean_folder(dir)))
        .map_err(|_| "local folder is unavailable")?;
    let c = local::read_checkout(&path).ok_or("folder is not a checkout with an origin remote")?;
    if remote(&c.remote).as_ref() != Some(&want) {
        return Err("folder origin does not match this repository's host and path");
    }
    safe_checkout(c).ok_or("folder origin is unsupported")
}

pub(crate) fn request_link(p: &mut GitPlugin, key: &str, dir: &str) -> Vec<Effect> {
    request_relink(p, key, dir, None)
}

pub(crate) fn request_relink(
    p: &mut GitPlugin,
    key: &str,
    dir: &str,
    previous: Option<String>,
) -> Vec<Effect> {
    if repository(key).is_none() || folder_problem(dir).is_some() {
        return stale();
    }
    p.scoped.next = p.scoped.next.wrapping_add(1);
    let nonce = p.scoped.next;
    p.scoped.pending = Some(PendingLink {
        nonce,
        key: key.into(),
        dir: dir.into(),
        previous: previous.clone(),
        links: p.cfg.links.clone(),
    });
    let (key, dir) = (key.to_string(), dir.to_string());
    vec![task(move || Message::Linked {
        nonce,
        result: checked(&key, &dir),
        key,
        dir,
        previous,
    })]
}

fn scan(p: &mut GitPlugin) -> Vec<Effect> {
    if p.local.scanning {
        return vec![Effect::Notify("already scanning local folders".into())];
    }
    if p.cfg.roots.is_empty() && p.cfg.links.is_empty() {
        return vec![Effect::Notify(
            "no local folders configured; link a folder or add scan roots in settings".into(),
        )];
    }
    p.scoped.next = p.scoped.next.wrapping_add(1);
    let nonce = p.scoped.next;
    let config = p.cfg.clone();
    p.scoped.scan = Some((nonce, config.clone()));
    p.local.scanning = true;
    p.local.started = true;
    let now = p.now;
    vec![
        Effect::Journal(radar_core::journal::ActionDescriptor::new(
            "git.checkouts.scan",
            radar_core::journal::Origin::Plugin("git".into()),
        )),
        task_or(
            move || {
                let mut index = local::scan(&config.roots, config.depth);
                index.at = now;
                let mut linked = local::read_links(&config.links);
                let omitted = safe_checkouts(&mut index.checkouts) + safe_checkouts(&mut linked);
                Message::Scanned {
                    nonce,
                    index,
                    linked,
                    omitted,
                }
            },
            Message::ScanFailed { nonce },
        ),
    ]
}

fn launch(p: &mut GitPlugin, sel: &Selection, what: &str, picked: Option<String>) -> Vec<Effect> {
    if !current(p, sel) {
        return stale();
    }
    let Some(c) = chosen(p, &sel.key) else {
        return stale();
    };
    let editor = picked
        .or_else(|| p.cfg.type_of(c).map(|t| t.editor.clone()))
        .unwrap_or_default();
    let terminal = p.cfg.terminal.clone();
    let selection = sel.clone();
    let links = p.cfg.links.clone();
    let want_editor = what == "editor";
    let journal = crate::act::journal_at(
        if want_editor {
            "local.editor.launch"
        } else {
            "local.terminal.launch"
        },
        &c.dir,
        &c.project,
        &c.branch,
    );
    vec![
        journal,
        task(move || {
            let Some(dir) = selection.dir.as_deref() else {
                return Message::Refused;
            };
            let Ok(c) = checked(&selection.key, dir) else {
                return Message::Refused;
            };
            if Some(&c.branch) != selection.branch.as_ref() {
                return Message::Refused;
            }
            let env = Env::detect();
            if want_editor {
                let program = system::split_command(&editor)
                    .into_iter()
                    .next()
                    .unwrap_or_default();
                if program.is_empty() || !system::on_path(&program) {
                    return Message::Editors {
                        selection,
                        links,
                        installed: system::installed_editors(&env),
                    };
                }
            }
            let name = Path::new(&c.dir)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let what = if want_editor {
                InFolder::Editor(&editor)
            } else {
                InFolder::Terminal
            };
            let ways = system::folder_ways(&env, &c.dir, what, &terminal, &name);
            Message::Launched {
                result: system::start_folder(ways, Path::new(&c.dir)),
            }
        }),
    ]
}

pub(crate) fn on_msg(p: &mut GitPlugin, message: Message) -> Vec<Effect> {
    match message {
        Message::ScanFailed { nonce } => {
            if p.scoped
                .scan
                .as_ref()
                .is_some_and(|(expected, _)| *expected == nonce)
            {
                p.scoped.scan = None;
                p.local.scanning = false;
            }
            vec![
                Effect::Report(radar_core::journal::Outcome::Unknown),
                Effect::error(None, "local-folder scan failed; try scanning again"),
            ]
        }
        Message::Scanned {
            nonce,
            mut index,
            mut linked,
            omitted,
        } => {
            let Some((expected, config)) = p.scoped.scan.as_ref() else {
                return stale();
            };
            if *expected != nonce {
                return stale();
            }
            let current = config == &p.cfg;
            p.scoped.scan = None;
            p.local.scanning = false;
            if !current {
                return stale();
            }
            // Keep the reducer boundary defensive even when a fixture or future
            // worker implementation bypasses the normal scoped scan helper.
            let omitted =
                omitted + safe_checkouts(&mut index.checkouts) + safe_checkouts(&mut linked);
            let partial = omitted > 0
                || index.cut
                || !index.missing.is_empty()
                || p.cfg.links.iter().any(|link| {
                    let dir = local::expand(&link.dir).display().to_string();
                    !linked.iter().any(|c| {
                        c.dir == dir
                            && (!is_scoped(&link.repo)
                                || remote(&c.remote) == repository(&link.repo))
                    })
                });
            let message = if partial {
                "local-folder scan incomplete; some folders are missing, unsupported, mismatched or beyond the scan limit"
            } else {
                "local folders and linked origins checked again"
            };
            p.local.index = Some(index);
            p.local.linked = linked;
            p.local.linked_read = true;
            vec![
                Effect::Report(if partial {
                    radar_core::journal::Outcome::Partial
                } else {
                    radar_core::journal::Outcome::Succeeded
                }),
                persist(p),
                Effect::Notify(message.into()),
            ]
        }
        Message::Linked {
            nonce,
            key,
            dir,
            previous,
            result,
        } => {
            if !p.scoped.pending.as_ref().is_some_and(|pending| {
                pending.nonce == nonce
                    && pending.links == p.cfg.links
                    && pending.key == key
                    && pending.dir == dir
                    && pending.previous == previous
            }) {
                return stale();
            }
            p.scoped.pending = None;
            let c = match result {
                Ok(c) => c,
                Err(why) => return vec![Effect::error(None, why)],
            };
            if repository(&key).is_none()
                || remote(&c.remote) != repository(&key)
                || !Path::new(&c.dir).is_absolute()
            {
                return stale();
            }
            let Some(c) = safe_checkout(c) else {
                return stale();
            };
            if let Some(link) = p
                .cfg
                .links
                .iter_mut()
                .find(|l| l.repo == key || previous.as_ref() == Some(&l.repo))
            {
                link.repo = key.clone();
                link.dir = local::tilde(&c.dir);
            } else {
                p.cfg.links.push(Link {
                    repo: key.clone(),
                    dir: local::tilde(&c.dir),
                    ..Link::default()
                });
            }
            p.local.use_worktree.insert(key, c.dir.clone());
            p.local.linked.retain(|old| old.dir != c.dir);
            p.local.linked.push(c);
            p.local.rows[2] = p.cfg.links.len();
            vec![
                persist(p),
                Effect::Notify("repository linked to its matching local folder".into()),
            ]
        }
        Message::Editors {
            selection,
            links,
            installed,
        } => {
            if links != p.cfg.links || !current(p, &selection) {
                return stale();
            }
            if installed.is_empty() {
                return vec![Effect::Notify(
                    "no editor found; set one in settings under editors".into(),
                )];
            }
            let title = format!(
                "open current folder for {} with which editor?",
                selection.key
            );
            let id = menu_context(
                p,
                selection,
                Some(installed.iter().map(|(cmd, _)| cmd.clone()).collect()),
            );
            let items = installed
                .into_iter()
                .map(|(cmd, name)| MenuItem::pick(name, "open current folder", cmd))
                .collect();
            vec![Effect::Menu(Menu::new(id, title, items))]
        }
        Message::Launched { result } => local::on_msg(
            p,
            local::LocalMsg::Launched {
                result,
                what: "local folder".into(),
                also: String::new(),
            },
        ),
        Message::Refused => vec![
            Effect::Report(radar_core::journal::Outcome::Blocked),
            Effect::Notify("local folder or origin changed; scan and choose again".into()),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_core::Plugin;
    use std::sync::atomic::{AtomicU64, Ordering};

    const BASE: &str = "https://forge.example/acme/api";
    const FORK: &str = "https://forge.example/alice/api";

    fn repo(key: &str) -> Subject {
        Subject::new("repository", key, "api")
            .with_url(key)
            .with("repository_url", key)
    }
    fn pull(source: Option<&str>) -> Subject {
        let url = format!("{BASE}/pull/7");
        let s = Subject::new("pull-request", &url, "api #7")
            .with_url(url)
            .with("repository_url", BASE)
            .with("branch", "review/topic");
        match source {
            Some(source) => s.with("source_repository_url", source),
            None => s,
        }
    }
    fn checkout(dir: &str, remote: &str) -> Checkout {
        Checkout {
            dir: dir.into(),
            remote: remote.into(),
            project: "acme/api".into(),
            branch: "main".into(),
            ..Checkout::default()
        }
    }
    fn fixture() -> GitPlugin {
        let mut p = GitPlugin::new();
        p.local.index = Some(local::Index {
            checkouts: vec![
                checkout("/code/base", "git@forge.example:acme/api.git"),
                checkout("/code/wrong-host", "git@other.example:acme/api.git"),
                checkout("/code/fork", "https://forge.example/alice/api.git"),
            ],
            ..local::Index::default()
        });
        p
    }
    #[test]
    fn source_navigation_related_folders_keep_exact_source_scope() {
        let p = fixture();
        let ids = |subject: Subject| {
            p.related(&subject)
                .into_iter()
                .map(|s| s.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(repo(BASE)), ["/code/base"]);
        assert_eq!(ids(pull(Some(FORK))), ["/code/fork"]);
        assert!(ids(pull(None)).is_empty());
        assert!(ids(pull(Some("https://other.example/acme/api"))).is_empty());
        assert!(ids(pull(Some("https://user:secret@forge.example/alice/api"))).is_empty());
        assert!(ids(pull(Some(FORK)).with("repository", "another/api")).is_empty());
        let before = p.snapshot();
        let candidates = p.related(&pull(Some(FORK)));
        assert_eq!(candidates[0].label, "local folder · /code/fork");
        assert_eq!(
            p.journal_subject(&candidates[0]).unwrap().key.provider,
            "local"
        );
        assert_eq!(p.snapshot(), before);
    }

    fn actions(p: &GitPlugin, s: &Subject) -> Vec<Action> {
        match p.contribute(Point::Actions, s) {
            Some(Contribution::Actions(actions)) => actions,
            _ => vec![],
        }
    }
    fn action_id(p: &GitPlugin, s: &Subject, verb: &str) -> String {
        actions(p, s)
            .into_iter()
            .find(|a| a.id.starts_with(&format!("scoped:{verb}:")))
            .unwrap()
            .id
    }
    fn menu(effects: Vec<Effect>) -> Menu {
        effects
            .into_iter()
            .find_map(|e| match e {
                Effect::Menu(m) => Some(m),
                _ => None,
            })
            .unwrap()
    }
    async fn run_task(effects: Vec<Effect>) -> Msg {
        let task = effects
            .into_iter()
            .find_map(|e| match e {
                Effect::Spawn(t) => Some(t),
                _ => None,
            })
            .unwrap();
        task.await
    }
    fn no_task(effects: &[Effect]) -> bool {
        effects
            .iter()
            .all(|e| !matches!(e, Effect::Spawn(_) | Effect::SpawnAction { .. }))
    }

    struct Scratch(std::path::PathBuf);
    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!(
                "radar-scoped-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            // Resolved: macOS's temporary folder is a link (/var -> /private/var).
            Self(std::fs::canonicalize(&dir).unwrap())
        }
        fn checkout(&self, name: &str, origin: &str) -> String {
            let dir = self.0.join(name);
            std::fs::create_dir_all(dir.join(".git")).unwrap();
            std::fs::write(
                dir.join(".git/config"),
                format!("[remote \"origin\"]\nurl = {origin}\n"),
            )
            .unwrap();
            std::fs::write(dir.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
            dir.display().to_string()
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn origins_normalize_without_discarding_host_or_fork() {
        for origin in [
            "https://forge.example/acme/api.git",
            "https://FORGE.EXAMPLE/Acme/API.git/",
            "https://forge.example:443/acme/api",
            "ssh://git@forge.example/acme/api.git",
            "ssh://git@forge.example:22/acme/api.git",
            "git@forge.example:acme/api.git",
        ] {
            assert_eq!(remote(origin), repository(BASE), "{origin}");
        }
        assert_ne!(remote("git@other.example:acme/api.git"), repository(BASE));
        assert_ne!(remote("git@forge.example:alice/api.git"), repository(BASE));
        assert_ne!(
            remote("git@forge.example.evil.test:acme/api.git"),
            repository(BASE)
        );
        for unsafe_origin in [
            "https://user:secret@forge.example/acme/api.git",
            "https://user@forge.example/acme/api",
            "ssh://git:secret@forge.example/acme/api",
            "ssh://git@forge.example:2222/acme/api",
            "https://forge.example/acme/../api",
            "https://forge.example/acme/%61pi",
            "https://forge.example/acme/api?token=x",
            "https://forge.example/acme/api#x",
            "https://forge.example/acme/api\n",
            "file:///code/acme/api",
            "/code/acme/api",
            "C:\\code\\acme\\api",
            "https://forge.example//acme/api",
            "https://forge.example/acme/api//",
        ] {
            assert!(remote(unsafe_origin).is_none(), "{unsafe_origin}");
        }
    }

    #[test]
    fn subjects_choose_exact_base_or_exact_source_and_never_guess() {
        let p = fixture();
        let base = target(&repo(BASE)).unwrap();
        assert_eq!(
            selection(&p, &base).unwrap().dir.as_deref(),
            Some("/code/base")
        );
        let fork = target(&pull(Some(FORK))).unwrap();
        assert_eq!(
            selection(&p, &fork).unwrap().dir.as_deref(),
            Some("/code/fork")
        );
        assert!(actions(&p, &pull(None)).is_empty());
        let Some(Contribution::Line(line)) = p.contribute(Point::Line, &pull(None)) else {
            panic!()
        };
        assert!(
            line.iter()
                .any(|c| c.text.contains("source repository unavailable"))
        );
        for bad in [
            repo(BASE).with("repository_url", FORK),
            repo(BASE).with_url(FORK),
            pull(Some(FORK)).with("source_repository_url", "https://other.example/alice/api"),
            pull(Some(FORK)).with("number", "8"),
            pull(Some(FORK)).with("instance", "https://other.example"),
        ] {
            assert!(target(&bad).is_none());
            assert!(actions(&p, &bad).is_empty());
        }
    }

    #[test]
    fn scoped_folder_actions_never_offer_repository_mutations() {
        let p = fixture();
        let actions = actions(&p, &pull(Some(FORK)));
        assert_eq!(actions.len(), 4);
        for a in actions {
            assert!(
                ["editor", "terminal", "scan", "link"]
                    .iter()
                    .any(|v| a.id.starts_with(&format!("scoped:{v}:")))
            );
        }
        let Some(Contribution::Line(line)) = p.contribute(Point::Line, &pull(Some(FORK))) else {
            panic!()
        };
        assert!(line.iter().any(|c| c.text.contains("/code/fork")));
        assert!(line.iter().any(|c| c.text.contains(" · on ")));
    }

    #[test]
    fn old_path_links_and_selected_worktrees_cannot_cross_into_scoped_matching() {
        let mut p = fixture();
        p.cfg.links.push(Link {
            repo: "acme/api".into(),
            dir: "/code/wrong-host".into(),
            ..Link::default()
        });
        p.local.linked = p.local.index.as_ref().unwrap().checkouts.clone();
        p.local
            .use_worktree
            .insert("acme/api".into(), "/code/wrong-host".into());
        assert_eq!(chosen(&p, BASE).unwrap().dir, "/code/base");
        p.cfg.links.push(Link {
            repo: BASE.into(),
            dir: "/code/wrong-host".into(),
            ..Link::default()
        });
        p.local
            .use_worktree
            .insert(BASE.into(), "/code/wrong-host".into());
        assert_eq!(chosen(&p, BASE).unwrap().dir, "/code/base");
        p.local.index = None;
        assert!(chosen(&p, BASE).is_none());
    }

    #[test]
    fn stale_action_and_followup_menu_cannot_open_a_new_selection() {
        let mut p = fixture();
        let s = repo(BASE);
        let id = action_id(&p, &s, "terminal");
        p.local.index.as_mut().unwrap().checkouts[0].remote =
            "git@other.example:acme/api.git".into();
        assert!(no_task(&p.act(&s, &id)));
        let mut p = fixture();
        let id = action_id(&p, &s, "link");
        let m = menu(p.act(&s, &id));
        p.cfg.links.push(Link::default());
        assert!(no_task(&p.on_menu(&m.id, "/code/base")));
        let mut p = fixture();
        let id = action_id(&p, &pull(Some(FORK)), "terminal");
        assert!(no_task(&p.act(&pull(Some(BASE)), &id)));
    }

    #[tokio::test]
    async fn manual_link_is_checked_saved_reloaded_and_does_not_migrate_old_links() {
        let scratch = Scratch::new();
        let dir = scratch.checkout("api", "git@forge.example:acme/api.git");
        let mut p = GitPlugin::new();
        let old = Link {
            repo: "acme/legacy".into(),
            dir: "/legacy".into(),
            branch: "old/{key}".into(),
            base: "trunk".into(),
            ..Default::default()
        };
        p.cfg.links.push(old.clone());
        let s = repo(BASE);
        let id = action_id(&p, &s, "link");
        let m = menu(p.act(&s, &id));
        assert!(
            m.title.contains(BASE),
            "the follow-on prompt visibly binds its exact repository"
        );
        assert!(matches!(
            p.typing(&m.id, "relative/path").verdict,
            Some(Verdict::Bad(_))
        ));
        let effects = p.on_menu(&m.id, &dir);
        assert_eq!(
            p.cfg.links.as_slice(),
            std::slice::from_ref(&old),
            "no link before filesystem verification"
        );
        let result = run_task(effects).await;
        p.on_msg(result);
        assert_eq!(p.cfg.links.len(), 2);
        assert_eq!(p.cfg.links[0], old);
        assert_eq!(p.cfg.links[1].repo, BASE);
        let saved = p.config().unwrap();
        let mut restored = GitPlugin::new();
        restored.configure(Some(&saved)).unwrap();
        local::refresh_links(&mut restored);
        assert_eq!(chosen(&restored, BASE).unwrap().dir, dir);
        assert_eq!(restored.cfg.links[0], old);
    }

    #[tokio::test]
    async fn link_settings_keyboard_flow_verifies_before_autosave() {
        use radar_core::plugin::{Outcome, parse_key};
        let scratch = Scratch::new();
        let dir = scratch.checkout("settings", "git@forge.example:acme/api.git");
        let mut p = GitPlugin::new();
        p.section_key("links", parse_key("enter").unwrap());
        p.paste(BASE);
        p.section_key("links", parse_key("enter").unwrap());
        p.paste(&dir);
        let Outcome::Handled(effects) = p.section_key("links", parse_key("enter").unwrap()) else {
            panic!()
        };
        assert!(p.cfg.links.is_empty());
        assert!(!p.capturing());
        let result = run_task(effects).await;
        p.on_msg(result);
        assert_eq!(p.cfg.links[0].repo, BASE);
        assert_eq!(chosen(&p, BASE).unwrap().dir, dir);
    }

    #[tokio::test]
    async fn wrong_host_fork_missing_path_and_stale_validation_are_never_linked() {
        let scratch = Scratch::new();
        for (name, origin) in [
            ("other-host", "git@other.example:acme/api.git"),
            ("fork", "git@forge.example:alice/api.git"),
        ] {
            let dir = scratch.checkout(name, origin);
            let mut p = GitPlugin::new();
            let result = run_task(request_link(&mut p, BASE, &dir)).await;
            p.on_msg(result);
            assert!(p.cfg.links.is_empty());
        }
        let mut p = GitPlugin::new();
        let result = run_task(request_link(
            &mut p,
            BASE,
            &scratch.0.join("missing").display().to_string(),
        ))
        .await;
        p.on_msg(result);
        assert!(p.cfg.links.is_empty());
        let dir = scratch.checkout("valid", "git@forge.example:acme/api.git");
        let result = run_task(request_link(&mut p, BASE, &dir)).await;
        p.cfg.links.push(Link::default());
        p.on_msg(result);
        assert_eq!(p.cfg.links, [Link::default()]);
    }

    #[tokio::test]
    async fn relinking_checks_new_repository_before_preserving_existing_options() {
        let scratch = Scratch::new();
        let dir = scratch.checkout("api", "git@forge.example:acme/api.git");
        let mut p = GitPlugin::new();
        p.cfg.links.push(Link {
            repo: "acme/api".into(),
            dir: dir.clone(),
            branch: "me/{key}".into(),
            base: "trunk".into(),
            worktree: "/work/trees/watch/{topic}".into(),
        });
        let result = run_task(request_relink(&mut p, BASE, &dir, Some("acme/api".into()))).await;
        p.on_msg(result);
        assert_eq!(p.cfg.links.len(), 1);
        assert_eq!(p.cfg.links[0].repo, BASE);
        assert_eq!(p.cfg.links[0].branch, "me/{key}");
        assert_eq!(p.cfg.links[0].worktree, "/work/trees/watch/{topic}");
        assert_eq!(p.cfg.links[0].base, "trunk");
    }

    #[tokio::test]
    async fn launch_rechecks_origin_on_disk_before_any_external_process() {
        let scratch = Scratch::new();
        let dir = scratch.checkout("api", "git@other.example:acme/api.git");
        let mut p = fixture();
        p.local.index.as_mut().unwrap().checkouts[0].dir = dir;
        let s = repo(BASE);
        let id = action_id(&p, &s, "terminal");
        let message = run_task(p.act(&s, &id)).await;
        assert!(matches!(
            *message.downcast::<Message>().unwrap(),
            Message::Refused
        ));
    }

    #[tokio::test]
    async fn old_link_result_cannot_collide_with_request_after_reconfigure() {
        let scratch = Scratch::new();
        let old_dir = scratch.checkout("old", "git@forge.example:acme/api.git");
        let new_dir = scratch.checkout("new", "git@forge.example:acme/api.git");
        let mut p = GitPlugin::new();
        let old = run_task(request_link(&mut p, BASE, &old_dir)).await;
        p.configure(None).unwrap();
        let new = run_task(request_link(&mut p, BASE, &new_dir)).await;
        p.on_msg(old);
        assert!(p.cfg.links.is_empty());
        p.on_msg(new);
        assert_eq!(p.cfg.links.len(), 1);
        assert_eq!(
            local::expand(&p.cfg.links[0].dir).display().to_string(),
            new_dir
        );
    }

    #[tokio::test]
    async fn explicit_link_replaces_previous_folder_selection() {
        let scratch = Scratch::new();
        let dir = scratch.checkout("new", "git@forge.example:acme/api.git");
        let mut p = fixture();
        p.local
            .use_worktree
            .insert(BASE.into(), "/code/base".into());
        let linked = run_task(request_link(&mut p, BASE, &dir)).await;
        let effects = p.on_msg(linked);
        assert_eq!(chosen(&p, BASE).unwrap().dir, dir);
        assert!(effects.iter().any(|e| matches!(e, Effect::PersistCache)));
    }

    #[tokio::test]
    async fn scoped_scan_refreshes_explicit_folders_outside_roots_and_recovers_branch_changes() {
        let scratch = Scratch::new();
        let dir = scratch.checkout("outside-roots", "git@forge.example:acme/api.git");
        let mut p = GitPlugin::new();
        let linked = run_task(request_link(&mut p, BASE, &dir)).await;
        p.on_msg(linked);
        let s = repo(BASE);
        let old_action = action_id(&p, &s, "terminal");
        std::fs::write(
            Path::new(&dir).join(".git/HEAD"),
            "ref: refs/heads/new-branch\n",
        )
        .unwrap();
        let refused = run_task(p.act(&s, &old_action)).await;
        assert!(matches!(
            *refused.downcast::<Message>().unwrap(),
            Message::Refused
        ));
        let scan_action = action_id(&p, &s, "scan");
        let scanned = run_task(p.act(&s, &scan_action)).await;
        p.on_msg(scanned);
        assert!(!p.local.scanning);
        assert_eq!(chosen(&p, BASE).unwrap().branch, "new-branch");
        assert!(
            no_task(&p.act(&s, &old_action)),
            "old menu remains stale after recovery"
        );
        assert_ne!(action_id(&p, &s, "terminal"), old_action);
        std::fs::write(
            Path::new(&dir).join(".git/config"),
            "[remote \"origin\"]\nurl = git@other.example:acme/api.git\n",
        )
        .unwrap();
        let scan_action = action_id(&p, &s, "scan");
        let scanned = run_task(p.act(&s, &scan_action)).await;
        p.on_msg(scanned);
        assert!(chosen(&p, BASE).is_none());
        assert!(
            !actions(&p, &s)
                .iter()
                .any(|a| a.id.starts_with("scoped:terminal:"))
        );
    }
    #[tokio::test]
    async fn scoped_scan_reports_missing_folders_as_incomplete() {
        let scratch = Scratch::new();
        let mut p = GitPlugin::new();
        p.cfg
            .roots
            .push(scratch.0.join("missing").display().to_string());
        let scanned = run_task(scan(&mut p)).await;
        let effects = p.on_msg(scanned);
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::Report(radar_core::journal::Outcome::Partial)))
        );
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::Notify(s) if s.contains("incomplete")))
        );
        assert!(!p.local.scanning);
    }

    #[test]
    fn failed_scan_is_retryable_without_canceling_a_newer_scan() {
        let mut p = GitPlugin::new();
        p.cfg.roots.push("/missing".into());
        let _ = scan(&mut p);
        let first = p.scoped.scan.as_ref().unwrap().0;
        on_msg(&mut p, Message::ScanFailed { nonce: first });
        assert!(!p.local.scanning);
        let _ = scan(&mut p);
        on_msg(&mut p, Message::ScanFailed { nonce: first });
        assert!(p.local.scanning);
    }

    const CANARY: &str = "SYNTHETIC_ORIGIN_SECRET_731";

    fn restored_unsafe_index(p: &mut GitPlugin) {
        let unsafe_checkout = Checkout {
            remote: format!("https://forge.example/acme/api?token={CANARY}"),
            project: format!("acme/api?token={CANARY}"),
            ..checkout("/code/private", "")
        };
        let mut checkouts = fixture().local.index.unwrap().checkouts;
        checkouts.push(checkout("/code/second", "git@forge.example:acme/api.git"));
        checkouts.push(unsafe_checkout.clone());
        p.restore(serde_json::json!({
            "index": local::Index { checkouts, ..local::Index::default() },
            "use": { format!("acme/api?token={CANARY}"): "/code/private", "acme/legacy": "/code/legacy" }
        }));
        p.local.linked.push(unsafe_checkout.clone());
        p.local
            .looked
            .insert("acme/api".into(), vec![unsafe_checkout]);
    }

    fn no_canary(p: &GitPlugin, effects: &[Effect]) {
        assert!(!p.snapshot().unwrap().to_string().contains(CANARY));
        assert!(!format!("{p:?}").contains(CANARY));
        assert!(!format!("{effects:?}").contains(CANARY));
    }

    #[tokio::test]
    async fn scoped_scan_drops_credentials_and_derived_query_fields_before_results_or_cache() {
        let scratch = Scratch::new();
        scratch.checkout("safe", "git@FORGE.EXAMPLE:Acme/API.git");
        let unsafe_dir = scratch.checkout(
            "userinfo",
            &format!("https://user:{CANARY}@forge.example/acme/api.git"),
        );
        scratch.checkout(
            "query",
            &format!("https://forge.example/acme/api?token={CANARY}"),
        );
        scratch.checkout(
            "ssh-password",
            &format!("ssh://git:{CANARY}@forge.example/acme/api.git"),
        );
        let mut p = GitPlugin::new();
        p.cfg.roots.push(scratch.0.display().to_string());
        p.cfg.links.push(Link {
            repo: BASE.into(),
            dir: unsafe_dir.clone(),
            ..Link::default()
        });
        let result = run_task(scan(&mut p)).await;
        let Message::Scanned {
            index,
            linked,
            omitted,
            ..
        } = result.downcast_ref::<Message>().unwrap()
        else {
            panic!()
        };
        assert_eq!(index.checkouts.len(), 1);
        assert!(linked.is_empty());
        assert!(*omitted >= 3);
        assert!(!format!("{index:?}{linked:?}").contains(CANARY));
        assert_eq!(index.checkouts[0].project, "Acme/API");
        assert_eq!(index.checkouts[0].remote, "git@FORGE.EXAMPLE:Acme/API.git");
        let effects = p.on_msg(result);
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::Report(radar_core::journal::Outcome::Partial)))
        );
        no_canary(&p, &effects);
        let result = run_task(request_link(&mut p, BASE, &unsafe_dir)).await;
        let effects = p.on_msg(result);
        no_canary(&p, &effects);
        assert!(
            effects.iter().any(
                |e| matches!(e, Effect::Notice(text) if text.tone == radar_core::view::chrome::NoticeTone::Error && text.contains("origin does not match"))
            )
        );
    }

    #[tokio::test]
    async fn scoped_link_scrubs_restored_origins_before_persisting_without_a_scan() {
        let scratch = Scratch::new();
        let dir = scratch.checkout("safe", "git@forge.example:acme/api.git");
        let mut p = GitPlugin::new();
        restored_unsafe_index(&mut p);
        let result = run_task(request_link(&mut p, BASE, &dir)).await;
        let effects = p.on_msg(result);
        assert!(effects.iter().any(|e| matches!(e, Effect::PersistCache)));
        no_canary(&p, &effects);
        assert_eq!(
            p.local.use_worktree.get("acme/legacy").map(String::as_str),
            Some("/code/legacy")
        );
    }

    #[test]
    fn scoped_use_scrubs_restored_origins_before_persisting_without_a_scan() {
        let mut p = GitPlugin::new();
        restored_unsafe_index(&mut p);
        let s = repo(BASE);
        let action = action_id(&p, &s, "use");
        let prompt = menu(p.act(&s, &action));
        let effects = p.on_menu(&prompt.id, "use:/code/second");
        assert!(effects.iter().any(|e| matches!(e, Effect::PersistCache)));
        no_canary(&p, &effects);
        assert_eq!(chosen(&p, BASE).unwrap().dir, "/code/second");
    }

    #[test]
    fn sanitizing_an_origin_is_idempotent_for_names_ending_in_clone_suffix() {
        let original = checkout("/code/api", "git@forge.example:acme/api.git.git");
        let sanitized = safe_checkout(original.clone()).unwrap();
        assert_eq!(sanitized, original);
        assert_eq!(safe_checkout(sanitized.clone()).unwrap(), sanitized);
    }

    #[test]
    fn scoped_persistence_preserves_safe_legacy_transports_and_metadata() {
        let mut p = GitPlugin::new();
        restored_unsafe_index(&mut p);
        let mut expected = Vec::new();
        for (i, origin) in [
            "ssh://git@forge.example:2222/acme/legacy.git",
            "https://forge.example:8443/acme/legacy.git",
            "http://forge.example/acme/legacy.git",
            "/srv/code/legacy.git",
            "C:\\repos\\legacy.git",
            "ssh://alice@forge-work/acme/legacy.git",
            "forge-work:acme/legacy.git",
        ]
        .into_iter()
        .enumerate()
        {
            let c = Checkout {
                project: "acme/Legacy".into(),
                ..checkout(&format!("/code/legacy-{i}"), origin)
            };
            expected.push(c.clone());
            p.local.index.as_mut().unwrap().checkouts.push(c.clone());
            p.local.linked.push(c.clone());
            p.local
                .looked
                .entry("acme/legacy".into())
                .or_default()
                .push(c);
        }
        let effect = persist(&mut p);
        no_canary(&p, &[effect]);
        for c in expected {
            assert!(p.local.index.as_ref().unwrap().checkouts.contains(&c));
            assert!(p.local.linked.contains(&c));
            assert!(p.local.looked["acme/legacy"].contains(&c));
        }
        assert!(
            checkouts(&p, "https://forge.example/acme/legacy").is_empty(),
            "preserving a legacy entry never qualifies it for scoped matching"
        );
    }
}
