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
    assert_eq!(text(&off, Icon::Repo), "");
    assert_eq!(with(&off, Icon::Repo, "api"), "api");
    assert_eq!(lead_width(&off), 0);
    let cfg = ThemeConfig { icons: "unicode".into(), ..Default::default() };
    let t = Theme::build(&cfg, Tier::Mono, &[]);
    assert_eq!(text(&t, Icon::Repo), "▤ ");
    assert_eq!(with(&t, Icon::Bug, "crash"), "✱ crash");
    assert_eq!(lead_width(&t), 2);
    let cfg = ThemeConfig { icons: "emoji".into(), ..Default::default() };
    let t = Theme::build(&cfg, Tier::TrueColor, &[]);
    assert_eq!(lead_width(&t), 3);
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
