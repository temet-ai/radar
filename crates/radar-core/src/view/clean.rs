//! Terminal controls in view text, stripped once where the host hands a
//! view out.
//!
//! Service text (a run's name, a title, a message) reaches
//! view data as the service sent it, so it may hold ESC, CSI, BEL, NEL or any
//! other C0 or C1 control (`char::is_control`, DEL included). A renderer
//! that writes such a character to the terminal lets a service move the
//! cursor, recolour the screen or set the clipboard. [`controls`] takes them
//! out of every string of a view; the host calls it in `Host::screen_view_in`,
//! `Host::section_view_in`, the open menu, the chrome and the dock, so no part
//! does and both renderers and the text form see clean text.
//!
//! What a string means decides what stays:
//!
//! - A tab becomes a space; any other control is dropped.
//! - A newline is dropped too, because a [`crate::view::Text`] run is one
//!   line. It stays only where the text is made of lines: inside a rich
//!   document (`doc`), in a detail fact's value, and in [`multiline`] views
//!   (the chrome's menus and notices, the dock's transcript).
//! - Names stay as they are: the `id`, `target`, `value` and similar strings
//!   a renderer sends back and the host looks up are not drawn, and changing
//!   them would break the lookup.
//! - An extension's `data` is the capability's own payload, not view text.
//!
//! The check walks the view through a `serde::Serializer` that builds
//! nothing, so a clean view costs one pass and no allocation. Only a dirty
//! one is converted to a JSON value, scrubbed and converted back.

use std::fmt;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde::ser::{self, Serializer};
use serde_json::Value;

/// Fields that hold a name, not words: strings (and lists of strings) under
/// these keys are left alone. A key of a field that holds structures is not
/// a name for what is inside them.
const NAMES: &[&str] = &[
    "id", "target", "path", "selected", "open", "value", "token", "capability", "part", "action",
    "pictures", "src", "kind", "active", "attention", "stripe", "glow", "look", "ink",
];

/// Fields whose text is made of lines.
const LINES: &[&str] = &["doc", "value"];

/// Fields that are not view text.
const UNTOUCHED: &[&str] = &["data"];

#[derive(Clone, Copy, Default)]
struct Ctx {
    /// A newline is part of the text.
    lines: bool,
    /// A name: left as it is.
    name: bool,
    /// Not view text, whatever it holds.
    skip: bool,
}

impl Ctx {
    /// The context of the field `key` of an object in this one.
    fn field(self, key: &str) -> Ctx {
        Ctx {
            lines: self.lines || LINES.contains(&key),
            name: NAMES.contains(&key),
            skip: self.skip || UNTOUCHED.contains(&key),
        }
    }

    /// Inside an object: its fields are named by their own keys.
    fn within(self) -> Ctx {
        Ctx { name: false, ..self }
    }

    fn keeps(self, c: char) -> bool {
        !c.is_control() || (self.lines && c == '\n')
    }

    fn dirty(self, s: &str) -> bool {
        !self.skip && !self.name && !s.chars().all(|c| self.keeps(c))
    }
}

/// `s` without its controls, a tab (and a newline on one line) as a space.
fn stripped(s: &str, ctx: Ctx) -> String {
    s.chars()
        .filter_map(|c| match c {
            '\t' => Some(' '),
            c if ctx.keeps(c) => Some(c),
            '\n' => Some(' '),
            _ => None,
        })
        .collect()
}

/// Strip the C0 and C1 controls from every string of a view whose text is
/// one line per run: a tab becomes a space and a newline goes (except in a
/// document or a fact's value).
pub fn controls<T: Serialize + DeserializeOwned>(view: &mut T) {
    run(view, Ctx::default());
}

/// [`controls`] for views whose strings are lines of text (the chrome's menus
/// and notices, the dock's transcript): a newline stays.
pub fn multiline<T: Serialize + DeserializeOwned>(view: &mut T) {
    run(view, Ctx { lines: true, ..Ctx::default() });
}

fn run<T: Serialize + DeserializeOwned>(view: &mut T, root: Ctx) {
    if view.serialize(Scan(root)).is_ok() {
        return;
    }
    let cleaned = serde_json::to_value(&*view).and_then(|mut value| {
        scrub(&mut value, root);
        serde_json::from_value::<T>(value)
    });
    match cleaned {
        Ok(clean) => *view = clean,
        Err(error) => {
            debug_assert!(false, "a view with controls would not go through JSON: {error}");
            crate::log::error(format!("view text kept its controls: {error}"));
        }
    }
}

fn scrub(value: &mut Value, ctx: Ctx) {
    match value {
        Value::String(s) if ctx.dirty(s) => *s = stripped(s, ctx),
        Value::Array(items) => items.iter_mut().for_each(|item| scrub(item, ctx)),
        Value::Object(fields) => {
            let inside = ctx.within();
            for (key, item) in fields.iter_mut() {
                scrub(item, inside.field(key));
            }
        }
        _ => {}
    }
}

/// The scan found a string with controls in it.
#[derive(Debug)]
struct Dirty;

impl fmt::Display for Dirty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a string holds terminal controls")
    }
}

impl std::error::Error for Dirty {}

impl ser::Error for Dirty {
    fn custom<T: fmt::Display>(_: T) -> Self {
        Dirty
    }
}

/// A serializer that writes nothing and stops at the first string that
/// holds controls.
struct Scan(Ctx);

/// What a sequence, map or struct under a scan is.
struct Walk(Ctx);

macro_rules! nothing {
    ($($method:ident($ty:ty)),* $(,)?) => {
        $(fn $method(self, _: $ty) -> Result<(), Dirty> { Ok(()) })*
    };
}

impl Serializer for Scan {
    type Ok = ();
    type Error = Dirty;
    type SerializeSeq = Walk;
    type SerializeTuple = Walk;
    type SerializeTupleStruct = Walk;
    type SerializeTupleVariant = Walk;
    type SerializeMap = Walk;
    type SerializeStruct = Walk;
    type SerializeStructVariant = Walk;

    nothing!(
        serialize_bool(bool),
        serialize_i8(i8),
        serialize_i16(i16),
        serialize_i32(i32),
        serialize_i64(i64),
        serialize_u8(u8),
        serialize_u16(u16),
        serialize_u32(u32),
        serialize_u64(u64),
        serialize_f32(f32),
        serialize_f64(f64),
        serialize_char(char),
        serialize_bytes(&[u8]),
    );

    fn serialize_str(self, v: &str) -> Result<(), Dirty> {
        if self.0.dirty(v) { Err(Dirty) } else { Ok(()) }
    }

    fn serialize_none(self) -> Result<(), Dirty> {
        Ok(())
    }

    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<(), Dirty> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<(), Dirty> {
        Ok(())
    }

    fn serialize_unit_struct(self, _: &'static str) -> Result<(), Dirty> {
        Ok(())
    }

    fn serialize_unit_variant(self, _: &'static str, _: u32, _: &'static str) -> Result<(), Dirty> {
        Ok(())
    }

    fn serialize_newtype_struct<T: ?Sized + Serialize>(self, _: &'static str, value: &T) -> Result<(), Dirty> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        value: &T,
    ) -> Result<(), Dirty> {
        value.serialize(self)
    }

    fn serialize_seq(self, _: Option<usize>) -> Result<Walk, Dirty> {
        Ok(Walk(self.0))
    }

    fn serialize_tuple(self, _: usize) -> Result<Walk, Dirty> {
        Ok(Walk(self.0))
    }

    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Walk, Dirty> {
        Ok(Walk(self.0))
    }

    fn serialize_tuple_variant(self, _: &'static str, _: u32, _: &'static str, _: usize) -> Result<Walk, Dirty> {
        Ok(Walk(self.0))
    }

    fn serialize_map(self, _: Option<usize>) -> Result<Walk, Dirty> {
        Ok(Walk(self.0.within()))
    }

    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Walk, Dirty> {
        Ok(Walk(self.0.within()))
    }

    fn serialize_struct_variant(self, _: &'static str, _: u32, _: &'static str, _: usize) -> Result<Walk, Dirty> {
        Ok(Walk(self.0.within()))
    }
}

impl ser::SerializeSeq for Walk {
    type Ok = ();
    type Error = Dirty;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Dirty> {
        value.serialize(Scan(self.0))
    }

    fn end(self) -> Result<(), Dirty> {
        Ok(())
    }
}

impl ser::SerializeTuple for Walk {
    type Ok = ();
    type Error = Dirty;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Dirty> {
        value.serialize(Scan(self.0))
    }

    fn end(self) -> Result<(), Dirty> {
        Ok(())
    }
}

impl ser::SerializeTupleStruct for Walk {
    type Ok = ();
    type Error = Dirty;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Dirty> {
        value.serialize(Scan(self.0))
    }

    fn end(self) -> Result<(), Dirty> {
        Ok(())
    }
}

impl ser::SerializeTupleVariant for Walk {
    type Ok = ();
    type Error = Dirty;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Dirty> {
        value.serialize(Scan(self.0))
    }

    fn end(self) -> Result<(), Dirty> {
        Ok(())
    }
}

impl ser::SerializeMap for Walk {
    type Ok = ();
    type Error = Dirty;

    /// A map's keys are names.
    fn serialize_key<T: ?Sized + Serialize>(&mut self, _: &T) -> Result<(), Dirty> {
        Ok(())
    }

    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Dirty> {
        value.serialize(Scan(self.0))
    }

    fn end(self) -> Result<(), Dirty> {
        Ok(())
    }
}

impl ser::SerializeStruct for Walk {
    type Ok = ();
    type Error = Dirty;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, key: &'static str, value: &T) -> Result<(), Dirty> {
        value.serialize(Scan(self.0.field(key)))
    }

    fn end(self) -> Result<(), Dirty> {
        Ok(())
    }
}

impl ser::SerializeStructVariant for Walk {
    type Ok = ();
    type Error = Dirty;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, key: &'static str, value: &T) -> Result<(), Dirty> {
        value.serialize(Scan(self.0.field(key)))
    }

    fn end(self) -> Result<(), Dirty> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::chrome::{Badge, ChromeView, Choice, Message};
    use crate::view::detail::DetailFact;
    use crate::view::doc::{Block as DocBlock, Doc, Inline};
    use crate::view::{
        Block, CellView, Ink, ListRow, MenuView, Node, PaneId, PaneView, ScreenView, Text, Tree, TreeRow,
    };

    /// ESC [ 31 m, BEL, NEL, CSI, DEL and a tab, as a service could send them.
    const DIRTY: &str = "a\u{1b}[31mb\u{7}c\u{85}d\u{9b}31me\u{7f}f\tg";
    const CLEAN: &str = "a[31mbcd31mef g";

    fn dirty_text() -> Text {
        Text::of(DIRTY, Ink::Primary)
    }

    /// Every string of a view that is not a name, with the controls in it.
    fn leaks(value: &Value, key: &str, lines: bool, out: &mut Vec<String>) {
        let lines = lines || LINES.contains(&key);
        match value {
            Value::String(s) if !NAMES.contains(&key) && !UNTOUCHED.contains(&key) => {
                if s.chars().any(|c| c.is_control() && !(lines && c == '\n')) {
                    out.push(format!("{key}: {s:?}"));
                }
            }
            Value::Array(items) => items.iter().for_each(|item| leaks(item, key, lines, out)),
            Value::Object(fields) => fields.iter().for_each(|(k, item)| leaks(item, k, lines, out)),
            _ => {}
        }
    }

    fn leaked(view: &impl Serialize, lines: bool) -> Vec<String> {
        let mut out = Vec::new();
        leaks(&serde_json::to_value(view).unwrap(), "", lines, &mut out);
        out
    }

    fn screen() -> ScreenView {
        let mut row = TreeRow::new(DIRTY, dirty_text());
        row = row.cell_view(CellView { heading: "state".into(), text: dirty_text(), show: None, target: None });
        let tree = Tree { rows: vec![row], selected: vec![DIRTY.into()], ..Tree::default() };
        let list = Node::List {
            rows: vec![ListRow::new(DIRTY, dirty_text()).right(dirty_text()).detail(dirty_text()).linked(Some(DIRTY.into()))],
            selected: Some(0),
            editing: None,
        };
        let doc = Doc::new(vec![DocBlock::para(vec![
            Inline::text(format!("{DIRTY}\nnext line")),
            Inline::link(DIRTY, "https://example.test/\u{1b}]52;c;x\u{7}"),
        ])]);
        let rich = Node::Rich {
            blocks: vec![
                Block::line(dirty_text()),
                Block::Doc { doc, bar: None, most: None, urls: None },
                Block::section(None, DIRTY, Some(DIRTY.into()), vec![]),
            ],
            pictures: None,
        };
        ScreenView::new(
            "watch",
            vec![
                PaneView::new(PaneId::Nav, DIRTY, list),
                PaneView::new(PaneId::Middle, "middle", Node::Tree(tree)),
                PaneView::new(PaneId::Detail, "detail", rich),
            ],
        )
    }

    #[test]
    fn a_screen_comes_out_without_controls() {
        let mut view = screen();
        assert!(!leaked(&view, false).is_empty(), "the fixture is dirty");
        controls(&mut view);
        assert_eq!(leaked(&view, false), Vec::<String>::new());

        let Node::List { rows, .. } = &view.panes[0].body else { panic!("a list") };
        assert_eq!(view.panes[0].title, CLEAN);
        assert_eq!(rows[0].text.words(), CLEAN, "list text");
        assert_eq!(rows[0].right.as_ref().unwrap().words(), CLEAN, "list right");
        assert_eq!(rows[0].detail.as_ref().unwrap().words(), CLEAN, "list detail");
        assert_eq!(rows[0].link.as_deref(), Some(CLEAN), "a link is words");
        assert_eq!(rows[0].target.0, DIRTY, "a target is a name");

        let Node::Tree(tree) = &view.panes[1].body else { panic!("a tree") };
        assert_eq!(tree.rows[0].name.words(), CLEAN, "tree name");
        assert_eq!(tree.rows[0].cells[0].text.words(), CLEAN, "tree cell");
        assert_eq!(tree.rows[0].id, DIRTY, "a row id is a name");
        assert_eq!(tree.selected, vec![DIRTY.to_string()], "a path is names");

        let Node::Rich { blocks, .. } = &view.panes[2].body else { panic!("blocks") };
        let Block::Doc { doc, .. } = &blocks[1] else { panic!("a doc") };
        let DocBlock::Para { text } = &doc.blocks[0] else { panic!("a paragraph") };
        assert_eq!(text[0].text, format!("{CLEAN}\nnext line"), "a document keeps its lines");
        assert_eq!(text[1].text, CLEAN);
        let crate::view::doc::Kind::Link { url } = &text[1].kind else { panic!("a link") };
        assert_eq!(url, "https://example.test/]52;c;x", "a link's address is words too");
        let Block::Section { title, detail, .. } = &blocks[2] else { panic!("a section") };
        assert_eq!((title.as_str(), detail.as_deref()), (CLEAN.to_uppercase().as_str(), Some(CLEAN)), "a section title is in capitals");
    }

    #[test]
    fn newlines_stay_only_where_the_text_is_lines() {
        let mut text = Text::of("one\r\ntwo\nthree", Ink::Primary);
        controls(&mut text);
        assert_eq!(text.words(), "one two three", "a run is one line; its words stay apart");

        let mut fact = DetailFact::new("note", Text::of("one\r\ntwo", Ink::Primary));
        controls(&mut fact);
        assert_eq!(fact.value.words(), "one\ntwo", "a fact's value may run to lines");

        let mut menu = MenuView::new("m", "line one\nline two\u{1b}[2J", vec![]);
        multiline(&mut menu);
        assert_eq!(menu.title, "line one\nline two[2J");
    }

    #[test]
    fn the_chrome_comes_out_without_controls() {
        let mut chrome = ChromeView::new(80, 24);
        chrome.status.scope = DIRTY.into();
        chrome.status.counts = vec![Badge { text: DIRTY.into(), ink: Ink::Warn }];
        chrome.status.message = Some(Message { text: format!("{DIRTY}\nsecond"), icon: None, ink: Ink::Plain });
        let item = |value: &str| Choice {
            mnemonic_at: None,
            mnemonic_shift: false,
            value: value.into(),
            label: DIRTY.into(),
            hint: DIRTY.into(),
            key: None,
            keys: DIRTY.into(),
            icon: None,
            heading: false,
            note: false,
            sample: None,
            shown_only: false,
            muted: false,
            target: None,
        };
        let mut menu = MenuView::new("actions", DIRTY, vec![item("run:1"), item(DIRTY)]);
        menu.typed = Some(DIRTY.into());
        chrome.menu = Some(menu);
        assert!(!leaked(&chrome, true).is_empty(), "the fixture is dirty");
        multiline(&mut chrome);
        assert_eq!(leaked(&chrome, true), Vec::<String>::new());

        assert_eq!(chrome.status.scope, CLEAN);
        assert_eq!(chrome.status.counts[0].text, CLEAN);
        assert_eq!(chrome.status.message.as_ref().unwrap().text, format!("{CLEAN}\nsecond"));
        let menu = chrome.menu.as_ref().unwrap();
        assert_eq!(menu.title, CLEAN);
        assert_eq!(menu.typed.as_deref(), Some(CLEAN));
        assert_eq!(menu.items[0].label, CLEAN);
        assert_eq!(menu.items[0].hint, CLEAN);
        assert_eq!(menu.items[0].keys, CLEAN);
        assert_eq!(menu.items[0].value, "run:1");
        assert_eq!(menu.items[1].value, DIRTY, "an Invoke answers with the value the host holds");
    }

    #[test]
    fn an_extension_keeps_its_payload_and_a_clean_view_is_left_alone() {
        let fallback = Node::Document { lines: vec![dirty_text()], cut: false };
        let mut node = Node::Extension {
            capability: "terminal".into(),
            version: 1,
            data: serde_json::json!({ "screen": DIRTY }),
            fallback: Box::new(fallback),
        };
        controls(&mut node);
        let Node::Extension { data, fallback, .. } = &node else { panic!("an extension") };
        assert_eq!(data["screen"], DIRTY, "the capability's own payload");
        let Node::Document { lines, .. } = fallback.as_ref() else { panic!("a document") };
        assert_eq!(lines[0].words(), CLEAN, "the fallback is view text");

        let mut view = screen();
        controls(&mut view);
        assert!(view.serialize(Scan(Ctx::default())).is_ok(), "nothing left to find");
        let before = view.clone();
        controls(&mut view);
        assert_eq!(view, before);
    }
}
