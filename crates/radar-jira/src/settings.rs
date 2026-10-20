//! The Jira settings sections, each with its id equal to its name:
//! `connection` (what was found in the environment or a `.mcp.json`, then
//! each site's address, account, token, who it signs in as and check;
//! refreshing), `projects` (watched per site, with the field that holds
//! acceptance criteria, and the site's colour), `views` (conditions in
//! words, or JQL that Jira checks as you type) and `look`. Every row has an
//! options menu (`⏎`) whose keys also work straight from the row; the
//! detail panel says what the row is, what it accepts and whether it was
//! checked.

use radar_core::design::help_bar::KeyHint;
use radar_core::input::{Edit, TextInput};
use radar_core::plugin::{Binding, Effect, Menu, MenuItem, Msg, Outcome, key_text};
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent};
use radar_core::theme::slot_info;
use radar_core::ui::connection;
use radar_core::ui::icon::Icon;
use radar_core::ui::rows::{Move, select};
use radar_core::ui::status;
use radar_core::view as v;
use radar_core::{log, time, widgets};

use crate::config::{self, Offer, Project, SITE_COLORS, Site, View, default_styles};
use crate::model::Me;
use crate::settings_checks::{self, Target as CheckTarget};
use crate::{Check, JiraMsg, JiraPlugin, Target, view};

// --- rows --------------------------------------------------------------------

/// The label of the row that sets `[jira] on_start`.
const ON_START: &str = "on start";

/// A site's rows in the connection section, in the order every part has
/// (`radar_core::ui::connection`): where, the account and token, who they sign
/// in as, the check; then the site's colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Field {
    Url,
    Email,
    Token,
    SignedIn,
    Check,
    /// The projects watched on the site: ⏎ watches another, typed into
    /// the projects section's own add field.
    Projects,
}

impl Field {
    const ALL: [Self; 6] = [Self::Url, Self::Email, Self::Token, Self::SignedIn, Self::Check, Self::Projects];

    const fn label(self) -> &'static str {
        match self {
            Self::Url => "site",
            Self::Email => connection::ACCOUNT,
            Self::Token => connection::TOKEN,
            Self::SignedIn => connection::SIGNED_IN,
            Self::Check => connection::CHECK,
            Self::Projects => "projects",
        }
    }

    fn about(self) -> String {
        match self {
            Self::Url => "The Jira Cloud site radar reads tickets from.".into(),
            Self::Email => "The email of the Atlassian account the API token belongs to.".into(),
            Self::Token => {
                "How radar proves who you are to Jira: an API token made at id.atlassian.com, under security, API tokens. radar only reads with it.".into()
            }
            Self::SignedIn => format!(
                "{} \"Assigned to you\" means this person.",
                connection::about_signed_in("Jira")
            ),
            Self::Check => connection::about_check("Jira"),
            Self::Projects => "The projects whose tickets radar reads on this site. ⏎ watches another.".into(),
        }
    }

    fn accepts(self) -> Option<String> {
        Some(match self {
            Self::Url => "https://acme.atlassian.net, any link on the site, or env:NAME".into(),
            Self::Email => "an email address, or env:NAME to read it from the environment".into(),
            Self::Token => connection::TOKEN_ACCEPTS.into(),
            Self::Projects => "a project key like WATCH, or a link to the project".into(),
            Self::SignedIn | Self::Check => return None,
        })
    }

    const fn key(self) -> &'static str {
        match self {
            Self::Url => "url",
            Self::Email => "email",
            Self::Token => "token",
            Self::Projects => "projects",
            Self::SignedIn | Self::Check => "",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SiteRow {
    /// A site found in the environment or a `.mcp.json`, asked about first.
    Offer(usize),
    /// `+ add a site`: first after what was found.
    Add,
    Site(usize),
    Field(usize, Field),
    Refresh,
    /// `[jira] on_start`: what Jira does when work starts on a ticket.
    OnStart,
}

/// An offer as the question the connection asks: the variables' names
/// (never their values), or what a `.mcp.json` names.
fn found(p: &JiraPlugin, o: &Offer) -> connection::Found {
    let s = &o.site;
    let mut names = Vec::new();
    for (spec, holds) in [(&s.url, "the site's address"), (&s.email, "the account"), (&s.token, "the token")] {
        match spec.strip_prefix("env:") {
            Some(var) => names.push((var.to_string(), holds.to_string())),
            // Not secret, written into the file: kept as written.
            None if !spec.is_empty() && holds != "the token" => {
                let shown = if holds == "the account" { spec.to_string() } else { config::host_of(spec) };
                names.push((shown, format!("{holds}, as written")))
            }
            None => {}
        }
    }
    let mut notes = Vec::new();
    if !o.site.from.is_empty() {
        notes.push(format!("It is the file's \"{}\" entry.", o.label));
    }
    if !s.projects.is_empty() {
        let keys: Vec<String> = s
            .projects
            .iter()
            .map(|pr| match pr.ac_field.as_str() {
                "" => pr.key.clone(),
                field => format!("{} (acceptance criteria in {field})", pr.key),
            })
            .collect();
        notes.push(format!("It also watches {}.", keys.join(", ")));
    }
    if o.token_typed {
        notes.push("Its token is typed into the file: radar does not copy it, and asks which variable holds it.".into());
    }
    let name = if p.cfg.sites.iter().any(|x| x.name == s.name) {
        format!("{}{}", s.name, p.cfg.sites.len() + 1)
    } else {
        s.name.clone()
    };
    connection::Found {
        what: "a Jira site".into(),
        names,
        file: (!o.site.from.is_empty()).then(|| o.site.from.clone()),
        does: if o.token_typed {
            format!("adds the site {name} and asks which variable holds its token")
        } else {
            format!("adds the site {name}, reads these each time, and checks it with Jira")
        },
        note: (!notes.is_empty()).then(|| notes.join(" ")),
    }
}

pub(crate) fn site_rows(p: &JiraPlugin) -> Vec<SiteRow> {
    let mut rows: Vec<SiteRow> = (0..p.offers().len()).map(SiteRow::Offer).collect();
    rows.push(SiteRow::Add);
    for i in 0..p.cfg.sites.len() {
        rows.push(SiteRow::Site(i));
        rows.extend(Field::ALL.iter().map(|f| SiteRow::Field(i, *f)));
    }
    // As a setup step the card leaves out refresh and on start, which only
    // Settings needs; the plugin remembers how the card was last drawn, so
    // keys and clicks walk what is drawn.
    if !p.setup_card.get() {
        rows.push(SiteRow::Refresh);
        rows.push(SiteRow::OnStart);
    }
    rows
}

/// The `on start` row's value: what happens when work starts on a ticket.
fn on_start_value(p: &JiraPlugin) -> &'static str {
    match p.cfg.on_start {
        config::OnStart::Ask => "ask first",
        config::OnStart::Off => "off",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProjRow {
    Site(usize),
    /// `+ watch a project` on a site.
    Add(usize),
    /// The site's colour, kept with the grouping it colours.
    Colour(usize),
    Project(usize, usize),
}

pub(crate) fn project_rows(p: &JiraPlugin) -> Vec<ProjRow> {
    let mut rows = Vec::new();
    for (i, s) in p.cfg.sites.iter().enumerate() {
        rows.push(ProjRow::Site(i));
        rows.push(ProjRow::Add(i));
        rows.push(ProjRow::Colour(i));
        rows.extend((0..s.projects.len()).map(|j| ProjRow::Project(i, j)));
    }
    rows
}

/// The views section: `+ new view`, then each view.
fn view_index(p: &JiraPlugin) -> Option<usize> {
    p.section_rows[2].checked_sub(1).filter(|i| *i < p.cfg.views.len())
}

const LOOK: [(&str, &[(&str, &str)]); 5] = [
    (
        "statuses",
        &[
            ("to do", "a ticket not started"),
            ("doing", "a ticket being worked on"),
            ("done", "a finished ticket"),
        ],
    ),
    (
        "types",
        &[
            ("bug", "a bug"),
            ("story", "a story"),
            ("task", "a task"),
            ("epic", "an epic"),
            ("subtask", "a sub-task"),
            ("other", "any other type"),
        ],
    ),
    (
        "ages",
        &[
            ("fresh", "changed in the last 2 days"),
            ("aging", "changed in the last 2 weeks"),
            ("stale", "not changed for 2 weeks or more"),
        ],
    ),
    (
        "tickets",
        &[
            ("key", "a ticket's key"),
            ("label", "a ticket's labels"),
            ("you", "\"you\", the reader, in place of their own name"),
            ("failed", "a site or project that could not be reached"),
            ("site", "a site's name; its colour is the site's own"),
        ],
    ),
    (
        "tickets tree",
        &[
            ("band", "the shade behind every other ticket and its children"),
            ("heading", "the shade behind a site's heading"),
            ("lines", "the tree lines joining statuses and tickets"),
        ],
    ),
];

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

fn selected_look(p: &JiraPlugin) -> Option<(&'static str, &'static str)> {
    match look_rows().get(p.section_rows[3]) {
        Some(LookRow::Word(w, d)) => Some((w, d)),
        _ => None,
    }
}

const fn index(section: &str) -> usize {
    match section.as_bytes() {
        b"connection" => 0,
        b"projects" => 1,
        b"views" => 2,
        _ => 3,
    }
}

fn row_count(p: &JiraPlugin, section: &str) -> usize {
    match section {
        "connection" => site_rows(p).len(),
        "projects" => project_rows(p).len(),
        "views" => p.cfg.views.len() + 1,
        _ => look_rows().len(),
    }
}

fn site_row(p: &JiraPlugin) -> SiteRow {
    let rows = site_rows(p);
    rows.get(p.section_rows[0]).cloned().unwrap_or(SiteRow::Add)
}

fn proj_row(p: &JiraPlugin) -> Option<ProjRow> {
    let tree = project_state(p);
    project_rows(p).into_iter().find(|row| project_path(p, row) == tree.selected)
}

fn field_value(p: &JiraPlugin, i: usize, f: Field) -> String {
    let s = &p.cfg.sites[i];
    let or_dash = |v: &str| if v.trim().is_empty() { "—".to_string() } else { v.to_string() };
    match f {
        Field::Url => or_dash(&s.url),
        Field::Email => or_dash(&s.email),
        Field::Token => connection::masked(&s.token, &[]),
        Field::SignedIn => p
            .data
            .get(&s.name)
            .and_then(|d| d.me.as_ref())
            .map_or_else(|| "—".into(), |m| m.name.clone()),
        Field::Check => match p.checks.get(&CheckTarget::Site(s.name.clone())) {
            None => "not checked yet".into(),
            Some((Check::Running, _)) => "asking Jira…".into(),
            Some((Check::Passed(m), _)) => m.clone(),
            Some((Check::Failed(e), _)) => e.clone(),
        },
        Field::Projects if s.projects.is_empty() => "none yet".into(),
        Field::Projects => s.projects.iter().map(|pr| pr.key.as_str()).collect::<Vec<_>>().join(", "),
    }
}

/// A project's name, as Jira gave it in a check or on a ticket.
fn project_name(p: &JiraPlugin, i: usize, key: &str) -> Option<String> {
    let site = &p.cfg.sites[i].name;
    if let Some((Check::Passed(m), _)) = p.checks.get(&CheckTarget::project(site, key)) {
        return Some(m.trim_start_matches("found ").to_string());
    }
    p.data
        .get(site)?
        .tickets
        .values()
        .find(|t| t.project == key && !t.project_name.is_empty())
        .map(|t| t.project_name.clone())
}

// --- checks --------------------------------------------------------------------

pub(crate) fn check_site(p: &mut JiraPlugin, i: usize) -> Vec<Effect> {
    use crate::settings_history::{self, Kind};
    let Some(site) = p.cfg.sites.get(i).cloned() else {
        return vec![settings_history::unavailable(Kind::Site)];
    };
    let target = CheckTarget::Site(site.name.clone());
    let request = settings_checks::begin(p, &target);
    let action = settings_history::action(Kind::Site, Some(&site), "");
    p.checks.insert(target.clone(), (Check::Running, p.now));
    let provider = p.provider.clone();
    vec![settings_history::spawn(target, request, action, async move {
        let result = settings_history::read_site(provider.client(&site)).await;
        match &result {
            Ok(got) => log::info(format!("jira check site:{}: signed in as {}", site.name, got.value)),
            Err(e) => log::warn(format!("jira check site:{} failed: {:#}", site.name, e.error())),
        }
        result
    })]
}

pub(crate) fn check_project(p: &mut JiraPlugin, i: usize, project: &str) -> Vec<Effect> {
    use crate::settings_history::{self, Kind};
    let Some(site) = p.cfg.sites.get(i).cloned() else {
        return vec![settings_history::unavailable(Kind::Project)];
    };
    let target = CheckTarget::project(&site.name, project);
    let request = settings_checks::begin(p, &target);
    let action = settings_history::action(Kind::Project, Some(&site), project);
    p.checks.insert(target.clone(), (Check::Running, p.now));
    let project = project.to_string();
    let provider = p.provider.clone();
    vec![settings_history::spawn(target, request, action, async move {
        settings_history::read_project(provider.client(&site), &project).await
    })]
}

pub(crate) fn on_checked(p: &mut JiraPlugin, target: CheckTarget, result: Result<String, String>) -> Vec<Effect> {
    let say = match &result {
        Ok(m) if matches!(target, CheckTarget::Site(_)) => Effect::Notify(radar_core::writes::done_line(&format!("signed in as {m}"))),
        Ok(m) => Effect::Notify(radar_core::writes::done_line(m)),
        Err(e) => Effect::error(None, radar_core::writes::failed_line(e)),
    };
    if let (CheckTarget::Site(site), Ok(name)) = (&target, &result) {
        let data = p.data.entry(site.clone()).or_default();
        let id = data.me.as_ref().map(|m| m.account_id.clone()).unwrap_or_default();
        data.me = Some(Me {
            account_id: id,
            name: name.clone(),
        });
    }
    let check = match result {
        Ok(m) if matches!(target, CheckTarget::Site(_)) => Check::Passed(format!("signed in as {m}")),
        Ok(m) => Check::Passed(m),
        Err(e) => Check::Failed(e),
    };
    p.checks.insert(target, (check, p.now));
    vec![say]
}

fn ask_to_check(target: &CheckTarget, what: &str) -> Effect {
    Effect::Menu(Menu::confirm(
        target.menu(),
        format!("check {what} now?"),
        "check it",
        "leave it unchecked",
    ))
}

/// Where the check `key` stands, as the kit's check: the row's mark and
/// the detail's line.
fn check(p: &JiraPlugin, key: &CheckTarget, now: i64) -> status::Check {
    let ago = |at: &i64| Some(time::ago(*at, now));
    match p.checks.get(key) {
        None => status::Check::NotYet,
        Some((Check::Running, _)) => status::Check::Running,
        Some((Check::Passed(m), at)) => status::Check::Passed { said: m.clone(), ago: ago(at) },
        Some((Check::Failed(e), at)) => status::Check::Failed { said: e.clone(), ago: ago(at) },
    }
}

/// Ask Jira about the JQL being typed once typing has paused.
pub(crate) fn tick(p: &mut JiraPlugin) -> Vec<Effect> {
    let Some((Target::ViewJql(_), input)) = &p.edit else {
        return vec![];
    };
    let text = input.value().trim().to_string();
    let quiet = p
        .typed_at
        .is_none_or(|at| at.elapsed() >= std::time::Duration::from_millis(700));
    if text.is_empty() || !quiet || p.jql_checks.contains_key(&text) {
        return vec![];
    }
    ask_jira_about(p, text)
}

/// Ask Jira to read JQL: the site the view asks, else the first.
fn ask_jira_about(p: &mut JiraPlugin, text: String) -> Vec<Effect> {
    let only = match &p.edit {
        Some((Target::ViewJql(i), _)) => p.cfg.views.get(*i).map(|v| v.site.clone()),
        _ => None,
    }
    .unwrap_or_default();
    let Some(site) = p
        .cfg
        .sites
        .iter()
        .find(|s| s.name == only)
        .or_else(|| p.cfg.sites.first())
        .cloned()
    else {
        return vec![];
    };
    p.jql_checks.insert(text.clone(), Check::Running);
    let provider = p.provider.clone();
    vec![Effect::Spawn(Box::pin(async move {
        let result = async { provider.client(&site)?.check_jql(&text).await }
            .await
            .map_err(|e| format!("{e:#}"));
        Box::new(JiraMsg::Jql { text, result }) as Msg
    }))]
}

/// Fields named like acceptance criteria on a site, and how many of a
/// project's tickets fill each in.
fn ask_ac_fields(p: &mut JiraPlugin, i: usize, project: &str) -> Vec<Effect> {
    let Some(site) = p.cfg.sites.get(i).cloned() else {
        return vec![];
    };
    let project = project.to_string();
    let provider = p.provider.clone();
    vec![
        Effect::Notify(format!("asking {} which fields could hold acceptance criteria…", site.name)),
        Effect::Spawn(Box::pin(async move {
            let result = async {
                let c = provider.client(&site)?;
                // Each field's count is asked at once, not one after another.
                let mut counts = tokio::task::JoinSet::new();
                for (id, name) in c.fields().await? {
                    if !ac_like(&name) {
                        continue;
                    }
                    let (c, project) = (c.clone(), project.clone());
                    counts.spawn(async move {
                        let n = match id.strip_prefix("customfield_") {
                            Some(num) => c
                                .count(&format!("project = {project} AND cf[{num}] is not EMPTY"))
                                .await
                                .unwrap_or(0),
                            None => 0,
                        };
                        (id, name, n)
                    });
                }
                let mut out = counts.join_all().await;
                // Named "acceptance criteria" first, then the most filled in.
                out.sort_by_key(|(_, name, n)| {
                    (!name.to_lowercase().contains("acceptance criteria"), std::cmp::Reverse(*n))
                });
                anyhow::Ok(out)
            }
            .await
            .map_err(|e| format!("{e:#}"));
            Box::new(JiraMsg::AcFields {
                site: site.name,
                project,
                result,
            }) as Msg
        })),
    ]
}

/// Whether a field's name sounds like acceptance criteria.
pub(crate) fn ac_like(name: &str) -> bool {
    let n = name.to_lowercase();
    n.contains("acceptance") || n.contains("criteria") || n.split(|c: char| !c.is_alphanumeric()).any(|w| w == "ac")
}

pub(crate) fn on_ac_fields(
    p: &mut JiraPlugin,
    site: &str,
    project: &str,
    result: Result<Vec<(String, String, u64)>, String>,
) -> Vec<Effect> {
    let fields = match result {
        Ok(f) => f,
        Err(e) => return vec![crate::model::failed_notice(&e.to_string())],
    };
    if fields.is_empty() {
        return vec![Effect::Notify(format!(
            "{site} has no field named like acceptance criteria"
        ))];
    }
    let Some(i) = p.cfg.sites.iter().position(|s| s.name == site) else {
        return vec![];
    };
    let current = p.cfg.sites[i].ac_field(project).unwrap_or("").to_string();
    let mut items: Vec<MenuItem> = fields
        .into_iter()
        .map(|(id, name, n)| {
            let used = if id == current { "in use · " } else { "" };
            MenuItem::pick(
                name,
                format!("{used}{id} · {n} of {project}'s tickets fill it in"),
                id,
            )
        })
        .collect();
    items.push(MenuItem::pick("none", "show no acceptance criteria", ""));
    vec![Effect::Menu(Menu::new(
        format!("ac:{i}:{project}"),
        format!("which field holds {project}'s acceptance criteria?"),
        items,
    ))]
}

fn ask_recent(p: &mut JiraPlugin, i: usize) -> Vec<Effect> {
    let Some(site) = p.cfg.sites.get(i).cloned() else {
        return vec![];
    };
    let provider = p.provider.clone();
    vec![Effect::Spawn(Box::pin(async move {
        let result = async { provider.client(&site)?.recent_projects().await }
            .await
            .map_err(|e| format!("{e:#}"));
        Box::new(JiraMsg::Recent {
            site: site.name,
            result,
        }) as Msg
    }))]
}

pub(crate) fn on_recent(
    p: &mut JiraPlugin,
    site: &str,
    result: Result<Vec<(String, String)>, String>,
) -> Vec<Effect> {
    let projects = match result {
        Ok(v) => v,
        Err(e) => return vec![crate::model::failed_notice(&e.to_string())],
    };
    let Some(i) = p.cfg.sites.iter().position(|s| s.name == site) else {
        return vec![];
    };
    let items: Vec<MenuItem> = projects
        .into_iter()
        .filter(|(k, _)| !p.cfg.sites[i].watches(k))
        .map(|(k, name)| MenuItem::pick(format!("{k}  {name}"), "", k))
        .collect();
    if items.is_empty() {
        return vec![Effect::Notify(
            "no other projects lately; type a key instead (a)".into(),
        )];
    }
    vec![Effect::Menu(Menu::new(
        format!("recent:{i}"),
        format!("watch which project on {site}?"),
        items,
    ))]
}

// --- bulk ----------------------------------------------------------------------

pub fn bulk(p: &JiraPlugin, section: &str) -> Vec<MenuItem> {
    match section {
        "connection" if !p.cfg.sites.is_empty() => vec![MenuItem::new(
            'c',
            "check everything",
            "each site: who the token is, then each project",
            "s-check-all",
        )],
        "projects" if p.cfg.sites.iter().any(|s| !s.projects.is_empty()) => vec![MenuItem::new(
            'c',
            "check every project",
            "that each exists and can be seen",
            "p-check-all",
        )],
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
                "l-reset-all",
            )]
        }
        _ => vec![],
    }
}

fn check_all_projects(p: &mut JiraPlugin) -> Vec<Effect> {
    let all: Vec<(usize, String)> = p
        .cfg
        .sites
        .iter()
        .enumerate()
        .flat_map(|(i, s)| s.projects.iter().map(move |pr| (i, pr.key.clone())))
        .collect();
    all.into_iter().flat_map(|(i, k)| check_project(p, i, &k)).collect()
}

pub fn on_bulk(p: &mut JiraPlugin, value: &str) -> Vec<Effect> {
    match value {
        "s-check-all" => {
            let mut fx: Vec<Effect> = (0..p.cfg.sites.len()).flat_map(|i| check_site(p, i)).collect();
            fx.extend(check_all_projects(p));
            fx
        }
        "p-check-all" => check_all_projects(p),
        "l-reset-all" => {
            p.cfg.style = default_styles();
            vec![Effect::Notify("every word back to its default".into())]
        }
        _ => vec![],
    }
}

// --- live validation ----------------------------------------------------------------

fn env_ok(spec: &str) -> Option<Result<String, String>> {
    let var = spec.strip_prefix("env:")?;
    Some(match std::env::var(var) {
        Ok(v) if !v.trim().is_empty() => Ok(format!("{var} is set")),
        _ => Err(format!("{var} is not set in this shell")),
    })
}

fn validate(p: &JiraPlugin, target: &Target, text: &str) -> Result<String, String> {
    let text = text.trim();
    #[cfg(any(test, feature = "test-support"))]
    if let Some(fixture) = &p.provider.fixture
        && text.starts_with("env:")
        && matches!(target, Target::SiteAdd | Target::SiteUrl(_) | Target::SiteEmail(_) | Target::SiteToken(_))
    {
        fixture.blocked("credential environment validation");
        return Err("Jira hosted fixture does not resolve environment references".into());
    }
    match target {
        Target::SiteAdd | Target::SiteUrl(_) => match config::url_problem(text) {
            Some(why) => Err(why),
            None if text.starts_with("env:") => Ok(format!("{} is set", &text[4..])),
            None => {
                let host = config::host_of(text);
                if p.cfg.sites.iter().enumerate().any(|(j, s)| {
                    config::host_of(&s.url) == host && *target != Target::SiteUrl(j)
                }) {
                    Err("another site has that address".into())
                } else {
                    Ok(format!("the site {host}"))
                }
            }
        },
        Target::SiteName(i) => {
            if text.is_empty() || text.chars().count() > 20 {
                Err("1 to 20 letters, digits, - or _".into())
            } else if !text.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                Err("letters, digits, - or _ only".into())
            } else if p.cfg.sites.iter().enumerate().any(|(j, s)| j != *i && s.name == text) {
                Err("another site has that name".into())
            } else {
                Ok(format!("{text} will name the site in the tree"))
            }
        }
        Target::SiteEmail(_) => match env_ok(text) {
            Some(r) => r,
            None if text.contains('@') && !text.contains(' ') => Ok("an email address".into()),
            None => Err("an email address, or env:NAME".into()),
        },
        Target::SiteToken(_) => match env_ok(text) {
            Some(r) => r,
            None if text.len() < 16 => Err("too short for an API token".into()),
            None => Ok("a token typed in; it is stored in config.toml".into()),
        },
        Target::Refresh => match text.trim_end_matches('s').parse::<u64>() {
            Ok(n) if n >= 30 => Ok(format!("every {}", time::ago(0, n as i64))),
            Ok(_) => Err("at least 30 seconds: Jira limits how often it is asked".into()),
            Err(_) => Err("whole seconds, e.g. 120".into()),
        },
        Target::ProjectAdd(i) => {
            let key = config::project_key_of(text);
            if text.is_empty() {
                Err("a project key like ABC, or a link to the project or a ticket".into())
            } else if !config::is_project_key(&key) {
                Err(format!("\"{key}\" is not a project key: capital letters and digits"))
            } else if p.cfg.sites[*i].watches(&key) {
                Err(format!("{key} is watched already"))
            } else {
                Ok(format!("will watch {key} on {}", p.cfg.sites[*i].name))
            }
        }
        Target::ViewAdd | Target::ViewName(_) => {
            if text.is_empty() {
                Err("needs a name".into())
            } else if p.cfg.views.iter().enumerate().any(|(j, v)| v.name == text && *target != Target::ViewName(j)) {
                Err("another view has that name".into())
            } else {
                Ok("a view name".into())
            }
        }
        Target::ViewJql(_) => {
            if text.is_empty() {
                return Err("type a JQL query, e.g. project = ABC AND statusCategory != Done".into());
            }
            match p.jql_checks.get(text) {
                Some(Check::Passed(m)) => Ok(m.clone()),
                Some(Check::Failed(e)) => Err(format!("Jira says: {e}")),
                Some(Check::Running) => Err("asking Jira…".into()),
                None => Err("Jira reads it when you stop typing".into()),
            }
        }
        Target::ViewWords(_) => {
            if text.is_empty() {
                Err("type words to look for".into())
            } else {
                Ok(format!("mentioning \"{text}\""))
            }
        }
        Target::ViewType(_) => {
            if text.is_empty() {
                Err("a type's name, e.g. Bug".into())
            } else {
                Ok(format!("of type {text}"))
            }
        }
        Target::ViewLabel(_) => {
            if text.is_empty() || text.contains(char::is_whitespace) {
                Err("one label, no spaces".into())
            } else {
                Ok(format!("labelled {text}"))
            }
        }
    }
}

// --- drawing ----------------------------------------------------------------------

// Where an input starts on its row: exactly where the value it replaces was
// drawn. `RowList` puts a one-cell margin before every row's text.
/// The margin `RowList` leaves at the left of a row.
const MARGIN: u16 = 1;
/// The width `add_row`'s ` + ` takes before its words.
const ADD_W: u16 = 3;
const ADD_AT: u16 = MARGIN + ADD_W;
/// A site's name starts its tree row.
const NAME_AT: u16 = 0;
/// The words after an add row's `+ ` in a tree.
const TREE_ADD_AT: u16 = 2;

fn look_summary(spec: &str, word: &str) -> String {
    let mut parts = spec.split_whitespace();
    let colour = if word == "site" {
        widgets::own_colour("site")
    } else {
        parts.next().map(|s| slot_info(s).label).unwrap_or_default()
    };
    let mods: Vec<&str> = if word == "site" { spec.split_whitespace().collect() } else { parts.collect() };
    if mods.is_empty() {
        colour
    } else {
        format!("{colour} · {}", mods.join(" "))
    }
}

/// The appearance section through the renderer-neutral contract. The row
/// targets keep the same indices as the settings state and menu handlers.
pub fn look_view(p: &JiraPlugin) -> v::SectionView {
    let rows = look_rows()
        .into_iter()
        .enumerate()
        .map(|(i, row)| match row {
            LookRow::Header(group) => v::ListRow::heading(
                format!("row:{i}"),
                v::Text::of(group.to_uppercase(), v::Ink::Muted),
            ),
            LookRow::Word(word, about) => {
                let spec = p.cfg.style_for(word);
                let sample = if word == "site" { "acme" } else { word };
                let look = if word == "site" {
                    format!("jira.blue {spec}")
                } else {
                    spec.clone()
                };
                let mut row = v::ListRow::new(
                    format!("row:{i}"),
                    v::Text::new()
                        .push(v::Seg::new("   ", v::Ink::Plain))
                        .push(v::Seg::styled(format!("{sample:<9}"), look)),
                );
                row.detail = Some(v::Text::of(about, v::Ink::Muted));
                row.right = Some(v::Text::of(look_summary(&spec, word), v::Ink::Muted));
                row
            }
        })
        .collect();
    let mut detail = Vec::new();
    if let Some((word, about)) = selected_look(p) {
        let spec = p.cfg.style_for(word);
        detail.push(v::Text::plain(format!("How {about} is drawn.")));
        detail.push(v::Text::new());
        detail.push(v::Text::new()
            .push(v::Seg::new("  sample  ", v::Ink::Muted))
            .push(v::Seg::styled(if word == "site" { "acme" } else { word },
                if word == "site" { format!("jira.blue {spec}") } else { spec.clone() })));
        detail.push(v::Text::new());
        if word != "site" {
            let colour = spec.split_whitespace().next().unwrap_or("text.primary");
            let info = slot_info(colour);
            detail.push(v::Text::new()
                .push(v::Seg::new("colour    ", v::Ink::Muted))
                .push(v::Seg::styled("██ ", colour))
                .push(v::Seg::new(info.label.clone(), v::Ink::Plain)));
            detail.push(v::Text::of(
                format!("          the theme's {} colour, used for {}", info.label, info.meaning),
                v::Ink::Muted,
            ));
        }
        if !is_background(word) {
            let mods: Vec<_> = spec.split_whitespace().skip(usize::from(word != "site")).collect();
            detail.push(v::Text::new()
                .push(v::Seg::new("emphasis  ", v::Ink::Muted))
                .push(v::Seg::new(if mods.is_empty() { "none".into() } else { mods.join(", ") }, v::Ink::Plain)));
        }
        if default_styles().get(word).is_some_and(|default| *default != spec) {
            detail.push(v::Text::of(format!("default   {}", look_summary(&default_styles()[word], word)), v::Ink::Muted));
        }
        detail.push(v::Text::new());
        detail.push(v::Text::of("What each colour looks like is set in colours, under general.", v::Ink::Muted));
        detail.push(v::Text::of(format!("config: [jira.style] \"{word}\""), v::Ink::Muted));
    }
    v::SectionView {
        version: v::VERSION,
        scope: "look".into(),
        body: v::Node::List { rows, selected: Some(p.section_rows[3]), editing: None },
        detail_title: detail_title(p, "look"),
        detail,
        detail_blocks: vec![],
        actions: actions(p, "look"),
    }
}

pub fn views_view(p: &JiraPlugin, width: u16) -> v::SectionView {
    let hint = "   starts from your tickets; add conditions after";
    let mut add = v::Text::new()
        .push(v::Seg::new(" + ", v::Ink::Accent).strong())
        .push(v::Seg::new("new view", v::Ink::AccentSecondary).strong());
    if usize::from(width) > 3 + "new view".len() + hint.len() + 2 {
        add.add(v::Seg::new(hint, v::Ink::Muted));
    }
    let mut rows = vec![v::ListRow::new("row:0", add)];
    let name_w = p.cfg.views.iter().map(|v| v.name.chars().count()).max().unwrap_or(0).clamp(12, 24) + 2;
    for (i, item) in p.cfg.views.iter().enumerate() {
        let words = if item.jql.trim().is_empty() {
            view::describe_view(item).join(", ")
        } else {
            "by its own JQL".into()
        };
        rows.push(v::ListRow::new(
            format!("row:{}", i + 1),
            v::Text::new()
                .push(v::Seg::new(format!(" {:<name_w$}", item.name), v::Ink::Plain).strong())
                .push(v::Seg::new(words, v::Ink::Muted)),
        ));
    }
    let editing = p.edit.as_ref().and_then(|(target, input)| {
        let (row, at) = match target {
            Target::ViewAdd => (0, ADD_AT),
            Target::ViewName(i) => (i + 1, MARGIN + 1),
            Target::ViewJql(i) | Target::ViewWords(i) | Target::ViewType(i) | Target::ViewLabel(i) => {
                (i + 1, MARGIN + 1 + name_w as u16)
            }
            _ => return None,
        };
        Some(v::Editing { row, at, text: input.value(), caret: input.cursor() })
    });
    let mut detail = Vec::new();
    if let Some((target, input)) = &p.edit {
        detail.push(v::Text::of("AS YOU TYPE", v::Ink::Muted));
        let validation = validate(p, target, &input.value());
        detail.push(v::kit::as_you_type(validation.as_deref().map_err(String::as_str)));
        detail.push(v::Text::of("⏎ keeps it · esc cancels", v::Ink::Muted));
        detail.push(v::Text::new());
    }
    if let Some(i) = view_index(p) {
        let item = &p.cfg.views[i];
        detail.push(v::Text::plain("A view is a saved question to Jira; it is listed in the nav pane."));
        detail.push(v::Text::new());
        detail.push(v::Text::of("SHOWS TICKETS", v::Ink::Muted));
        detail.extend(view::describe_view(item).into_iter().map(|word| v::Text::of(format!("  • {word}"), v::Ink::Muted)));
        if !item.jql.trim().is_empty() {
            detail.push(v::Text::new());
            detail.push(v::Text::plain(format!("JQL   {}", item.jql)));
        }
        for condition in &item.when {
            if let Some(why) = view::problem(condition) {
                detail.push(v::kit::marked(Icon::Error, &why, v::Ink::Warn));
            }
        }
        detail.push(v::Text::new());
        detail.push(v::Text::of("ASKS", v::Ink::Muted));
        for (site_i, site) in p.cfg.sites.iter().enumerate() {
            let asks = p.jql_for(site_i, item).is_some();
            detail.push(if asks {
                v::Text::plain("  ").push(v::Seg::icon(Icon::RunPassed)).push(v::Seg::new(site.name.clone(), v::Ink::Plain))
            } else {
                v::Text::of(format!("  · not {}: {}", site.name, view::why_not(site, item)), v::Ink::Muted)
            });
        }
    } else {
        detail.push(v::Text::plain("Make another view for the Jira nav pane."));
        detail.push(v::Text::new());
        detail.push(v::Text::of("Give it a name; it starts with your open tickets. Then add conditions from its menu: who it is assigned to, status, sprint, project, type, label, when it changed, or words. JQL by hand is there too.", v::Ink::Muted));
    }
    v::SectionView {
        version: v::VERSION,
        scope: "views".into(),
        body: v::Node::List { rows, selected: Some(p.section_rows[2]), editing },
        detail_title: detail_title(p, "views"),
        detail,
        detail_blocks: vec![],
        actions: actions(p, "views"),
    }
}

pub fn connection_view(p: &JiraPlugin, width: u16, now: i64) -> v::SectionView {
    let offers = p.offers();
    let flat = site_rows(p);
    // A row's id is its place in the flat list the keys walk; a field sits
    // under its site.
    let id = |i: usize| format!("row:{i}");
    let mut roots: Vec<v::TreeRow> = Vec::new();
    let mut site_id = String::new();
    let mut editing_at: Option<(Vec<String>, u16)> = None;
    for (i, row) in flat.iter().cloned().enumerate() {
        match row {
            SiteRow::Offer(k) => roots.push(v::TreeRow::new(id(i), found(p, &offers[k]).text(width))),
            SiteRow::Add => {
                if matches!(p.edit, Some((Target::SiteAdd, _))) { editing_at = Some((vec![id(i)], TREE_ADD_AT)); }
                let mut add = v::kit::add_line("add a site", "paste its link, e.g. https://acme.atlassian.net", width.saturating_sub(4));
                if let Some(first) = add.0.first_mut() { first.text = "+ ".into(); }
                roots.push(v::TreeRow::new(id(i), add));
            }
            SiteRow::Site(site_i) => {
                let site = &p.cfg.sites[site_i];
                if matches!(p.edit, Some((Target::SiteName(k), _)) if k == site_i) {
                    editing_at = Some((vec![id(i)], NAME_AT));
                }
                site_id = id(i);
                let mut root = v::TreeRow::new(id(i), v::Text::new()
                    .push(v::Seg::styled(site.name.clone(), format!("jira.{} {}", site.color, p.cfg.style_for("site")))))
                    .detail(v::Text::of(format!("   {} · {} project{}", site.host(), site.projects.len(), if site.projects.len() == 1 { "" } else { "s" }), v::Ink::Muted))
                    .stripe(format!("jira.{}", site.color));
                if let Some(mark) = check(p, &CheckTarget::Site(site.name.clone()), now).mark_text() {
                    root = root.cell("check", mark);
                }
                roots.push(root);
            }
            SiteRow::Field(site_i, field) => {
                let target = match field {
                    Field::Url => Some(Target::SiteUrl(site_i)),
                    Field::Email => Some(Target::SiteEmail(site_i)),
                    Field::Token => Some(Target::SiteToken(site_i)),
                    Field::Projects => Some(Target::ProjectAdd(site_i)),
                    _ => None,
                };
                if target.as_ref().is_some_and(|target| p.edit.as_ref().is_some_and(|(edit, _)| edit == target)) {
                    editing_at = Some((vec![site_id.clone(), id(i)], connection::value_x(0)));
                }
                let value = if field == Field::Check {
                    connection::check_field_text(v::Text::new(), &check(p, &CheckTarget::Site(p.cfg.sites[site_i].name.clone()), now), "Jira")
                } else {
                    let value = if field == Field::SignedIn {
                        connection::signed_in_text(&field_value(p, site_i, field), "")
                    } else {
                        v::Text::of(field_value(p, site_i, field), v::Ink::Primary)
                    };
                    connection::field_text(v::Text::new(), field.label(), value)
                };
                if let Some(root) = roots.last_mut() { root.children.push(v::TreeRow::new(id(i), value)); }
            }
            SiteRow::Refresh => {
                if matches!(p.edit, Some((Target::Refresh, _))) {
                    editing_at = Some((vec![id(i)], connection::value_x(0)));
                }
                roots.push(v::TreeRow::new(id(i), connection::field_text(
                    v::Text::new(), connection::REFRESH,
                    v::Text::of(connection::every(p.cfg.refresh_secs), v::Ink::Primary),
                )));
            }
            SiteRow::OnStart => roots.push(v::TreeRow::new(id(i), connection::field_text(
                v::Text::new(), ON_START,
                v::Text::of(on_start_value(p), v::Ink::Primary),
            ))),
        }
    }
    let selected = match flat.get(p.section_rows[0]) {
        Some(SiteRow::Field(site_i, _)) => flat.iter().position(|r| *r == SiteRow::Site(*site_i))
            .map(|site| vec![id(site), id(p.section_rows[0])]).unwrap_or_default(),
        Some(_) => vec![id(p.section_rows[0])],
        None => vec![],
    };
    let open: Vec<Vec<String>> = flat.iter().enumerate()
        .filter(|(_, r)| matches!(r, SiteRow::Site(_))).map(|(i, _)| vec![id(i)]).collect();
    let editing = editing_at.and_then(|(path, at)| p.edit.as_ref().map(|(_, input)| Box::new(v::TreeEditing {
        path, at, text: input.value(), caret: input.cursor(),
    })));
    let typing = || p.edit.as_ref().map(|(target, input)| validate(p, target, &input.value()));
    let detail = match site_row(p) {
        SiteRow::Offer(k) => offers.get(k).map_or_else(Vec::new, |offer| found(p, offer).detail_text()),
        SiteRow::Add => {
            let mut d = Vec::new();
            if let Some(verdict) = typing() {
                d.push(v::kit::heading("as you type"));
                d.push(v::kit::as_you_type(verdict.as_deref().map_err(String::as_str)));
                d.push(v::Text::of("⏎ keeps it · esc cancels", v::Ink::Muted));
                d.push(v::Text::new());
            }
            d.extend([
                v::Text::of("Read tickets from another Jira Cloud site.", v::Ink::Primary),
                v::Text::new(), v::kit::heading("paste or type any of"),
                v::Text::of("  https://acme.atlassian.net", v::Ink::Secondary),
                v::Text::of("  a link to any page on the site", v::Ink::Secondary),
                v::Text::of("  env:NAME, to read it from the environment", v::Ink::Secondary),
                v::Text::new(),
                v::Text::of("Then its email and API token; radar offers to check them with Jira.", v::Ink::Muted),
            ]);
            d
        }
        SiteRow::Site(site_i) => {
            let site = &p.cfg.sites[site_i];
            let mut d = Vec::new();
            if let Some(verdict) = typing() {
                d.extend([v::kit::heading("as you type"), v::kit::as_you_type(verdict.as_deref().map_err(String::as_str)),
                    v::Text::of("⏎ keeps it · esc cancels", v::Ink::Muted), v::Text::new()]);
            }
            d.extend([v::kit::heading("checked"), check(p, &CheckTarget::Site(site.name.clone()), now).text("not checked yet", "checking with Jira…"),
                v::Text::new(), v::Text::of("A Jira site: its tickets group under it, in its colour.", v::Ink::Primary), v::Text::new(),
                v::kit::labelled("name", 10, v::Text::of(site.name.clone(), v::Ink::Emphasis)),
                v::kit::labelled("address", 10, v::Text::plain(site.host()))]);
            if !site.from.is_empty() { d.push(v::kit::labelled("from", 10, v::Text::plain(site.from.clone()))); }
            d.push(v::kit::labelled("projects", 10, v::Text::plain(if site.projects.is_empty() { "none yet: projects".into() } else { site.projects.iter().map(|pr| pr.key.as_str()).collect::<Vec<_>>().join(", ") })));
            d.extend([v::Text::new(), v::Text::of("config: [[jira.site]] name", v::Ink::Muted)]);
            d
        }
        SiteRow::Field(site_i, field) => {
            let mut d = connection::DetailText::new(field.about());
            d.accepts = field.accepts();
            if field != Field::Check { d.now = Some(field_value(p, site_i, field)); }
            if field != Field::Projects {
                d.checked = Some(check(p, &CheckTarget::Site(p.cfg.sites[site_i].name.clone()), now).text("not checked yet", "checking with Jira…"));
            }
            d.typing = typing();
            d.config = match field {
                Field::Projects => Some("[[jira.site.project]] key".into()),
                f if !f.key().is_empty() => Some(format!("[[jira.site]] {}", f.key())),
                _ => None,
            };
            d.text()
        }
        SiteRow::Refresh => {
            let mut d = connection::DetailText::new(format!("{} It asks only for the view on screen.", connection::about_refresh("Jira")));
            d.accepts = Some(connection::refresh_accepts(30));
            d.now = Some(connection::every(p.cfg.refresh_secs));
            d.config = Some("[jira] refresh_secs".into());
            d.typing = typing();
            d.text()
        }
        SiteRow::OnStart => {
            let mut d = connection::DetailText::new("When work starts on a ticket somewhere else (a workspace made its branches), Jira asks once whether to assign the ticket to you and move it into progress. Nothing is sent before you say yes, and radar's mode can refuse it. [jira] start_status names the status to move to; without it, the first move into an in-progress status.");
            d.accepts = Some("ask, or off".into());
            d.now = Some(on_start_value(p).into());
            d.config = Some("[jira] on_start".into());
            d.text()
        }
    };
    // The sheet: the row's detail, then the help that row needs.
    let help = match site_row(p) {
        SiteRow::Field(_, Field::Projects) => crate::setup::projects_help(),
        SiteRow::Refresh | SiteRow::OnStart => vec![],
        _ => crate::setup::access_help(),
    };
    let mut detail_blocks: Vec<v::Block> = detail.iter().cloned().map(v::Block::line).collect();
    if !help.is_empty() {
        detail_blocks.push(v::Block::line(v::Text::new()));
        detail_blocks.extend(help);
    }
    v::SectionView {
        version: v::VERSION,
        scope: "connection".into(),
        body: v::Node::Tree(v::Tree {
            rows: roots, selected, open,
            columns: vec![v::ColumnSpec::new("check", 10, 0)],
            name_width: width.saturating_sub(12),
            look: Some(v::TreeLook {
                lines: p.cfg.style_for("lines"), band: p.cfg.style_for("band"),
                heading: p.cfg.style_for("heading"), band_level: 2,
            }),
            editing,
            ..v::Tree::default()
        }),
        detail_title: detail_title(p, "connection"),
        detail,
        detail_blocks,
        actions: actions(p, "connection"),
    }
}

fn project_path(p: &JiraPlugin, row: &ProjRow) -> Vec<String> {
    let site_id = |i: usize| crate::screen::site_id(&p.cfg.sites[i].name);
    match row {
        ProjRow::Site(i) => vec![site_id(*i)],
        ProjRow::Add(i) => vec![site_id(*i), "add".into()],
        ProjRow::Colour(i) => vec![site_id(*i), "colour".into()],
        ProjRow::Project(i, j) => vec![site_id(*i), p.cfg.sites[*i].projects[*j].key.clone()],
    }
}

/// The structural tree is small and independent of display text. Synchronizing
/// it keeps action lookup tied to the selected identity after a site/project
/// reorder, while accepting explicit row changes from legacy settings callers.
fn project_state(p: &JiraPlugin) -> v::Tree {
    let rows = p.cfg.sites.iter().map(|site| {
        v::TreeRow::new(crate::screen::site_id(&site.name), v::Text::new())
            .child(v::TreeRow::new("add", v::Text::new()))
            .child(v::TreeRow::new("colour", v::Text::new()))
            .children(site.projects.iter().map(|project| v::TreeRow::new(project.key.clone(), v::Text::new())))
    }).collect::<Vec<_>>();
    let mut nav = p.project_tree.borrow_mut();
    if nav.selected.is_empty() || p.project_row_index.get() != p.section_rows[1] {
        nav.select(project_rows(p).get(p.section_rows[1]).map(|row| project_path(p, row)).unwrap_or_default());
    }
    nav.sync(&rows);
    p.project_row_index.set(p.section_rows[1]);
    v::Tree { rows, selected: nav.selected.clone(), open: nav.open.clone(), ..v::Tree::default() }
}

fn select_project_row(p: &mut JiraPlugin, row: ProjRow, reveal: bool) {
    let path = project_path(p, &row);
    p.section_rows[1] = project_rows(p).iter().position(|candidate| *candidate == row).unwrap_or(0);
    p.project_row_index.set(p.section_rows[1]);
    let mut nav = p.project_tree.borrow_mut();
    if reveal {
        for depth in 1..path.len() {
            nav.open(path[..depth].to_vec());
        }
    }
    nav.select(path);
}

pub fn projects_view(p: &JiraPlugin, width: u16, now: i64) -> v::SectionView {
    let rows: Vec<v::TreeRow> = p.cfg.sites.iter().enumerate().map(|(i, site)| {
        let mut root = v::TreeRow::new(crate::screen::site_id(&site.name), v::Text::new()
            .push(v::Seg::icon(Icon::Site))
            .push(v::Seg::styled(site.name.clone(), format!("jira.{} {}", site.color, p.cfg.style_for("site"))))
            .push(v::Seg::new(format!("  {}", site.host()), v::Ink::Muted)))
            .stripe(format!("jira.{}", site.color));
        let mut add = v::kit::add_line("watch a project", "its key, or a link to it or a ticket", width.saturating_sub(6));
        if let Some(first) = add.0.first_mut() { first.text = "+ ".into(); }
        root = root.child(v::TreeRow::new("add", add));
        root = root.child(v::TreeRow::new("colour", connection::field_text(
            v::Text::new(), "colour", v::Text::new()
                .push(v::Seg::styled("██ ", format!("jira.{}", site.color)))
                .push(v::Seg::new(site.color.clone(), v::Ink::Primary)),
        )));
        for project in &site.projects {
            let mut row = v::TreeRow::new(project.key.clone(), v::Text::new()
                .push(v::Seg::icon(Icon::Folder))
                .push(v::Seg::styled(radar_core::ui::pad(&project.key, 8), p.cfg.style_for("key")))
                .push(v::Seg::new(project_name(p, i, &project.key).unwrap_or_default(), v::Ink::Secondary)))
                .detail(v::Text::of(
                    if project.ac_field.is_empty() { "   acceptance criteria: not set".into() }
                    else { format!("   acceptance criteria: {}", project.ac_field) }, v::Ink::Muted,
                ));
            if let Some(mark) = check(p, &CheckTarget::project(&site.name, &project.key), now).mark_text() {
                row = row.cell("check", mark);
            }
            root = root.child(row);
        }
        root
    }).collect();
    let state = project_state(p);
    let editing = match &p.edit {
        Some((Target::ProjectAdd(i), input)) => Some(Box::new(v::TreeEditing {
            path: project_path(p, &ProjRow::Add(*i)), at: 2,
            text: input.value(), caret: input.cursor(),
        })),
        _ => None,
    };
    let detail = match proj_row(p) {
        Some(ProjRow::Site(i)) => vec![v::Text::plain(format!("The projects watched on {}. Views that say \"in your watched projects\" ask about these.", p.cfg.sites[i].name))],
        Some(ProjRow::Colour(i)) => {
            let site = &p.cfg.sites[i];
            let mut d = connection::DetailText::new("The site's colour: its stripe in the tree and its name in breadcrumbs.");
            d.accepts = Some("one of the site colours".into());
            d.now = Some(site.color.clone());
            d.config = Some("[[jira.site]] color".into());
            d.text()
        }
        Some(ProjRow::Add(i)) => {
            let mut d = Vec::new();
            if let Some((target, input)) = &p.edit {
                d.extend([v::kit::heading("as you type"), v::kit::as_you_type(validate(p, target, &input.value()).as_deref().map_err(String::as_str)),
                    v::Text::of("⏎ keeps it · esc cancels", v::Ink::Muted), v::Text::new()]);
            }
            d.extend([v::Text::plain(format!("Watch another project on {}.", p.cfg.sites[i].name)),
                v::Text::new(), v::kit::heading("paste or type any of"),
                v::Text::of("  ABC", v::Ink::Secondary),
                v::Text::of("  a ticket's key or link: ABC-12", v::Ink::Secondary),
                v::Text::of("  a board or project link", v::Ink::Secondary)]);
            d
        }
        Some(ProjRow::Project(i, j)) => {
            let site = &p.cfg.sites[i];
            let project = &site.projects[j];
            vec![v::kit::heading("checked"),
                check(p, &CheckTarget::project(&site.name, &project.key), now).text("not checked yet", "checking with Jira…"),
                v::Text::new(), v::Text::plain(format!("{} on {}: watched.", project.key, site.name)),
                v::Text::new(), v::kit::labelled("name", 12, v::Text::plain(project_name(p, i, &project.key).unwrap_or_else(|| "not known until checked".into()))),
                v::Text::new(), v::kit::heading("acceptance criteria"),
                v::Text::of(if project.ac_field.is_empty() {
                    "No field set: tickets show no acceptance criteria. The menu asks Jira which fields are named like it, and how many tickets fill each in.".into()
                } else { format!("Read from {}.", project.ac_field) }, v::Ink::Secondary),
                v::Text::new(), v::Text::of("config: [[jira.site.project]] key, ac_field", v::Ink::Muted)]
        }
        None => Vec::new(),
    };
    v::SectionView {
        version: v::VERSION,
        scope: "projects".into(),
        body: v::Node::Tree(v::Tree {
            rows, selected: state.selected, open: state.open,
            columns: vec![v::ColumnSpec::new("check", 10, 0)],
            name_width: width.saturating_sub(12),
            look: Some(v::TreeLook {
                lines: p.cfg.style_for("lines"), band: p.cfg.style_for("band"),
                heading: p.cfg.style_for("heading"), band_level: 2,
            }),
            note: p.cfg.sites.is_empty().then(|| v::Note { state: v::State::Empty,
                text: "Add a site first, under connection; then watch its projects here.".into() }),
            editing,
            ..v::Tree::default()
        }),
        detail_title: detail_title(p, "projects"), detail,
        detail_blocks: vec![],
        actions: actions(p, "projects"),
    }
}

pub fn look_intent(p: &mut JiraPlugin, intent: &v::Intent) -> Outcome {
    match intent {
        v::Intent::Select { target } | v::Intent::Context { target } => click(p, "look", &target.0),
        v::Intent::Activate { target } => {
            click(p, "look", &target.0);
            key(p, "look", radar_core::view_tui::key_of(v::Chord::new(v::Key::Enter)))
        }
        v::Intent::Key { .. } | v::Intent::Invoke { .. } => intent.chord().map_or(Outcome::Ignored, |chord| {
            key(p, "look", radar_core::view_tui::key_of(chord))
        }),
        _ => Outcome::Ignored,
    }
}

pub fn views_intent(p: &mut JiraPlugin, intent: &v::Intent) -> Outcome {
    match intent {
        v::Intent::Select { target } | v::Intent::Context { target } => click(p, "views", target.as_str()),
        v::Intent::Activate { target } => {
            click(p, "views", target.as_str());
            key(p, "views", radar_core::view_tui::key_of(v::Chord::new(v::Key::Enter)))
        }
        v::Intent::Key { .. } | v::Intent::Invoke { .. } => intent.chord().map_or(Outcome::Ignored, |chord| {
            key(p, "views", radar_core::view_tui::key_of(chord))
        }),
        _ => Outcome::Ignored,
    }
}

pub fn connection_intent(p: &mut JiraPlugin, intent: &v::Intent) -> Outcome {
    // A tree row's target is its path; the row itself is the last id (`row:<n>`).
    let row = |target: &v::Target| v::TreeRow::path_of(target).pop().unwrap_or_default();
    match intent {
        v::Intent::Select { target } | v::Intent::Context { target } => click(p, "connection", &row(target)),
        v::Intent::Activate { target } => {
            click(p, "connection", &row(target));
            key(p, "connection", radar_core::view_tui::key_of(v::Chord::new(v::Key::Enter)))
        }
        v::Intent::Key { .. } | v::Intent::Invoke { .. } => intent.chord().map_or(Outcome::Ignored, |chord| {
            key(p, "connection", radar_core::view_tui::key_of(chord))
        }),
        _ => Outcome::Ignored,
    }
}

pub fn projects_intent(p: &mut JiraPlugin, intent: &v::Intent) -> Outcome {
    let select = |p: &mut JiraPlugin, target: &v::Target| {
        if p.edit.is_some() { return false; }
        let tree = project_state(p);
        let path = v::TreeRow::path_of(target);
        if !tree.selectable(&path) { return false; }
        if let Some(row) = project_rows(p).into_iter().find(|row| project_path(p, row) == path) {
            select_project_row(p, row, false);
            return true;
        }
        false
    };
    match intent {
        v::Intent::Select { target } | v::Intent::Context { target } => {
            if !select(p, target) && matches!(intent, v::Intent::Context { .. }) {
                return Outcome::Ignored;
            }
            Outcome::done()
        }
        v::Intent::Fold { target, .. } => {
            if p.edit.is_none() {
                let tree = project_state(p);
                let path = v::TreeRow::path_of(target);
                if tree.row(&path).is_some_and(|row| !row.children.is_empty()) {
                    p.project_tree.borrow_mut().apply(intent);
                    if let Some(row) = proj_row(p) {
                        select_project_row(p, row, false);
                    }
                }
            }
            Outcome::done()
        }
        v::Intent::Activate { target } => {
            if select(p, target) {
                key(p, "projects", radar_core::view_tui::key_of(v::Chord::new(v::Key::Enter)))
            } else {
                Outcome::done()
            }
        }
        v::Intent::Key { .. } | v::Intent::Invoke { .. } => intent.chord().map_or(Outcome::Ignored, |chord| {
            key(p, "projects", radar_core::view_tui::key_of(chord))
        }),
        _ => Outcome::Ignored,
    }
}

pub fn detail_title(p: &JiraPlugin, section: &str) -> String {
    match section {
        "connection" => match site_row(p) {
            SiteRow::Add => "add a site".into(),
            SiteRow::Offer(k) => p
                .offers()
                .get(k)
                .map_or_else(|| "found".into(), |f| found(p, f).title()),
            SiteRow::Site(i) => p.cfg.sites[i].name.clone(),
            SiteRow::Field(_, f) => f.label().into(),
            SiteRow::Refresh => connection::REFRESH.into(),
            SiteRow::OnStart => ON_START.into(),
        },
        "projects" => match proj_row(p) {
            Some(ProjRow::Site(i)) => p.cfg.sites[i].name.clone(),
            Some(ProjRow::Add(_)) => "watch a project".into(),
            Some(ProjRow::Colour(_)) => "colour".into(),
            Some(ProjRow::Project(i, j)) => p.cfg.sites[i].projects[j].key.clone(),
            None => "projects".into(),
        },
        "views" => view_index(p).map_or_else(|| "new view".into(), |i| p.cfg.views[i].name.clone()),
        _ => selected_look(p).map_or_else(|| "look".into(), |(w, _)| w.to_string()),
    }
}

// --- menus ---------------------------------------------------------------------

fn row_menu(p: &JiraPlugin, section: &str) -> Option<Menu> {
    match section {
        "connection" => match site_row(p) {
            SiteRow::Add => None,
            SiteRow::Offer(k) => {
                let q = found(p, p.offers().get(k)?);
                Some(Menu::new(format!("offer:{k}"), q.question(), vec![MenuItem::new('y', "yes, use it", q.does, "use")]))
            }
            SiteRow::Site(i) => Some(Menu::new(
                format!("site:{i}"),
                p.cfg.sites[i].name.clone(),
                vec![
                    MenuItem::new('c', "check the connection", "ask Jira whether it works and who the token is", "check"),
                    MenuItem::new('n', "rename", "the name shown in the tree", "rename"),
                    MenuItem::new('a', "add a site", "paste another site's link", "add"),
                    MenuItem::new('d', "disconnect site…", "remove this site", "remove"),
                ],
            )),
            // The projects section's own menu for the site.
            SiteRow::Field(i, Field::Projects) => Some(Menu::new(
                format!("psite:{i}"),
                format!("projects on {}", p.cfg.sites[i].name),
                vec![
                    MenuItem::new('a', "watch a project", "type its key or paste a link", "add"),
                    MenuItem::new('p', "pick from Jira…", "the projects you looked at lately", "recent"),
                ],
            )),
            SiteRow::Field(i, f) => {
                let mut items = Vec::new();
                match f {
                    Field::SignedIn | Field::Check => {}
                    _ => items.push(MenuItem::new('e', "change it", f.accepts().unwrap_or_default(), "edit")),
                }
                items.push(MenuItem::new('c', "check the connection", "ask Jira whether it works and who the token is", "check"));
                Some(Menu::new(format!("field:{i}:{}", f.label()), f.label(), items))
            }
            SiteRow::Refresh => Some(Menu::new("refresh", connection::REFRESH, vec![MenuItem::new('e', "change it", connection::refresh_accepts(30), "edit")])),
            SiteRow::OnStart => Some(Menu::new(
                "on-start",
                ON_START,
                vec![
                    MenuItem::new('a', "ask", "ask to assign the ticket to you and move it", "ask"),
                    MenuItem::new('o', "off", "leave the ticket alone", "off"),
                ],
            )),
        },
        "projects" => match proj_row(p)? {
            ProjRow::Site(i) | ProjRow::Add(i) => Some(Menu::new(
                format!("psite:{i}"),
                format!("projects on {}", p.cfg.sites[i].name),
                vec![
                    MenuItem::new('a', "watch a project", "type its key or paste a link", "add"),
                    MenuItem::new('p', "pick from Jira…", "the projects you looked at lately", "recent"),
                ],
            )),
            ProjRow::Colour(i) => Some(Menu::new(
                format!("psite:{i}"),
                format!("colour of {}", p.cfg.sites[i].name),
                vec![MenuItem::new('o', "colour…", "pick the site's colour", "colour")],
            )),
            ProjRow::Project(i, j) => {
                let key = &p.cfg.sites[i].projects[j].key;
                Some(Menu::new(
                    format!("project:{i}:{j}"),
                    format!("{key} on {}", p.cfg.sites[i].name),
                    vec![
                        MenuItem::new('f', "acceptance-criteria field…", "pick among the fields named like it", "ac"),
                        MenuItem::new('c', "check it", "ask Jira whether it exists and can be seen", "check"),
                        MenuItem::new('a', "watch another project", "type its key or paste a link", "add"),
                        MenuItem::new('d', "drop watched project…", "views stop asking about it", "remove"),
                    ],
                ))
            }
        },
        "views" => {
            let i = view_index(p)?;
            let v = &p.cfg.views[i];
            let mut items = vec![MenuItem::new('a', "add a condition…", "narrow what it shows", "cond")];
            if !v.when.is_empty() && v.jql.trim().is_empty() {
                items.push(MenuItem::new('-', "remove a condition…", "widen what it shows", "uncond"));
            }
            items.push(MenuItem::new(
                't',
                if v.jql.trim().is_empty() { "write it as JQL" } else { "edit the JQL" },
                "Jira checks it as you type",
                "jql",
            ));
            if !v.jql.trim().is_empty() {
                items.push(MenuItem::new('c', "back to conditions", "drop the JQL; the conditions in words return", "unjql"));
            }
            if p.cfg.sites.len() > 1 {
                items.push(MenuItem::new(
                    's',
                    "which sites…",
                    if v.site.is_empty() { "asks every site now".to_string() } else { format!("asks {} only now", v.site) },
                    "sites",
                ));
            }
            items.push(MenuItem::new('r', "rename", "change the name in the nav pane", "rename"));
            if i > 0 {
                items.push(MenuItem::new('<', "move up", "earlier in the nav pane", "up"));
            }
            if i + 1 < p.cfg.views.len() {
                items.push(MenuItem::new('>', "move down", "later in the nav pane", "down"));
            }
            items.extend([
                MenuItem::new('n', "new view", "starts from your open tickets", "new"),
                MenuItem::new('d', "delete…", "remove this view", "delete"),
            ]);
            Some(Menu::new(format!("view:{i}"), v.name.clone(), items))
        }
        _ => {
            let (w, _) = selected_look(p)?;
            let spec = p.cfg.style_for(w);
            let has = |m: &str| spec.split_whitespace().any(|x| x == m);
            let onoff = |m: &str| if has(m) { "on, turn off" } else { "off, turn on" };
            let mut items = Vec::new();
            if w != "site" {
                items.push(MenuItem::new('c', "colour…", "pick from the theme's colours", "colour"));
            }
            if !is_background(w) {
                items.extend([
                    MenuItem::new('o', "bold", onoff("bold"), "bold"),
                    MenuItem::new('i', "italic", onoff("italic"), "italic"),
                    MenuItem::new('u', "underline", onoff("underline"), "underline"),
                    MenuItem::new('d', "dim", onoff("dim"), "dim"),
                ]);
            }
            if default_styles().get(w).is_some_and(|d| *d != spec) {
                items.push(MenuItem::new('r', "restore default", look_summary(&default_styles()[w], w), "reset"));
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
            let hint = if name == current { format!("in use · {}", info.meaning) } else { info.meaning };
            MenuItem::pick(info.label, hint, name).sample(name)
        })
        .collect();
    Menu::new(id, title, items)
}

fn site_colour_menu(p: &JiraPlugin, i: usize) -> Effect {
    let items = SITE_COLORS
        .iter()
        .map(|c| {
            let hint = if p.cfg.sites[i].color == *c { "in use" } else { "" };
            MenuItem::pick(*c, hint, *c).sample(format!("jira.{c}"))
        })
        .collect();
    Effect::Menu(Menu::new(format!("site-colour:{i}"), "site colour", items))
}

/// The conditions a view can be given, by menu.
fn cond_menu(i: usize) -> Effect {
    Effect::Menu(Menu::new(
        format!("cond:{i}"),
        "add a condition",
        vec![
            MenuItem::new('m', "assigned to me", "or to anyone but me", "me"),
            MenuItem::new('u', "unassigned", "or to someone", "none"),
            MenuItem::new('r', "reported by me", "or by someone else", "reporter"),
            MenuItem::new('s', "by status…", "not done, to do, doing, done", "status"),
            MenuItem::new('p', "in an open sprint", "or not", "sprint"),
            MenuItem::new('w', "in my watched projects", "or outside them", "watched"),
            MenuItem::new('o', "in a project…", "pick a watched project", "project"),
            MenuItem::new('t', "of a type…", "Bug, Story, …", "type"),
            MenuItem::new('l', "with a label…", "pick from labels seen", "label"),
            MenuItem::new('d', "changed lately…", "in the last few days", "updated"),
            MenuItem::new('x', "text contains…", "anywhere in the ticket", "words"),
        ],
    ))
}

/// Include or leave out: `cond` or `-cond`.
fn sign_menu(i: usize, cond: &str) -> Effect {
    let words = view::describe(cond);
    let not = view::describe(&format!("-{cond}"));
    Effect::Menu(Menu::new(
        format!("cond-sign:{i}:{cond}"),
        words.clone(),
        vec![
            MenuItem::new('i', "include only these", format!("tickets {words}"), "include"),
            MenuItem::new('e', "all but these", format!("tickets {not}"), "exclude"),
        ],
    ))
}

fn add_cond(p: &mut JiraPlugin, i: usize, cond: &str) -> Vec<Effect> {
    let Some(v) = p.cfg.views.get_mut(i) else { return vec![] };
    v.when = view::add(&v.when, cond);
    let words = view::describe(cond);
    let mut fx = vec![Effect::Notify(format!("{}: now also {words}", v.name))];
    if i == p.view {
        fx.extend(p.sync_view(false));
    }
    fx
}

fn add_site(p: &mut JiraPlugin, site: Site) -> usize {
    p.cfg.sites.push(site);
    crate::write_context::reconcile(p);
    p.cfg.sites.len() - 1
}

/// The sites section's row for a site's field.
fn field_row(p: &JiraPlugin, i: usize, f: Field) -> usize {
    site_rows(p)
        .iter()
        .position(|r| *r == SiteRow::Field(i, f))
        .unwrap_or(0)
}

pub fn on_menu(p: &mut JiraPlugin, menu: &str, value: &str) -> Vec<Effect> {
    let (kind, arg) = menu.split_once(':').unwrap_or((menu, ""));
    let idx = |s: &str| s.split(':').next().and_then(|n| n.parse::<usize>().ok());
    let sites = p.cfg.sites.len();
    match (kind, value) {
        ("ask-check", "yes") => {
            let Some(target) = CheckTarget::from_menu(arg) else { return vec![] };
            let Some(i) = p.cfg.sites.iter().position(|site| site.name == target.site()) else { return vec![] };
            match target {
                CheckTarget::Site(_) => check_site(p, i),
                CheckTarget::Project { project, .. } => check_project(p, i, &project),
            }
        }
        ("ask-check", _) => vec![],
        ("offer", "use") => {
            let Some(f) = idx(arg).and_then(|k| p.offers().get(k).cloned()) else {
                return vec![];
            };
            use_offer(p, &f)
        }
        ("site", v) => {
            let Some(i) = idx(arg).filter(|i| *i < sites) else { return vec![] };
            match v {
                "check" => check_site(p, i),
                "rename" => {
                    p.edit = Some((Target::SiteName(i), TextInput::new(&p.cfg.sites[i].name)));
                    vec![]
                }
                "add" => {
                    p.section_rows[0] = p.offers().len();
                    p.edit = Some((Target::SiteAdd, TextInput::new("")));
                    vec![]
                }
                "remove" => vec![Effect::Menu(Menu::confirm(
                    format!("site-remove:{i}"),
                    format!("stop using {}?", p.cfg.sites[i].name),
                    "stop using it",
                    "keep it",
                )
                .saying("its tickets leave every view"))],
                _ => vec![],
            }
        }
        ("site-colour", c) => {
            if let Some(i) = idx(arg).filter(|i| *i < sites) {
                p.cfg.sites[i].color = c.to_string();
            }
            vec![]
        }
        ("site-remove", "yes") => {
            let Some(i) = idx(arg).filter(|i| *i < sites) else { return vec![] };
            let gone = p.cfg.sites.remove(i);
            crate::write_context::reconcile(p);
            p.data.remove(&gone.name);
            p.section_rows[0] = 0;
            vec![Effect::Notify(format!("stopped using {}", gone.name))]
        }
        ("field", v) => {
            let mut parts = arg.splitn(2, ':');
            let (Some(i), Some(label)) = (parts.next().and_then(|n| n.parse::<usize>().ok()), parts.next()) else {
                return vec![];
            };
            if i >= sites {
                return vec![];
            }
            let Some(f) = Field::ALL.into_iter().find(|f| f.label() == label) else { return vec![] };
            match (v, f) {
                ("check", _) => check_site(p, i),
                ("edit", Field::Url) => {
                    p.edit = Some((Target::SiteUrl(i), TextInput::new(&p.cfg.sites[i].url)));
                    vec![]
                }
                ("edit", Field::Email) => {
                    p.edit = Some((Target::SiteEmail(i), TextInput::new(&p.cfg.sites[i].email)));
                    vec![]
                }
                ("edit", Field::Token) => {
                    // A typed token is not shown again: start from its env: name or empty.
                    let now = &p.cfg.sites[i].token;
                    let start = if now.starts_with("env:") { now.clone() } else { String::new() };
                    p.edit = Some((Target::SiteToken(i), TextInput::new(&start)));
                    vec![]
                }
                _ => vec![],
            }
        }
        ("on-start", v @ ("ask" | "off")) => {
            p.cfg.on_start = if v == "off" { config::OnStart::Off } else { config::OnStart::Ask };
            vec![Effect::Notify("changed".into())]
        }
        ("refresh", "edit") => {
            p.edit = Some((Target::Refresh, TextInput::new(&p.cfg.refresh_secs.to_string())));
            vec![]
        }
        ("psite", v) => {
            let Some(i) = idx(arg).filter(|i| *i < sites) else { return vec![] };
            match v {
                "add" => start_project_add(p, i),
                "recent" => ask_recent(p, i),
                "colour" => vec![site_colour_menu(p, i)],
                _ => vec![],
            }
        }
        ("recent", key) => {
            let Some(i) = idx(arg).filter(|i| *i < sites) else { return vec![] };
            watch(p, i, key)
        }
        ("project", v) => {
            let mut parts = arg.split(':');
            let (Some(i), Some(j)) = (
                parts.next().and_then(|n| n.parse::<usize>().ok()),
                parts.next().and_then(|n| n.parse::<usize>().ok()),
            ) else {
                return vec![];
            };
            let Some(key) = p.cfg.sites.get(i).and_then(|s| s.projects.get(j)).map(|pr| pr.key.clone()) else {
                return vec![];
            };
            match v {
                "ac" => ask_ac_fields(p, i, &key),
                "check" => check_project(p, i, &key),
                "add" => start_project_add(p, i),
                "remove" => vec![Effect::Menu(Menu::confirm(
                    format!("project-remove:{i}:{j}"),
                    format!("stop watching {key} on {}?", p.cfg.sites[i].name),
                    "stop watching it",
                    "keep it",
                )
                .saying("views stop asking about it"))],
                _ => vec![],
            }
        }
        ("project-remove", "yes") => {
            let mut parts = arg.split(':');
            if let (Some(i), Some(j)) = (
                parts.next().and_then(|n| n.parse::<usize>().ok()),
                parts.next().and_then(|n| n.parse::<usize>().ok()),
            ) && let Some(s) = p.cfg.sites.get_mut(i)
                && j < s.projects.len()
            {
                let gone = s.projects.remove(j);
                crate::write_context::reconcile(p);
                let row = if j > 0 { ProjRow::Project(i, j - 1) } else { ProjRow::Colour(i) };
                select_project_row(p, row, true);
                return vec![Effect::Notify(format!("stopped watching {}", gone.key))];
            }
            vec![]
        }
        ("ac", id) => {
            let Some((i, project)) = arg.split_once(':') else { return vec![] };
            let Ok(i) = i.parse::<usize>() else { return vec![] };
            if let Some(pr) = p.cfg.sites.get_mut(i).and_then(|s| s.projects.iter_mut().find(|pr| pr.key == project)) {
                pr.ac_field = id.to_string();
                // Tickets carry it from the next fetch.
                p.sync.started.clear();
                return vec![Effect::Notify(if id.is_empty() {
                    format!("{project} shows no acceptance criteria")
                } else {
                    format!("{project}'s acceptance criteria from {id}")
                })];
            }
            vec![]
        }
        ("view", v) => {
            let Some(i) = idx(arg).filter(|i| *i < p.cfg.views.len()) else { return vec![] };
            match v {
                "cond" => vec![cond_menu(i)],
                "uncond" => {
                    let items = p.cfg.views[i]
                        .when
                        .iter()
                        .enumerate()
                        .map(|(n, c)| MenuItem::pick(view::describe(c), "", n.to_string()))
                        .collect();
                    vec![Effect::Menu(Menu::new(format!("cond-remove:{i}"), "remove which?", items))]
                }
                "jql" => {
                    let start = if p.cfg.views[i].jql.trim().is_empty() {
                        // Start from what the conditions ask the first site.
                        p.jql_for(0, &p.cfg.views[i].clone()).unwrap_or_default()
                    } else {
                        p.cfg.views[i].jql.clone()
                    };
                    p.typed_at = Some(std::time::Instant::now());
                    p.edit = Some((Target::ViewJql(i), TextInput::new(&start)));
                    vec![]
                }
                "unjql" => {
                    p.cfg.views[i].jql.clear();
                    vec![Effect::Notify("back to conditions in words".into())]
                }
                "rename" => {
                    p.edit = Some((Target::ViewName(i), TextInput::new(&p.cfg.views[i].name)));
                    vec![]
                }
                "connection" => {
                    let now = p.cfg.views[i].site.clone();
                    let mut items = vec![MenuItem::new(
                        'e',
                        "every site",
                        if now.is_empty() { "asked now" } else { "" },
                        "",
                    )];
                    items.extend(p.cfg.sites.iter().map(|s| {
                        MenuItem::pick(format!("{} only", s.name), if now == s.name { "asked now" } else { "" }, s.name.clone())
                    }));
                    vec![Effect::Menu(Menu::new(format!("view-site:{i}"), "ask which sites?", items))]
                }
                "up" if i > 0 => {
                    p.cfg.views.swap(i, i - 1);
                    p.section_rows[2] = i;
                    vec![]
                }
                "down" if i + 1 < p.cfg.views.len() => {
                    p.cfg.views.swap(i, i + 1);
                    p.section_rows[2] = i + 2;
                    vec![]
                }
                "new" => {
                    p.section_rows[2] = 0;
                    p.edit = Some((Target::ViewAdd, TextInput::new("")));
                    vec![]
                }
                "delete" if p.cfg.views.len() > 1 => vec![Effect::Menu(Menu::confirm(
                    format!("view-delete:{i}"),
                    format!("delete the view {}?", p.cfg.views[i].name),
                    "delete it",
                    "keep it",
                )
                .saying("it leaves the nav pane"))],
                "delete" => vec![Effect::Notify("the last view stays".into())],
                _ => vec![],
            }
        }
        ("view-site", site) => {
            let Some(i) = idx(arg).filter(|i| *i < p.cfg.views.len()) else { return vec![] };
            p.cfg.views[i].site = site.to_string();
            if i == p.view {
                return p.sync_view(false);
            }
            vec![]
        }
        ("view-delete", "yes") => {
            if let Some(i) = idx(arg).filter(|i| *i < p.cfg.views.len()) {
                let v = p.cfg.views.remove(i);
                p.section_rows[2] = i.max(1).min(p.cfg.views.len());
                return vec![Effect::Notify(format!("deleted {}", v.name))];
            }
            vec![]
        }
        ("cond", what) => {
            let Some(i) = idx(arg) else { return vec![] };
            match what {
                "me" => vec![sign_menu(i, "assignee:me")],
                "none" => vec![sign_menu(i, "assignee:none")],
                "reporter" => vec![sign_menu(i, "reporter:me")],
                "sprint" => vec![sign_menu(i, "sprint:open")],
                "watched" => vec![sign_menu(i, "project:watched")],
                "status" => vec![Effect::Menu(Menu::new(
                    format!("cond-pick:{i}"),
                    "which status?",
                    ["status:open", "status:todo", "status:doing", "status:done"]
                        .iter()
                        .map(|c| MenuItem::pick(view::describe(c), "", *c))
                        .collect(),
                ))],
                "updated" => vec![Effect::Menu(Menu::new(
                    format!("cond-pick:{i}"),
                    "changed in the last",
                    [1, 3, 7, 14, 30, 90]
                        .iter()
                        .map(|d| {
                            let c = format!("updated:{d}d");
                            MenuItem::pick(view::describe(&c), "", c)
                        })
                        .collect(),
                ))],
                "project" => {
                    let mut keys: Vec<String> = p.cfg.sites.iter().flat_map(|s| s.projects.iter().map(|pr| pr.key.clone())).collect();
                    keys.sort();
                    keys.dedup();
                    if keys.is_empty() {
                        return vec![Effect::Notify("no watched projects yet: settings, projects".into())];
                    }
                    vec![Effect::Menu(Menu::new(
                        format!("cond-signed:{i}"),
                        "which project?",
                        keys.into_iter().map(|k| MenuItem::pick(k.clone(), "", format!("project:{k}"))).collect(),
                    ))]
                }
                "type" | "label" => {
                    let mut seen: Vec<String> = p
                        .data
                        .values()
                        .flat_map(|d| d.tickets.values())
                        .flat_map(|t| if what == "type" { vec![t.kind.clone()] } else { t.labels.clone() })
                        .filter(|s| !s.is_empty())
                        .collect();
                    seen.sort();
                    seen.dedup();
                    let mut items = vec![MenuItem::new('y', "type it…", "a name not listed here", format!("\u{0}{what}"))];
                    items.extend(seen.into_iter().map(|s| MenuItem::pick(s.clone(), "", format!("{what}:{s}"))));
                    vec![Effect::Menu(Menu::new(format!("cond-signed:{i}"), format!("which {what}?"), items))]
                }
                "words" => {
                    p.edit = Some((Target::ViewWords(i), TextInput::new("")));
                    vec![]
                }
                _ => vec![],
            }
        }
        ("cond-pick", c) => {
            let Some(i) = idx(arg) else { return vec![] };
            add_cond(p, i, c)
        }
        ("cond-signed", c) => {
            let Some(i) = idx(arg) else { return vec![] };
            match c {
                "\u{0}type" => {
                    p.edit = Some((Target::ViewType(i), TextInput::new("")));
                    return vec![];
                }
                "\u{0}label" => {
                    p.edit = Some((Target::ViewLabel(i), TextInput::new("")));
                    return vec![];
                }
                _ => {}
            }
            vec![sign_menu(i, c)]
        }
        ("cond-sign", sign) => {
            let Some((i, cond)) = arg.split_once(':') else { return vec![] };
            let Ok(i) = i.parse::<usize>() else { return vec![] };
            let cond = if sign == "exclude" { format!("-{cond}") } else { cond.to_string() };
            add_cond(p, i, &cond)
        }
        ("cond-remove", n) => {
            if let (Some(i), Ok(n)) = (idx(arg), n.parse::<usize>())
                && let Some(v) = p.cfg.views.get_mut(i)
                && n < v.when.len()
            {
                v.when.remove(n);
            }
            vec![]
        }
        ("look", v) => {
            let word = arg.to_string();
            let spec = p.cfg.style_for(&word);
            match v {
                "colour" => {
                    let current = spec.split_whitespace().next().unwrap_or("").to_string();
                    return vec![Effect::Menu(colour_menu(format!("look-colour:{word}"), "colour", &current))];
                }
                "reset" => {
                    if let Some(d) = default_styles().get(word.as_str()) {
                        p.cfg.style.insert(word.clone(), d.clone());
                    }
                }
                m => {
                    let mut words: Vec<String> = spec.split_whitespace().map(String::from).collect();
                    let start = usize::from(word != "site");
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
            let new = std::iter::once(slot).chain(mods).collect::<Vec<_>>().join(" ");
            p.cfg.style.insert(word, new);
            vec![]
        }
        _ => vec![],
    }
}

/// Add the site an offer describes, and check it; when its token was typed
/// into a file, ask for the variable that holds it instead.
fn use_offer(p: &mut JiraPlugin, f: &Offer) -> Vec<Effect> {
    let mut site = f.site.clone();
    site.color = SITE_COLORS[p.cfg.sites.len() % SITE_COLORS.len()].to_string();
    if p.cfg.sites.iter().any(|s| s.name == site.name) {
        site.name = format!("{}{}", site.name, p.cfg.sites.len() + 1);
    }
    let i = add_site(p, site);
    let name = p.cfg.sites[i].name.clone();
    if f.token_typed {
        p.section_rows[0] = field_row(p, i, Field::Token);
        p.edit = Some((Target::SiteToken(i), TextInput::new(&format!("env:{}", config::DEFAULT_TOKEN))));
        return vec![Effect::Notify(format!(
            "added {name} · its token is typed into the file and radar does not copy it: give the env: name it is exported as"
        ))];
    }
    p.section_rows[0] = site_rows(p).iter().position(|r| *r == SiteRow::Site(i)).unwrap_or(0);
    let mut fx = vec![Effect::Notify(format!("added {name}, {}", f.source))];
    fx.extend(check_site(p, i));
    fx.extend(p.sync_view(true));
    fx
}

fn start_project_add(p: &mut JiraPlugin, i: usize) -> Vec<Effect> {
    select_project_row(p, ProjRow::Add(i), true);
    p.edit = Some((Target::ProjectAdd(i), TextInput::new("")));
    vec![]
}

/// Watch a project on site `i`, then offer to check it and to pick its
/// acceptance-criteria field.
fn watch(p: &mut JiraPlugin, i: usize, key: &str) -> Vec<Effect> {
    if !config::is_project_key(key) || p.cfg.sites[i].watches(key) {
        return vec![];
    }
    p.cfg.sites[i].projects.push(Project {
        key: key.to_string(),
        ac_field: String::new(),
    });
    crate::write_context::reconcile(p);
    let j = p.cfg.sites[i].projects.len() - 1;
    select_project_row(p, ProjRow::Project(i, j), true);
    let name = p.cfg.sites[i].name.clone();
    let mut fx = p.sync_view(true);
    fx.push(ask_to_check(&CheckTarget::project(&name, key), &format!("{key} on {name}")));
    fx
}

// --- keys ------------------------------------------------------------------------

pub fn key(p: &mut JiraPlugin, section: &str, key: KeyEvent) -> Outcome {
    if let Some((target, mut input)) = p.edit.take() {
        return match input.key(key) {
            Edit::Cancel => Outcome::done(),
            Edit::Typing => {
                p.typed_at = Some(std::time::Instant::now());
                p.edit = Some((target, input));
                Outcome::done()
            }
            Edit::Submit(text) => match validate(p, &target, &text) {
                Err(why) => {
                    let mut fx = vec![Effect::error(None, format!("not yet: {why}"))];
                    // JQL Jira has not seen yet: ask now.
                    if matches!(target, Target::ViewJql(_)) && !p.jql_checks.contains_key(text.trim()) && !text.trim().is_empty() {
                        fx.extend(ask_jira_about(p, text.trim().to_string()));
                    }
                    p.edit = Some((target, input));
                    Outcome::Handled(fx)
                }
                Ok(_) => Outcome::Handled(commit(p, target, text.trim().to_string())),
            },
        };
    }
    let at = index(section);
    if let Some(mv) = Move::from_key(&key, p.section_scroll[at].height()) {
        if section == "projects" {
            let tree = project_state(p);
            let shown = tree.visible();
            let at = shown.iter().position(|(path, _)| *path == tree.selected).unwrap_or(0);
            let next = select(shown.len(), at, mv, |_| true);
            if let Some((path, _)) = shown.get(next)
                && let Some(row) = project_rows(p).into_iter().find(|row| project_path(p, row) == *path)
            {
                select_project_row(p, row, false);
            }
            return Outcome::done();
        }
        // Pure labels in look remain nonselectable.
        let can = |row: usize| section != "look" || !matches!(look_rows().get(row), Some(LookRow::Header(_)));
        p.section_rows[at] = select(row_count(p, section), p.section_rows[at], mv, can);
        return Outcome::done();
    }
    if key.code == KeyCode::Enter {
        // ⏎ on an add row starts typing; on an offer it uses it.
        match (section, site_row(p), proj_row(p)) {
            ("connection", SiteRow::Add, _) => {
                p.edit = Some((Target::SiteAdd, TextInput::new("")));
                return Outcome::done();
            }
            ("connection", SiteRow::Offer(k), _) => {
                return Outcome::Handled(on_menu(p, &format!("offer:{k}"), "use"));
            }
            ("connection", SiteRow::Field(i, Field::Check), _) => return Outcome::Handled(check_site(p, i)),
            ("connection", SiteRow::Field(i, Field::Projects), _) | ("projects", _, Some(ProjRow::Add(i))) => {
                return Outcome::Handled(start_project_add(p, i));
            }
            ("projects", _, Some(ProjRow::Colour(i))) => return Outcome::Handled(vec![site_colour_menu(p, i)]),
            ("views", _, _) if view_index(p).is_none() => {
                p.edit = Some((Target::ViewAdd, TextInput::new("")));
                return Outcome::done();
            }
            _ => {}
        }
        // As in every part's connection: on a site's field, refresh or
        // idle, ⏎ goes straight to changing it (or checking, or the colours).
        if section == "connection"
            && matches!(site_row(p), SiteRow::Field(..) | SiteRow::Refresh)
            && let Some(menu) = row_menu(p, section)
            && let Some(first) = menu.items.first()
            && (menu.items.len() == 1 || first.key == Some('e'))
        {
            let value = first.value.clone();
            return Outcome::Handled(on_menu(p, &menu.id, &value));
        }
        return Outcome::Handled(row_menu(p, section).map(Effect::Menu).into_iter().collect());
    }
    if let KeyCode::Char(c) = key.code
        && let Some(menu) = row_menu(p, section)
        && let Some(item) = menu.items.iter().find(|i| i.key == Some(c))
    {
        let value = item.value.clone();
        return Outcome::Handled(on_menu(p, &menu.id, &value));
    }
    match (section, key.code) {
        // Atlassian's API token page: `o` opens it, `y` copies its link.
        ("connection", KeyCode::Char('o')) => Outcome::Handled(vec![Effect::Open(crate::setup::TOKEN_PAGE.into())]),
        ("connection", KeyCode::Char('y')) => Outcome::Handled(vec![
            Effect::Copy(crate::setup::TOKEN_PAGE.into()),
            Effect::Notify("copied the API token page's link".into()),
        ]),
        ("connection", KeyCode::Char('a')) => {
            // The add row, under what was found.
            p.section_rows[0] = p.offers().len();
            p.edit = Some((Target::SiteAdd, TextInput::new("")));
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

fn commit(p: &mut JiraPlugin, target: Target, value: String) -> Vec<Effect> {
    let site_name = |p: &JiraPlugin, i: usize| p.cfg.sites[i].name.clone();
    match target {
        Target::SiteAdd => {
            let url = if value.starts_with("env:") { value.clone() } else { config::site_url(&value) };
            let host = config::host_of(&url);
            let mut name: String = host.split('.').next().unwrap_or("jira").chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
            if name.is_empty() || p.cfg.sites.iter().any(|s| s.name == name) {
                name = format!("site{}", p.cfg.sites.len() + 1);
            }
            let color = SITE_COLORS[p.cfg.sites.len() % SITE_COLORS.len()].to_string();
            let i = add_site(
                p,
                Site {
                    name: name.clone(),
                    url,
                    // The team's names, as its tools read them.
                    email: format!("env:{}", config::DEFAULT_EMAIL),
                    token: format!("env:{}", config::DEFAULT_TOKEN),
                    color,
                    projects: config::default_projects(&p.vars()),
                    from: String::new(),
                },
            );
            // Its email next, starting from the team's variable.
            p.section_rows[0] = field_row(p, i, Field::Email);
            p.edit = Some((Target::SiteEmail(i), TextInput::new(&format!("env:{}", config::DEFAULT_EMAIL))));
            vec![Effect::Notify(format!(
                "added {name} · its email and token read {} and {}; ⏎ keeps them",
                config::DEFAULT_EMAIL,
                config::DEFAULT_TOKEN
            ))]
        }
        Target::SiteName(i) => {
            let old = site_name(p, i);
            if let Some(d) = p.data.remove(&old) {
                p.data.insert(value.clone(), d);
            }
            p.cfg.sites[i].name = value;
            crate::write_context::reconcile(p);
            p.sync.started.clear();
            vec![]
        }
        Target::SiteUrl(i) | Target::SiteEmail(i) | Target::SiteToken(i) => {
            match target {
                Target::SiteUrl(_) => {
                    p.cfg.sites[i].url = if value.starts_with("env:") { value } else { config::site_url(&value) }
                }
                Target::SiteEmail(_) => p.cfg.sites[i].email = value,
                _ => p.cfg.sites[i].token = value,
            }
            crate::write_context::reconcile(p);
            let name = site_name(p, i);
            p.checks.remove(&CheckTarget::Site(name.clone()));
            p.data.remove(&name);
            p.sync.started.clear();
            vec![ask_to_check(&CheckTarget::Site(name.clone()), &name)]
        }
        Target::Refresh => {
            p.cfg.refresh_secs = value.trim_end_matches('s').parse().unwrap_or(p.cfg.refresh_secs);
            vec![Effect::Notify("changed".into())]
        }
        Target::ProjectAdd(i) => watch(p, i, &config::project_key_of(&value)),
        Target::ViewAdd => {
            p.cfg.views.push(View {
                name: value,
                when: vec!["assignee:me".into(), "status:open".into()],
                jql: String::new(),
                site: String::new(),
            });
            p.section_rows[2] = p.cfg.views.len();
            vec![Effect::Notify("new view: your open tickets · a adds a condition".into())]
        }
        Target::ViewName(i) => {
            p.cfg.views[i].name = value;
            vec![]
        }
        Target::ViewJql(i) => {
            p.cfg.views[i].jql = value;
            if i == p.view {
                return p.sync_view(true);
            }
            vec![Effect::Notify("kept".into())]
        }
        Target::ViewWords(i) => add_cond(p, i, &format!("text:{value}")),
        Target::ViewType(i) => vec![sign_menu(i, &format!("type:{value}"))],
        Target::ViewLabel(i) => vec![sign_menu(i, &format!("label:{value}"))],
    }
}

pub fn click(p: &mut JiraPlugin, section: &str, id: &str) -> Outcome {
    if p.edit.is_some() {
        return Outcome::done();
    }
    if let Some(i) = id.strip_prefix("row:").and_then(|n| n.parse::<usize>().ok()) {
        p.section_rows[index(section)] = i;
    }
    Outcome::done()
}

pub fn hints(p: &JiraPlugin) -> Vec<KeyHint<'static>> {
    if p.edit.is_some() {
        return vec![KeyHint::new("⏎", "keep"), KeyHint::new("esc", "cancel")];
    }
    vec![]
}

/// The actions of a section's `x` menu as a view lists them, each under its
/// stable id (`jira.<section>.<verb>`), then the kept-text row every section has.
pub(crate) fn actions(p: &JiraPlugin, section: &str) -> Vec<v::ActionSpec> {
    let kept = (format!("jira.{section}.kept_text"), Binding::new("ctrl-r", "kept Jira text…"));
    rows(p, section).into_iter().chain(std::iter::once(kept)).map(|(id, b)| v::ActionSpec::named(id, &b)).collect()
}

/// The rows of a section's `x` menu, each with its stable id. The verbs below
/// are written out here and never made from a label; a menu item's id is its
/// own `value`, which `on_menu` already takes. A heading has no id.
pub(crate) fn rows(p: &JiraPlugin, section: &str) -> Vec<(String, Binding)> {
    let id = |verb: &str| format!("jira.{section}.{verb}");
    let heading = |b: Binding| (String::new(), b);
    match (section, site_row(p), proj_row(p)) {
        ("connection", SiteRow::Add, _) => {
            return vec![
                heading(Binding::heading("add a site")),
                (id("add_site"), Binding::new("⏎", "type or paste its link")),
            ];
        }
        ("connection", SiteRow::Offer(k), _) => {
            let Some(o) = p.offers().get(k).cloned() else { return vec![] };
            return vec![heading(Binding::heading(found(p, &o).title())), (id("use_offer"), Binding::new("⏎", "use it"))];
        }
        ("connection", SiteRow::Field(_, Field::Check), _) => {
            return vec![
                heading(Binding::heading(connection::CHECK)),
                (id("check"), Binding::new("⏎", "ask Jira now")),
                (id("token_page"), Binding::new("o", "open Atlassian's API token page")),
                (id("copy_token_link"), Binding::new("y", "copy the API token page's link")),
            ];
        }
        ("connection", SiteRow::Field(i, Field::Projects), _) => {
            return vec![
                heading(Binding::heading(format!("projects on {}", p.cfg.sites[i].name))),
                (id("watch"), Binding::new("⏎", "watch a project")),
                (id("recent"), Binding::new("p", "pick from Jira…")),
            ];
        }
        ("projects", _, Some(ProjRow::Add(i))) => {
            return vec![
                heading(Binding::heading(format!("watch a project on {}", p.cfg.sites[i].name))),
                (id("add"), Binding::new("⏎", "type its key or paste a link")),
                (id("recent"), Binding::new("p", "pick from Jira…")),
            ];
        }
        ("views", _, _) if view_index(p).is_none() => {
            return vec![heading(Binding::heading("new view")), (id("new"), Binding::new("⏎", "type its name"))];
        }
        _ => {}
    }
    let Some(menu) = row_menu(p, section) else {
        return vec![];
    };
    let mut b = vec![heading(Binding::heading(menu.title))];
    b.extend(menu.items.iter().filter_map(|i| Some((id(&i.value), Binding::new(key_text(i.key?), &i.label)))));
    if section == "connection" && matches!(site_row(p), SiteRow::Field(..) | SiteRow::Site(_)) {
        b.extend([
            (id("token_page"), Binding::new("o", "open Atlassian's API token page")),
            (id("copy_token_link"), Binding::new("y", "copy the API token page's link")),
        ]);
    }
    b
}

#[cfg(test)]
#[path = "settings_checks/tests.rs"]
mod check_ownership_tests;

#[cfg(test)]
mod mnemonic_tests {
    use super::*;

    fn check_menu(menu: &Menu) -> usize {
        let mut checked = 0;
        for item in &menu.items {
            if !item.heading && !item.note && radar_core::mnemonic::letter(&item.keys).is_some() {
                let words = radar_core::mnemonic::choice_words(menu.question, &item.label, &item.hint);
                assert!(radar_core::mnemonic::at(&item.keys, words).is_some(),
                    "{}: {:?} has no mnemonic in {:?}", menu.id, item.keys, words);
                checked += 1;
            }
        }
        checked
    }


    fn check_setup_link(blocks: &[radar_core::view::Block], url: &str, copy: &str, key: &str) {
        use radar_core::view::{Block, Text};
        fn find<'a>(blocks: &'a [Block], url: &str) -> Option<&'a Text> {
            blocks.iter().find_map(|block| match block {
                Block::Line { text, .. } if text.0.iter().any(|seg| seg.link.as_deref() == Some(url)) => Some(text),
                Block::Section { blocks, .. } => find(blocks, url),
                _ => None,
            })
        }
        let text = find(blocks, url).expect("actual setup token link");
        assert_eq!(text.0[0].link.as_deref(), Some(url));
        assert!(text.words().ends_with(&format!("   open · {copy}")));
        assert!(text.plain_words().ends_with(&format!("   open [o] · {copy} [{key}]")));
        let underlined: Vec<_> = text.0.iter().filter(|seg| seg.look.as_deref().is_some_and(|look| look.contains("underline"))).map(|seg| seg.text.as_str()).collect();
        assert_eq!(underlined, ["o", key]);
        let hints: Vec<_> = text.0.iter().filter_map(|seg| seg.key_hint.as_deref()).collect();
        assert_eq!(hints, ["o", key]);
        assert!(text.0[1..].iter().all(|seg| seg.link.is_none() && seg.target.is_none()));
    }

    #[test]
    fn settings_producers_embed_every_letter_in_action_words() {
        check_setup_link(&crate::setup::access_help(), crate::setup::TOKEN_PAGE, "copy", "y");
        let projects = crate::setup::projects_help();
        let radar_core::view::Block::Section { blocks, .. } = &projects[0] else { panic!("project help section") };
        let pick = blocks.iter().find_map(|block| match block {
            radar_core::view::Block::Line { text, .. } if text.0.iter().any(|seg| seg.key_hint.as_deref() == Some("p")) => Some(text),
            _ => None,
        }).expect("actual project picker hint");
        assert_eq!(pick.words(), "pick from the projects you looked at lately.");
        assert_eq!(pick.plain_words(), "pick from the projects you looked at lately. [p]");
        assert_eq!(pick.0.iter().filter(|seg| seg.look.as_deref().is_some_and(|look| look.contains("underline"))).map(|seg| seg.text.as_str()).collect::<Vec<_>>(), ["p"]);
        assert!(pick.0.iter().all(|seg| seg.link.is_none() && seg.target.is_none()));
        let mut checked = 0;
        for changed in [false, true] {
            let (mut p, fixture) = crate::test_support::hosted_plugin(vec![]).unwrap();
            if changed {
                for style in p.cfg.style.values_mut() { *style = "text.primary italic".into(); }
            }

            for section in ["connection", "projects", "views", "look"] {
                for row in 0..row_count(&p, section) {
                    p.section_rows[index(section)] = row;
                    if let Some(menu) = row_menu(&p, section) { checked += check_menu(&menu); }
                }
            }
            fixture.assert_finished();
        }
        assert!(checked > 20, "exercise real settings rows, including reset variants");
    }

    #[test]
    fn condition_producers_keep_values_and_meaningful_mnemonics() {
        for effect in [cond_menu(0), sign_menu(0, "me")] {
            let Effect::Menu(menu) = effect else { panic!("condition menu") };
            assert!(check_menu(&menu) > 0);
            if menu.id == "cond:0" {
                let item = menu.items.iter().find(|item| item.key == Some('u')).unwrap();
                assert_eq!((&*item.label, &*item.value), ("unassigned", "none"));
                let item = menu.items.iter().find(|item| item.key == Some('x')).unwrap();
                assert_eq!((&*item.label, &*item.value), ("text contains…", "words"));
            }
        }
    }
}

#[cfg(test)]
mod tree_tests;
