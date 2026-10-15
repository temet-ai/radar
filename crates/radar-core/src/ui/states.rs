//! What a panel shows before it has anything: an empty state that says what
//! to do, the set-up state of a part that is not set up, loading and failed
//! lines, and the spinner (`throbber-widgets-tui`).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use throbber_widgets_tui::{BRAILLE_SIX, Throbber, ThrobberState};

use super::icon::Icon;
use crate::design::theme::Slot;
use crate::theme::Theme;
use crate::widgets;

/// Muted text wrapped in the panel: say what to do ("No shelves yet. Press
/// S, then add one under shelves.").
pub fn empty(theme: &Theme, area: Rect, buf: &mut Buffer, text: &str) {
    widgets::empty(buf, area, theme, text);
}

/// The set-up state: `<name> is not set up yet.`, what it needs, and how
/// to set it up (`Press ⏎ to set it up in settings, under …`).
pub fn set_up(theme: &Theme, area: Rect, buf: &mut Buffer, name: &str, needs: &str, how: Option<&str>) {
    let mut lines = vec![
        Line::from(vec![
            super::icon::lead(theme, Icon::Settings),
            Span::styled(format!("{name} is not set up yet."), theme.bold(Slot::TextEmphasis)),
        ]),
        Line::default(),
        Line::styled(needs.to_string(), theme.s(Slot::TextPrimary)),
        Line::default(),
    ];
    if let Some(how) = how {
        lines.push(Line::styled(how.to_string(), theme.s(Slot::TextSecondary)));
    }
    widgets::paragraph(buf, area, lines);
}

/// `loading…`, muted.
pub fn loading(theme: &Theme) -> Line<'static> {
    Line::styled("loading…", theme.s(Slot::TextMuted))
}

/// `✗ could not load: <why>`, with the error icon when icons are on.
pub fn failed(theme: &Theme, why: &str) -> Line<'static> {
    let style = theme.s(Slot::StatusError);
    Line::from(vec![
        super::icon::or(theme, Icon::Error, "✗", style),
        Span::styled(format!(" could not load: {why}"), style),
    ])
}

/// `✗ <source> could not sync: <why>`: a nav pane's line for a source that
/// failed, the same in every part (`source` is a site, a tenancy, a team).
pub fn sync_failed(theme: &Theme, source: &str, why: &str) -> Line<'static> {
    let style = theme.s(Slot::StatusError);
    Line::from(vec![
        super::icon::or(theme, Icon::Error, "✗", style),
        Span::styled(format!(" {source} could not sync: {why}"), style),
    ])
}

/// The line for something fetched: what went wrong, or `loading…` while
/// there is nothing to show yet; nothing once there is.
pub fn fetching(theme: &Theme, loading: bool, shown: bool, error: Option<&str>) -> Option<Line<'static>> {
    if let Some(e) = error {
        return Some(failed(theme, e));
    }
    (loading && !shown).then(|| self::loading(theme))
}

/// The spinner's symbol at `frame` (the host's frame count), in the info
/// colour: the same one the top bar shows while a part is busy.
pub fn spinner(theme: &Theme, frame: usize) -> Span<'static> {
    let span = throbber(theme).to_symbol_span(&at(frame));
    Span::styled(span.content.trim_end().to_string(), span.style)
}

/// `⠋ asking the server…`: the spinner and what is running.
pub fn busy(theme: &Theme, frame: usize, what: &str) -> Line<'static> {
    let line = throbber(theme)
        .label(what.to_string())
        .style(theme.s(Slot::StatusInfo))
        .to_line(&at(frame));
    Line::from(
        line.spans
            .into_iter()
            .map(|s| Span::styled(s.content.into_owned(), s.style))
            .collect::<Vec<_>>(),
    )
}

fn throbber(theme: &Theme) -> Throbber<'static> {
    Throbber::default()
        .throbber_set(BRAILLE_SIX)
        .throbber_style(theme.s(Slot::StatusInfo))
}

/// The spinner's state at `frame`. A step of 0 would pick at random.
fn at(frame: usize) -> ThrobberState {
    let mut state = ThrobberState::default();
    let i = frame % BRAILLE_SIX.symbols.len();
    if i > 0 {
        state.calc_step(i as i8);
    }
    state
}
