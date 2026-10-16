//! View data for what the host draws itself: the set-up page of a part that is
//! not set up yet, and the host-owned settings sections (parts, mode). The
//! Ratatui shell draws these straight into a buffer from the same host state
//! (`parts::setup_page`, `parts::render`, `mode::render`); a renderer that
//! draws only view data (Ink) is sent these instead.

use crate::compose::{self, Setup};
use crate::host::{Host, Owner};
use crate::ui::icon::Icon;
use crate::view::{Ink, ListRow, Node, SectionView, Seg, State, Text};
use crate::writes::{self, Mode};

/// The current settings section when the host owns it (parts, mode); `None`
/// for a part's section (its own view) and for the sections not yet given a
/// view.
pub fn section_view(host: &Host, tier: crate::design::theme::Tier) -> Option<SectionView> {
    match host.current_section()? {
        (Owner::Parts, s) => Some(parts(host, s.id)),
        (Owner::Mode, s) => Some(mode(host, s.id)),
        (Owner::Look, _) => Some(host.look_settings_view()),
        (Owner::Host, s) if host.on_icons() => Some(icons(host, s.id)),
        (Owner::Host, s) => Some(colours(host, s.id, tier)),
        _ => None,
    }
}

/// A muted capitals heading, as `widgets::heading` draws one.
fn heading(words: &str) -> Text {
    Text::new().push(Seg::new(words.to_uppercase(), Ink::Muted).strong())
}

fn blank(n: usize) -> ListRow {
    ListRow::heading(format!("blank:{n}"), Text::new())
}

fn kv(key: &str, width: usize, value: Seg) -> Text {
    Text::new().push(Seg::new(crate::ui::pad(key, width), Ink::Muted)).push(value)
}

fn section(scope: &str, body: Node, detail_title: String, detail: Vec<Text>) -> SectionView {
    SectionView {
        version: crate::view::VERSION,
        scope: scope.into(),
        body,
        detail_title,
        detail,
        detail_blocks: vec![],
        actions: vec![],
    }
}

fn parts(host: &Host, scope: &str) -> SectionView {
    let title = crate::parts::detail_title(host);
    if host.plugins.is_empty() {
        let text = "No parts are compiled into this build of radar. Each part is a cargo feature: \
                    build radar with the ones you want (cargo build --features <part>).";
        return section(scope, Node::State { state: State::Empty, text: text.into() }, title, parts_detail(host));
    }
    let name_w = host.plugins.iter().map(|p| crate::ui::width(p.name())).max().unwrap_or(0) + 3;
    let mut rows = Vec::new();
    if let Some(terminal) = &host.terminal {
        let failed = matches!(terminal.route, crate::terminal_host::Route::Failed(_));
        rows.push(ListRow::heading("terminal", heading("running in")));
        rows.push(ListRow::heading("terminal.stack", Text::of(terminal.stack.description(), Ink::Primary)));
        rows.push(ListRow::heading(
            "terminal.right_click",
            Text::of(format!("right-click: {}", terminal.right_click_words()), if failed { Ink::Warn } else { Ink::Muted }),
        ));
        rows.push(blank(0));
    }
    rows.push(ListRow::heading("compiled", heading("compiled into this radar")));
    rows.push(blank(1));
    let first = rows.len();
    for (i, p) in host.plugins.iter().enumerate() {
        let (mark, state) = if compose::ready(p.as_ref()) {
            (Seg::new("✓ ", Ink::Good), "set up")
        } else {
            (Seg::new("○ ", Ink::Warn), "not set up")
        };
        let text = Text::new()
            .push(mark)
            .push(Seg::new(crate::ui::pad(p.name(), name_w), Ink::Primary).strong())
            .push(Seg::new(state, Ink::Secondary));
        rows.push(ListRow::new(format!("part:{i}"), text));
    }
    let selected = Some(first + host.parts_row.min(host.plugins.len() - 1));
    section(scope, Node::List { rows, selected, editing: None }, title, parts_detail(host))
}

/// `parts::detail_lines` as view text.
fn parts_detail(host: &Host) -> Vec<Text> {
    let Some(p) = host.plugins.get(host.parts_row) else {
        let mut lead = Seg::icon(Icon::Empty);
        lead.icon_lead = true;
        return vec![Text::new().push(lead).push(Seg::new(
            "A part is one ability of radar, usually one tool it works with, compiled in \
             as a cargo feature. Parts never depend on each other: each works alone, \
             and each one present adds to the others.",
            Ink::Muted,
        ))];
    };
    let mut lines = vec![match p.set_up() {
        Setup::Ready => Text::of("✓ set up", Ink::Good),
        Setup::Needed(what) => Text::new().push(Seg::new("○ not set up: ", Ink::Warn)).push(Seg::new(what, Ink::Primary)),
    }];
    if let Some(terminal) = &host.terminal {
        lines.push(Text::new());
        lines.push(heading("terminal"));
        lines.push(Text::of(terminal.stack.description(), Ink::Primary));
        lines.push(Text::of(format!("right-click: {}", terminal.right_click_words()), Ink::Secondary));
        if let crate::terminal_host::Route::Failed(error) = &terminal.route {
            lines.push(Text::of(error.clone(), Ink::Warn));
        }
    }
    lines.push(Text::new());
    let w = 8;
    let kinds = |list: Vec<&compose::Owned>| list.iter().map(|o| o.name).collect::<Vec<_>>().join(", ");
    let owns = p.owns();
    if !owns.is_empty() {
        lines.push(kv("has", w, Seg::new(kinds(owns.iter().collect()), Ink::Primary)));
        let pages = kinds(owns.iter().filter(|o| o.page).collect());
        if !pages.is_empty() {
            lines.push(kv("opens", w, Seg::new(format!("{pages} on their own pages"), Ink::Primary)));
        }
        lines.push(Text::new());
    }
    let offers = p.offers();
    if !offers.is_empty() {
        lines.push(heading("other parts can add"));
        for o in offers {
            lines.push(Text::of(format!("· {} {}", o.point.words(), o.place), Ink::Primary));
        }
        lines.push(Text::new());
    }
    let others: Vec<_> = host.plugins.iter().filter(|o| o.id() != p.id()).collect();
    let name = |kind: &str| others.iter().flat_map(|o| o.owns()).find(|o| o.kind == kind).map(|o| o.name);
    let adds: Vec<(&str, &str)> =
        p.adds().into_iter().filter_map(|a| name(a.on).map(|n| (a.what, n))).collect();
    lines.push(heading(&format!("{} adds", p.name())));
    if adds.is_empty() {
        lines.push(Text::of("nothing to the other parts in this build", Ink::Muted));
    } else {
        let note = if compose::ready(p.as_ref()) { "" } else { " (once set up)" };
        for (what, on) in adds {
            lines.push(Text::of(format!("· {what} on {on}{note}"), Ink::Primary));
        }
    }
    lines
}

fn mode(host: &Host, scope: &str) -> SectionView {
    let m = host.mode();
    let mut icon = Seg::icon(m.icon());
    icon.icon_lead = true;
    let mode_row = ListRow::new(
        "mode",
        Text::new()
            .push(icon)
            .push(Seg::new(crate::ui::pad("mode", 8), Ink::Primary).strong())
            .push(Seg::new(m.word(), Ink::Primary)),
    );
    let mut pause_icon = Seg::icon(Icon::Settings);
    pause_icon.icon_lead = true;
    let pause_row = ListRow::new(
        "pause",
        Text::new()
            .push(pause_icon)
            .push(Seg::new(crate::ui::pad("pause if idle", 14), Ink::Primary).strong())
            .push(Seg::new(crate::mode::idle_words(host.pause.idle_minutes), Ink::Primary)),
    );
    // Under the mode, so it is never cut: who set it for this run.
    let set_by = match &host.mode_env {
        Some((_, v)) => ListRow::heading("mode.set_by", Text::of(format!("        set by {v}"), Ink::Warn)),
        None => blank(1),
    };
    let rows = vec![
        ListRow::heading("mode.what", heading("what radar may change")),
        blank(0),
        mode_row,
        set_by,
        blank(2),
        ListRow::heading("mode.idle", heading("when idle")),
        blank(3),
        pause_row,
    ];
    let selected = Some(if host.pause.row == 1 { 7 } else { 2 });
    section(scope, Node::List { rows, selected, editing: None }, "the modes".into(), mode_detail(host))
}

/// `mode::detail_lines` as view text.
fn mode_detail(host: &Host) -> Vec<Text> {
    if host.pause.row == 1 {
        return vec![
            Text::of(
                "Every part stops asking its service when nobody has pressed a key or clicked for this long, so a forgotten window stays quiet. Any key resumes it; z pauses and resumes by hand.",
                Ink::Primary,
            ),
            Text::new(),
            Text::of("whole minutes; never switches it off · [general] idle_minutes", Ink::Muted),
        ];
    }
    let now = host.mode();
    let mut lines = Vec::new();
    for m in Mode::ALL {
        let mut icon = Seg::icon(m.icon());
        icon.icon_lead = true;
        lines.push(
            Text::new()
                .push(Seg::new(if m == now { "● " } else { "  " }, Ink::Accent))
                .push(icon)
                .push(Seg::new(m.word(), Ink::Primary).strong()),
        );
        lines.push(Text::of(format!("    {}", m.about()), Ink::Secondary));
    }
    lines.push(Text::new());
    lines.push(Text::of("Local actions (check out, an editor, a terminal) and settings work in every mode.", Ink::Muted));
    lines.push(Text::of(format!("{} sets it for one run: read-only, write or edit.", writes::MODE_VAR), Ink::Muted));
    if let Some((env, v)) = &host.mode_env {
        lines.push(Text::new());
        lines.push(Text::of(
            format!("Now {v} keeps radar {} for this run; the setting is kept for the next.", env.word()),
            Ink::Warn,
        ));
    }
    lines
}

#[path = "settings_view_colours.rs"]
mod colours_view;
use colours_view::{colours, icons};

#[cfg(test)]
#[path = "settings_view_tests.rs"]
mod tests;
