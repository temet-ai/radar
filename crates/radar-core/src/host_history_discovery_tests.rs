//! Baseline-compatible behavioral controls for the new entry points.
use super::*;
struct Item;
impl Plugin for Item {
    fn id(&self) -> &'static str {
        "item"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn screens(&self) -> Vec<ScreenSpec> {
        vec![ScreenSpec {
            id: "items",
            name: "items",
            key: 'I',
        }]
    }
    fn owns(&self) -> Vec<crate::compose::Owned> {
        vec![crate::compose::Owned::page("card", "cards")]
    }
    fn subject(&self, _: &str, _: Pane) -> Option<Subject> {
        Some(Subject::new("card", "ONE-1", "One"))
    }
    fn journal_subject(&self, _: &Subject) -> Option<crate::journal::SubjectSnapshot> {
        Some(crate::journal::SubjectSnapshot::new(
            crate::journal::SubjectKey::new("probe", "https://example.test", "card", "ONE-1")
                .unwrap(),
            crate::journal::SubjectRole::Primary,
        ))
    }
}
#[test]
fn all_activity_is_discoverable_from_a_screen_without_plugin_actions() {
    let mut host = Host::new(vec![Box::new(Item)], 0).unwrap();
    host.update(Event::Key(KeyEvent::new(
        KeyCode::Char('x'),
        KeyModifiers::NONE,
    )));
    assert!(
        host.menu.as_ref().is_some_and(|menu| menu
            .menu
            .items
            .iter()
            .any(|row| row.label == "all activity history")),
        "x must offer read-only history even when a part has no actions"
    );
}
#[test]
fn selected_subject_history_is_discoverable_in_the_context_menu() {
    let mut host = Host::new(vec![Box::new(Item)], 0).unwrap();
    host.update(Event::Key(KeyEvent::new(
        KeyCode::Char('x'),
        KeyModifiers::NONE,
    )));
    assert!(
        host.menu.as_ref().is_some_and(|menu| menu
            .menu
            .items
            .iter()
            .any(|row| row.label == "history for selected item")),
        "a validated scoped subject must have a history entry"
    );
}
