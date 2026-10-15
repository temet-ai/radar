//! The Agents section as view data: a list of the CLIs, then the defaults,
//! with the selected row's detail beside it.

use radar_core::view::{ActionSpec, Ink, ListRow, Node, SectionView, Seg, State as Ui, Text, ViewCx};
use radar_core::{Binding, ui::icon::Icon};

use super::status::State;
use super::{AgentsSettings, Row, SECTION};
use crate::adapter::AdapterStatus;
use crate::login::Login;

fn heading(words: &str) -> Text {
    Text::new().push(Seg::new(words.to_uppercase(), Ink::Muted).strong())
}

fn lead(icon: Icon, words: &str, ink: Ink) -> Seg {
    Seg::icon_or_lead(icon, words, ink)
}

fn state_seg(s: &State) -> Seg {
    match s {
        State::Ready { .. } => lead(Icon::Done, "✓ ", Ink::Good),
        State::Installed => lead(Icon::Done, "✓ ", Ink::Secondary),
        State::Unread => lead(Icon::Time, "… ", Ink::Muted),
        State::NoAdapter => lead(Icon::Time, "– ", Ink::Muted),
        State::SignedOut => lead(Icon::Warning, "○ ", Ink::Warn),
        State::PolicyBlocked(_) => lead(Icon::Warning, "! ", Ink::Warn),
        State::Missing => lead(Icon::Error, "✗ ", Ink::Bad),
        State::Error(_) => lead(Icon::Error, "✗ ", Ink::Bad),
    }
}

fn ink_of(s: &State) -> Ink {
    match s {
        State::Ready { .. } => Ink::Good,
        State::Installed | State::Unread | State::NoAdapter => Ink::Secondary,
        State::SignedOut | State::PolicyBlocked(_) => Ink::Warn,
        State::Missing | State::Error(_) => Ink::Bad,
    }
}

/// A path with the home folder written `~`.
fn tilde(path: &std::path::Path) -> String {
    let p = path.display().to_string();
    match std::env::var("HOME") {
        Ok(h) if !h.is_empty() && p.starts_with(&h) => format!("~{}", &p[h.len()..]),
        _ => p,
    }
}

fn list(items: &[String]) -> String {
    if items.is_empty() { "none".into() } else { items.join(", ") }
}

fn yes(on: bool) -> &'static str {
    if on { "yes" } else { "no" }
}

pub(super) fn section(s: &AgentsSettings, cx: &ViewCx<'_>) -> SectionView {
    let d = s.defaults();
    let name_w = 14;
    let mut rows = vec![ListRow::heading("h:clis", heading("command line tools")), ListRow::heading("b:0", Text::new())];
    let first = rows.len();
    let all = s.rows();
    for r in &all {
        if let Row::Cli(id) = r {
            let state = s.state(id);
            let mut line = Text::new()
                .push(state_seg(&state))
                .push(Seg::new(radar_core::ui::pad(&s.name_of(id), name_w), Ink::Primary).strong())
                .push(Seg::new(state.word(), ink_of(&state)));
            if let State::Ready { account } = &state {
                line.add(Seg::new(format!("  {account}"), Ink::Secondary));
            }
            let mut row = ListRow::new(r.target(), line);
            if let Some(v) = s.status(id).and_then(|st| st.version.as_ref()) {
                row = row.right(Text::of(v.clone(), Ink::Muted));
            }
            rows.push(row);
        }
    }
    rows.push(ListRow::heading("b:1", Text::new()));
    let scope = s.workspace.as_deref().map_or("for every workspace".to_string(), |w| format!("for workspace {w}"));
    rows.push(ListRow::heading("h:defaults", heading(&format!("new sessions · {scope}"))));
    rows.push(ListRow::heading("b:2", Text::new()));
    let kv = |row: &Row, key: &str, value: String| {
        ListRow::new(row.target(), Text::new().push(Seg::new(radar_core::ui::pad(key, 22), Ink::Muted)).push(Seg::new(value, Ink::Primary)))
    };
    let recipes_dir = s.recipes_dir();
    let mut recipes_heading = false;
    for r in &all {
        if !r.is_default() && !recipes_heading && !matches!(r, Row::Cli(_)) {
            recipes_heading = true;
            rows.push(ListRow::heading("b:3", Text::new()));
            rows.push(ListRow::heading("h:recipes", heading("recipes and reviews")));
            rows.push(ListRow::heading("b:4", Text::new()));
        }
        let row = match r {
            Row::Cli(_) => continue,
            Row::Provider => kv(r, "provider", s.name_of(&d.provider)),
            Row::Model => kv(r, "model", d.model.clone().unwrap_or_else(|| "the provider's own".into())),
            Row::WorkDir => kv(r, "working folder", d.work_dir.clone().unwrap_or_else(|| "where radar was started".into())),
            Row::Roots => kv(r, "permitted folders", list(&d.permitted_roots)),
            Row::Skills => kv(r, "skill folders", list(&d.skill_dirs)),
            Row::ProjectConfig => kv(r, "CLI's own config", yes(d.project_config).into()),
            Row::Allow => kv(r, "always allow", list(&d.allow.iter().map(ToString::to_string).collect::<Vec<_>>())),
            Row::Deny => kv(r, "always deny", list(&d.deny.iter().map(ToString::to_string).collect::<Vec<_>>())),
            Row::LocalOnly => kv(r, "local only", yes(d.local_only).into()),
            Row::Reviewer => kv(r, "reviewer commands", yes(d.reviewer_may_run_commands).into()),
            Row::RecipesDir => kv(r, "recipes folder", recipes_dir.as_ref().map_or("none".into(), |p| tilde(p))),
            Row::Recipe(name) => {
                let (recipe, source, broken) = crate::brief::Recipe::load(name, recipes_dir.as_deref());
                let whose = match (&source, broken.is_some()) {
                    (_, true) => "yours is broken: built-in used",
                    (crate::brief::recipe::Source::File(_), _) => "yours",
                    _ => "built-in",
                };
                let mut v = format!("v{} · {whose}", recipe.as_ref().map_or(0, |r| r.version));
                if let Some(a) = recipe.as_ref().and_then(|r| r.agent.clone()) {
                    v.push_str(&format!(" · agent {a}"));
                }
                if let Some(sk) = recipe.as_ref().filter(|r| !r.skills.is_empty()) {
                    v.push_str(&format!(" · skills {}", sk.skills.join(", ")));
                }
                kv(r, &format!("recipe {name}"), v)
            }
            Row::ReviewsDir => kv(r, "reviews folder", s.cfg.reviews_dir.clone().unwrap_or_else(|| "radar's state folder".into())),
            Row::Notify => {
                rows.push(ListRow::heading("b:5", Text::new()));
                rows.push(ListRow::heading("h:away", heading("while you are away")));
                rows.push(ListRow::heading("b:6", Text::new()));
                kv(r, "desktop notifications", yes(s.cfg.notify).into())
            }
        };
        rows.push(row);
    }
    let target = s.selected().target();
    let selected = rows.iter().position(|r| r.target.as_str() == target).or(Some(first));
    let _ = cx;
    SectionView {
        version: radar_core::view::VERSION,
        scope: SECTION.into(),
        body: Node::List { rows, selected, editing: None },
        detail_title: detail_title(s),
        detail: detail(s),
        detail_blocks: vec![],
        actions: actions(s),
    }
}

fn detail_title(s: &AgentsSettings) -> String {
    match s.selected() {
        Row::Cli(id) => s.name_of(&id),
        Row::Provider => "default provider".into(),
        Row::Model => "default model".into(),
        Row::WorkDir => "working folder".into(),
        Row::Skills => "skill folders".into(),
        Row::ProjectConfig => "the CLI's own config".into(),
        Row::RecipesDir => "recipes folder".into(),
        Row::Recipe(name) => format!("recipe {name}"),
        Row::ReviewsDir => "reviews folder".into(),
        Row::Roots => "permitted folders".into(),
        Row::Allow => "always allow".into(),
        Row::Deny => "always deny".into(),
        Row::LocalOnly => "local only".into(),
        Row::Reviewer => "reviewer sessions".into(),
        Row::Notify => "desktop notifications".into(),
    }
}

fn cli_detail(s: &AgentsSettings, id: &str) -> Vec<Text> {
    let state = s.state(id);
    let status: Option<&AdapterStatus> = s.status(id);
    let kv = |k: &str, v: String| Text::new().push(Seg::new(radar_core::ui::pad(k, 14), Ink::Muted)).push(Seg::new(v, Ink::Primary));
    let mut lines = vec![];
    // A sign-in under way comes first: the code must be in view in a short pane.
    if let Some((p, l)) = &s.login
        && p == id
        && *l != Login::Idle
    {
        lines.push(heading("sign-in"));
        lines.extend(l.words().into_iter().map(|w| Text::of(w, Ink::Primary)));
        lines.push(Text::new());
    }
    lines.push(Text::new().push(state_seg(&state)).push(Seg::new(state.advice(), ink_of(&state))));
    lines.push(Text::new());
    if let Some(st) = status {
        lines.push(kv("installed", yes(st.installed).into()));
        if let Some(v) = &st.version {
            lines.push(kv("version", v.clone()));
        }
        if let Some(p) = &st.protocol {
            lines.push(kv("protocol", p.clone()));
        }
        if let State::Ready { account } = &state {
            lines.push(kv("signed in as", account.clone()));
        }
        let c = &st.capabilities;
        lines.push(Text::new());
        lines.push(heading("what it can do"));
        for (word, on) in [
            ("resume a session", c.resume),
            ("stop a turn", c.stop),
            ("send while a turn runs", c.steer),
            ("list its sessions", c.list_sessions),
            ("list models", c.list_models),
            ("ask for permission", c.permissions),
            ("structured events", c.structured_events),
        ] {
            let why = c.unsupported.iter().find(|u| u.control.eq_ignore_ascii_case(word) || word.contains(u.control.as_str()));
            let mut line = Text::new().push(if on { lead(Icon::Done, "✓ ", Ink::Good) } else { lead(Icon::Error, "✗ ", Ink::Muted) }).push(Seg::new(word, Ink::Primary));
            if let (false, Some(u)) = (on, why) {
                line.add(Seg::new(format!("  {}", u.reason), Ink::Muted));
            }
            lines.push(line);
        }
    }
    lines
}

fn detail(s: &AgentsSettings) -> Vec<Text> {
    let words = |t: &str| vec![Text::of(t, Ink::Primary)];
    match s.selected() {
        Row::Cli(id) => cli_detail(s, &id),
        Row::Provider => words("The CLI a new session starts with. A session already running keeps its own."),
        Row::Model => words("The model a new session asks for. Empty leaves it to the provider."),
        Row::Roots => words("Folders a session may read and write besides its working directory. Empty: its working directory only. Copilot and Claude Code get them as extra directories, Codex as writable roots of its sandbox."),
        Row::WorkDir => words("Where a session started from the dock works, and where its CLI finds the project's AGENTS.md, CLAUDE.md, agents and skills. Empty: the folder radar was started in. A review works in its own worktree."),
        Row::Skills => words("Skill folders (each skill a folder with a SKILL.md) every CLI loads besides its own. A CLI's own row (p) adds folders for that CLI only."),
        Row::ProjectConfig => words("Each CLI loads its own configuration of you and of the project: AGENTS.md or CLAUDE.md, its agents, skills, MCP servers and hooks. Off: only what these settings name."),
        Row::RecipesDir => recipes_detail(s),
        Row::Recipe(name) => recipe_detail(s, &name),
        Row::ReviewsDir => words("Where a local review is saved when no workspace keeps notes for it. Empty: radar's state folder ($XDG_STATE_HOME/radar/reviews)."),
        Row::Allow => words("Requests that never ask, as kind(argument): shell(git diff), read(src/**), url(github.com). Each provider's adapter maps them onto its own permissions."),
        Row::Deny => words("Requests that are always refused, as kind(argument). A deny wins over an allow."),
        Row::LocalOnly => words("Sessions run on this machine only: no remote or delegated runs. Copilot refuses /delegate, & and --remote and keeps custom agents local; Codex and Claude Code have no remote run to refuse."),
        Row::Reviewer => words("A reviewer session may ask to run commands that execute the change's code; each one is approved in the dock. Off: it reads and runs git diff, log and show only."),
        Row::Notify => words("A desktop notification when a session needs you (an approval, a question) or its turn ends while the dock is hidden or shows another tab: notify-send on Linux, the notification centre on macOS. The status line's chip says the same in radar whether this is on or off. Off by default."),
    }
}

fn recipes_detail(s: &AgentsSettings) -> Vec<Text> {
    let dir = s.recipes_dir().map_or("none".into(), |p| p.display().to_string());
    vec![
        Text::of(format!("Your recipes: {dir}"), Ink::Primary),
        Text::new(),
        Text::of("A recipe is a Markdown file: a front matter of name, version, inputs, scope, permissions, expected-output and stopping-point, then the instructions for the agent. Optional: agent (one name, or provider=name pairs) and skills (comma separated).", Ink::Primary),
        Text::new(),
        Text::of("A file named like a built-in (ask.md, review-mr.md) replaces it; any other file is a recipe of its own. e copies the built-ins there and opens the folder in your editor.", Ink::Secondary),
    ]
}

fn recipe_detail(s: &AgentsSettings, name: &str) -> Vec<Text> {
    let (recipe, source, broken) = crate::brief::Recipe::load(name, s.recipes_dir().as_deref());
    let kv = |k: &str, v: String| Text::new().push(Seg::new(radar_core::ui::pad(k, 14), Ink::Muted)).push(Seg::new(v, Ink::Primary));
    let mut lines = vec![];
    if let Some(why) = broken {
        lines.push(Text::new().push(lead(Icon::Warning, "! ", Ink::Warn)).push(Seg::new(format!("not used: {why}"), Ink::Warn)));
        lines.push(Text::new());
    }
    lines.push(kv(
        "from",
        match source {
            crate::brief::recipe::Source::File(p) => tilde(&p),
            crate::brief::recipe::Source::BuiltIn => "built into radar".into(),
        },
    ));
    if let Some(r) = recipe {
        lines.push(kv("version", r.version.to_string()));
        lines.push(kv("agent", r.agent.clone().unwrap_or_else(|| "the setting's".into())));
        lines.push(kv("skills", list(&r.skills)));
        lines.push(Text::new());
        lines.push(kv("scope", r.scope.clone()));
        lines.push(kv("permissions", r.permissions.clone()));
        lines.push(kv("output", r.expected_output.clone()));
        lines.push(kv("stops", r.stopping_point.clone()));
    }
    lines.push(Text::new());
    lines.push(Text::of(
        match name {
            "ask" => "Used by A (ask an agent about it) on any row: the brief's task and limits.",
            "review-mr" => "Used by review locally: the reviewer's task, limits and report.",
            _ => "A recipe of your own.",
        },
        Ink::Secondary,
    ));
    lines
}

fn actions(s: &AgentsSettings) -> Vec<ActionSpec> {
    let named = |id: &str, b: Binding| ActionSpec::named(id, &b);
    match s.selected() {
        Row::Recipe(_) | Row::RecipesDir => vec![
            named("", Binding::heading("recipes")),
            named("sessions.agents.edit", Binding::new("⏎", "agent, skills, edit")),
            named("sessions.agents.recipe.edit", Binding::new("e", "edit in your editor")),
        ],
        Row::Cli(_) => {
            let mut v = vec![
                named("", Binding::heading("this tool")),
                named("sessions.agents.sign_in", Binding::new("i", "sign in")),
                named("sessions.agents.refresh", Binding::new("r", "read its status again")),
                named("sessions.agents.paths", Binding::new("p", "paths, agent and folders")),
            ];
            if s.login.as_ref().is_some_and(|(_, l)| l.active()) {
                v.push(named("sessions.agents.cancel", Binding::new("esc", "cancel the sign-in")));
            }
            v
        }
        _ => vec![named("", Binding::heading("this default")), named("sessions.agents.edit", Binding::new("⏎", "change it"))],
    }
}

#[allow(dead_code)]
fn _unused(_: Ui) {}
