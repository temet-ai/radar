//! Which terminal opens in a checkout (`t`), for the `terminal` settings
//! section: what radar would start with nothing set and how it found it
//! (`ghostty · from $TERMINAL`), and the terminals installed on this
//! machine, the one radar runs in first, to pick one from. Only terminals
//! of this system are looked for (Linux programs on PATH, Mac apps in the
//! Applications folders), and only those found are offered. The choice is
//! kept in `[git] terminal`; starting it is radar-core's
//! ([`system::folder_ways`]).

use radar_core::system::{self, Env};

/// A terminal radar knows how to start in a folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Known {
    /// Its name as shown.
    pub name: &'static str,
    /// The command line kept in `[git] terminal`: `{dir}` is the folder,
    /// `{cmd}` what it runs (else `-e` and the command are added).
    pub command: &'static str,
    /// Where it is looked for: a program on PATH (Linux), or the app's
    /// name under the Applications folders (the Mac).
    pub look: Look,
}

/// Where a terminal is looked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Look {
    Path(&'static str),
    App(&'static str),
}

const fn linux(name: &'static str, command: &'static str) -> Known {
    Known { name, command, look: Look::Path(name) }
}

const fn mac(name: &'static str, app: &'static str, command: &'static str) -> Known {
    Known { name, command, look: Look::App(app) }
}

/// The Linux terminals radar can start in a folder.
pub const LINUX: [Known; 15] = [
    linux("ghostty", "ghostty --working-directory={dir}"),
    linux("kitty", "kitty --directory {dir} {cmd}"),
    linux("alacritty", "alacritty --working-directory {dir}"),
    linux("wezterm", "wezterm start --cwd {dir} {cmd}"),
    linux("foot", "foot --working-directory={dir} {cmd}"),
    linux("gnome-terminal", "gnome-terminal --working-directory={dir} -- {cmd}"),
    linux("ptyxis", "ptyxis --new-window --working-directory={dir}"),
    linux("kgx", "kgx --working-directory={dir}"),
    linux("konsole", "konsole --workdir {dir}"),
    linux("xfce4-terminal", "xfce4-terminal --working-directory={dir}"),
    linux("tilix", "tilix --working-directory={dir}"),
    linux("terminator", "terminator --working-directory={dir}"),
    linux("rio", "rio --working-dir {dir}"),
    linux("st", "st -d {dir}"),
    linux("xterm", "xterm"),
];

/// The Mac terminals radar can start in a folder, by their app.
pub const MAC: [Known; 8] = [
    mac("Terminal", "Terminal.app", "open -a Terminal {dir}"),
    mac("iTerm", "iTerm.app", "open -a iTerm {dir}"),
    mac("Ghostty", "Ghostty.app", "open -a Ghostty {dir}"),
    mac("kitty", "kitty.app", "open -na kitty --args --directory {dir}"),
    mac("Alacritty", "Alacritty.app", "open -na Alacritty --args --working-directory {dir}"),
    mac("WezTerm", "WezTerm.app", "open -na WezTerm --args start --cwd {dir}"),
    mac("Warp", "Warp.app", "open -a Warp {dir}"),
    mac("Hyper", "Hyper.app", "open -a Hyper {dir}"),
];

/// Where a Mac app may be: the system's, then everyone's, then the user's.
pub fn app_places(home: Option<&std::path::Path>, app: &str) -> Vec<std::path::PathBuf> {
    let mut out = vec![
        std::path::Path::new("/System/Applications/Utilities").join(app),
        std::path::Path::new("/Applications").join(app),
        std::path::Path::new("/Applications/Utilities").join(app),
    ];
    if let Some(home) = home {
        out.push(home.join("Applications").join(app));
    }
    out
}

/// What was found here: the terminal radar starts with nothing set, the
/// one radar runs in, and the terminals installed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Terminals {
    /// The program, and how it was found (`from $TERMINAL`, `found on
    /// PATH`); `None` when nothing can open here.
    pub chosen: Option<(String, String)>,
    /// The installed terminals of this system, the one radar runs in first.
    pub found: Vec<Known>,
    /// The terminal radar runs in now, when it is one of [`Terminals::found`].
    pub current: Option<&'static str>,
}

/// The terminal radar runs in, from what the terminal sets in the
/// environment (tmux and the like pass most of these through).
pub fn running_in(var: &dyn Fn(&str) -> Option<String>, macos: bool) -> Option<&'static str> {
    let set = |k: &str| var(k).is_some_and(|v| !v.is_empty());
    let program = var("TERM_PROGRAM").unwrap_or_default().to_lowercase();
    let term = var("TERM").unwrap_or_default();
    let (lin, mac) = if set("GHOSTTY_RESOURCES_DIR") || program == "ghostty" {
        ("ghostty", "Ghostty")
    } else if set("KITTY_WINDOW_ID") || term == "xterm-kitty" {
        ("kitty", "kitty")
    } else if set("WEZTERM_EXECUTABLE") || program == "wezterm" {
        ("wezterm", "WezTerm")
    } else if set("ALACRITTY_WINDOW_ID") || set("ALACRITTY_SOCKET") {
        ("alacritty", "Alacritty")
    } else if set("ITERM_SESSION_ID") || program == "iterm.app" {
        ("", "iTerm")
    } else if program == "apple_terminal" {
        ("", "Terminal")
    } else if program == "warpterminal" {
        ("", "Warp")
    } else if program == "hyper" {
        ("", "Hyper")
    } else if term.starts_with("foot") {
        ("foot", "")
    } else if set("KONSOLE_VERSION") {
        ("konsole", "")
    } else if set("PTYXIS_VERSION") {
        ("ptyxis", "")
    } else if set("TILIX_ID") {
        ("tilix", "")
    } else {
        ("", "")
    };
    let name = if macos { mac } else { lin };
    (!name.is_empty()).then_some(name)
}

/// What radar would start, as [`system::folder_ways`] tries it (tmux
/// first, then the system's own way, `$TERMINAL`, then the launchers found
/// on PATH), and this system's terminals that are installed, the one radar
/// runs in (`current`) first.
pub fn detect(env: &Env, current: Option<&'static str>, on_path: impl Fn(&str) -> bool, app: impl Fn(&str) -> bool) -> Terminals {
    let desktop = !env.ssh && (env.macos || env.windows || env.wsl || env.wayland || env.x11);
    let found = |what: &str, how: &str| Some((what.to_string(), how.to_string()));
    let chosen = if env.tmux {
        found("tmux", "a new window, inside tmux")
    } else if !desktop {
        None
    } else if env.macos {
        found("Terminal", "the Mac's own")
    } else if env.windows || env.wsl {
        found("Windows Terminal", "on Windows")
    } else if let Some(t) = &env.terminal
        && let Some(prog) = system::split_command(t).into_iter().next()
    {
        Some((prog, "from $TERMINAL".into()))
    } else {
        ["xdg-terminal-exec", "x-terminal-emulator"]
            .into_iter()
            .find(|p| on_path(p))
            .and_then(|p| found(p, "found on PATH"))
    };
    let mine: &[Known] = if env.macos {
        &MAC
    } else if env.windows {
        &[]
    } else {
        &LINUX
    };
    let mut installed: Vec<Known> = mine
        .iter()
        .filter(|k| match k.look {
            Look::Path(p) => on_path(p),
            Look::App(a) => app(a),
        })
        .copied()
        .collect();
    installed.sort_by_key(|k| Some(k.name) != current);
    let current = current.filter(|c| installed.iter().any(|k| k.name == *c));
    Terminals { chosen, found: installed, current }
}

/// This machine's, read now (a few looks along PATH and in the
/// Applications folders).
pub fn here() -> Terminals {
    let env = Env::detect();
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let var = |k: &str| std::env::var(k).ok();
    detect(&env, running_in(&var, env.macos), system::on_path, |a| {
        app_places(home.as_deref(), a).iter().any(|p| p.exists())
    })
}

/// The terminal named `name`, of either system.
pub fn by_name(name: &str) -> Option<&'static Known> {
    LINUX.iter().chain(MAC.iter()).find(|k| k.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linux() -> Env {
        Env { wayland: true, ..Env::default() }
    }

    #[test]
    fn it_says_what_opens_and_how_it_was_found() {
        let none = |_: &str| false;
        let t = detect(&Env { terminal: Some("ghostty --gtk-single-instance=true".into()), ..linux() }, None, none, none);
        assert_eq!(t.chosen, Some(("ghostty".into(), "from $TERMINAL".into())));
        let t = detect(&linux(), None, |p| p == "x-terminal-emulator", none);
        assert_eq!(t.chosen, Some(("x-terminal-emulator".into(), "found on PATH".into())));
        assert_eq!(detect(&linux(), None, none, none).chosen, None, "nothing to start");
        let t = detect(&Env { tmux: true, ssh: true, ..Env::default() }, None, none, none);
        assert_eq!(t.chosen.unwrap().0, "tmux");
        assert_eq!(detect(&Env { ssh: true, ..linux() }, None, none, none).chosen, None, "over ssh no window opens");
    }

    #[test]
    fn only_this_systems_installed_terminals_are_listed_the_one_in_use_first() {
        let t = detect(&linux(), Some("foot"), |p| ["kitty", "foot"].contains(&p), |_| true);
        let names: Vec<_> = t.found.iter().map(|k| k.name).collect();
        assert_eq!(names, ["foot", "kitty"], "only what is installed, the one in use first");
        assert_eq!(t.current, Some("foot"));
        let mac = detect(&Env { macos: true, ..Env::default() }, Some("Ghostty"), |_| true, |a| ["Terminal.app", "Ghostty.app"].contains(&a));
        let names: Vec<_> = mac.found.iter().map(|k| k.name).collect();
        assert_eq!(names, ["Ghostty", "Terminal"], "Mac apps on the Mac, no Linux programs");
        // In use but not installed as an app (run from a build): not marked.
        let mac = detect(&Env { macos: true, ..Env::default() }, Some("Warp"), |_| true, |a| a == "Terminal.app");
        assert_eq!(mac.current, None);
        assert_eq!(by_name("kitty").map(|k| k.command), Some("kitty --directory {dir} {cmd}"));
    }

    #[test]
    fn the_terminal_in_use_is_read_from_what_it_sets() {
        let env = |pairs: &'static [(&'static str, &'static str)]| move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string());
        assert_eq!(running_in(&env(&[("GHOSTTY_RESOURCES_DIR", "/x")]), false), Some("ghostty"));
        assert_eq!(running_in(&env(&[("GHOSTTY_RESOURCES_DIR", "/x")]), true), Some("Ghostty"));
        assert_eq!(running_in(&env(&[("TERM_PROGRAM", "Apple_Terminal")]), true), Some("Terminal"));
        assert_eq!(running_in(&env(&[("TERM_PROGRAM", "iTerm.app")]), true), Some("iTerm"));
        assert_eq!(running_in(&env(&[("TERM", "foot")]), false), Some("foot"));
        assert_eq!(running_in(&env(&[("TERM", "xterm-256color")]), false), None);
    }
}

/// What this machine has, printed: `cargo nextest run -p radar-git
/// --run-ignored only what_is_here --no-capture`, on each system by hand.
#[cfg(test)]
#[test]
#[ignore = "reads the real machine"]
fn what_is_here() {
    let t = here();
    println!("chosen {:?}", t.chosen);
    println!("current {:?}", t.current);
    for k in &t.found {
        println!("found {} -> {}", k.name, k.command);
    }
}
