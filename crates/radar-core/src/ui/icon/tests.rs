use super::*;
use crate::design::theme::Tier;
use crate::theme::ThemeConfig;

#[test]
fn auto_is_off_until_the_runner_says_and_picks_by_the_terminal() {
    std::thread::spawn(|| {
        assert_eq!(auto_choice(), "off");
        assert!(!Theme::new(Tier::Mono).icons().on());
        set_auto("nerd");
        assert_eq!(Theme::new(Tier::Mono).icons().pack, "nerd");
    })
    .join()
    .unwrap();
    let env = |pairs: &'static [(&'static str, &'static str)]| {
        move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    };
    assert_eq!(detect(&env(&[("TERM_PROGRAM", "ghostty")])), "nerd");
    // A multiplexer that sets TERM_PROGRAM=ghostty for its panes.
    assert_eq!(detect(&env(&[("TERM_PROGRAM", "ghostty"), ("TERM", "xterm-256color")])), "nerd");
    assert_eq!(detect(&env(&[("TERM", "xterm-kitty")])), "nerd");
    assert_eq!(detect(&env(&[("TERM_PROGRAM", "WezTerm")])), "nerd");
    // tmux hides TERM_PROGRAM, but Ghostty's variables come through.
    assert_eq!(detect(&env(&[("TERM_PROGRAM", "tmux"), ("GHOSTTY_RESOURCES_DIR", "/usr/share/ghostty")])), "nerd");
    assert_eq!(detect(&env(&[("TERM_PROGRAM", "tmux"), ("TERM", "tmux-256color")])), "unicode");
    assert_eq!(detect(&env(&[("TERM", "xterm-256color")])), "unicode");
    assert_eq!(detect(&env(&[("TERM_PROGRAM", "ghostty"), ("SSH_CONNECTION", "1 2 3 4")])), "unicode");
}

#[test]
fn drawing_puts_the_icon_and_a_gap_or_nothing() {
    let off = Theme::new(Tier::Mono);
    assert_eq!(lead(&off, Icon::Repo).content, "");
    assert_eq!(text(&off, Icon::Repo), "");
    assert_eq!(with(&off, Icon::Repo, "api"), "api");
    assert_eq!(lead_width(&off), 0);
    let cfg = ThemeConfig { icons: "unicode".into(), ..Default::default() };
    let t = Theme::build(&cfg, Tier::Mono, &[]);
    assert_eq!(lead(&t, Icon::Repo).content, "▤ ");
    assert_eq!(with(&t, Icon::Bug, "crash"), "✱ crash");
    assert_eq!(lead_width(&t), 2);
    let cfg = ThemeConfig { icons: "emoji".into(), ..Default::default() };
    let t = Theme::build(&cfg, Tier::TrueColor, &[]);
    assert_eq!(lead_width(&t), 3);
    assert_eq!(span(&t, Icon::RunFailed).style, t.spec("status.error"));
}

#[test]
fn a_chips_icon_keeps_its_own_colour_on_the_chips_shade() {
    use crate::compose::Chip;
    use crate::design::theme::Slot;
    use crate::plugin::Tone;
    use crate::ui::chips::{chip, chip_spans};
    let joined = |s: &[Span<'_>]| s.iter().map(|s| s.content.as_ref()).collect::<String>();
    let c = Chip::new("failed · 2", Tone::Bad).icon(Icon::RunFailed);
    // Off: as a chip always was.
    let off = Theme::new(Tier::TrueColor);
    assert_eq!(chip_spans(&off, &c), vec![chip(&off, &Chip::new("failed · 2", Tone::Bad))]);
    let cfg = ThemeConfig { icons: "unicode".into(), ..Default::default() };
    let t = Theme::build(&cfg, Tier::TrueColor, &[]);
    let spans = chip_spans(&t, &c);
    assert_eq!(joined(&spans), " ✗ failed · 2 ");
    assert_eq!(joined(&spans), chip(&t, &c).content, "the same cells as the one-span chip");
    let shade = crate::widgets::chip_ground(&t, t.s(Slot::StatusError).fg).bg;
    assert!(spans.iter().all(|s| s.style.bg == shade), "all on the chip's shade: {spans:?}");
    assert_eq!(spans[1].style.fg, t.spec("status.error").fg, "the icon in its own colour");
    let linked = chip_spans(&t, &c.clone().link("https://example.com/x"));
    assert!(linked.iter().all(|s| s.style == crate::ui::link::style(s.style, "https://example.com/x")));
}

#[test]
fn a_contributed_cells_icon_keeps_its_own_colour_before_the_value() {
    use crate::compose::Cell;
    use crate::plugin::Tone;
    let c = Cell::new("note", "N-1 · open").tone(Tone::Good).icon(Icon::Story);
    let off = Theme::new(Tier::TrueColor);
    assert_eq!(crate::view_tui::cell_spans(&c, &off), vec![crate::view_tui::cell_span(&c, &off)], "off: the value alone");
    let cfg = ThemeConfig { icons: "unicode".into(), ..Default::default() };
    let t = Theme::build(&cfg, Tier::TrueColor, &[]);
    let spans = crate::view_tui::cell_spans(&c, &t);
    assert_eq!(spans.len(), 2);
    assert_eq!(spans[0], span(&t, Icon::Story).clone().content(format!("{} ", span(&t, Icon::Story).content)));
    assert_eq!(spans[1], crate::view_tui::cell_span(&c, &t), "the value in its tone");
}

#[test]
fn plain_words_follow_the_icons_the_host_drew_with_last() {
    std::thread::spawn(|| {
        assert_eq!(plain(Icon::Bug), "", "before any theme: none");
        let mut host = crate::Host::new(vec![], 0).unwrap();
        host.load_config(Some("[theme]\nicons = \"unicode\"\n")).unwrap();
        let _ = host.build_theme(Tier::Mono);
        assert_eq!(plain_with(Icon::Bug, "crash"), "✱ crash");
        assert_eq!(plain_or(Icon::Bug, "x"), (Some(Icon::Bug), String::new()), "on: the icon, named");
        host.colours.cfg.icons = "off".into();
        let _ = host.build_theme(Tier::Mono);
        assert_eq!(plain_with(Icon::Bug, "crash"), "crash");
        assert_eq!(plain_or(Icon::Bug, "x"), (None, "x ".to_string()), "off: the symbol, as it was");
    })
    .join()
    .unwrap();
}
