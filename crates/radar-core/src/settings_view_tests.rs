//! The set-up page and the host-owned settings sections as view data say
//! what Ratatui draws for the same host state (`parts::render`,
//! `mode::render`, their detail lines and `parts::setup_page`).

use super::*;
use crate::compose::Setup;
use crate::design::theme::Tier;
use crate::host::View;
use crate::plugin::{Plugin, SectionSpec};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Line;

struct Probe {
    id: &'static str,
    ready: bool,
}

impl Plugin for Probe {
    fn id(&self) -> &'static str {
        self.id
    }
    fn name(&self) -> &'static str {
        self.id
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn set_up(&self) -> Setup {
        if self.ready { Setup::Ready } else { Setup::Needed("Add a tenancy.".into()) }
    }
    fn sections(&self) -> Vec<SectionSpec> {
        vec![SectionSpec { id: self.id, name: self.id, group: "Probe group" }]
    }
}

fn host() -> Host {
    Host::new(
        vec![Box::new(Probe { id: "Alpha", ready: false }), Box::new(Probe { id: "Beta", ready: true })],
        0,
    )
    .unwrap()
}

fn goto(host: &mut Host, id: &str) {
    host.view = View::Settings;
    host.section = host.sections.iter().position(|(_, s)| s.id == id).expect("a section");
}

fn row_text(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim().to_string()
}

fn plain(lines: &[Line<'_>]) -> Vec<String> {
    lines.iter().map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>().trim_end().to_string()).collect()
}

fn words(lines: &[Text]) -> Vec<String> {
    lines.iter().map(|l| l.words().trim_end().to_string()).collect()
}

fn theme(host: &Host) -> crate::theme::Theme {
    host.build_theme(Tier::Mono)
}

#[test]
fn only_the_hosts_own_sections_have_a_view_here() {
    let mut host = host();
    for (id, has) in [("parts", true), ("mode", true), ("Alpha", false)] {
        goto(&mut host, id);
        assert_eq!(section_view(&host, Tier::Mono).is_some(), has, "{id}");
    }
}

#[test]
fn the_parts_section_lists_each_part_as_ratatui_does() {
    let mut host = host();
    goto(&mut host, "parts");
    host.parts_row = 1;
    let view = section_view(&host, Tier::Mono).expect("parts view");
    assert_eq!(view.scope, "parts");
    let Node::List { rows, selected, .. } = &view.body else { panic!("{:?}", view.body) };
    let t = theme(&host);
    let area = Rect::new(0, 0, 60, 12);
    let mut buf = Buffer::empty(area);
    let hits = crate::parts::render(&host, &t, area, &mut buf, true);
    assert_eq!(hits.len(), 2);
    for (k, (rect, i)) in hits.iter().enumerate() {
        let row = rows.iter().find(|r| r.target.0 == format!("part:{i}")).expect("a row per part");
        assert_eq!(row.text.words().trim_end(), row_text(&buf, rect.y), "part {k}");
    }
    assert_eq!(row_text(&buf, 0), "COMPILED INTO THIS RADAR");
    assert_eq!(rows[0].text.words(), "COMPILED INTO THIS RADAR");
    assert!(rows[0].heading);
    // Selected: the row of the part the cursor is on, not the heading.
    assert_eq!(rows[selected.unwrap()].target.0, "part:1");
    assert_eq!(view.detail_title, "Beta");
    assert_eq!(words(&view.detail), plain(&crate::parts::detail_lines(&host, &t)));
}

#[test]
fn the_parts_detail_names_what_a_part_needs() {
    let mut host = host();
    goto(&mut host, "parts");
    host.parts_row = 0;
    let view = section_view(&host, Tier::Mono).unwrap();
    assert_eq!(view.detail[0].words(), "○ not set up: Add a tenancy.");
    assert_eq!(view.detail_title, "Alpha");
}

#[test]
fn the_mode_section_reads_as_ratatui_draws_it() {
    let mut host = host();
    goto(&mut host, "mode");
    let view = section_view(&host, Tier::Mono).expect("mode view");
    let Node::List { rows, selected, .. } = &view.body else { panic!("{:?}", view.body) };
    let t = theme(&host);
    let area = Rect::new(0, 0, 60, 12);
    let mut buf = Buffer::empty(area);
    crate::mode::render(&host, &t, area, &mut buf, true);
    // One row per line of the drawing, from its first.
    for (y, row) in rows.iter().enumerate() {
        assert_eq!(row.text.words().trim(), row_text(&buf, y as u16), "line {y}");
    }
    assert_eq!(rows[selected.unwrap()].target.0, "mode");
    assert_eq!(view.detail_title, "the modes");
    assert_eq!(words(&view.detail), plain(&crate::mode::detail_lines(&host, &t)));
}

#[test]
fn the_pause_row_selected_shows_its_own_detail() {
    let mut host = host();
    goto(&mut host, "mode");
    host.pause.row = 1;
    let view = section_view(&host, Tier::Mono).unwrap();
    let Node::List { rows, selected, .. } = &view.body else { panic!() };
    assert_eq!(rows[selected.unwrap()].target.0, "pause");
    assert_eq!(words(&view.detail), plain(&crate::mode::detail_lines(&host, &theme(&host))));
}

#[test]
fn the_colours_section_has_a_row_per_ratatui_row_and_its_detail() {
    let mut host = host();
    goto(&mut host, "colours");
    let t = theme(&host);
    // Every selectable row in turn: its row text and its detail are Ratatui's.
    let n = host.colours.rows().len();
    for at in 0..n {
        host.colours.row = at;
        if matches!(host.colours.rows()[at], crate::colours::Row::Header(_)) {
            continue;
        }
        let view = section_view(&host, Tier::Mono).expect("colours view");
        let Node::List { rows, selected, .. } = &view.body else { panic!("{:?}", view.body) };
        assert_eq!(rows.len(), n);
        assert_eq!(*selected, Some(at));
        assert_eq!(view.detail_title, host.colours.detail_title());
        assert_eq!(words(&view.detail), plain(&host.colours.detail_lines(&t)), "row {at}");
    }
    let area = Rect::new(0, 0, 60, n as u16);
    let mut buf = Buffer::empty(area);
    let cursor = std::cell::Cell::new(None);
    host.colours.row = 1;
    host.colours.render(&t, area, &mut buf, true, &cursor);
    let view = section_view(&host, Tier::Mono).unwrap();
    let Node::List { rows, .. } = &view.body else { panic!() };
    for (y, row) in rows.iter().enumerate() {
        let right = row.right.as_ref().map(|r| r.words()).unwrap_or_default();
        let said = format!("{}{}", row.text.words().trim_end(), if right.is_empty() { String::new() } else { format!(" {right}") });
        let drawn = row_text(&buf, y as u16);
        assert!(drawn.replace("  ", " ").starts_with(said.replace("  ", " ").split(" yours").next().unwrap().trim()), "row {y}: {drawn:?} vs {said:?}");
    }
}

#[test]
fn typing_a_colour_is_an_editing_field_on_its_row() {
    let mut host = host();
    goto(&mut host, "colours");
    let at = host.colours.rows().iter().position(|r| matches!(r, crate::colours::Row::Slot(_))).unwrap();
    host.colours.row = at;
    host.colours.edit = Some(crate::input::TextInput::new("#abc"));
    let view = section_view(&host, Tier::Mono).unwrap();
    let Node::List { editing, .. } = &view.body else { panic!() };
    let e = editing.as_ref().expect("a field");
    assert_eq!((e.row, e.at, e.text.as_str()), (at, 23, "#abc"));
}

#[test]
fn the_icons_section_has_a_row_per_group_and_icon() {
    let mut host = host();
    goto(&mut host, "icons");
    assert!(host.on_icons());
    host.colours.icons_first_row();
    let view = section_view(&host, Tier::Mono).expect("icons view");
    let Node::List { rows, selected, .. } = &view.body else { panic!("{:?}", view.body) };
    assert_eq!(rows.len(), crate::icons::rows().len());
    assert!(rows[0].heading);
    assert!(!rows[selected.unwrap()].heading);
    assert_eq!(view.detail_title, host.colours.icons_detail_title());
    assert_eq!(words(&view.detail), plain(&host.colours.icons_detail_lines(&theme(&host))));
}
