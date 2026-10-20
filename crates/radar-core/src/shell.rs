//! The chrome every screen shares: the top bar (brand, screen tabs with
//! attention marks, sync state), the status line (mode, keys, counts), menus and help.
//! A plugin's screen draws the whole body between them. The settings screen
//! is laid out here: nav (the sections tree) │ middle (the section) │ detail (what the selected row is).

use crate::design::theme::{Slot, Tier};
use crate::view::chrome::{ChromeView, Hint, KeysAt, StatusLine};
use crate::view::{Chord, Ink, Target};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use tui_tree_widget::TreeState;

use crate::compose::Parts;
#[cfg(test)]
use crate::design::help_bar::KeyHint;
use crate::host::{Hit, Host, Owner, View};
use crate::motion::{self, Region};
use crate::plugin::Pane;
use crate::view_tui::Cx;
use crate::theme::Theme;
use crate::ui::icon::{self, Icon};
use crate::{ui, widgets};

pub(crate) mod menu;
mod notices;
pub(crate) use notices::content as notice_content;
mod setup;
mod tour;

#[cfg(test)]
mod setup_hints_tests;

pub fn render(host: &Host, theme: &Theme, f: &mut Frame) {
    #[cfg(feature = "test-support")]
    host.scenario_shell_frames
        .set(host.scenario_shell_frames.get() + 1);
    let area = f.area();
    let cursor = std::cell::Cell::new(None);
    host.hits.borrow_mut().clear();
    host.frontend.begin_selectable();
    motion::begin();
    // The chrome is view data the host produces (having measured what
    // scrolls at this size); this paints it.
    host.frontend.set_lead(crate::ui::icon::lead_width(theme));
    host.frontend.set_mode(host.mode());
    let displayed = host.displayed_screen(area.width, area.height);
    let chrome = host.chrome_view_with(area.width, area.height, theme);
    if host.motion_enabled() && chrome.menu.is_none() && chrome.tour.is_none() && chrome.notice_history.is_none() {
        motion::frames::context(host.motion_context(), host.motion_fold().cloned());
    }
    if chrome.notice_history.is_some() {
        *host.hovered.borrow_mut() = None;
        let [top, body, bottom] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
        ])
        .areas(area);
        let buf = f.buffer_mut();
        top_bar(host, theme, &chrome, top, buf);
        notices::history(host, theme, &chrome, body, buf);
        status_line(host, theme, &chrome.status, bottom, buf);
        hover(host, theme, buf, true);
        return;
    }
    if let Some(view) = &chrome.history {
        *host.hovered.borrow_mut() = None;
        let [top, body] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(area);
        let buf = f.buffer_mut();
        top_bar(host, theme, &chrome, top, buf);
        // Painted from the chrome's history view; the overlay records which
        // target each area is, so a press resolves to it.
        if let Some(history) = &host.history {
            history.paint(view, body, buf, theme);
            // Its events (rows), its detail (text) or its help (text): each
            // panel's inside; the help alone fills the body.
            let inside = |r: Rect| r.inner(ratatui::layout::Margin::new(1, 1));
            let (list, detail) = history.panes();
            if list.is_empty() && detail.is_empty() {
                host.frontend.mark_selectable(inside(body), true);
            }
            host.frontend.mark_selectable(inside(list), false);
            host.frontend.mark_selectable(inside(detail), true);
            host.hits
                .borrow_mut()
                .extend(history.hit_regions().into_iter().map(|area| (area, Hit::History)));
        }
        hover(host, theme, f.buffer_mut(), true);
        return;
    }
    {
        let buf = f.buffer_mut();
        buf.set_style(area, theme.bg(Slot::BgBase));
        let [top, body, bottom] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
        ])
        .areas(area);
        motion::region(Region::Body, body);

        match &chrome.setup {
            Some(s) => setup::top_bar(host, theme, &chrome, s, top, buf),
            None => top_bar(host, theme, &chrome, top, buf),
        }
        let body = if !chrome.sources.is_empty() && body.height > 1 {
            let [source_row, content] =
                Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(body);
            source_tabs(host, theme, &chrome, source_row, buf);
            content
        } else {
            body
        };
        match host.screen() {
            _ if let Some(s) = &chrome.setup => setup::body(host, theme, s, body, buf, &cursor),
            Some((p, _)) if !host.ready(p) => crate::parts::setup_page(
                host, p, theme, body, buf,
                pane_ramp_allowed(&chrome, host.dock_view().is_some_and(|d| d.visible && d.focused)),
            ),
            Some((p, spec)) => {
                let plugin_hits = std::cell::RefCell::new(Vec::new());
                let cx = Cx {
                    theme,
                    now: host.now,
                    focused: false,
                    focus: host.focus,
                    bar: host.bar || host.sources,
                    tabs: host.tabs && host.page_tabs().is_some(),
                    cursor: &cursor,
                    hits: &plugin_hits,
                    parts: Parts::new(&host.plugins, Some(p)),
                };
                host.frontend.set_lead(crate::ui::icon::lead_width(theme));
                match displayed {
                    Some(mut view) => {
                        // Ratatui draws one line in the dock's place
                        // ([`crate::dock::NOTICE`]), under the middle pane.
                        view.dock = host.dock_view();
                        host.frontend.resolve_pictures(&view, host.plugins[p].as_ref());
                        crate::view_tui::render_screen_with_ramp_and_jumps(
                            &host.frontend, &view, body, buf, &cx,
                            pane_ramp_allowed(&chrome, false), &chrome.pane_jumps,
                        )
                    }
                    None => widgets::empty(buf, body, theme, "This part has no view for this screen."),
                }
                adopt_hits(host, p, spec.id, plugin_hits);
            }
            None => settings(host, theme, &chrome, body, buf, &cursor),
        }
        // The menu and the status line paint from the chrome alone.
        let modal = chrome
            .menu
            .as_ref()
            .map(|view| (view, menu::Layout::new(view, theme, body)));
        status_line(host, theme, &chrome.status, bottom, buf);
        if let Some(card) = &chrome.tour {
            tour::card(theme, card, body, buf);
        }
        // A menu, question or help sits over a scrim and throws a shadow;
        // nothing behind it is lit.
        let lit = modal.is_none();
        if let Some((view, layout)) = modal {
            cursor.set(None);
            let frame = layout.frame();
            ui::scrim::scrim(buf, frame, theme);
            // Nothing behind the scrim is selected: only the menu's inside.
            host.frontend.begin_selectable();
            host.frontend.mark_selectable(frame.inner(ratatui::layout::Margin::new(1, 1)), false);
            menu::render(host, view, theme, layout, buf, &cursor);
            ui::scrim::shadow(buf, frame, theme);
        }
        hover(host, theme, buf, lit);
    }
    // The terminal's own cursor shows only while text is being edited.
    if let Some(at) = cursor.get() {
        f.set_cursor_position(at);
    }
}

/// Composite transient notices after motion has retained the clean frame.
/// The runner does not invoke this while the splash is showing.
pub fn render_notices(host: &Host, theme: &Theme, buf: &mut Buffer) {
    notices::transient(host, theme, buf);
}

fn source_tabs(host: &Host, theme: &Theme, chrome: &ChromeView, area: Rect, buf: &mut Buffer) {
    buf.set_style(area, theme.bg(Slot::BgOverlay));
    let mut x = area.x;
    for tab in &chrome.sources {
        let selected = tab.active;
        let style = if selected && chrome.keys_at == KeysAt::Sources {
            theme
                .bold(Slot::BgBase)
                .patch(Style::new().bg(theme.s(Slot::AccentPrimary).fg.unwrap_or_default()))
        } else if selected {
            theme
                .bold(Slot::TextEmphasis)
                .patch(theme.bg(Slot::BgSelection))
        } else {
            theme.s(Slot::TextSecondary)
        };
        let label = format!("  {}  ", tab.label);
        let width = label.chars().count() as u16;
        if x >= area.right() {
            break;
        }
        let rect = Rect::new(x, area.y, width.min(area.right() - x), 1);
        buf.set_string(x, area.y, label, style);
        if let Some(hit) = host.chrome_hit(&tab.target) {
            host.hits.borrow_mut().push((rect, hit));
        }
        x = x.saturating_add(width);
    }
}

/// Light what the pointer is over, as this frame's hits place it (a
/// redraw can move a row under a still pointer). A menu's item is not lit:
/// the menu's highlight moved there instead. Nothing is lit unless `lit`.
fn hover(host: &Host, theme: &Theme, buf: &mut Buffer, lit: bool) {
    let now = host.pointer.and_then(|at| host.hover_at(at));
    if let Some((rect, hit)) = &now
        && lit
        && !matches!(hit, Hit::MenuItem(_))
    {
        ui::hover::paint(buf, *rect, theme);
    }
    *host.hovered.borrow_mut() = now;
}

/// Move a plugin's marked areas into the host's hit map.
fn adopt_hits(
    host: &Host,
    plugin: usize,
    scope: &'static str,
    hits: std::cell::RefCell<Vec<(Rect, Pane, String)>>,
) {
    let mut all = host.hits.borrow_mut();
    for (rect, pane, id) in hits.into_inner() {
        all.push((
            rect,
            Hit::Plugin {
                plugin,
                scope,
                pane,
                id,
            },
        ));
    }
}

/// A name with its shortcut letter underlined in place, menu-bar style:
/// `Books` with the B underlined, for the key `B`.
fn mnemonic(name: &str, key: char, base: Style, mark: Style) -> Vec<Span<'static>> {
    let mut chars: Vec<char> = name.chars().collect();
    if let Some(c) = chars.first_mut() {
        *c = c.to_ascii_uppercase();
    }
    let at = chars.iter().position(|c| c.eq_ignore_ascii_case(&key));
    match at {
        Some(i) => vec![
            Span::styled(chars[..i].iter().collect::<String>(), base),
            Span::styled(chars[i].to_string(), mark),
            Span::styled(chars[i + 1..].iter().collect::<String>(), base),
        ],
        // The key is not in the name: say it after, quietly.
        None => vec![
            Span::styled(chars.iter().collect::<String>(), base),
            Span::styled(format!(" ({key})"), mark),
        ],
    }
}

/// ` radar   Books ●   Settings          ⠷ synced 12s · every 60s `
/// with each tab's shortcut letter underlined.
fn top_bar(host: &Host, theme: &Theme, chrome: &ChromeView, area: Rect, buf: &mut Buffer) {
    buf.set_style(area, theme.bg(Slot::BgOverlay));
    let bar = &chrome.top_bar;
    let mut right: Vec<Span<'static>> = Vec::new();
    if bar.busy {
        let frames = throbber_widgets_tui::BRAILLE_SIX.symbols;
        right.push(Span::styled(
            format!("{} ", frames[host.frame % frames.len()]),
            theme.s(Slot::StatusInfo),
        ));
    }
    if let Some(words) = &bar.words {
        right.push(Span::styled(format!("{} ", words.text), ink_style(theme, words.ink, false)));
    }
    let terminal_status = bar
        .terminal
        .as_ref()
        .map(|t| Span::styled(format!("{} ", t.text), ink_style(theme, t.ink, false)));
    if let Some(status) = &terminal_status {
        right.push(status.clone());
    }
    // The mode, at all times, at the right end; a click opens its menu.
    let mode = Span::styled(
        format!(" {} ", mode_words(&bar.mode)),
        ink_style(theme, bar.mode.ink, bar.mode.strong),
    );
    let mode_w = mode.width() as u16;
    let mut right = Line::from(right);
    let mut part_w = right.width() as u16;
    // The tabs carry their icons while they and the words on the right
    // fit side by side; where they would meet, the icons go first, then
    // the part's words. The mode stays.
    let tabs = |icons, compact| tabs(host, theme, chrome, area, icons, compact);
    let (mut spans, mut hits) = tabs(true, false);
    let width = |s: &[Span<'static>]| s.iter().map(|s| s.width() as u16).sum::<u16>();
    if width(&spans) + part_w + mode_w > area.width {
        (spans, hits) = tabs(false, false);
    }
    if width(&spans) + part_w + mode_w > area.width {
        (spans, hits) = tabs(false, true);
    }
    if width(&spans) + part_w + mode_w > area.width {
        let available = area.width.saturating_sub(width(&spans) + mode_w);
        if available > 0 {
            let words = right
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>();
            let style = right.spans.first().map_or(Style::new(), |span| span.style);
            right = Line::from(Span::styled(ui::fit(&words, available as usize), style));
            part_w = right.width() as u16;
        }
    }
    if width(&spans) + part_w + mode_w > area.width {
        right = Line::from(terminal_status.into_iter().collect::<Vec<_>>());
        part_w = right.width() as u16;
    }
    if width(&spans) + part_w + mode_w > area.width {
        (spans, hits) = tabs(false, false);
    }
    if width(&spans) + part_w + mode_w > area.width {
        (spans, hits) = tabs(false, true);
    }
    let part_w = if width(&spans) + part_w + mode_w > area.width {
        0
    } else {
        part_w
    };
    if part_w == 0 {
        right = Line::default();
    }
    right.spans.push(mode);
    let right_w = part_w + mode_w;
    let tab_area = Rect {
        width: if right_w < area.width { area.width - right_w } else { area.width },
        ..area
    };
    hits.iter_mut().for_each(|(rect, _)| *rect = rect.intersection(tab_area));
    hits.retain(|(rect, _)| !rect.is_empty());
    host.hits.borrow_mut().append(&mut hits);
    buf.set_line(area.x, area.y, &Line::from(spans), tab_area.width);
    if right_w < area.width {
        buf.set_line(area.x + area.width - right_w, area.y, &right, right_w);
        // A padded target, as a label's (`Cx::hit_label`).
        let label = Rect {
            x: area.x + area.width - mode_w,
            y: area.y,
            width: mode_w,
            height: 1,
        };
        if let Some(hit) = host.chrome_hit(&bar.mode.target) {
            host.hits.borrow_mut().push((crate::view_tui::padded(label, area), hit));
        }
    }
}

/// A chrome word's style: its ink's slot, bold when strong.
/// The mode chip's words as the terminal draws them: the pack's icon, else
/// the symbol, then the word.
fn mode_words(mode: &crate::view::chrome::ModeChip) -> String {
    let lead = mode.icon.map_or_else(String::new, icon::plain);
    let lead = if lead.is_empty() && mode.icon.is_some() {
        if mode.icon == Some(Icon::ModeRead) { "◉ ".to_string() } else { "✎ ".to_string() }
    } else {
        lead
    };
    format!("{lead}{}", mode.text)
}

fn ink_style(theme: &Theme, ink: Ink, strong: bool) -> Style {
    let slot = crate::view_tui::slot(ink).unwrap_or(Slot::TextPrimary);
    if strong { theme.bold(slot) } else { theme.s(slot) }
}

/// Body ownership is separate from legacy focus styles and in-pane capture.
fn pane_ramp_allowed(chrome: &ChromeView, dock_takes_keys: bool) -> bool {
    chrome.keys_at == KeysAt::Body && chrome.menu.is_none() && chrome.notice_history.is_none() && !dock_takes_keys
}

/// Keep the original padded span below truecolor, including its clipping.
fn brand_spans(theme: &Theme, brand: &str, trailing: bool) -> Vec<Span<'static>> {
    let base = theme.bold(Slot::AccentPrimary);
    if theme.tier() != Tier::TrueColor {
        let text = if trailing { format!(" {brand} ") } else { format!(" {brand}") };
        return vec![Span::styled(text, base)];
    }
    let mut spans = vec![Span::styled(" ", base)];
    spans.extend(ui::frame::wordmark(theme, brand, base));
    if trailing {
        spans.push(Span::styled(" ", base));
    }
    spans
}

#[cfg(test)]
mod focus_ramp_tests {
    use super::*;

    #[test]
    fn only_body_keys_without_a_menu_or_shown_dock_allow_pane_ramps() {
        let mut chrome = ChromeView::new(80, 24);
        assert!(pane_ramp_allowed(&chrome, false));
        assert!(!pane_ramp_allowed(&chrome, true));
        for keys in [KeysAt::Bar, KeysAt::Sources, KeysAt::PageTabs] {
            chrome.keys_at = keys;
            assert!(!pane_ramp_allowed(&chrome, false));
        }
        chrome.keys_at = KeysAt::Body;
        chrome.menu = Some(crate::view::chrome::MenuView::new("menu", "choices", vec![]));
        assert!(!pane_ramp_allowed(&chrome, false));
    }

    #[test]
    fn normal_and_setup_wordmarks_keep_padding_and_exact_low_tier_spans() {
        for tier in [Tier::TrueColor, Tier::Ansi256, Tier::Ansi16, Tier::Mono] {
            let theme = Theme::build(&crate::theme::ThemeConfig { icons: "off".into(), ..Default::default() }, tier, &[]);
            let base = theme.bold(Slot::AccentPrimary);
            for trailing in [false, true] {
                let want = if trailing { " a\u{301}界z " } else { " a\u{301}界z" };
                let spans = brand_spans(&theme, "a\u{301}界z", trailing);
                assert_eq!(spans.iter().map(|s| s.content.as_ref()).collect::<String>(), want);
                if tier == Tier::TrueColor {
                    assert_eq!(spans[0], Span::styled(" ", base));
                    assert_eq!(spans[1].content, "a\u{301}");
                    assert_eq!(spans[1].style.fg, theme.s(Slot::GlowPurple).fg);
                    assert_eq!(spans[3].style.fg, theme.s(Slot::GlowCyan).fg);
                    if trailing { assert_eq!(spans.last().unwrap(), &Span::styled(" ", base)); }
                } else {
                    assert_eq!(spans, vec![Span::styled(want, base)]);
                    for width in 1..=8 {
                        let area = Rect::new(0, 0, width, 1);
                        let mut old = Buffer::empty(area);
                        let mut new = Buffer::empty(area);
                        old.set_line(0, 0, &Line::from(Span::styled(want, base)), width);
                        new.set_line(0, 0, &Line::from(spans.clone()), width);
                        assert_eq!(new, old);
                    }
                }
            }
        }
    }
}

/// The top bar's `radar` and its tabs, with or without their icons, and each
/// tab's area for clicks.
fn tabs(
    host: &Host,
    theme: &Theme,
    chrome: &ChromeView,
    area: Rect,
    icons: bool,
    compact: bool,
) -> (Vec<Span<'static>>, Vec<(Rect, Hit)>) {
    let mut hits = Vec::new();
    let brand = format!(" {} ", chrome.top_bar.brand);
    let mut x = area.x + ui::width(&brand) as u16 + 2;
    let mut spans = brand_spans(theme, &chrome.top_bar.brand, true);
    spans.push(Span::raw("  "));
    let bar_has_keys = chrome.keys_at == KeysAt::Bar;
    let mut tab = |spans: &mut Vec<Span<'static>>,
                   key: char,
                   name: &str,
                   icon: Option<Icon>,
                   attention: bool,
                   on: bool,
                   hit: Option<Hit>| {
        let base = if on && bar_has_keys {
            // The top bar has the keys: its tab stands out as a focused
            // row does.
            theme
                .bold(Slot::BgBase)
                .patch(Style::new().bg(theme.s(Slot::AccentPrimary).fg.unwrap_or_default()))
        } else if on {
            theme
                .bold(Slot::TextEmphasis)
                .patch(theme.bg(Slot::BgSelection))
        } else {
            theme.s(Slot::TextSecondary)
        };
        let bg = base.bg.map_or(Style::new(), |c| Style::new().bg(c));
        let mark = base
            .patch(theme.bold(Slot::AccentSecondary))
            .add_modifier(Modifier::UNDERLINED);
        let mut parts = vec![Span::styled(" ", base)];
        if let Some(i) = icon.filter(|_| icons) {
            let mut lead = icon::lead(theme, i);
            lead.style = base.patch(lead.style);
            parts.push(lead);
        }
        parts.extend(mnemonic(name, key, base, mark));
        if attention {
            parts.push(Span::styled(" ", base));
            let mut dot = icon::attention(theme);
            dot.style = dot.style.patch(bg);
            parts.push(dot);
        }
        parts.push(Span::styled(" ", base));
        let w: u16 = parts.iter().map(|s| s.width() as u16).sum();
        // A tab is clicked on its padding and on the gap after it, so the
        // bar has no dead cells between tabs; the first takes the gap
        // after `radar` too.
        let lead = if hits.is_empty() { 2 } else { 0 };
        let rect = Rect {
            x: x - lead,
            y: area.y,
            width: w + 1 + lead,
            height: 1,
        }
        .intersection(area);
        if let Some(hit) = hit {
            hits.push((rect, hit));
        }
        x += w + 1;
        spans.extend(parts);
        spans.push(Span::raw(" "));
    };
    for t in &chrome.top_bar.tabs {
        let hit = host.chrome_hit(&t.target);
        // Compact, a screen's tab is its key; history and settings keep
        // their names.
        let screen = matches!(hit, Some(Hit::Tab(View::Screen(_))));
        let name = if compact && screen {
            t.mnemonic.to_string()
        } else {
            t.label.clone()
        };
        tab(
            &mut spans,
            t.mnemonic,
            &name,
            t.icon,
            t.attention,
            t.active,
            hit,
        );
    }
    (spans, hits)
}

/// ` BOOKS   PgUp/PgDn scroll   q quit   x menu          11 open · 3 need you `
fn status_line(host: &Host, theme: &Theme, status: &StatusLine, area: Rect, buf: &mut Buffer) {
    buf.set_style(area, theme.bg(Slot::BgOverlay));
    let mode = &status.scope;
    let chip = theme
        .bold(Slot::BgBase)
        .patch(Style::new().bg(theme.s(Slot::AccentPrimary).fg.unwrap_or_default()));
    let mut counts: Vec<Span<'static>> = Vec::new();
    for (i, count) in status.counts.iter().enumerate() {
        if i > 0 {
            counts.push(Span::styled(" · ", theme.s(Slot::TextMuted)));
        }
        counts.push(Span::styled(count.text.clone(), widgets::tone(theme, count.ink.tone())));
    }
    // The services' chips at the far right, a space apart, two cells after
    // the counts: each its glyph (its name where the pack has none) and the
    // count of failing sources, in its state's ink.
    let mut health: Vec<(Span<'static>, Option<Hit>)> = Vec::new();
    for (i, chip) in status.health.iter().enumerate() {
        if i > 0 {
            health.push((Span::raw(" "), None));
        }
        let span = Span::styled(chip.text(), ink_style(theme, chip.ink, false));
        health.push((span, host.chrome_hit(&chip.target)));
    }
    let health_w = health.iter().map(|(s, _)| s.width()).sum::<usize>() as u16;
    let sep_w = if !counts.is_empty() && health_w > 0 { 2 } else { 0 };
    let counts_w = counts.iter().map(Span::width).sum::<usize>() as u16;
    let (mut show_counts, mut show_health) = (true, health_w > 0);
    let width_of = |counts: bool, health: bool| -> u16 {
        let c = if counts { counts_w } else { 0 };
        let h = if health { health_w } else { 0 };
        c + h + if counts && health { sep_w } else { 0 } + 1
    };
    let mut right_w = if width_of(true, show_health) < area.width / 2 {
        width_of(true, show_health)
    } else if show_health && width_of(false, true) < area.width / 2 {
        show_counts = false;
        width_of(false, true)
    } else {
        show_health = false;
        0
    };
    let mut left = Line::from(vec![
        Span::styled(format!(" {mode} "), chip.add_modifier(Modifier::BOLD)),
        Span::raw("  "),
    ]);
    // An agent session that wants the user, out of sight: its key, then a
    // square chip on its state's ground, after the scope; it never gives way.
    if let Some(a) = &status.attention {
        let x = area.x + left.width() as u16;
        let ground = ink_style(theme, a.ink, false).fg.unwrap_or_default();
        let on = theme.bold(Slot::BgBase).patch(Style::new().bg(ground));
        let glyph = a.icon.map(|i| icon::text(theme, i)).unwrap_or_default();
        left.spans.push(Span::styled(a.keys.clone(), theme.bold(Slot::AccentSecondary)));
        left.spans.push(Span::raw(" "));
        left.spans.push(Span::styled(format!(" {glyph}{} ", a.text), on));
        let w = (area.x + left.width() as u16).saturating_sub(x);
        if let Some(hit) = host.chrome_hit(&a.target) {
            host.hits.borrow_mut().push((Rect { x, width: w, ..area }.intersection(area), hit));
        }
        left.spans.push(Span::raw("  "));
    }

    // The keys come before the counts: when the bar does not fit, the
    // counts give way, then the keys needed least, then the services'
    // chips. `extra actions` always stays.
    let mut keys: Vec<&Hint> = status.hints.iter().collect();
    let keys_w = |keys: &[&Hint]| -> usize {
        keys.iter()
            .map(|h| {
                let lead = if h.icon.is_some() {
                    icon::lead_width(theme)
                } else {
                    0
                };
                widgets::key_hint(theme, &h.keys, &h.label, h.mnemonic_at, h.mnemonic_shift).iter().map(Span::width).sum::<usize>() + 3 + lead
            })
            .sum()
    };
    let fits = |keys: &[&Hint], right_w: u16| {
        left.width() + keys_w(keys) < area.width.saturating_sub(right_w) as usize
    };
    if status.message.is_none() && !fits(&keys, right_w) {
        show_counts = false;
        right_w = if show_health { width_of(false, true) } else { 0 };
        // The keys needed least give way first, in the host's order.
        for least in &status.least_needed {
            if fits(&keys, right_w) {
                break;
            }
            keys.retain(|h| h.label != *least);
        }
        if !fits(&keys, right_w) {
            show_health = false;
            right_w = 0;
        }
        // The original actions-menu input remains reachable even when the
        // remaining action phrases alone overflow. Preserve its chord/target,
        // independent of wording, and let trailing nonmandatory hints give way.
        let actions = |h: &&Hint| h.keys_of.iter().any(|key| key.chord == Chord::char('x'));
        if keys.iter().any(actions) {
            while !fits(&keys, right_w) {
                let Some(drop) = keys.iter().rposition(|h| !actions(h)) else { break };
                keys.remove(drop);
            }
        }
    }

    // Left: the mode, then the message or as many whole hints as fit.
    let room = area.width.saturating_sub(right_w + 1) as usize;
    match &status.message {
        Some(m) => {
            let x = left.width() as u16;
            if let Some(i) = m.icon {
                left.spans.push(icon::lead(theme, i));
            }
            let message = Span::styled(m.text.clone(), ink_style(theme, m.ink, false));
            let width = (message.width() as u16 + left.width() as u16 - x)
                .min((room as u16).saturating_sub(x));
            motion::region(
                Region::Message,
                Rect {
                    x: area.x + x,
                    width,
                    ..area
                },
            );
            left.spans.push(message);
        }
        None => {
            let mut hits = host.hits.borrow_mut();
            for h in keys {
                let mut hint = widgets::key_hint(theme, &h.keys, &h.label, h.mnemonic_at, h.mnemonic_shift);
                if let Some(i) = h.icon {
                    let lead = icon::lead(theme, i);
                    if !lead.content.is_empty() {
                        // A named key/modifier precedes the icon; the mnemonic
                        // letter itself stays inside the word.
                        let prefix = crate::mnemonic::prefix(&h.keys, &h.label, h.mnemonic_at, h.mnemonic_shift);
                        hint.insert(usize::from(!prefix.is_empty() && !crate::mnemonic::valid_at(&h.keys, &h.label, h.mnemonic_at)), lead);
                    }
                }
                hint.push(Span::raw("   "));
                let w: usize = hint.iter().map(Span::width).sum();
                if left.width() + w > room {
                    break;
                }
                let x = area.x + left.width() as u16;
                let line = Rect {
                    x,
                    width: w as u16,
                    ..area
                };
                hits.extend(key_hits(h, line).into_iter().filter_map(|(r, target)| {
                    Some((r.intersection(area), host.chrome_hit(target)?))
                }));
                left.spans.extend(hint);
            }
        }
    }
    buf.set_line(area.x, area.y, &left, room as u16);
    if right_w > 0 {
        let mut right: Vec<Span<'static>> = Vec::new();
        if show_counts {
            right.extend(counts);
            if show_health && sep_w > 0 {
                right.push(Span::raw("  "));
            }
        }
        let mut x = area.x + area.width - right_w + right.iter().map(Span::width).sum::<usize>() as u16;
        if show_health {
            let mut hits = host.hits.borrow_mut();
            for (span, hit) in health {
                let w = span.width() as u16;
                // A chip is clicked on its cells and the one after it.
                if let Some(hit) = hit {
                    hits.push((Rect { x, width: w + 1, ..area }.intersection(area), hit));
                }
                x += w;
                right.push(span);
            }
        }
        right.push(Span::raw(" "));
        buf.set_line(area.x + area.width - right_w, area.y, &Line::from(right), right_w);
    }
}

/// Where a click presses each key of a hint drawn across `line` (the key
/// label, its words and the three cells after them): one key, then the
/// whole hint and a cell before it; several (`PgUp/PgDn`, `+/-`, `←→↑↓`),
/// then each label, and each word when there is a word per key
/// (`expand/collapse`). Each comes with the target the host gave its key.
pub(crate) fn key_hits(h: &Hint, line: Rect) -> Vec<(Rect, &Target)> {
    let cell = |x: u16, w: u16| Rect {
        x,
        width: w,
        ..line
    };
    if h.keys_of.is_empty() {
        return vec![];
    }
    // The labels with where each starts in the hint: `/` stands between
    // labels written with one (`PgUp/PgDn`); arrows sit side by side.
    let gap = u16::from(h.keys_of.len() > 1 && h.keys.contains('/'));
    let mut labels: Vec<(u16, &str)> = Vec::new();
    let mut at = 0;
    for k in &h.keys_of {
        labels.push((at, k.label.as_str()));
        at += ui::width(&k.label) as u16 + gap;
    }
    let keys: Vec<&Target> = h.keys_of.iter().map(|k| &k.target).collect();
    // The words end three cells before the end of the hint (the gap after
    // them), whatever icon comes between them and the keys.
    let words_end = line.x + line.width.saturating_sub(3);
    let span = |from: u16, to: u16| cell(from, to.saturating_sub(from));
    if keys.len() == 1 {
        let from = line.x.saturating_sub(1);
        return vec![(span(from, words_end + 1), keys[0])];
    }
    let mut out = Vec::new();
    let last = keys.len() - 1;
    for (i, ((at, label), k)) in labels.iter().zip(&keys).enumerate() {
        // The first label takes the cell before it, the last the one
        // after; a `/` between two goes with the one before it.
        let from = (line.x + at).saturating_sub(u16::from(i == 0));
        let to = match labels.get(i + 1) {
            Some((next, _)) => line.x + next,
            None => line.x + at + ui::width(label) as u16 + 1,
        };
        out.push((span(from, to), *k));
    }
    let words: Vec<&str> = h.label.split('/').collect();
    if words.len() == keys.len() {
        let mut at = words_end.saturating_sub(ui::width(&h.label) as u16);
        for (i, (w, k)) in words.iter().zip(&keys).enumerate() {
            let w = ui::width(w) as u16;
            out.push((span(at, at + w + u16::from(i == last)), *k));
            at += w + 1;
        }
    }
    out
}

/// Settings' panes in `body`: the sections, the section, its detail (side by
/// side from 120 cells, the detail under the section below that).
fn settings_areas(body: Rect) -> [Rect; 3] {
    if body.width >= 120 {
        Layout::horizontal([
            Constraint::Length(24),
            Constraint::Fill(3),
            Constraint::Fill(2),
        ])
        .areas(body)
    } else {
        let [nav, content] = Layout::horizontal([
            Constraint::Length(24.min(body.width / 3)),
            Constraint::Fill(1),
        ])
        .areas(body);
        let [middle, detail] =
            Layout::vertical([Constraint::Fill(1), Constraint::Fill(1)]).areas(content);
        [nav, middle, detail]
    }
}

/// The body a frame `width` × `height` cells big leaves the screen or
/// settings: under the top bar (and a source row) and over the status line.
pub(crate) fn body_of(host: &Host, width: u16, height: u16) -> Rect {
    let area = Rect::new(0, 0, width, height);
    let [_, body, _] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(area);
    let sources = matches!(host.view, View::Screen(i) if host.group_screens(i).len() > 1);
    if sources && body.height > 1 {
        Rect { y: body.y + 1, height: body.height - 1, ..body }
    } else {
        body
    }
}

/// What a frame `width` × `height` cells big measures, done before anything
/// is drawn so the chrome the host builds is the same for every frontend:
/// how far settings' detail can scroll, and the open menu's lines, rows
/// shown, usable controls and reading position. [`Host::chrome_view`] runs
/// it; painting afterwards finds the same numbers.
pub(crate) fn measure(host: &Host, theme: &Theme, width: u16, height: u16) {
    if host.history.is_some() {
        return;
    }
    let body = body_of(host, width, height);
    if host.setup.is_some() {
        setup::measure(host, theme, body);
    } else if host.screen().is_none() {
        measure_settings_detail(host, theme, body);
    }
    if let Some(open) = &host.menu {
        menu::measure(&host.menu_view().expect("menu is open"), &open.viewport, theme, body);
    }
}

/// How far settings' detail can scroll at this size (`Host::settings_detail`).
fn measure_settings_detail(host: &Host, theme: &Theme, body: Rect) {
    let [_, middle, detail] = settings_areas(body);
    let det = widgets::panel(theme, "", false).inner(detail);
    if host.capturing() {
        host.settings_detail.top();
    }
    let section_view = match host.current_section() {
        Some((Owner::Plugin(p), s)) => {
            host.section_view_in(p, s.id, middle.width.saturating_sub(2), middle.height.saturating_sub(2))
        }
        _ => None,
    };
    let lines = match host.current_section() {
        Some((Owner::Plugin(_), _)) => section_view
            .as_ref()
            .map(|view| section_detail_lines(theme, view, det.width)),
        Some((Owner::Mode, _)) => Some(crate::mode::detail_lines(host, theme)),
        Some((Owner::Look, _)) => Some(section_detail_lines(theme, &host.look_settings_view(), det.width)),
        Some((Owner::Parts, _)) => Some(crate::parts::detail_lines(host, theme)),
        Some((Owner::Host, _)) if host.on_icons() => Some(host.colours.icons_detail_lines(theme)),
        Some((Owner::Host, _)) => Some(host.colours.detail_lines(theme)),
        None => return,
    };
    let text = widgets::padded(det);
    match lines {
        Some(lines) if !text.is_empty() => {
            let total = ratatui::widgets::Paragraph::new(lines)
                .wrap(ratatui::widgets::Wrap { trim: false })
                .line_count(text.width)
                .min(u16::MAX as usize) as u16;
            host.settings_detail.fit(text.height, total);
        }
        _ => {
            host.settings_detail.fit(0, 0);
        }
    }
}

/// A part's section detail as lines at the detail's inner width. The blocks
/// fill one cell past the width they are given (a section's rule, a line's
/// `[key]`), so they are given one less than the padded pane, which they
/// then fill exactly instead of wrapping that cell onto a line of its own.
fn section_detail_lines(theme: &Theme, view: &crate::view::SectionView, det_width: u16) -> Vec<Line<'static>> {
    if view.detail_blocks.is_empty() {
        view.detail.iter().map(|l| crate::view_tui::line(theme, l)).collect()
    } else {
        crate::view_tui::rich_lines_with_urls(theme, &view.detail_blocks, det_width.saturating_sub(3), false, false).0
    }
}

/// Settings: the sections as a tree grouped by plugin │ the section │ about
/// the selected row.
fn settings(
    host: &Host,
    theme: &Theme,
    chrome: &ChromeView,
    body: Rect,
    buf: &mut Buffer,
    cursor: &std::cell::Cell<Option<ratatui::layout::Position>>,
) {
    let [nav, middle, detail] = settings_areas(body);

    // Sections tree: each group a heading with its sections under it, as the
    // parts' own trees are drawn; a group folds.
    // Each row is known by its target (the tree's identifiers are never
    // drawn).
    let empty = crate::view::chrome::SettingsNav::default();
    let snav = chrome.settings.as_ref().unwrap_or(&empty);
    let nodes: Vec<crate::ui::Node> = snav
        .groups
        .iter()
        .map(|g| {
            crate::ui::Node::new(
                g.target.0.clone(),
                vec![Span::styled(g.name.clone(), theme.bold(Slot::TextEmphasis))],
            )
            .children(g.sections.iter().map(|s| {
                let mut name = Vec::new();
                if let Some(i) = s.icon {
                    name.push(icon::lead(theme, i));
                }
                name.push(Span::styled(s.name.clone(), theme.s(Slot::TextPrimary)));
                crate::ui::Node::new(s.target.0.clone(), name)
            }))
        })
        .collect();
    let mut state = TreeState::default();
    let mut rows: Vec<&Target> = Vec::new();
    for g in &snav.groups {
        rows.push(&g.target);
        if g.open {
            state.open(vec![g.target.0.clone()]);
            rows.extend(g.sections.iter().map(|s| &s.target));
        }
        if snav.selected.as_ref() == Some(&g.target) {
            state.select(vec![g.target.0.clone()]);
        } else if let Some(s) = g.sections.iter().find(|s| snav.selected.as_ref() == Some(&s.target)) {
            state.select(vec![g.target.0.clone(), s.target.0.clone()]);
        }
    }
    // A pane draws as focused unless the top bar has the keys.
    let has = |pane: Pane| chrome.keys_at != KeysAt::Bar && host.focus == pane;
    let nav_focused = has(Pane::Nav);
    let dock_takes_keys = host.current_section().is_some()
        && host.dock_view().is_some_and(|d| d.visible && d.focused);
    let ramp_allowed = pane_ramp_allowed(chrome, dock_takes_keys);
    let nav_title = crate::view::chrome::pane_title(&chrome.pane_jumps, crate::view::PaneId::Nav, "settings");
    let mut block = widgets::panel(theme, &nav_title, nav_focused);
    if snav.unsaved {
        block = block.title_bottom(Line::styled(
            " unsaved · w retries ",
            theme.s(Slot::StatusWarning),
        ));
    }
    let inner = block.inner(nav);
    block.render(nav, buf);
    let mut keep = vec![ui::frame::title_region(
        nav, ui::width(&nav_title) + 2, ui::frame::TitleEdge::Top,
    )];
    if snav.unsaved {
        keep.push(ui::frame::title_region(
            nav, ui::width(" unsaved · w retries "), ui::frame::TitleEdge::Bottom,
        ));
    }
    motion::frames::with_frame(crate::view::PaneId::Nav, "settings_nav", || {
        ui::frame::focus_border(theme, nav, buf, nav_focused && ramp_allowed, &keep);
    });
    let columns = crate::ui::Columns::default();
    let rows_area = crate::ui::BandedTree::new(&nodes, &columns, crate::ui::TreeLook::new(theme))
        .focused(nav_focused)
        .render(theme, inner, buf, &mut state);
    if motion::frames::enabled() {
        let visible: Vec<(Vec<String>, u16)> = (rows_area.y..rows_area.bottom()).filter_map(|y| {
            state.rendered_at(ratatui::layout::Position::new(rows_area.x, y)).map(|path| (path.to_vec(), y))
        }).collect();
        let open = snav.groups.iter().filter(|group| group.open && visible.iter().any(|(path, _)| path == &vec![group.target.0.clone()])).map(|group| vec![group.target.0.clone()]).collect();
        let viewport = Rect { width: inner.width.saturating_sub(1), ..inner };
        motion::frames::tree(crate::view::PaneId::Nav, "settings_nav", viewport, visible, open);
    }
    // Clickable: the panes, then each row the tree drew.
    {
        let mut hits = host.hits.borrow_mut();
        hits.push((nav, Hit::Pane(Pane::Nav)));
        hits.push((middle, Hit::Pane(Pane::Middle)));
        hits.push((detail, Hit::Pane(Pane::Detail)));
        for (k, target) in rows
            .into_iter()
            .skip(state.get_offset())
            .take(rows_area.height as usize)
            .enumerate()
        {
            let rect = Rect {
                y: rows_area.y + k as u16,
                height: 1,
                ..rows_area
            };
            if let Some(hit) = host.chrome_hit(target) {
                hits.push((rect, hit));
            }
        }
    }

    // The section and its detail.
    let plugin_hits = std::cell::RefCell::new(Vec::new());
    let cx = |pane: Pane, p: usize| Cx {
        theme,
        now: host.now,
        focused: has(pane),
        focus: host.focus,
        bar: chrome.keys_at == KeysAt::Bar,
        tabs: false,
        cursor,
        hits: &plugin_hits,
        parts: Parts::new(&host.plugins, Some(p)),
    };
    host.frontend.set_lead(crate::ui::icon::lead_width(theme));
    let section_view = match host.current_section() {
        Some((Owner::Plugin(p), s)) => {
            host.section_view_in(p, s.id, middle.width.saturating_sub(2), middle.height.saturating_sub(2))
        }
        _ => None,
    };
    // A part's section names its own detail (view data); the host's are
    // in the chrome.
    let name = crate::view::chrome::pane_title(&chrome.pane_jumps, crate::view::PaneId::Middle, &snav.section_title);
    let about = section_view
        .as_ref()
        .map_or_else(|| snav.detail_title.clone(), |v| v.detail_title.clone());
    let about = crate::view::chrome::pane_title(&chrome.pane_jumps, crate::view::PaneId::Detail, &about);
    let mid = motion::frames::with_frame(crate::view::PaneId::Middle, "body", || ui::card::panel_owned(
        theme, middle, buf, &name,
        ui::frame::Focus { focused: has(Pane::Middle), owns_keys: has(Pane::Middle) && ramp_allowed },
    ));
    let det = motion::frames::with_frame(crate::view::PaneId::Detail, "body", || ui::card::panel_owned(
        theme, detail, buf, &about,
        ui::frame::Focus { focused: has(Pane::Detail), owns_keys: has(Pane::Detail) && ramp_allowed },
    ));
    // Rows to choose in the navigation and the section, text in the detail.
    // The nav's rows, not its scrollbar column (the tree's rows area is as
    // wide as the rows it drew).
    host.frontend.mark_selectable(Rect { width: rows_area.width, ..inner }, false);
    host.frontend.mark_selectable(mid, false);
    host.frontend.mark_selectable(det, true);
    // Capture owns every key, so the live verdict at the start stays visible.
    if host.capturing() {
        host.settings_detail.top();
    }
    let draw_detail = |lines, buf: &mut Buffer| {
        crate::ui::scroll::paragraph(
            theme,
            &host.settings_detail,
            detail,
            widgets::padded(det),
            buf,
            lines,
        );
    };
    match host.current_section() {
        Some((Owner::Plugin(p), s)) if section_view.is_some() => {
            let view = section_view.as_ref().expect("checked");
            crate::view_tui::render_section(&host.frontend, view, mid, buf, &cx(Pane::Middle, p));
            draw_detail(section_detail_lines(theme, view, det.width), buf);
            adopt_hits(host, p, s.id, plugin_hits);
        }
        Some((Owner::Plugin(_), _)) => {
            host.settings_detail.fit(0, 0);
        }
        Some((Owner::Look, _)) => {
            let view = host.look_settings_view();
            let look_hits = std::cell::RefCell::new(Vec::new());
            let look_cx = Cx { hits: &look_hits, parts: Parts::new(&host.plugins, None), ..cx(Pane::Middle, 0) };
            crate::view_tui::render_section(&host.frontend, &view, mid, buf, &look_cx);
            draw_detail(section_detail_lines(theme, &view, det.width), buf);
            let mut hits = host.hits.borrow_mut();
            for (rect, _, id) in look_hits.into_inner() {
                if id != crate::view_tui::PANE_HIT {
                    hits.push((rect, Hit::Setup(id)));
                }
            }
        }
        Some((Owner::Mode, _)) => {
            if let Some(r) = crate::mode::render(host, theme, mid, buf, has(Pane::Middle)) {
                host.hits.borrow_mut().push((r, Hit::Mode));
                if mid.height > 6 {
                    let pause = Rect { y: mid.y + 7, ..r };
                    host.hits.borrow_mut().push((pause, Hit::PauseIdle));
                }
            }
            draw_detail(crate::mode::detail_lines(host, theme), buf);
        }
        Some((Owner::Parts, _)) => {
            let rows = crate::parts::render(host, theme, mid, buf, has(Pane::Middle));
            host.hits
                .borrow_mut()
                .extend(rows.into_iter().map(|(r, i)| (r, Hit::PartsRow(i))));
            draw_detail(crate::parts::detail_lines(host, theme), buf);
        }
        Some((Owner::Host, _)) if host.on_icons() => {
            let rows = host
                .colours
                .icons_render(theme, mid, buf, has(Pane::Middle), cursor);
            host.hits
                .borrow_mut()
                .extend(rows.into_iter().map(|(r, i)| (r, Hit::ColourRow(i))));
            draw_detail(host.colours.icons_detail_lines(theme), buf);
        }
        Some((Owner::Host, _)) => {
            let rows = host
                .colours
                .render(theme, mid, buf, has(Pane::Middle), cursor);
            host.hits
                .borrow_mut()
                .extend(rows.into_iter().map(|(r, i)| (r, Hit::ColourRow(i))));
            draw_detail(host.colours.detail_lines(theme), buf);
        }
        None => widgets::empty(buf, mid, theme, "No settings."),
    }
}

/// Draw the host once into a buffer of this size, in a theme at `tier`.
fn paint(host: &Host, theme: &Theme, width: u16, height: u16) -> Buffer {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    let mut term = Terminal::new(TestBackend::new(width, height)).expect("test backend");
    term.draw(|f| { render(host, theme, f); render_notices(host, theme, f.buffer_mut()); }).expect("draw");
    term.backend().buffer().clone()
}

/// Render the host as an SVG: every cell at its colour, bold, italic,
/// underline, reversed and dim honoured, wide characters on two cells, in
/// full colour (unless the host's config asks for fewer). The same screen
/// gives the same text, so two of them diff. See [`crate::visual`].
pub fn to_svg(host: &Host, width: u16, height: u16) -> String {
    let theme = host.build_theme(crate::design::theme::Tier::TrueColor);
    let buf = paint(host, &theme, width, height);
    crate::visual::buffer_to_svg(&buf, &crate::visual::Look::of(&theme))
}

/// Render the host as each line's runs and their styles (colours by slot
/// name), for snapshots that must notice a colour change.
/// See [`crate::visual::style_dump`].
pub fn to_styles(host: &Host, width: u16, height: u16) -> String {
    let theme = host.build_theme(crate::design::theme::Tier::TrueColor);
    crate::visual::style_dump(&paint(host, &theme, width, height), &theme)
}

/// Render the host into a plain-text screen, for snapshot tests. Monochrome.
pub fn to_text(host: &Host, width: u16, height: u16) -> String {
    text_in(
        host,
        &host.build_theme(crate::design::theme::Tier::Mono),
        width,
        height,
    )
}

/// [`to_text`] in a theme already built (building one reads the icon
/// packs: a test drawing thousands of frames builds it once).
pub fn text_in(host: &Host, theme: &Theme, width: u16, height: u16) -> String {
    let buf = &paint(host, theme, width, height);
    lines_of(buf, height)
}

/// [`text_in`], each cell's modifiers (where a cursor row shows without
/// colour), and where the terminal's cursor is while text is typed.
pub fn text_and_cursor(
    host: &Host,
    theme: &Theme,
    width: u16,
    height: u16,
) -> (String, Vec<u16>, Option<ratatui::layout::Position>) {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    let mut term = Terminal::new(TestBackend::new(width, height)).expect("test backend");
    term.draw(|f| { render(host, theme, f); render_notices(host, theme, f.buffer_mut()); }).expect("draw");
    let b = term.backend();
    let cursor = b.cursor_visible().then(|| b.cursor_position());
    let looks = b
        .buffer()
        .content
        .iter()
        .map(|c| c.modifier.bits())
        .collect();
    (lines_of(b.buffer(), height), looks, cursor)
}

fn lines_of(buf: &Buffer, height: u16) -> String {
    (0..height)
        .map(|y| row_text(buf, y))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Row `y` of `buf` as a terminal shows it: a wide character (an emoji, CJK)
/// once, its second cell skipped, so a snapshot's columns read as on screen.
/// Trailing spaces trimmed.
pub fn row_text(buf: &Buffer, y: u16) -> String {
    let a = buf.area;
    let mut out = String::new();
    let mut x = a.x;
    while x < a.x + a.width {
        let s = buf[(x, y)].symbol();
        out.push_str(s);
        x += ui::width(s).max(1) as u16;
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::tests::probe;

    mod mnemonic_tests {
        use super::*;
        use ratatui::layout::Position;

        #[test]
        fn mandatory_actions_keep_their_word_and_click_target_when_other_hints_overflow() {
            let host = two();
            let theme = host.build_theme(crate::design::theme::Tier::TrueColor);
            let mut status = host.chrome_status();
            let actions = status.hints.iter().find(|h| h.keys_of.iter().any(|k| k.chord == Chord::char('x'))).unwrap().clone();
            status.hints = [("⏎", "use it"), ("r", "retry status"), ("A", "ask an agent"), ("b", "bulk")]
                .iter().enumerate().map(|(n, (key, words))| crate::host::chrome_hint(n, &KeyHint::new(key, words))).collect();
            status.hints.push(actions);
            status.hints.push(crate::host::chrome_hint(5, &KeyHint::new("~", "notices")));
            status.least_needed.clear();
            status.counts.clear();
            status.health.clear();
            for width in [80, 40] {
                host.hits.borrow_mut().clear();
                let area = Rect::new(0, 0, width, 1);
                let mut buf = Buffer::empty(area);
                status_line(&host, &theme, &status, area, &mut buf);
                let shown = row_text(&buf, 0);
                let at = shown.find("extra actions").unwrap_or_else(|| panic!("mandatory action missing: {shown:?}"));
                let start = ui::width(&shown[..at]) as u16;
                assert!(buf[(start + 1, 0)].modifier.contains(Modifier::UNDERLINED));
                for x in start..start + 13 {
                    assert!(host.hits.borrow().iter().any(|(rect, hit)| rect.contains(Position::new(x, 0))
                        && matches!(hit, Hit::Key(key, _) if key.key == crate::view::Key::Char('x') && key.bare())),
                        "the whole action word keeps its original input at width {width}, cell {x}");
                }
            }
        }
    }

    mod tab_attention {
        use super::*;
        use crate::design::theme::Tier;

        #[test]
        fn attention_dot_keeps_mnemonic_style_and_click_target() {
            let mut host = two();
            let mut chrome = host.chrome_view(80, 24);
            chrome.top_bar.brand = "界".into();
            chrome.top_bar.tabs[1].label = "日本Beta".into();
            chrome.top_bar.tabs[1].attention = true;
            for tab in &mut chrome.top_bar.tabs { tab.icon = None; }
            chrome.top_bar.words = None;
            let mut wide = None;
            for pack in ["off", "unicode", "emoji"] {
                host.colours.cfg.icons = pack.into();
                let theme = host.build_theme(Tier::TrueColor);
                for width in [80, 35] {
                    let area = Rect::new(0, 0, width, 1);
                    let mut buf = Buffer::empty(area);
                    host.hits.borrow_mut().clear();
                    top_bar(&host, &theme, &chrome, area, &mut buf);
                    let text = row_text(&buf, 0);
                    if width == 80 {
                        let dot = text.find(icon::ATTENTION_DOT).unwrap();
                        let x = ui::width(&text[..dot]) as u16;
                        assert_eq!(text.matches(icon::ATTENTION_DOT).count(), 1);
                        assert_eq!(Some(buf[(x, 0)].fg), theme.s(Slot::StatusWarning).fg);
                        assert_eq!(host.hit_at(x, 0), Some(Hit::Tab(View::Screen(1))));
                        let key = ui::width(&text[..text.find('B').unwrap()]) as u16;
                        assert!(buf[(key, 0)].modifier.contains(Modifier::UNDERLINED));
                        // The mode chip at the right is the pack's glyph: compare the tabs.
                        let tabs = text[..text.find("(,)").unwrap()].to_string();
                        if let Some(before) = &wide { assert_eq!(&tabs, before); }
                        wide = Some(tabs);
                    }
                    let mode_w = ui::width(&format!(" {} ", mode_words(&chrome.top_bar.mode))) as u16;
                    if mode_w < width {
                        for x in width - mode_w..width {
                            assert!(!matches!(host.hit_at(x, 0), Some(Hit::Tab(_) | Hit::HistoryTab)),
                                "covered tab target at {x}: {text}");
                        }
                    }
                    assert!(host.hits.borrow().iter().all(|(rect, _)| rect.right() <= width));
                }
            }
        }
    }

    #[test]
    fn the_top_bar_has_history_before_settings() {
        let host = Host::new(
            vec![probe("alpha", 'A', "a").0, probe("beta", 'B', "b").0],
            0,
        )
        .unwrap();
        let top = to_text(&host, 80, 24).lines().next().unwrap().to_string();
        for want in [" radar ", " Alpha ", " Beta ", " History ", " Settings "] {
            assert!(top.contains(want), "{want:?} missing from {top:?}");
        }
        assert!(top.find("History").unwrap() < top.find("Settings").unwrap());
        assert!(
            !top.contains("A alpha"),
            "the key is not repeated in front: {top:?}"
        );
    }

    #[test]
    fn the_shortcut_letter_is_underlined_inside_the_name() {
        let spans = mnemonic(
            "repositories",
            'R',
            Style::new(),
            Style::new().add_modifier(Modifier::UNDERLINED),
        );
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "Repositories");
        let marked: Vec<&str> = spans
            .iter()
            .filter(|s| s.style.add_modifier.contains(Modifier::UNDERLINED))
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(marked, ["R"]);
    }

    #[test]
    fn clicking_a_tab_switches_and_a_double_click_on_a_plugin_row_is_enter() {
        use crate::host::{Event, MouseKind};
        let mut host = Host::new(
            vec![probe("alpha", 'A', "a").0, probe("beta", 'B', "b").0],
            0,
        )
        .unwrap();
        let text = to_text(&host, 80, 24);
        let x = text.lines().next().unwrap().find("Beta").unwrap() as u16;
        host.update(Event::Mouse {
            kind: MouseKind::Press,
            column: x,
            row: 0,
            at_ms: 0,
        });
        assert_eq!(host.view, View::Screen(1));
        let _ = to_text(&host, 80, 24);
        let h = text.lines().next().unwrap().find("History").unwrap() as u16;
        let effects = host.update(Event::Mouse {
            kind: MouseKind::Press,
            column: h,
            row: 0,
            at_ms: 500,
        });
        assert!(matches!(
            effects.as_slice(),
            [crate::host::HostEffect::History(_)]
        ));
        assert!(host.history.is_some());
        let history_top = to_text(&host, 80, 24).lines().next().unwrap().to_string();
        assert!(history_top.contains("History"));
        assert!(history_top.contains("Settings"));
        let theme = host.build_theme(crate::design::theme::Tier::TrueColor);
        let buf = paint(&host, &theme, 80, 24);
        let history_x = history_top.find("History").unwrap() as u16;
        let beta_x = history_top.find("Beta").unwrap() as u16;
        assert_ne!(buf[(history_x, 0)].bg, buf[(beta_x, 0)].bg);
        let s = text.lines().next().unwrap().find("Settings").unwrap() as u16;
        host.update(Event::Mouse {
            kind: MouseKind::Press,
            column: s,
            row: 0,
            at_ms: 1000,
        });
        assert!(host.history.is_none());
        assert_eq!(host.view, View::Settings);
    }

    #[test]
    fn modal_hints_ignore_the_settings_detail_scroll_behind_them() {
        let mut host = Host::new(vec![], 0).unwrap();
        host.view = View::Settings;
        host.menu = Some(crate::host::OpenMenu::open(
            Owner::Host,
            crate::plugin::Menu::write_confirm(
                "hint-scope",
                "Read these notes?",
                &"note\n".repeat(30),
                "post it",
                "drop it",
            ),
        ));
        let _ = to_text(&host, 34, 10);
        let theme = host.build_theme(crate::design::theme::Tier::TrueColor);
        let area = Rect::new(0, 0, 34, 1);
        let status = |host: &Host| {
            let mut buf = Buffer::empty(area);
            status_line(host, &theme, &host.chrome_status(), area, &mut buf);
            buf
        };
        host.settings_detail.fit(10, 10);
        let fitting = status(&host);
        host.settings_detail.fit(10, 50);
        assert_eq!(
            status(&host),
            fitting,
            "modal hints belong only to the modal"
        );
    }

    fn two() -> Host {
        Host::new(
            vec![probe("alpha", 'A', "a").0, probe("beta", 'B', "b").0],
            0,
        )
        .unwrap()
    }

    fn at_text(text: &str, line: usize, needle: &str) -> ratatui::layout::Position {
        let l = text.lines().nth(line).unwrap();
        let x = l[..l
            .find(needle)
            .unwrap_or_else(|| panic!("{needle:?} in {l:?}"))]
            .chars()
            .count();
        ratatui::layout::Position::new(x as u16, line as u16)
    }

    #[test]
    fn the_tab_under_the_pointer_is_lit_with_the_hover_colour_and_the_pointer_is_a_hand() {
        let mut host = two();
        let theme = host.build_theme(crate::design::theme::Tier::TrueColor);
        let text = to_text(&host, 80, 24);
        let beta = at_text(&text, 0, "Beta");
        assert!(host.hover(Some(beta)), "something new is lit");
        assert!(
            !host.hover(Some(ratatui::layout::Position {
                x: beta.x + 1,
                ..beta
            })),
            "the same tab: nothing to draw"
        );
        let buf = paint(&host, &theme, 80, 24);
        let hover = theme.bg(Slot::BgHover).bg;
        assert_eq!(Some(buf[(beta.x, 0)].bg), hover);
        assert_eq!(Some(buf[(beta.x - 1, 0)].bg), hover, "its padding too");
        let alpha = at_text(&text, 0, "Alpha");
        assert_ne!(
            Some(buf[(alpha.x, 0)].bg),
            hover,
            "the screen showing keeps its own look"
        );
        assert_ne!(Some(buf[(beta.x, 5)].bg), hover, "only the tab");
        assert_eq!(host.pointer_shape(), crate::ui::pointer::Shape::Hand);
        // The body's empty space: a click there only focuses, so nothing
        // is lit and the pointer is the terminal's.
        assert!(host.hover(Some(ratatui::layout::Position::new(40, 12))));
        assert_eq!(*host.hovered.borrow(), None);
        assert_eq!(host.pointer_shape(), crate::ui::pointer::Shape::Default);
        let buf = paint(&host, &theme, 80, 24);
        assert!(
            (0..80).all(|x| Some(buf[(x, 0)].bg) != hover),
            "nothing lit"
        );
        // Gone from the window: nothing lit.
        host.hover(Some(beta));
        assert!(host.hover(None));
        assert_eq!(host.pointer_shape(), crate::ui::pointer::Shape::Default);
    }

    #[test]
    fn the_top_bar_has_no_dead_cells_between_its_tabs() {
        let host = two();
        let text = to_text(&host, 80, 24);
        let end = at_text(&text, 0, "Settings").x + "Settings".len() as u16;
        let first = at_text(&text, 0, "Alpha").x;
        for x in first - 2..=end {
            assert!(
                matches!(host.hit_at(x, 0), Some(Hit::Tab(_) | Hit::HistoryTab)),
                "x {x} of {:?}",
                text.lines().next()
            );
        }
    }

    #[test]
    fn a_key_on_the_status_line_is_clicked_to_press_it() {
        use crate::host::{Event, MouseKind};
        let mut host = two();
        let text = to_text(&host, 120, 24);
        let menu = at_text(&text, 23, "extra actions");
        host.update(Event::Mouse {
            kind: MouseKind::Press,
            column: menu.x,
            row: 23,
            at_ms: 0,
        });
        // Even when the probe has no actions, x exposes host history.
        assert!(
            host.menu.as_ref().is_some_and(|open| open
                .menu
                .items
                .iter()
                .any(|item| item.label == "all activity history")),
            "x was pressed and opened the host menu"
        );
        host.message = None;
        host.menu = Some(crate::host::OpenMenu {
            owner: crate::host::Owner::Host,
            menu: crate::plugin::Menu::new(
                "m",
                "pick",
                vec![crate::plugin::MenuItem::pick("one", "", "1")],
            ),
            row: 0,
            input: None,
            refused: None,
            viewport: Default::default(),
            anchor: None,
        });
        let text = to_text(&host, 120, 24);
        let close = at_text(&text, 23, "close");
        host.update(Event::Mouse {
            kind: MouseKind::Press,
            column: close.x,
            row: 23,
            at_ms: 1000,
        });
        assert!(host.menu.is_none(), "esc close, clicked, closes it");
        let text = to_text(&host, 120, 24);
        let quit = at_text(&text, 23, "quit");
        host.update(Event::Mouse {
            kind: MouseKind::Press,
            column: quit.x,
            row: 23,
            at_ms: 2000,
        });
        assert!(host.quit);
    }

    #[test]
    fn several_keys_in_one_hint_are_each_their_own_target() {
        use crate::view::{Chord, Key};
        let h = crate::host::chrome_hint(0, &KeyHint::new("+/-", "expand/collapse"));
        let line = Rect::new(10, 23, 3 + 1 + 15 + 3, 1);
        let hits = key_hits(&h, line);
        let chord_of = |t: &Target| h.keys_of.iter().find(|k| k.target == *t).map(|k| k.chord);
        let key = |c| Chord::char(c);
        let at = |x: u16| {
            hits.iter()
                .find(|(r, _)| r.contains(ratatui::layout::Position::new(x, 23)))
                .and_then(|(_, t)| chord_of(t))
        };
        assert_eq!(at(10), Some(key('+')));
        assert_eq!(
            at(11),
            Some(key('+')),
            "the slash goes with the key before it"
        );
        assert_eq!(at(12), Some(key('-')));
        assert_eq!(at(14), Some(key('+')), "expand");
        assert_eq!(at(21), Some(key('-')), "collapse");
        let x = crate::host::chrome_hint(0, &KeyHint::new("x", "menu"));
        let one = key_hits(&x, Rect::new(10, 23, 9, 1));
        assert_eq!(one.len(), 1);
        assert_eq!(
            one[0].0,
            Rect::new(9, 23, 8, 1),
            "the key, its words and a cell either side"
        );
        let pg = crate::host::chrome_hint(0, &KeyHint::new("PgUp/PgDn", "scroll"));
        let chords: Vec<_> = key_hits(&pg, Rect::new(0, 23, 19, 1))
            .iter()
            .filter_map(|(_, t)| pg.keys_of.iter().find(|k| k.target == **t).map(|k| k.chord.key))
            .collect();
        assert_eq!(chords, [Key::PageUp, Key::PageDown]);
    }

    #[test]
    fn a_question_is_a_dialog_with_its_choices_as_buttons_in_a_row() {
        use crate::host::{Event, MouseKind, OpenMenu, Owner};
        let (plugin, log) = probe("alpha", 'A', "a");
        let mut host = Host::new(vec![plugin], 0).unwrap();
        let ask = || {
            crate::plugin::Menu::write_confirm(
                "note",
                "post this on #12?",
                "hello",
                "post it",
                "drop it",
            )
        };
        host.menu = Some(OpenMenu::open(Owner::Plugin(0), ask()));
        let text = to_text(&host, 80, 24);
        let lines: Vec<&str> = text.lines().collect();
        let at = lines
            .iter()
            .position(|l| l.contains("│ hello"))
            .expect("what it sends");
        // What it sends, a blank line, then raised buttons against the
        // right: a light edge, the face, a dark edge.
        let blank = lines[at + 1];
        let pipes: Vec<_> = blank.match_indices('│').map(|(at, _)| at).collect();
        let inside = &blank[pipes[1] + '│'.len_utf8()..pipes[2]];
        assert!(inside.trim().is_empty(), "{text}");
        let edges = |c: &str| format!("{}  {}  {}│", c.repeat(14), c.repeat(14), c.repeat(13));
        assert!(lines[at + 2].contains(&edges("▔")), "{text}");
        assert!(
            lines[at + 3].contains("yes, post it    keep writing    no, drop it │"),
            "{text}"
        );
        assert!(lines[at + 4].contains(&edges("▁")), "{text}");
        assert!(
            lines[23].contains("←→ move   ⏎ choose   esc close"),
            "{text}"
        );
        // The pointer over a button (its edge too) highlights it; a click
        // chooses it.
        let edit = at_text(&text, at + 3, "keep writing");
        assert!(host.hover(Some(ratatui::layout::Position {
            y: edit.y + 1,
            ..edit
        })));
        assert_eq!(host.menu.as_ref().unwrap().row, 2);
        let no = at_text(&text, at + 3, "no, drop it");
        host.update(Event::Mouse {
            kind: MouseKind::Press,
            column: no.x + 3,
            row: no.y,
            at_ms: 0,
        });
        assert_eq!(log.lock().unwrap().bus, vec!["menu note no"]);
        // A narrow terminal: a button that does not fit starts a new row.
        host.menu = Some(OpenMenu::open(Owner::Plugin(0), ask()));
        let text = to_text(&host, 34, 24);
        let lines: Vec<&str> = text.lines().collect();
        let at = lines
            .iter()
            .position(|l| l.contains("yes, post it    keep writing"))
            .expect("two fit");
        assert!(lines[at + 3].contains("no, drop it │"), "{text}");
        let no = at_text(&text, at + 3, "no, drop it");
        host.update(Event::Mouse {
            kind: MouseKind::Press,
            column: no.x,
            row: no.y,
            at_ms: 0,
        });
        assert_eq!(
            log.lock().unwrap().bus,
            vec!["menu note no", "menu note no"]
        );
    }

    #[test]
    fn the_pointer_over_a_menu_item_moves_the_menus_highlight() {
        use crate::host::{OpenMenu, Owner};
        use crate::plugin::{Menu, MenuItem};
        let mut host = two();
        host.menu = Some(OpenMenu {
            owner: Owner::Host,
            menu: Menu::new(
                "m",
                "pick",
                vec![
                    MenuItem::heading("which"),
                    MenuItem::pick("one", "", "1"),
                    MenuItem::pick("two", "", "2"),
                ],
            ),
            row: 1,
            input: None,
            refused: None,
            viewport: Default::default(),
            anchor: None,
        });
        let text = to_text(&host, 80, 24);
        let two = text.lines().position(|l| l.contains(" two ")).unwrap();
        let x = at_text(&text, two, "two");
        assert!(
            host.hover(Some(x)),
            "{x:?} {:?}\n{text}",
            host.hits
                .borrow()
                .iter()
                .filter(|(r, _)| r.y >= 10)
                .collect::<Vec<_>>()
        );
        assert_eq!(host.menu.as_ref().unwrap().row, 2);
        let heading = text
            .lines()
            .position(|l| l.contains("WHICH") || l.contains("which"))
            .unwrap();
        host.hover(Some(at_text(
            &text,
            heading,
            if text.contains("WHICH") {
                "WHICH"
            } else {
                "which"
            },
        )));
        assert_eq!(
            host.menu.as_ref().unwrap().row,
            2,
            "a heading is not a choice"
        );
        assert_eq!(host.pointer_shape(), crate::ui::pointer::Shape::Default);
    }

    #[test]
    fn a_click_on_the_screen_showing_gives_its_tab_the_keys() {
        use crate::host::{Event, MouseKind};
        let mut host = two();
        let text = to_text(&host, 80, 24);
        let alpha = at_text(&text, 0, "Alpha");
        host.update(Event::Mouse {
            kind: MouseKind::Press,
            column: alpha.x,
            row: 0,
            at_ms: 0,
        });
        assert!(host.bar && host.view == View::Screen(0));
    }

    #[test]
    fn a_frame_marks_the_body_the_message_and_the_menu_for_motion() {
        use crate::host::Event;
        use ratatui::{Terminal, backend::TestBackend};
        let mut host = Host::new(vec![probe("alpha", 'A', "a").0], 0).unwrap();
        let _ = to_text(&host, 80, 24);
        let marks = motion::take();
        assert_eq!(marks.body, Some(Rect::new(0, 1, 80, 22)));
        assert_eq!((marks.popup, marks.message), (None, None));
        host.update(Event::Notice("copied 3 characters".into()));
        let theme = host.build_theme(Tier::TrueColor);
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut clean = String::new();
        term.draw(|f| {
            render(&host, &theme, f);
            clean = lines_of(f.buffer_mut(), 24);
            render_notices(&host, &theme, f.buffer_mut());
        })
        .unwrap();
        assert!(!clean.contains("copied 3 characters"));
        assert!(row_text(term.backend().buffer(), 21).contains("copied 3 characters"));
        assert!(!row_text(term.backend().buffer(), 23).contains("copied 3 characters"));
        let marks = motion::take();
        assert_eq!(marks.body, Some(Rect::new(0, 1, 80, 22)));
        assert!(
            marks.message.is_none(),
            "transient notices never enter retained motion marks"
        );
        host.update(Event::Key(ratatui::crossterm::event::KeyEvent::from(
            ratatui::crossterm::event::KeyCode::Char('m'),
        )));
        host.update(Event::Notice("copied 3 characters".into()));
        let text = to_text(&host, 80, 24);
        let marks = motion::take();
        assert!(marks.message.is_none());
        assert!(
            !text.contains("copied 3 characters"),
            "a modal owns the foreground"
        );
        let popup = marks.popup.expect("the menu is marked");
        assert!(text.lines().nth(popup.y as usize).unwrap().contains('╭'));
        let _ = to_text(&host, 80, 24);
        assert_eq!(
            motion::take().rows,
            [],
            "a new frame starts its marks afresh"
        );
    }

    /// A screen with an icon on its tab and words on the right of the bar.
    struct Synced;
    impl crate::Plugin for Synced {
        fn id(&self) -> &'static str {
            "synced"
        }
        fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
            Ok(())
        }
        fn config(&self) -> Option<toml::Value> {
            None
        }
        fn screens(&self) -> Vec<crate::ScreenSpec> {
            vec![crate::ScreenSpec {
                id: "s",
                name: "shelves and boxes",
                key: 'B',
            }]
        }
        fn icon(&self, _: &str) -> Option<Icon> {
            Some(Icon::Repo)
        }
        fn top_right(&self, _: &str) -> Option<String> {
            Some("synced 12s ago · every 1m".into())
        }
    }

    #[test]
    fn tab_icons_give_way_before_the_words_on_the_right_of_the_bar() {
        let mut host = Host::new(vec![Box::new(Synced)], 0).unwrap();
        host.load_config(Some("[theme]\nicons = \"unicode\"\n"))
            .unwrap();
        let top = |w| to_text(&host, w, 24).lines().next().unwrap().to_string();
        let wide = top(120);
        assert!(
            wide.contains("▤ Shelves and boxes") && wide.contains("⚙ Settings"),
            "{wide}"
        );
        // The mode stays last, at every width.
        assert!(
            wide.ends_with("synced 12s ago · every 1m  ✎ edit"),
            "{wide}"
        );
        let narrow = top(78);
        assert!(narrow.contains(" B ") && !narrow.contains('▤'), "{narrow}");
        assert!(
            narrow.contains("History")
                && narrow.contains("Settings (,) ")
                && narrow.ends_with("synced 12s ago · every 1m  ✎ edit"),
            "{narrow}"
        );
        // Narrower still: the part's words shorten, the mode stays.
        let narrowest = top(50);
        assert!(
            narrowest.contains("History")
                && narrowest.contains("Settings")
                && narrowest.ends_with("✎ edit"),
            "{narrowest}"
        );
    }

    /// Settings at the sizes the mix screens are recorded at: at 80x24 the
    /// sections beside the section with its detail under it; at 160x40 all
    /// three side by side, from 120 cells.
    #[test]
    fn settings_stack_the_detail_narrow_and_put_it_beside_wide() {
        let [nav, middle, detail] = settings_areas(Rect::new(0, 1, 80, 22));
        assert_eq!((nav.x, nav.width, nav.height), (0, 24, 22));
        assert_eq!((middle.x, middle.width), (24, 56));
        assert_eq!((detail.x, detail.width), (24, 56));
        assert_eq!((middle.y, middle.height + detail.height), (1, 22));
        assert_eq!(detail.y, middle.y + middle.height, "the detail under the section");
        let [nav, middle, detail] = settings_areas(Rect::new(0, 1, 160, 38));
        assert_eq!((nav.width, nav.height), (24, 38));
        assert_eq!(middle.x, nav.x + nav.width);
        assert_eq!(detail.x, middle.x + middle.width, "the detail beside the section");
        assert_eq!(middle.width + detail.width, 136);
        assert_eq!((middle.height, detail.height), (38, 38));
        let [_, _, at_119] = settings_areas(Rect::new(0, 1, 119, 38));
        assert!(at_119.height < 38, "under 120 cells the detail goes under");
    }

    #[test]
    fn settings_lists_sections_under_their_group() {
        let mut host = Host::new(
            vec![probe("a", 'A', "alpha").0, probe("b", 'B', "beta").0],
            0,
        )
        .unwrap();
        host.view = View::Settings;
        host.focus = Pane::Nav;
        let text = to_text(&host, 100, 24);
        for want in [
            "│General",
            "├─ parts",
            "├─ colours",
            "└─ icons",
            "│a ",
            "└─ alpha",
            "│b ",
            "└─ beta",
        ] {
            assert!(text.contains(want), "missing {want:?} in\n{text}");
        }
        assert!(
            !text.contains("GENERAL"),
            "group names are not shouted:\n{text}"
        );
        let at = |s: &str| text.find(s).unwrap();
        assert!(
            at("General") < at("└─ alpha"),
            "general comes first:\n{text}"
        );
        assert!(
            !text.contains('▾') && !text.contains('▸'),
            "no chevrons:\n{text}"
        );
    }
}
