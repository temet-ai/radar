//! The host's `parts` settings section: every part compiled into this build,
//! whether it is set up, what it has, and what it adds to the others. So the
//! user sees what turning one on would bring. The section's view is
//! [`crate::settings_view`]'s.

use crate::compose;
use crate::host::{Host, HostEffect};
use crate::plugin::{Binding, Pane};
use crate::view::{Chord, Key};

impl Host {
    /// A key in the parts section: move, go to a part's settings, back out.
    pub(crate) fn parts_key(&mut self, key: Chord) -> Vec<HostEffect> {
        let n = self.plugins.len();
        if let Some(down) = key.vertical() {
            self.parts_row = crate::ui::rows::step(self.parts_row, n, down);
            return vec![];
        }
        match key.key {
            Key::Enter if self.parts_row < n => self.go_to_settings_of(self.parts_row),
            Key::Esc => self.focus = Pane::Nav,
            _ => {}
        }
        vec![]
    }
}

/// The section's `x` menu.
pub(crate) fn keys(host: &Host) -> Vec<Binding> {
    let mut rows = vec![Binding::heading("parts")];
    if let Some(p) = host.plugins.get(host.parts_row)
        && host.first_section_of(host.parts_row).is_some()
    {
        let name = p.name();
        rows.push(Binding::new(
            "⏎",
            if compose::ready(p.as_ref()) {
                format!("go to {name}'s settings")
            } else {
                format!("set up {name}")
            },
        ));
    }
    rows
}

/// The detail panel's title: the selected part.
pub(crate) fn detail_title(host: &Host) -> String {
    host.plugins
        .get(host.parts_row)
        .map_or("about".into(), |p| p.name().to_string())
}
