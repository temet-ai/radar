//! The status strip (`4 open · 1 to review · 1 ready`) and a check's
//! verdict: a mark for a row and a line for its detail panel.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};

use crate::design::theme::Slot;
use crate::plugin::Tone;
use crate::theme::Theme;
use crate::widgets;

/// Counts in a row, each in its tone, between muted `·`.
pub fn strip(theme: &Theme, items: &[(String, Tone)]) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, (text, tone)) in items.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", theme.s(Slot::TextMuted)));
        }
        spans.push(Span::styled(text.clone(), widgets::tone(theme, *tone)));
    }
    Line::from(spans)
}

/// The strip at the right edge of `area`, when it fits in half of it.
pub fn strip_right(theme: &Theme, area: Rect, buf: &mut Buffer, items: &[(String, Tone)]) {
    let line = strip(theme, items);
    let w = line.width() as u16;
    if w > 0 && w <= area.width {
        buf.set_line(area.x + area.width - w, area.y, &line, w);
    }
}

/// Where a check of a setting stands: not run, running, or what the
/// service said and how long ago.
/// The top bar's sync words, the same in every part: `syncing Notes`
/// while it runs, else `synced 4s ago`, or `not synced yet`; then
/// ` · every 1m` when `every_secs` is given.
pub fn sync_words(part: &str, busy: bool, last: Option<i64>, now: i64, every_secs: Option<u64>) -> String {
    let said = match (busy, last) {
        (true, _) => format!("syncing {part}"),
        (false, Some(t)) => format!("synced {} ago", crate::time::ago(t, now)),
        (false, None) => "not synced yet".to_string(),
    };
    match every_secs {
        Some(s) => format!("{said} · {}", super::connection::every(s)),
        None => said,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    NotYet,
    Running,
    Passed { said: String, ago: Option<String> },
    Failed { said: String, ago: Option<String> },
}

impl Check {
    /// The mark at the right of a setting's row: `checking`, `✓`, `✗`.
    pub fn mark(&self, theme: &Theme) -> Option<Span<'static>> {
        match self {
            Check::NotYet => None,
            Check::Running => Some(Span::styled("checking", theme.s(Slot::StatusInfo))),
            Check::Passed { .. } => Some(Span::styled("✓", theme.s(Slot::StatusSuccess))),
            Check::Failed { .. } => Some(Span::styled("✗", theme.s(Slot::StatusError))),
        }
    }

    /// [`Check::mark`] as view text.
    pub fn mark_text(&self) -> Option<crate::view::Text> {
        use crate::view::{Ink, Text};
        match self {
            Check::NotYet => None,
            Check::Running => Some(Text::of("checking", Ink::Info)),
            Check::Passed { .. } => Some(Text::of("✓", Ink::Good)),
            Check::Failed { .. } => Some(Text::of("✗", Ink::Bad)),
        }
    }

    /// [`Check::line`] as view text.
    pub fn text(&self, not_yet: &str, running: &str) -> crate::view::Text {
        use crate::view::{Ink, Seg, Text};
        let said = |mark: &str, said: &str, ink: Ink, ago: &Option<String>| {
            let mut t = Text::of(format!("{mark} {said}"), ink);
            if let Some(a) = ago {
                t.add(Seg::new(format!("  {a} ago"), Ink::Muted));
            }
            t
        };
        match self {
            Check::NotYet => Text::of(not_yet, Ink::Muted),
            Check::Running => Text::of(running, Ink::Info),
            Check::Passed { said: s, ago } => said("✓", s, Ink::Good, ago),
            Check::Failed { said: s, ago } => said("✗", s, Ink::Bad, ago),
        }
    }

    /// The line in its detail panel. `not_yet` and `running` say it in the
    /// part's words (`not checked yet · c checks it`, `asking the server…`).
    pub fn line(&self, theme: &Theme, not_yet: &str, running: &str) -> Line<'static> {
        let ago = |a: &Option<String>| {
            a.as_ref()
                .map(|a| Span::styled(format!("  {a} ago"), theme.s(Slot::TextMuted)))
        };
        match self {
            Check::NotYet => Line::styled(not_yet.to_string(), theme.s(Slot::TextMuted)),
            Check::Running => Line::styled(running.to_string(), theme.s(Slot::StatusInfo)),
            Check::Passed { said, ago: a } => Line::from(
                std::iter::once(Span::styled(format!("✓ {said}"), theme.s(Slot::StatusSuccess)))
                    .chain(ago(a))
                    .collect::<Vec<_>>(),
            ),
            Check::Failed { said, ago: a } => Line::from(
                std::iter::once(Span::styled(format!("✗ {said}"), theme.s(Slot::StatusError)))
                    .chain(ago(a))
                    .collect::<Vec<_>>(),
            ),
        }
    }
}

/// The verdict on what is being typed: `✓ …` or `✗ …`.
pub fn as_you_type(theme: &Theme, verdict: Result<&str, &str>) -> Line<'static> {
    match verdict {
        Ok(m) => Line::styled(format!("✓ {m}"), theme.s(Slot::StatusSuccess)),
        Err(m) => Line::styled(format!("✗ {m}"), theme.s(Slot::StatusWarning)),
    }
}

/// The verdict on what is being typed when it is taken but may not work
/// (a command not found here): `⚠ …`.
pub fn as_you_type_unsure(theme: &Theme, why: &str) -> Line<'static> {
    Line::styled(format!("⚠ {why}"), theme.s(Slot::StatusWarning))
}
