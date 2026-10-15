//! A part's `connection` settings section, the same in every part: what was
//! found in the environment (asked as a question), then where it connects,
//! the account and the token, who that signs in as, the check, how often it
//! refreshes and when it pauses. The words, the label column and the detail
//! panels live here so the parts read alike.
//!
//! ```text
//!  ? Found in your environment: a shelf token (SHELF_TOKEN). Use it?
//!  server         https://shelves.example
//!  token          env:SHELF_TOKEN
//!  signed in as   alice
//!  check          ✓ signed in as alice  2m ago
//!  refresh        every 1m
//!  pause if idle  after 30 minutes
//! ```

use ratatui::text::{Line, Span};

use super::status::{Check, as_you_type};
use crate::design::theme::Slot;
use crate::theme::Theme;
use crate::widgets;

/// The section's name in every part (each under its part's heading).
pub const SECTION: &str = "connection";

/// The label column: values line up in every part.
pub const LABEL_W: usize = 16;

/// The account a token belongs to, where the service asks for one.
pub const ACCOUNT: &str = "account";
pub const TOKEN: &str = "token";
pub const SIGNED_IN: &str = "signed in as";
/// The row that asks the service whether the connection works.
pub const CHECK: &str = "check";
pub const REFRESH: &str = "refresh";
pub const IDLE: &str = "pause if idle";

/// What the token row accepts, before the part's own ways.
pub const TOKEN_ACCEPTS: &str = "env:NAME reads an environment variable each time · or the token itself, stored in config.toml";
pub const IDLE_ACCEPTS: &str = "whole minutes; 0 never pauses";

/// `lead` (a space, or a tree joint), the label in its column, the value.
pub fn field(
    theme: &Theme,
    lead: Vec<Span<'static>>,
    label: &str,
    value: Vec<Span<'static>>,
) -> Line<'static> {
    let mut spans = lead;
    spans.extend(widgets::kv(theme, label, LABEL_W, value).spans);
    Line::from(spans)
}

/// [`field`] as view text.
pub fn field_text(lead: crate::view::Text, label: &str, value: crate::view::Text) -> crate::view::Text {
    let mut text = lead;
    text.extend(crate::view::kit::labelled(label, LABEL_W, value));
    text
}

/// [`check_field`] as view text.
pub fn check_field_text(lead: crate::view::Text, check: &Check, service: &str) -> crate::view::Text {
    field_text(lead, CHECK, check.text("not checked yet", &format!("asking {service}…")))
}

/// [`signed_in`] as view text.
pub fn signed_in_text(name: &str, handle: &str) -> crate::view::Text {
    use crate::view::{Ink, Seg, Text};
    let (name, handle) = (name.trim(), handle.trim());
    let first = if name.is_empty() { handle } else { name };
    let mut text = Text::of(first, Ink::Primary);
    if !name.is_empty() && !handle.is_empty() && handle != name {
        text.add(Seg::new(format!("  {handle}"), Ink::Muted));
    }
    text
}

/// Where a field's value, and so its text input, starts: after a lead of
/// `lead` cells.
pub const fn value_x(lead: usize) -> u16 {
    (lead + LABEL_W) as u16
}

/// The check row: what the last check said, or that none ran.
pub fn check_field(
    theme: &Theme,
    lead: Vec<Span<'static>>,
    check: &Check,
    service: &str,
) -> Line<'static> {
    let said = check.line(theme, "not checked yet", &format!("asking {service}…"));
    field(theme, lead, CHECK, said.spans)
}

/// A token as shown: empty as `—`; `env:NAME`, and the words in `named`
/// (a word that means "ask that program"), by name; a token typed in never
/// in full.
pub fn masked(token: &str, named: &[&str]) -> String {
    let t = token.trim();
    if t.is_empty() {
        "—".into()
    } else if t.starts_with("env:") || named.contains(&t) {
        t.to_string()
    } else if t.chars().count() >= 16 {
        let first: String = t.chars().take(4).collect();
        let last: String = t.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
        format!("{first}…{last} (typed in)")
    } else {
        "typed in, not shown".into()
    }
}

/// `every 1m`, `every 90s`, `every 2h`: how the top bar says it, and the
/// refresh row of settings.
pub fn every(secs: u64) -> String {
    match secs {
        s if s >= 3600 && s % 3600 == 0 => format!("every {}h", s / 3600),
        s if s >= 60 && s % 60 == 0 => format!("every {}m", s / 60),
        s => format!("every {s}s"),
    }
}

/// The signed-in row's value: the display name, then the handle muted
/// (`Alice Example  alice`). A handle equal to the name, or none, is shown
/// once.
pub fn signed_in(theme: &Theme, name: &str, handle: &str) -> Vec<Span<'static>> {
    let (name, handle) = (name.trim(), handle.trim());
    let first = if name.is_empty() { handle } else { name };
    let mut spans = vec![Span::styled(first.to_string(), theme.s(Slot::TextPrimary))];
    if !name.is_empty() && !handle.is_empty() && handle != name {
        spans.push(Span::styled(format!("  {handle}"), theme.s(Slot::TextMuted)));
    }
    spans
}

/// `never`, `after 1 minute`, `after 30 minutes`.
pub fn idle(minutes: u64) -> String {
    match minutes {
        0 => "never".into(),
        1 => "after 1 minute".into(),
        m => format!("after {m} minutes"),
    }
}

/// What the rows every part has are, in the same words.
pub fn about_signed_in(service: &str) -> String {
    format!("Who {service} says the token belongs to, as the last check found.")
}

pub fn about_check(service: &str) -> String {
    format!(
        "Asks {service} whether the connection above works and who the token belongs to. radar offers it after you change the connection."
    )
}

pub fn about_refresh(service: &str) -> String {
    format!("How often radar asks {service} for what changed while it is open.")
}

pub fn about_idle(service: &str) -> String {
    format!(
        "Refreshing pauses itself when nobody has pressed a key or clicked for this long, so a forgotten window stops asking {service}. Any key resumes it; z pauses and resumes it by hand."
    )
}

pub fn refresh_accepts(at_least: u64) -> String {
    format!("whole seconds, at least {at_least}")
}

/// A connection row's detail panel, in the same order in every part.
#[derive(Debug, Default, Clone)]
pub struct Detail {
    /// What it is, in a sentence or two.
    pub about: String,
    /// Lines after it: facts the part has (a site's projects, a scope).
    pub more: Vec<Line<'static>>,
    pub accepts: Option<String>,
    /// The value now.
    pub now: Option<String>,
    /// The check's line, for rows the check confirms.
    pub checked: Option<Line<'static>>,
    /// The verdict on what is being typed, while typing.
    pub typing: Option<Result<String, String>>,
    /// A line on what a key does here (`⏎ asks now`).
    pub keys: Option<String>,
    /// `[shelves] url`
    pub config: Option<String>,
}

impl Detail {
    pub fn new(about: impl Into<String>) -> Self {
        Self {
            about: about.into(),
            ..Self::default()
        }
    }

    pub fn lines(self, theme: &Theme) -> Vec<Line<'static>> {
        let muted = theme.s(Slot::TextMuted);
        let mut lines = Vec::new();
        let block = |lines: &mut Vec<Line<'static>>, heading: &str, body: Line<'static>| {
            if !lines.is_empty() {
                lines.push(Line::default());
            }
            lines.push(widgets::heading(theme, heading));
            lines.push(body);
        };
        if let Some(v) = self.typing {
            block(
                &mut lines,
                "as you type",
                as_you_type(theme, v.as_deref().map_err(String::as_str)),
            );
        }
        if let Some(c) = self.checked {
            block(&mut lines, "checked", c);
        }
        if let Some(n) = self.now {
            block(
                &mut lines,
                "now",
                Line::styled(n, theme.bold(Slot::TextEmphasis)),
            );
        }
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        lines.push(Line::styled(self.about, theme.s(Slot::TextPrimary)));
        if !self.more.is_empty() {
            lines.push(Line::default());
            lines.extend(self.more);
        }
        if let Some(a) = self.accepts {
            block(
                &mut lines,
                "accepts",
                Line::styled(a, theme.s(Slot::TextSecondary)),
            );
        }
        if let Some(k) = self.keys {
            lines.push(Line::default());
            lines.push(Line::styled(k, muted));
        }
        if let Some(c) = self.config {
            lines.push(Line::default());
            lines.push(Line::styled(format!("config: {c}"), muted));
        }
        lines
    }
}

/// [`Detail`] as view text: the same blocks in the same order.
#[derive(Debug, Default, Clone)]
pub struct DetailText {
    pub about: String,
    pub more: Vec<crate::view::Text>,
    pub accepts: Option<String>,
    pub now: Option<String>,
    pub checked: Option<crate::view::Text>,
    pub typing: Option<Result<String, String>>,
    pub keys: Option<String>,
    pub config: Option<String>,
}

impl DetailText {
    pub fn new(about: impl Into<String>) -> Self {
        Self {
            about: about.into(),
            ..Self::default()
        }
    }

    pub fn text(self) -> Vec<crate::view::Text> {
        use crate::view::{Ink, Seg, Text, kit};
        let mut lines: Vec<Text> = Vec::new();
        let block = |lines: &mut Vec<Text>, heading: &str, body: Text| {
            if !lines.is_empty() {
                lines.push(Text::new());
            }
            lines.push(kit::heading(heading));
            lines.push(body);
        };
        if let Some(v) = self.typing {
            block(&mut lines, "as you type", kit::as_you_type(v.as_deref().map_err(String::as_str)));
        }
        if let Some(c) = self.checked {
            block(&mut lines, "checked", c);
        }
        if let Some(n) = self.now {
            block(&mut lines, "now", Text::new().push(Seg::new(n, Ink::Emphasis).strong()));
        }
        if !lines.is_empty() {
            lines.push(Text::new());
        }
        lines.push(Text::of(self.about, Ink::Primary));
        if !self.more.is_empty() {
            lines.push(Text::new());
            lines.extend(self.more);
        }
        if let Some(a) = self.accepts {
            block(&mut lines, "accepts", Text::of(a, Ink::Secondary));
        }
        if let Some(k) = self.keys {
            lines.push(Text::new());
            lines.push(Text::of(k, Ink::Muted));
        }
        if let Some(c) = self.config {
            lines.push(Text::new());
            lines.push(Text::of(format!("config: {c}"), Ink::Muted));
        }
        lines
    }
}

/// Something a part found that would set its connection up: a token in an
/// environment variable, a site's variables, a file that describes one.
/// Shown as a question at the top of the connection section; only names are
/// ever shown or stored, never the values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// What it is: `a shelf token`, `a shelf site`.
    pub what: String,
    /// The names found, each with what it holds: `("SHELF_X_TOKEN", "the token")`.
    pub names: Vec<(String, String)>,
    /// Where, when not the environment: a file's path (`~/work/.mcp.json`).
    pub file: Option<String>,
    /// What choosing it does: `sets the token to env:SHELF_TOKEN and checks it`.
    pub does: String,
    /// Anything else to know first.
    pub note: Option<String>,
}

impl Found {
    /// `your environment`, or the file.
    pub fn place(&self) -> String {
        self.file.clone().unwrap_or_else(|| "your environment".into())
    }

    fn listed(&self) -> String {
        self.names
            .iter()
            .map(|(n, _)| n.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// `Found in your environment: a shelf site (A, B, C). Use it?`
    pub fn question(&self) -> String {
        format!("Found in {}: {} ({}). Use it?", self.place(), self.what, self.listed())
    }

    /// The row of a [`RowList`](super::rows::RowList) `width` wide: the
    /// question as whole as fits. The names matter most, then where: all of
    /// them and where; the first and `…` and where; all without where; the
    /// first without where; then none (the detail panel lists them).
    pub fn row(&self, theme: &Theme, width: u16) -> Line<'static> {
        crate::view_tui::line(theme, &self.text(width))
    }

    /// [`Found::row`] as view text.
    pub fn text(&self, width: u16) -> crate::view::Text {
        use crate::view::{Ink, Seg, Text};
        // The list keeps a cell each side.
        let room = (width as usize).saturating_sub(2);
        let first = self.names.first().map(|(n, _)| n.as_str()).unwrap_or_default();
        let all = format!(" ({})", self.listed());
        let some = if self.names.len() > 1 { format!(" ({first}, …)") } else { all.clone() };
        let (all, some) = if self.names.is_empty() { (String::new(), String::new()) } else { (all, some) };
        let place = format!("Found in {}: ", self.place());
        let short = "Found: ".to_string();
        let tries = [
            (place.clone(), all.clone()),
            (place.clone(), some.clone()),
            (short.clone(), all),
            (short.clone(), some),
            (place, String::new()),
        ];
        let len = |(p, n): &(String, String)| 3 + super::width(p) + super::width(&self.what) + super::width(n) + 9;
        let (place, names) = tries
            .into_iter()
            .find(|t| len(t) <= room)
            .unwrap_or((short, String::new()));
        let mut text = Text::new()
            .push(Seg::new(" ? ", Ink::Accent).strong())
            .push(Seg::new(place, Ink::Secondary))
            .push(Seg::new(self.what.clone(), Ink::Primary).strong());
        if !names.is_empty() {
            text.add(Seg::new(names, Ink::Muted));
        }
        text.push(Seg::new(". Use it?", Ink::AccentSecondary).strong())
    }

    /// The detail panel's title.
    pub fn title(&self) -> String {
        format!("found in {}", self.place())
    }

    /// [`Found::detail`] as view text.
    pub fn detail_text(&self) -> Vec<crate::view::Text> {
        use crate::view::{Ink, Seg, Text, kit};
        let w = self.names.iter().map(|(n, _)| super::width(n)).max().unwrap_or(0) + 2;
        let mut lines = vec![
            Text::of(format!("radar found {} in {}. Do you want to use it?", self.what, self.place()), Ink::Primary),
            Text::new(),
            kit::heading("found"),
        ];
        for (name, holds) in &self.names {
            lines.push(
                Text::new()
                    .push(Seg::new(format!("{name:<w$}"), Ink::Emphasis).strong())
                    .push(Seg::new(holds.clone(), Ink::Secondary)),
            );
        }
        lines.push(Text::new());
        lines.push(Text::of(
            "radar stores only the variables' names (env:NAME) and reads their values each time it starts. It never shows the values or saves them.",
            Ink::Primary,
        ));
        if let Some(n) = &self.note {
            lines.push(Text::new());
            lines.push(Text::of(n.clone(), Ink::Primary));
        }
        lines.push(Text::new());
        lines.push(kit::heading("if you use it"));
        lines.push(Text::of(format!("radar {}.", self.does), Ink::Secondary));
        lines.push(Text::new());
        lines.push(Text::of("⏎ uses it", Ink::Muted));
        lines
    }

    /// The detail panel: what was found, what radar keeps of it, what choosing
    /// it does.
    pub fn detail(&self, theme: &Theme) -> Vec<Line<'static>> {
        let text = theme.s(Slot::TextPrimary);
        let muted = theme.s(Slot::TextMuted);
        let w = self
            .names
            .iter()
            .map(|(n, _)| super::width(n))
            .max()
            .unwrap_or(0)
            + 2;
        let mut lines = vec![
            Line::styled(
                format!("radar found {} in {}. Do you want to use it?", self.what, self.place()),
                text,
            ),
            Line::default(),
            widgets::heading(theme, "found"),
        ];
        for (name, holds) in &self.names {
            lines.push(widgets::kv(
                theme,
                "",
                0,
                vec![
                    Span::styled(format!("{name:<w$}"), theme.bold(Slot::TextEmphasis)),
                    Span::styled(holds.clone(), theme.s(Slot::TextSecondary)),
                ],
            ));
        }
        lines.push(Line::default());
        lines.push(Line::styled(
            "radar stores only the variables' names (env:NAME) and reads their values each time it starts. It never shows the values or saves them.",
            text,
        ));
        if let Some(n) = &self.note {
            lines.push(Line::default());
            lines.push(Line::styled(n.clone(), text));
        }
        lines.push(Line::default());
        lines.push(widgets::heading(theme, "if you use it"));
        lines.push(Line::styled(format!("radar {}.", self.does), theme.s(Slot::TextSecondary)));
        lines.push(Line::default());
        lines.push(Line::styled("⏎ uses it", muted));
        lines
    }
}
