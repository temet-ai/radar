//! Settings viewport and interaction checks, with no service work.

use crate::host::{Event, View};
use crate::{Host, Outcome, Pane, Plugin, SectionSpec};
use crate::view::{self as v, Intent};
use crate::view::{Chord, Key};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct Settings(Arc<AtomicUsize>);
impl Plugin for Settings {
    fn id(&self) -> &'static str {
        "settings-probe"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn section_folds(&self, _: &str) -> bool { true }
    fn bulk(&self, _: &str) -> Vec<crate::MenuItem> {
        vec![crate::MenuItem::new('c', "check all", "check the connections", "check")]
    }
    fn sections(&self) -> Vec<SectionSpec> {
        vec![SectionSpec {
            id: "connection",
            name: "connection",
            group: "probe",
        }]
    }
    fn section_view(&self, _: &str, cx: &v::ViewCx<'_>) -> Option<v::SectionView> {
        let selected = self.0.load(Ordering::Relaxed);
        let mut detail = vec![v::Text::plain(format!("checked site {selected} at {}", cx.now))];
        detail.extend((0..35).map(|i| v::Text::plain(format!("explanation {i}"))));
        detail.push(v::Text::plain(format!("config: site {selected} token")));
        Some(v::SectionView {
            version: v::VERSION,
            scope: "connection".into(),
            body: v::Node::List {
                rows: (0..2).map(|i| v::ListRow::new(i.to_string(), format!("site {i} token"))).collect(),
                selected: Some(selected),
                editing: None,
            },
            detail_title: "token".into(),
            detail,
            detail_blocks: vec![],
            actions: vec![],
        })
    }
    fn intent(&mut self, _: &str, _: Pane, intent: &Intent) -> Outcome {
        if let Intent::Select { target } = intent
            && let Ok(i) = target.as_str().parse() {
            self.0.store(i, Ordering::Relaxed);
            return Outcome::Handled(vec![]);
        }
        Outcome::Ignored
    }
}

fn setup() -> (Host, Arc<AtomicUsize>) {
    let selected = Arc::new(AtomicUsize::new(0));
    let mut host = Host::new(vec![Box::new(Settings(selected.clone()))], 0).unwrap();
    host.view = View::Settings;
    host.focus = Pane::Middle;
    (host, selected)
}
fn press(host: &mut Host, code: Key) {
    host.update(crate::host::chord_event(Chord::new(code)));
}
fn draw(host: &Host, width: u16, height: u16) -> String {
    crate::headless::text_of(host, width, height)
}
#[test]
fn settings_show_the_checked_site_at_both_widths() {
    let (host, _) = setup();
    for (width, height) in [(80, 24), (160, 40)] {
        let text = draw(&host, width, height);
        assert!(text.contains("checked site 0"), "{text}");
    }
}

#[test]
fn compact_settings_keep_scroll_and_navigation_hints_when_bulk_and_folds_compete() {
    let (host, _) = setup();
    let text = draw(&host, 80, 24);
    let bar = text.lines().last().unwrap();
    assert!(bar.contains("PgUp/PgDn scroll"), "{bar}");
    assert!(bar.contains("←→↑↓ navigate") && bar.contains("extra actions"), "{bar}");
}

#[test]
fn settings_detail_keyboard_and_menu_agree() {
    let (mut host, selected) = setup();
    draw(&host, 80, 24);
    press(&mut host, Key::Tab);
    assert_eq!(host.focus, Pane::Detail);
    press(&mut host, Key::Down);
    assert_eq!(host.settings_detail.at(), 1);
    assert_eq!(selected.load(Ordering::Relaxed), 0);
    press(&mut host, Key::Down);
    assert_eq!(host.settings_detail.at(), 2);
    // Nothing here acts on a row, so `x` opens the pane's menu, which goes back.
    press(&mut host, Key::Char('x'));
    let menu = &host.menu.as_ref().unwrap().menu;
    assert!(menu.items.iter().all(|i| !i.label.contains("go into")));
    assert!(
        menu.items
            .iter()
            .any(|i| i.label == "back to the settings fields")
    );
    press(&mut host, Key::Esc);
    press(&mut host, Key::Esc);
    assert_eq!(host.focus, Pane::Middle);
    press(&mut host, Key::Char('3'));
    assert_eq!(host.focus, Pane::Detail);
    press(&mut host, Key::BackTab);
    assert_eq!(host.focus, Pane::Middle);
}

#[test]
fn settings_detail_resets_for_rows_not_focus_or_changing_check_text() {
    let (mut host, selected) = setup();
    draw(&host, 80, 24);
    press(&mut host, Key::PageDown);
    let at = host.settings_detail.at();
    assert!(at > 0);
    assert_eq!(selected.load(Ordering::Relaxed), 0);
    press(&mut host, Key::Right);
    press(&mut host, Key::Left);
    host.update(Event::Tick(1));
    draw(&host, 80, 24);
    assert_eq!(host.settings_detail.at(), at);
    press(&mut host, Key::Down);
    assert_eq!(selected.load(Ordering::Relaxed), 1);
    assert_eq!(
        host.settings_detail.at(),
        0,
        "same title, different site's field"
    );
    press(&mut host, Key::PageDown);
    draw(&host, 80, 24);
    host.update(Event::Intent(Intent::Select { target: v::Target::new("0") }));
    assert_eq!(selected.load(Ordering::Relaxed), 0);
    assert_eq!(host.settings_detail.at(), 0);
}

#[test]
fn settings_detail_reaches_the_end_and_clamps_after_resize() {
    let (mut host, _) = setup();
    draw(&host, 80, 24);
    press(&mut host, Key::Char('3'));
    press(&mut host, Key::End);
    let text = draw(&host, 80, 24);
    assert!(text.contains("config: site 0 token"), "{text}");
    assert!(host.settings_detail.at() > 0);
    draw(&host, 160, 80);
    assert_eq!(host.settings_detail.at(), 0);
    assert_eq!(host.settings_detail.max(), 0);
    press(&mut host, Key::PageDown);
    assert_eq!(host.message.as_deref(), Some("nothing to scroll here"));
}
