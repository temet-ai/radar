use std::collections::{BTreeMap, HashSet};

use super::*;
use crate::design::theme::Tier;
use crate::theme::ThemeConfig;

fn pack(text: &str) -> Pack {
    let (p, w) = Pack::parse(text, "mine").expect("parses");
    assert!(w.is_empty(), "{w:?}");
    p
}

#[test]
fn every_icon_has_a_unique_name_and_is_found_by_it() {
    let mut seen = HashSet::new();
    for &i in Icon::ALL {
        assert!(seen.insert(i.name()), "{} twice", i.name());
        assert_eq!(Icon::from_name(i.name()), Some(i));
        assert!(!i.about().is_empty());
    }
    assert_eq!(Icon::from_name("nope"), None);
}

#[test]
fn built_in_packs_fill_every_icon_at_their_own_width() {
    for name in ["nerd", "unicode", "emoji"] {
        let p = Pack::built_in(name).unwrap();
        let w = p.width.unwrap();
        for &i in Icon::ALL {
            let g = p.icons[i.name()].text();
            assert_eq!(check(g, w), Ok(()), "{name} {}: {g:?}", i.name());
            assert_eq!(super::super::width(g), w, "{name} {}: {g:?} is not {w} wide", i.name());
        }
    }
    // The unicode tier is never emoji.
    for g in Pack::built_in("unicode").unwrap().icons.values() {
        assert!(g.text().chars().all(|c| (c as u32) < 0x1f000), "{g:?}");
    }
    let (off, w) = resolve("off", &[], &BTreeMap::new());
    assert!(w.is_empty() && !off.on());
    assert!(Icon::ALL.iter().all(|&i| off.get(i).glyph.is_empty()));
}

/// Each built-in pack as the layout takes it: every icon its own glyph, at
/// the width the resolved icons say (what a row, a tab or a chip leaves for
/// it), or nothing at all when icons are off. The mix screens record one
/// screen per pack; this is where a pack is checked.
#[test]
fn each_pack_resolves_every_icon_at_the_width_the_layout_takes() {
    for name in BUILT_IN {
        let (icons, w) = resolve(name, &[], &BTreeMap::new());
        assert!(w.is_empty(), "{name}: {w:?}");
        assert_eq!(icons.pack, name);
        let want = Pack::built_in(name).unwrap().width.unwrap();
        assert_eq!(icons.width, want, "{name}");
        assert_eq!(icons.on(), want > 0, "{name}");
        for &i in Icon::ALL {
            let r = icons.get(i);
            if name == "off" {
                assert_eq!(r.glyph, "", "{name} {}", i.name());
                continue;
            }
            assert_eq!(r.from, name, "{name} {}: from its own table", i.name());
            assert_eq!(super::super::width(&r.glyph), want, "{name} {}: {:?}", i.name(), r.glyph);
        }
    }
}

#[test]
fn a_pack_extends_another_and_what_it_leaves_out_comes_from_there() {
    let mine = pack("name = \"mine\"\nextends = \"nerd\"\n[icons]\nbug = \"B\"\nrepo = { glyph = \"R\", style = \"accent.primary bold\" }\n");
    let (icons, w) = resolve("mine", std::slice::from_ref(&mine), &BTreeMap::new());
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(icons.pack, "mine");
    assert_eq!(icons.width, 1, "the width comes from the pack it extends");
    assert_eq!(icons.get(Icon::Bug).glyph, "B");
    assert_eq!(icons.get(Icon::Bug).style, "status.error", "no style: the icon's own");
    assert_eq!(icons.get(Icon::Repo).style, "accent.primary bold");
    assert_eq!(icons.get(Icon::Branch).glyph, "\u{f418}");
    assert_eq!(icons.get(Icon::Branch).from, "nerd");
}

#[test]
fn a_partial_pack_falls_back_to_unicode_and_a_pack_two_wide_pads() {
    let mine = pack("width = 2\n[icons]\nbug = \"🐞\"\n");
    let (icons, w) = resolve("mine", std::slice::from_ref(&mine), &BTreeMap::new());
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(icons.get(Icon::Bug).glyph, "🐞");
    assert_eq!(icons.get(Icon::RunPassed).glyph, "✓ ", "unicode, padded to the pack's two cells");
    assert_eq!(icons.get(Icon::RunPassed).from, "unicode");
}

#[test]
fn overrides_win_over_the_pack() {
    let mut over = BTreeMap::new();
    over.insert("bug".to_string(), Glyph::new("b"));
    over.insert("repo".to_string(), Glyph::Styled { glyph: "r".into(), style: Some("accent.primary".into()) });
    let (icons, w) = resolve("nerd", &[], &over);
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(icons.get(Icon::Bug).glyph, "b");
    assert_eq!(icons.get(Icon::Bug).from, "[icons]");
    assert_eq!(icons.get(Icon::Repo).style, "accent.primary");
    assert_eq!(icons.get(Icon::Branch).glyph, "\u{f418}");
    // `off` stays off whatever is overridden.
    assert!(!resolve("off", &[], &over).0.on());
}

#[test]
fn a_glyph_too_wide_or_joined_is_refused_with_a_reason() {
    assert!(check("🐞", 1).unwrap_err().contains("2 cells wide"));
    assert!(check("👩‍💻", 2).unwrap_err().contains("joined"));
    assert!(check("👍🏽", 2).is_err());
    assert!(check("\u{7}", 1).is_err());
    assert_eq!(check("", 1), Ok(()));
    let mut over = BTreeMap::new();
    over.insert("bug".to_string(), Glyph::new("🐞"));
    let (icons, w) = resolve("unicode", &[], &over);
    assert_eq!(icons.get(Icon::Bug).glyph, "✱", "refused, so the pack's own");
    assert!(w.iter().any(|w| w.contains("bug") && w.contains("refused")), "{w:?}");
}

#[test]
fn unknown_names_are_warned_about_not_refused() {
    let (p, w) = Pack::parse("[icons]\nbugg = \"x\"\nbug = \"y\"\n", "mine").unwrap();
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].contains("bugg"));
    assert_eq!(p.icons.len(), 2);
    let mut over = BTreeMap::new();
    over.insert("nope".to_string(), Glyph::new("x"));
    let (_, w) = resolve("unicode", &[], &over);
    assert!(w.iter().any(|w| w.contains("nope")), "{w:?}");
    assert!(Pack::parse("width = 3", "x").is_err());
    let (_, w) = resolve("missing", &[], &BTreeMap::new());
    assert!(w[0].contains("missing"), "{w:?}");
}

#[test]
fn packs_load_from_a_folder_and_a_built_in_name_is_passed_over() {
    let dir = std::env::temp_dir().join(format!("radar-icons-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("calm.toml"), "description = \"calm\"\nextends = \"unicode\"\n[icons]\nbug = \"b\"\n").unwrap();
    std::fs::write(dir.join("nerd.toml"), "[icons]\nbug = \"b\"\n").unwrap();
    std::fs::write(dir.join("broken.toml"), "this is not toml =").unwrap();
    std::fs::write(dir.join("notes.txt"), "not a pack").unwrap();
    let (packs, warnings) = load_dir(&dir);
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(packs.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["calm"]);
    assert_eq!(packs[0].description, "calm");
    assert!(packs[0].path.is_some());
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(warnings.iter().any(|w| w.contains("built-in")));
    assert!(warnings.iter().any(|w| w.contains("broken.toml")));
    assert!(load_dir(Path::new("/no/such/folder")).0.is_empty());
}

#[test]
fn an_exported_pack_reads_back_as_the_same_icons() {
    for name in ["nerd", "unicode", "emoji"] {
        let text = export(name, &[]).unwrap();
        let (p, w) = Pack::parse(&text, "x").unwrap();
        assert!(w.is_empty(), "{name}: {w:?}");
        assert_eq!(p.extends.as_deref(), Some(name));
        let (a, _) = resolve(name, &[], &BTreeMap::new());
        let (b, w) = resolve(&p.name, std::slice::from_ref(&p), &BTreeMap::new());
        assert!(w.is_empty(), "{name}: {w:?}");
        for &i in Icon::ALL {
            assert_eq!(a.get(i).glyph, b.get(i).glyph, "{name} {}", i.name());
        }
    }
    assert!(export("missing", &[]).is_none());
}

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

#[test]
fn a_file_is_known_by_its_extension_or_its_name() {
    assert_eq!(Icon::for_file("src/main.rs"), Icon::FileRust);
    assert_eq!(Icon::for_file("web/App.TSX"), Icon::FileTypescript);
    assert_eq!(Icon::for_file("Dockerfile"), Icon::FileDocker);
    assert_eq!(Icon::for_file("ci/Dockerfile.build"), Icon::FileDocker);
    assert_eq!(Icon::for_file("Cargo.lock"), Icon::FileLock);
    assert_eq!(Icon::for_file(".gitignore"), Icon::FileGit);
    assert_eq!(Icon::for_file("config.yaml"), Icon::FileYaml);
    assert_eq!(Icon::for_file("README"), Icon::File);
}

#[test]
fn the_example_pack_in_the_docs_reads_without_a_warning() {
    let text = include_str!("../../../../../docs/icons/example.toml");
    let (p, w) = Pack::parse(text, "example").unwrap();
    assert!(w.is_empty(), "{w:?}");
    let (icons, w) = resolve(&p.name, std::slice::from_ref(&p), &BTreeMap::new());
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(icons.width, 2);
    assert_eq!(icons.get(Icon::Bug).glyph, "🐛");
    assert_eq!(icons.get(Icon::Repo).glyph, "\u{f401} ", "nerd's, padded to two cells");
}

#[test]
fn the_icons_table_round_trips_through_the_config() {
    let v: toml::Value = toml::from_str("bug = \"🐞\"\nrepo = { glyph = \"r\", style = \"accent.primary\" }\nbad = 3\n").unwrap();
    let over = overrides(Some(&v));
    assert_eq!(over.len(), 2);
    assert_eq!(over["repo"].style(), Some("accent.primary"));
    let back = toml::Value::try_from(&over).unwrap();
    assert_eq!(overrides(Some(&back)), over);
}
