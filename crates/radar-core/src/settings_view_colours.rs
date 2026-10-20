//! The host's colours and icons settings sections as view data: a row per
//! colour slot or icon, and the detail of the row selected.

use super::{kv, section};
use crate::colours::{Colours, Row};
use crate::design::theme::Tier;
use crate::host::Host;
use crate::icons::{self, IconRow};
use crate::motion::Level;
use crate::splash::Setting;
use crate::theme::{
    COLOR_LEVELS, SWITCHES, Theme, color_level_label, palettes, parse_hex, slot_info, switch_label,
};
use crate::ui::icon;
use crate::view::{Editing, Ink, ListRow, Node, SectionView, Seg, Text};

fn tier_label(theme: &Theme) -> &'static str {
    match theme.tier() {
        Tier::TrueColor => "full colour",
        Tier::Ansi256 => "256 colours",
        Tier::Ansi16 => "16 colours",
        Tier::Mono => "no colour",
    }
}

fn muted(s: impl Into<String>) -> Text {
    Text::of(s, Ink::Muted)
}

fn plain(s: impl Into<String>) -> Text {
    Text::of(s, Ink::Primary)
}

/// A choice in a detail list: the one in use bold in the emphasis colour.
fn choice(s: String, on: bool) -> Text {
    if on { Text::new().push(Seg::new(s, Ink::Emphasis).strong()) } else { plain(s) }
}

fn kv_row(label: &str, value: String) -> Text {
    kv(label, 16, Seg::new(value, Ink::Primary))
}

pub(super) fn colours(host: &Host, scope: &str, tier: Tier) -> SectionView {
    let c = &host.colours;
    let theme = Theme::build(&c.cfg, tier, &c.declared);
    let mut editing = None;
    let rows: Vec<ListRow> = c
        .rows()
        .into_iter()
        .enumerate()
        .map(|(i, row)| {
            let target = format!("row:{i}");
            match row {
                Row::Header(g) => ListRow::heading(target, muted(g)),
                Row::Palette => ListRow::new(target, kv_row("  palette", c.cfg.palette.clone())),
                Row::Depth => ListRow::new(target, kv_row("  colours", color_level_label(&c.cfg.colors).into())),
                Row::Motion => ListRow::new(target, kv_row("  motion", Level::parse(&c.cfg.motion).word().into())),
                Row::Splash => ListRow::new(target, kv_row("  splash", Setting::parse(&c.cfg.splash).word().into())),
                Row::Icons => ListRow::new(target, kv_row("  icons", c.icons_label())),
                Row::Links => ListRow::new(target, kv_row("  links", switch_label(&c.cfg.hyperlinks).into())),
                Row::Images => ListRow::new(target, kv_row("  pictures", switch_label(&c.cfg.images).into())),
                Row::Pointer => ListRow::new(target, kv_row("  pointer", switch_label(&c.cfg.pointer).into())),
                Row::Slot(name) => {
                    let mut text = Text::new()
                        .push(Seg::new("  ", Ink::Plain))
                        .push(Seg::styled("██ ", name.clone()))
                        .push(Seg::new(format!("{:<16}", slot_info(&name).label), Ink::Primary));
                    if let Some(input) = c.edit.as_ref().filter(|_| i == c.row) {
                        // The input takes the value's place on this row.
                        editing = Some(Editing { row: i, at: 23, text: input.value(), caret: input.cursor() });
                        return ListRow::new(target, text);
                    }
                    text.add(Seg::new(theme.hex(&name), Ink::Secondary));
                    let row = ListRow::new(target, text);
                    if c.cfg.slots.contains_key(&name) { row.right(Text::of("yours", Ink::Warn)) } else { row }
                }
            }
        })
        .collect();
    let selected = Some(c.row.min(rows.len().saturating_sub(1)));
    section(
        scope,
        Node::List { rows, selected, editing },
        c.detail_title(),
        colours_detail(c, &theme),
    )
}

fn colours_detail(c: &Colours, theme: &Theme) -> Vec<Text> {
    let mut lines: Vec<Text> = Vec::new();
    match c.selected() {
        Row::Palette => {
            lines.push(plain("The set of colours everything starts from."));
            lines.push(Text::new());
            lines.push(kv("in use", 10, Seg::new(c.cfg.palette.clone(), Ink::Emphasis)));
            lines.push(kv("choices", 10, Seg::new(format!("{} palettes", palettes().len()), Ink::Primary)));
            lines.push(Text::new());
            lines.push(muted("Colours you set below stay set when you change palette."));
        }
        Row::Depth => {
            lines.push(plain("How many colours radar uses."));
            lines.push(Text::new());
            for l in COLOR_LEVELS {
                lines.push(choice(format!("  {}", color_level_label(l)), l == c.cfg.colors));
            }
            lines.push(Text::new());
            lines.push(muted(format!("Drawing with: {}", tier_label(theme))));
            lines.push(muted("NO_COLOR in the environment always wins."));
        }
        Row::Motion => {
            let level = Level::parse(&c.cfg.motion);
            lines.push(plain(
                "Short effects that show what changed: a screen fading in, a menu \
                 sweeping open, rows a sync changed glowing for a moment.",
            ));
            lines.push(Text::new());
            for l in Level::ALL {
                let word = Seg::new(format!("  {:<8}", l.word()), if l == level { Ink::Emphasis } else { Ink::Primary });
                let word = if l == level { word.strong() } else { word };
                lines.push(Text::new().push(word).push(Seg::new(l.about(), Ink::Muted)));
            }
            lines.push(Text::new());
            let here = crate::motion::held_back(theme.tier(), crate::system::Env::detect().ssh);
            lines.push(match (level, here) {
                (Level::Off, _) => muted("Nothing moves."),
                (_, Some(why)) => Text::of(format!("Off here all the same: {why}."), Ink::Warn),
                _ => muted("A key always acts at once; the effect gives way."),
            });
            lines.push(Text::new());
            lines.push(muted("config: [theme] motion"));
        }
        Row::Splash => {
            let setting = Setting::parse(&c.cfg.splash);
            lines.push(plain(
                "The radar mark while radar starts, with the parts in this build lighting as each is ready; \
                 then the first screen fades in. A key or click goes straight to the screen and acts.",
            ));
            lines.push(Text::new());
            for s in Setting::ALL {
                let word = Seg::new(format!("  {:<8}", s.word()), if s == setting { Ink::Emphasis } else { Ink::Primary });
                let word = if s == setting { word.strong() } else { word };
                lines.push(Text::new().push(word).push(Seg::new(s.about(), Ink::Muted)));
            }
            lines.push(Text::new());
            lines.push(muted("How it moves follows motion: with motion off it shows only when radar is slow to start."));
            lines.push(Text::new());
            lines.push(muted("config: [theme] splash"));
        }
        Row::Icons => {
            lines.push(plain(
                "Icons beside names and states, from a pack. Packs extend each other; what one leaves out comes from the one it extends, and in the end from unicode.",
            ));
            lines.push(Text::new());
            let user = icon::user_packs();
            let mut names = vec!["auto".to_string()];
            names.extend(icon::pack_names());
            for name in names {
                let shown = if name == "auto" { icon::auto_choice() } else { name.clone() };
                lines.push(
                    Text::new()
                        .push({
                            let s = Seg::new(format!("  {}", crate::ui::pad(&name, 10)), if name == c.cfg.icons { Ink::Emphasis } else { Ink::Primary });
                            if name == c.cfg.icons { s.strong() } else { s }
                        })
                        .push(Seg::new(icon::preview(&shown, &user), Ink::Primary)),
                );
            }
            lines.push(Text::new());
            lines.push(muted(format!("auto here: {}. Nerd Font symbols where the terminal brings them (Ghostty, kitty, WezTerm), else unicode; unicode over SSH.", icon::auto_choice())));
            lines.push(muted("Your packs: <name>.toml in the icons folder beside config.toml (radar icons path), read when radar starts. One icon at a time: General › icons."));
            lines.push(Text::new());
            lines.push(muted("config: [theme] icons"));
        }
        Row::Slot(name) => {
            let info = slot_info(&name);
            if let Some(t) = c.edit.as_ref().map(crate::input::TextInput::value) {
                lines.push(match parse_hex(&t) {
                    Some(_) => Text::of("⏎ keeps it", Ink::Good),
                    None => Text::of("Type # and six hex digits, e.g. #e135ff", Ink::Warn),
                });
                lines.push(Text::new());
            }
            lines.push(plain(format!("Used for {}.", info.meaning)));
            lines.push(Text::new());
            lines.push(Text::new().push(Seg::styled("██████ ", name.clone())).push(Seg::new(theme.hex(&name), Ink::Primary)));
            if c.edit.is_none() {
                lines.push(Text::new());
            }
            if let Some(v) = c.cfg.slots.get(&name).filter(|_| c.edit.is_none()) {
                lines.push(kv("yours", 10, Seg::new(v.clone(), Ink::Primary)));
                lines.push(muted("⏎ to change it or go back to the palette's."));
            } else if c.edit.is_none() {
                lines.push(muted("From the palette. ⏎ to set your own."));
            }
            lines.push(Text::new());
            lines.push(muted(format!("config: [theme.slots] \"{name}\"")));
        }
        Row::Links => switch_detail(
            &mut lines,
            "Links in text open on a click (Ctrl, or Cmd on a Mac): the terminal opens them. o and the Links tabs open them too, whatever this says.",
            &c.cfg.hyperlinks,
            if crate::ui::link::on() { "Drawing links: yes." } else { "Drawing links: no." },
            "Where the terminal can: Ghostty, kitty, WezTerm, iTerm2, GNOME's and Windows Terminal, and terminals that pass them on. Not inside tmux, nor over ssh unless the terminal says what it is.",
            "config: [theme] hyperlinks",
        ),
        Row::Images => switch_detail(
            &mut lines,
            "Pictures in messages and descriptions, drawn in the terminal. Where it cannot, a line names each and its link.",
            &c.cfg.images,
            if crate::ui::image::on() { "Drawing pictures: yes." } else { "Drawing pictures: no." },
            "radar asks the terminal when it starts (kitty's, sixel or iTerm2's graphics): turning them on takes effect then. Not inside tmux unless on.",
            "config: [theme] images",
        ),
        Row::Pointer => switch_detail(
            &mut lines,
            "The mouse pointer turns into a hand over what a click acts on, and into a text cursor while you drag to select. What it is over is lit either way.",
            &c.cfg.pointer,
            if crate::ui::pointer::on() { "Changing the pointer: yes." } else { "Changing the pointer: no." },
            "Where the terminal can: Ghostty, kitty, foot and WezTerm. Not inside tmux unless on.",
            "config: [theme] pointer",
        ),
        Row::Header(_) => {}
    }
    lines
}

fn switch_detail(lines: &mut Vec<Text>, about: &str, now: &str, drawing: &str, where_: &str, config: &str) {
    lines.push(plain(about));
    lines.push(Text::new());
    for v in SWITCHES {
        lines.push(choice(format!("  {}", switch_label(v)), v == now));
    }
    lines.push(Text::new());
    lines.push(muted(drawing));
    lines.push(muted(where_));
    lines.push(Text::new());
    lines.push(muted(config));
}

pub(super) fn icons(host: &Host, scope: &str) -> SectionView {
    let c = &host.colours;
    let now = c.icons_now();
    let shown = now.width.max(1);
    let mut editing = None;
    let rows: Vec<ListRow> = icons::rows()
        .into_iter()
        .enumerate()
        .map(|(n, r)| match r {
            IconRow::Heading(g) => ListRow::heading(format!("icon:{n}"), muted(g)),
            IconRow::Icon(i) => {
                let r = now.get(i);
                let glyph = if r.glyph.is_empty() { crate::ui::pad("·", shown) } else { r.glyph.clone() };
                if n == c.icon_row
                    && let Some(input) = &c.edit
                {
                    editing = Some(Editing { row: n, at: 3, text: input.value(), caret: input.cursor() });
                }
                let text = Text::new()
                    .push(Seg::new("  ", Ink::Plain))
                    .push(Seg::styled(glyph, r.style.clone()))
                    .push(Seg::new("  ", Ink::Plain))
                    .push(Seg::new(crate::ui::pad(i.name(), 18), Ink::Primary));
                let row = ListRow::new(format!("icon:{n}"), text).detail(Text::of(i.about(), Ink::Secondary));
                if c.cfg.icon_overrides.contains_key(i.name()) { row.right(Text::of("yours", Ink::Warn)) } else { row }
            }
        })
        .collect();
    let selected = Some(c.icon_row.min(rows.len().saturating_sub(1)));
    section(scope, Node::List { rows, selected, editing }, c.icons_detail_title(), icons_detail(c))
}

fn icons_detail(c: &Colours) -> Vec<Text> {
    let icon = c.icon_selected();
    let now = c.icons_now();
    let r = now.get(icon);
    let mut lines = vec![plain(format!("Shows {}.", icon.about())), Text::new()];
    let glyph = if r.glyph.is_empty() { "none".to_string() } else { r.glyph.trim_end().to_string() };
    lines.push(kv("now", 10, Seg::styled(glyph, r.style.clone())));
    lines.push(kv("from", 10, Seg::new(r.from.clone(), Ink::Primary)));
    lines.push(kv("colour", 10, Seg::new(r.style.clone(), Ink::Primary)));
    lines.push(kv("pack", 10, Seg::new(c.icons_label(), Ink::Primary)));
    lines.push(Text::new());
    if !now.on() {
        lines.push(Text::of("Icons are off: pick a pack in General › colours, icons.", Ink::Warn));
    }
    lines.push(muted(if c.cfg.icon_overrides.contains_key(icon.name()) {
        "Yours. ⏎ types another, r gives it back to the pack."
    } else {
        "The pack's. ⏎ types your own."
    }));
    lines.push(muted(format!(
        "A glyph takes at most {} cell{} in this pack; joined emoji, skin tones and flags are refused.",
        now.width.max(1),
        if now.width > 1 { "s" } else { "" }
    )));
    lines.push(Text::new());
    lines.push(muted(format!("config: [icons] {}", icon.name())));
    lines
}
