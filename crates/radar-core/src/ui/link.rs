//! Links the terminal can open: whether this terminal opens words drawn with
//! where they go (OSC 8 hyperlinks), and which addresses may be handed to
//! it. Writing the sequences is the renderer's.
//!
//! Off unless the runner turns it on ([`set`]) for a terminal known to
//! handle them (`[theme] hyperlinks = auto|on|off`, [`wanted`]).

use std::cell::Cell as Flag;

thread_local! {
    /// Whether links are drawn, on the thread that draws.
    static ON: Flag<bool> = const { Flag::new(false) };
}

/// Whether links are drawn as links on this thread.
pub fn on() -> bool {
    ON.with(Flag::get)
}

/// Draw links as links (the runner, once it knows the terminal), or not.
pub fn set(on: bool) {
    ON.with(|f| f.set(on));
}

/// A url a terminal may be handed: a web or mail link with no control
/// characters (which could end the sequence and start another).
pub fn openable(url: &str) -> bool {
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:"))
        && url.len() <= 2048
        && !url.chars().any(|c| c.is_control())
}

/// What to do with `[theme] hyperlinks`: `on`, `off`, or `auto` (on for a
/// terminal known to open them, off for any other, inside tmux and over a
/// connection whose terminal is not known). `var` reads the environment.
pub fn wanted(setting: &str, var: &dyn Fn(&str) -> Option<String>) -> bool {
    match setting {
        "on" => true,
        "off" => false,
        _ => detect(var),
    }
}

/// Whether this terminal is one known to open OSC 8 links.
pub fn detect(var: &dyn Fn(&str) -> Option<String>) -> bool {
    let set = |k: &str| var(k).is_some_and(|v| !v.is_empty());
    let term = var("TERM").unwrap_or_default();
    if term == "dumb" || term.starts_with("screen") || term.starts_with("tmux") || set("TMUX") {
        return false;
    }
    let program = var("TERM_PROGRAM").unwrap_or_default();
    if [
        "ghostty",
        "WezTerm",
        "iTerm.app",
        "vscode",
        "Hyper",
        "rio",
        "WarpTerminal",
        "Tabby",
    ]
    .contains(&program.as_str())
    {
        return true;
    }
    if ["kitty", "ghostty", "wezterm", "foot", "alacritty", "contour"]
        .iter()
        .any(|t| term.contains(t))
    {
        return true;
    }
    if set("KITTY_WINDOW_ID") || set("WT_SESSION") || set("GHOSTTY_RESOURCES_DIR") && !set("SSH_CONNECTION") {
        return true;
    }
    var("VTE_VERSION")
        .and_then(|v| v.trim().parse::<u32>().ok())
        .is_some_and(|v| v >= 5000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k: &str| map.get(k).cloned()
    }

    #[test]
    fn only_web_and_mail_links_without_control_characters_are_handed_on() {
        assert!(openable("https://example.com/x?a=1&b=2"));
        assert!(openable("mailto:alice@example.com"));
        assert!(!openable("javascript:alert(1)"));
        assert!(!openable("file:///etc/passwd"));
        assert!(!openable("https://example.com/\x1b]8;;evil\x1b\\"));
        assert!(!openable("https://example.com/\x07"));
    }

    #[test]
    fn auto_turns_links_on_only_for_terminals_known_to_open_them() {
        let auto = |pairs: &[(&str, &str)]| wanted("auto", &env(pairs));
        assert!(auto(&[("TERM_PROGRAM", "ghostty"), ("TERM", "xterm-256color")]));
        assert!(auto(&[("TERM", "xterm-kitty")]));
        assert!(auto(&[("TERM_PROGRAM", "WezTerm")]));
        assert!(auto(&[("TERM_PROGRAM", "iTerm.app")]));
        assert!(auto(&[("VTE_VERSION", "7600")]));
        assert!(auto(&[("WT_SESSION", "x")]));
        // A multiplexer between radar and herdr's terminal passes them on;
        // herdr keeps TERM_PROGRAM.
        assert!(auto(&[("TERM_PROGRAM", "ghostty"), ("HERDR_ENV", "1")]));
        assert!(!auto(&[("TERM", "xterm-256color")]));
        assert!(!auto(&[("VTE_VERSION", "4200")]));
        assert!(!auto(&[("TERM", "dumb"), ("TERM_PROGRAM", "ghostty")]));
        assert!(!auto(&[("TERM_PROGRAM", "tmux"), ("TERM", "tmux-256color")]));
        assert!(!auto(&[("TERM_PROGRAM", "ghostty"), ("TMUX", "/tmp/tmux-1/default,1,0")]));
        assert!(!auto(&[("GHOSTTY_RESOURCES_DIR", "/usr/share/ghostty"), ("SSH_CONNECTION", "1 2 3 4")]));
        assert!(wanted("on", &env(&[("TERM", "dumb")])));
        assert!(!wanted("off", &env(&[("TERM_PROGRAM", "ghostty")])));
    }
}
