//! Markdown as lines: text written in markdown (a description, a comment)
//! parsed into the kit's rich-text model ([`super::rich::Doc`]) and laid
//! out by it for a panel `width` cells wide, in theme slots only, so it
//! reads without colour. `pulldown-cmark` does the parsing; this module
//! turns its events into blocks and runs: paragraphs, headings, lists,
//! quotes, code, tables, links (shown as their words, their urls collected),
//! images (a line of their own is an image block; one among words shows its
//! words), and the HTML that descriptions carry (comments dropped,
//! `<details>` shown as its summary and its body, a few tags mapped to
//! marks, the rest dropped with their text kept).

use pulldown_cmark::{Alignment, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use super::rich::{self, Align, Block, Doc, Inline, Item, Kind, Marker, Marks, Media};
use crate::theme::Theme;

pub use super::rich::Link;

/// The laid-out text and the links in it, in order, one per url.
pub type Markdown = rich::Laid;

/// GFM-flavoured markdown (tables, strikethrough, task lists, the HTML
/// descriptions carry) laid out for a panel `width` cells wide.
pub fn render(theme: &Theme, text: &str, width: u16) -> Markdown {
    rich::layout(&parse(text), &rich::Look::new(theme), width)
}

/// The plain text of the same layout (for quoting or copying): the lines
/// joined with `\n`.
pub fn plain(theme: &Theme, text: &str, width: u16) -> String {
    rich::text_of(&render(theme, text, width).lines)
}

/// Markdown as a document.
pub fn parse(text: &str) -> Doc {
    let mut p = Builder::default();
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    for event in Parser::new_ext(text, options) {
        p.event(event);
    }
    p.finish()
}

// ---------------------------------------------------------------------
// HTML helpers.

/// `&amp;`, `&#39;` and the like, decoded; anything unknown stays as it is.
fn decode(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest.find(';').filter(|e| *e <= 10);
        let decoded = end.and_then(|e| {
            let name = &rest[1..e];
            match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                _ => name
                    .strip_prefix('#')
                    .and_then(|n| match n.strip_prefix(['x', 'X']) {
                        Some(h) => u32::from_str_radix(h, 16).ok(),
                        None => n.parse().ok(),
                    })
                    .and_then(char::from_u32),
            }
        });
        match (decoded, end) {
            (Some(c), Some(e)) => {
                out.push(c);
                rest = &rest[e + 1..];
            }
            _ => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The value of attribute `name` in a tag's attributes.
fn attr(attrs: &str, name: &str) -> Option<String> {
    let lower = attrs.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name) {
        let at = from + i;
        let rest = attrs[at + name.len()..].trim_start();
        if (at == 0 || lower.as_bytes()[at - 1].is_ascii_whitespace()) && rest.starts_with('=') {
            let v = rest[1..].trim_start();
            return Some(match v.chars().next().filter(|c| *c == '"' || *c == '\'') {
                Some(q) => {
                    let inner = &v[1..];
                    inner[..inner.find(q).unwrap_or(inner.len())].to_string()
                }
                None => v.split_whitespace().next().unwrap_or("").to_string(),
            });
        }
        from = at + name.len();
    }
    None
}

/// Where the `>` that closes the tag at the start of `s` is, quotes honoured.
fn tag_end(s: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (i, c) in s.char_indices().skip(1) {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => return Some(i),
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------
// Events to blocks.

/// A container still being filled.
enum Open {
    Quote(Vec<Block>),
    List {
        next: Option<u64>,
        items: Vec<Item>,
    },
    Item(Item),
    Details {
        summary: Vec<Inline>,
        blocks: Vec<Block>,
    },
}

struct TableB {
    aligns: Vec<Align>,
    head: Vec<Vec<Inline>>,
    rows: Vec<Vec<Vec<Inline>>>,
    row: Vec<Vec<Inline>>,
}

#[derive(Default)]
struct Builder {
    root: Vec<Block>,
    open: Vec<Open>,
    /// The runs gathered since the last block ended.
    cur: Vec<Inline>,
    bold: usize,
    italic: usize,
    strike: usize,
    code: usize,
    /// The images being read, by where they go.
    images: Vec<String>,
    /// The links being read, by url.
    links: Vec<String>,
    heading: Option<u8>,
    summary: bool,
    in_comment: bool,
    a_stack: Vec<bool>,
    code_block: Option<(String, String)>,
    table: Option<TableB>,
}

impl Builder {
    fn finish(mut self) -> Doc {
        self.flush();
        while !self.open.is_empty() {
            self.close_innermost();
        }
        Doc::new(self.root)
    }

    fn marks(&self) -> Marks {
        Marks {
            bold: self.bold > 0,
            italic: self.italic > 0,
            strike: self.strike > 0,
            code: self.code > 0,
            underline: false,
        }
    }

    /// A block into the innermost container that holds blocks.
    fn push(&mut self, b: Block) {
        for o in self.open.iter_mut().rev() {
            match o {
                Open::Quote(blocks) | Open::Details { blocks, .. } => return blocks.push(b),
                Open::Item(item) => return item.blocks.push(b),
                Open::List { .. } => {}
            }
        }
        self.root.push(b);
    }

    /// The runs gathered so far as a block: a heading, a details' summary,
    /// images on a line of their own, or a paragraph.
    fn flush(&mut self) {
        let cur = std::mem::take(&mut self.cur);
        if cur.iter().all(|i| matches!(i.kind, Kind::Break) || i.text.trim().is_empty())
            && !cur.iter().any(|i| matches!(i.kind, Kind::Image { .. }))
        {
            return;
        }
        if let Some(level) = self.heading {
            return self.push(Block::Heading { level, text: cur });
        }
        if self.summary {
            for o in self.open.iter_mut().rev() {
                if let Open::Details { summary, .. } = o {
                    summary.extend(cur);
                    return;
                }
            }
        }
        if let Some(media) = alone(&cur) {
            for m in media {
                self.push(Block::Media(m));
            }
            return;
        }
        self.push(Block::para(cur));
    }

    fn close_innermost(&mut self) {
        let Some(o) = self.open.pop() else { return };
        match o {
            Open::Quote(blocks) => self.push(Block::Quote { blocks }),
            Open::List { items, .. } => self.push(Block::List { items }),
            Open::Item(item) => match self.open.last_mut() {
                Some(Open::List { items, .. }) => items.push(item),
                _ => self.push(Block::List { items: vec![item] }),
            },
            Open::Details { summary, blocks } => self.push(Block::Details { summary, blocks }),
        }
    }

    /// Close containers up to and including the innermost `is` matches.
    fn close(&mut self, is: impl Fn(&Open) -> bool) {
        if !self.open.iter().any(&is) {
            return;
        }
        while let Some(o) = self.open.last() {
            let last = is(o);
            self.close_innermost();
            if last {
                break;
            }
        }
    }

    fn text(&mut self, s: &str) {
        let marks = self.marks();
        let kind = match (self.links.last(), self.images.last()) {
            (Some(url), _) => Kind::Link { url: url.clone() },
            (None, Some(src)) => Kind::Image { src: src.clone() },
            _ => Kind::Text,
        };
        self.cur.push(Inline {
            text: s.to_string(),
            kind,
            marks,
        });
    }

    fn event(&mut self, ev: Event<'_>) {
        match ev {
            Event::Start(t) => self.start(t),
            Event::End(t) => self.end(t),
            Event::Text(t) => match &mut self.code_block {
                Some((_, c)) => c.push_str(&t),
                None => self.text(&t),
            },
            Event::Code(t) => {
                self.code += 1;
                self.text(&t);
                self.code -= 1;
            }
            Event::Html(h) | Event::InlineHtml(h) => self.html(&h),
            Event::SoftBreak => self.text(" "),
            Event::HardBreak => self.cur.push(Inline::line_break()),
            Event::Rule => {
                self.flush();
                self.push(Block::Rule);
            }
            Event::TaskListMarker(done) => {
                if let Some(Open::Item(item)) = self
                    .open
                    .iter_mut()
                    .rev()
                    .find(|o| matches!(o, Open::Item(_)))
                {
                    item.marker = if done { Marker::Done } else { Marker::Todo };
                }
            }
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph | Tag::HtmlBlock => self.flush(),
            Tag::Heading { level, .. } => {
                self.flush();
                self.heading = Some(level as u8);
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.open.push(Open::Quote(Vec::new()));
            }
            Tag::CodeBlock(kind) => {
                self.flush();
                let lang = match kind {
                    CodeBlockKind::Fenced(l) => l.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code_block = Some((lang, String::new()));
            }
            Tag::List(start) => {
                self.flush();
                self.open.push(Open::List {
                    next: start,
                    items: Vec::new(),
                });
            }
            Tag::Item => {
                self.flush();
                let marker = match self.open.last_mut() {
                    Some(Open::List { next: Some(n), .. }) => {
                        let m = Marker::Number(*n);
                        *n += 1;
                        m
                    }
                    _ => Marker::Bullet,
                };
                self.open.push(Open::Item(Item {
                    marker,
                    blocks: Vec::new(),
                }));
            }
            Tag::Table(aligns) => {
                self.flush();
                self.table = Some(TableB {
                    aligns: aligns
                        .into_iter()
                        .map(|a| match a {
                            Alignment::Right => Align::Right,
                            Alignment::Center => Align::Center,
                            _ => Align::Left,
                        })
                        .collect(),
                    head: Vec::new(),
                    rows: Vec::new(),
                    row: Vec::new(),
                });
            }
            Tag::Emphasis => self.italic += 1,
            Tag::Strong => self.bold += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Link { dest_url, .. } => self.links.push(dest_url.to_string()),
            Tag::Image { dest_url, .. } => self.images.push(dest_url.to_string()),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::HtmlBlock => self.flush(),
            TagEnd::Heading(_) => {
                self.flush();
                self.heading = None;
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.close(|o| matches!(o, Open::Quote(_)));
            }
            TagEnd::CodeBlock => {
                if let Some((lang, text)) = self.code_block.take() {
                    self.push(Block::Code { lang, text });
                }
            }
            TagEnd::List(_) => {
                self.flush();
                self.close(|o| matches!(o, Open::List { .. }));
            }
            TagEnd::Item => {
                self.flush();
                self.close(|o| matches!(o, Open::Item(_)));
            }
            TagEnd::Table => {
                if let Some(t) = self.table.take() {
                    self.push(Block::Table {
                        aligns: t.aligns,
                        head: t.head,
                        rows: t.rows,
                    });
                }
            }
            TagEnd::TableHead => {
                if let Some(t) = &mut self.table {
                    t.head = std::mem::take(&mut t.row);
                }
            }
            TagEnd::TableRow => {
                if let Some(t) = &mut self.table {
                    let row = std::mem::take(&mut t.row);
                    t.rows.push(row);
                }
            }
            TagEnd::TableCell => {
                let cell = std::mem::take(&mut self.cur);
                if let Some(t) = &mut self.table {
                    t.row.push(cell);
                }
            }
            TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
            TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            TagEnd::Link => self.end_link(),
            TagEnd::Image => {
                self.images.pop();
            }
            _ => {}
        }
    }

    /// A link ends; one with no words is still a link (its url its words).
    fn end_link(&mut self) {
        let Some(url) = self.links.pop() else { return };
        let had = self
            .cur
            .last()
            .is_some_and(|i| matches!(&i.kind, Kind::Link { url: u } if *u == url));
        if !had {
            self.cur.push(Inline::link("", url));
        }
    }

    // --- html

    fn html_text(&mut self, s: &str) {
        if !s.is_empty() {
            let d = decode(s);
            self.text(&d);
        }
    }

    fn html(&mut self, s: &str) {
        let mut rest = s;
        loop {
            if self.in_comment {
                match rest.find("-->") {
                    Some(i) => {
                        rest = &rest[i + 3..];
                        self.in_comment = false;
                        continue;
                    }
                    None => return,
                }
            }
            let Some(i) = rest.find('<') else {
                self.html_text(rest);
                return;
            };
            self.html_text(&rest[..i]);
            rest = &rest[i..];
            if let Some(r) = rest.strip_prefix("<!--") {
                self.in_comment = true;
                rest = r;
                continue;
            }
            let next = rest[1..].chars().next();
            if !next.is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, '/' | '!' | '?')) {
                self.html_text("<");
                rest = &rest[1..];
                continue;
            }
            match tag_end(rest) {
                Some(end) => {
                    let inner = &rest[1..end];
                    rest = &rest[end + 1..];
                    self.read_tag(inner);
                }
                None => {
                    self.html_text(rest);
                    return;
                }
            }
        }
    }

    fn soft_break(&mut self) {
        if !matches!(
            self.cur.last(),
            None | Some(Inline {
                kind: Kind::Break,
                ..
            })
        ) {
            self.cur.push(Inline::line_break());
        }
    }

    /// One tag's inside (`a href="u"`, `/b`, `br/`).
    fn read_tag(&mut self, inner: &str) {
        let (closing, inner) = match inner.strip_prefix('/') {
            Some(i) => (true, i),
            None => (false, inner),
        };
        let name_end = inner
            .find(|c: char| c.is_whitespace() || c == '/')
            .unwrap_or(inner.len());
        let name = inner[..name_end].to_ascii_lowercase();
        let attrs = &inner[name_end..];
        let dec = |n: &mut usize| *n = if closing { n.saturating_sub(1) } else { *n + 1 };
        match name.as_str() {
            "br" => self.cur.push(Inline::line_break()),
            "details" => {
                self.flush();
                if closing {
                    self.close(|o| matches!(o, Open::Details { .. }));
                } else {
                    self.open.push(Open::Details {
                        summary: Vec::new(),
                        blocks: Vec::new(),
                    });
                }
            }
            "summary" => {
                self.flush();
                self.summary = !closing;
            }
            "a" => {
                if closing {
                    if self.a_stack.pop() == Some(true) {
                        self.end_link();
                    }
                } else if let Some(href) = attr(attrs, "href") {
                    self.links.push(decode(&href));
                    self.a_stack.push(true);
                } else {
                    self.a_stack.push(false);
                }
            }
            "img" => {
                if !closing {
                    let src = attr(attrs, "src").map(|s| decode(&s)).unwrap_or_default();
                    self.images.push(src);
                    let alt = attr(attrs, "alt").unwrap_or_default();
                    let marks = self.marks();
                    let kind = match (self.links.last(), self.images.last()) {
                        (Some(url), _) => Kind::Link { url: url.clone() },
                        (None, Some(src)) => Kind::Image { src: src.clone() },
                        _ => Kind::Text,
                    };
                    self.cur.push(Inline {
                        text: decode(&alt),
                        kind,
                        marks,
                    });
                    self.images.pop();
                }
            }
            "b" | "strong" => dec(&mut self.bold),
            "i" | "em" => dec(&mut self.italic),
            "code" => dec(&mut self.code),
            "s" | "strike" | "del" => dec(&mut self.strike),
            "p" | "div" | "ul" | "ol" | "li" | "tr" | "table" | "blockquote" | "hr" | "pre"
            | "dl" | "dt" | "dd" | "section" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.soft_break()
            }
            _ => {}
        }
    }
}

/// Images with nothing but space and breaks around them: each as an image
/// block (its words its name, its address where it opens and what draws it).
fn alone(runs: &[Inline]) -> Option<Vec<Media>> {
    let mut out: Vec<Media> = Vec::new();
    let mut last: Option<&str> = None;
    for r in runs {
        match &r.kind {
            Kind::Image { src } => {
                if last == Some(src.as_str())
                    && let Some(m) = out.last_mut()
                {
                    m.name.push_str(&r.text);
                    continue;
                }
                last = Some(src);
                out.push(Media {
                    image: true,
                    name: r.text.clone(),
                    url: src.clone(),
                    src: src.clone(),
                    size: None,
                });
            }
            Kind::Break => last = None,
            _ if r.text.trim().is_empty() => last = None,
            _ => return None,
        }
    }
    for m in &mut out {
        m.name = m.name.split_whitespace().collect::<Vec<_>>().join(" ");
    }
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::design::theme::{Slot, Tier};
    use ratatui::style::{Modifier, Style};
    use unicode_width::UnicodeWidthStr;

    fn width_of(s: &str) -> usize {
        UnicodeWidthStr::width(s)
    }

    fn mono() -> Theme {
        Theme::new(Tier::Mono)
    }

    fn at(md: &str, w: u16) -> Vec<String> {
        plain(&mono(), md, w).lines().map(String::from).collect()
    }

    /// A dependency-update description: a comment header, a table of
    /// badges in links, headings, a rule, folding html, a schedule line
    /// and a trailing comment.
    const SAMPLE: &str = "<!-- update-bot: header\nsecond line of the comment -->\n\
This update bumps two dependencies.\n\n\
## Update summary\n\n\
| Package | Change | Age | Confidence |\n\
|---|---|---|---|\n\
| [acme/widget-lib](https://example.com/widget-lib) | `1.2.2` -> `1.2.3` | [![age](https://img/age.svg)](https://example.com/age) | [![confidence](https://img/conf.svg)](https://example.com/conf) |\n\
| [acme/gadget-kit](https://example.com/gadget-kit) | `4.0.0` -> `4.1.0` | [![age](https://img/age2.svg)](https://example.com/age2) | [![confidence](https://img/conf2.svg)](https://example.com/conf2) |\n\n\
---\n\n\
### Release notes\n\n\
<details><summary>acme/widget-lib (widget-lib)</summary>\n\n\
### [`v1.2.3`](https://example.com/v1.2.3)\n\n\
- Fixes a **crash** when the shelf is empty\n\
- Adds `--quiet`, thanks to the <a href=\"https://example.com/team\">widget team</a>\n\
\x20\x20- nested note that is long enough to wrap onto a second line of the panel\n\n\
</details>\n\n\
---\n\n\
📅 **Schedule**: Branch creation - At any time, Automerge - At any time (no schedule defined).\n\n\
<!--debug:eyJyZXBvIjoiYWNtZSJ9-->\n";

    fn rendered(md: &str, w: u16) -> Markdown {
        render(&mono(), md, w)
    }

    fn fits(md: &str) {
        for w in 1..=120u16 {
            for (i, l) in rendered(md, w).lines.iter().enumerate() {
                let n: usize = l.spans.iter().map(|s| width_of(&s.content)).sum();
                assert!(n <= w as usize, "width {w}, line {i} is {n} wide: {l:?}");
            }
        }
    }

    /// The style of the span holding `word`.
    fn style_of(md: &str, word: &str) -> Style {
        let m = render(&Theme::new(Tier::TrueColor), md, 60);
        m.lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.contains(word))
            .unwrap_or_else(|| panic!("no span with {word}"))
            .style
    }

    #[test]
    fn paragraphs_wrap_at_words_with_one_blank_between_and_none_at_the_ends() {
        let out = at("\n\none two three four five six\n\n\n\nseven\n\n", 14);
        assert_eq!(out, ["one two three", "four five six", "", "seven"]);
        assert!(at("", 40).is_empty());
        assert!(at("  \n\n \n", 40).is_empty());
    }

    #[test]
    fn headings_have_no_hashes_and_a_blank_after() {
        let out = at("# Big\n\n## Medium\ntext\n\n### Small", 40);
        assert_eq!(out, ["Big", "───", "", "Medium", "", "text", "", "Small"]);
        let t = Theme::new(Tier::TrueColor);
        let s = style_of("## Medium", "Medium");
        assert_eq!(s.fg, t.s(Slot::TextEmphasis).fg);
        assert!(s.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn emphasis_strong_strike_and_code_have_their_own_looks() {
        assert_eq!(at("*a* **b** ~~c~~ `d`", 40), ["a b c d"]);
        assert!(
            style_of("*a* b", "a")
                .add_modifier
                .contains(Modifier::ITALIC)
        );
        assert!(
            style_of("**a** b", "a")
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert!(
            style_of("~~a~~ b", "a")
                .add_modifier
                .contains(Modifier::CROSSED_OUT)
        );
        let t = Theme::new(Tier::TrueColor);
        assert_eq!(style_of("x `d` y", "d").fg, t.s(Slot::AccentSecondary).fg);
        assert_eq!(at("a**b**c", 40), ["abc"]);
    }

    #[test]
    fn lists_bullet_nest_number_and_hang_wrapped_lines_under_the_text() {
        let md = "- one\n- two is a longer item that wraps\n  - nested\n    - deeper\n\n1. first\n2. second\n\n- [ ] todo\n- [x] done";
        assert_eq!(
            at(md, 20),
            [
                "• one",
                "• two is a longer",
                "  item that wraps",
                "  ◦ nested",
                "    ◦ deeper",
                "",
                "1. first",
                "2. second",
                "",
                "☐ todo",
                "☑ done",
            ]
        );
        assert_eq!(
            at("9. a\n10. long enough to wrap here", 14),
            ["9. a", "10. long", "    enough to", "    wrap here"]
        );
    }

    #[test]
    fn a_loose_list_keeps_its_items_together_and_its_paragraphs_apart() {
        let out = at("- one\n\n  more\n\n- two", 30);
        assert_eq!(out, ["• one", "", "  more", "• two"]);
    }

    #[test]
    fn block_quotes_carry_a_bar_on_every_line() {
        let out = at(
            "before\n\n> quoted words that wrap around\n>\n> second\n\nafter",
            16,
        );
        assert_eq!(
            out,
            [
                "before",
                "",
                "│ quoted words",
                "│ that wrap",
                "│ around",
                "│",
                "│ second",
                "",
                "after"
            ]
        );
        let t = Theme::new(Tier::TrueColor);
        assert_eq!(
            style_of("> quoted", "quoted").fg,
            t.s(Slot::TextSecondary).fg
        );
    }

    #[test]
    fn code_is_kept_as_written_and_hard_wrapped_with_the_same_indent() {
        assert_eq!(
            at("```rust\nlet a = **b**;\n\n  if x {}\n```", 40),
            ["  let a = **b**;", "", "    if x {}"]
        );
        assert_eq!(at("    indented code", 40), ["  indented code"]);
        assert_eq!(at("```\nabcdefghijkl\n```", 8), ["  abcdef", "  ghijkl"]);
        let t = Theme::new(Tier::TrueColor);
        assert_eq!(
            style_of("```\ncode\n```", "code").fg,
            t.s(Slot::AccentSecondary).fg
        );
    }

    #[test]
    fn a_rule_is_at_most_24_wide_or_the_width() {
        assert_eq!(
            at("a\n\n---\n\nb", 80),
            ["a", "", "─".repeat(24).as_str(), "", "b"]
        );
        assert_eq!(at("---", 10), ["─".repeat(10)]);
    }

    #[test]
    fn links_show_their_words_and_collect_their_urls_once_in_order() {
        let md = "See [the docs](https://x/docs) and [again](https://x/docs), <https://x/auto>, [![alt words](https://img/b.svg)](https://x/badge) and [![](https://img/c.svg)](https://x/empty).";
        let m = rendered(md, 80);
        assert_eq!(
            plain(&mono(), md, 80),
            "See the docs and again, https://x/auto, alt words and ."
        );
        assert_eq!(
            m.links,
            [
                Link {
                    text: "the docs".into(),
                    url: "https://x/docs".into()
                },
                Link {
                    text: "https://x/auto".into(),
                    url: "https://x/auto".into()
                },
                Link {
                    text: "alt words".into(),
                    url: "https://x/badge".into()
                },
                Link {
                    text: "https://x/empty".into(),
                    url: "https://x/empty".into()
                },
            ]
        );
        let s = style_of("a [link](https://x) b", "link");
        let t = Theme::new(Tier::TrueColor);
        assert_eq!(s.fg, t.s(Slot::AccentPrimary).fg);
        assert!(s.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn images_show_their_alt_text_muted_and_nothing_without_one() {
        assert_eq!(
            at("a ![shown](https://i/x.png) b ![](https://i/y.png) c", 40),
            ["a shown b c"]
        );
        let t = Theme::new(Tier::TrueColor);
        assert_eq!(
            style_of("a ![shown](https://i/x.png) b", "shown").fg,
            t.s(Slot::TextMuted).fg
        );
    }

    #[test]
    fn comments_are_dropped_whole_blocks_inline_and_across_lines() {
        let md = "<!-- one\ntwo\nthree -->\nkept <!-- inline --> text\n\n<!--\nblock\n-->\n\nend<!--x-->";
        assert_eq!(at(md, 40), ["kept text", "", "end"]);
        assert!(rendered("<!-- only -->", 40).lines.is_empty());
    }

    #[test]
    fn details_show_a_bold_summary_and_an_indented_body_without_chevrons() {
        let md = "<details>\n<summary>The summary</summary>\n\nBody text.\n\n<details><summary>Inner</summary>\n\n- item\n\n</details>\n\n</details>\n\nafter";
        let out = at(md, 40);
        assert_eq!(
            out,
            [
                "The summary",
                "",
                "  Body text.",
                "",
                "  Inner",
                "",
                "    • item",
                "",
                "after"
            ]
        );
        for bad in ["▸", "▾", "[+]", "[-]"] {
            assert!(!out.join("\n").contains(bad));
        }
        let s = style_of(
            "<details><summary>Sum</summary>\n\nbody\n\n</details>",
            "Sum",
        );
        let t = Theme::new(Tier::TrueColor);
        assert_eq!(s.fg, t.s(Slot::TextSecondary).fg);
        assert!(s.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn html_tags_map_to_styles_or_drop_with_their_text_kept() {
        let md = "a<br>b<br/>c <b>bold</b> <i>it</i> <code>co</code> <kbd>K</kbd> <sub>s</sub><sup>p</sup> <span>sp</span> <img alt=\"pic\" src=\"https://i/p.png\"> <img src=\"x\"> <a href=\"https://x/u\">link text</a>";
        let m = rendered(md, 80);
        assert_eq!(
            plain(&mono(), md, 80),
            "a\nb\nc bold it co K sp sp pic link text"
        );
        assert_eq!(
            m.links,
            [Link {
                text: "link text".into(),
                url: "https://x/u".into()
            }]
        );
        assert!(
            style_of("x <b>bold</b>", "bold")
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert!(
            style_of("x <i>it</i>", "it")
                .add_modifier
                .contains(Modifier::ITALIC)
        );
        let t = Theme::new(Tier::TrueColor);
        assert_eq!(
            style_of("x <code>co</code>", "co").fg,
            t.s(Slot::AccentSecondary).fg
        );
    }

    #[test]
    fn html_blocks_with_several_tags_and_text_on_a_line_keep_their_text() {
        let out = at(
            "<div><p>First</p><p>Second <b>one</b></p></div>\n\nafter",
            40,
        );
        assert_eq!(out, ["First", "Second one", "", "after"]);
    }

    #[test]
    fn entities_are_decoded() {
        assert_eq!(
            at(
                "<p>a &amp; b &lt;c&gt; &quot;d&quot; it&#39;s&nbsp;ok &#x41; &bogus;</p>",
                80
            ),
            ["a & b <c> \"d\" it's ok A &bogus;"]
        );
        assert_eq!(at("a &amp; b", 40), ["a & b"]);
    }

    #[test]
    fn tables_are_columns_with_a_bold_header_a_rule_and_alignment() {
        let md = "| Name | Qty | Note |\n|:--|--:|:-:|\n| apple | 3 | ok |\n| fig | 12 | fine |";
        assert_eq!(
            at(md, 40),
            [
                "Name   Qty  Note",
                "─────  ───  ────",
                "apple    3   ok",
                "fig     12  fine"
            ]
        );
        assert!(
            style_of("| H |\n|---|\n| c |", "H")
                .add_modifier
                .contains(Modifier::BOLD)
        );
    }

    #[test]
    fn a_table_that_is_too_wide_shrinks_its_widest_columns_and_wraps_cells() {
        let md = "| Package | Change |\n|---|---|\n| acme/widget-lib | `1.2.2` -> `1.2.3` and more words |";
        let out = at(md, 24);
        assert!(out.iter().all(|l| l.chars().count() <= 24), "{out:?}");
        assert!(out.len() > 3, "{out:?}");
        assert!(out[2].starts_with("acme/widget-lib  "), "{out:?}");
        assert!(out[0].starts_with("Package"));
        assert!(out[1].starts_with("───"));
        let joined = out.join(" ");
        for word in ["acme/widget-lib", "more", "words"] {
            assert!(joined.contains(word), "{word} in {out:?}");
        }
    }

    #[test]
    fn a_table_with_no_room_for_columns_becomes_a_block_per_row() {
        let md = "| Package | Change | Age | Confidence |\n|---|---|---|---|\n| acme | 1 -> 2 | 3d | high |\n| widget | 4 -> 5 | 9d | low |";
        let out = at(md, 14);
        assert_eq!(
            out,
            [
                "Package: acme",
                "Change: 1 -> 2",
                "Age: 3d",
                "Confidence:",
                "high",
                "",
                "Package:",
                "widget",
                "Change: 4 -> 5",
                "Age: 9d",
                "Confidence:",
                "low",
            ]
        );
        let s = style_of("| Head |\n|---|\n| v |", "Head");
        assert!(s.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn table_cells_hold_code_links_images_and_emphasis() {
        let md = "| A | B |\n|---|---|\n| `code` [l](https://x/1) | ![img](https://i) **b** *i* |";
        let m = rendered(md, 60);
        assert_eq!(at(md, 60)[2], "code l  img b i");
        assert_eq!(m.links.len(), 1);
    }

    #[test]
    fn other_blocks_keep_their_text() {
        assert_eq!(
            at("term\n: definition\n\nnote[^1]\n\n[^1]: the footnote", 60),
            [
                "term : definition",
                "",
                "note[^1]",
                "",
                "[^1]: the footnote"
            ]
        );
    }

    #[test]
    fn no_line_is_wider_than_the_panel_at_any_width() {
        fits(SAMPLE);
        fits(
            "- a very long item with averyveryverylongunbreakableword inside\n  - nested averyveryverylongunbreakableword\n\n> quote averyveryverylongunbreakableword\n\n```\nlong code line long code line long code line\n```\n\n| a | b |\n|---|---|\n| averyverylongcell | another long cell |",
        );
    }

    #[test]
    fn the_update_description_reads_as_columns_wide_and_degrades_narrow() {
        for w in [40u16, 100] {
            let m = rendered(SAMPLE, w);
            let text = plain(&mono(), SAMPLE, w);
            for l in &m.lines {
                let n: usize = l.spans.iter().map(|s| width_of(&s.content)).sum();
                assert!(n <= w as usize, "{n} > {w}: {l:?}");
            }
            for gone in [
                "<!--",
                "update-bot",
                "debug",
                "eyJ",
                "<details",
                "<summary",
                "▸",
                "▾",
                "](http",
                "img/",
            ] {
                assert!(!text.contains(gone), "{gone} in {text}");
            }
            assert!(text.contains("acme/widget-lib (widget-lib)"));
            assert!(text.contains("v1.2.3"));
            assert!(text.contains("📅 Schedule: Branch creation"));
            assert_eq!(
                m.links.iter().map(|l| l.url.as_str()).collect::<Vec<_>>(),
                [
                    "https://example.com/widget-lib",
                    "https://example.com/age",
                    "https://example.com/conf",
                    "https://example.com/gadget-kit",
                    "https://example.com/age2",
                    "https://example.com/conf2",
                    "https://example.com/v1.2.3",
                    "https://example.com/team",
                ]
            );
        }
        let wide = at(SAMPLE, 100);
        assert!(
            wide.iter()
                .any(|l| l.contains("acme/widget-lib  1.2.2 -> 1.2.3  age  confidence")),
            "{wide:?}"
        );
        let narrow = at(SAMPLE, 40);
        assert!(
            narrow
                .iter()
                .any(|l| l.starts_with("Package") && l.contains("Confidence")),
            "{narrow:?}"
        );
        assert!(narrow.iter().filter(|l| l.contains("acme/")).count() >= 2);
    }

    fn snapshot(w: u16) -> String {
        plain(&mono(), SAMPLE, w)
    }

    #[test]
    fn markdown_80() {
        insta::assert_snapshot!(snapshot(80));
    }

    #[test]
    fn markdown_160() {
        insta::assert_snapshot!(snapshot(160));
    }
}
