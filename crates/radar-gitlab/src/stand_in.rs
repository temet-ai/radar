//! GitLab next to another part: a stand-in ticket tracker that knows
//! nothing of GitLab but the shared vocabulary (a `merge-request` subject
//! with a `title` field). It owns `ticket`s, has a screen with a page per
//! shown ticket, and adds chips, a column, a section, a tab and actions to
//! merge requests whose title names one of its tickets.

use radar_core::host::{Event, View};
use radar_core::plugin::{Binding, Effect, Outcome, Pane, ScreenSpec, SectionSpec, Tone};
use radar_core::view::{Chord, Key};
use crate::tests::shown as to_text;
use radar_core::view as v;
use radar_core::{
    Action, Adds, Chip, Contribution, Host, Owned, Plugin, Point, Row, Showing, Subject,
};

/// The tracker's own cache: key, status, summary.
const TICKETS: [(&str, &str, &str); 2] = [
    ("ABC-12", "Review", "Add a stats endpoint for the dashboard"),
    ("ABC-9", "Done", "Cache the dashboard"),
];

#[derive(Default)]
pub(crate) struct Tracker {
    pages: Vec<Subject>,
}

fn ticket(key: &str) -> Subject {
    Subject::new("ticket", key, key).with_url(format!("https://tracker.example/{key}"))
}

impl Tracker {
    /// The rows of the actions menu, with the id of each.
    fn rows(&self) -> Vec<crate::Row> {
        let mut b = vec![
            crate::head(Binding::heading(self.subject("tickets", Pane::Middle).unwrap().label)),
            crate::act("tracker.tickets.open_merge_request", Binding::new("e", "open its merge request")),
        ];
        if !self.pages.is_empty() {
            b.push(crate::act("tracker.tickets.back", Binding::new("esc", "back")));
        }
        b
    }

    /// The ticket a merge request's title names, from the tracker's cache.
    fn of(subject: &Subject) -> Option<(&'static str, &'static str, &'static str)> {
        if !subject.is("merge-request") {
            return None;
        }
        let title = subject.field("title")?;
        TICKETS.into_iter().find(|(k, _, _)| title.contains(k))
    }
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
    fn adds(&self) -> Vec<Adds> {
        vec![Adds::new(
            "merge-request",
            "ticket chips, a ticket column and tab",
        )]
    }
    fn show(&mut self, subject: &Subject) -> Showing {
        self.pages.push(subject.clone());
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
    fn sections(&self) -> Vec<SectionSpec> {
        vec![SectionSpec {
            id: "projects",
            name: "projects",
            group: "tracker",
        }]
    }
    fn subject(&self, _: &str, _: Pane) -> Option<Subject> {
        Some(
            self.pages
                .last()
                .cloned()
                .unwrap_or_else(|| ticket("ABC-12")),
        )
    }
    fn view(&self, scope: &str, _: &v::ViewCx<'_>) -> Option<v::ScreenView> {
        let on = self.subject("tickets", Pane::Middle).unwrap();
        let text = match self.pages.last() {
            Some(_) => format!("ticket page {}", on.label),
            None => "ABC-12 Review · ABC-9 Done".to_string(),
        };
        let mut view = v::ScreenView::new(scope, vec![v::PaneView::new(
            v::PaneId::Middle,
            "tickets",
            v::Node::Document { lines: vec![v::Text::plain(text)], cut: false },
        )]);
        view.actions = crate::actions(&self.rows());
        Some(view)
    }
    fn section_view(&self, scope: &str, _: &v::ViewCx<'_>) -> Option<v::SectionView> {
        Some(v::SectionView {
            version: v::VERSION,
            scope: scope.into(),
            body: v::Node::Document { lines: vec![], cut: false },
            detail_title: "about".into(),
            detail: vec![],
            detail_blocks: vec![],
            actions: vec![],
        })
    }
    fn intent(&mut self, _: &str, _: Pane, intent: &v::Intent) -> Outcome {
        match intent {
            v::Intent::Key { chord } if chord.key == v::Key::Esc && !self.pages.is_empty() => {
                self.pages.pop();
                Outcome::done()
            }
            // The merge request linked to ABC-12, by the shared vocabulary.
            v::Intent::Key { chord } if chord.key == v::Key::Char('e') => Outcome::Handled(vec![Effect::Show(
                Subject::new("merge-request", "acme/team-a/api!263", "api !263")
                    .with("project", "acme/team-a/api")
                    .with("iid", "263"),
            )]),
            _ => Outcome::Ignored,
        }
    }
    fn keys(&self, _: &str, _: Pane) -> Vec<Binding> {
        crate::bindings_of(self.rows())
    }
    fn contribute(&self, point: Point, subject: &Subject) -> Option<Contribution> {
        let (key, status, summary) = Self::of(subject)?;
        Some(match point {
            Point::Chips => {
                Contribution::Chips(vec![Chip::new("tracked", Tone::Muted)])
            }
            Point::Column => Contribution::Cell(
                radar_core::Cell::new("ticket", format!("{key} · {status}"))
                    .tone(Tone::Warn)
                    .show(ticket(key)),
            ),
            Point::Section => Contribution::Section {
                icon: None,
                title: format!("ticket {key}"),
                lines: vec![radar_core::view::Text::plain(format!("{status} · {summary}"))],
            },
            Point::Tab => Contribution::Tab {
                title: "Ticket".into(),
                rows: vec![Row {
                    line: radar_core::view::Text::plain(format!("{key}  {status}  {summary}")),
                    subject: Some(ticket(key)),
                }],
            },
            Point::Actions => Contribution::Actions(vec![
                Action::new('s', format!("start work on {key}…"), "start"),
                Action::new('o', format!("open {key} in the tracker"), "open"),
            ]),
            Point::Line => Contribution::Line(vec![Chip::new(format!("tracked as {key}"), Tone::Muted)]),
            Point::Mark => return None,
        })
    }
    fn act(&mut self, subject: &Subject, action: &str) -> Vec<Effect> {
        match action {
            "open" => vec![Effect::Show(ticket(Self::of(subject).map_or("", |t| t.0)))],
            _ => vec![Effect::Notify(format!("{action}: {}", subject.label))],
        }
    }
}

/// GitLab's sample next to the tracker, through the real host.
pub(crate) fn hosted_with_tracker() -> Host {
    let mut host = Host::new(
        vec![
            Box::new(crate::tests::sample()),
            Box::new(Tracker::default()),
        ],
        1_791_271_000,
    )
    .unwrap();
    host.load_config(Some("")).unwrap();
    host.plugins[0] = Box::new(crate::tests::sample());
    host
}

fn press(host: &mut Host, code: Key) {
    let _ = to_text(host, 160, 40);
    host.update(crate::key_event(Chord::new(code)));
}

/// !263 in api: the tree is b, worker, !342, a, api, !263.
fn on_263(host: &mut Host) {
    for _ in 0..5 {
        press(host, Key::Down);
    }
}

/// The tree's columns as the view holds them (the text form leaves out
/// their headings and widths).
fn columns(host: &Host, width: u16) -> Vec<v::ColumnSpec> {
    let view = host.screen_view_in(0, "repos", width, 40).expect("the screen");
    let v::Node::Tree(tree) = &view.pane(v::PaneId::Middle).expect("the tree").body else {
        panic!("the repositories tree")
    };
    tree.columns.clone()
}

#[test]
fn the_tree_carries_the_trackers_chips_and_column() {
    let mut host = hosted_with_tracker();
    // The ticket in a column of its own beside the state, which a renderer
    // drops whole when the room is short (`keep`, never half a word), and
    // the tracker's chip after the title.
    for width in [220, 80] {
        let ticket = columns(&host, width).into_iter().find(|c| c.heading == "ticket").expect("the tracker's column");
        assert!(ticket.extra && ticket.keep > 0, "{ticket:?}");
    }
    let text = to_text(&host, 160, 40);
    assert!(text.contains("[tracked]") && text.contains("ABC-12 · Review"), "{text}");
    insta::assert_snapshot!("tree_with_tracker", text);
    // A press on the cell shows the ticket, and selects its row. The cell
    // names the subject it shows.
    let view = host.screen_view(0, "repos").expect("the screen");
    let v::Node::Tree(tree) = &view.pane(v::PaneId::Middle).expect("the tree").body else {
        panic!("the repositories tree")
    };
    let (path, subject) = tree_paths(&tree.rows)
        .into_iter()
        .find_map(|(path, row)| {
            let cell = row.cells.iter().find(|cell| cell.show.is_some())?;
            (row.id.contains("!263")).then(|| (path, cell.show.clone().unwrap()))
        })
        .expect("the !263 row carries the ticket cell");
    let _ = to_text(&host, 160, 40);
    host.focus = radar_core::Pane::Middle;
    host.update(Event::Intent(v::Intent::Show { target: v::TreeRow::target_of(&path), subject }));
    let text = to_text(&host, 160, 40);
    assert_eq!(host.view, radar_core::host::View::Screen(1), "on the tracker's screen:\n{text}");
    assert!(text.contains("ABC-12"), "its page:\n{text}");
}

/// Every row of `rows` with its path.
fn tree_paths(rows: &[v::TreeRow]) -> Vec<(Vec<String>, &v::TreeRow)> {
    fn walk<'a>(rows: &'a [v::TreeRow], at: &mut Vec<String>, out: &mut Vec<(Vec<String>, &'a v::TreeRow)>) {
        for row in rows {
            at.push(row.id.clone());
            out.push((at.clone(), row));
            walk(&row.children, at, out);
            at.pop();
        }
    }
    let mut out = Vec::new();
    walk(rows, &mut Vec::new(), &mut out);
    out
}

#[test]
fn the_merge_request_page_has_the_trackers_section_and_tab() {
    let mut host = hosted_with_tracker();
    on_263(&mut host);
    press(&mut host, Key::Enter);
    let text = to_text(&host, 160, 40);
    assert!(text.contains("TICKET ABC-12"), "{text}");
    assert!(text.contains("Commits  Ticket"), "{text}");
    insta::assert_snapshot!("mr_page_with_tracker", text);
    for _ in 0..5 {
        press(&mut host, Key::Char(']'));
    }
    let text = to_text(&host, 160, 40);
    assert!(text.contains("Ticket>"), "{text}");
    insta::assert_snapshot!("mr_page_tracker_tab", text);
    press(&mut host, Key::Char('x'));
    insta::assert_snapshot!("mr_page_tracker_tab_keys", to_text(&host, 160, 40));
}

/// The target of the pinned Detail menu control, as the host displays it.
fn preview_menu_target(host: &Host) -> v::Target {
    let view = host.displayed_screen(160, 40).expect("the screen");
    let foot = view.pane(v::PaneId::Detail).and_then(|pane| pane.foot_actions.as_ref()).expect("the pinned actions");
    assert_eq!(foot.menu.action.label, "extra actions");
    foot.menu.target.clone().expect("the footer has a live target")
}

#[test]
fn another_parts_line_sits_under_the_title_and_its_actions_in_the_preview_act_on_a_press() {
    let mut host = hosted_with_tracker();
    on_263(&mut host);
    assert_eq!(host.focus, Pane::Middle, "the preview starts unfocused");
    let target = preview_menu_target(&host);
    let before = host.notices.through_id();
    host.update(Event::Intent(v::Intent::Context { target: target.clone() }));
    assert!(host.menu.is_none(), "a context press does not open or run the control");
    assert_eq!(host.focus, Pane::Middle);
    assert_eq!(host.notices.through_id(), before);
    host.update(Event::Intent(v::Intent::Activate { target }));
    assert_eq!(host.focus, Pane::Detail, "one press focuses the control's pane");
    assert_eq!(host.notices.through_id(), before, "opening the menu runs no contribution");
    let menu = &host.menu.as_ref().expect("the original contextual menu").menu;
    assert_eq!(menu.id, radar_core::host::KEYS_MENU);
    assert_eq!(menu.title, "api !263");
    let start = menu.items.iter().position(|item| item.label == "start work on ABC-12…").unwrap();
    assert_eq!(menu.items[start].keys, "s");
    assert!(menu.items.iter().any(|item| item.label == "Tracker" && item.heading));
    let collision = menu.items.iter().find(|item| item.label == "open ABC-12 in the tracker").unwrap();
    assert_eq!(collision.keys, "p", "the contribution does not displace GitLab's o: it takes a free letter");
    assert_eq!(collision.key, Some('p'));
    assert!(menu.items.iter().any(|item| item.keys == "o" && item.label == "open it in GitLab"));
    // The contributed row is in the menu the host shows, and a press on it
    // is its value chosen.
    let text = to_text(&host, 160, 40);
    assert!(text.contains("start work on ABC-12"), "no contributed menu row in:\n{text}");
    let value = host.menu.as_ref().unwrap().menu.items[start].value.clone();
    host.update(Event::Intent(v::Intent::Invoke { action: value }));
    assert!(host.menu.is_none());
    assert_eq!(host.message.as_deref(), Some("start: api !263"));
    assert_eq!(host.notices.since(before).map(|notice| notice.text.as_str()).collect::<Vec<_>>(),
        ["start: api !263"], "one activation dispatches the contribution exactly once");
    // Its page: the tracker's line under the title.
    press(&mut host, Key::Enter);
    let text = to_text(&host, 160, 40);
    assert!(text.contains("tracked as ABC-12"), "{text}");
    assert_eq!(host.plugins[0].depth("repos"), 1);
}

#[test]
fn contributed_preview_actions_without_an_icon_get_a_generic_action_icon() {
    let mut host = hosted_with_tracker();
    host.load_config(Some("[theme]\nicons = \"unicode\"\n")).unwrap();
    host.plugins[0] = Box::new(crate::tests::sample());
    on_263(&mut host);
    let target = preview_menu_target(&host);
    host.update(Event::Intent(v::Intent::Activate { target }));
    let menu = host.menu_view().expect("full menu shared by native and Ink");
    let action = menu.items.iter().find(|item| item.label == "start work on ABC-12…").unwrap();
    assert_eq!(action.icon, Some(radar_core::ui::icon::Icon::Action));
    assert_eq!(action.keys, "s");
    assert!(action.target.is_some());
}

#[test]
fn enter_on_the_ticket_tab_shows_it_and_esc_comes_back_to_the_page() {
    let mut host = hosted_with_tracker();
    on_263(&mut host);
    press(&mut host, Key::Enter);
    for _ in 0..5 {
        press(&mut host, Key::Char(']'));
    }
    press(&mut host, Key::Enter);
    assert_eq!(host.screen().unwrap().1.name, "tickets");
    assert!(to_text(&host, 160, 40).contains("ticket page ABC-12"));
    press(&mut host, Key::Esc);
    assert_eq!(host.screen().unwrap().1.name, "repositories");
    let text = to_text(&host, 160, 40);
    assert!(
        text.contains("/ api / !263") && text.contains("Ticket>"),
        "{text}"
    );
}

#[test]
fn o_and_y_on_another_parts_tab_act_on_the_rows_link() {
    let mut host = hosted_with_tracker();
    on_263(&mut host);
    press(&mut host, Key::Enter);
    let open = |host: &mut Host, c: char| {
        let _ = to_text(host, 160, 40);
        host.update(crate::key_event(Chord::char(c)))
    };
    // On GitLab's own tabs, o is the merge request.
    let fx = open(&mut host, 'o');
    assert!(
        matches!(&fx[..], [radar_core::host::HostEffect::Open(u)] if u.contains("merge_requests/263")),
        "{fx:?}"
    );
    for _ in 0..5 {
        press(&mut host, Key::Char(']'));
    }
    let fx = open(&mut host, 'o');
    assert!(
        matches!(&fx[..], [radar_core::host::HostEffect::Open(u)] if u == "https://tracker.example/ABC-12"),
        "{fx:?}"
    );
    let fx = open(&mut host, 'y');
    assert!(
        matches!(&fx[..], [radar_core::host::HostEffect::Copy(u)] if u == "https://tracker.example/ABC-12"),
        "{fx:?}"
    );
    press(&mut host, Key::Char('x'));
    let labels: Vec<String> = host
        .menu
        .as_ref()
        .unwrap()
        .menu
        .items
        .iter()
        .map(|i| format!("{} {}", i.keys, i.label))
        .collect();
    assert!(labels.contains(&"o open ABC-12's link".to_string()), "{labels:?}");
    assert!(labels.contains(&"y copy ABC-12's link".to_string()), "{labels:?}");
}

#[test]
fn the_tracker_shows_a_merge_request_and_esc_returns_to_it() {
    let mut host = hosted_with_tracker();
    press(&mut host, Key::Char('T'));
    press(&mut host, Key::Char('e'));
    assert_eq!(host.screen().unwrap().1.name, "repositories");
    let text = to_text(&host, 160, 40);
    assert!(text.contains("Repositories / a / api / !263"), "{text}");
    press(&mut host, Key::Char('x'));
    // The way back is the pane's: `x` again, inside the row menu.
    press(&mut host, Key::Char('x'));
    let menu = to_text(&host, 160, 40);
    assert!(menu.contains("back to tickets"), "{menu}");
    press(&mut host, Key::Esc);
    press(&mut host, Key::Esc);
    assert_eq!(host.screen().unwrap().1.name, "tickets");
    assert_eq!(host.plugins[0].depth("repos"), 0, "GitLab's tree as it was");
}

#[test]
fn a_contributed_action_with_gitlabs_key_is_chosen_from_the_menu_only() {
    let mut host = hosted_with_tracker();
    on_263(&mut host);
    press(&mut host, Key::Char('x'));
    let menu = &host.menu.as_ref().unwrap().menu;
    let row = |label: &str| menu.items.iter().find(|i| i.label == label).cloned();
    assert_eq!(row("start work on ABC-12…").unwrap().keys, "s");
    assert_eq!(
        row("open ABC-12 in the tracker").unwrap().keys,
        "p",
        "o is GitLab's: the contribution takes the first free letter of its words"
    );
    assert!(row("Tracker").unwrap().heading);
    // o still opens the merge request in GitLab.
    press(&mut host, Key::Esc);
    let fx = host.update(crate::key_event(Chord::char('o')));
    assert!(
        matches!(&fx[..], [radar_core::host::HostEffect::Open(u)] if u.contains("merge_requests/263")),
        "{fx:?}"
    );
    // s, which GitLab does not use on a tree row, is the tracker's.
    press(&mut host, Key::Char('s'));
    assert_eq!(host.message.as_deref(), Some("start: api !263"));
    // Choosing the keyless row shows the ticket.
    press(&mut host, Key::Char('x'));
    let at = host
        .menu
        .as_ref()
        .unwrap()
        .menu
        .items
        .iter()
        .position(|i| i.label == "open ABC-12 in the tracker")
        .unwrap();
    host.menu.as_mut().unwrap().row = at;
    press(&mut host, Key::Enter);
    assert_eq!(host.view, View::Screen(1));
}

#[test]
fn a_merge_request_with_no_ticket_gets_nothing_from_the_tracker() {
    let mut host = hosted_with_tracker();
    // !342 in worker: no ticket key in its title.
    press(&mut host, Key::Down);
    press(&mut host, Key::Down);
    press(&mut host, Key::Char('x'));
    let labels: Vec<String> = host
        .menu
        .as_ref()
        .unwrap()
        .menu
        .items
        .iter()
        .map(|i| i.label.clone())
        .collect();
    assert_eq!(host.menu.as_ref().unwrap().menu.title, "worker !342");
    assert!(!labels.contains(&"Tracker".to_string()), "{labels:?}");
}
