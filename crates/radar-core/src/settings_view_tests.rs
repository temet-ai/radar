//! The set-up page and the host-owned settings sections as view data: their
//! rows, which one is selected, and the detail each selection shows.

use super::*;
use crate::compose::Setup;
use crate::design::theme::Tier;
use crate::host::View;
use crate::plugin::{Plugin, SectionSpec};
use crate::view::measure;

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

fn words(lines: &[Text]) -> Vec<String> {
    lines.iter().map(|l| l.words().trim_end().to_string()).collect()
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
fn the_parts_section_lists_each_part_under_a_heading() {
    let mut host = host();
    goto(&mut host, "parts");
    host.parts_row = 1;
    let view = section_view(&host, Tier::Mono).expect("parts view");
    assert_eq!(view.scope, "parts");
    let Node::List { rows, selected, .. } = &view.body else { panic!("{:?}", view.body) };
    assert_eq!(rows[0].text.words(), "COMPILED INTO THIS RADAR");
    assert!(rows[0].heading);
    let parts: Vec<_> = rows.iter().filter(|r| r.target.0.starts_with("part:")).collect();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].text.words().trim_end(), "○ Alpha   not set up");
    assert_eq!(parts[1].text.words().trim_end(), "✓ Beta    set up");
    // Selected: the row of the part the cursor is on, not the heading.
    assert_eq!(rows[selected.unwrap()].target.0, "part:1");
    assert_eq!(view.detail_title, "Beta");
    let detail = words(&view.detail);
    assert_eq!(detail.first().map(String::as_str), Some("✓ set up"));
    assert!(detail.iter().any(|l| l == "BETA ADDS"), "{detail:?}");
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
fn the_mode_section_has_its_rows_and_a_detail_for_every_mode() {
    let mut host = host();
    goto(&mut host, "mode");
    let view = section_view(&host, Tier::Mono).expect("mode view");
    let Node::List { rows, selected, .. } = &view.body else { panic!("{:?}", view.body) };
    assert_eq!(rows[0].text.words(), "WHAT RADAR MAY CHANGE");
    assert_eq!(rows[5].text.words(), "WHEN IDLE");
    assert!(rows[0].heading && rows[5].heading);
    assert!(rows[2].text.words().contains(host.mode().word()), "{}", rows[2].text.words());
    assert!(rows[7].text.words().contains("pause if idle"));
    assert_eq!(rows[selected.unwrap()].target.0, "mode");
    assert_eq!(view.detail_title, "the modes");
    let detail = words(&view.detail);
    for (k, m) in Mode::ALL.into_iter().enumerate() {
        assert_eq!(detail[2 * k + 1], format!("    {}", m.about()), "{}", m.word());
        // The mode in force is the one marked.
        assert_eq!(view.detail[2 * k].words().starts_with("● "), m == host.mode(), "{}", m.word());
    }
    assert!(detail.iter().any(|l| l.starts_with("Local actions")), "{detail:?}");
}

#[test]
fn the_pause_row_selected_shows_its_own_detail() {
    let mut host = host();
    goto(&mut host, "mode");
    host.pause.row = 1;
    let view = section_view(&host, Tier::Mono).unwrap();
    let Node::List { rows, selected, .. } = &view.body else { panic!() };
    assert_eq!(rows[selected.unwrap()].target.0, "pause");
    let detail = words(&view.detail);
    assert_eq!(detail.len(), 3);
    assert!(detail[0].starts_with("Every part stops asking its service"), "{detail:?}");
    assert!(detail[2].ends_with("[general] idle_minutes"), "{detail:?}");
}

#[test]
fn the_colours_section_has_a_row_per_model_row_and_its_detail() {
    let mut host = host();
    goto(&mut host, "colours");
    // Every selectable row in turn: the list is the model's rows, the
    // selection is the cursor, and the detail is that row's.
    let model = host.colours.rows();
    for at in 0..model.len() {
        host.colours.row = at;
        if matches!(model[at], crate::colours::Row::Header(_)) {
            continue;
        }
        let view = section_view(&host, Tier::Mono).expect("colours view");
        let Node::List { rows, selected, .. } = &view.body else { panic!("{:?}", view.body) };
        assert_eq!(rows.len(), model.len());
        assert_eq!(*selected, Some(at));
        assert_eq!(view.detail_title, host.colours.detail_title());
        assert!(!view.detail.is_empty(), "row {at}");
        for (y, row) in rows.iter().enumerate() {
            assert_eq!(row.heading, matches!(model[y], crate::colours::Row::Header(_)), "row {y}");
        }
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
    let icon = host.colours.icon_selected();
    let detail = words(&view.detail);
    assert_eq!(detail[0], format!("Shows {}.", icon.about()));
    assert_eq!(detail.last().map(String::as_str), Some(format!("config: [icons] {}", icon.name()).as_str()));
}

#[test]
fn a_sections_detail_rows_set_how_far_settings_scrolls() {
    let mut host = host();
    goto(&mut host, "mode");
    let view = section_view(&host, Tier::Mono).unwrap();
    // Wide enough that no line wraps: a row per line.
    assert_eq!(measure::detail_rows(&view, 400, measure::SETTINGS_INSET), view.detail.len());
    // A narrow pane wraps the long lines: more rows than lines.
    let narrow = measure::detail_rows(&view, 30, measure::SETTINGS_INSET);
    assert!(narrow > view.detail.len(), "{narrow} rows for {} lines", view.detail.len());
    // The pane's text room at a 100×30 body is 72×13: it scrolls by the rest.
    let room = measure::settings_detail_text(measure::Size::new(100, 30));
    assert_eq!((room.width, room.height), (72, 13));
    let rows = measure::detail_rows(&view, room.width, measure::SETTINGS_INSET);
    let scroll = crate::ui::Scroll::new();
    scroll.fit(room.height, rows as u16);
    assert_eq!(usize::from(scroll.max()), rows.saturating_sub(usize::from(room.height)));
}
