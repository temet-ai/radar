//! Parts joining through the host, with three stand-in parts that know
//! nothing of each other: `shelf` owns books and has a page per book;
//! `notes` owns notes and adds to books; `bell` owns nothing and adds
//! actions and chips.

use std::sync::{Arc, Mutex};

use crate::view::{Chord, Key};

use super::*;
use crate::host::{HostEffect, Owner, View};
use crate::plugin::{Binding, Effect, Outcome, Pane, ScreenSpec, SectionSpec, Tone};

/// What the host shows, as the view text a frontend is given.
fn to_text(host: &Host, width: u16, height: u16) -> String {
    crate::headless::host_text_of(host, width, height)
}
use crate::view as v;
use crate::{Host, Plugin};

type Log = Arc<Mutex<Vec<String>>>;

const BOOKS: &str = "https://shelf.example/books/";

fn book(n: u64) -> Subject {
    Subject::new("book", n.to_string(), format!("book {n}"))
        .with_url(format!("{BOOKS}{n}"))
        .with("title", "A stats endpoint")
}

fn note(key: &str) -> Subject {
    Subject::new("note", key, key)
}

/// Owns books: a list of one, and a page per shown one.
struct Shelf {
    pages: Vec<Subject>,
    log: Log,
    /// Its rows say what they are for, one is asked for now, one is the pane's.
    grouped: bool,
}

impl Plugin for Shelf {
    fn id(&self) -> &'static str {
        "shelf"
    }
    fn name(&self) -> &'static str {
        "Shelf"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn owns(&self) -> Vec<Owned> {
        vec![Owned::page("book", "books")]
    }
    fn offers(&self) -> Vec<Offer> {
        vec![Offer::new(Point::Chips, "book", "book rows")]
    }
    fn recognise(&self, text: &str) -> Vec<(Range<usize>, Subject)> {
        text.find(BOOKS)
            .map(|at| {
                let end = text[at..]
                    .find(char::is_whitespace)
                    .map_or(text.len(), |n| at + n);
                let n = text[at + BOOKS.len()..end].parse().unwrap();
                (at..end, book(n))
            })
            .into_iter()
            .collect()
    }
    fn show(&mut self, subject: &Subject) -> Showing {
        self.pages.push(subject.clone());
        Showing::Page {
            screen: "main",
            effects: vec![],
        }
    }
    fn depth(&self, _: &str) -> usize {
        self.pages.len()
    }
    fn subject(&self, _: &str, _: Pane) -> Option<Subject> {
        Some(self.pages.last().cloned().unwrap_or(book(1)))
    }
    fn related(&self, subject: &Subject) -> Vec<Subject> {
        if subject.is("book") {
            vec![Subject::new("shelf", "main", "main shelf")]
        } else {
            vec![]
        }
    }
    fn screens(&self) -> Vec<ScreenSpec> {
        vec![ScreenSpec {
            id: "main",
            name: "shelf",
            key: 'B',
        }]
    }
    fn view(&self, scope: &str, cx: &v::ViewCx<'_>) -> Option<v::ScreenView> {
        let s = self.subject("main", Pane::Middle).unwrap();
        let mut title = s.label.clone();
        for c in cx.parts.chips(&s) {
            title.push_str(&format!(" [{}]", c.text));
        }
        let mut lines = vec![v::Text::plain(title)];
        lines.extend(cx.parts.sections(&s).into_iter().map(|(title, _)| v::Text::plain(title)));
        let mut view = v::ScreenView::new(scope, vec![v::PaneView::new(
            v::PaneId::Middle, "shelf", v::Node::Document { lines, cut: false },
        )]);
        view.actions = self.keys(scope, Pane::Middle).iter()
            .map(|b| v::ActionSpec::named(format!("shelf.{scope}.{}", b.keys), b)).collect();
        Some(view)
    }
    fn intent(&mut self, _: &str, _: Pane, intent: &v::Intent) -> Outcome {
        let v::Intent::Key { chord } = intent else { return Outcome::Ignored };
        match chord.key {
            v::Key::Esc if !self.pages.is_empty() => {
                self.pages.pop();
                Outcome::done()
            }
            v::Key::Char('o') => {
                self.log.lock().unwrap().push("shelf o".into());
                Outcome::done()
            }
            // A deeper page of its own.
            v::Key::Char('n') => {
                self.pages.push(book(9));
                Outcome::done()
            }
            v::Key::Char('t') => Outcome::Handled(vec![Effect::Show(note("N-1"))]),
            v::Key::Char('u') => {
                Outcome::Handled(vec![Effect::Show(Subject::link(format!("{BOOKS}2")))])
            }
            v::Key::Char('v') => Outcome::Handled(vec![Effect::Show(Subject::link(
                "https://elsewhere.example/x",
            ))]),
            // Its app's own link, else the web one.
            v::Key::Char('p') => Outcome::Handled(vec![Effect::OpenApp {
                link: "shelf://book?id=2".into(),
                web: format!("{BOOKS}2"),
            }]),
            _ => Outcome::Ignored,
        }
    }
    fn keys(&self, _: &str, _: Pane) -> Vec<Binding> {
        let mut b = vec![
            Binding::heading(self.subject("main", Pane::Middle).unwrap().label),
            Binding::new("o", "open it"),
        ];
        if self.grouped {
            b.push(Binding::new("m", "mend it").group(v::ActionGroup::Change));
            b.push(Binding::new("p", "see its pages").hint("3 pages").lead());
            b.push(Binding::new("r", "refresh the shelf").pane());
        }
        if !self.pages.is_empty() {
            b.push(Binding::new("esc", "back to the list"));
        }
        b
    }
}

/// Owns notes; adds chips, a column, a section, a tab and actions to books.
struct Notes {
    ready: bool,
    pages: Vec<Subject>,
    log: Log,
}

impl Plugin for Notes {
    fn id(&self) -> &'static str {
        "notes"
    }
    fn name(&self) -> &'static str {
        "Notes"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn set_up(&self) -> Setup {
        if self.ready {
            Setup::Ready
        } else {
            Setup::Needed("Add a notebook to follow.".into())
        }
    }
    fn owns(&self) -> Vec<Owned> {
        vec![Owned::page("note", "notes")]
    }
    fn adds(&self) -> Vec<Adds> {
        vec![
            Adds::new("book", "note chips"),
            // No part here owns cards: never mentioned.
            Adds::new("card", "note links"),
        ]
    }
    fn recognise(&self, text: &str) -> Vec<(Range<usize>, Subject)> {
        text.find("N-")
            .map(|at| (at..at + 3, note(&text[at..at + 3])))
            .into_iter()
            .collect()
    }
    fn show(&mut self, subject: &Subject) -> Showing {
        self.pages.push(subject.clone());
        Showing::Page {
            screen: "main",
            effects: vec![],
        }
    }
    fn depth(&self, _: &str) -> usize {
        self.pages.len()
    }
    fn badge(&self, _: &str) -> Option<(String, Tone)> {
        Some(("7".into(), Tone::Warn))
    }
    fn screens(&self) -> Vec<ScreenSpec> {
        vec![ScreenSpec {
            id: "main",
            name: "notes",
            key: 'N',
        }]
    }
    fn sections(&self) -> Vec<SectionSpec> {
        vec![SectionSpec {
            id: "notebooks",
            name: "notebooks",
            group: "notes",
        }]
    }
    fn view(&self, scope: &str, _: &v::ViewCx<'_>) -> Option<v::ScreenView> {
        let label = self.pages.last().map_or("notes".to_string(), |page| format!("note {}", page.label));
        Some(v::ScreenView::new(scope, vec![v::PaneView::new(
            v::PaneId::Middle, "notes",
            v::Node::Document { lines: vec![v::Text::plain(label)], cut: false },
        )]))
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
        let v::Intent::Key { chord } = intent else { return Outcome::Ignored };
        match chord.key {
            v::Key::Esc if !self.pages.is_empty() => {
                self.pages.pop();
                Outcome::done()
            }
            v::Key::Char('e') => Outcome::Handled(vec![Effect::Show(book(1))]),
            _ => Outcome::Ignored,
        }
    }
    fn contribute(&self, point: Point, subject: &Subject) -> Option<Contribution> {
        if !subject.is("book") {
            return None;
        }
        Some(match point {
            Point::Chips => Contribution::Chips(vec![Chip::new("N-1 · open", Tone::Warn)]),
            Point::Column => Contribution::Cell(
                Cell::new("note", "N-1 · open")
                    .tone(Tone::Warn)
                    .show(Subject::new("note", "N-1", "N-1")),
            ),
            Point::Section => Contribution::Section {
                icon: Some(crate::ui::icon::Icon::File),
                title: "Note N-1".into(),
                lines: std::iter::once(crate::view::Text::plain(subject.field("title").unwrap_or("")))
                    .chain(
                        (1..subject.field("lines").and_then(|n| n.parse().ok()).unwrap_or(1))
                            .map(|i| crate::view::Text::plain(format!("line {i}"))),
                    )
                    .collect(),
            },
            Point::Tab => Contribution::Tab {
                title: "Notes".into(),
                rows: vec![Row {
                    line: crate::view::Text::plain("N-1 about this book"),
                    subject: Some(note("N-1")),
                }],
            },
            Point::Actions => Contribution::Actions(vec![
                Action::new('s', "start a note", "start"),
                Action::new('o', "open N-1", "open"),
            ]),
            Point::Line => Contribution::Line(vec![
                Chip::new("noted in ", Tone::Muted),
                Chip::new("N-1", Tone::Warn),
            ]),
            Point::Mark => Contribution::Mark {
                icon: crate::ui::icon::Icon::Comment,
                words: vec![Chip::new("noted", Tone::Muted)],
            },
        })
    }
    fn act(&mut self, subject: &Subject, action: &str) -> Vec<Effect> {
        self.log
            .lock()
            .unwrap()
            .push(format!("notes {action} {}", subject.label));
        vec![Effect::Notify(format!("{action} done"))]
    }
}

/// Owns nothing; adds actions and chips to anything.
struct Bell {
    log: Log,
}

impl Plugin for Bell {
    fn id(&self) -> &'static str {
        "bell"
    }
    fn name(&self) -> &'static str {
        "Bell"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn contribute(&self, point: Point, _: &Subject) -> Option<Contribution> {
        match point {
            Point::Chips => Some(Contribution::Chips(vec![Chip::new("rang 3d", Tone::Muted)])),
            Point::Actions => Some(Contribution::Actions(vec![
                Action::new('s', "ring for it", "ring"),
                Action::new('a', "announce it", "announce"),
                Action::new('j', "jump", "jump"),
            ])),
            _ => None,
        }
    }
    fn act(&mut self, subject: &Subject, action: &str) -> Vec<Effect> {
        self.log
            .lock()
            .unwrap()
            .push(format!("bell {action} {}", subject.label));
        // Jumping presses another key, as a row drawn for an action does.
        match action {
            "jump" => vec![Effect::Press('a')],
            _ => vec![],
        }
    }
}

/// Cached relations, independent of any subject owner or drawing point.
struct Relations {
    id: &'static str,
    ready: bool,
    subjects: Vec<Subject>,
}

impl Plugin for Relations {
    fn id(&self) -> &'static str {
        self.id
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn set_up(&self) -> Setup {
        if self.ready {
            Setup::Ready
        } else {
            Setup::Needed("Choose a source.".into())
        }
    }
    fn related(&self, subject: &Subject) -> Vec<Subject> {
        assert!(self.ready, "a part not set up must not be asked");
        assert_eq!(subject, &book(1), "only the given subject is queried");
        self.subjects.clone()
    }
}

#[derive(Debug, PartialEq)]
struct RelatedAction {
    subject: Subject,
    action: String,
    related: Vec<Subject>,
}

/// Acts on books using cached relations from the other parts.
struct Reader {
    actions: Arc<Mutex<Vec<RelatedAction>>>,
}

impl Plugin for Reader {
    fn id(&self) -> &'static str {
        "reader"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn contribute(&self, point: Point, subject: &Subject) -> Option<Contribution> {
        (point == Point::Actions && subject.is("book")).then(|| {
            Contribution::Actions(vec![
                Action::new('a', "read related", "read"),
                // The owner keeps o; this row can only be chosen in the menu.
                Action::new('o', "open related", "open"),
            ])
        })
    }
    fn related(&self, _: &Subject) -> Vec<Subject> {
        panic!("the action recipient must not be asked for its own relations")
    }
    fn act_with_related(
        &mut self,
        subject: &Subject,
        action: &str,
        related: &[Subject],
    ) -> Vec<Effect> {
        assert_eq!(subject.related, related, "both action interfaces carry the same references");
        self.actions.lock().unwrap().push(RelatedAction {
            subject: subject.clone(),
            action: action.into(),
            related: related.to_vec(),
        });
        vec![Effect::Notify(format!("{action} related done"))]
    }
}

/// The host with the parts named, in that order: `s`helf, `n`otes (`N`
/// when not set up), `b`ell.
fn host(which: &str) -> (Host, Log) {
    let log: Log = Arc::default();
    let plugins: Vec<Box<dyn Plugin>> = which
        .chars()
        .map(|c| -> Box<dyn Plugin> {
            match c {
                's' | 'g' => Box::new(Shelf {
                    pages: vec![],
                    log: log.clone(),
                    grouped: c == 'g',
                }),
                'd' => Box::new(Desk),
                'n' | 'N' => Box::new(Notes {
                    ready: c == 'n',
                    pages: vec![],
                    log: log.clone(),
                }),
                _ => Box::new(Bell { log: log.clone() }),
            }
        })
        .collect();
    let mut host = Host::new(plugins, 0).unwrap();
    host.load_config(Some("")).unwrap();
    (host, log)
}

fn press(host: &mut Host, code: Key) -> Vec<HostEffect> {
    host.update(crate::host::chord_event(Chord::new(code)))
}

fn screen_name(host: &Host) -> &'static str {
    host.screen().map_or("settings", |(_, s)| s.name)
}

fn labels(host: &Host) -> Vec<String> {
    host.menu
        .as_ref()
        .map(|m| m.menu.items.iter().map(|i| i.label.clone()).collect())
        .unwrap_or_default()
}

#[test]
fn show_goes_to_the_owner_and_esc_comes_back() {
    let (mut host, _) = host("sn");
    press(&mut host, Key::Char('N'));
    host.focus = Pane::Nav;
    press(&mut host, Key::Char('e'));
    assert_eq!(screen_name(&host), "shelf", "shown by its owner");
    assert_eq!(host.focus, Pane::Middle);
    assert_eq!(host.plugins[0].depth("main"), 1);
    // A page of the owner's own on top: esc pops it, and stays.
    press(&mut host, Key::Char('n'));
    press(&mut host, Key::Esc);
    assert_eq!(screen_name(&host), "shelf");
    // Past the shown page: back where the show came from.
    press(&mut host, Key::Esc);
    assert_eq!(screen_name(&host), "notes");
    assert_eq!(host.focus, Pane::Nav, "focus as it was");
    assert!(host.back.is_empty());
}

#[test]
fn the_x_menu_says_where_esc_goes_back_to() {
    let (mut host, _) = host("sn");
    press(&mut host, Key::Char('N'));
    press(&mut host, Key::Char('e'));
    // Going back is the pane's, not the row's: `x` again.
    press(&mut host, Key::Char('x'));
    press(&mut host, Key::Char('x'));
    assert!(
        labels(&host).contains(&"back to notes".to_string()),
        "{:?}",
        labels(&host)
    );
}

#[test]
fn switching_screens_forgets_the_way_back() {
    let (mut host, _) = host("sn");
    press(&mut host, Key::Char('N'));
    press(&mut host, Key::Char('e'));
    press(&mut host, Key::Char('N'));
    assert!(host.back.is_empty());
}

#[test]
fn a_link_an_owner_recognises_is_shown_as_what_it_is() {
    let (mut host, _) = host("sn");
    let fx = press(&mut host, Key::Char('u'));
    assert!(fx.is_empty(), "nothing opened in a browser: {fx:?}");
    assert_eq!(host.plugins[0].depth("main"), 1, "its page is open");
    assert_eq!(host.plugins[0].subject("main", Pane::Middle), Some(book(2)));
}

#[test]
fn with_no_owner_a_link_opens_and_anything_else_says_so() {
    let (mut host, _) = host("s");
    // A note, with no part here that owns notes.
    let fx = press(&mut host, Key::Char('t'));
    assert!(fx.is_empty());
    assert_eq!(host.message.as_deref(), Some("cannot open N-1 here"));
    // A link no part recognises opens as a link.
    let fx = press(&mut host, Key::Char('v'));
    assert!(
        matches!(&fx[..], [HostEffect::Open(u)] if u == "https://elsewhere.example/x"),
        "{fx:?}"
    );
    assert_eq!(
        host.message.as_deref(),
        Some("opening https://elsewhere.example/x")
    );
}

#[test]
fn an_app_link_reaches_the_runner_with_its_web_link_beside_it() {
    let (mut host, _) = host("s");
    let fx = press(&mut host, Key::Char('p'));
    assert!(
        matches!(&fx[..], [HostEffect::OpenApp { link, web }]
            if link == "shelf://book?id=2" && *web == format!("{BOOKS}2")),
        "{fx:?}"
    );
}

#[test]
fn a_part_not_set_up_has_a_set_up_page_and_adds_nothing() {
    let (mut host, log) = host("sN");
    // Nothing of it on the book: no chips, no section, no actions.
    let text = to_text(&host, 80, 24);
    assert!(!text.contains("N-1"), "{text}");
    assert!(!text.contains(" 7 "), "no badge either:\n{text}");
    press(&mut host, Key::Char('x'));
    assert!(
        !labels(&host).contains(&"Notes".to_string()),
        "{:?}",
        labels(&host)
    );
    press(&mut host, Key::Esc);
    // Showing a note does not reach it.
    press(&mut host, Key::Char('t'));
    assert_eq!(host.message.as_deref(), Some("cannot open N-1 here"));
    assert!(log.lock().unwrap().is_empty());
    // Its screen: the set-up page; ⏎ goes to its settings.
    press(&mut host, Key::Char('N'));
    let text = to_text(&host, 80, 24);
    assert!(text.contains("Notes is not set up yet."), "{text}");
    assert!(text.contains("Add a notebook to follow."), "{text}");
    assert!(!text.contains("PgUp"), "nothing to scroll:\n{text}");
    press(&mut host, Key::Char('x'));
    assert_eq!(labels(&host), ["set up Notes in settings"]);
    press(&mut host, Key::Enter);
    assert_eq!(host.view, View::Settings);
    assert_eq!(host.current_section().unwrap().1.name, "notebooks");
}

#[test]
fn a_detail_pane_shows_a_contributed_section_marked_and_cut_to_its_share() {
    let (host, _) = host("snb");
    let parts = Parts::new(&host.plugins, Some(0));
    let short = parts.detail_sections(&book(1));
    let crate::view::Block::Section { mark, title, blocks, .. } = &short[0] else { panic!("{short:?}") };
    assert_eq!((*mark, title.as_str(), blocks.len()), (Some(crate::ui::icon::Icon::File), "NOTE N-1", 1));
    let long = book(1).with("lines", "14");
    let cut = parts.detail_sections(&long);
    let crate::view::Block::Section { blocks, .. } = &cut[0] else { panic!("{cut:?}") };
    assert_eq!(blocks.len(), crate::compose::DETAIL_SECTION_MOST);
    let last = match blocks.last() {
        Some(crate::view::Block::Line { text, .. }) => text.words(),
        other => panic!("{other:?}"),
    };
    assert_eq!(last, "⋯ 5 more lines on its page");
    // The page keeps every line.
    assert_eq!(parts.sections(&long)[0].1.len(), 14);
}

#[test]
fn contributions_come_in_plugin_order_and_never_from_the_drawer() {
    let (host, _) = host("snb");
    let parts = Parts::new(&host.plugins, Some(0));
    let chips: Vec<String> = parts.chips(&book(1)).into_iter().map(|c| c.text).collect();
    assert_eq!(chips, ["N-1 · open", "rang 3d"]);
    let names: Vec<&str> = parts
        .contributions(Point::Actions, &book(1))
        .iter()
        .map(|(n, _)| *n)
        .collect();
    assert_eq!(names, ["Notes", "Bell"]);
    let from_notes = Parts::new(&host.plugins, Some(1));
    assert_eq!(from_notes.chips(&book(1)).len(), 1, "its own are not asked");
    let cells = parts.cells(&book(1));
    assert_eq!(cells.len(), 1);
    // A cell carries its feeling and what clicking it shows.
    assert_eq!((cells[0].heading.as_str(), cells[0].tone), ("note", Tone::Warn));
    assert_eq!(cells[0].show.as_ref().map(|s| s.id.as_str()), Some("N-1"));
    assert_eq!(cells[0].text, "N-1 · open");
    let sections = parts.sections(&book(1));
    assert_eq!(sections[0].0, "Note N-1");
    assert_eq!(
        sections[0].1[0],
        crate::view::Text::plain("A stats endpoint"),
        "matched on a field"
    );
    assert_eq!(parts.tabs(&book(1))[0].1[0].subject, Some(note("N-1")));
    let text = to_text(&host, 80, 24);
    assert!(text.contains("book 1 [N-1 · open] [rang 3d]"), "{text}");
    assert!(text.contains("Note N-1"), "{text}");
}

#[test]
fn related_subjects_come_from_ready_others_once_per_kind_and_id() {
    let first = note("N-1")
        .with_url("https://notes.example/1")
        .with("title", "first");
    let plugins: Vec<Box<dyn Plugin>> = vec![
        Box::new(Relations {
            id: "first",
            ready: true,
            subjects: vec![first.clone(), note("N-1"), note("N-2")],
        }),
        Box::new(Relations {
            id: "self",
            ready: true,
            subjects: vec![note("own")],
        }),
        Box::new(Relations {
            id: "unconfigured",
            ready: false,
            subjects: vec![note("unconfigured")],
        }),
        Box::new(Relations {
            id: "last",
            ready: true,
            subjects: vec![
                Subject::new("note", "N-1", "a later label").with("title", "later"),
                Subject::new("book", "N-1", "a different kind"),
            ],
        }),
    ];
    let related = Parts::new(&plugins, Some(1)).related(&book(1));
    assert_eq!(
        related,
        [first.clone(), note("N-2"), Subject::new("book", "N-1", "")]
    );
    assert_eq!(related[0].label, first.label);
    assert_eq!(related[0].url, first.url);
    assert_eq!(
        related[0].fields, first.fields,
        "the first provider's fields are kept"
    );
}

#[test]
fn no_relations_are_the_default_and_absent_parts_add_none() {
    assert!(Parts::none().related(&book(1)).is_empty());
    let (host, _) = host("b");
    assert!(Parts::new(&host.plugins, None).related(&book(1)).is_empty());
    let plugins: Vec<Box<dyn Plugin>> = vec![Box::new(Relations {
        id: "unconfigured",
        ready: false,
        subjects: vec![note("N-1")],
    })];
    assert!(Parts::new(&plugins, None).related(&book(1)).is_empty());
}

#[test]
fn contributed_keys_and_menu_choices_receive_the_other_parts_relations() {
    let actions = Arc::default();
    let mut host = Host::new(
        vec![
            Box::new(Shelf {
                pages: vec![],
                log: Arc::default(),
                grouped: false,
            }),
            Box::new(Relations {
                id: "relations",
                ready: true,
                subjects: vec![note("N-1"), note("N-1")],
            }),
            Box::new(Relations {
                id: "unconfigured",
                ready: false,
                subjects: vec![note("unconfigured")],
            }),
            Box::new(Reader {
                actions: Arc::clone(&actions),
            }),
        ],
        0,
    )
    .unwrap();
    host.load_config(Some("")).unwrap();

    // On the screen: a contributed key gets both the owner's relation
    // and the other provider's, and its effects still go through the host.
    press(&mut host, Key::Char('a'));
    assert_eq!(host.message.as_deref(), Some("read related done"));

    // A keyless row takes a free letter of its words in the menu, and
    // takes the same path.
    press(&mut host, Key::Char('x'));
    let at = labels(&host)
        .iter()
        .position(|l| l == "open related")
        .unwrap();
    assert_eq!(host.menu.as_ref().unwrap().menu.items[at].key, Some('p'));
    host.menu.as_mut().unwrap().row = at;
    press(&mut host, Key::Enter);
    assert_eq!(host.message.as_deref(), Some("open related done"));

    let related = vec![Subject::new("shelf", "main", "main shelf"), note("N-1")];
    assert_eq!(
        *actions.lock().unwrap(),
        [
            RelatedAction {
                subject: book(1),
                action: "read".into(),
                related: related.clone()
            },
            RelatedAction {
                subject: book(1),
                action: "open".into(),
                related
            },
        ]
    );
}

#[test]
fn related_actions_keep_legacy_act_as_the_default() {
    let (mut host, log) = host("n");
    let fx = host.plugins[0].act_with_related(&book(1), "start", &[note("N-1")]);
    assert_eq!(*log.lock().unwrap(), ["notes start book 1"]);
    assert!(matches!(&fx[..], [Effect::Notify(text)] if text == "start done"));
}

#[test]
fn the_action_check_uses_the_related_hook_with_no_other_parts() {
    let actions = Arc::default();
    let make = || {
        Box::new(Reader {
            actions: Arc::clone(&actions),
        }) as Box<dyn Plugin>
    };
    let mut check = crate::check::Check::new(&make);
    assert_eq!(check.actions(&book(1)), 2);
    assert!(check.gaps.is_empty(), "{:?}", check.gaps);
    let actions = actions.lock().unwrap();
    assert_eq!(actions.len(), 2);
    assert!(actions.iter().all(|a| a.related.is_empty()));
}

#[test]
fn contributed_actions_sit_under_their_part_and_take_a_free_letter_for_keys_others_use() {
    let (mut host, log) = host("snb");
    press(&mut host, Key::Char('x'));
    let rows: Vec<(String, String, bool, Option<char>)> = host
        .menu
        .as_ref()
        .unwrap()
        .menu
        .items
        .iter()
        .map(|i| (i.keys.clone(), i.label.clone(), i.heading, i.key))
        .collect();
    let want: Vec<(String, String, bool, Option<char>)> = [
        ("o", "open it", false, Some('o')),
        ("", "Notes", true, None),
        ("s", "start a note", false, Some('s')),
        // o is the owner's: the menu gives it the next free letter of its words.
        ("p", "open N-1", false, Some('p')),
        ("", "Bell", true, None),
        // s went to Notes, which comes first.
        ("r", "ring for it", false, Some('r')),
        ("a", "announce it", false, Some('a')),
        // j moves in lists: never a contributed key.
        ("u", "jump", false, Some('u')),
    ]
    .into_iter()
    .map(|(k, l, h, c)| (k.to_string(), l.to_string(), h, c))
    .collect();
    assert_eq!(rows, want);
    assert_eq!(host.menu.as_ref().unwrap().menu.title, "book 1");

    // Chosen from the menu: the keyless row reaches its part, with the subject.
    let at = rows.iter().position(|r| r.1 == "open N-1").unwrap();
    host.menu.as_mut().unwrap().row = at;
    let fx = press(&mut host, Key::Enter);
    assert!(fx.is_empty());
    assert_eq!(host.message.as_deref(), Some("open done"));
    // Typed in the menu.
    press(&mut host, Key::Char('x'));
    press(&mut host, Key::Char('s'));
    // Pressed on the screen, when the owner does not use the key.
    press(&mut host, Key::Char('a'));
    // The owner's own key stays the owner's.
    press(&mut host, Key::Char('o'));
    assert_eq!(
        *log.lock().unwrap(),
        [
            "notes open book 1",
            "notes start book 1",
            "bell announce book 1",
            "shelf o"
        ]
    );
}

#[test]
fn an_owner_lists_the_actions_as_the_menu_keeps_them_and_a_press_does_what_the_key_does() {
    let (mut host, log) = host("snb");
    let parts = Parts::new(&host.plugins, Some(0));
    let lines = parts.lines(&book(1));
    assert_eq!(lines[0].0, "Notes");
    let words: Vec<&str> = lines[0].1.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(words.concat(), "noted in N-1");
    assert_eq!(words.iter().map(|w| w.chars().count()).collect::<Vec<_>>(), [9, 3], "one chip each, in order");
    let (icon, words) = parts.mark(&book(1)).expect("a mark");
    assert_eq!((icon, words[0].text.as_str()), (crate::ui::icon::Icon::Comment, "noted"));
    let keys: Vec<(&str, String, Option<char>)> = parts
        .actions(&book(1), &['o'])
        .into_iter()
        .flat_map(|(name, actions)| actions.into_iter().map(move |a| (name, a.what, a.key)))
        .collect();
    assert_eq!(
        keys,
        [
            ("Notes", "start a note".to_string(), Some('s')),
            ("Notes", "open N-1".to_string(), None),
            ("Bell", "ring for it".to_string(), None),
            ("Bell", "announce it".to_string(), Some('a')),
            ("Bell", "jump".to_string(), None),
        ]
    );
    // An effect that presses a key: as typed, so a's action runs.
    press(&mut host, Key::Char('x'));
    let at = labels(&host).iter().position(|l| l == "jump").unwrap();
    host.menu.as_mut().unwrap().row = at;
    press(&mut host, Key::Enter);
    assert_eq!(*log.lock().unwrap(), ["bell jump book 1", "bell announce book 1"]);
    // A chip's action, done by the part that added it, by its name.
    let fx = host.route(0, vec![Effect::Act { part: "Bell", subject: book(1), id: "ring".into() }], 0);
    assert!(fx.is_empty());
    assert_eq!(log.lock().unwrap().last().map(String::as_str), Some("bell ring book 1"));
}

/// Adds to books: actions that say what they are for, and one that does not.
struct Desk;

impl Plugin for Desk {
    fn id(&self) -> &'static str {
        "desk"
    }
    fn name(&self) -> &'static str {
        "Desk"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn contribute(&self, point: Point, _: &Subject) -> Option<Contribution> {
        match point {
            Point::Actions => Some(Contribution::Actions(vec![
                Action::new('c', "check it out", "check").hint("2 behind").group(v::ActionGroup::Local),
                Action::new('t', "ask about it", "ask").group(v::ActionGroup::Ask),
                Action::new('d', "dust it", "dust"),
            ])),
            _ => None,
        }
    }
}

#[test]
fn a_rows_menu_groups_by_what_doing_it_is_for_and_leaves_the_panes_actions_to_the_pane() {
    let (mut host, _) = host("gd");
    press(&mut host, Key::Char('x'));
    let menu = &host.menu.as_ref().unwrap().menu;
    let rows: Vec<(&str, &str, &str)> =
        menu.items.iter().map(|i| (i.keys.as_str(), i.label.as_str(), i.hint.as_str())).collect();
    assert_eq!(
        rows,
        [
            ("o", "open it", ""),
            // What the row's state asks for follows the default action; its fact is at the right.
            ("p", "see its pages", "3 pages"),
            ("", "change it", ""),
            ("m", "mend it", ""),
            // Another part's grouped actions join the owner's headings.
            ("", "work on it here", ""),
            ("c", "check it out", "2 behind"),
            ("", "ask someone", ""),
            ("t", "ask about it", ""),
            // One with no group stays under its part's name.
            ("", "Desk", ""),
            ("d", "dust it", ""),
        ]
    );
    // `x` again: the pane's, with the shelf's refresh and the session.
    press(&mut host, Key::Char('x'));
    assert_eq!(
        labels(&host),
        ["refresh the shelf", "this session", "recent notices", "activity history"]
    );
}

fn rows_of(host: &Host) -> Vec<(String, String, String)> {
    host.menu.as_ref().unwrap().menu.items.iter().map(|i| (i.keys.clone(), i.label.clone(), i.hint.clone())).collect()
}

fn select(host: &mut Host, label: &str) {
    let open = host.menu.as_mut().unwrap();
    open.row = open.menu.items.iter().position(|i| i.label == label).unwrap();
}

#[test]
fn the_gear_makes_a_rows_menu_its_own_editor_and_keeps_what_changed_for_its_kind_of_place() {
    let (mut host, _) = host("gd");
    press(&mut host, Key::Char('x'));
    assert!(host.menu.as_ref().unwrap().menu.gear, "a row's menu has a gear");
    press(&mut host, Key::Char(','));
    let open = host.menu.as_ref().unwrap();
    assert!(open.menu.customising);
    assert_eq!(open.menu.title, "customise · book rows");
    assert!(open.menu.items.last().unwrap().note, "the editor says its keys");

    // Hide one, pin one, group one, give one another key.
    select(&mut host, "mend it");
    press(&mut host, Key::Char(' '));
    assert_eq!(rows_of(&host).iter().find(|r| r.1 == "mend it").unwrap().2, "hidden");
    select(&mut host, "ask about it");
    press(&mut host, Key::Char('*'));
    select(&mut host, "dust it");
    press(&mut host, Key::Char('f'));
    // `g` is radar's own: refused, the row keeps its key.
    select(&mut host, "check it out");
    press(&mut host, Key::Char('='));
    press(&mut host, Key::Char('g'));
    assert_eq!(rows_of(&host).iter().find(|r| r.1 == "check it out").unwrap().0, "c");
    press(&mut host, Key::Char('='));
    press(&mut host, Key::Char('v'));
    press(&mut host, Key::Char(','));
    assert!(!host.menu.as_ref().unwrap().menu.customising, ", is done");

    let rows = rows_of(&host);
    let labels: Vec<&str> = rows.iter().map(|r| r.1.as_str()).collect();
    assert_eq!(labels[0], "ask about it", "pinned first: {labels:?}");
    assert!(!labels.contains(&"mend it"), "hidden: {labels:?}");
    assert_eq!(rows.iter().find(|r| r.1 == "check it out").unwrap().0, "v");
    // The grouped one sits behind one row at the end, which opens it.
    let more = rows.last().unwrap();
    assert_eq!(more.1, "dust it…");
    select(&mut host, "dust it…");
    press(&mut host, Key::Enter);
    assert_eq!(labels_of(&host), ["dust it"]);
    press(&mut host, Key::Esc);

    let text = host.config_text();
    assert!(text.contains("[menus.book]"), "{text}");
    assert!(text.contains("\"desk.check\" = \"v\""), "{text}");
    assert!(text.contains("pinned = [\"desk.ask\"]"), "{text}");

    // A key that another row shows is swapped with it.
    press(&mut host, Key::Char('x'));
    press(&mut host, Key::Char(','));
    select(&mut host, "open it");
    press(&mut host, Key::Char('='));
    press(&mut host, Key::Char('v'));
    let rows = rows_of(&host);
    assert_eq!(rows.iter().find(|r| r.1 == "open it").unwrap().0, "v");
    assert_eq!(rows.iter().find(|r| r.1 == "check it out").unwrap().0, "o");
    // A key that moves in menus is refused.
    select(&mut host, "see its pages");
    press(&mut host, Key::Char('='));
    press(&mut host, Key::Char('j'));
    assert_eq!(rows_of(&host).iter().find(|r| r.1 == "see its pages").unwrap().0, "p");
    // `d`: the place as the parts have it.
    press(&mut host, Key::Char('d'));
    press(&mut host, Key::Esc);
    assert!(!host.config_text().contains("menus"), "{}", host.config_text());
    assert_eq!(rows_of(&host)[0].1, "open it");
}

#[test]
fn a_key_given_in_the_editor_does_the_action_on_the_row_too() {
    let (mut host, log) = host("sb");
    press(&mut host, Key::Char('x'));
    press(&mut host, Key::Char(','));
    select(&mut host, "announce it");
    press(&mut host, Key::Char('='));
    press(&mut host, Key::Char('v'));
    press(&mut host, Key::Esc);
    press(&mut host, Key::Esc);
    assert!(host.menu.is_none());
    press(&mut host, Key::Char('v'));
    assert_eq!(*log.lock().unwrap(), ["bell announce book 1"]);
}

#[test]
fn settings_menus_lists_each_kind_of_place_with_its_changes_and_the_switches() {
    let (mut host, _) = host("gd");
    press(&mut host, Key::Char('x'));
    press(&mut host, Key::Char(','));
    select(&mut host, "mend it");
    press(&mut host, Key::Char(' '));
    press(&mut host, Key::Esc);
    press(&mut host, Key::Esc);
    press(&mut host, Key::Char(crate::host::SETTINGS_KEY));
    host.section = host.sections.iter().position(|(o, _)| *o == Owner::Menus).unwrap();
    host.focus = Pane::Middle;
    let text = to_text(&host, 140, 30);
    assert!(text.contains("books rows") && text.contains("1 hidden"), "{text}");
    assert!(text.contains("other parts' actions") && text.contains("in the menu"), "{text}");
    // `d` on a kind of place puts it back.
    press(&mut host, Key::Char('d'));
    assert!(!host.config_text().contains("menus"), "{}", host.config_text());
    // A switch flips on ⏎ and is kept.
    press(&mut host, Key::Char('j'));
    press(&mut host, Key::Enter);
    assert!(host.config_text().contains("state_first = false"), "{}", host.config_text());
    // Off: what the row's state asks for stays where its part put it.
    host.view = View::Screen(0);
    host.focus = Pane::Middle;
    press(&mut host, Key::Char('x'));
    assert_eq!(labels(&host)[..3], ["open it", "see its pages", "change it"]);
    let at = |host: &Host, l: &str| labels(host).iter().position(|x| x == l).unwrap();
    assert!(at(&host, "see its pages") < at(&host, "change it"));
    press(&mut host, Key::Esc);
    press(&mut host, Key::Char(crate::host::SETTINGS_KEY));
    host.section = host.sections.iter().position(|(o, _)| *o == Owner::Menus).unwrap();
    host.focus = Pane::Middle;
    press(&mut host, Key::Char('j'));
    press(&mut host, Key::Enter);
    assert!(host.config_text().contains("other_parts = \"folded\""), "{}", host.config_text());
}

#[test]
fn moving_an_action_keeps_it_among_its_headings_rows() {
    let (mut host, _) = host("gd");
    press(&mut host, Key::Char('x'));
    press(&mut host, Key::Char(','));
    select(&mut host, "see its pages");
    press(&mut host, Key::Char('K'));
    press(&mut host, Key::Char(','));
    let labels: Vec<String> = rows_of(&host).into_iter().map(|r| r.1).collect();
    assert_eq!(labels[..2], ["see its pages".to_string(), "open it".to_string()]);
    // At the top of its heading's rows it goes no further.
    press(&mut host, Key::Char(','));
    select(&mut host, "see its pages");
    press(&mut host, Key::Char('K'));
    assert_eq!(rows_of(&host)[0].1, "see its pages");
}

fn labels_of(host: &Host) -> Vec<String> {
    labels(host)
}

#[test]
fn alone_a_part_shows_nothing_of_the_others() {
    let (mut host, _) = host("s");
    let text = to_text(&host, 80, 24);
    assert!(text.contains("book 1") && !text.contains('['), "{text}");
    press(&mut host, Key::Char('x'));
    assert_eq!(labels(&host), ["open it"]);
}

#[test]
fn has_and_can_show_follow_what_is_set_up() {
    let (host, _) = host("sn");
    let parts = Parts::new(&host.plugins, Some(0));
    assert!(parts.has("note") && parts.can_show("note"));
    assert!(parts.has("book"));
    assert!(!parts.has("card"));
    let (host, _) = super::tests::host("sN");
    let parts = Parts::new(&host.plugins, Some(0));
    assert!(!parts.has("note"), "not set up: not here");
}

#[test]
fn recognising_merges_every_owners_finds_in_order() {
    let (host, _) = host("sn");
    let link = format!("{BOOKS}2");
    let text = format!("see {link} for N-7");
    let found = recognise(&host.plugins, &text);
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].1, book(2));
    assert_eq!(&text[found[0].0.clone()], link);
    assert_eq!(found[1].1, note("N-7"));
}

#[test]
fn a_subject_is_its_kind_and_id() {
    assert_eq!(book(1), Subject::new("book", "1", "another label"));
    assert_ne!(book(1), Subject::new("note", "1", "book 1"));
    let link = Subject::link("https://x.example/");
    assert!(link.is(LINK) && link.url.as_deref() == Some("https://x.example/"));
}

#[test]
fn the_parts_section_lists_each_part_and_what_it_adds() {
    let (mut host, _) = host("sN");
    host.view = View::Settings;
    host.section = host
        .sections
        .iter()
        .position(|(o, _)| *o == Owner::Parts)
        .unwrap();
    host.focus = Pane::Middle;
    press(&mut host, Key::Char('j'));
    let text = to_text(&host, 120, 30);
    assert!(text.contains("✓ Shelf"), "{text}");
    assert!(text.contains("Notes   not set up"), "{text}");
    assert!(text.contains("has     notes"), "{text}");
    assert!(text.contains("· note chips on books"), "{text}");
    assert!(
        text.contains("(once set"),
        "said only of what it would add:\n{text}"
    );
    assert!(
        !text.contains("note links"),
        "nothing about a kind no part here owns:\n{text}"
    );
    press(&mut host, Key::Enter);
    assert_eq!(host.current_section().unwrap().1.name, "notebooks");
}

#[test]
fn with_no_parts_radar_says_so_in_settings() {
    let mut host = Host::new(vec![], 0).unwrap();
    host.load_config(Some("")).unwrap();
    assert_eq!(host.view, View::Settings);
    host.focus = Pane::Middle;
    let text = crate::headless::host_text_of(&host, 100, 24);
    assert!(
        text.contains("No parts are compiled into this build"),
        "{text}"
    );
    insta::assert_snapshot!(text);
}

#[test]
fn actions_receive_cached_references_once_from_ready_parts_only() {
    struct References {
        ready: bool,
        subject: Subject,
        received: Arc<Mutex<Vec<Subject>>>,
    }
    impl Plugin for References {
        fn id(&self) -> &'static str { "references" }
        fn name(&self) -> &'static str { "References" }
        fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> { Ok(()) }
        fn config(&self) -> Option<toml::Value> { None }
        fn set_up(&self) -> Setup {
            if self.ready { Setup::Ready } else { Setup::Needed("not configured".into()) }
        }
        fn related(&self, _: &Subject) -> Vec<Subject> {
            vec![self.subject.clone(), self.subject.clone(), book(1)]
        }
        fn act(&mut self, subject: &Subject, _: &str) -> Vec<Effect> {
            *self.received.lock().unwrap() = subject.related.clone();
            vec![]
        }
    }
    let received = Arc::new(Mutex::new(vec![]));
    let mut plugins: Vec<Box<dyn Plugin>> = vec![
        Box::new(References { ready: true, subject: note("own"), received: received.clone() }),
        Box::new(References { ready: true, subject: note("N-1"), received: received.clone() }),
        Box::new(References { ready: false, subject: note("N-2"), received: received.clone() }),
    ];
    let subject = book(1);
    act(&mut plugins, 0, &subject, "use");
    assert_eq!(*received.lock().unwrap(), [note("N-1")]);
    assert!(subject.related.is_empty(), "the original reference stays unchanged");
}
