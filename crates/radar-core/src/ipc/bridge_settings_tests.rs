//! A field a part's settings section asks for, through the bridge as a
//! renderer that lays the section out itself (Ink) sees it: a press on the
//! row selects it from the nav, the field reaches the snapshot, every
//! letter typed is a new snapshot, ⏎ saves the text and esc leaves it, and
//! the snapshot after says the field is gone.

use super::*;
use crate::host::{Host, View};
use crate::plugin::{Effect, Menu, Outcome, Pane, Plugin, SectionSpec};
use crate::view::{self as v, Chord, Intent, Key, Target};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Rules {
    selected: usize,
    allowed: Vec<String>,
}

struct RulesProbe(Arc<Mutex<Rules>>);

const ROWS: [&str; 2] = ["mode", "allow"];

impl Plugin for RulesProbe {
    fn id(&self) -> &'static str {
        "rules-probe"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn sections(&self) -> Vec<SectionSpec> {
        vec![SectionSpec {
            id: "rules",
            name: "rules",
            group: "probe",
        }]
    }
    fn section_view(&self, _: &str, _: &v::ViewCx<'_>) -> Option<v::SectionView> {
        let s = self.0.lock().unwrap();
        let allowed = if s.allowed.is_empty() { "none".to_string() } else { s.allowed.join(", ") };
        Some(v::SectionView {
            version: v::VERSION,
            scope: "rules".into(),
            body: v::Node::List {
                rows: vec![
                    v::ListRow::new("mode", "mode          quiet"),
                    v::ListRow::new("allow", format!("always allow  {allowed}")),
                ],
                selected: Some(s.selected),
                editing: None,
            },
            detail_title: ROWS[s.selected].into(),
            detail: vec![v::Text::plain("what the row does")],
            detail_blocks: vec![],
            actions: vec![],
        })
    }
    fn intent(&mut self, _: &str, _: Pane, intent: &Intent) -> Outcome {
        let at = |t: &Target| ROWS.iter().position(|r| *r == t.as_str());
        // ⏎ on the selected row is activating it, as a part's section does.
        let selected = Intent::Activate { target: Target::new(ROWS[self.0.lock().unwrap().selected]) };
        let intent = match intent {
            Intent::Key { chord } if chord.key == Key::Enter => &selected,
            other => other,
        };
        match intent {
            Intent::Select { target } if let Some(i) = at(target) => {
                self.0.lock().unwrap().selected = i;
                Outcome::Handled(vec![])
            }
            Intent::Activate { target } if let Some(i) = at(target) => {
                self.0.lock().unwrap().selected = i;
                if i == 1 {
                    Outcome::Handled(vec![Effect::Menu(Menu::typed("probe.allow", "Always allow, as kind(argument)", ""))])
                } else {
                    Outcome::Handled(vec![])
                }
            }
            _ => Outcome::Ignored,
        }
    }
    fn on_menu(&mut self, menu: &str, value: &str) -> Vec<Effect> {
        if menu != "probe.allow" || value.trim().is_empty() {
            return vec![];
        }
        let mut s = self.0.lock().unwrap();
        match s.allowed.iter().position(|r| r == value) {
            Some(i) => {
                s.allowed.remove(i);
                vec![Effect::Notify("removed".into())]
            }
            None => {
                s.allowed.push(value.into());
                vec![Effect::Notify("added".into())]
            }
        }
    }
}

fn hello() -> ToHost {
    ToHost::Hello {
        protocol: PROTOCOL,
        view: crate::view::VERSION,
        capabilities: Capabilities {
            columns: 160,
            rows: 40,
            mouse: true,
            true_colour: true,
            image: ImageProtocol::None,
            plain_text: false,
        },
    }
}

/// On Settings, the probe's section chosen, the nav focused (as after a
/// click on the section's nav row).
fn on_its_section() -> (Bridge, Arc<Mutex<Rules>>) {
    let rules = Arc::new(Mutex::new(Rules::default()));
    let mut host = Host::new(vec![Box::new(RulesProbe(rules.clone()))], 0).unwrap();
    host.view = View::Settings;
    host.section = host.sections.iter().position(|s| s.1.id == "rules").expect("the probe's section");
    host.focus = crate::Pane::Nav;
    let mut bridge = Bridge::new(host, 160, 40);
    bridge.receive(hello());
    (bridge, rules)
}

/// What the renderer is sent after `intent`, each snapshot acknowledged as
/// it arrives.
fn after(bridge: &mut Bridge, seq: &mut u64, intent: Intent) -> Vec<ToRenderer> {
    let before = bridge.outgoing();
    ack(bridge, &before);
    *seq += 1;
    bridge.receive(ToHost::Intent { seq: *seq, cursor: 0, intent });
    let out = bridge.outgoing();
    ack(bridge, &out);
    out
}

fn ack(bridge: &mut Bridge, out: &[ToRenderer]) {
    if let Some(cursor) = out.iter().rev().find_map(|m| match m {
        ToRenderer::Snapshot(s) => Some(s.cursor),
        _ => None,
    }) {
        bridge.receive(ToHost::Ack { cursor });
    }
}

fn snapshot(out: &[ToRenderer]) -> &Snapshot {
    out.iter()
        .rev()
        .find_map(|m| match m {
            ToRenderer::Snapshot(s) => Some(&**s),
            _ => None,
        })
        .unwrap_or_else(|| panic!("a snapshot: {out:?}"))
}

fn key(k: Key) -> Intent {
    Intent::Key { chord: Chord::new(k) }
}

fn hints(s: &Snapshot) -> Vec<String> {
    s.chrome.status.hints.iter().map(|h| format!("{} {}", h.keys, h.label)).collect()
}

fn allow_row(s: &Snapshot) -> String {
    let Some(v::Node::List { rows, .. }) = s.section.as_ref().map(|s| &s.body) else {
        panic!("a list section: {s:?}")
    };
    rows[1].text.0.iter().map(|i| i.text.as_str()).collect()
}

#[test]
fn a_press_on_a_section_row_while_the_nav_has_the_keys_selects_it() {
    let (mut bridge, rules) = on_its_section();
    let mut seq = 0;
    let out = after(&mut bridge, &mut seq, Intent::Select { target: Target::new("allow") });
    assert_eq!(rules.lock().unwrap().selected, 1, "the row is selected");
    let s = snapshot(&out);
    assert_eq!(s.focus, crate::view::PaneId::Middle, "the section has the keys");
    assert_eq!(s.section.as_ref().map(|s| s.detail_title.as_str()), Some("allow"));
}

#[test]
fn a_field_a_section_asks_for_is_drawn_typed_into_kept_and_gone_after() {
    let (mut bridge, rules) = on_its_section();
    let mut seq = 0;
    let out = after(&mut bridge, &mut seq, Intent::Activate { target: Target::new("allow") });
    assert!(
        out.iter().any(|m| matches!(m, ToRenderer::InputRequest(r) if r.title.starts_with("Always allow"))),
        "the field is asked for: {out:?}"
    );
    let s = snapshot(&out);
    let menu = s.chrome.menu.as_ref().expect("the snapshot carries the field");
    assert_eq!((menu.typed.as_deref(), menu.caret), (Some(""), Some(0)));
    assert_eq!(hints(s), ["⏎ save", "esc cancel"], "how to finish it");

    // Each letter is a new snapshot with the text so far.
    let mut text = String::new();
    for c in "shell(ls)".chars() {
        text.push(c);
        let out = after(&mut bridge, &mut seq, key(Key::Char(c)));
        let menu = snapshot(&out).chrome.menu.clone().expect("still open");
        assert_eq!(menu.typed.as_deref(), Some(text.as_str()));
        assert_eq!(menu.caret, Some(text.chars().count()));
    }

    // ⏎ saves it: the field is gone from the snapshot and the row reads it.
    let out = after(&mut bridge, &mut seq, key(Key::Enter));
    let s = snapshot(&out);
    assert!(s.chrome.menu.is_none(), "the field closed: {:?}", s.chrome.menu);
    assert_eq!(rules.lock().unwrap().allowed, ["shell(ls)"]);
    assert!(allow_row(s).ends_with("shell(ls)"), "{}", allow_row(s));
    assert_eq!(rules.lock().unwrap().selected, 1, "back on the same row");

    // Opened again, esc leaves it and keeps nothing.
    let out = after(&mut bridge, &mut seq, key(Key::Enter));
    assert!(snapshot(&out).chrome.menu.as_ref().is_some_and(|m| m.typed.is_some()), "⏎ on the row opens it again");
    after(&mut bridge, &mut seq, key(Key::Char('x')));
    let out = after(&mut bridge, &mut seq, key(Key::Esc));
    let s = snapshot(&out);
    assert!(s.chrome.menu.is_none(), "esc closed it");
    assert_eq!(rules.lock().unwrap().allowed, ["shell(ls)"]);
    assert_eq!(s.focus, crate::view::PaneId::Middle);
    assert_eq!(rules.lock().unwrap().selected, 1);
}
