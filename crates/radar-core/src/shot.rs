//! `radar shot`: pictures of radar's places, from sample data, never a live service.
//!
//! ```text
//! radar shot --list                                   the places, with titles
//! radar shot --place repositories --size 160x40 --out shots/
//! radar shot --out shots/                             every place, 80x24 and 160x40
//! ```
//!
//! A place is a screen (`repositories`), a settings section
//! (`settings.<part>.connection`) or a page of the kit gallery
//! (`gallery.tree`). Each is drawn into an SVG, and into a PNG when radar is
//! built with the `png` feature (on by default in the binary). The parts
//! bring their sample data through [`Plugin::demo`](crate::Plugin::demo)
//! (their `demo` feature); the host here never reads the user's config or
//! caches. `scripts/shots` renders everything into a folder with a page that
//! shows it, and compares two git refs.

use std::path::Path;

use anyhow::{Context, Result, bail};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::design::theme::Tier;
use crate::host::{Host, View};
use crate::plugin::{DEMO_NOW, Plugin};
use crate::theme::{Theme, ThemeConfig};
use crate::ui::gallery::{self, Gallery};
use crate::visual::{self, Look};

/// What a place is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum What {
    Screen(usize),
    Section(usize),
    Gallery(usize),
}

/// A named thing to draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    /// What `--place` takes and the files are named after.
    pub name: String,
    /// For people: the index page's heading.
    pub title: String,
    what: What,
}

/// `Repositories & more` -> `repositories-more`.
fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// Every place of this host: its screens, its settings sections, then the
/// gallery's pages.
pub fn places(host: &Host) -> Vec<Place> {
    let mut all = Vec::new();
    for (i, (p, spec)) in host.screens.iter().enumerate() {
        all.push(Place {
            name: slug(spec.name),
            title: format!("{} (screen of {})", spec.name, host.plugins[*p].name()),
            what: What::Screen(i),
        });
    }
    for (i, (_, spec)) in host.sections.iter().enumerate() {
        all.push(Place {
            name: format!("settings.{}.{}", slug(spec.group), slug(spec.id)),
            title: format!("settings: {} / {}", host.group_name(spec.group), host.section_title(i)),
            what: What::Section(i),
        });
    }
    for (i, tab) in gallery::tab_names().iter().enumerate() {
        all.push(Place {
            name: format!("gallery.{}", slug(tab)),
            title: format!("kit gallery: {tab}"),
            what: What::Gallery(i),
        });
    }
    all
}

/// A host over these parts as `radar shot` draws them: no config, their sample
/// data, the clock at [`DEMO_NOW`].
pub fn demo_host(plugins: Vec<Box<dyn Plugin>>) -> Result<Host> {
    let mut host = Host::new(plugins, DEMO_NOW)?;
    host.load_config(Some(""))?;
    // Loading a config reconfigures the parts: their samples go in after.
    for p in &mut host.plugins {
        p.demo();
    }
    Ok(host)
}

/// The place drawn at `width` x `height` cells, as an SVG.
pub fn svg(host: &mut Host, place: &Place, width: u16, height: u16) -> String {
    match place.what {
        What::Gallery(tab) => {
            // As the gallery example draws it: the kit's own look, with no
            // part's slots in it.
            let cfg = ThemeConfig {
                icons: host.colours.cfg.icons.clone(),
                ..ThemeConfig::default()
            };
            let theme = Theme::build(&cfg, Tier::TrueColor, &[]);
            let area = Rect::new(0, 0, width, height);
            let mut buf = Buffer::empty(area);
            let mut g = Gallery::new();
            g.tab = tab;
            // Twice: the tree opens and selects on its first frame.
            for _ in 0..2 {
                g.render(&theme, area, &mut buf, "colours: full colour");
            }
            visual::buffer_to_svg(&buf, &Look::of(&theme))
        }
        What::Screen(i) => {
            host.view = View::Screen(i);
            drawn(host, width, height)
        }
        What::Section(i) => {
            host.view = View::Settings;
            host.section = i;
            host.on_heading = false;
            host.focus = crate::plugin::Pane::Middle;
            drawn(host, width, height)
        }
    }
}

/// Draw once to let panes learn their sizes, then again for the picture.
fn drawn(host: &Host, width: u16, height: u16) -> String {
    let _ = crate::shell::to_svg(host, width, height);
    crate::shell::to_svg(host, width, height)
}

/// `160x40`.
pub fn parse_size(text: &str) -> Result<(u16, u16)> {
    let (w, h) = text
        .split_once(['x', 'X'])
        .with_context(|| format!("a size is WIDTHxHEIGHT, like 160x40: {text:?}"))?;
    let num = |s: &str| {
        s.trim()
            .parse::<u16>()
            .ok()
            .filter(|n| (1..=1000).contains(n))
            .with_context(|| format!("a size is WIDTHxHEIGHT, like 160x40: {text:?}"))
    };
    Ok((num(w)?, num(h)?))
}

/// An SVG as a PNG. Fonts come from the system: a Nerd Font when one is
/// there (icons draw), else JetBrains Mono, else a monospace face.
#[cfg(feature = "png")]
pub fn to_png(svg: &str) -> Result<Vec<u8>> {
    use resvg::{tiny_skia, usvg};
    let mut opt = usvg::Options::default();
    {
        let db = opt.fontdb_mut();
        db.load_system_fonts();
        let wanted = [
            "0xProto Nerd Font Mono",
            "Symbols Nerd Font Mono",
            "JetBrainsMono Nerd Font",
            "JetBrains Mono",
            "DejaVu Sans Mono",
            "Liberation Mono",
        ];
        let have = |name: &str| db.faces().any(|f| f.families.iter().any(|(n, _)| n == name));
        let family = wanted.iter().find(|n| have(n)).map(|n| n.to_string()).or_else(|| {
            db.faces()
                .find(|f| f.monospaced)
                .and_then(|f| f.families.first().map(|(n, _)| n.clone()))
        });
        if let Some(family) = family {
            db.set_monospace_family(family.clone());
            db.set_sans_serif_family(family);
        }
    }
    let tree = usvg::Tree::from_str(svg, &opt).context("reading the svg")?;
    let size = tree.size().to_int_size();
    let mut pixmap = tiny_skia::Pixmap::new(size.width(), size.height()).context("an empty picture")?;
    resvg::render(&tree, tiny_skia::Transform::identity(), &mut pixmap.as_mut());
    pixmap.encode_png().context("encoding the png")
}

#[cfg(not(feature = "png"))]
pub fn to_png(_svg: &str) -> Result<Vec<u8>> {
    bail!("this radar was built without PNG output (the `png` feature); the SVG is written")
}

/// `radar shot`'s definition for the command line.
pub fn command() -> clap::Command {
    use clap::{Arg, ArgAction};
    clap::Command::new("shot")
        .about("Draw places (screens, settings sections, the kit gallery) from sample data, as SVG and PNG")
        .arg(Arg::new("list").long("list").action(ArgAction::SetTrue).help("Print the places, one per line: name, a tab, title"))
        .arg(
            Arg::new("place")
                .long("place")
                .action(ArgAction::Append)
                .value_name("NAME")
                .help("A place to draw (repeat it); default: every place. See --list"),
        )
        .arg(
            Arg::new("size")
                .long("size")
                .action(ArgAction::Append)
                .value_name("WxH")
                .help("Cells wide by high (repeat it); default: 80x24 and 160x40"),
        )
        .arg(Arg::new("out").long("out").value_name("DIR").default_value("shots").help("The folder to write into"))
        .arg(
            Arg::new("icons")
                .long("icons")
                .value_name("PACK")
                .default_value("nerd")
                .help("The icon pack: nerd, unicode, emoji, off, or one of yours"),
        )
        .arg(
            Arg::new("format")
                .long("format")
                .value_parser(["svg", "png", "both"])
                .default_value(if cfg!(feature = "png") { "both" } else { "svg" })
                .help("What to write"),
        )
}

/// Run `radar shot` over these parts.
pub fn run(plugins: Vec<Box<dyn Plugin>>, m: &clap::ArgMatches) -> Result<()> {
    let mut host = demo_host(plugins)?;
    // Any pack: the built-in ones and the user's.
    let (packs, warnings) = crate::ui::icon::load_dir(&crate::ui::icon::dir());
    for w in warnings {
        eprintln!("icons: {w}");
    }
    crate::ui::icon::set_packs(packs);
    let pack = m.get_one::<String>("icons").map_or("nerd", String::as_str);
    if crate::ui::icon::find(pack, &crate::ui::icon::user_packs()).is_none() {
        bail!("no icon pack is called {pack:?}; the packs are: {}", crate::ui::icon::pack_names().join(", "));
    }
    host.colours.cfg.icons = pack.to_string();
    let all = places(&host);
    if m.get_flag("list") {
        for p in &all {
            println!("{}\t{}", p.name, p.title);
        }
        return Ok(());
    }
    let wanted: Vec<&String> = m.get_many::<String>("place").map(|v| v.collect()).unwrap_or_default();
    let mut chosen = Vec::new();
    if wanted.is_empty() {
        chosen = all.clone();
    }
    for w in wanted {
        match all.iter().find(|p| p.name == *w).or_else(|| all.iter().find(|p| p.name == slug(w))) {
            Some(p) => chosen.push(p.clone()),
            None => bail!(
                "no place called {w:?}; the places are: {}",
                all.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ")
            ),
        }
    }
    let sizes = match m.get_many::<String>("size") {
        Some(v) => v.map(|s| parse_size(s)).collect::<Result<Vec<_>>>()?,
        None => vec![(80, 24), (160, 40)],
    };
    let format = m.get_one::<String>("format").map_or("both", String::as_str);
    let out = Path::new(m.get_one::<String>("out").map_or("shots", String::as_str));
    std::fs::create_dir_all(out).with_context(|| format!("making {}", out.display()))?;
    for place in &chosen {
        for &(w, h) in &sizes {
            let svg = svg(&mut host, place, w, h);
            // Names have dots in them: add the extension, never replace one.
            let base = format!("{}_{w}x{h}", place.name);
            if format != "png" {
                let path = out.join(format!("{base}.svg"));
                std::fs::write(&path, &svg).with_context(|| format!("writing {}", path.display()))?;
                println!("{}", path.display());
            }
            if format != "svg" {
                let path = out.join(format!("{base}.png"));
                std::fs::write(&path, to_png(&svg)?).with_context(|| format!("writing {}", path.display()))?;
                println!("{}", path.display());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_parse_and_places_slug() {
        assert_eq!(parse_size("160x40").unwrap(), (160, 40));
        assert!(parse_size("160").is_err());
        assert!(parse_size("0x40").is_err());
        assert_eq!(slug("Rows & states"), "rows-states");
    }

    #[test]
    fn a_host_without_parts_still_has_settings_and_gallery_places() {
        let mut host = demo_host(vec![]).unwrap();
        let all = places(&host);
        let names: Vec<&str> = all.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"settings.general.parts"), "{names:?}");
        assert!(names.contains(&"gallery.tree"), "{names:?}");
        let mut seen = std::collections::HashSet::new();
        assert!(names.iter().all(|n| seen.insert(*n)), "unique names: {names:?}");
        for p in &all {
            let svg = svg(&mut host, p, 80, 24);
            assert!(svg.starts_with("<svg") && svg.ends_with("</svg>\n"), "{}", p.name);
        }
    }
}
