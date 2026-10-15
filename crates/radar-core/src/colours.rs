//! The host's `colours` settings section: the theme. Which palette, how many
//! colours, and what each named colour looks like. (Which colour a plugin
//! uses for what is the plugin's own `look` section.)

use crate::design::theme::{Slot, SlotValue};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};

use crate::input::{Edit, TextInput};
use crate::motion::Level;
use crate::splash::Setting;
use crate::plugin::{Menu, MenuItem};
use crate::theme::{
    COLOR_LEVELS, SLOT_GROUPS, SWITCHES, Theme, ThemeConfig, color_level_label, palettes,
    parse_hex, slot_info, switch_label,
};
use crate::ui::icon;
use crate::widgets;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Header(&'static str),
    Palette,
    Depth,
    Motion,
    /// `[theme] splash`: the startup splash.
    Splash,
    /// `[theme] icons`: the icon pack.
    Icons,
    Slot(String),
    /// `[theme] hyperlinks`.
    Links,
    /// `[theme] images`.
    Images,
    /// `[theme] pointer`.
    Pointer,
}

/// What a key did in the colours section.
pub enum Reply {
    Ignored,
    Done,
    Menu(Menu),
    Say(String),
}

#[derive(Default)]
pub struct Colours {
    pub cfg: ThemeConfig,
    /// Slots plugins declared, with defaults.
    pub declared: Vec<(String, SlotValue)>,
    /// Index into [`Colours::rows`]; always a selectable row.
    pub row: usize,
    /// A `#rrggbb` being typed for the selected slot, or a glyph for the
    /// selected icon in the icons section.
    pub edit: Option<TextInput>,
    /// The row of the icons section ([`crate::icons`]).
    pub icon_row: usize,
    pub icon_scroll: crate::ui::Scroll,
}

impl Colours {
    /// The pack in words: its name, and what `auto` picked here.
    pub fn icons_label(&self) -> String {
        if self.cfg.icons == "auto" {
            format!("auto ({})", icon::auto_choice())
        } else {
            self.cfg.icons.clone()
        }
    }

    pub fn slot_names(&self) -> Vec<String> {
        Slot::ALL
            .iter()
            .map(|s| s.token_name().to_string())
            .chain(self.declared.iter().map(|(n, _)| n.clone()))
            .collect()
    }

    pub fn rows(&self) -> Vec<Row> {
        let mut rows = vec![
            Row::Header("theme"),
            Row::Palette,
            Row::Depth,
            Row::Motion,
            Row::Splash,
            Row::Icons,
        ];
        let names = self.slot_names();
        for group in SLOT_GROUPS {
            let in_group: Vec<&String> = names
                .iter()
                .filter(|n| slot_info(n).group == group)
                .collect();
            if !in_group.is_empty() {
                rows.push(Row::Header(group));
                rows.extend(in_group.into_iter().map(|n| Row::Slot(n.clone())));
            }
        }
        rows.extend([
            Row::Header("terminal"),
            Row::Links,
            Row::Images,
            Row::Pointer,
        ]);
        rows
    }

    pub fn selected(&self) -> Row {
        let rows = self.rows();
        let at = self.row.min(rows.len() - 1);
        match &rows[at] {
            Row::Header(_) => rows.into_iter().nth(at + 1).unwrap_or(Row::Palette),
            r => r.clone(),
        }
    }

    /// Move to the next selectable row, skipping headings.
    fn step(&mut self, down: bool) {
        let rows = self.rows();
        let mut i = self.row.min(rows.len() - 1);
        loop {
            let next = if down {
                i + 1
            } else {
                i.checked_sub(1).unwrap_or(i)
            };
            if next == i || next >= rows.len() {
                return;
            }
            i = next;
            if !matches!(rows[i], Row::Header(_)) {
                self.row = i;
                return;
            }
        }
    }

    pub fn first_row(&mut self) {
        if matches!(self.rows().get(self.row), Some(Row::Header(_)) | None) {
            self.row = 0;
            self.step(true);
        }
    }

    pub fn capturing(&self) -> bool {
        self.edit.is_some()
    }

    pub fn key(&mut self, key: KeyEvent) -> Reply {
        if let Some(mut input) = self.edit.take() {
            return match input.key(key) {
                Edit::Cancel => Reply::Done,
                Edit::Typing => {
                    self.edit = Some(input);
                    Reply::Done
                }
                Edit::Submit(text) => {
                    let Row::Slot(name) = self.selected() else {
                        return Reply::Done;
                    };
                    match parse_hex(&text) {
                        Some(_) => {
                            let v = text.trim().to_lowercase();
                            self.cfg.slots.insert(name.clone(), v.clone());
                            Reply::Say(format!("{} is now {v}", slot_info(&name).label))
                        }
                        None => {
                            self.edit = Some(input);
                            Reply::Say(
                                "not yet: a colour is # and six hex digits, e.g. #e135ff".into(),
                            )
                        }
                    }
                }
            };
        }
        if let Some(down) = widgets::vertical(&key) {
            self.step(down);
            return Reply::Done;
        }
        if key.code != KeyCode::Enter {
            return Reply::Ignored;
        }
        Reply::Menu(match self.selected() {
            Row::Palette => Menu::new(
                "palette",
                "palette",
                palettes()
                    .into_iter()
                    .map(|p| {
                        MenuItem::pick(
                            p.clone(),
                            if p == self.cfg.palette { "in use" } else { "" },
                            p,
                        )
                    })
                    .collect(),
            ),
            Row::Depth => Menu::new(
                "depth",
                "colours",
                COLOR_LEVELS
                    .iter()
                    .map(|l| {
                        let hint = if *l == self.cfg.colors { "in use" } else { "" };
                        MenuItem::pick(color_level_label(l), hint, *l)
                    })
                    .collect(),
            ),
            Row::Motion => Menu::new(
                "motion",
                "motion",
                Level::ALL
                    .iter()
                    .map(|l| {
                        let hint = if l.word() == self.cfg.motion {
                            format!("in use: {}", l.about())
                        } else {
                            l.about().to_string()
                        };
                        MenuItem::pick(l.word(), hint, l.word())
                    })
                    .collect(),
            ),
            Row::Splash => Menu::new(
                "splash",
                "splash",
                Setting::ALL
                    .iter()
                    .map(|s| {
                        let hint = if s.word() == Setting::parse(&self.cfg.splash).word() {
                            format!("in use: {}", s.about())
                        } else {
                            s.about().to_string()
                        };
                        MenuItem::pick(s.word(), hint, s.word())
                    })
                    .collect(),
            ),
            Row::Icons => {
                let user = icon::user_packs();
                let here = icon::auto_choice();
                let mut items = vec![MenuItem::pick(
                    "auto",
                    format!(
                        "{}by the terminal: {here}   {}",
                        if self.cfg.icons == "auto" {
                            "in use · "
                        } else {
                            ""
                        },
                        icon::preview(&here, &user)
                    ),
                    "auto",
                )];
                for name in icon::pack_names() {
                    let about = match user.iter().find(|p| p.name == name) {
                        Some(p) if !p.description.is_empty() => p.description.clone(),
                        Some(_) => "your pack".into(),
                        None => icon::built_in_about(&name).into(),
                    };
                    let preview = icon::preview(&name, &user);
                    let hint = format!(
                        "{}{}{about}",
                        if name == self.cfg.icons {
                            "in use · "
                        } else {
                            ""
                        },
                        if preview.is_empty() {
                            String::new()
                        } else {
                            format!("{preview}   ")
                        },
                    );
                    items.push(MenuItem::pick(name.clone(), hint, name));
                }
                Menu::new("icons", "icons", items)
            }
            Row::Slot(name) => {
                let mut items = vec![MenuItem::new(
                    'e',
                    "enter a colour",
                    "type it as #rrggbb",
                    "edit",
                )];
                if self.cfg.slots.contains_key(&name) {
                    items.push(MenuItem::new(
                        'r',
                        "use the palette's",
                        "drop your colour for this one",
                        "reset",
                    ));
                }
                Menu::new(format!("slot:{name}"), slot_info(&name).label, items)
            }
            Row::Links => Menu::new(
                "hyperlinks",
                "links you click",
                SWITCHES
                    .iter()
                    .map(|v| {
                        let hint = if *v == self.cfg.hyperlinks {
                            "in use"
                        } else {
                            ""
                        };
                        MenuItem::pick(switch_label(v), hint, *v)
                    })
                    .collect(),
            ),
            Row::Images => Menu::new(
                "images",
                "pictures",
                SWITCHES
                    .iter()
                    .map(|v| {
                        let hint = if *v == self.cfg.images { "in use" } else { "" };
                        MenuItem::pick(switch_label(v), hint, *v)
                    })
                    .collect(),
            ),
            Row::Pointer => Menu::new(
                "pointer",
                "the mouse pointer",
                SWITCHES
                    .iter()
                    .map(|v| {
                        let hint = if *v == self.cfg.pointer { "in use" } else { "" };
                        MenuItem::pick(switch_label(v), hint, *v)
                    })
                    .collect(),
            ),
            Row::Header(_) => return Reply::Ignored,
        })
    }

    pub fn on_menu(&mut self, menu: &str, value: &str) -> Option<String> {
        match menu {
            "palette" => {
                self.cfg.palette = value.to_string();
                Some(format!("palette {value}"))
            }
            "depth" => {
                self.cfg.colors = value.to_string();
                Some(format!("colours: {}", color_level_label(value)))
            }
            "hyperlinks" => {
                self.cfg.hyperlinks = value.to_string();
                Some(format!("links you click: {}", switch_label(value)))
            }
            "images" => {
                self.cfg.images = value.to_string();
                Some(format!("pictures: {}", switch_label(value)))
            }
            "pointer" => {
                self.cfg.pointer = value.to_string();
                Some(format!("the mouse pointer: {}", switch_label(value)))
            }
            "motion" => {
                self.cfg.motion = Level::parse(value).word().to_string();
                Some(format!("motion: {}", self.cfg.motion))
            }
            "splash" => {
                self.cfg.splash = Setting::parse(value).word().to_string();
                Some(format!("splash: {}", self.cfg.splash))
            }
            "icons" => {
                self.cfg.icons = value.to_string();
                Some(format!("icons: {}", self.icons_label()))
            }
            m => {
                let name = m.strip_prefix("slot:")?.to_string();
                match value {
                    "edit" => {
                        let start = self
                            .cfg
                            .slots
                            .get(&name)
                            .cloned()
                            .unwrap_or_else(|| "#".into());
                        self.edit = Some(TextInput::new(&start));
                    }
                    "reset" => {
                        self.cfg.slots.remove(&name);
                    }
                    _ => {}
                }
                None
            }
        }
    }

    pub fn render(
        &self,
        theme: &Theme,
        inner: Rect,
        buf: &mut Buffer,
        focused: bool,
        cursor: &std::cell::Cell<Option<ratatui::layout::Position>>,
    ) -> Vec<(Rect, usize)> {
        let rows = self.rows();
        let h = inner.height as usize;
        let offset = widgets::scroll_offset(self.row, h);
        // Where each selectable row was drawn, for clicks.
        let mut drawn = Vec::new();
        for (n, (i, row)) in rows.iter().enumerate().skip(offset).enumerate().take(h) {
            let y = inner.y + n as u16;
            let selected = i == self.row;
            let style = widgets::row_style(theme, focused, selected);
            if !matches!(row, Row::Header(_)) {
                drawn.push((
                    Rect {
                        y,
                        height: 1,
                        ..inner
                    },
                    i,
                ));
            }
            let (left, right) = match row {
                Row::Header(g) => {
                    widgets::row(
                        buf,
                        inner,
                        y,
                        widgets::row_style(theme, false, false),
                        Line::styled(*g, theme.s(Slot::TextMuted)),
                        None,
                    );
                    continue;
                }
                Row::Palette => (
                    widgets::kv(
                        theme,
                        "  palette",
                        16,
                        vec![Span::styled(
                            self.cfg.palette.clone(),
                            theme.s(Slot::TextPrimary),
                        )],
                    ),
                    None,
                ),
                Row::Depth => (
                    widgets::kv(
                        theme,
                        "  colours",
                        16,
                        vec![Span::styled(
                            color_level_label(&self.cfg.colors),
                            theme.s(Slot::TextPrimary),
                        )],
                    ),
                    None,
                ),
                Row::Motion => (
                    widgets::kv(
                        theme,
                        "  motion",
                        16,
                        vec![Span::styled(
                            Level::parse(&self.cfg.motion).word(),
                            theme.s(Slot::TextPrimary),
                        )],
                    ),
                    None,
                ),
                Row::Splash => (
                    widgets::kv(
                        theme,
                        "  splash",
                        16,
                        vec![Span::styled(
                            Setting::parse(&self.cfg.splash).word(),
                            theme.s(Slot::TextPrimary),
                        )],
                    ),
                    None,
                ),
                Row::Icons => (
                    widgets::kv(
                        theme,
                        "  icons",
                        16,
                        vec![Span::styled(self.icons_label(), theme.s(Slot::TextPrimary))],
                    ),
                    None,
                ),
                Row::Slot(name) => {
                    let mut spans = vec![
                        Span::raw("  "),
                        Span::styled("██ ", theme.style(name)),
                        Span::styled(
                            format!("{:<16}", slot_info(name).label),
                            theme.s(Slot::TextPrimary),
                        ),
                    ];
                    if let Some(input) = self.edit.as_ref().filter(|_| selected) {
                        // The input takes the value's place on this row.
                        widgets::row(buf, inner, y, style, Line::from(spans), None);
                        let x = inner.x + 23;
                        let rect = Rect {
                            x,
                            y,
                            width: inner.width.saturating_sub(24),
                            height: 1,
                        };
                        input.render(theme, rect, buf, cursor);
                        continue;
                    }
                    spans.push(Span::styled(theme.hex(name), theme.s(Slot::TextSecondary)));
                    let mine = self
                        .cfg
                        .slots
                        .contains_key(name)
                        .then(|| Span::styled("yours", theme.s(Slot::StatusWarning)));
                    (Line::from(spans), mine)
                }
                Row::Links => (
                    widgets::kv(
                        theme,
                        "  links",
                        16,
                        vec![Span::styled(
                            switch_label(&self.cfg.hyperlinks),
                            theme.s(Slot::TextPrimary),
                        )],
                    ),
                    None,
                ),
                Row::Images => (
                    widgets::kv(
                        theme,
                        "  pictures",
                        16,
                        vec![Span::styled(
                            switch_label(&self.cfg.images),
                            theme.s(Slot::TextPrimary),
                        )],
                    ),
                    None,
                ),
                Row::Pointer => (
                    widgets::kv(
                        theme,
                        "  pointer",
                        16,
                        vec![Span::styled(
                            switch_label(&self.cfg.pointer),
                            theme.s(Slot::TextPrimary),
                        )],
                    ),
                    None,
                ),
            };
            widgets::row(buf, inner, y, style, left, right);
        }
        drawn
    }

    pub fn detail_title(&self) -> String {
        match self.selected() {
            Row::Palette => "palette".into(),
            Row::Depth => "colours".into(),
            Row::Motion => "motion".into(),
            Row::Splash => "splash".into(),
            Row::Icons => "icons".into(),
            Row::Slot(name) => slot_info(&name).label,
            Row::Links => "links you click".into(),
            Row::Images => "pictures".into(),
            Row::Pointer => "the mouse pointer".into(),
            Row::Header(g) => g.into(),
        }
    }

    pub fn render_detail(&self, theme: &Theme, inner: Rect, buf: &mut Buffer) {
        widgets::paragraph(buf, inner, self.detail_lines(theme));
    }

    /// The selected setting's detail, ready for the host to wrap and scroll.
    pub fn detail_lines(&self, theme: &Theme) -> Vec<Line<'static>> {
        let muted = theme.s(Slot::TextMuted);
        let text = theme.s(Slot::TextPrimary);
        let mut lines: Vec<Line<'static>> = Vec::new();
        match self.selected() {
            Row::Palette => {
                lines.push(Line::styled(
                    "The set of colours everything starts from.",
                    text,
                ));
                lines.push(Line::default());
                lines.push(widgets::kv(
                    theme,
                    "in use",
                    10,
                    vec![Span::styled(
                        self.cfg.palette.clone(),
                        theme.s(Slot::TextEmphasis),
                    )],
                ));
                lines.push(widgets::kv(
                    theme,
                    "choices",
                    10,
                    vec![Span::styled(format!("{} palettes", palettes().len()), text)],
                ));
                lines.push(Line::default());
                lines.push(Line::styled(
                    "Colours you set below stay set when you change palette.",
                    muted,
                ));
            }
            Row::Depth => {
                lines.push(Line::styled("How many colours radar uses.", text));
                lines.push(Line::default());
                for l in COLOR_LEVELS {
                    let style = if l == self.cfg.colors {
                        theme.bold(Slot::TextEmphasis)
                    } else {
                        text
                    };
                    lines.push(Line::styled(format!("  {}", color_level_label(l)), style));
                }
                lines.push(Line::default());
                lines.push(Line::styled(
                    format!("Drawing with: {}", tier_label(theme)),
                    muted,
                ));
                lines.push(Line::styled(
                    "NO_COLOR in the environment always wins.",
                    muted,
                ));
            }
            Row::Motion => {
                let level = Level::parse(&self.cfg.motion);
                lines.push(Line::styled(
                    "Short effects that show what changed: a screen fading in, a menu \
                     sweeping open, rows a sync changed glowing for a moment.",
                    text,
                ));
                lines.push(Line::default());
                for l in Level::ALL {
                    let style = if l == level {
                        theme.bold(Slot::TextEmphasis)
                    } else {
                        text
                    };
                    lines.push(Line::from(vec![
                        Span::styled(format!("  {:<8}", l.word()), style),
                        Span::styled(l.about(), muted),
                    ]));
                }
                lines.push(Line::default());
                let here = crate::motion::held_back(theme.tier(), crate::system::Env::detect().ssh);
                lines.push(match (level, here) {
                    (Level::Off, _) => Line::styled("Nothing moves.", muted),
                    (_, Some(why)) => Line::styled(
                        format!("Off here all the same: {why}."),
                        theme.s(Slot::StatusWarning),
                    ),
                    _ => Line::styled("A key always acts at once; the effect gives way.", muted),
                });
                lines.push(Line::default());
                lines.push(Line::styled("config: [theme] motion", muted));
            }
            Row::Splash => {
                let setting = Setting::parse(&self.cfg.splash);
                lines.push(Line::styled(
                    "The radar mark while radar starts, with the parts in this build lighting as each is ready; \
                     then the first screen fades in. A key or click goes straight to the screen and acts.",
                    text,
                ));
                lines.push(Line::default());
                for s in Setting::ALL {
                    let style = if s == setting { theme.bold(Slot::TextEmphasis) } else { text };
                    lines.push(Line::from(vec![
                        Span::styled(format!("  {:<8}", s.word()), style),
                        Span::styled(s.about(), muted),
                    ]));
                }
                lines.push(Line::default());
                lines.push(Line::styled(
                    "How it moves follows motion: with motion off it shows only when radar is slow to start.",
                    muted,
                ));
                lines.push(Line::default());
                lines.push(Line::styled("config: [theme] splash", muted));
            }
            Row::Icons => {
                lines.push(Line::styled(
                    "Icons beside names and states, from a pack. Packs extend each other; what one leaves out comes from the one it extends, and in the end from unicode.",
                    text,
                ));
                lines.push(Line::default());
                let user = icon::user_packs();
                let mut names = vec!["auto".to_string()];
                names.extend(icon::pack_names());
                for name in names {
                    let style = if name == self.cfg.icons {
                        theme.bold(Slot::TextEmphasis)
                    } else {
                        text
                    };
                    let shown = if name == "auto" {
                        icon::auto_choice()
                    } else {
                        name.clone()
                    };
                    lines.push(Line::from(vec![
                        Span::styled(format!("  {}", crate::ui::pad(&name, 10)), style),
                        Span::styled(icon::preview(&shown, &user), text),
                    ]));
                }
                lines.push(Line::default());
                lines.push(Line::styled(
                    format!("auto here: {}. Nerd Font symbols where the terminal brings them (Ghostty, kitty, WezTerm), else unicode; unicode over SSH.", icon::auto_choice()),
                    muted,
                ));
                lines.push(Line::styled(
                    "Your packs: <name>.toml in the icons folder beside config.toml (radar icons path), read when radar starts. One icon at a time: General › icons.",
                    muted,
                ));
                lines.push(Line::default());
                lines.push(Line::styled("config: [theme] icons", muted));
            }
            Row::Slot(name) => {
                let info = slot_info(&name);
                if let Some(t) = self.edit.as_ref().map(TextInput::value) {
                    lines.push(match parse_hex(&t) {
                        Some(_) => Line::styled("⏎ keeps it", theme.s(Slot::StatusSuccess)),
                        None => Line::styled(
                            "Type # and six hex digits, e.g. #e135ff",
                            theme.s(Slot::StatusWarning),
                        ),
                    });
                    lines.push(Line::default());
                }
                lines.push(Line::styled(format!("Used for {}.", info.meaning), text));
                lines.push(Line::default());
                lines.push(Line::from(vec![
                    Span::styled("██████ ", theme.style(&name)),
                    Span::styled(theme.hex(&name), text),
                ]));
                if self.edit.is_none() {
                    lines.push(Line::default());
                }
                if let Some(v) = self.cfg.slots.get(&name).filter(|_| self.edit.is_none()) {
                    lines.push(widgets::kv(
                        theme,
                        "yours",
                        10,
                        vec![Span::styled(v.clone(), text)],
                    ));
                    lines.push(Line::styled(
                        "⏎ to change it or go back to the palette's.",
                        muted,
                    ));
                } else if self.edit.is_none() {
                    lines.push(Line::styled("From the palette. ⏎ to set your own.", muted));
                }
                lines.push(Line::default());
                lines.push(Line::styled(
                    format!("config: [theme.slots] \"{name}\""),
                    muted,
                ));
            }
            Row::Links => {
                lines.push(Line::styled(
                    "Links in text open on a click (Ctrl, or Cmd on a Mac): the terminal opens them. o and the Links tabs open them too, whatever this says.",
                    text,
                ));
                lines.push(Line::default());
                for v in SWITCHES {
                    let style = if v == self.cfg.hyperlinks {
                        theme.bold(Slot::TextEmphasis)
                    } else {
                        text
                    };
                    lines.push(Line::styled(format!("  {}", switch_label(v)), style));
                }
                lines.push(Line::default());
                lines.push(Line::styled(
                    if crate::ui::link::on() {
                        "Drawing links: yes."
                    } else {
                        "Drawing links: no."
                    },
                    muted,
                ));
                lines.push(Line::styled(
                    "Where the terminal can: Ghostty, kitty, WezTerm, iTerm2, GNOME's and Windows Terminal, and terminals that pass them on. Not inside tmux, nor over ssh unless the terminal says what it is.",
                    muted,
                ));
                lines.push(Line::default());
                lines.push(Line::styled("config: [theme] hyperlinks", muted));
            }
            Row::Images => {
                lines.push(Line::styled(
                    "Pictures in messages and descriptions, drawn in the terminal. Where it cannot, a line names each and its link.",
                    text,
                ));
                lines.push(Line::default());
                for v in SWITCHES {
                    let style = if v == self.cfg.images {
                        theme.bold(Slot::TextEmphasis)
                    } else {
                        text
                    };
                    lines.push(Line::styled(format!("  {}", switch_label(v)), style));
                }
                lines.push(Line::default());
                lines.push(Line::styled(
                    if crate::ui::image::on() {
                        "Drawing pictures: yes."
                    } else {
                        "Drawing pictures: no."
                    },
                    muted,
                ));
                lines.push(Line::styled(
                    "radar asks the terminal when it starts (kitty's, sixel or iTerm2's graphics): turning them on takes effect then. Not inside tmux unless on.",
                    muted,
                ));
                lines.push(Line::default());
                lines.push(Line::styled("config: [theme] images", muted));
            }
            Row::Pointer => {
                lines.push(Line::styled(
                    "The mouse pointer turns into a hand over what a click acts on, and into a text cursor while you drag to select. What it is over is lit either way.",
                    text,
                ));
                lines.push(Line::default());
                for v in SWITCHES {
                    let style = if v == self.cfg.pointer {
                        theme.bold(Slot::TextEmphasis)
                    } else {
                        text
                    };
                    lines.push(Line::styled(format!("  {}", switch_label(v)), style));
                }
                lines.push(Line::default());
                lines.push(Line::styled(
                    if crate::ui::pointer::on() {
                        "Changing the pointer: yes."
                    } else {
                        "Changing the pointer: no."
                    },
                    muted,
                ));
                lines.push(Line::styled(
                    "Where the terminal can: Ghostty, kitty, foot and WezTerm. Not inside tmux unless on.",
                    muted,
                ));
                lines.push(Line::default());
                lines.push(Line::styled("config: [theme] pointer", muted));
            }
            Row::Header(_) => {}
        }
        lines
    }
}

fn tier_label(theme: &Theme) -> &'static str {
    use crate::design::theme::Tier;
    match theme.tier() {
        Tier::TrueColor => "full colour",
        Tier::Ansi256 => "256 colours",
        Tier::Ansi16 => "16 colours",
        Tier::Mono => "no colour",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyModifiers;

    fn press(c: &mut Colours, code: KeyCode) -> Reply {
        c.key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn rows_are_grouped_and_movement_skips_headings() {
        let mut c = Colours::default();
        c.first_row();
        assert_eq!(c.selected(), Row::Palette);
        press(&mut c, KeyCode::Char('j'));
        assert_eq!(c.selected(), Row::Depth);
        press(&mut c, KeyCode::Char('j'));
        assert_eq!(c.selected(), Row::Motion);
        press(&mut c, KeyCode::Char('j'));
        assert_eq!(c.selected(), Row::Splash);
        press(&mut c, KeyCode::Char('j'));
        assert_eq!(c.selected(), Row::Icons);
        press(&mut c, KeyCode::Char('j'));
        assert_eq!(
            c.selected(),
            Row::Slot("bg.base".into()),
            "skipped the backgrounds heading"
        );
        assert!(c.rows().contains(&Row::Header("status")));
    }

    #[test]
    fn enter_opens_a_menu_and_the_choice_applies() {
        let mut c = Colours::default();
        c.first_row();
        press(&mut c, KeyCode::Char('j'));
        assert!(
            matches!(press(&mut c, KeyCode::Enter), Reply::Menu(m) if m.id == "depth" && m.items.len() == 5)
        );
        c.on_menu("depth", "16");
        assert_eq!(c.cfg.colors, "16");
    }

    #[test]
    fn links_are_the_last_row_and_choose_auto_on_or_off() {
        let mut c = Colours::default();
        c.row = c.rows().len() - 3;
        assert_eq!(c.selected(), Row::Links);
        assert!(
            matches!(press(&mut c, KeyCode::Enter), Reply::Menu(m) if m.id == "hyperlinks" && m.items.len() == 3)
        );
        assert_eq!(
            c.on_menu("hyperlinks", "off").as_deref(),
            Some("links you click: off")
        );
        assert_eq!(c.cfg.hyperlinks, "off");
        let text = toml::to_string(&c.cfg).unwrap();
        assert!(text.contains("hyperlinks = \"off\""), "{text}");
        let back: ThemeConfig = toml::from_str("palette = \"x\"").unwrap();
        assert_eq!(back.hyperlinks, "auto");
        assert!(!toml::to_string(&back).unwrap().contains("hyperlinks"));
    }

    #[test]
    fn pictures_are_the_last_row_and_choose_auto_on_or_off() {
        let mut c = Colours::default();
        c.row = c.rows().len() - 2;
        assert_eq!(c.selected(), Row::Images);
        assert!(
            matches!(press(&mut c, KeyCode::Enter), Reply::Menu(m) if m.id == "images" && m.items.len() == 3)
        );
        assert_eq!(c.on_menu("images", "off").as_deref(), Some("pictures: off"));
        assert!(
            toml::to_string(&c.cfg)
                .unwrap()
                .contains("images = \"off\"")
        );
        assert_eq!(ThemeConfig::default().images, "auto");
    }

    #[test]
    fn the_pointer_is_the_last_row_and_chooses_auto_on_or_off() {
        let mut c = Colours::default();
        c.row = c.rows().len() - 1;
        assert_eq!(c.selected(), Row::Pointer);
        assert!(
            matches!(press(&mut c, KeyCode::Enter), Reply::Menu(m) if m.id == "pointer" && m.items.len() == 3)
        );
        assert_eq!(
            c.on_menu("pointer", "off").as_deref(),
            Some("the mouse pointer: off")
        );
        assert!(
            toml::to_string(&c.cfg)
                .unwrap()
                .contains("pointer = \"off\"")
        );
        assert_eq!(ThemeConfig::default().pointer, "auto");
        assert!(
            !toml::to_string(&ThemeConfig::default())
                .unwrap()
                .contains("pointer")
        );
    }

    #[test]
    fn motion_is_chosen_from_a_menu_saying_what_each_shows() {
        let mut c = Colours::default();
        c.first_row();
        assert_eq!(c.cfg.motion, "subtle", "subtle by default");
        press(&mut c, KeyCode::Char('j'));
        press(&mut c, KeyCode::Char('j'));
        let Reply::Menu(m) = press(&mut c, KeyCode::Enter) else {
            panic!("a menu")
        };
        assert_eq!(m.id, "motion");
        let words: Vec<_> = m.items.iter().map(|i| i.value.as_str()).collect();
        assert_eq!(words, ["full", "subtle", "off"]);
        assert!(m.items[1].hint.starts_with("in use"));
        assert_eq!(c.on_menu("motion", "off").as_deref(), Some("motion: off"));
        assert_eq!(c.cfg.motion, "off");
        let back: ThemeConfig = toml::from_str(&toml::to_string(&c.cfg).unwrap()).unwrap();
        assert_eq!(back.motion, "off", "it is saved under [theme]");
        let old: ThemeConfig = toml::from_str("palette = \"x\"").unwrap();
        assert_eq!(old.motion, "subtle", "a file without it reads as subtle");
    }

    #[test]
    fn a_slot_colour_is_typed_checked_and_reset() {
        let mut c = Colours::default();
        c.first_row();
        press(&mut c, KeyCode::Char('j'));
        press(&mut c, KeyCode::Char('j'));
        press(&mut c, KeyCode::Char('j'));
        press(&mut c, KeyCode::Char('j'));
        press(&mut c, KeyCode::Char('j'));
        c.on_menu("slot:bg.base", "edit");
        for ch in "12zz".chars() {
            press(&mut c, KeyCode::Char(ch));
        }
        assert_eq!(
            c.edit.as_ref().map(TextInput::value).as_deref(),
            Some("#12zz")
        );
        assert!(
            matches!(press(&mut c, KeyCode::Enter), Reply::Say(m) if m.contains("six hex digits")),
            "refused, and still editing"
        );
        press(&mut c, KeyCode::Backspace);
        press(&mut c, KeyCode::Backspace);
        for ch in "3456".chars() {
            press(&mut c, KeyCode::Char(ch));
        }
        press(&mut c, KeyCode::Enter);
        assert_eq!(
            c.cfg.slots.get("bg.base").map(String::as_str),
            Some("#123456")
        );
        assert!(matches!(press(&mut c, KeyCode::Enter), Reply::Menu(m) if m.items.len() == 2));
        c.on_menu("slot:bg.base", "reset");
        assert!(c.cfg.slots.is_empty());
    }

    #[test]
    fn a_slot_typing_verdict_precedes_its_explanation() {
        let mut c = Colours::default();
        c.row = c
            .rows()
            .iter()
            .position(|r| *r == Row::Slot("bg.base".into()))
            .unwrap();
        let theme = Theme::new(crate::design::theme::Tier::Mono);
        for (input, expected) in [
            ("#123456", "⏎ keeps it"),
            ("#12zz", "Type # and six hex digits, e.g. #e135ff"),
        ] {
            c.edit = Some(TextInput::new(input));
            let lines = c.detail_lines(&theme);
            assert_eq!(lines[0].to_string(), expected);
            assert!(lines[2].to_string().starts_with("Used for "));
            assert_eq!(
                lines.last().unwrap().to_string(),
                "config: [theme.slots] \"bg.base\""
            );
        }
    }
}
