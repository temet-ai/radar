//! A part's sync words and a check's verdict: a mark for a row and text for
//! its detail panel.

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
    pub fn mark_text(&self) -> Option<crate::view::Text> {
        use crate::view::{Ink, Seg, Text};
        match self {
            Check::NotYet => None,
            Check::Running => Some(Text::of("checking", Ink::Info)),
            // The mark is all the row says, so with icons off the symbol stays.
            Check::Passed { .. } => Some(Text::new().push(Seg::icon_or(super::icon::Icon::RunPassed, "✓", Ink::Good))),
            Check::Failed { .. } => Some(Text::new().push(Seg::icon_or(super::icon::Icon::Error, "✗", Ink::Bad))),
        }
    }

    /// The line in its detail panel. `not_yet` and `running` say it in the
    /// part's words (`not checked yet · c checks it`, `asking the server…`).
    pub fn text(&self, not_yet: &str, running: &str) -> crate::view::Text {
        use crate::view::{Ink, Seg, Text};
        let said = |mark: super::icon::Icon, said: &str, ink: Ink, ago: &Option<String>| {
            let mut t = Text::new().push(Seg::icon(mark)).push(Seg::new(said, ink));
            if let Some(a) = ago {
                t.add(Seg::new(format!("  {a} ago"), Ink::Muted));
            }
            t
        };
        match self {
            Check::NotYet => Text::of(not_yet, Ink::Muted),
            Check::Running => Text::of(running, Ink::Info),
            Check::Passed { said: s, ago } => said(super::icon::Icon::RunPassed, s, Ink::Good, ago),
            Check::Failed { said: s, ago } => said(super::icon::Icon::Error, s, Ink::Bad, ago),
        }
    }
}
