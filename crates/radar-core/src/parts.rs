//! The host's `parts` settings section: every part compiled into this build,
//! whether it is set up, what it has, and what it adds to the others. So the
//! user sees what turning one on would bring.

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};

use crate::compose::{self, Owned, Setup};
use crate::design::theme::Slot;
use crate::host::{Host, HostEffect};
use crate::plugin::{Binding, Pane, Plugin};
use crate::theme::Theme;
use crate::widgets;

impl Host {
    /// A key in the parts section: move, go to a part's settings, back out.
    pub(crate) fn parts_key(&mut self, key: KeyEvent) -> Vec<HostEffect> {
        let n = self.plugins.len();
        if let Some(down) = widgets::vertical(&key) {
            self.parts_row = widgets::step(self.parts_row, n, down);
            return vec![];
        }
        match key.code {
            KeyCode::Enter if self.parts_row < n => self.go_to_settings_of(self.parts_row),
            KeyCode::Esc => self.focus = Pane::Nav,
            _ => {}
        }
        vec![]
    }
}

/// The section's `x` menu.
pub(crate) fn keys(host: &Host) -> Vec<Binding> {
    let mut rows = vec![Binding::heading("parts")];
    if let Some(p) = host.plugins.get(host.parts_row)
        && host.first_section_of(host.parts_row).is_some()
    {
        let name = p.name();
        rows.push(Binding::new(
            "⏎",
            if compose::ready(p.as_ref()) {
                format!("go to {name}'s settings")
            } else {
                format!("set up {name}")
            },
        ));
    }
    rows
}

/// The kinds a part owns, in its own words: `books, shelves`.
fn kinds<'a>(list: impl Iterator<Item = &'a Owned>) -> String {
    list.map(|o| o.name).collect::<Vec<_>>().join(", ")
}

/// `✓ Books   set up`, one per part. Returns each row's area, for clicks.
pub(crate) fn render(
    host: &Host,
    t: &Theme,
    area: Rect,
    buf: &mut Buffer,
    focused: bool,
) -> Vec<(Rect, usize)> {
    if host.plugins.is_empty() {
        widgets::empty(
            buf,
            area,
            t,
            "No parts are compiled into this build of radar. Each part is a cargo feature: \
             build radar with the ones you want (cargo build --features <part>).",
        );
        return vec![];
    }
    let name_w = host
        .plugins
        .iter()
        .map(|p| crate::ui::width(p.name()))
        .max()
        .unwrap_or(0)
        + 3;
    let mut hits = Vec::new();
    let extra = if let Some(terminal) = &host.terminal {
        buf.set_line(area.x + 1, area.y, &widgets::heading(t, "running in"), area.width.saturating_sub(2));
        if area.height > 1 {
            buf.set_line(area.x + 1, area.y + 1,
                &Line::styled(terminal.stack.description(), t.s(Slot::TextPrimary)),
                area.width.saturating_sub(2));
        }
        if area.height > 2 {
            let slot = if matches!(terminal.route, crate::terminal_host::Route::Failed(_)) {
                Slot::StatusWarning
            } else { Slot::TextMuted };
            buf.set_line(area.x + 1, area.y + 2,
                &Line::styled(format!("right-click: {}", terminal.right_click_words()), t.s(slot)),
                area.width.saturating_sub(2));
        }
        4
    } else { 0 };
    let head = Rect { y: area.y + extra, height: 1, ..area };
    buf.set_line(
        area.x + 1,
        head.y,
        &widgets::heading(t, "compiled into this radar"),
        head.width.saturating_sub(2),
    );
    for (i, p) in host.plugins.iter().enumerate() {
        let y = area.y + extra + 2 + i as u16;
        if y >= area.y + area.height {
            break;
        }
        let ready = compose::ready(p.as_ref());
        let (mark, state) = if ready {
            (Span::styled("✓ ", t.s(Slot::StatusSuccess)), "set up")
        } else {
            (Span::styled("○ ", t.s(Slot::StatusWarning)), "not set up")
        };
        let spans = vec![
            mark,
            Span::styled(crate::ui::pad(p.name(), name_w), t.bold(Slot::TextPrimary)),
            Span::styled(state.to_string(), t.s(Slot::TextSecondary)),
        ];
        let style = widgets::row_style(t, focused, i == host.parts_row);
        widgets::row(buf, area, y, style, Line::from(spans), None);
        hits.push((
            Rect {
                y,
                height: 1,
                ..area
            },
            i,
        ));
    }
    hits
}

/// The detail panel's title: the selected part.
pub(crate) fn detail_title(host: &Host) -> String {
    host.plugins
        .get(host.parts_row)
        .map_or("about".into(), |p| p.name().to_string())
}

/// About the selected part: set up or what it needs, what it has, where
/// others can add to it, and what it adds to them.
pub(crate) fn detail_lines(host: &Host, t: &Theme) -> Vec<Line<'static>> {
    let Some(p) = host.plugins.get(host.parts_row) else {
        return vec![Line::from(vec![
            crate::ui::icon::lead(t, crate::ui::icon::Icon::Empty),
            Span::styled(
                "A part is one ability of radar, usually one tool it works with, compiled in \
                 as a cargo feature. Parts never depend on each other: each works alone, \
                 and each one present adds to the others.",
                t.s(Slot::TextMuted),
            ),
        ])];
    };
    let mut lines = vec![match p.set_up() {
        Setup::Ready => Line::styled("✓ set up", t.s(Slot::StatusSuccess)),
        Setup::Needed(what) => Line::from(vec![
            Span::styled("○ not set up: ", t.s(Slot::StatusWarning)),
            Span::styled(what, t.s(Slot::TextPrimary)),
        ]),
    }];
    if let Some(terminal) = &host.terminal {
        lines.push(Line::default());
        lines.push(widgets::heading(t, "terminal"));
        lines.push(Line::styled(terminal.stack.description(), t.s(Slot::TextPrimary)));
        lines.push(Line::styled(format!("right-click: {}", terminal.right_click_words()), t.s(Slot::TextSecondary)));
        if let crate::terminal_host::Route::Failed(error) = &terminal.route {
            lines.push(Line::styled(error.clone(), t.s(Slot::StatusWarning)));
        }
    }
    lines.push(Line::default());
    let w = 8;
    let owns = p.owns();
    if !owns.is_empty() {
        lines.push(widgets::kv(
            t,
            "has",
            w,
            vec![Span::styled(kinds(owns.iter()), t.s(Slot::TextPrimary))],
        ));
        let pages = kinds(owns.iter().filter(|o| o.page));
        if !pages.is_empty() {
            lines.push(widgets::kv(
                t,
                "opens",
                w,
                vec![Span::styled(
                    format!("{pages} on their own pages"),
                    t.s(Slot::TextPrimary),
                )],
            ));
        }
        lines.push(Line::default());
    }
    let offers = p.offers();
    if !offers.is_empty() {
        lines.push(widgets::heading(t, "other parts can add"));
        for o in offers {
            lines.push(Line::styled(
                format!("· {} {}", o.point.words(), o.place),
                t.s(Slot::TextPrimary),
            ));
        }
        lines.push(Line::default());
    }
    // Only what it adds to a part that is here: absent means absent.
    let others: Vec<&Box<dyn Plugin>> = host.plugins.iter().filter(|o| o.id() != p.id()).collect();
    let name = |kind: &str| {
        others
            .iter()
            .flat_map(|o| o.owns())
            .find(|o| o.kind == kind)
            .map(|o| o.name)
    };
    let adds: Vec<(&str, &str)> = p
        .adds()
        .into_iter()
        .filter_map(|a| name(a.on).map(|n| (a.what, n)))
        .collect();
    lines.push(widgets::heading(t, &format!("{} adds", p.name())));
    if adds.is_empty() {
        lines.push(Line::styled(
            "nothing to the other parts in this build",
            t.s(Slot::TextMuted),
        ));
    } else {
        let note = if compose::ready(p.as_ref()) {
            ""
        } else {
            " (once set up)"
        };
        for (what, on) in adds {
            lines.push(Line::styled(
                format!("· {what} on {on}{note}"),
                t.s(Slot::TextPrimary),
            ));
        }
    }
    lines
}

/// The set-up page of a part that is not set up yet, over its screen.
pub(crate) fn setup_page(host: &Host, p: usize, t: &Theme, area: Rect, buf: &mut Buffer) {
    let plugin: &dyn Plugin = host.plugins[p].as_ref();
    let name = plugin.name();
    let block = widgets::panel(t, &format!("set up {name}"), true);
    let inner = block.inner(area);
    ratatui::widgets::Widget::render(block, area, buf);
    let what = match plugin.set_up() {
        Setup::Needed(what) => what,
        Setup::Ready => String::new(),
    };
    let how = host.first_section_of(p).is_some().then(|| {
        format!(
            "Press ⏎ to set it up in settings, under {}.",
            plugin.sections()[0].group
        )
    });
    crate::ui::states::set_up(t, inner, buf, name, &what, how.as_deref());
}
