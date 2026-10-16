//! The plugin's settings sections: `connection`, `tenancies`, `views` and
//! `look`. Every row has an options menu (`⏎`), and the menu's keys also work
//! straight from the row. The detail panel says what the selected row is,
//! what it accepts, and whether it has been checked. Anything GitLab can
//! confirm (connection, groups) is offered a check after you change it.

use radar_core::design::help_bar::KeyHint;
use radar_core::input::{Edit, TextInput};
use radar_core::plugin::{Binding, Effect, Menu, MenuItem, Outcome, key_text};
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent};
use radar_core::theme::slot_info;
use radar_core::ui::connection;
use radar_core::ui::rows::{Move, select};
use radar_core::view as v;
use radar_core::{time, ui, widgets};

use crate::config::{Source, TENANT_COLORS, View, default_styles};
use crate::filter;
use crate::model::Role;
use crate::settings_checks::{self, Target as CheckTarget};
use crate::{Check, Client, GitlabPlugin, Target, config};

// --- rows ------------------------------------------------------------------

/// A row of the connection section, in the order every part has
/// (`radar_core::ui::connection`): where, the token, who it signs in as, the
/// check, refreshing; then GitLab's own (history).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Url,
    Base,
    Token,
    SignedIn,
    Check,
    Refresh,
    History,
}

impl Field {
    pub const ALL: [Self; 7] = [
        Self::Url,
        Self::Base,
        Self::Token,
        Self::SignedIn,
        Self::Check,
        Self::Refresh,
        Self::History,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Url => "server",
            Self::Base => "base group",
            Self::Token => connection::TOKEN,
            Self::SignedIn => connection::SIGNED_IN,
            Self::Check => connection::CHECK,
            Self::Refresh => connection::REFRESH,
            Self::History => "merged, closed",
        }
    }

    fn about(self) -> String {
        match self {
            Self::Url => "The GitLab server radar talks to.".into(),
            Self::Token => {
                "How radar proves who you are to GitLab. It needs the scope access asks for: read_api, or api for read and write. A token from glab is renewed once if GitLab turns it down.".into()
            }
            Self::Base => "The group your tenancies sit under. Tenancy paths are relative to it.".into(),
            Self::Refresh => format!(
                "{} After the first sync it asks only for what changed, so a short wait is cheap; under 10 seconds would only load the server.",
                connection::about_refresh("GitLab")
            ),
            Self::History => {
                "How far back merged and closed merge requests go, for views that show them (state:merged). They are fetched only while such a view exists.".into()
            }
            Self::SignedIn => format!(
                "{} Your merge requests are this person's.",
                connection::about_signed_in("GitLab")
            ),
            Self::Check => format!("{} It also looks for the base group.", connection::about_check("GitLab")),
        }
    }

    fn accepts(self) -> Option<String> {
        Some(match self {
            Self::Url => "https://gitlab.com or your own server's address".into(),
            Self::Token => format!("{} · glab reuses glab's login", connection::TOKEN_ACCEPTS),
            Self::Base => {
                "the group's GitLab link or path, e.g. acme/platform; empty if tenancies are full paths".into()
            }
            Self::Refresh => connection::refresh_accepts(10),
            Self::History => "whole days, 1 to 90".into(),
            Self::SignedIn | Self::Check => return None,
        })
    }

    const fn key(self) -> &'static str {
        match self {
            Self::Url => "url",
            Self::Token => "token",
            Self::Base => "base",
            Self::Refresh => "refresh_secs",
            Self::History => "history_days",
            Self::SignedIn | Self::Check => "",
        }
    }

    /// Whether it can be typed into.
    const fn editable(self) -> bool {
        !matches!(self, Self::SignedIn | Self::Check)
    }

    /// The check that confirms this field, if GitLab can confirm it.
    const fn check(self) -> Option<&'static str> {
        match self {
            Self::Url | Self::Token | Self::SignedIn | Self::Check => Some("auth"),
            Self::Base => Some("base"),
            Self::Refresh | Self::History => None,
        }
    }
}

/// A row of the connection section: a token found in the environment,
/// asked about first, then the fields, then the groups followed (the
/// tenancies, as setup's GitLab step shows them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnRow {
    Offer(usize),
    Field(Field),
    /// The gap before the access choices and before the follow list: not
    /// selectable.
    Gap,
    /// `ACCESS`: not selectable.
    AccessHeading,
    /// What the token is made to allow, `[gitlab] access`.
    Access(config::TokenAccess),
    /// `FOLLOW`: not selectable.
    FollowHeading,
    /// `+ add a group or repository`: types into the same field as the
    /// tenancies section's add row.
    FollowAdd,
    /// A tenancy.
    Follow(usize),
}

impl ConnRow {
    /// Whether the cursor stops on it.
    const fn selectable(self) -> bool {
        !matches!(self, Self::Gap | Self::AccessHeading | Self::FollowHeading)
    }
}

pub fn conn_rows(p: &GitlabPlugin) -> Vec<ConnRow> {
    let mut rows: Vec<ConnRow> = (0..p.offers().len()).map(ConnRow::Offer).collect();
    // As a setup step the card leaves out what only Settings needs; the
    // plugin remembers how the card was last drawn, so keys walk what is drawn.
    let setup = p.setup_card.get();
    rows.extend(
        Field::ALL
            .into_iter()
            .filter(|f| !(setup && matches!(f, Field::Base | Field::Refresh | Field::History)))
            .map(ConnRow::Field),
    );
    rows.extend([ConnRow::Gap, ConnRow::AccessHeading]);
    rows.extend(config::TokenAccess::ALL.map(ConnRow::Access));
    rows.extend([ConnRow::Gap, ConnRow::FollowHeading, ConnRow::FollowAdd]);
    rows.extend((0..p.cfg.sources.len()).map(ConnRow::Follow));
    rows
}

/// A tenancy as the follow list shows it: its mark and label in its
/// colour, its path, and how many repos and open merge requests it has.
fn follow_row(p: &GitlabPlugin, i: usize, now: i64) -> v::ListRow {
    let source = &p.cfg.sources[i];
    let look = format!("tenant.{} {}", source.color, p.cfg.style_for("tenant"));
    let mut text = v::Text::plain("   ")
        .push(v::Seg::icon(ui::icon::Icon::Group))
        .push(v::Seg::styled(ui::pad(&source.code, CODE_W + 1), look))
        .push(v::Seg::new(p.cfg.group_path(source), v::Ink::Emphasis).strong());
    let open = p.tenancy_counts().get(i).copied().unwrap_or(0);
    let mrs = format!("{open} open MR{}", if open == 1 { "" } else { "s" });
    let counts = if kind_of(p, source) == "a single repo" {
        format!("a single repo · {mrs}")
    } else {
        let repos = repos_of(p, i).len();
        format!("{repos} repo{} · {mrs}", if repos == 1 { "" } else { "s" })
    };
    text.add(v::Seg::new(format!("   {counts}"), v::Ink::Muted));
    let mut row = v::ListRow::new("", text);
    row.right = check_of(p, &format!("group:{}", p.cfg.group_path(source)), now).mark_text();
    row
}

/// A token variable found, as the question the connection asks.
fn found(var: &str) -> connection::Found {
    connection::Found {
        what: "a GitLab token".into(),
        names: vec![(var.to_string(), "the token".into())],
        file: None,
        does: format!("sets the token to env:{var} and checks it with GitLab"),
        note: None,
    }
}

/// One row of the tenancies section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceRow {
    /// `+ add a tenancy`: always first, so adding is always in sight.
    Add,
    Tenancy(usize),
    Repo {
        source: usize,
        rel: String,
    },
}

/// The views section's first row is `+ new view`; the views follow it.
/// The view the selected row is, or `None` on the add row.
fn view_index(p: &GitlabPlugin) -> Option<usize> {
    p.section_rows[2]
        .checked_sub(1)
        .filter(|i| *i < p.cfg.views.len())
}

/// The views section's row for view `i`.
const fn view_row(i: usize) -> usize {
    i + 1
}

/// The look section's groups: each word, and what it is.
const LOOK: [(&str, &[(&str, &str)]); 4] = [
    (
        "merge request states",
        &[
            ("review", "a merge request open for your review"),
            (
                "fix",
                "yours, and what is in its way is yours to do: a rebase, conflicts, a red pipeline, open threads",
            ),
            ("ready", "yours, and ready to merge: nothing in its way"),
            (
                "waiting",
                "yours, waiting on approvals or a running pipeline",
            ),
            ("draft", "your draft"),
            ("you", "the reader, in place of their own name"),
            ("team", "a teammate's merge request, not asking you"),
            ("bot", "a merge request from a bot"),
            ("merged", "a merge request that was merged"),
            ("closed", "a merge request closed without merging"),
            ("label", "a merge request's labels"),
        ],
    ),
    (
        "pipelines",
        &[
            ("passed", "a pipeline that passed"),
            ("failed", "a pipeline that failed"),
            ("running", "a pipeline that is running"),
            ("manual", "a pipeline waiting for a manual job"),
            ("pending", "a pipeline that has not started"),
            ("canceled", "a pipeline that was canceled"),
            ("skipped", "a pipeline that was skipped"),
        ],
    ),
    (
        "tenancies",
        &[
            (
                "tenant",
                "a tenancy's code; its colour is the tenancy's own",
            ),
            ("shown", "a repo shown in merge requests"),
            ("hidden", "a repo you have hidden"),
        ],
    ),
    (
        "repositories tree",
        &[
            (
                "band",
                "the shade behind every other repo and its merge requests",
            ),
            ("heading", "the shade behind a tenancy's heading"),
            ("lines", "the tree lines joining repos and merge requests"),
        ],
    ),
];

/// Look words that are a background: only their colour means anything.
fn is_background(word: &str) -> bool {
    matches!(word, "band" | "heading")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LookRow {
    Header(&'static str),
    Word(&'static str, &'static str),
}

fn look_rows() -> Vec<LookRow> {
    let mut rows = Vec::new();
    for (group, words) in LOOK {
        rows.push(LookRow::Header(group));
        rows.extend(words.iter().map(|(w, d)| LookRow::Word(w, d)));
    }
    rows
}

const fn index(section: &str) -> usize {
    match section.as_bytes() {
        b"connection" => 0,
        b"tenancies" => 1,
        b"views" => 2,
        _ => 3,
    }
}

/// The tenancies section's rows, in order: `+ add`, then each tenancy and,
/// unless it is folded, its repos.
pub fn source_rows(p: &GitlabPlugin) -> Vec<SourceRow> {
    let mut rows = vec![SourceRow::Add];
    for i in 0..p.cfg.sources.len() {
        rows.push(SourceRow::Tenancy(i));
        if !p.folded.contains(&p.cfg.sources[i].group) {
            rows.extend(
                repos_of(p, i)
                    .into_iter()
                    .map(|rel| SourceRow::Repo { source: i, rel }),
            );
        }
    }
    rows
}

/// A row of the tenancies tree by its ids, as the tree knows it.
fn source_path(row: &SourceRow) -> Vec<String> {
    match row {
        SourceRow::Add => vec!["add".into()],
        SourceRow::Tenancy(i) => vec![format!("t{i}")],
        SourceRow::Repo { source, rel } => vec![format!("t{source}"), rel.clone()],
    }
}

/// Draw the tenancies section as a banded tree: `+ add`, then each tenancy
/// with its stripe, open or folded (`⋯`), with its repos. The word at a
/// row's right (shown, hidden, the check) is a column. Returns where each
/// row of [`source_rows`] landed, for clicks; a text input being typed goes
/// on its row through the tree.
/// The tenancy label's column in the tenancies tree.
const CODE_W: usize = 5;
/// Where an add row's input starts in a list: the margin, then ` + `.
pub(crate) const ADD_AT: u16 = 4;

/// A tenancy's repos (relative paths), as GitLab last reported them, plus any
/// hidden ones GitLab did not report.
fn repos_of(p: &GitlabPlugin, i: usize) -> Vec<String> {
    let source = &p.cfg.sources[i];
    let group = p.cfg.group_path(source);
    let prefix = format!("{group}/");
    let mut rels: Vec<String> = p
        .data
        .get(&group)
        .map(|d| {
            d.projects
                .iter()
                .filter(|pr| {
                    p.cfg
                        .source_for(&pr.full_path)
                        .is_some_and(|(s, _)| s.group == source.group)
                })
                .filter_map(|pr| pr.full_path.strip_prefix(&prefix).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    for h in &source.hide {
        if !rels.contains(h) {
            rels.push(h.clone());
        }
    }
    rels.sort();
    rels
}

fn row_count(p: &GitlabPlugin, section: &str) -> usize {
    match section {
        "connection" => conn_rows(p).len(),
        "tenancies" => source_rows(p).len(),
        "views" => p.cfg.views.len() + 1,
        _ => look_rows().len(),
    }
}

fn conn_row(p: &GitlabPlugin) -> ConnRow {
    let rows = conn_rows(p);
    rows[p.section_rows[0].min(rows.len() - 1)]
}

/// Capture only rows which survive a source sync. Loaded changes offer/repo
/// rows, but never configuration fields or the tenancy list itself. This keeps
/// the host's existing x-menu key replay on the same surviving target. It does
/// not fence actions for disappearing offer/repo rows; those remain unchanged.
pub(crate) struct StableSelection {
    field: Option<Field>,
    tenancy: Option<usize>,
}

impl StableSelection {
    pub(crate) fn capture(p: &GitlabPlugin) -> Self {
        Self {
            field: match conn_row(p) {
                ConnRow::Field(f) => Some(f),
                _ => None,
            },
            tenancy: match source_rows(p).get(p.section_rows[1]) {
                Some(SourceRow::Tenancy(i)) => Some(*i),
                _ => None,
            },
        }
    }

    pub(crate) fn restore(self, p: &mut GitlabPlugin) {
        if let Some(field) = self.field
            && let Some(row) = conn_rows(p)
                .iter()
                .position(|r| *r == ConnRow::Field(field))
        {
            p.section_rows[0] = row;
        }
        if let Some(i) = self.tenancy
            && let Some(row) = source_rows(p)
                .iter()
                .position(|r| *r == SourceRow::Tenancy(i))
        {
            p.section_rows[1] = row;
        }
    }
}

/// Use the token variable offer `k` names, and check it.
fn use_offer(p: &mut GitlabPlugin, k: usize) -> Vec<Effect> {
    let Some(var) = p.offers().get(k).cloned() else {
        return vec![];
    };
    p.replace_connection(p.cfg.url.clone(), format!("env:{var}"));
    p.checks.remove("auth");
    p.data.clear();
    crate::runs_ownership::reconcile(p);
    p.section_rows[0] = conn_rows(p)
        .iter()
        .position(|r| *r == ConnRow::Field(Field::Token))
        .unwrap_or(0);
    let mut fx = vec![Effect::Notify(format!("the token reads env:{var}"))];
    fx.extend(check(p, CheckTarget::Auth));
    fx.extend(p.refresh());
    fx
}

/// What an access choice lets the token do, as its row says it: what it
/// reads or adds, then the scope a new token asks for.
fn access_about(access: config::TokenAccess) -> &'static str {
    match access {
        config::TokenAccess::Read => "merge requests, pipelines, diffs",
        config::TokenAccess::Write => "adds comment, approve, merge, rebase",
    }
}

/// The access row's detail: what the token may do, the scope it asks for,
/// and for write, that each write is still asked and the mode must allow it.
fn access_detail(access: config::TokenAccess) -> String {
    match access {
        config::TokenAccess::Read => {
            "The token reads merge requests, pipelines, threads and diffs. The new-token link asks for read_api.".into()
        }
        config::TokenAccess::Write => format!(
            "The token may also comment, reply, approve, merge and rebase. The new-token link asks for api. Each write is still asked first, and only while the mode allows writes; the mode is {} now.",
            radar_core::writes::mode().0.word()
        ),
    }
}

/// An access choice taken: kept in the config, and a check already made
/// asked again, so the check row says whether the token allows it.
fn choose_access(p: &mut GitlabPlugin, access: config::TokenAccess) -> Vec<Effect> {
    if p.cfg.access == access {
        return vec![];
    }
    p.cfg.access = access;
    let mut fx = vec![Effect::Notify(format!("the token's access: {}", access.label()))];
    if p.checks.contains_key("auth") {
        fx.extend(check(p, CheckTarget::Auth));
    }
    fx
}

/// The check row: the connection, and the base group when there is one.
fn check_connection(p: &mut GitlabPlugin) -> Vec<Effect> {
    let mut fx = check(p, CheckTarget::Auth);
    if !p.cfg.base.is_empty() {
        fx.extend(check(p, CheckTarget::Base));
    }
    fx
}

fn selected_look(p: &GitlabPlugin) -> Option<(&'static str, &'static str)> {
    match look_rows().get(p.section_rows[3]) {
        Some(LookRow::Word(w, d)) => Some((w, d)),
        _ => None,
    }
}

fn field_value(p: &GitlabPlugin, f: Field) -> String {
    match f {
        Field::Url => p.cfg.url.clone(),
        Field::Token => masked(&p.cfg.token),
        Field::Base => {
            if p.cfg.base.is_empty() {
                "—".into()
            } else {
                p.cfg.base.clone()
            }
        }
        Field::Refresh => connection::every(p.cfg.refresh_secs),
        Field::History => format!("the last {} days", p.cfg.history_days),
        Field::SignedIn => {
            if p.me.is_empty() {
                "—".into()
            } else {
                match my_name(p) {
                    Some(name) if name != p.me => format!("{name}  {}", p.me),
                    _ => p.me.clone(),
                }
            }
        }
        Field::Check => match p.checks.get("auth") {
            None => "not checked yet".into(),
            Some((Check::Running, _)) => "asking GitLab…".into(),
            Some((Check::Passed(m), _)) => radar_core::writes::done_line(m),
            Some((Check::Failed(e), _)) => radar_core::writes::failed_line(e),
        },
    }
}

/// The signed-in person's display name, as their own merge requests carry it.
fn my_name(p: &GitlabPlugin) -> Option<String> {
    p.data
        .values()
        .flat_map(|d| d.mrs.iter())
        .find(|m| m.author.username == p.me)
        .map(|m| m.author.name.clone())
        .filter(|n| !n.is_empty())
}

/// A token is shown by name (`env:NAME`, `glab`); one typed in never whole.
fn masked(token: &str) -> String {
    connection::masked(token, &["glab"])
}

// --- checks ----------------------------------------------------------------

/// Begin ownership before any asynchronous setup, including token resolution.
fn check(p: &mut GitlabPlugin, target: CheckTarget) -> Vec<Effect> {
    let request = settings_checks::begin(p, &target);
    let key = target.key();
    p.checks.insert(key.clone(), (Check::Running, p.now));
    let (url, token, access) = (p.cfg.url.clone(), p.cfg.token.clone(), p.cfg.access);
    let kind = target.kind();
    let path = match &target {
        CheckTarget::Auth => String::new(),
        CheckTarget::Base => p.cfg.base.clone(),
        // Preserve the legacy repeated-prefix handling for malformed hand config.
        CheckTarget::Source(path) => path.trim_start_matches("group:").to_owned(),
    };
    let action = crate::settings_history::action(kind, &url, &path);
    vec![crate::settings_history::spawn(
        action,
        target,
        request,
        async move {
            let attempt = crate::settings_history::Attempt::default();
            let result = crate::auth::run(&url, &token, |client: Client| {
                attempt.enter();
                let path = path.clone();
                async move {
                    match kind {
                        crate::settings_history::Kind::Auth => {
                            let mut got = client.check_user_observed().await?;
                            // Reading is proven by the answer; writing needs
                            // the token's scopes, asked only when chosen.
                            if access == config::TokenAccess::Write {
                                let scopes = client.token_scopes().await;
                                if let Some(note) = crate::setup::scope_note(access, scopes.as_deref()) {
                                    got.value.push_str(&note);
                                }
                            }
                            Ok(got)
                        }
                        _ => client.check_path_observed(&path).await,
                    }
                }
            })
            .await;
            let result = attempt.finish(result);
            match &result {
                Ok(got) => radar_core::log::info(format!("gitlab check {key}: {}", got.value)),
                Err(error) => {
                    radar_core::log::warn(format!("gitlab check {key} failed: {}", error.message))
                }
            }
            result
        },
    )]
}

// --- bulk --------------------------------------------------------------------

/// Bulk commands (`b`) for a section: done to every row at once.
pub fn bulk(p: &GitlabPlugin, section: &str) -> Vec<MenuItem> {
    let n = p.cfg.sources.len();
    match section {
        "connection" => vec![MenuItem::new(
            'c',
            "check everything",
            "the token, the base group and every tenancy",
            "check-all",
        )],
        "tenancies" if n > 0 => {
            let mut out = vec![MenuItem::new(
                'c',
                "check every tenancy",
                format!("{n} with GitLab"),
                "check-groups",
            )];
            let hidden: usize = p.cfg.sources.iter().map(|s| s.hide.len()).sum();
            if hidden > 0 {
                out.push(MenuItem::new(
                    's',
                    "show every hidden repo",
                    format!("{hidden} come back"),
                    "unhide",
                ));
            }
            out.extend([
                MenuItem::new('f', "fold every tenancy", "just the tenancies", "fold"),
                MenuItem::new('u', "unfold every tenancy", "show their repos", "unfold"),
            ]);
            out
        }
        "look" => {
            let changed = default_styles()
                .iter()
                .filter(|(w, d)| p.cfg.style_for(w) != **d)
                .count();
            if changed == 0 {
                return vec![];
            }
            vec![MenuItem::new(
                'r',
                "every word back to its default",
                format!("{changed} changed"),
                "reset-all",
            )]
        }
        _ => vec![],
    }
}

/// Run the bulk command `value`.
pub fn on_bulk(p: &mut GitlabPlugin, value: &str) -> Vec<Effect> {
    let groups: Vec<CheckTarget> = p
        .cfg
        .sources
        .iter()
        .map(|s| CheckTarget::Source(p.cfg.group_path(s)))
        .collect();
    match value {
        "check-all" => {
            let mut fx = check(p, CheckTarget::Auth);
            if !p.cfg.base.trim().is_empty() {
                fx.extend(check(p, CheckTarget::Base));
            }
            for g in &groups {
                fx.extend(check(p, g.clone()));
            }
            fx
        }
        "check-groups" => groups.into_iter().flat_map(|g| check(p, g)).collect(),
        "unhide" => {
            for s in &mut p.cfg.sources {
                s.hide.clear();
            }
            vec![Effect::Notify("every repo shown".into())]
        }
        "fold" => {
            p.folded = p.cfg.sources.iter().map(|s| s.group.clone()).collect();
            vec![]
        }
        "unfold" => {
            p.folded.clear();
            vec![]
        }
        "reset-all" => {
            p.cfg.style = default_styles();
            vec![Effect::Notify("every word back to its default".into())]
        }
        _ => vec![],
    }
}

fn ask_to_check(target: CheckTarget, what: &str) -> Effect {
    Effect::Menu(Menu::confirm(
        target.menu(),
        format!("check {what} now?"),
        "check it",
        "leave it unchecked",
    ))
}

/// Where the check `key` stands, as the kit shows a check.
fn check_of(p: &GitlabPlugin, key: &str, now: i64) -> ui::status::Check {
    use ui::status::Check as Shown;
    let ago = |at: &i64| Some(time::ago(*at, now));
    match p.checks.get(key) {
        None => Shown::NotYet,
        Some((Check::Running, _)) => Shown::Running,
        Some((Check::Passed(m), at)) => Shown::Passed {
            said: m.clone(),
            ago: ago(at),
        },
        Some((Check::Failed(e), at)) => Shown::Failed {
            said: e.clone(),
            ago: ago(at),
        },
    }
}

// --- live validation ---------------------------------------------------------

/// What a tenancy path would track. A path does not say whether it is a
/// group or a repo, unless radar has seen it: GitLab tells on the check.
fn will_track(p: &GitlabPlugin, rel: &str) -> String {
    let full = config::join_path(&p.cfg.base, rel);
    let seen = p
        .data
        .values()
        .flat_map(|d| d.projects.iter())
        .any(|pr| pr.full_path == full);
    if seen {
        format!("will track the repo {full} on its own")
    } else {
        format!("will track {full}: a group, or a single repo; the check tells which")
    }
}

/// What a tenancy is, as far as radar knows: from its last sync, else its check.
fn kind_of(p: &GitlabPlugin, s: &Source) -> &'static str {
    let full = p.cfg.group_path(s);
    match p.data.get(&full).map(|d| d.repo) {
        Some(true) => "a single repo",
        Some(false) => "a group",
        None => match p.checks.get(&format!("group:{full}")) {
            Some((Check::Passed(m), _)) if m.starts_with("found the repo") => "a single repo",
            Some((Check::Passed(_), _)) => "a group",
            _ => "not known until it is checked or synced",
        },
    }
}

/// Whether typed text would be accepted, and why not.
fn validate(p: &GitlabPlugin, target: &Target, text: &str) -> Result<String, String> {
    let text = text.trim();
    match target {
        Target::Conn(Field::Url) => {
            if text.starts_with("https://") || text.starts_with("http://") {
                Ok("looks like a server address".into())
            } else {
                Err("starts with https://".into())
            }
        }
        Target::Conn(Field::Token) => match text {
            "" => Err("needs env:NAME, glab, or the token".into()),
            "glab" => Ok("reuses glab's login".into()),
            t if t.starts_with("env:") => {
                let var = &t[4..];
                if std::env::var(var).is_ok() {
                    Ok(format!("{var} is set"))
                } else {
                    Err(format!("{var} is not set in this shell"))
                }
            }
            t if t.len() < 20 => Err("too short for a GitLab token".into()),
            _ => Ok("a token typed in; it is stored in config.toml".into()),
        },
        Target::Conn(Field::Base) => {
            let group = config::group_path_of(text);
            if group.is_empty() {
                Ok("no base: tenancies are full paths".into())
            } else if let Some(why) = config::group_path_problem(&group) {
                Err(why)
            } else {
                Ok(format!("base group {group}"))
            }
        }
        Target::Conn(Field::Refresh) => match text.trim_end_matches('s').parse::<u64>() {
            Ok(n) if n >= 10 => Ok(connection::every(n)),
            Ok(_) => Err("at least 10 seconds".into()),
            Err(_) => Err("whole seconds, e.g. 60".into()),
        },
        Target::Conn(Field::History) => match text.trim_end_matches('d').trim().parse::<u64>() {
            Ok(n) if (1..=90).contains(&n) => Ok(format!(
                "merged and closed ones from the last {n} day{}",
                if n == 1 { "" } else { "s" }
            )),
            Ok(_) => Err("1 to 90 days".into()),
            Err(_) => Err("whole days, e.g. 14".into()),
        },
        Target::SourceCode(_) => {
            if (1..=4).contains(&text.chars().count())
                && text.chars().all(|c| c.is_ascii_alphanumeric())
            {
                Ok(format!(
                    "{text} will show beside this tenancy's merge requests"
                ))
            } else {
                Err("1 to 4 letters or digits, e.g. cm".into())
            }
        }
        Target::SourceGroup(i) => {
            let group = config::relative_group(text, &p.cfg.base);
            if let Some(why) = config::group_path_problem(&group) {
                Err(why)
            } else if p
                .cfg
                .sources
                .iter()
                .enumerate()
                .any(|(j, s)| j != *i && s.group == group)
            {
                Err("another tenancy already tracks it".into())
            } else {
                Ok(will_track(p, &group))
            }
        }
        Target::SourceAdd => {
            let group = config::relative_group(text, &p.cfg.base);
            if let Some(why) = config::group_path_problem(&group) {
                Err(why)
            } else if p.cfg.sources.iter().any(|s| s.group == group) {
                Err("already a tenancy".into())
            } else {
                Ok(will_track(p, &group))
            }
        }
        Target::ViewFilter(_) => {
            let problems = filter::problems(text);
            if problems.is_empty() {
                Ok(filter::describe(text))
            } else {
                Err(problems.join("; "))
            }
        }
        Target::ViewName(_) | Target::ViewAdd => {
            if text.is_empty() {
                Err("needs a name".into())
            } else if p.cfg.views.iter().any(|v| v.name == text) {
                Err("another view has that name".into())
            } else {
                Ok("a view name".into())
            }
        }
        Target::ViewWords(_) => {
            if text.is_empty() {
                Err("type words to look for".into())
            } else {
                Ok(format!("mentioning \"{text}\""))
            }
        }
        Target::Conn(Field::SignedIn | Field::Check) => Err("found by checking".into()),
    }
}

/// `warning · bold` for a style spec, for people.
fn look_summary(spec: &str, word: &str) -> String {
    let mut parts = spec.split_whitespace();
    let colour = if word == "tenant" {
        widgets::own_colour("tenancy")
    } else {
        parts.next().map(|s| slot_info(s).label).unwrap_or_default()
    };
    let mods: Vec<&str> = if word == "tenant" {
        spec.split_whitespace().collect()
    } else {
        parts.collect()
    };
    if mods.is_empty() {
        colour
    } else {
        format!("{colour} · {}", mods.join(" "))
    }
}

pub fn look_view(p: &GitlabPlugin) -> v::SectionView {
    let rows = look_rows().into_iter().enumerate().map(|(i, row)| match row {
        LookRow::Header(group) => v::ListRow::heading(format!("row:{i}"), v::kit::heading(group)),
        LookRow::Word(word, about) => {
            let spec = p.cfg.style_for(word);
            let sample = if word == "tenant" { "code" } else { word };
            let look = if word == "tenant" { format!("tenant.cyan {spec}") } else { spec.clone() };
            let mut row = v::ListRow::new(format!("row:{i}"), v::Text::new()
                .push(v::Seg::new("   ", v::Ink::Plain))
                .push(v::Seg::styled(format!("{sample:<9}"), look)));
            row.detail = Some(v::Text::of(about, v::Ink::Muted));
            row.right = Some(v::Text::of(look_summary(&spec, word), v::Ink::Secondary));
            row
        }
    }).collect();
    let mut detail = Vec::new();
    if let Some((word, about)) = selected_look(p) {
        let spec = p.cfg.style_for(word);
        detail.extend([v::Text::plain(format!("How {about} is drawn.")), v::Text::new(),
            v::Text::new().push(v::Seg::new("  sample  ", v::Ink::Muted))
                .push(v::Seg::styled(if word == "tenant" { "cm" } else { word },
                    if word == "tenant" { format!("tenant.cyan {spec}") } else { spec.clone() })),
            v::Text::new()]);
        if word != "tenant" {
            let colour = spec.split_whitespace().next().unwrap_or("text.primary");
            let info = slot_info(colour);
            detail.push(v::kit::labelled("colour", 10, v::Text::new()
                .push(v::Seg::styled("██ ", colour)).push(v::Seg::new(info.label.clone(), v::Ink::Primary))));
            detail.push(v::Text::of(format!("          the theme's {} colour, used for {}", info.label, info.meaning), v::Ink::Muted));
        }
        if !is_background(word) {
            let mods: Vec<_> = spec.split_whitespace().skip(usize::from(word != "tenant")).collect();
            detail.push(v::kit::labelled("emphasis", 10, v::Text::plain(if mods.is_empty() { "none".into() } else { mods.join(", ") })));
        }
        if default_styles().get(word).is_some_and(|default| *default != spec) {
            detail.push(v::kit::labelled("default", 10, v::Text::of(look_summary(&default_styles()[word], word), v::Ink::Muted)));
        }
        detail.extend([v::Text::new(), v::Text::of("What each colour looks like is set in colours, under general.", v::Ink::Muted),
            v::Text::of(format!("config: [gitlab.style] {word}"), v::Ink::Muted)]);
    }
    v::SectionView {
        version: v::VERSION, scope: "look".into(),
        body: v::Node::List { rows, selected: Some(p.section_rows[3]), editing: None },
        detail_title: detail_title(p, "look"), detail,
        detail_blocks: vec![],
        actions: action_specs(p, "look"),
    }
}

pub fn views_view(p: &GitlabPlugin, width: u16) -> v::SectionView {
    let mut rows = vec![v::ListRow::new("row:0", v::kit::add_line(
        "new view", "starts from everything; add conditions after", width,
    ))];
    for (i, item) in p.cfg.views.iter().enumerate() {
        let mut row = v::ListRow::new(format!("row:{}", view_row(i)), v::Text::new()
            .push(v::Seg::new(format!(" {:<13} ", item.name), v::Ink::Emphasis).strong())
            .push(v::Seg::new(filter::describe(&item.filter), v::Ink::Secondary)));
        row.right = Some(v::Text::of(p.count(&item.filter).to_string(), v::Ink::Number));
        rows.push(row);
    }
    let editing = p.edit.as_ref().and_then(|(target, input)| {
        let (row, at) = match target {
            Target::ViewAdd => (0, ADD_AT),
            Target::ViewName(i) => (view_row(*i), 2),
            Target::ViewFilter(i) | Target::ViewWords(i) => (view_row(*i), 3 + p.cfg.views[*i].name.chars().count().max(13) as u16),
            _ => return None,
        };
        Some(v::Editing { row, at, text: input.value(), caret: input.cursor() })
    });
    let mut detail = Vec::new();
    if let Some((target, input)) = &p.edit {
        detail.extend([v::kit::heading("as you type"),
            v::kit::as_you_type(validate(p, target, &input.value()).as_deref().map_err(String::as_str)),
            v::Text::of("⏎ keeps it · esc cancels", v::Ink::Muted), v::Text::new()]);
    }
    if let Some(i) = view_index(p) {
        let item = &p.cfg.views[i];
        detail.extend([v::Text::plain("A view is a saved filter; it is listed in the merge requests nav pane."),
            v::Text::new(), v::kit::heading("shows merge requests")]);
        if item.filter.trim().is_empty() { detail.push(v::Text::of("  • all of them", v::Ink::Secondary)); }
        detail.extend(item.filter.split_whitespace().map(|term| v::Text::of(format!("  • {}", filter::describe_term(term)), v::Ink::Secondary)));
        detail.extend([v::Text::new(),
            v::kit::labelled("matches", 10, v::Text::of(format!("{} now", p.count(&item.filter)), v::Ink::Number)),
            v::kit::labelled("as text", 10, v::Text::of(if item.filter.is_empty() { "—".into() } else { item.filter.clone() }, v::Ink::Muted))]);
        detail.extend(filter::problems(&item.filter).into_iter().map(|problem| v::Text::of(radar_core::writes::failed_line(&problem), v::Ink::Warn)));
        if let Some((Target::ViewFilter(_), input)) = &p.edit {
            detail.push(v::Text::new());
            detail.push(v::kit::labelled("would match", 12, v::Text::of(format!("{} now", p.count(&input.value())), v::Ink::Number)));
        }
    } else {
        detail.extend([v::Text::plain("Make another view for the merge requests nav pane."), v::Text::new(),
            v::Text::of("Give it a name; it starts by showing every open merge request. Then add conditions to it: by state, who opened it, tenancy, repo, label, merged or closed, or words.", v::Ink::Secondary)]);
    }
    v::SectionView {
        version: v::VERSION, scope: "views".into(),
        body: v::Node::List { rows, selected: Some(p.section_rows[2]), editing },
        detail_title: detail_title(p, "views"), detail,
        detail_blocks: vec![],
        actions: action_specs(p, "views"),
    }
}

pub fn connection_view(p: &GitlabPlugin, width: u16, now: i64) -> v::SectionView {
    let offers = p.offers();
    let mut editing_at = None;
    let rows = conn_rows(p).into_iter().enumerate().map(|(i, row)| match row {
        ConnRow::Offer(k) => v::ListRow::new(format!("row:{i}"), found(&offers[k]).text(width)),
        ConnRow::Gap => v::ListRow::heading(format!("row:{i}"), v::Text::new()),
        ConnRow::AccessHeading => v::ListRow::heading(format!("row:{i}"), v::Text::plain(" ")
            .push(v::Seg::new("ACCESS", v::Ink::Muted).strong())
            .push(v::Seg::new("  what radar may do in GitLab", v::Ink::Muted))),
        ConnRow::Access(a) => {
            use radar_core::ui::icon::Icon;
            let mark = if a == p.cfg.access {
                v::Seg::icon_or_lead(Icon::Done, "[x] ", v::Ink::Good)
            } else {
                v::Seg::icon_or_lead(Icon::Todo, "[ ] ", v::Ink::Muted)
            };
            v::ListRow::new(format!("row:{i}"), v::Text::plain(" ")
                .push(mark)
                .push(v::Seg::new(radar_core::ui::pad(a.label(), 16), v::Ink::Primary))
                .push(v::Seg::new(radar_core::ui::pad(access_about(a), 38), v::Ink::Muted))
                .push(v::Seg::new(a.scope(), v::Ink::Secondary)))
        }
        ConnRow::FollowHeading => v::ListRow::heading(format!("row:{i}"), v::Text::plain(" ")
            .push(v::Seg::new("FOLLOW", v::Ink::Muted).strong())
            .push(v::Seg::new("  groups and repositories", v::Ink::Muted))),
        ConnRow::FollowAdd => {
            if matches!(p.edit, Some((Target::SourceAdd, _))) {
                editing_at = Some((i, ADD_AT));
            }
            v::ListRow::new(format!("row:{i}"), v::kit::add_line(
                "add a group or repository", "paste a link or a path", width,
            ))
        }
        ConnRow::Follow(t) => {
            let mut row = follow_row(p, t, now);
            row.target = v::Target::new(format!("row:{i}"));
            row
        }
        ConnRow::Field(field) => {
            if p.edit.as_ref().is_some_and(|(target, _)| *target == Target::Conn(field)) {
                editing_at = Some((i, 1 + connection::value_x(1)));
            }
            let lead = v::Text::plain(" ");
            let value = if field == Field::Check {
                connection::check_field_text(lead, &check_of(p, "auth", now), "GitLab")
            } else {
                let text = if field == Field::SignedIn && !p.me.is_empty() {
                    connection::signed_in_text(&my_name(p).unwrap_or_default(), &p.me)
                } else {
                    v::Text::of(field_value(p, field), v::Ink::Primary)
                };
                connection::field_text(lead, field.label(), text)
            };
            let mut row = v::ListRow::new(format!("row:{i}"), value);
            if field == Field::Base { row.right = check_of(p, "base", now).mark_text(); }
            row
        }
    }).collect();
    let editing = editing_at.and_then(|(row, at)| p.edit.as_ref().map(|(_, input)| v::Editing {
        row, at, text: input.value(), caret: input.cursor(),
    }));
    let typing = || p.edit.as_ref().map(|(target, input)| validate(p, target, &input.value()));
    let row = conn_row(p);
    let detail = match row {
        ConnRow::Offer(k) => offers.get(k).map_or_else(Vec::new, |offer| found(offer).detail_text()),
        ConnRow::Field(field) => {
            let mut d = connection::DetailText::new(field.about());
            d.accepts = field.accepts();
            if field != Field::Check { d.now = Some(field_value(p, field)); }
            d.checked = field.check().map(|key| check_of(p, key, now).text("not checked yet", "checking with GitLab…"));
            d.typing = typing();
            if !field.key().is_empty() { d.config = Some(format!("[gitlab] {}", field.key())); }
            d.text()
        }
        ConnRow::Access(a) => {
            let mut d = connection::DetailText::new(access_detail(a));
            d.accepts = Some("read or write".into());
            d.now = Some(p.cfg.access.label().into());
            d.checked = Some(check_of(p, "auth", now).text("not checked yet", "checking with GitLab…"));
            d.config = Some("[gitlab] access".into());
            d.text()
        }
        ConnRow::Gap | ConnRow::AccessHeading | ConnRow::FollowHeading | ConnRow::FollowAdd => {
            let mut d = Vec::new();
            if let Some(verdict) = typing() {
                d.extend([v::kit::heading("as you type"), v::kit::as_you_type(verdict.as_deref().map_err(String::as_str)),
                    v::Text::of("⏎ keeps it · esc cancels", v::Ink::Muted), v::Text::new()]);
            }
            d.push(v::Text::plain("Follow a GitLab group (every repo in it), or one repo on its own."));
            d
        }
        ConnRow::Follow(t) => {
            let source = &p.cfg.sources[t];
            let full = p.cfg.group_path(source);
            vec![
                check_of(p, &format!("group:{full}"), now).text("not checked yet", "checking with GitLab…"),
                v::Text::new(),
                v::kit::labelled("path", 10, v::Text::of(full, v::Ink::Emphasis)),
                v::kit::labelled("is", 10, v::Text::of(kind_of(p, source), v::Ink::Secondary)),
                v::kit::labelled("open MRs", 10, v::Text::of(p.tenancy_counts().get(t).copied().unwrap_or(0).to_string(), v::Ink::Number)),
            ]
        }
    };
    // The sheet: the row's detail, then the help that row needs.
    let help = match row {
        ConnRow::Offer(_) | ConnRow::Access(_) | ConnRow::AccessHeading
        | ConnRow::Field(Field::Url | Field::Token | Field::SignedIn | Field::Check) => crate::setup::token_help(p),
        ConnRow::Field(Field::Base) | ConnRow::Gap | ConnRow::FollowHeading | ConnRow::FollowAdd | ConnRow::Follow(_) => {
            crate::setup::follow_help(p)
        }
        ConnRow::Field(Field::Refresh | Field::History) => vec![],
    };
    let mut detail_blocks: Vec<v::Block> = detail.iter().cloned().map(v::Block::line).collect();
    if !help.is_empty() {
        detail_blocks.push(v::Block::line(v::Text::new()));
        detail_blocks.extend(help);
    }
    v::SectionView {
        version: v::VERSION, scope: "connection".into(),
        body: v::Node::List { rows, selected: Some(p.section_rows[0]), editing },
        detail_title: detail_title(p, "connection"), detail,
        detail_blocks,
        actions: action_specs(p, "connection"),
    }
}

pub fn tenancies_view(p: &GitlabPlugin, width: u16, now: i64) -> v::SectionView {
    let mut rows = vec![v::TreeRow::new("add", v::kit::add_line(
        "add a tenancy", "paste a GitLab group or repo link, or its path", width,
    ))];
    for (i, source) in p.cfg.sources.iter().enumerate() {
        let repos = repos_of(p, i);
        let look = format!("tenant.{} {}", source.color, p.cfg.style_for("tenant"));
        let counts = if kind_of(p, source) == "a single repo" { "   a single repo".into() }
            else { format!("   {} repos · {} hidden", repos.len(), source.hide.len()) };
        let mut row = v::TreeRow::new(format!("t{i}"), v::Text::new()
            .push(v::Seg::styled("● ", look.clone()))
            .push(v::Seg::styled(format!("{:<CODE_W$}", source.code), look))
            .push(v::Seg::new(source.group.clone(), v::Ink::Emphasis).strong()))
            .detail(v::Text::of(counts, v::Ink::Muted))
            .stripe(format!("tenant.{}", source.color));
        if let Some(mark) = check_of(p, &format!("group:{}", p.cfg.group_path(source)), now).mark_text() {
            row = row.cell("", mark);
        }
        for rel in repos {
            let hidden = source.hide.contains(&rel);
            let word = if hidden { "hidden" } else { "shown" };
            row = row.child(v::TreeRow::new(rel.clone(), v::Text::of(rel,
                if hidden { v::Ink::Muted } else { v::Ink::Primary }))
                .cell("", v::Text::new().push(v::Seg::styled(word, p.cfg.style_for(word)))));
        }
        rows.push(row);
    }
    let ordered = source_rows(p);
    let column_width = rows.iter().flat_map(|row| std::iter::once(row).chain(row.children.iter()))
        .flat_map(|row| row.cells.iter()).map(|cell| radar_core::ui::width(&cell.text.words())).max().unwrap_or(0) as u16 + 1;
    let selected = ordered.get(p.section_rows[1]).map(source_path).unwrap_or_default();
    let editing = p.edit.as_ref().and_then(|(target, input)| {
        let (path, at) = match target {
            Target::SourceAdd => (vec!["add".into()], 3),
            Target::SourceCode(i) => (vec![format!("t{i}")], 2),
            Target::SourceGroup(i) => (vec![format!("t{i}")], 2 + p.cfg.sources[*i].code.chars().count().max(CODE_W) as u16),
            _ => return None,
        };
        Some(Box::new(v::TreeEditing { path, at, text: input.value(), caret: input.cursor() }))
    });
    let mut detail = Vec::new();
    match ordered.get(p.section_rows[1]) {
        Some(SourceRow::Tenancy(i)) => {
            let source = &p.cfg.sources[*i];
            let full = p.cfg.group_path(source);
            let kind = kind_of(p, source);
            detail.extend([v::kit::heading("checked"), check_of(p, &format!("group:{full}"), now).text("not checked yet", "checking with GitLab…"),
                v::Text::new(), v::Text::plain(if kind == "a single repo" {
                    "A tenancy of one repo: its merge requests show in your views."
                } else { "A tenancy: every repo in one GitLab group. Its merge requests show unless you hide the repo." }),
                v::Text::new(), v::kit::labelled("path", 10, v::Text::of(full, v::Ink::Emphasis)),
                v::kit::labelled("is", 10, v::Text::of(kind, v::Ink::Secondary)),
                v::kit::labelled("label", 10, v::Text::new().push(v::Seg::styled(source.code.clone(), format!("tenant.{} bold", source.color)))),
                v::kit::labelled("colour", 10, v::Text::new().push(v::Seg::styled("██ ", format!("tenant.{}", source.color)))
                    .push(v::Seg::new(source.color.clone(), v::Ink::Primary)))]);
            if kind != "a single repo" {
                detail.push(v::kit::labelled("repos", 10, v::Text::plain(format!("{} · {} hidden", repos_of(p, *i).len(), source.hide.len()))));
            }
            detail.push(v::kit::labelled("open MRs", 10, v::Text::of(p.tenancy_counts().get(*i).copied().unwrap_or(0).to_string(), v::Ink::Number)));
        }
        Some(SourceRow::Repo { source, rel }) => {
            let tenancy = &p.cfg.sources[*source];
            let full = format!("{}/{rel}", p.cfg.group_path(tenancy));
            let open = p.data.get(&p.cfg.group_path(tenancy)).map_or(0, |data| data.mrs.iter()
                .filter(|mr| mr.project_path == full && mr.state == crate::model::MrState::Opened).count());
            detail.extend([v::Text::of(full, v::Ink::Emphasis), v::Text::new(),
                v::Text::plain(if tenancy.hide.contains(rel) {
                    "Hidden: its merge requests are left out of every view. It is still tracked."
                } else { "Shown: its merge requests appear in your views." }),
                v::kit::labelled("open MRs", 10, v::Text::of(open.to_string(), v::Ink::Number))]);
        }
        Some(SourceRow::Add) | None => {
            if let Some((target, input)) = &p.edit {
                detail.extend([v::kit::heading("as you type"), v::kit::as_you_type(validate(p, target, &input.value()).as_deref().map_err(String::as_str)),
                    v::Text::of("⏎ keeps it · esc cancels", v::Ink::Muted), v::Text::new()]);
            }
            detail.extend([v::Text::plain("Track another GitLab group (every repo in it), or one repo on its own."),
                v::Text::new(), v::kit::heading("paste or type any of")]);
            detail.extend(["https://gitlab.com/<base>/tenants/<group>/", "/<part of base>/tenants/<group>/",
                "tenants/<group>", "a repo's link, or <group>/<repo>"].map(|line| v::Text::of(format!("  {line}"), v::Ink::Secondary)));
            detail.extend([v::Text::new(), v::Text::of(format!("Paths are taken under the base group{}.",
                if p.cfg.base.is_empty() { String::new() } else { format!(" {}", p.cfg.base) }), v::Ink::Muted),
                v::Text::of("radar offers to check it with GitLab once added; the check says whether it is a group or a repo.", v::Ink::Muted)]);
        }
    }
    v::SectionView {
        version: v::VERSION, scope: "tenancies".into(),
        body: v::Node::Tree(v::Tree {
            rows, selected,
            open: p.cfg.sources.iter().enumerate().filter(|(_, s)| !p.folded.contains(&s.group))
                .map(|(i, _)| vec![format!("t{i}")]).collect(),
            columns: vec![v::ColumnSpec::new("", column_width, 0)], name_width: width.saturating_sub(column_width + 2),
            look: Some(v::TreeLook { lines: p.cfg.style_for("lines"), band: String::new(), heading: String::new(), band_level: 1 }),
            editing, ..v::Tree::default()
        }),
        detail_title: detail_title(p, "tenancies"), detail,
        detail_blocks: vec![],
        actions: action_specs(p, "tenancies"),
    }
}

pub fn tenancies_intent(p: &mut GitlabPlugin, intent: &v::Intent) -> Outcome {
    let select = |p: &mut GitlabPlugin, target: &v::Target| {
        if p.edit.is_some() { return; }
        let path = v::TreeRow::path_of(target);
        if let Some(i) = source_rows(p).iter().position(|row| source_path(row) == path) { p.section_rows[1] = i; }
    };
    match intent {
        v::Intent::Select { target } | v::Intent::Context { target } => { select(p, target); Outcome::done() }
        v::Intent::Activate { target } => {
            select(p, target);
            key(p, "tenancies", radar_core::view_tui::key_of(v::Chord::new(v::Key::Enter)))
        }
        v::Intent::Fold { target, open } => {
            select(p, target);
            key(p, "tenancies", radar_core::view_tui::key_of(v::Chord::char(if *open { '+' } else { '-' })))
        }
        v::Intent::Key { .. } | v::Intent::Invoke { .. } => intent.chord().map_or(Outcome::Ignored, |chord| {
            key(p, "tenancies", radar_core::view_tui::key_of(chord))
        }),
        _ => Outcome::Ignored,
    }
}

pub fn list_intent(p: &mut GitlabPlugin, section: &str, intent: &v::Intent) -> Outcome {
    match intent {
        v::Intent::Select { target } | v::Intent::Context { target } => click(p, section, target.as_str()),
        v::Intent::Activate { target } => {
            click(p, section, target.as_str());
            key(p, section, radar_core::view_tui::key_of(v::Chord::new(v::Key::Enter)))
        }
        v::Intent::Key { .. } | v::Intent::Invoke { .. } => intent.chord().map_or(Outcome::Ignored, |chord| {
            key(p, section, radar_core::view_tui::key_of(chord))
        }),
        _ => Outcome::Ignored,
    }
}

pub fn detail_title(p: &GitlabPlugin, section: &str) -> String {
    match section {
        "connection" => match conn_row(p) {
            ConnRow::Offer(k) => p
                .offers()
                .get(k)
                .map_or_else(|| "found".into(), |v| found(v).title()),
            ConnRow::Field(f) => f.label().into(),
            ConnRow::Access(a) => a.label().into(),
            ConnRow::AccessHeading => "access".into(),
            ConnRow::Gap | ConnRow::FollowHeading | ConnRow::FollowAdd => "follow a group".into(),
            ConnRow::Follow(i) => p.cfg.sources[i].code.clone(),
        },
        "tenancies" => match source_rows(p).get(p.section_rows[1]) {
            Some(SourceRow::Add) | None => "add a tenancy".into(),
            Some(SourceRow::Tenancy(i)) => p.cfg.sources[*i].code.clone(),
            Some(SourceRow::Repo { rel, .. }) => rel.rsplit('/').next().unwrap_or(rel).to_string(),
        },
        "views" => view_index(p).map_or_else(|| "new view".into(), |i| p.cfg.views[i].name.clone()),
        _ => selected_look(p).map_or_else(|| "look".into(), |(w, _)| w.to_string()),
    }
}

// --- menus ------------------------------------------------------------------

/// The options for the selected row. Its keys also work without opening it.
fn row_menu(p: &GitlabPlugin, section: &str) -> Option<Menu> {
    match section {
        "connection" => {
            let f = match conn_row(p) {
                ConnRow::Offer(k) => {
                    let var = p.offers().get(k)?.clone();
                    let q = found(&var);
                    return Some(Menu::new(
                        format!("offer:{k}"),
                        q.question(),
                        vec![MenuItem::new('y', "yes, use it", q.does, "use")],
                    ));
                }
                ConnRow::Field(f) => f,
                ConnRow::Gap | ConnRow::AccessHeading | ConnRow::Access(_) | ConnRow::FollowHeading | ConnRow::FollowAdd => {
                    return None;
                }
                // The tenancy's own menu, for what setup needs: check it,
                // stop following it.
                ConnRow::Follow(i) => {
                    let source = p.cfg.sources.get(i)?;
                    return Some(Menu::new(format!("tenancy:{i}"), source.group.clone(), vec![
                        MenuItem::new(
                            'c',
                            "check it",
                            "ask GitLab whether it exists, and if it is a group or a repo",
                            format!("check:{}", CheckTarget::Source(p.cfg.group_path(source)).payload()),
                        ),
                        MenuItem::new('d', "stop tracking…", "remove this tenancy", "remove"),
                    ]));
                }
            };
            let mut items = Vec::new();
            if f.editable() {
                items.push(MenuItem::new(
                    'e',
                    "change it",
                    f.accepts().unwrap_or_default(),
                    "edit",
                ));
            }
            match f.check() {
                Some("base") => items.push(MenuItem::new(
                    'c',
                    "check the base group",
                    "ask GitLab whether it can see it",
                    "check",
                )),
                Some(_) => items.push(MenuItem::new(
                    'c',
                    "check the connection",
                    "ask GitLab whether it works and who the token is",
                    "check",
                )),
                None => {}
            }
            Some(Menu::new(format!("conn:{}", f.label()), f.label(), items))
        }
        "tenancies" => match source_rows(p).get(p.section_rows[1])? {
            SourceRow::Add => None,
            SourceRow::Tenancy(i) => Some(Menu::new(
                format!("tenancy:{i}"),
                p.cfg.sources[*i].group.clone(),
                vec![
                    MenuItem::new(
                        'e',
                        "edit the path",
                        "paste a new GitLab group or repo link, or path",
                        "group",
                    ),
                    MenuItem::new(
                        't',
                        "change the label",
                        format!(
                            "the short tag, now {}, beside its merge requests",
                            p.cfg.sources[*i].code
                        ),
                        "code",
                    ),
                    MenuItem::new('o', "colour…", "pick the tenancy's colour", "colour"),
                    MenuItem::new(
                        'c',
                        "check it",
                        "ask GitLab whether it exists, and if it is a group or a repo",
                        format!(
                            "check:{}",
                            CheckTarget::Source(p.cfg.group_path(&p.cfg.sources[*i])).payload()
                        ),
                    ),
                    MenuItem::new(
                        'a',
                        "add a tenancy",
                        "paste a group or repo link, or path",
                        "add",
                    ),
                ]
                .into_iter()
                // Only the moves it can make; `<` and `>` are never a screen key.
                .chain((*i > 0).then(|| MenuItem::new('<', "move up", "earlier in the tree", "up")))
                .chain(
                    (*i + 1 < p.cfg.sources.len())
                        .then(|| MenuItem::new('>', "move down", "later in the tree", "down")),
                )
                .chain([MenuItem::new(
                    'd',
                    "stop tracking…",
                    "remove this tenancy",
                    "remove",
                )])
                .collect(),
            )),
            SourceRow::Repo { source, rel } => {
                let hidden = p.cfg.sources[*source].hide.contains(rel);
                Some(Menu::new(
                    format!("repo:{source}:{rel}"),
                    rel.clone(),
                    vec![
                        if hidden {
                            MenuItem::new(' ', "show it", "its merge requests come back", "toggle")
                        } else {
                            MenuItem::new(' ', "hide it", "leave its merge requests out", "toggle")
                        },
                        MenuItem::new('a', "add a tenancy", "paste a group link or path", "add"),
                    ],
                ))
            }
        },
        "views" => {
            let i = view_index(p)?;
            let v = &p.cfg.views[i];
            let mut items = vec![MenuItem::new(
                'a',
                "add a condition…",
                "narrow what it shows",
                "cond",
            )];
            if !v.filter.trim().is_empty() {
                items.push(MenuItem::new(
                    '-',
                    "remove a condition…",
                    "widen what it shows",
                    "uncond",
                ));
            }
            items.extend([
                MenuItem::new('t', "edit as text", "type the filter yourself", "text"),
                MenuItem::new('r', "rename", "change the name in the nav pane", "rename"),
            ]);
            // Only the moves it can make.
            if i > 0 {
                items.push(MenuItem::new('<', "move up", "earlier in the nav pane", "up"));
            }
            if i + 1 < p.cfg.views.len() {
                items.push(MenuItem::new('>', "move down", "later in the nav pane", "down"));
            }
            items.extend([
                MenuItem::new('n', "new view", "start from everything", "new"),
                MenuItem::new('d', "delete…", "remove this view", "delete"),
            ]);
            Some(Menu::new(format!("view:{i}"), v.name.clone(), items))
        }
        _ => {
            let (w, _) = selected_look(p)?;
            let spec = p.cfg.style_for(w);
            let has = |m: &str| spec.split_whitespace().any(|x| x == m);
            let onoff = |m: &str| {
                if has(m) {
                    "on, turn off"
                } else {
                    "off, turn on"
                }
            };
            let mut items = Vec::new();
            if w != "tenant" {
                items.push(MenuItem::new(
                    'c',
                    "colour…",
                    "pick from the theme's colours",
                    "colour",
                ));
            }
            // A background has a colour and nothing else.
            if !is_background(w) {
                items.extend([
                    // b is bulk and l a pane everywhere, so bold is o.
                    MenuItem::new('o', "bold", onoff("bold"), "bold"),
                    MenuItem::new('i', "italic", onoff("italic"), "italic"),
                    MenuItem::new('u', "underline", onoff("underline"), "underline"),
                    MenuItem::new('d', "dim", onoff("dim"), "dim"),
                ]);
            }
            if default_styles().get(w).is_some_and(|d| *d != spec) {
                items.push(MenuItem::new(
                    'r',
                    "back to default",
                    look_summary(&default_styles()[w], w),
                    "reset",
                ));
            }
            Some(Menu::new(format!("look:{w}"), w, items))
        }
    }
}

fn colour_menu(id: String, title: &str, current: &str) -> Menu {
    let items = radar_core::theme::SLOT_GROUPS
        .iter()
        .filter(|g| !matches!(**g, "backgrounds" | "plugins"))
        .flat_map(|g| {
            radar_core::design::theme::Slot::ALL
                .iter()
                .map(|s| s.token_name())
                .filter(move |n| slot_info(n).group == *g)
        })
        .map(|name| {
            let info = slot_info(name);
            let hint = if name == current {
                format!("in use · {}", info.meaning)
            } else {
                info.meaning
            };
            MenuItem::pick(info.label, hint, name).sample(name)
        })
        .collect();
    Menu::new(id, title, items)
}

pub fn on_menu(p: &mut GitlabPlugin, menu: &str, value: &str) -> Vec<Effect> {
    let (kind, arg) = menu.split_once(':').unwrap_or((menu, ""));
    let idx = |s: &str| s.split(':').next().and_then(|n| n.parse::<usize>().ok());
    match (kind, value) {
        ("ask-check", "yes") => CheckTarget::from_payload(arg)
            .map(|target| check(p, target))
            .unwrap_or_default(),
        ("ask-check", _) => vec![],
        ("offer", "use") => idx(arg).map(|k| use_offer(p, k)).unwrap_or_default(),
        ("conn", "edit") => {
            let Some(f) = Field::ALL.into_iter().find(|f| f.label() == arg) else {
                return vec![];
            };
            let current = match f {
                Field::Url => p.cfg.url.clone(),
                // A typed token is not shown again: start from its name
                // (env:NAME, glab) or empty.
                Field::Token if p.cfg.token.starts_with("env:") || p.cfg.token == "glab" => {
                    p.cfg.token.clone()
                }
                Field::Token => String::new(),
                Field::Base => p.cfg.base.clone(),
                Field::Refresh => p.cfg.refresh_secs.to_string(),
                Field::History => p.cfg.history_days.to_string(),
                Field::SignedIn | Field::Check => return vec![],
            };
            p.edit = Some((Target::Conn(f), TextInput::new(&current)));
            vec![]
        }
        ("conn", "check") => match Field::ALL
            .into_iter()
            .find(|f| f.label() == arg)
            .and_then(Field::check)
        {
            Some("base") => check(p, CheckTarget::Base),
            Some(_) => check_connection(p),
            None => vec![],
        },
        ("tenancy", v) => {
            if let Some(payload) = v.strip_prefix("check:") {
                return CheckTarget::from_payload(payload)
                    .filter(|target| matches!(target, CheckTarget::Source(_)))
                    .map(|target| check(p, target))
                    .unwrap_or_default();
            }
            let Some(i) = idx(arg).filter(|i| *i < p.cfg.sources.len()) else {
                return vec![];
            };
            match v {
                "code" => {
                    p.edit = Some((
                        Target::SourceCode(i),
                        TextInput::new(&p.cfg.sources[i].code),
                    ))
                }
                "group" => {
                    p.edit = Some((
                        Target::SourceGroup(i),
                        TextInput::new(&p.cfg.sources[i].group),
                    ))
                }
                "colour" => {
                    let items = TENANT_COLORS
                        .iter()
                        .map(|c| {
                            let hint = if p.cfg.sources[i].color == *c {
                                "in use"
                            } else {
                                ""
                            };
                            MenuItem::pick(*c, hint, *c).sample(format!("tenant.{c}"))
                        })
                        .collect();
                    return vec![Effect::Menu(Menu::new(
                        format!("tenancy-colour:{i}"),
                        "tenancy colour",
                        items,
                    ))];
                }
                "add" => {
                    p.section_rows[1] = 0;
                    p.edit = Some((Target::SourceAdd, TextInput::new("")));
                }
                "up" | "down" => {
                    let j = if v == "up" { i.wrapping_sub(1) } else { i + 1 };
                    if j < p.cfg.sources.len() {
                        p.cfg.sources.swap(i, j);
                        // The cursor follows the tenancy.
                        if let Some(at) = source_rows(p)
                            .iter()
                            .position(|r| *r == SourceRow::Tenancy(j))
                        {
                            p.section_rows[1] = at;
                        }
                    }
                }
                "remove" => {
                    let s = &p.cfg.sources[i];
                    return vec![Effect::Menu(
                        Menu::confirm(
                            format!("tenancy-remove:{i}"),
                            format!("stop tracking {}?", s.group),
                            "stop tracking it",
                            "keep it",
                        )
                        .saying("its merge requests leave every view"),
                    )];
                }
                _ => {}
            }
            vec![]
        }
        ("tenancy-colour", c) => {
            if let Some(i) = idx(arg).filter(|i| *i < p.cfg.sources.len()) {
                p.cfg.sources[i].color = c.to_string();
            }
            vec![]
        }
        ("tenancy-remove", "yes") => {
            let Some(i) = idx(arg).filter(|i| *i < p.cfg.sources.len()) else {
                return vec![];
            };
            let removed = p.cfg.sources.remove(i);
            settings_checks::reconcile(p);
            p.data.remove(&p.cfg.group_path(&removed));
            crate::runs_ownership::reconcile(p);
            p.section_rows[1] = 0;
            vec![Effect::Notify(format!(
                "stopped tracking {}",
                removed.group
            ))]
        }
        ("repo", "toggle") => {
            let Some((s, rel)) = arg.split_once(':') else {
                return vec![];
            };
            let Ok(s) = s.parse::<usize>() else {
                return vec![];
            };
            let Some(source) = p.cfg.sources.get_mut(s) else {
                return vec![];
            };
            match source.hide.iter().position(|h| h == rel) {
                Some(pos) => {
                    source.hide.remove(pos);
                }
                None => {
                    source.hide.push(rel.to_string());
                    source.hide.sort();
                }
            }
            vec![]
        }
        ("repo", "add") => {
            p.section_rows[1] = 0;
            p.edit = Some((Target::SourceAdd, TextInput::new("")));
            vec![]
        }
        ("view", v) => {
            let Some(i) = idx(arg).filter(|i| *i < p.cfg.views.len()) else {
                return vec![];
            };
            match v {
                "cond" => {
                    return vec![Effect::Menu(Menu::new(
                        format!("cond:{i}"),
                        "add a condition",
                        vec![
                            MenuItem::new(
                                's',
                                "by state…",
                                "review, fix, ready, waiting, draft, team, bot",
                                "is",
                            ),
                            MenuItem::new('m', "opened by me", "or everyone but me", "me"),
                            MenuItem::new(
                                'o',
                                "opened by someone…",
                                "pick from people seen",
                                "author",
                            ),
                            MenuItem::new('t', "in a tenancy…", "pick a tenancy", "tenant"),
                            MenuItem::new('r', "in a repo…", "pick a repo", "repo"),
                            MenuItem::new(
                                'c',
                                "open, merged or closed…",
                                "open ones only, unless you pick",
                                "state",
                            ),
                            MenuItem::new('l', "with a label…", "pick from labels seen", "label"),
                            MenuItem::new(
                                'w',
                                "mentioning words…",
                                "in the title, branch or repo",
                                "words",
                            ),
                        ],
                    ))];
                }
                "uncond" => {
                    let items = p.cfg.views[i]
                        .filter
                        .split_whitespace()
                        .enumerate()
                        .map(|(n, term)| {
                            MenuItem::pick(filter::describe_term(term), term, n.to_string())
                        })
                        .collect();
                    return vec![Effect::Menu(Menu::new(
                        format!("cond-remove:{i}"),
                        "remove which?",
                        items,
                    ))];
                }
                "text" => {
                    p.edit = Some((
                        Target::ViewFilter(i),
                        TextInput::new(&p.cfg.views[i].filter),
                    ))
                }
                "rename" => {
                    p.edit = Some((Target::ViewName(i), TextInput::new(&p.cfg.views[i].name)))
                }
                "up" if i > 0 => {
                    p.cfg.views.swap(i, i - 1);
                    p.section_rows[2] = view_row(i - 1);
                }
                "down" if i + 1 < p.cfg.views.len() => {
                    p.cfg.views.swap(i, i + 1);
                    p.section_rows[2] = view_row(i + 1);
                }
                "new" => {
                    p.section_rows[2] = 0;
                    p.edit = Some((Target::ViewAdd, TextInput::new("")));
                }
                "delete" if p.cfg.views.len() > 1 => {
                    return vec![Effect::Menu(
                        Menu::confirm(
                            format!("view-delete:{i}"),
                            format!("delete the view {}?", p.cfg.views[i].name),
                            "delete it",
                            "keep it",
                        )
                        .saying("it leaves the nav pane"),
                    )];
                }
                "delete" => return vec![Effect::Notify("the last view stays".into())],
                _ => {}
            }
            vec![]
        }
        ("view-delete", "yes") => {
            if let Some(i) = idx(arg).filter(|i| *i < p.cfg.views.len()) {
                let v = p.cfg.views.remove(i);
                p.section_rows[2] = view_row(i.saturating_sub(1));
                return vec![Effect::Notify(format!("deleted {}", v.name))];
            }
            vec![]
        }
        ("cond", kind) => {
            let Some(i) = idx(arg) else { return vec![] };
            let items: Vec<MenuItem> = match kind {
                "is" => Role::ALL
                    .iter()
                    .map(|r| MenuItem::pick(r.word(), r.sentence(), r.word()))
                    .collect(),
                "me" => return vec![sign_menu(i, "author", "@me", "opened by you")],
                "author" => p
                    .people()
                    .into_iter()
                    .map(|(u, n)| MenuItem::pick(n, u.clone(), u))
                    .collect(),
                "tenant" => p
                    .cfg
                    .sources
                    .iter()
                    .map(|s| MenuItem::pick(s.code.clone(), s.group.clone(), s.code.clone()))
                    .collect(),
                "repo" => p
                    .repo_names()
                    .into_iter()
                    .map(|r| MenuItem::pick(r.clone(), "", r))
                    .collect(),
                "label" => p
                    .labels()
                    .into_iter()
                    .map(|l| MenuItem::pick(l.clone(), "", l))
                    .collect(),
                "state" => ["merged", "closed", "all", "open"]
                    .iter()
                    .map(|w| {
                        let hint = match *w {
                            "all" => "open, merged and closed".to_string(),
                            "open" => "only those still open (as without this)".to_string(),
                            _ => format!("from the last {} days", p.cfg.history_days),
                        };
                        MenuItem::pick(filter::describe_term(&format!("state:{w}")), hint, *w)
                    })
                    .collect(),
                "words" => {
                    p.edit = Some((Target::ViewWords(i), TextInput::new("")));
                    return vec![];
                }
                _ => return vec![],
            };
            if items.is_empty() {
                return vec![Effect::Notify(
                    "nothing to pick from yet; refresh first".into(),
                )];
            }
            vec![Effect::Menu(Menu::new(
                format!("cond-{kind}:{i}"),
                "which?",
                items,
            ))]
        }
        ("cond-state", v) => {
            if let Some(view) = idx(arg).and_then(|i| p.cfg.views.get_mut(i)) {
                view.filter = filter::add(&view.filter, false, "state", v);
            }
            vec![]
        }
        (k @ ("cond-is" | "cond-author" | "cond-tenant" | "cond-repo" | "cond-label"), v) => {
            let Some(i) = idx(arg) else { return vec![] };
            let key = &k[5..];
            vec![sign_menu(
                i,
                key,
                v,
                &filter::describe_term(&format!("{key}:{v}")),
            )]
        }
        ("cond-sign", sign) => {
            // arg = <view>:<key>:<value>
            let mut parts = arg.splitn(3, ':');
            let (Some(i), Some(key), Some(v)) = (
                parts.next().and_then(|n| n.parse::<usize>().ok()),
                parts.next(),
                parts.next(),
            ) else {
                return vec![];
            };
            if let Some(view) = p.cfg.views.get_mut(i) {
                view.filter = filter::add(&view.filter, sign == "exclude", key, v);
            }
            vec![]
        }
        ("cond-remove", n) => {
            if let (Some(i), Ok(n)) = (idx(arg), n.parse::<usize>())
                && let Some(view) = p.cfg.views.get_mut(i)
            {
                view.filter = filter::remove(&view.filter, n);
            }
            vec![]
        }
        ("look", v) => {
            let word = arg.to_string();
            let spec = p.cfg.style_for(&word);
            match v {
                "colour" => {
                    let current = spec.split_whitespace().next().unwrap_or("").to_string();
                    return vec![Effect::Menu(colour_menu(
                        format!("look-colour:{word}"),
                        "colour",
                        &current,
                    ))];
                }
                "reset" => {
                    p.cfg
                        .style
                        .insert(word.clone(), default_styles()[word.as_str()].clone());
                }
                m => {
                    let mut words: Vec<String> =
                        spec.split_whitespace().map(String::from).collect();
                    let start = usize::from(word != "tenant");
                    match words.iter().skip(start).position(|x| x == m) {
                        Some(at) => {
                            words.remove(at + start);
                        }
                        None => words.push(m.to_string()),
                    }
                    p.cfg.style.insert(word, words.join(" "));
                }
            }
            vec![]
        }
        ("look-colour", slot) => {
            let word = arg.to_string();
            let spec = p.cfg.style_for(&word);
            let mods: Vec<&str> = spec.split_whitespace().skip(1).collect();
            let new = std::iter::once(slot)
                .chain(mods)
                .collect::<Vec<_>>()
                .join(" ");
            p.cfg.style.insert(word, new);
            vec![]
        }
        _ => vec![],
    }
}

fn sign_menu(view: usize, key: &str, value: &str, described: &str) -> Effect {
    Effect::Menu(Menu::new(
        format!("cond-sign:{view}:{key}:{value}"),
        described.to_string(),
        vec![
            MenuItem::new(
                'i',
                "only these",
                format!("show only those {described}"),
                "include",
            ),
            MenuItem::new(
                'e',
                "all but these",
                format!("leave out those {described}"),
                "exclude",
            ),
        ],
    ))
}

// --- keys --------------------------------------------------------------------

/// The actions of a section's view: its keys, and `ctrl-r` for the comments
/// kept for later.
pub(crate) fn action_specs(p: &GitlabPlugin, section: &str) -> Vec<v::ActionSpec> {
    let mut rows = key_rows(p, section);
    rows.push(crate::act(
        &format!("gitlab.{section}.kept_comments"),
        Binding::new("ctrl-r", "kept comments…"),
    ));
    crate::actions(&rows)
}

pub fn key(p: &mut GitlabPlugin, section: &str, key: KeyEvent) -> Outcome {
    if let Some((target, mut input)) = p.edit.take() {
        return match input.key(key) {
            Edit::Cancel => Outcome::done(),
            Edit::Typing => {
                p.edit = Some((target, input));
                Outcome::done()
            }
            Edit::Submit(text) => match validate(p, &target, &text) {
                Err(why) => {
                    p.edit = Some((target, input));
                    Outcome::Handled(vec![Effect::Notify(format!("not yet: {why}"))])
                }
                Ok(_) => Outcome::Handled(commit(p, target, text.trim().to_string())),
            },
        };
    }
    let at = index(section);
    if let Some(mv) = Move::from_key(&key, p.section_scroll[at].height()) {
        // Headings are not selectable, in look and in connection.
        let conn = conn_rows(p);
        let can = |row: usize| match section {
            "look" => !matches!(look_rows().get(row), Some(LookRow::Header(_))),
            "connection" => conn.get(row).is_none_or(|r| r.selectable()),
            _ => true,
        };
        p.section_rows[at] = select(row_count(p, section), p.section_rows[at], mv, can);
        return Outcome::done();
    }
    // Space folds and unfolds a tenancy (h and l move between panes).
    if section == "tenancies"
        && key.code == KeyCode::Char(' ')
        && let Some(SourceRow::Tenancy(i)) = source_rows(p).get(p.section_rows[1]).cloned()
    {
        let g = p.cfg.sources[i].group.clone();
        if !p.folded.remove(&g) {
            p.folded.insert(g);
        }
        return Outcome::done();
    }
    // `+` (or `=`) opens a folded tenancy; `-` folds the tenancy the row is
    // in, the cursor to it.
    if section == "tenancies"
        && let KeyCode::Char(c @ ('+' | '=' | '-')) = key.code
    {
        let rows = source_rows(p);
        let here = p.section_rows[1];
        let Some(i) = rows[..=here.min(rows.len().saturating_sub(1))]
            .iter()
            .rev()
            .find_map(|r| {
                if let SourceRow::Tenancy(i) = r {
                    Some(*i)
                } else {
                    None
                }
            })
        else {
            return Outcome::Ignored;
        };
        let g = p.cfg.sources[i].group.clone();
        // Nothing to open or close: the host says so.
        if c == '-' {
            if !p.folded.insert(g) {
                return Outcome::Ignored;
            }
            p.section_rows[1] = source_rows(p)
                .iter()
                .position(|r| *r == SourceRow::Tenancy(i))
                .unwrap_or(here);
        } else if !matches!(rows.get(here), Some(SourceRow::Tenancy(t)) if *t == i)
            || !p.folded.remove(&g)
        {
            return Outcome::Ignored;
        }
        return Outcome::done();
    }
    // ⏎ on an add row starts typing there; elsewhere it opens the row's options.
    let on_add_row = match section {
        "tenancies" => matches!(source_rows(p).get(p.section_rows[1]), Some(SourceRow::Add)),
        "views" => view_index(p).is_none(),
        _ => false,
    };
    // ⏎ answers a found token's question, and asks GitLab on the check row.
    if key.code == KeyCode::Enter && section == "connection" {
        match conn_row(p) {
            ConnRow::Offer(k) => return Outcome::Handled(use_offer(p, k)),
            ConnRow::Field(Field::Check) => return Outcome::Handled(check_connection(p)),
            ConnRow::Access(a) => return Outcome::Handled(choose_access(p, a)),
            ConnRow::AccessHeading => return Outcome::done(),
            // The follow list's add row types into the tenancies' add field.
            ConnRow::FollowAdd => {
                p.edit = Some((Target::SourceAdd, TextInput::new("")));
                return Outcome::done();
            }
            ConnRow::Gap | ConnRow::FollowHeading => return Outcome::done(),
            ConnRow::Follow(_) => {}
            // As in every part's connection: ⏎ goes straight to changing
            // the field (or checking, on one that only checks).
            ConnRow::Field(_) => {
                if let Some(menu) = row_menu(p, section)
                    && let Some(first) = menu.items.first()
                    && (menu.items.len() == 1 || first.key == Some('e'))
                {
                    let value = first.value.clone();
                    return Outcome::Handled(on_menu(p, &menu.id, &value));
                }
            }
        }
    }
    if key.code == KeyCode::Enter && on_add_row {
        p.edit = Some(if section == "tenancies" {
            (Target::SourceAdd, TextInput::new(""))
        } else {
            (Target::ViewAdd, TextInput::new(""))
        });
        return Outcome::done();
    }
    if key.code == KeyCode::Enter {
        return Outcome::Handled(row_menu(p, section).map(Effect::Menu).into_iter().collect());
    }
    // A menu item's key works straight from the row.
    if let KeyCode::Char(c) = key.code
        && let Some(menu) = row_menu(p, section)
        && let Some(item) = menu.items.iter().find(|i| i.key == Some(c))
    {
        let value = item.value.clone();
        return Outcome::Handled(on_menu(p, &menu.id, &value));
    }
    // Adding works from anywhere in the section, even when it is empty.
    match (section, key.code) {
        // The server's new-token page: `o` opens it, `y` copies its link.
        ("connection", KeyCode::Char('o')) => {
            Outcome::Handled(vec![Effect::Open(crate::setup::token_page(p))])
        }
        ("connection", KeyCode::Char('y')) => Outcome::Handled(vec![
            Effect::Copy(crate::setup::token_page(p)),
            Effect::Notify("copied the new-token page's link".into()),
        ]),
        // From anywhere in the section: jump to the add row and type there.
        ("tenancies", KeyCode::Char('a')) => {
            p.section_rows[1] = 0;
            p.edit = Some((Target::SourceAdd, TextInput::new("")));
            Outcome::done()
        }
        ("views", KeyCode::Char('n')) => {
            p.section_rows[2] = 0;
            p.edit = Some((Target::ViewAdd, TextInput::new("")));
            Outcome::done()
        }
        _ => Outcome::Ignored,
    }
}

fn commit(p: &mut GitlabPlugin, target: Target, value: String) -> Vec<Effect> {
    match target {
        Target::Conn(f) => {
            match f {
                // Who is signed in is found again with the new server or token.
                Field::Url => {
                    p.replace_connection(value, p.cfg.token.clone());
                    p.me.clear();
                }
                Field::Token => {
                    p.replace_connection(p.cfg.url.clone(), value);
                    p.me.clear();
                }
                Field::Base => {
                    p.cfg.base = config::group_path_of(&value);
                    settings_checks::reconcile(p);
                }
                Field::Refresh => {
                    p.cfg.refresh_secs = value
                        .trim_end_matches('s')
                        .parse()
                        .unwrap_or(p.cfg.refresh_secs)
                }
                Field::History => {
                    let before = p.cfg.history_days;
                    p.cfg.history_days =
                        value.trim_end_matches('d').trim().parse().unwrap_or(before);
                    // Further back than what is held: fetch it again.
                    if p.cfg.history_days > before {
                        for d in p.data.values_mut() {
                            d.ended_since = None;
                        }
                    }
                }
                Field::SignedIn | Field::Check => return vec![],
            }
            match f.check() {
                Some(key) => {
                    p.checks.remove(key);
                    p.data.clear();
                    crate::runs_ownership::reconcile(p);
                    let mut fx = p.refresh();
                    fx.push(ask_to_check(
                        if key == "auth" {
                            CheckTarget::Auth
                        } else {
                            CheckTarget::Base
                        },
                        if key == "auth" {
                            "the connection"
                        } else {
                            "the base group"
                        },
                    ));
                    fx
                }
                _ => vec![Effect::Notify(format!("{} changed", f.label()))],
            }
        }
        Target::SourceCode(i) => {
            p.cfg.sources[i].code = value;
            vec![]
        }
        Target::SourceGroup(i) => {
            let value = config::relative_group(&value, &p.cfg.base);
            if p.cfg.sources[i].group == value {
                return vec![];
            }
            let old = p.cfg.group_path(&p.cfg.sources[i]);
            p.data.remove(&old);
            crate::runs_ownership::reconcile(p);
            p.checks.remove(&format!("group:{old}"));
            p.cfg.sources[i].group = value.clone();
            settings_checks::reconcile(p);
            // Hidden repos belonged to the old group.
            p.cfg.sources[i].hide.clear();
            let path = config::join_path(&p.cfg.base, &value);
            let mut fx = p.refresh();
            fx.push(ask_to_check(CheckTarget::Source(path.clone()), &path));
            fx
        }
        Target::SourceAdd => {
            let value = config::relative_group(&value, &p.cfg.base);
            let last = value.rsplit('/').next().unwrap_or(&value);
            let code = last
                .chars()
                .filter(char::is_ascii_alphanumeric)
                .take(3)
                .collect();
            let color = TENANT_COLORS[p.cfg.sources.len() % TENANT_COLORS.len()].to_string();
            p.cfg.sources.push(Source {
                group: value.clone(),
                code,
                color,
                hide: vec![],
            });
            settings_checks::reconcile(p);
            p.section_rows[1] = source_rows(p)
                .iter()
                .rposition(|r| matches!(r, SourceRow::Tenancy(_)))
                .unwrap_or(0);
            let path = config::join_path(&p.cfg.base, &value);
            let mut fx = p.refresh();
            fx.push(ask_to_check(CheckTarget::Source(path.clone()), &path));
            fx
        }
        Target::ViewFilter(i) => {
            p.cfg.views[i].filter = value;
            vec![]
        }
        Target::ViewName(i) => {
            p.cfg.views[i].name = value;
            vec![]
        }
        Target::ViewAdd => {
            p.cfg.views.push(View {
                name: value,
                filter: String::new(),
                repos: Default::default(),
            });
            p.section_rows[2] = view_row(p.cfg.views.len() - 1);
            vec![Effect::Notify(
                "new view shows everything · a adds a condition".into(),
            )]
        }
        Target::ViewWords(i) => {
            for w in value.split_whitespace() {
                p.cfg.views[i].filter =
                    filter::add(&p.cfg.views[i].filter, false, "text", w).replace("text:", "");
            }
            vec![]
        }
    }
}

/// A click on a row selects it; the host turns a double click into `⏎`,
/// which opens the row's options or starts adding.
pub fn click(p: &mut GitlabPlugin, section: &str, id: &str) -> Outcome {
    if p.edit.is_some() {
        return Outcome::done();
    }
    if let Some(i) = id
        .strip_prefix("row:")
        .and_then(|n| n.parse::<usize>().ok())
    {
        p.section_rows[index(section)] = i;
    }
    Outcome::done()
}

pub fn hints(p: &GitlabPlugin, _section: &str) -> Vec<KeyHint<'static>> {
    if p.edit.is_some() {
        return vec![KeyHint::new("⏎", "keep"), KeyHint::new("esc", "cancel")];
    }
    vec![]
}

/// The actions menu (`x`) for the selected row: its options, whose keys
/// work straight from the row, named after the row. Each row has its id (`gitlab.<section>.<verb>`). The
/// row menu's items carry the written name of what they do (their `value`);
/// the other rows are named here.
fn key_rows(p: &GitlabPlugin, section: &str) -> Vec<crate::Row> {
    use crate::{act, head};
    let on_add_row = match section {
        "tenancies" => matches!(source_rows(p).get(p.section_rows[1]), Some(SourceRow::Add)),
        "views" => view_index(p).is_none(),
        _ => false,
    };
    if section == "connection" {
        // The new-token page, from every row but a found token's (its `y`
        // is yes).
        let page = |offer: bool| {
            let mut rows = vec![act("gitlab.connection.token_page", Binding::new("o", "open GitLab's new-token page"))];
            if !offer {
                rows.push(act("gitlab.connection.copy_token_link", Binding::new("y", "copy the new-token page's link")));
            }
            rows
        };
        match conn_row(p) {
            ConnRow::Offer(k) => {
                let Some(var) = p.offers().get(k).cloned() else {
                    return vec![];
                };
                let mut rows = vec![
                    head(Binding::heading(found(&var).title())),
                    act("gitlab.connection.use_offer", Binding::new("⏎", "use it")),
                ];
                rows.extend(page(true));
                return rows;
            }
            ConnRow::Field(Field::Check) => {
                let mut rows = vec![
                    head(Binding::heading(connection::CHECK)),
                    act("gitlab.connection.check", Binding::new("⏎", "ask GitLab now")),
                ];
                rows.extend(page(false));
                return rows;
            }
            ConnRow::Access(a) => {
                let mut rows = vec![
                    head(Binding::heading(a.label())),
                    act("gitlab.connection.choose", Binding::new("⏎", "give the token this access")),
                ];
                rows.extend(page(false));
                return rows;
            }
            ConnRow::Gap | ConnRow::AccessHeading | ConnRow::FollowHeading | ConnRow::FollowAdd => {
                let mut rows = vec![
                    head(Binding::heading("follow a group")),
                    act("gitlab.connection.follow", Binding::new("⏎", "type or paste the group's link or path")),
                ];
                rows.extend(page(false));
                return rows;
            }
            ConnRow::Follow(i) => {
                let mut rows = vec![
                    head(Binding::heading(p.cfg.sources[i].group.clone())),
                    act("gitlab.connection.follow_check", Binding::new("c", "check it")),
                    act("gitlab.connection.unfollow", Binding::new("d", "stop tracking…")),
                ];
                rows.extend(page(false));
                return rows;
            }
            ConnRow::Field(_) => {
                let mut rows = key_rows_of_menu(p, section);
                rows.extend(page(false));
                return rows;
            }
        }
    }
    if on_add_row {
        let (title, id, what) = if section == "tenancies" {
            ("add a tenancy", "gitlab.tenancies.add", "type or paste the group's link or path")
        } else {
            ("new view", "gitlab.views.add", "type its name")
        };
        return vec![head(Binding::heading(title)), act(id, Binding::new("⏎", what))];
    }
    key_rows_of_menu(p, section)
}

/// The rows of the selected row's options menu, named after the row.
fn key_rows_of_menu(p: &GitlabPlugin, section: &str) -> Vec<crate::Row> {
    use crate::{act, head};
    let Some(menu) = row_menu(p, section) else {
        return vec![];
    };
    let mut b = vec![head(Binding::heading(menu.title))];
    if let Some(SourceRow::Tenancy(i)) = source_rows(p).get(p.section_rows[1])
        && section == "tenancies"
    {
        let folded = p.folded.contains(&p.cfg.sources[*i].group);
        b.push(act(
            "gitlab.tenancies.toggle_fold",
            Binding::new(
                "space",
                if folded {
                    "show its repos"
                } else {
                    "hide its repos"
                },
            ),
        ));
    }
    b.extend(menu.items.iter().filter_map(|i| {
        let id = format!("gitlab.{section}.{}", i.value.replace('-', "_"));
        Some((id, Binding::new(key_text(i.key?), &i.label)))
    }));
    b
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{iids, sample};
    use radar_core::ratatui::crossterm::event::KeyModifiers;

    fn press(p: &mut GitlabPlugin, section: &str, code: KeyCode) -> Outcome {
        key(p, section, KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn typed(p: &mut GitlabPlugin, section: &str, text: &str) -> Outcome {
        for c in text.chars() {
            press(p, section, KeyCode::Char(c));
        }
        press(p, section, KeyCode::Enter)
    }

    /// `<` and `>` move a tenancy or a view: never `J`/`K`, which the host
    /// takes as a screen key in a mix with Jira.
    #[test]
    fn tenancies_and_views_move_with_lt_and_gt_and_the_cursor_follows() {
        let mut p = sample();
        let groups: Vec<String> = p.cfg.sources.iter().map(|s| s.group.clone()).collect();
        assert!(groups.len() > 1);
        p.section_rows[1] = source_rows(&p)
            .iter()
            .position(|r| *r == SourceRow::Tenancy(0))
            .unwrap();
        let menu = row_menu(&p, "tenancies").unwrap();
        let keys: Vec<Option<char>> = menu.items.iter().map(|i| i.key).collect();
        assert!(
            keys.contains(&Some('>')) && !keys.contains(&Some('<')),
            "first: only down"
        );
        assert!(!keys.iter().any(|k| matches!(k, Some('J' | 'K'))));
        press(&mut p, "tenancies", KeyCode::Char('>'));
        assert_eq!(p.cfg.sources[1].group, groups[0]);
        assert_eq!(
            source_rows(&p)[p.section_rows[1]],
            SourceRow::Tenancy(1),
            "the cursor follows it"
        );
        press(&mut p, "tenancies", KeyCode::Char('<'));
        assert_eq!(p.cfg.sources[0].group, groups[0]);

        let names: Vec<String> = p.cfg.views.iter().map(|v| v.name.clone()).collect();
        p.section_rows[2] = view_row(1);
        press(&mut p, "views", KeyCode::Char('>'));
        assert_eq!(p.cfg.views[2].name, names[1]);
        press(&mut p, "views", KeyCode::Char('K'));
        assert_eq!(p.cfg.views[2].name, names[1], "K is not a move key");
    }

    fn menu_of(out: &Outcome) -> Option<&Menu> {
        match out {
            Outcome::Handled(fx) => fx.iter().find_map(|e| {
                if let Effect::Menu(m) = e {
                    Some(m)
                } else {
                    None
                }
            }),
            Outcome::Ignored => None,
        }
    }

    #[test]
    fn hiding_a_repo_removes_its_mrs_at_once() {
        let mut p = sample();
        let at = source_rows(&p)
            .iter()
            .position(|r| matches!(r, SourceRow::Repo { rel, .. } if rel == "web"))
            .unwrap();
        p.section_rows[1] = at;
        press(&mut p, "tenancies", KeyCode::Char(' '));
        assert!(!iids(&p).contains(&187), "web's merge request is gone");
        assert_eq!(p.cfg.sources[1].hide, vec!["tools/renovate", "web"]);
    }

    #[test]
    fn plus_and_minus_fold_a_tenancy_and_minus_on_its_repo_folds_it_and_lands_on_it() {
        let mut p = sample();
        assert!(radar_core::Plugin::section_folds(&p, "tenancies"));
        let at = source_rows(&p)
            .iter()
            .position(|r| matches!(r, SourceRow::Repo { rel, .. } if rel == "web"))
            .unwrap();
        let SourceRow::Repo { source, .. } = source_rows(&p)[at].clone() else {
            unreachable!()
        };
        let g = p.cfg.sources[source].group.clone();
        p.section_rows[1] = at;
        press(&mut p, "tenancies", KeyCode::Char('+'));
        assert!(!p.folded.contains(&g), "+ on a repo opens nothing");
        press(&mut p, "tenancies", KeyCode::Char('-'));
        assert!(p.folded.contains(&g));
        assert_eq!(
            source_rows(&p)[p.section_rows[1]],
            SourceRow::Tenancy(source),
            "on its tenancy"
        );
        press(&mut p, "tenancies", KeyCode::Char('+'));
        assert!(!p.folded.contains(&g));
        press(&mut p, "tenancies", KeyCode::Char('-'));
        press(&mut p, "tenancies", KeyCode::Char('='));
        assert!(!p.folded.contains(&g));
        assert_eq!(
            p.cfg.sources[source].hide,
            vec!["tools/renovate"],
            "nothing hidden on the way"
        );
    }

    #[test]
    fn enter_opens_options_and_removing_asks_first() {
        let mut p = sample();
        p.section_rows[1] = 1; // the first tenancy, below "+ add a tenancy"
        let out = press(&mut p, "tenancies", KeyCode::Enter);
        let m = menu_of(&out).unwrap();
        assert!(m.items.iter().any(|i| i.label == "stop tracking…"));
        let out = press(&mut p, "tenancies", KeyCode::Char('d'));
        let confirm = menu_of(&out).unwrap().id.clone();
        assert_eq!(p.cfg.sources.len(), 2, "nothing removed before yes");
        on_menu(&mut p, &confirm, "yes");
        assert_eq!(p.cfg.sources.len(), 1);
    }

    #[test]
    fn changing_the_token_validates_and_offers_a_check() {
        let mut p = sample();
        p.section_rows[0] = Field::ALL.iter().position(|f| *f == Field::Token).unwrap();
        press(&mut p, "connection", KeyCode::Char('e'));
        for _ in 0..40 {
            press(&mut p, "connection", KeyCode::Backspace);
        }
        let out = typed(&mut p, "connection", "short");
        assert!(
            matches!(&out, Outcome::Handled(fx) if matches!(&fx[..], [Effect::Notify(m)] if m.contains("too short")))
        );
        assert!(p.edit.is_some(), "still editing after a refusal");
        for _ in 0..5 {
            press(&mut p, "connection", KeyCode::Backspace);
        }
        let out = typed(&mut p, "connection", "glab");
        assert_eq!(p.cfg.token, "glab");
        assert_eq!(menu_of(&out).unwrap().id, CheckTarget::Auth.menu());
        let fx = on_menu(&mut p, &CheckTarget::Auth.menu(), "yes");
        assert!(matches!(&fx[..], [Effect::SpawnAction { .. }]));
        assert_eq!(
            p.checks.get("auth").map(|(c, _)| c.clone()),
            Some(Check::Running)
        );
    }

    #[test]
    fn a_condition_is_built_from_menus_without_syntax() {
        let mut p = sample();
        let i = p
            .cfg
            .views
            .iter()
            .position(|v| v.name == "Everything")
            .unwrap();
        p.section_rows[2] = view_row(i);
        let out = press(&mut p, "views", KeyCode::Char('a'));
        assert_eq!(menu_of(&out).unwrap().id, format!("cond:{i}"));
        let fx = on_menu(&mut p, &format!("cond:{i}"), "is");
        let Effect::Menu(m) = &fx[0] else { panic!() };
        let fx = on_menu(&mut p, &m.id.clone(), "bot");
        let Effect::Menu(sign) = &fx[0] else { panic!() };
        on_menu(&mut p, &sign.id.clone(), "exclude");
        assert_eq!(p.cfg.views[i].filter, "-is:bot");
        assert_eq!(filter::describe(&p.cfg.views[i].filter), "not from bots");
    }

    #[test]
    fn look_toggles_and_colours_through_the_menu() {
        let mut p = sample();
        p.section_rows[3] = look_rows()
            .iter()
            .position(|r| matches!(r, LookRow::Word("review", _)))
            .unwrap();
        press(&mut p, "look", KeyCode::Char('o'));
        assert_eq!(p.cfg.style_for("review"), "status.warning");
        let out = press(&mut p, "look", KeyCode::Char('c'));
        let id = menu_of(&out).unwrap().id.clone();
        on_menu(&mut p, &id, "accent.primary");
        assert_eq!(p.cfg.style_for("review"), "accent.primary");
        press(&mut p, "look", KeyCode::Char('r'));
        assert_eq!(p.cfg.style_for("review"), "status.warning bold");
    }

    #[test]
    fn movement_in_look_skips_headings() {
        let mut p = sample();
        p.section_rows[3] = 1;
        // Eleven words, then the pipelines heading is skipped.
        for _ in 0..11 {
            press(&mut p, "look", KeyCode::Char('j'));
        }
        assert!(matches!(
            look_rows()[p.section_rows[3]],
            LookRow::Word("passed", _)
        ));
    }

    #[test]
    fn home_end_and_pages_in_look_rest_on_a_word_never_a_heading() {
        let mut p = sample();
        let on_word =
            |p: &GitlabPlugin| matches!(look_rows()[p.section_rows[3]], LookRow::Word(..));
        for k in [
            KeyCode::Home,
            KeyCode::End,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Char('g'),
            KeyCode::Char('G'),
        ] {
            press(&mut p, "look", k);
            assert!(on_word(&p), "{k:?}");
        }
        press(&mut p, "look", KeyCode::Home);
        assert_eq!(
            p.section_rows[3], 1,
            "the first word, under the first heading"
        );
    }

    #[test]
    fn history_is_typed_validated_and_kept() {
        let mut p = sample();
        p.section_rows[0] = Field::ALL
            .iter()
            .position(|x| *x == Field::History)
            .unwrap();
        press(&mut p, "connection", KeyCode::Char('e'));
        for _ in 0..4 {
            press(&mut p, "connection", KeyCode::Backspace);
        }
        assert!(validate(&p, &Target::Conn(Field::History), "365").is_err());
        typed(&mut p, "connection", "30");
        assert_eq!(p.cfg.history_days, 30);
        assert!(
            p.data.values().all(|d| d.ended_since.is_none()),
            "further back: fetched again"
        );
    }

    #[test]
    fn a_repo_link_is_accepted_as_a_tenancy_and_its_kind_comes_from_gitlab() {
        let mut p = sample();
        let ok = validate(
            &p,
            &Target::SourceAdd,
            "https://gitlab.com/acme/team-c/api/-/merge_requests/3",
        )
        .unwrap();
        assert!(
            ok.contains("acme/team-c/api") && ok.contains("check tells"),
            "{ok}"
        );
        let seen = validate(&p, &Target::SourceAdd, "acme/team-a/web").unwrap();
        assert!(seen.contains("the repo"), "{seen}");
        p.cfg.base.clear();
        p.cfg.sources.push(Source {
            group: "acme/team-c/api".into(),
            code: "api".into(),
            color: "blue".into(),
            hide: vec![],
        });
        let s = p.cfg.sources.last().unwrap().clone();
        assert!(kind_of(&p, &s).starts_with("not known"));
        p.checks.insert(
            "group:acme/team-c/api".into(),
            (
                Check::Passed("found the repo Acme / api: 2 open merge requests".into()),
                0,
            ),
        );
        assert_eq!(kind_of(&p, &s), "a single repo");
    }

    #[test]
    fn a_view_condition_for_merged_ones_and_for_a_label() {
        let mut p = sample();
        p.section_rows[2] = view_row(0);
        on_menu(&mut p, "cond-state:0", "merged");
        assert_eq!(p.cfg.views[0].filter, "state:merged");
        let fx = on_menu(&mut p, "cond:0", "label");
        let Effect::Menu(m) = &fx[0] else { panic!() };
        assert!(m.items.iter().any(|i| i.label == "performance"));
        let fx = on_menu(&mut p, &m.id.clone(), "backend");
        let Effect::Menu(sign) = &fx[0] else { panic!() };
        on_menu(&mut p, &sign.id.clone(), "include");
        assert_eq!(p.cfg.views[0].filter, "state:merged label:backend");
        assert_eq!(
            filter::describe(&p.cfg.views[0].filter),
            "merged lately, labelled backend"
        );
    }

    #[test]
    fn literal_tokens_are_masked() {
        assert_eq!(masked("env:GITLAB_TOKEN"), "env:GITLAB_TOKEN");
        // Not shaped like a real token, so secret scanners leave it alone.
        assert_eq!(
            masked("glpat-not.a.real.token.1234"),
            "glpa…1234 (typed in)"
        );
        assert_eq!(masked("glab"), "glab");
    }

    /// The connection, the same way in every part: where, the token, who it
    /// signs in as, the check, refreshing, then GitLab's own.
    #[test]
    fn the_connection_reads_in_the_order_every_part_has() {
        let labels: Vec<&str> = Field::ALL.iter().map(|f| f.label()).collect();
        assert_eq!(
            labels,
            [
                "server",
                "base group",
                "token",
                "signed in as",
                "check",
                "refresh",
                "merged, closed"
            ]
        );
    }

    #[test]
    fn a_token_in_the_environment_is_asked_about_first_and_used_by_name() {
        let mut p = sample();
        // A token typed in by hand, and GITLAB_TOKEN set: offered.
        p.cfg.token = "glab".into();
        p.env = Some(vec![(
            "GITLAB_TOKEN".into(),
            "glpat-not.a.real.token.1234".into(),
        )]);
        // Signed in, the token works: nothing is offered.
        assert!(p.offers().is_empty());
        p.me.clear();
        assert_eq!(conn_rows(&p)[0], ConnRow::Offer(0));
        let theme = radar_core::Theme::new(radar_core::design::theme::Tier::Mono);
        let row = found("GITLAB_TOKEN").row(&theme, 120).to_string();
        assert_eq!(
            row,
            " ? Found in your environment: a GitLab token (GITLAB_TOKEN). Use it?"
        );
        let out = press(&mut p, "connection", KeyCode::Enter);
        assert_eq!(
            p.cfg.token, "env:GITLAB_TOKEN",
            "the name, never the value"
        );
        assert!(
            matches!(&out, Outcome::Handled(fx) if fx.iter().any(|e| matches!(e, Effect::SpawnAction { .. }))),
            "and checks it"
        );
        assert!(p.offers().is_empty(), "in use: no longer offered");
        assert_eq!(
            conn_rows(&p)[p.section_rows[0]],
            ConnRow::Field(Field::Token)
        );
    }

    /// Setup's GitLab step is this section: groups are followed from it with
    /// the tenancies' own add field, the cursor steps over the follow list's
    /// heading, and the sheet carries the help to get a token.
    #[test]
    fn the_connection_follows_groups_with_the_tenancies_own_field() {
        let mut p = sample();
        let before = p.cfg.sources.len();
        let rows = conn_rows(&p);
        let add = rows.iter().position(|r| *r == ConnRow::FollowAdd).unwrap();
        p.section_rows[0] = add - 3;
        press(&mut p, "connection", KeyCode::Down);
        assert_eq!(conn_rows(&p)[p.section_rows[0]], ConnRow::FollowAdd, "past the gap and the heading");
        press(&mut p, "connection", KeyCode::Enter);
        assert!(matches!(p.edit, Some((Target::SourceAdd, _))));
        let out = typed(&mut p, "connection", "ankh-morpork/watch");
        assert_eq!(p.cfg.sources.len(), before + 1);
        assert!(p.cfg.sources.last().unwrap().group.ends_with("ankh-morpork/watch"));
        assert!(menu_of(&out).is_some_and(|m| m.id.starts_with("ask-check:")), "offers to check it");
        assert!(conn_rows(&p).contains(&ConnRow::Follow(before)));

        p.section_rows[0] = conn_rows(&p).iter().position(|r| *r == ConnRow::Follow(before)).unwrap();
        let out = press(&mut p, "connection", KeyCode::Char('d'));
        assert!(menu_of(&out).is_some_and(|m| m.id == format!("tenancy-remove:{before}")), "asked first");
        let ids: Vec<String> = action_specs(&p, "connection").into_iter().map(|a| a.id).collect();
        assert!(ids.contains(&"gitlab.connection.unfollow".to_string()));

        p.section_rows[0] = 0;
        let view = connection_view(&p, 80, 0);
        let sheet = format!("{:?}", view.detail_blocks);
        assert!(sheet.contains("GET A GITLAB TOKEN") && sheet.contains("personal_access_tokens?name=radar"));
        let out = press(&mut p, "connection", KeyCode::Char('o'));
        assert!(matches!(&out, Outcome::Handled(fx) if matches!(fx.as_slice(), [Effect::Open(url)] if url.contains("/-/user_settings/personal_access_tokens"))));
    }

    #[test]
    fn the_access_choice_sets_the_scope_the_token_link_asks_for() {
        use config::TokenAccess;
        use radar_core::plugin::Plugin;
        let mut p = sample();
        let at = |p: &GitlabPlugin, row| conn_rows(p).iter().position(|r| *r == row).unwrap();
        let rows = conn_rows(&p);
        let heading = at(&p, ConnRow::AccessHeading);
        assert_eq!(rows[heading - 1], ConnRow::Gap);
        assert_eq!(&rows[heading + 1..heading + 3], [ConnRow::Access(TokenAccess::Read), ConnRow::Access(TokenAccess::Write)]);
        // Up from the follow list's add row: over the gap and the heading.
        p.section_rows[0] = at(&p, ConnRow::FollowAdd);
        press(&mut p, "connection", KeyCode::Up);
        assert_eq!(conn_row(&p), ConnRow::Access(TokenAccess::Write));
        let ids: Vec<String> = action_specs(&p, "connection").into_iter().map(|a| a.id).collect();
        assert!(ids.contains(&"gitlab.connection.choose".to_string()), "{ids:?}");

        let view = connection_view(&p, 120, 0);
        let sheet = format!("{:?}", view.detail_blocks);
        assert!(sheet.contains("scopes=read_api") && sheet.contains("scope from access: read_api"), "{sheet}");
        // Nothing checked yet: choosing asks nothing of GitLab.
        let out = press(&mut p, "connection", KeyCode::Enter);
        assert_eq!(p.cfg.access, TokenAccess::Write);
        assert!(matches!(&out, Outcome::Handled(fx) if !fx.iter().any(|e| matches!(e, Effect::SpawnAction { .. }))));
        assert!(p.config().is_some_and(|c| c["access"].as_str() == Some("write")), "saved");
        let view = connection_view(&p, 120, 0);
        let sheet = format!("{:?}", view.detail_blocks);
        assert!(sheet.contains("scopes=api\"") && sheet.contains("scope from access: api"), "{sheet}");
        assert!(format!("{:?}", view.detail).contains("[gitlab] access"));
        let out = press(&mut p, "connection", KeyCode::Char('o'));
        assert!(matches!(&out, Outcome::Handled(fx) if matches!(fx.as_slice(), [Effect::Open(url)] if url.ends_with("scopes=api"))));

        // Once checked, a new choice asks again, so the check row says
        // whether the token allows it.
        p.checks.insert("auth".into(), (Check::Passed("signed in as vimes (Sam Vimes)".into()), 0));
        p.section_rows[0] = at(&p, ConnRow::Access(TokenAccess::Read));
        let out = press(&mut p, "connection", KeyCode::Enter);
        assert_eq!(p.cfg.access, TokenAccess::Read);
        assert!(matches!(&out, Outcome::Handled(fx) if fx.iter().any(|e| matches!(e, Effect::SpawnAction { .. }))));
        assert_eq!(p.checks.get("auth").map(|(c, _)| c.clone()), Some(Check::Running));
        assert!(p.config().is_some_and(|c| c.get("access").is_none()), "read is the default, left out");
    }

    #[test]
    fn enter_on_the_check_row_asks_gitlab() {
        let mut p = sample();
        p.section_rows[0] = Field::ALL.iter().position(|f| *f == Field::Check).unwrap();
        let out = press(&mut p, "connection", KeyCode::Enter);
        assert!(
            matches!(&out, Outcome::Handled(fx) if fx.len() == 2),
            "the connection and the base group"
        );
        assert_eq!(
            p.checks.get("auth").map(|(c, _)| c.clone()),
            Some(Check::Running)
        );
    }
}
