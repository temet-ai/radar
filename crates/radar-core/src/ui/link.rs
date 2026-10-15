//! Links the terminal can open: words drawn with where they go, so a
//! Ctrl/Cmd-click on them opens it (OSC 8 hyperlinks).
//!
//! ratatui's cells have no field for a link, and a link must survive
//! everything a cell goes through: wrapping, a scroll view's copy, the
//! selection's reversed look, and the diff that redraws only changed cells.
//! So a link travels in the cell's style: [`style`] gives the words an
//! underline colour that is an id for the url (the url is kept in a table
//! beside it), and the runner draws through [`Links`], a backend that turns
//! a run of cells with one id into `ESC ] 8 ; ; url ESC \` … `ESC ] 8 ; ;
//! ESC \` and hands the cells on without the id, so the terminal never sees
//! that colour. A changed link is a changed cell, so the diff redraws it;
//! text is untouched, so widths and `to_text` stay as they were.
//!
//! Off unless the runner turns it on ([`set`]) for a terminal known to
//! handle them (`[theme] hyperlinks = auto|on|off`, [`wanted`]); with it
//! off, [`style`] changes nothing.

use std::cell::Cell as Flag;
use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::{Mutex, OnceLock};

use ratatui::backend::{Backend, ClearType, CrosstermBackend, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

thread_local! {
    /// Whether links are drawn, on the thread that draws.
    static ON: Flag<bool> = const { Flag::new(false) };
}

/// The urls drawn lately, by id.
fn table() -> &'static Mutex<HashMap<u32, String>> {
    static T: OnceLock<Mutex<HashMap<u32, String>>> = OnceLock::new();
    T.get_or_init(|| Mutex::new(HashMap::new()))
}

/// How many urls are kept before the table starts again.
const KEEP: usize = 4096;

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

/// FNV-1a, folded to 24 bits: the same url always has the same id, so an
/// unchanged link is an unchanged cell.
fn id_of(url: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in url.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    (h ^ (h >> 24)) & 0x00ff_ffff
}

fn colour(id: u32) -> Color {
    Color::Rgb((id >> 16) as u8, (id >> 8) as u8, id as u8)
}

/// `style` carrying `url`, when links are on and the url is one a terminal
/// may open; else `style` as it was.
pub fn style(style: Style, url: &str) -> Style {
    if !on() || !openable(url) {
        return style;
    }
    let id = id_of(url);
    if let Ok(mut t) = table().lock() {
        if t.len() >= KEEP && !t.contains_key(&id) {
            t.clear();
        }
        t.insert(id, url.to_string());
    }
    style.underline_color(colour(id))
}

/// Words that open `url`.
pub fn span(text: impl Into<String>, s: Style, url: &str) -> Span<'static> {
    Span::styled(text.into(), style(s, url))
}

/// Every span of `line` opening `url`: a row that is a link.
pub fn line(mut line: Line<'static>, url: &str) -> Line<'static> {
    for s in &mut line.spans {
        s.style = style(s.style, url);
    }
    line
}

/// The url a cell carries, if any.
pub fn of(cell: &Cell) -> Option<String> {
    let Color::Rgb(r, g, b) = cell.underline_color else {
        return None;
    };
    let id = (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b);
    table().lock().ok()?.get(&id).cloned()
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

/// `url` as the bytes an OSC 8 sequence may hold: anything outside
/// printable ASCII percent-encoded.
fn encoded(url: &str) -> String {
    let mut out = String::with_capacity(url.len());
    for b in url.bytes() {
        if (0x21..0x7f).contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The terminal backend the runner draws through: crossterm's, with the
/// cells that carry a link written inside an OSC 8 hyperlink.
pub struct Links<W: Write> {
    inner: CrosstermBackend<W>,
}

impl<W: Write> Links<W> {
    pub fn new(writer: W) -> Self {
        Self {
            inner: CrosstermBackend::new(writer),
        }
    }

    /// One run of cells: inside a hyperlink when it has a url.
    fn run(&mut self, cells: &[(u16, u16, Cell)], url: Option<&str>) -> io::Result<()> {
        if cells.is_empty() {
            return Ok(());
        }
        if let Some(u) = url {
            write!(self.inner, "\x1b]8;;{}\x1b\\", encoded(u))?;
        }
        self.inner.draw(cells.iter().map(|(x, y, c)| (*x, *y, c)))?;
        if url.is_some() {
            self.inner.write_all(b"\x1b]8;;\x1b\\")?;
        }
        Ok(())
    }
}

impl<W: Write> Write for Links<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        Write::flush(&mut self.inner)
    }
}

impl<W: Write> Backend for Links<W> {
    type Error = io::Error;

    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        let shown = on();
        let mut cells: Vec<(u16, u16, Cell)> = Vec::new();
        let mut url: Option<String> = None;
        for (x, y, cell) in content {
            let here = of(cell);
            let mut c = cell.clone();
            if here.is_some() {
                // The id is for this backend only; the terminal sees no colour.
                c.underline_color = Color::Reset;
            }
            let here = here.filter(|_| shown);
            if here != url {
                self.run(&cells, url.as_deref())?;
                cells.clear();
                url = here;
            }
            cells.push((x, y, c));
        }
        self.run(&cells, url.as_deref())
    }

    fn append_lines(&mut self, n: u16) -> io::Result<()> {
        self.inner.append_lines(n)
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        self.inner.hide_cursor()
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        self.inner.show_cursor()
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        self.inner.get_cursor_position()
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        self.inner.set_cursor_position(position)
    }

    fn clear(&mut self) -> io::Result<()> {
        self.inner.clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
        self.inner.clear_region(clear_type)
    }

    fn size(&self) -> io::Result<Size> {
        self.inner.size()
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        self.inner.window_size()
    }

    fn flush(&mut self) -> io::Result<()> {
        Backend::flush(&mut self.inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k: &str| map.get(k).cloned()
    }

    #[test]
    fn with_links_off_a_style_is_left_as_it_was() {
        set(false);
        let s = Style::default().fg(Color::Blue);
        assert_eq!(style(s, "https://example.com/a"), s);
    }

    #[test]
    fn a_link_rides_in_the_style_and_is_read_back_from_the_cell() {
        set(true);
        let s = style(Style::default(), "https://example.com/a");
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 1));
        buf.set_span(0, 0, &Span::styled("see", s), 10);
        assert_eq!(of(&buf[(0, 0)]).as_deref(), Some("https://example.com/a"));
        assert_eq!(of(&buf[(3, 0)]), None);
        // The same url, the same id: an unchanged link is an unchanged cell.
        assert_eq!(style(Style::default(), "https://example.com/a"), s);
        assert_ne!(style(Style::default(), "https://example.com/b"), s);
        set(false);
    }

    #[test]
    fn only_web_and_mail_links_without_control_characters_are_handed_on() {
        assert!(openable("https://example.com/x?a=1&b=2"));
        assert!(openable("mailto:alice@example.com"));
        assert!(!openable("javascript:alert(1)"));
        assert!(!openable("file:///etc/passwd"));
        assert!(!openable("https://example.com/\x1b]8;;evil\x1b\\"));
        assert!(!openable("https://example.com/\x07"));
        set(true);
        assert_eq!(style(Style::default(), "javascript:x"), Style::default());
        set(false);
    }

    /// What the backend writes for a frame with `text` linked at the start.
    fn drawn(linked: bool) -> String {
        set(true);
        let mut buf = Buffer::empty(Rect::new(0, 0, 12, 1));
        let s = if linked {
            style(Style::default(), "https://example.com/ü")
        } else {
            Style::default()
        };
        buf.set_span(0, 0, &Span::styled("plan", s), 12);
        buf.set_span(5, 0, &Span::raw("then"), 12);
        let out = Shared::default();
        let mut b = Links::new(out.clone());
        b.draw(buf.content.iter().enumerate().map(|(i, c)| (i as u16, 0, c)))
            .unwrap();
        set(false);
        String::from_utf8(out.0.borrow().clone()).unwrap()
    }

    /// A writer the test reads after the backend wrote to it.
    #[derive(Clone, Default)]
    struct Shared(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn the_backend_writes_a_linked_run_inside_osc_8_and_never_the_id() {
        let out = drawn(true);
        let open = out.find("\x1b]8;;https://example.com/%C3%BC\x1b\\").expect(&out);
        let close = out.find("\x1b]8;;\x1b\\").expect(&out);
        let plan = out.find("plan").unwrap();
        let then = out.find("then").unwrap();
        assert!(open < plan && plan < close && close < then, "{out:?}");
        assert_eq!(out.matches("\x1b]8;;").count(), 2, "one open, one close: {out:?}");
        // No underline colour reached the terminal.
        assert!(!out.contains("\x1b[58;"), "{out:?}");
        let plain = drawn(false);
        assert!(!plain.contains("\x1b]8"), "{plain:?}");
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

    #[test]
    fn a_chip_with_a_url_opens_it() {
        let t = crate::theme::Theme::new(crate::design::theme::Tier::TrueColor);
        set(true);
        let c = crate::compose::Chip::new("asked 3d", crate::plugin::Tone::Muted);
        assert_eq!(super::super::chips::chip(&t, &c).style.underline_color, None);
        let c = c.link("https://example.com/thread");
        let s = super::super::chips::chip(&t, &c);
        assert_eq!(s.content, " asked 3d ");
        let mut buf = Buffer::empty(Rect::new(0, 0, 12, 1));
        buf.set_span(0, 0, &s, 12);
        assert_eq!(of(&buf[(1, 0)]).as_deref(), Some("https://example.com/thread"));
        set(false);
    }

    #[test]
    fn a_line_can_be_one_link() {
        set(true);
        let l = line(Line::from(vec![Span::raw("a"), Span::raw("b")]), "https://x.example/");
        assert!(l.spans.iter().all(|s| s.style.underline_color.is_some()));
        set(false);
    }
}
