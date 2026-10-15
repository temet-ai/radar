//! Slack next to other parts: two stand-ins that know nothing of Slack and
//! only the shared vocabulary. A code host owns `merge-request`s (a list
//! with chips, a page with sections, links it recognises); a tracker owns
//! `ticket`s (keys it recognises, a page with the tabs others add).

use radar_core::plugin::{Binding, Effect, Outcome, Pane, ScreenSpec};
use radar_core::view as v;
use radar_core::{Owned, Plugin, Row, Showing, Subject};

/// The code host's merge requests: iid, title.
const MRS: [(u64, &str); 2] = [
    (263, "Add a stats endpoint for the dashboard"),
    (270, "Tidy the README"),
];

pub(crate) fn mr(iid: u64) -> Subject {
    let title = MRS.iter().find(|(i, _)| *i == iid).map_or("", |m| m.1);
    Subject::new("merge-request", format!("acme/team-a/api!{iid}"), format!("api !{iid}"))
        .with_url(format!("https://git.example/acme/team-a/api/-/merge_requests/{iid}"))
        .with("project", "acme/team-a/api")
        .with("iid", iid.to_string())
        .with("title", title)
}

/// Owns merge requests: a list, and a page per merge request shown.
#[derive(Default)]
pub(crate) struct Code {
    row: usize,
    pages: Vec<Subject>,
}

impl Plugin for Code {
    fn id(&self) -> &'static str {
        "code"
    }
    fn name(&self) -> &'static str {
        "Code"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn owns(&self) -> Vec<Owned> {
        vec![Owned::page("merge-request", "merge requests")]
    }
    fn recognise(&self, text: &str) -> Vec<(std::ops::Range<usize>, Subject)> {
        let prefix = "https://git.example/acme/team-a/api/-/merge_requests/";
        let mut out = Vec::new();
        let mut from = 0;
        while let Some(at) = text[from..].find(prefix) {
            let start = from + at;
            let digits: String = text[start + prefix.len()..].chars().take_while(char::is_ascii_digit).collect();
            let end = start + prefix.len() + digits.len();
            if let Ok(iid) = digits.parse() {
                out.push((start..end, mr(iid)));
            }
            from = end.max(start + 1);
        }
        out
    }
    fn show(&mut self, s: &Subject) -> Showing {
        self.pages.push(s.clone());
        Showing::Page {
            screen: "code",
            effects: vec![],
        }
    }
    fn depth(&self, _: &str) -> usize {
        self.pages.len()
    }
    fn screens(&self) -> Vec<ScreenSpec> {
        vec![ScreenSpec {
            id: "code",
            name: "code",
            key: 'C',
        }]
    }
    fn subject(&self, _: &str, _: Pane) -> Option<Subject> {
        Some(self.pages.last().cloned().unwrap_or_else(|| mr(MRS[self.row].0)))
    }
    fn view(&self, scope: &str, cx: &v::ViewCx<'_>) -> Option<v::ScreenView> {
        let body = match self.pages.last() {
            Some(subject) => {
                let mut lines = vec![v::Text::plain(format!("merge request page {}", subject.label))];
                for (title, section) in cx.parts.sections(subject) {
                    lines.push(v::Text::plain(format!("== {title}")));
                    lines.extend(section);
                }
                v::Node::Document { lines, cut: false }
            }
            None => v::Node::List {
                rows: MRS.iter().map(|(iid, title)| {
                    let subject = mr(*iid);
                    let chips: Vec<String> = cx.parts.chips(&subject).into_iter().map(|c| format!("[{}]", c.text))
                        .chain(cx.parts.cells(&subject).into_iter().map(|c| format!("[{}]", c.text))).collect();
                    v::ListRow::new(iid.to_string(), format!("{} {title} {}", subject.label, chips.join(" ")))
                }).collect(),
                selected: Some(self.row),
                editing: None,
            },
        };
        let mut view = v::ScreenView::new(scope, vec![v::PaneView::new(v::PaneId::Middle, "code", body)]);
        view.actions = crate::actions(&self.rows());
        Some(view)
    }
    fn intent(&mut self, _: &str, _: Pane, intent: &v::Intent) -> Outcome {
        match intent {
            v::Intent::Key { chord } if chord.key == v::Key::Esc && !self.pages.is_empty() => {
                self.pages.pop();
                Outcome::done()
            }
            v::Intent::Select { target } if self.pages.is_empty() => {
                if let Some(i) = MRS.iter().position(|(iid, _)| target.as_str() == iid.to_string()) {
                    self.row = i;
                    Outcome::done()
                } else { Outcome::Ignored }
            }
            v::Intent::Activate { .. } if self.pages.is_empty() => {
                self.pages.push(mr(MRS[self.row].0));
                Outcome::done()
            }
            _ => Outcome::Ignored,
        }
    }
    fn keys(&self, _: &str, _: Pane) -> Vec<Binding> {
        self.rows().into_iter().map(|(_, b)| b).collect()
    }
}

pub(crate) fn ticket(key: &str) -> Subject {
    Subject::new("ticket", key, key)
        .with_url(format!("https://tracker.example/{key}"))
        .with("title", "Stats on the dashboard")
}

/// Owns tickets: recognises `ABC-<n>`, and draws the tabs others add on a
/// ticket's page, with `⏎` showing a row's subject.
#[derive(Default)]
pub(crate) struct Tracker {
    pages: Vec<Subject>,
    tab: usize,
    row: usize,
    rows: std::cell::RefCell<Vec<Row>>,
}

impl Plugin for Tracker {
    fn id(&self) -> &'static str {
        "tracker"
    }
    fn name(&self) -> &'static str {
        "Tracker"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn owns(&self) -> Vec<Owned> {
        vec![Owned::page("ticket", "tickets")]
    }
    fn recognise(&self, text: &str) -> Vec<(std::ops::Range<usize>, Subject)> {
        let mut out = Vec::new();
        let mut from = 0;
        while let Some(at) = text[from..].find("ABC-") {
            let start = from + at;
            let digits: String = text[start + 4..].chars().take_while(char::is_ascii_digit).collect();
            let end = start + 4 + digits.len();
            if !digits.is_empty() {
                out.push((start..end, ticket(&text[start..end])));
            }
            from = end.max(start + 1);
        }
        out
    }
    fn show(&mut self, s: &Subject) -> Showing {
        self.pages.push(s.clone());
        Showing::Page {
            screen: "tickets",
            effects: vec![],
        }
    }
    fn depth(&self, _: &str) -> usize {
        self.pages.len()
    }
    fn screens(&self) -> Vec<ScreenSpec> {
        vec![ScreenSpec {
            id: "tickets",
            name: "tickets",
            key: 'T',
        }]
    }
    fn subject(&self, _: &str, _: Pane) -> Option<Subject> {
        Some(self.pages.last().cloned().unwrap_or_else(|| ticket("ABC-12")))
    }
    fn view(&self, scope: &str, cx: &v::ViewCx<'_>) -> Option<v::ScreenView> {
        let mut lines = Vec::new();
        match self.pages.last() {
            None => lines.push(v::Text::plain("ABC-12 Stats on the dashboard")),
            Some(s) => {
                let tabs = cx.parts.tabs(s);
                let mut names = vec!["Overview".to_string()];
                names.extend(tabs.iter().map(|(t, _)| t.clone()));
                let shown: Vec<String> = names
                    .iter()
                    .enumerate()
                    .map(|(i, n)| if i == self.tab { format!("[{n}]") } else { n.clone() })
                    .collect();
                lines.push(v::Text::plain(format!("ticket page {} · {}", s.label, shown.join("  "))));
                let rows = if self.tab == 0 { vec![] } else { tabs.get(self.tab - 1).map(|(_, r)| r.clone()).unwrap_or_default() };
                for (i, r) in rows.iter().enumerate() {
                    let mut line = v::Text::plain(if i == self.row { "> " } else { "  " });
                    line.extend(r.line.clone());
                    lines.push(line);
                }
                *self.rows.borrow_mut() = rows;
            }
        }
        let mut view = v::ScreenView::new(scope, vec![v::PaneView::new(
            v::PaneId::Middle, "tickets", v::Node::Document { lines, cut: false },
        )]);
        view.actions = crate::actions(&self.rows());
        Some(view)
    }
    fn intent(&mut self, _: &str, _: Pane, intent: &v::Intent) -> Outcome {
        let v::Intent::Key { chord } = intent else { return Outcome::Ignored };
        match chord.key {
            v::Key::Esc if !self.pages.is_empty() => {
                self.pages.pop();
                Outcome::done()
            }
            v::Key::Enter if self.pages.is_empty() => {
                self.pages.push(ticket("ABC-12"));
                Outcome::done()
            }
            v::Key::Char(']') => {
                self.tab += 1;
                self.row = 0;
                Outcome::done()
            }
            v::Key::Down => {
                self.row += 1;
                Outcome::done()
            }
            v::Key::Enter => match self.rows.borrow().get(self.row).and_then(|r| r.subject.clone()) {
                Some(s) => Outcome::Handled(vec![Effect::Show(s)]),
                None => Outcome::Ignored,
            },
            _ => Outcome::Ignored,
        }
    }
    fn keys(&self, _: &str, _: Pane) -> Vec<Binding> {
        self.rows().into_iter().map(|(_, b)| b).collect()
    }
}

impl Code {
    /// The `x` rows, each with the id of its action.
    fn rows(&self) -> Vec<crate::Row> {
        let s = self.subject("code", Pane::Middle).expect("a merge request");
        let mut b = vec![crate::head(Binding::heading(s.label))];
        if self.pages.is_empty() {
            b.push(crate::act("slack.code.open_page", Binding::new("⏎", "open its page")));
        } else {
            b.push(crate::act("slack.code.back", Binding::new("esc", "back")));
        }
        b
    }
}

impl Tracker {
    /// The `x` rows, each with the id of its action.
    fn rows(&self) -> Vec<crate::Row> {
        let mut b = vec![crate::head(Binding::heading(
            self.subject("tickets", Pane::Middle).expect("a ticket").label,
        ))];
        if !self.pages.is_empty() {
            b.push(crate::act("slack.tickets.back", Binding::new("esc", "back")));
        }
        b
    }
}
