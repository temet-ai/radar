//! The host's `icons` settings section (General › icons): every icon by
//! name with its glyph in the pack in use. `⏎` types a glyph of your own for
//! the selected icon (written to `[icons]`), `r` gives it back to the pack.
//! Which pack is the colours section's `icons` row.

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span};

use crate::colours::{Colours, Reply};
use crate::design::theme::Slot;
use crate::input::{Edit, TextInput};
use crate::plugin::{Binding, SectionSpec};
use crate::theme::Theme;
use crate::ui::icon::{self, Glyph, Icon, Icons};
use crate::ui::rows::{ListRow, Move, RowList, select};
use crate::widgets;

/// The host's settings section for icons, under general.
pub const ICONS_SECTION: SectionSpec = SectionSpec {
    id: "icons",
    name: "icons",
    group: "general",
};

/// A row of the section: a heading (an icon's group) or an icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconRow {
    Heading(&'static str),
    Icon(Icon),
}

/// Every row: each group's heading, then its icons.
pub fn rows() -> Vec<IconRow> {
    let mut out = Vec::new();
    let mut group = "";
    for &i in Icon::ALL {
        if i.group() != group {
            group = i.group();
            out.push(IconRow::Heading(group));
        }
        out.push(IconRow::Icon(i));
    }
    out
}

impl Colours {
    /// The icons as they are drawn now: the chosen pack, the overrides.
    pub fn icons_now(&self) -> Icons {
        icon::resolve(
            &self.cfg.icons,
            &icon::user_packs(),
            &self.cfg.icon_overrides,
        )
        .0
    }

    /// The icon on the selected row.
    pub fn icon_selected(&self) -> Icon {
        let rows = rows();
        let at = self.icon_row.min(rows.len() - 1);
        rows[at..]
            .iter()
            .find_map(|r| match r {
                IconRow::Icon(i) => Some(*i),
                IconRow::Heading(_) => None,
            })
            .unwrap_or(Icon::ALL[0])
    }

    /// Keys in the icons section.
    pub fn icons_key(&mut self, key: KeyEvent) -> Reply {
        let icon = self.icon_selected();
        if let Some(mut input) = self.edit.take() {
            return match input.key(key) {
                Edit::Cancel => Reply::Done,
                Edit::Typing => {
                    self.edit = Some(input);
                    Reply::Done
                }
                Edit::Submit(text) => {
                    let glyph = text.trim().to_string();
                    let width = self.icons_now().width.max(1);
                    let refused = if glyph.is_empty() {
                        Some("type a glyph, or r gives it back to the pack".to_string())
                    } else {
                        icon::check(&glyph, width).err()
                    };
                    match refused {
                        Some(why) => {
                            self.edit = Some(input);
                            Reply::Error(format!("not yet: {why}"))
                        }
                        None => {
                            // A style written by hand stays.
                            let style = self
                                .cfg
                                .icon_overrides
                                .get(icon.name())
                                .and_then(|g| g.style().map(str::to_string));
                            let g = match style {
                                Some(style) => Glyph::Styled {
                                    glyph: glyph.clone(),
                                    style: Some(style),
                                },
                                None => Glyph::new(glyph.clone()),
                            };
                            self.cfg.icon_overrides.insert(icon.name().to_string(), g);
                            Reply::Say(format!("{} is now {glyph}", icon.name()))
                        }
                    }
                }
            };
        }
        let rows = rows();
        if let Some(mv) = crate::key_compat::chord_of(&key).and_then(|c| Move::from_key(c, self.icon_scroll.height())) {
            self.icon_row = select(rows.len(), self.icon_row, mv, |i| {
                matches!(rows[i], IconRow::Icon(_))
            });
            return Reply::Done;
        }
        match key.code {
            KeyCode::Enter => {
                let now = self.icons_now();
                let start = self
                    .cfg
                    .icon_overrides
                    .get(icon.name())
                    .map(|g| g.text().to_string())
                    .unwrap_or_else(|| now.get(icon).glyph.trim_end().to_string());
                self.edit = Some(TextInput::new(&start));
                Reply::Done
            }
            KeyCode::Char('r') if self.cfg.icon_overrides.contains_key(icon.name()) => {
                self.cfg.icon_overrides.remove(icon.name());
                Reply::Say(format!("{}: the pack's again", icon.name()))
            }
            _ => Reply::Ignored,
        }
    }

    /// The section's `x` menu rows.
    pub fn icons_keys(&self) -> Vec<Binding> {
        let icon = self.icon_selected();
        let mut rows = vec![
            Binding::heading(icon.name()),
            Binding::new("⏎", "type your own glyph"),
        ];
        if self.cfg.icon_overrides.contains_key(icon.name()) {
            rows.push(Binding::new("r", "restore the pack's glyph"));
        }
        rows
    }

    pub fn icons_render(
        &self,
        theme: &Theme,
        inner: Rect,
        buf: &mut Buffer,
        focused: bool,
        cursor: &std::cell::Cell<Option<Position>>,
    ) -> Vec<(Rect, usize)> {
        let now = self.icons_now();
        let shown = now.width.max(1);
        let muted = theme.s(Slot::TextMuted);
        let list: Vec<ListRow> = rows()
            .into_iter()
            .map(|r| match r {
                IconRow::Heading(g) => ListRow::heading(Line::styled(g, muted)),
                IconRow::Icon(i) => {
                    let r = now.get(i);
                    let glyph = if r.glyph.is_empty() {
                        crate::ui::pad("·", shown)
                    } else {
                        r.glyph.clone()
                    };
                    let mut row = ListRow::new(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(glyph, theme.spec(&r.style)),
                        Span::raw("  "),
                        Span::styled(crate::ui::pad(i.name(), 18), theme.s(Slot::TextPrimary)),
                    ]))
                    .detail(Span::styled(
                        i.about().to_string(),
                        theme.s(Slot::TextSecondary),
                    ));
                    if self.cfg.icon_overrides.contains_key(i.name()) {
                        row = row.right(Span::styled("yours", theme.s(Slot::StatusWarning)));
                    }
                    row
                }
            })
            .collect();
        let drawn = RowList::new(list)
            .cursor(self.icon_row)
            .focused(focused)
            .render_in(theme, inner, buf, &self.icon_scroll);
        if let Some(input) = &self.edit
            && let Some((rect, _)) = drawn.iter().find(|(_, i)| *i == self.icon_row)
        {
            // The input takes the glyph's place on the row.
            let x = 3;
            let area = Rect {
                x: rect.x + x,
                width: rect.width.saturating_sub(x + 1),
                ..*rect
            };
            input.render(theme, area, buf, cursor);
        }
        drawn
    }

    pub fn icons_detail_title(&self) -> String {
        self.icon_selected().name().to_string()
    }

    pub fn icons_render_detail(&self, theme: &Theme, inner: Rect, buf: &mut Buffer) {
        widgets::paragraph(buf, inner, self.icons_detail_lines(theme));
    }

    /// The selected icon's detail, ready for the host to wrap and scroll.
    pub fn icons_detail_lines(&self, theme: &Theme) -> Vec<Line<'static>> {
        let icon = self.icon_selected();
        let now = self.icons_now();
        let r = now.get(icon);
        let text = theme.s(Slot::TextPrimary);
        let muted = theme.s(Slot::TextMuted);
        let mut lines = vec![
            Line::styled(format!("Shows {}.", icon.about()), text),
            Line::default(),
        ];
        let glyph = if r.glyph.is_empty() {
            "none".to_string()
        } else {
            r.glyph.trim_end().to_string()
        };
        lines.push(widgets::kv(
            theme,
            "now",
            10,
            vec![Span::styled(glyph, theme.spec(&r.style))],
        ));
        lines.push(widgets::kv(
            theme,
            "from",
            10,
            vec![Span::styled(r.from.clone(), text)],
        ));
        lines.push(widgets::kv(
            theme,
            "colour",
            10,
            vec![Span::styled(r.style.clone(), text)],
        ));
        lines.push(widgets::kv(
            theme,
            "pack",
            10,
            vec![Span::styled(self.icons_label(), text)],
        ));
        lines.push(Line::default());
        if !now.on() {
            lines.push(Line::styled(
                "Icons are off: pick a pack in General › colours, icons.",
                theme.s(Slot::StatusWarning),
            ));
        }
        lines.push(Line::styled(
            if self.cfg.icon_overrides.contains_key(icon.name()) {
                "Yours. ⏎ types another, r gives it back to the pack."
            } else {
                "The pack's. ⏎ types your own."
            },
            muted,
        ));
        lines.push(Line::styled(
            format!(
                "A glyph takes at most {} cell{} in this pack; joined emoji, skin tones and flags are refused.",
                now.width.max(1),
                if now.width > 1 { "s" } else { "" }
            ),
            muted,
        ));
        lines.push(Line::default());
        lines.push(Line::styled(
            format!("config: [icons] {}", icon.name()),
            muted,
        ));
        lines
    }

    /// Put the cursor on the first icon if it is on a heading.
    pub fn icons_first_row(&mut self) {
        let rows = rows();
        if !matches!(rows.get(self.icon_row), Some(IconRow::Icon(_))) {
            self.icon_row = select(rows.len(), 0, Move::First, |i| {
                matches!(rows[i], IconRow::Icon(_))
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyModifiers;

    fn press(c: &mut Colours, code: KeyCode) -> Reply {
        c.icons_key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn typed(c: &mut Colours, text: &str) -> Reply {
        press(c, KeyCode::Enter);
        let input = c.edit.as_mut().expect("typing");
        input.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        input.paste(text);
        press(c, KeyCode::Enter)
    }

    #[test]
    fn enter_types_a_glyph_that_is_written_to_the_icons_table_and_r_resets_it() {
        let mut c = Colours::default();
        c.cfg.icons = "unicode".into();
        c.icons_first_row();
        assert_eq!(c.icon_selected(), Icon::ALL[0]);
        press(&mut c, KeyCode::Down);
        let icon = c.icon_selected();
        assert!(matches!(typed(&mut c, "R"), Reply::Say(m) if m.contains("is now R")));
        assert!(c.edit.is_none());
        assert_eq!(c.cfg.icon_overrides[icon.name()], Glyph::new("R"));
        assert_eq!(c.icons_now().get(icon).glyph, "R");
        assert!(c.icons_keys().iter().any(|b| b.keys == "r"));
        assert!(matches!(press(&mut c, KeyCode::Char('r')), Reply::Say(_)));
        assert!(c.cfg.icon_overrides.is_empty());
    }

    #[test]
    fn a_glyph_too_wide_for_the_pack_is_refused_with_its_reason_and_typing_goes_on() {
        let mut c = Colours::default();
        c.cfg.icons = "nerd".into();
        c.icons_first_row();
        let Reply::Error(m) = typed(&mut c, "🐞") else {
            panic!("refused in words")
        };
        assert!(m.starts_with("not yet:") && m.contains("2 cells"), "{m}");
        assert!(c.edit.is_some(), "still typing");
        assert!(c.cfg.icon_overrides.is_empty());
        // In a pack of two cells it fits.
        c.edit = None;
        c.cfg.icons = "emoji".into();
        assert!(matches!(typed(&mut c, "🐞"), Reply::Say(m) if m.contains("is now")));
    }

    fn settings_host(pack: &str) -> crate::Host {
        let mut host = crate::Host::new(vec![], 0).unwrap();
        host.load_config(Some(&format!(
            "[theme]\nicons = \"{pack}\"\n[icons]\nbug = \"B\"\nrepo = {{ glyph = \"R\", style = \"accent.primary\" }}\n"
        )))
        .unwrap();
        host.view = crate::host::View::Settings;
        host.section = host
            .sections
            .iter()
            .position(|(_, s)| s.id == ICONS_SECTION.id)
            .unwrap();
        host.focus = crate::Pane::Middle;
        host
    }

    #[test]
    fn the_icons_section_lists_every_icon_with_its_glyph() {
        let host = settings_host("unicode");
        assert!(host.on_icons());
        insta::assert_snapshot!(
            "icons_section_unicode",
            crate::headless::host_text_of(&host, 160, 40)
        );
        let host = settings_host("nerd");
        insta::assert_snapshot!(
            "icons_section_nerd",
            crate::headless::host_text_of(&host, 160, 40)
        );
        assert!(
            host.config_text().contains("[icons]"),
            "{}",
            host.config_text()
        );
        assert!(host.config_text().contains("bug = \"B\""));
    }

    #[test]
    fn the_icons_row_offers_every_pack_with_a_preview() {
        // The pack is chosen in Settings / general / look & terminal now:
        // its card lists the packs, each with a preview, and space picks one.
        let mut host = settings_host("nerd");
        host.section = host
            .sections
            .iter()
            .position(|(o, _)| *o == crate::host::Owner::Look)
            .unwrap();
        let key = |host: &mut crate::Host, code| {
            host.update(crate::host::Event::Key(KeyEvent::new(
                code,
                KeyModifiers::NONE,
            )));
        };
        let text = crate::headless::host_text_of(&host, 160, 40);
        for (pack, needs) in [
            ("nerd", "needs a Nerd Font"),
            ("plain", "any font"),
            ("emoji", "wide; columns shift"),
        ] {
            let line = text
                .lines()
                .find(|l| l.contains(pack) && l.contains(needs))
                .unwrap_or_else(|| panic!("a row for {pack}:\n{text}"));
            let between = line.split(pack).nth(1).unwrap().split(needs).next().unwrap();
            assert!(
                between.trim().chars().count() >= 2,
                "{pack} has a preview: {line}"
            );
        }
        // Down once is the second pack, plain (unicode); space chooses it.
        key(&mut host, KeyCode::Down);
        key(&mut host, KeyCode::Char(' '));
        assert_eq!(host.colours.cfg.icons, "unicode");
        assert!(host.menu.is_none(), "no menu: the card is the choice");
        key(&mut host, KeyCode::Up);
        key(&mut host, KeyCode::Enter);
        assert_eq!(host.colours.cfg.icons, "nerd", "⏎ chooses too");
    }

    #[test]
    fn headings_are_stepped_over() {
        let mut c = Colours::default();
        c.icons_first_row();
        for _ in 0..rows().len() {
            press(&mut c, KeyCode::Down);
            assert!(matches!(rows()[c.icon_row], IconRow::Icon(_)));
        }
    }
}
