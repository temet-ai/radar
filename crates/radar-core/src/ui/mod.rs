//! What the host and the parts share that is not drawing: the state of a
//! composer, a log, a diff and a scroll, the icon packs, rich text and
//! markdown as a document, the connection section's words and the status
//! words. Drawing is the renderer's (Ink); nothing here names a terminal
//! library.
//!
//! | Element | For | Module |
//! |---|---|---|
//! | log | a long output with a cursor, folding sections, search | [`log`] |
//! | highlight | code coloured by language, as tokens per line | [`highlight`] |
//! | diff | a file's changes: hunks, both sides' line numbers, a cursor | [`diff`] |
//! | rich text | one document model (blocks, runs with marks, links, mentions, emoji, badges, media) | [`rich`] |
//! | markdown | text written in markdown (a description, a comment) parsed into rich text | [`markdown`] |
//! | rows | movement along selectable rows | [`rows`] |
//! | composer | text typed for a write: `⏎` a new line, `ctrl-s` review, `esc` stop | [`composer`] |
//! | connection | a part's connection section: its words, and what was found in the environment as a question | [`connection`] |
//! | status | counts in a row; a check's verdict | [`status`] |

pub mod composer;
pub mod connection;
pub mod diff;
pub mod highlight;
pub mod icon;
pub mod image;
pub mod link;
pub mod log;
pub mod markdown;
pub mod pointer;
pub mod rich;
pub mod rows;
pub mod scroll;
pub mod status;

pub use scroll::Scroll;

use crate::plugin::Tone;

pub use crate::design::width::{clusters, fit, pad, take, width, wrap_words};

/// `Alice Example` → `Alice E.`; one name stays as it is. A surname with a
/// particle (a lower-case word between the first name and the last) stays
/// whole, since an initial would drop what makes it theirs: `Maria de Souza`
/// stays `Maria de Souza`, not `Maria S.`.
pub fn short_name(name: &str) -> String {
    let parts: Vec<&str> = name.split_whitespace().collect();
    match parts.as_slice() {
        [] => String::new(),
        [one] => (*one).to_string(),
        [first, middle @ .., last] => {
            let particle = middle
                .iter()
                .any(|w| w.chars().next().is_some_and(char::is_lowercase));
            if particle {
                parts.join(" ")
            } else {
                format!("{first} {}.", last.chars().next().unwrap_or(' '))
            }
        }
    }
}

/// The detail pane's width beside a list in `total` cells: 30%, never under 38
/// nor over 60. One fit for every screen of every part (the nav pane's width is
/// its own).
pub fn detail_width(total: u16) -> u16 {
    (total * 30 / 100).clamp(38, 60)
}

/// `45s`, `6m 12s`, `1h 05m`.
pub fn duration(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}h {:02}m", secs / 3600, secs % 3600 / 60)
    } else if secs >= 60 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else {
        format!("{secs}s")
    }
}

/// The state of a run, a step or a check, as a symbol that reads without
/// colour. A part maps its own states onto these and styles them with its
/// own look words; [`Mark::tone`] is the default feeling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum Mark {
    Done,
    Failed,
    Running,
    Waiting,
    Manual,
    Stopped,
    Skipped,
}

impl Mark {
    pub fn symbol(self) -> &'static str {
        match self {
            Mark::Done => "✓",
            Mark::Failed => "✗",
            Mark::Running => "●",
            Mark::Waiting => "◷",
            Mark::Manual => "▷",
            Mark::Stopped => "⊘",
            Mark::Skipped => "»",
        }
    }

    /// Its icon ([`icon`]): a run's state.
    pub fn icon(self) -> icon::Icon {
        use icon::Icon;
        match self {
            Mark::Done => Icon::RunPassed,
            Mark::Failed => Icon::RunFailed,
            Mark::Running => Icon::RunRunning,
            Mark::Waiting => Icon::RunPending,
            Mark::Manual => Icon::RunManual,
            Mark::Stopped => Icon::RunCanceled,
            Mark::Skipped => Icon::RunSkipped,
        }
    }

    pub fn tone(self) -> Tone {
        match self {
            Mark::Done => Tone::Good,
            Mark::Failed => Tone::Bad,
            Mark::Running => Tone::Info,
            Mark::Manual => Tone::Accent,
            Mark::Waiting | Mark::Stopped | Mark::Skipped => Tone::Muted,
        }
    }
}
