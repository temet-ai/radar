//! The host's `colours` settings section: the theme. Which palette, how many
//! colours, and what each named colour looks like. (Which colour a plugin
//! uses for what is the plugin's own `look` section.)

use crate::design::theme::{Slot, SlotValue};
use crate::input::{Edit, TextInput};
use crate::motion::Level;
use crate::splash::Setting;
use crate::plugin::{Menu, MenuItem};
use crate::theme::{
    COLOR_LEVELS, SLOT_GROUPS, SWITCHES, ThemeConfig, color_level_label, palettes, parse_hex,
    slot_info, switch_label,
};
use crate::ui::icon;
use crate::view::{Chord, Key};

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
    Error(String),
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
            // The palette, motion and icon pack are the look section's
            // (Settings / general / appearance), as in setup.
            Row::Header("theme"),
            Row::Depth,
            Row::Splash,
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
            Row::Header(_) => rows.into_iter().nth(at + 1).unwrap_or(Row::Depth),
            r => r.clone(),
        }
    }

    /// Move to the next selectable row, skipping headings.
    /// Select row `i` of the colours section, as a renderer's press or
    /// movement does; false for a heading or a row past the end.
    pub fn select(&mut self, i: usize) -> bool {
        let ok = self.rows().get(i).is_some_and(|r| !matches!(r, Row::Header(_)));
        if ok {
            self.row = i;
        }
        ok
    }

    /// Select row `n` of the icons section; false for a heading or past the end.
    pub fn select_icon(&mut self, n: usize) -> bool {
        let ok = matches!(crate::icons::rows().get(n), Some(crate::icons::IconRow::Icon(_)));
        if ok {
            self.icon_row = n;
        }
        ok
    }

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

    pub fn key(&mut self, key: Chord) -> Reply {
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
                            Reply::Error(
                                "not yet: a colour is # and six hex digits, e.g. #e135ff".into(),
                            )
                        }
                    }
                }
            };
        }
        if let Some(down) = key.vertical() {
            self.step(down);
            return Reply::Done;
        }
        if key.key != Key::Enter {
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
                        "restore the palette's",
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

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::Key as KeyCode;

    fn press(c: &mut Colours, code: KeyCode) -> Reply {
        c.key(Chord::new(code))
    }

    #[test]
    fn rows_are_grouped_and_movement_skips_headings() {
        let mut c = Colours::default();
        c.first_row();
        assert_eq!(c.selected(), Row::Depth, "the palette, motion and icons are the look section's");
        for gone in [Row::Palette, Row::Motion, Row::Icons] {
            assert!(!c.rows().contains(&gone), "{gone:?} is not a colours row");
        }
        press(&mut c, KeyCode::Char('j'));
        assert_eq!(c.selected(), Row::Splash);
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
    fn motion_is_chosen_by_word_and_saved_under_theme() {
        // The levels and what each shows are checked through the look
        // section (setup tests); here the menu's choice and the file.
        let mut c = Colours::default();
        assert_eq!(c.cfg.motion, "subtle", "subtle by default");
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
        press(&mut c, KeyCode::Char('j')); // splash
        press(&mut c, KeyCode::Char('j')); // the first slot
        c.on_menu("slot:bg.base", "edit");
        for ch in "12zz".chars() {
            press(&mut c, KeyCode::Char(ch));
        }
        assert_eq!(
            c.edit.as_ref().map(TextInput::value).as_deref(),
            Some("#12zz")
        );
        assert!(
            matches!(press(&mut c, KeyCode::Enter), Reply::Error(m) if m.contains("six hex digits")),
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
        let mut host = crate::Host::new(vec![], 0).unwrap();
        host.load_config(Some("")).unwrap();
        host.view = crate::host::View::Settings;
        host.section = host
            .sections
            .iter()
            .position(|(o, s)| *o == crate::host::Owner::Host && s.id == "colours")
            .expect("the colours section");
        host.colours.row = host
            .colours
            .rows()
            .iter()
            .position(|r| *r == Row::Slot("bg.base".into()))
            .unwrap();
        for (input, expected) in [
            ("#123456", "⏎ keeps it"),
            ("#12zz", "Type # and six hex digits, e.g. #e135ff"),
        ] {
            host.colours.edit = Some(TextInput::new(input));
            let view = crate::settings_view::section_view(&host, crate::design::theme::Tier::Mono)
                .expect("the colours view");
            let lines: Vec<String> = view.detail.iter().map(|t| t.words()).collect();
            assert_eq!(lines[0], expected);
            assert!(lines[2].starts_with("Used for "), "{lines:?}");
            assert_eq!(lines.last().unwrap(), "config: [theme.slots] \"bg.base\"");
        }
    }
}
