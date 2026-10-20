//! Rich text: one document model for text written in a service's own
//! format (markdown, a tracker's JSON documents, a chat's markup), and its
//! layout as lines for a panel.
//!
//! A part converts its format into a [`Doc`] (blocks of paragraphs,
//! headings, lists, quotes, call-outs, code, tables, rules, folded details
//! and media, holding inline runs of text with marks, links, mentions, emoji
//! and badges), and [`layout`] lays it out for a width with a [`Look`]: the
//! styles each kind of run is drawn in, which the part fills from its own
//! look words. [`super::markdown`] parses markdown into a `Doc`.
//!
//! Every returned line is at most `width` cells, so a wrapping `Paragraph`
//! never re-wraps it and the line count is the height. Links are shown as
//! their words and collected, in order, once per url.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::design::theme::Slot;
use crate::plugin::Tone;
use crate::theme::Theme;
use crate::widgets;

pub use crate::view::doc::*;

// ---------------------------------------------------------------------
// The look.

/// The styles a document is drawn in. [`Look::new`] gives the kit's; a
/// part sets the fields its look words choose (a chat part's `mention`,
/// `link`, `emoji`).
#[derive(Debug, Clone, Copy)]
pub struct Look {
    pub text: Style,
    pub heading: Style,
    /// A folded section's summary.
    pub summary: Style,
    /// Quoted text.
    pub quote: Style,
    /// The bar before quoted lines.
    pub bar: Style,
    /// Code, inline and in blocks.
    pub code: Style,
    pub link: Style,
    pub mention: Style,
    /// A mention of the reader.
    pub me: Style,
    pub emoji: Style,
    /// An image's words in a line, a file's name.
    pub muted: Style,
    /// List markers, table headings given as labels.
    pub marker: Style,
    /// Rules and the line under a table's headings.
    pub rule: Style,
    /// Show a link's address after its words, where they differ: for a
    /// terminal that cannot open the words themselves.
    pub urls: bool,
    /// Badges and call-out bars, by tone.
    tones: [Style; 7],
    badges: [Style; 7],
}

const TONES: [Tone; 7] = [
    Tone::Plain,
    Tone::Muted,
    Tone::Accent,
    Tone::Info,
    Tone::Good,
    Tone::Warn,
    Tone::Bad,
];

fn tone_at(t: Tone) -> usize {
    TONES.iter().position(|x| *x == t).unwrap_or(0)
}

impl Look {
    pub fn new(t: &Theme) -> Self {
        Self {
            text: t.s(Slot::TextPrimary),
            heading: t.bold(Slot::TextEmphasis),
            summary: t.bold(Slot::TextSecondary),
            quote: t.s(Slot::TextSecondary),
            bar: t.s(Slot::AccentMuted),
            code: t.s(Slot::AccentSecondary),
            link: t.s(Slot::AccentPrimary).add_modifier(Modifier::UNDERLINED),
            mention: t.s(Slot::AccentTertiary),
            me: t.bold(Slot::StatusWarning),
            emoji: t.s(Slot::TextSecondary),
            muted: t.s(Slot::TextMuted),
            marker: t.s(Slot::TextSecondary),
            rule: t.s(Slot::TextMuted),
            urls: false,
            tones: TONES.map(|x| widgets::tone(t, x)),
            badges: TONES.map(|x| widgets::chip_style(t, x)),
        }
    }

    /// The style of `tone`, for a call-out's bar.
    pub fn tone(&self, tone: Tone) -> Style {
        self.tones[tone_at(tone)]
    }

    /// A badge of `tone`: as the kit's chips are drawn.
    pub fn badge(&self, tone: Tone) -> Style {
        self.badges[tone_at(tone)]
    }
}

// ---------------------------------------------------------------------
// Layout.

/// A document laid out: its lines and the links in it, in order, one per
/// url; and where its images sit, when the terminal draws them.
#[derive(Debug, Clone, Default)]
pub struct Laid {
    pub lines: Vec<Line<'static>>,
    pub links: Vec<Link>,
    /// Images to draw over the rows kept for them ([`super::image`]).
    pub media: Vec<Placed>,
}

/// An image's place in laid-out lines: from line `line`, `x` cells in,
/// `cols` by `rows` cells, drawn from `src`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub line: usize,
    pub x: u16,
    pub cols: u16,
    pub rows: u16,
    pub src: String,
}

impl Placed {
    /// The same place once its lines are put `down` lines lower and
    /// `right` cells further in (a header above them, an indent).
    pub fn moved(mut self, down: usize, right: u16) -> Self {
        self.line += down;
        self.x += right;
        self
    }
}

/// `doc` laid out for a panel `width` cells wide, in `look`.
pub fn layout(doc: &Doc, look: &Look, width: u16) -> Laid {
    let mut l = Lay::new(look, width as usize);
    l.blocks(&doc.blocks);
    l.finish()
}

/// The words of the same layout, its lines joined with `\n`: for copying,
/// a command line's output, a test.
pub fn plain(doc: &Doc, width: u16) -> String {
    let look = Look::new(&Theme::new(crate::design::theme::Tier::Mono));
    text_of(&layout(doc, &look, width).lines)
}

/// Lines as their words, joined with `\n`.
pub fn text_of(lines: &[Line<'_>]) -> String {
    lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Narrowest a table column is squeezed to before the table is laid out as a
/// block per row instead.
const MIN_COLUMN: usize = 6;
/// The longest a horizontal rule is drawn.
const RULE: usize = 24;

/// What a run of inline text is made of before it is wrapped.
#[derive(Debug, Clone)]
enum Piece {
    Text(String, Style),
    Space(Style),
    Break,
}

type Cell = Vec<Piece>;

fn width_of(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

fn push_span(spans: &mut Vec<Span<'static>>, text: &str, style: Style) {
    if let Some(last) = spans.last_mut()
        && last.style == style
    {
        last.content.to_mut().push_str(text);
        return;
    }
    spans.push(Span::styled(text.to_string(), style));
}

/// `text` as words and spaces.
fn text_pieces(text: &str, style: Style) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut in_space = false;
    for c in text.chars() {
        if c.is_whitespace() {
            if !word.is_empty() {
                out.push(Piece::Text(std::mem::take(&mut word), style));
            }
            if !in_space {
                out.push(Piece::Space(style));
            }
            in_space = true;
        } else {
            in_space = false;
            word.push(c);
        }
    }
    if !word.is_empty() {
        out.push(Piece::Text(word, style));
    }
    out
}

fn trim_pieces(p: &mut Vec<Piece>) {
    while matches!(p.last(), Some(Piece::Space(_) | Piece::Break)) {
        p.pop();
    }
    let lead = p
        .iter()
        .take_while(|x| matches!(x, Piece::Space(_) | Piece::Break))
        .count();
    p.drain(..lead);
}

struct Acc {
    lines: Vec<Vec<Span<'static>>>,
    cur: Vec<Span<'static>>,
    w: usize,
}

impl Acc {
    fn put(&mut self, text: &str, style: Style, w: usize) {
        push_span(&mut self.cur, text, style);
        self.w += w;
    }

    fn newline(&mut self) {
        self.lines.push(std::mem::take(&mut self.cur));
        self.w = 0;
    }
}

fn place(
    acc: &mut Acc,
    word: &mut Vec<(String, Style)>,
    ww: &mut usize,
    space: &mut Option<Style>,
    room: usize,
) {
    if word.is_empty() {
        return;
    }
    let sp = usize::from(acc.w > 0 && space.is_some());
    if acc.w + sp + *ww <= room || (*ww <= room && acc.w == 0) {
        if let (1, Some(s)) = (sp, *space) {
            acc.put(" ", s, 1);
        }
        for (t, s) in word.drain(..) {
            acc.put(&t, s, width_of(&t));
        }
    } else if *ww <= room {
        acc.newline();
        for (t, s) in word.drain(..) {
            acc.put(&t, s, width_of(&t));
        }
    } else {
        // A word wider than the room: broken where it runs out.
        if acc.w > 0 {
            acc.newline();
        }
        for (t, s) in word.drain(..) {
            for (_, g, cw) in super::clusters(&t) {
                if acc.w > 0 && acc.w + cw > room {
                    acc.newline();
                }
                acc.put(g, s, cw);
            }
        }
    }
    *ww = 0;
    *space = None;
}

/// Pieces broken between words to lines at most `room` cells wide.
fn wrap(pieces: &[Piece], room: usize) -> Vec<Vec<Span<'static>>> {
    let room = room.max(1);
    let mut acc = Acc {
        lines: Vec::new(),
        cur: Vec::new(),
        w: 0,
    };
    let mut word: Vec<(String, Style)> = Vec::new();
    let mut ww = 0;
    let mut space: Option<Style> = None;
    for p in pieces {
        match p {
            Piece::Text(t, s) => {
                ww += width_of(t);
                word.push((t.clone(), *s));
            }
            Piece::Space(s) => {
                place(&mut acc, &mut word, &mut ww, &mut space, room);
                space = Some(*s);
            }
            Piece::Break => {
                place(&mut acc, &mut word, &mut ww, &mut space, room);
                acc.newline();
                space = None;
            }
        }
    }
    place(&mut acc, &mut word, &mut ww, &mut space, room);
    if !acc.cur.is_empty() {
        acc.newline();
    }
    acc.lines
}

fn spans_width(spans: &[Span<'static>]) -> usize {
    spans.iter().map(|s| width_of(&s.content)).sum()
}

/// `s` cut into chunks of at most `room` cells.
fn hard_chunks(s: &str, room: usize) -> Vec<String> {
    let join = |gs: &[(String, usize)]| gs.iter().map(|(g, _)| g.as_str()).collect::<String>();
    let mut out = Vec::new();
    let mut cur: Vec<(String, usize)> = Vec::new();
    let mut w = 0;
    for (_, g, cw) in super::clusters(s) {
        if w > 0 && w + cw > room {
            // After the last plain space, so no word is split; a word
            // wider than the room is cut where it reaches the edge.
            let at = cur.iter().rposition(|(g, _)| g == " ").filter(|&i| i > 0).map_or(cur.len(), |i| i + 1);
            let rest = cur.split_off(at);
            out.push(join(&cur).trim_end().to_string());
            cur = rest;
            w = cur.iter().map(|(_, w)| w).sum();
        }
        cur.push((g.to_string(), cw));
        w += cw;
    }
    if !cur.is_empty() {
        out.push(join(&cur));
    }
    out
}

/// A line cut to `width` cells, its trailing blanks gone.
fn finish_line(line: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
    let mut used = 0;
    let mut out: Vec<Span<'static>> = Vec::new();
    for s in line {
        if used >= width {
            break;
        }
        let sw = width_of(&s.content);
        if used + sw <= width {
            used += sw;
            out.push(s);
        } else {
            let mut cut = String::new();
            for (_, g, cw) in super::clusters(&s.content) {
                if used + cw > width {
                    break;
                }
                used += cw;
                cut.push_str(g);
            }
            out.push(Span::styled(cut, s.style));
            break;
        }
    }
    while let Some(last) = out.last_mut() {
        let t = last.content.trim_end().to_string();
        if t.is_empty() {
            out.pop();
        } else {
            last.content = t.into();
            break;
        }
    }
    out
}

fn blank_text(line: &Line<'_>) -> bool {
    line.spans.iter().all(|s| {
        s.content
            .chars()
            .all(|c| c.is_whitespace() || c == '│' || c == '▌')
    })
}

/// What the lines being written sit inside.
enum Ctx {
    Quote,
    /// A list item: its marker (written before its first line) and whether
    /// that is still to do. Later lines hang under the item's text.
    Item {
        marker: String,
        pending: bool,
    },
    /// A details body, indented once its summary has been shown.
    Details {
        body: bool,
    },
    /// A call-out: its bar's style.
    Callout(Style),
}

/// What a block's runs are, for their base style.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Body,
    Heading,
    Summary,
    TableHead,
}

struct Lay<'a> {
    look: &'a Look,
    width: usize,
    out: Vec<Line<'static>>,
    last_blank: bool,
    /// No blank line before the next block (it is first in a list item).
    no_sep: bool,
    ctx: Vec<Ctx>,
    /// How many lists deep.
    depth: usize,
    quote: usize,
    links: Vec<Link>,
    media: Vec<Placed>,
}

impl<'a> Lay<'a> {
    fn new(look: &'a Look, width: usize) -> Self {
        Self {
            look,
            width,
            out: Vec::new(),
            last_blank: false,
            no_sep: false,
            ctx: Vec::new(),
            depth: 0,
            quote: 0,
            links: Vec::new(),
            media: Vec::new(),
        }
    }

    fn finish(mut self) -> Laid {
        // The rows kept for an image are blank, and stay.
        let kept = self
            .media
            .iter()
            .map(|m| m.line + m.rows as usize)
            .max()
            .unwrap_or(0);
        while self.out.len() > kept && self.out.last().is_some_and(blank_text) {
            self.out.pop();
        }
        let lead = self.out.iter().take_while(|l| blank_text(l)).count();
        self.out.drain(..lead);
        for m in &mut self.media {
            m.line -= lead;
        }
        Laid {
            lines: self.out,
            links: self.links,
            media: self.media,
        }
    }

    // --- styles

    fn style_of(&self, i: &Inline, role: Role) -> Style {
        let l = self.look;
        let mut s = match role {
            Role::Heading => l.heading,
            Role::Summary => l.summary,
            _ if self.quote > 0 => l.quote,
            _ => l.text,
        };
        if i.marks.code {
            s = l.code;
        }
        match &i.kind {
            Kind::Text | Kind::Break => {}
            Kind::Link { url } => s = super::link::style(l.link, url),
            Kind::Mention { me: true } => s = l.me,
            Kind::Mention { me: false } => s = l.mention,
            Kind::Emoji => s = l.emoji,
            Kind::Image { .. } => s = l.muted,
            Kind::Badge { tone } => s = l.badge(*tone),
        }
        if i.marks.bold || role == Role::TableHead {
            s = s.add_modifier(Modifier::BOLD);
        }
        if i.marks.italic {
            s = s.add_modifier(Modifier::ITALIC);
        }
        if i.marks.strike {
            s = s.add_modifier(Modifier::CROSSED_OUT);
        }
        if i.marks.underline {
            s = s.add_modifier(Modifier::UNDERLINED);
        }
        s
    }

    fn pieces(&mut self, text: &[Inline], role: Role) -> Vec<Piece> {
        self.collect_links(text);
        let mut out = Vec::new();
        // The link whose words are being written, and its words so far.
        let mut open: Option<(&str, String)> = None;
        for (n, i) in text.iter().enumerate() {
            if let Kind::Link { url } = &i.kind {
                match &mut open {
                    Some((u, words)) if *u == url.as_str() => words.push_str(&i.text),
                    _ => open = Some((url, i.text.clone())),
                }
            }
            match &i.kind {
                Kind::Break => out.push(Piece::Break),
                Kind::Badge { .. } => {
                    let words = i.text.split_whitespace().collect::<Vec<_>>().join(" ");
                    out.push(Piece::Text(format!(" {words} "), self.style_of(i, role)));
                }
                _ => out.extend(text_pieces(&i.text, self.style_of(i, role))),
            }
            let next_url = match text.get(n + 1).map(|x| &x.kind) {
                Some(Kind::Link { url }) => Some(url.as_str()),
                _ => None,
            };
            if let Some((url, words)) = &open
                && next_url != Some(*url)
            {
                if self.look.urls && words.trim() != *url && !words.trim().is_empty() {
                    out.push(Piece::Space(self.look.muted));
                    let s = super::link::style(self.look.muted, url);
                    out.push(Piece::Text(format!("({url})"), s));
                }
                open = None;
            }
        }
        out
    }

    /// The links in `text`, each run of words with one url as one link.
    fn collect_links(&mut self, text: &[Inline]) {
        let mut run: Option<(String, String)> = None;
        for i in text.iter().map(Some).chain([None]) {
            let url = match i.map(|i| &i.kind) {
                Some(Kind::Link { url }) => Some(url),
                _ => None,
            };
            match (&mut run, url) {
                (Some((u, words)), Some(url)) if u == url => {
                    words.push_str(i.map_or("", |i| i.text.as_str()));
                    continue;
                }
                _ => {}
            }
            if let Some((url, words)) = run.take() {
                self.add_link(&words, url);
            }
            if let (Some(url), Some(i)) = (url, i) {
                run = Some((url.clone(), i.text.clone()));
            }
        }
    }

    fn add_link(&mut self, words: &str, url: String) {
        if url.is_empty() || self.links.iter().any(|l| l.url == url) {
            return;
        }
        let words = words.split_whitespace().collect::<Vec<_>>().join(" ");
        let text = if words.is_empty() { url.clone() } else { words };
        self.links.push(Link { text, url });
    }

    // --- lines out

    fn avail(&self) -> usize {
        let used: usize = self
            .ctx
            .iter()
            .map(|c| match c {
                Ctx::Quote | Ctx::Callout(_) => 2,
                Ctx::Item { marker, .. } => width_of(marker),
                Ctx::Details { body } => usize::from(*body) * 2,
            })
            .sum();
        self.width.saturating_sub(used).max(1)
    }

    /// What goes before a line: quote and call-out bars, item markers (the
    /// first line only) or the room they take, details' indent.
    fn prefix(&mut self, consume: bool) -> Vec<Span<'static>> {
        let l = self.look;
        let mut out = Vec::new();
        for c in &mut self.ctx {
            match c {
                Ctx::Quote => out.push(Span::styled("│ ", l.bar)),
                Ctx::Callout(s) => out.push(Span::styled("▌ ", *s)),
                Ctx::Item { marker, pending } => {
                    if *pending && consume {
                        out.push(Span::styled(marker.clone(), l.marker));
                        *pending = false;
                    } else {
                        out.push(Span::raw(" ".repeat(width_of(marker))));
                    }
                }
                Ctx::Details { body } => {
                    if *body {
                        out.push(Span::raw("  "));
                    }
                }
            }
        }
        out
    }

    fn blank_line(&mut self) {
        let line = self.prefix(false);
        let line = finish_line(line, self.width);
        self.out.push(Line::from(line));
        self.last_blank = true;
    }

    /// A blank line before a block, unless it would be the first, a second
    /// in a row, or the block opens a list item.
    fn sep(&mut self) {
        if !self.out.is_empty() && !self.last_blank && !self.no_sep {
            self.blank_line();
        }
    }

    fn emit(&mut self, lines: Vec<Vec<Span<'static>>>) {
        for spans in lines {
            if spans.iter().all(|s| s.content.trim().is_empty()) {
                self.blank_line();
                continue;
            }
            let mut line = self.prefix(true);
            line.extend(spans);
            let line = finish_line(line, self.width);
            self.out.push(Line::from(line));
            self.last_blank = false;
            self.no_sep = false;
        }
    }

    fn rule(&mut self, n: usize) {
        let s = self.look.rule;
        self.emit(vec![vec![Span::styled("─".repeat(n.max(1)), s)]]);
    }

    /// Pieces wrapped and written as one block; the widest line's width.
    fn write(&mut self, mut p: Vec<Piece>) -> usize {
        trim_pieces(&mut p);
        if p.is_empty() {
            return 0;
        }
        let lines = wrap(&p, self.avail());
        let widest = lines.iter().map(|l| spans_width(l)).max().unwrap_or(0);
        self.sep();
        self.emit(lines);
        widest
    }

    // --- blocks

    fn blocks(&mut self, blocks: &[Block]) {
        for b in blocks {
            self.block(b);
        }
    }

    fn block(&mut self, b: &Block) {
        match b {
            Block::Para { text } => {
                let p = self.pieces(text, Role::Body);
                self.write(p);
            }
            Block::Heading { level, text } => {
                let p = self.pieces(text, Role::Heading);
                let w = self.write(p);
                if *level == 1 && w > 0 {
                    self.rule(w);
                }
            }
            Block::List { items } => self.list(items),
            Block::Quote { blocks } => {
                self.sep();
                self.ctx.push(Ctx::Quote);
                self.quote += 1;
                self.blocks(blocks);
                self.quote -= 1;
                self.pop(|c| matches!(c, Ctx::Quote));
            }
            Block::Callout {
                tone,
                symbol,
                blocks,
            } => {
                self.sep();
                self.ctx.push(Ctx::Callout(self.look.tone(*tone)));
                let mut blocks = blocks.clone();
                if !symbol.is_empty() {
                    let mark = Inline::text(symbol.clone());
                    match blocks.first_mut() {
                        Some(Block::Para { text } | Block::Heading { text, .. }) => {
                            text.insert(0, mark);
                        }
                        _ => blocks.insert(0, Block::para(vec![mark])),
                    }
                }
                self.blocks(&blocks);
                self.pop(|c| matches!(c, Ctx::Callout(_)));
            }
            Block::Code { text, .. } => self.code(text),
            Block::Table { aligns, head, rows } => self.table(aligns, head, rows),
            Block::Rule => {
                self.sep();
                let n = RULE.min(self.avail());
                self.rule(n);
            }
            Block::Details { summary, blocks } => {
                self.ctx.push(Ctx::Details { body: false });
                let p = self.pieces(summary, Role::Summary);
                self.write(p);
                if let Some(Ctx::Details { body }) = self.ctx.last_mut() {
                    *body = true;
                }
                self.blocks(blocks);
                self.pop(|c| matches!(c, Ctx::Details { .. }));
            }
            Block::Media(m) => self.media(m),
        }
    }

    /// Drop the innermost context `is` matches, and those inside it.
    fn pop(&mut self, is: impl Fn(&Ctx) -> bool) {
        if let Some(i) = self.ctx.iter().rposition(is) {
            self.ctx.truncate(i);
        }
    }

    fn list(&mut self, items: &[Item]) {
        if self.depth == 0 {
            self.sep();
        }
        let top = self.depth == 0;
        self.depth += 1;
        for item in items {
            let marker = match item.marker {
                Marker::Bullet if top => "• ".to_string(),
                Marker::Bullet => "◦ ".to_string(),
                Marker::Number(n) => format!("{n}. "),
                Marker::Todo => "☐ ".to_string(),
                Marker::Done => "☑ ".to_string(),
                Marker::Decision => "◇ ".to_string(),
            };
            self.ctx.push(Ctx::Item {
                marker,
                pending: true,
            });
            self.no_sep = true;
            self.blocks(&item.blocks);
            // An item with nothing in it still shows its marker.
            if matches!(self.ctx.last(), Some(Ctx::Item { pending: true, .. })) {
                let line = finish_line(self.prefix(true), self.width);
                self.out.push(Line::from(line));
                self.last_blank = false;
            }
            self.pop(|c| matches!(c, Ctx::Item { .. }));
            self.no_sep = false;
        }
        self.depth -= 1;
    }

    fn media(&mut self, m: &Media) {
        let l = self.look;
        // Drawn as an image where the terminal can: its rows kept, its name
        // on the first until the image covers it.
        if let Some((cols, rows)) = super::image::fit(m, self.avail()) {
            if !m.url.is_empty() {
                self.add_link(&m.name, m.url.clone());
            }
            self.sep();
            let x = (self.width - self.avail()) as u16;
            let mut first = text_pieces("▣", l.muted);
            if !m.name.trim().is_empty() {
                first.push(Piece::Space(l.muted));
                first.extend(text_pieces(&m.name, l.muted));
            }
            let mut lines = wrap(&first, self.avail());
            lines.truncate(1);
            let line = self.out.len();
            self.emit(lines);
            for _ in 1..rows {
                let prefix = finish_line(self.prefix(false), self.width);
                self.out.push(Line::from(prefix));
            }
            self.last_blank = false;
            self.media.push(Placed {
                line,
                x,
                cols,
                rows,
                src: m.src.clone(),
            });
            return;
        }
        let label = if m.image { "▣ image" } else { "▤ file" };
        let mut p = text_pieces(label, l.marker);
        if !m.name.trim().is_empty() {
            p.push(Piece::Space(l.marker));
            p.extend(text_pieces(&m.name, l.marker));
        }
        if !m.url.is_empty() {
            p.push(Piece::Space(l.muted));
            p.push(Piece::Text(
                m.url.clone(),
                super::link::style(l.link, &m.url),
            ));
            self.add_link(&m.name, m.url.clone());
        }
        self.write(p);
    }

    // --- code, tables

    fn code(&mut self, code: &str) {
        let code = code.replace('\r', "");
        let mut lines: Vec<&str> = code.lines().collect();
        while lines.last().is_some_and(|l| l.trim().is_empty()) {
            lines.pop();
        }
        if lines.is_empty() {
            return;
        }
        self.sep();
        let room = self.avail().saturating_sub(2).max(1);
        let style = self.look.code;
        let mut rows: Vec<Vec<Span<'static>>> = Vec::new();
        for l in lines {
            let l = l.replace('\t', "    ");
            let l = l.trim_end();
            if l.is_empty() {
                rows.push(Vec::new());
                continue;
            }
            for chunk in hard_chunks(l, room) {
                rows.push(vec![Span::raw("  "), Span::styled(chunk, style)]);
            }
        }
        self.emit(rows);
    }

    fn table(&mut self, aligns: &[Align], head: &[Vec<Inline>], rows: &[Vec<Vec<Inline>>]) {
        let cells = |me: &mut Self, row: &[Vec<Inline>], role: Role| -> Vec<Cell> {
            row.iter()
                .map(|c| {
                    let mut p = me.pieces(c, role);
                    trim_pieces(&mut p);
                    p
                })
                .collect()
        };
        let head: Vec<Cell> = cells(self, head, Role::TableHead);
        let rows: Vec<Vec<Cell>> = rows.iter().map(|r| cells(self, r, Role::Body)).collect();
        let n = rows
            .iter()
            .map(Vec::len)
            .chain([head.len()])
            .max()
            .unwrap_or(0);
        if n == 0 {
            return;
        }
        let avail = self.avail();
        let natural = |cell: &Cell| {
            wrap(cell, usize::MAX / 4)
                .iter()
                .map(|l| spans_width(l))
                .max()
                .unwrap_or(0)
        };
        let column = |c: usize| {
            head.get(c)
                .into_iter()
                .chain(rows.iter().filter_map(move |r| r.get(c)))
        };
        let nat: Vec<usize> = (0..n)
            .map(|c| column(c).map(natural).max().unwrap_or(0).max(1))
            .collect();
        let mins: Vec<usize> = nat.iter().map(|w| (*w).min(MIN_COLUMN)).collect();
        let gaps = 2 * (n - 1);
        self.sep();
        if mins.iter().sum::<usize>() + gaps > avail {
            self.table_blocks(&head, &rows);
            return;
        }
        // The widest column gives way first, down to its longest word (a
        // word is cut only when that is not enough), never below the minimum.
        let longest = |cell: &Cell| {
            cell.iter()
                .map(|p| match p {
                    Piece::Text(s, _) => width_of(s),
                    _ => 0,
                })
                .max()
                .unwrap_or(0)
        };
        let keeps: Vec<usize> = (0..n)
            .map(|c| {
                let word = column(c).map(longest).max().unwrap_or(0);
                word.max(mins[c]).min(nat[c])
            })
            .collect();
        let mut ws = nat;
        for floors in [&keeps, &mins] {
            while ws.iter().sum::<usize>() + gaps > avail {
                let widest = ws
                    .iter()
                    .enumerate()
                    .filter(|(i, w)| **w > floors[*i])
                    .max_by_key(|(_, w)| **w)
                    .map(|(i, _)| i);
                match widest {
                    Some(i) => ws[i] -= 1,
                    None => break,
                }
            }
        }
        if !head.is_empty() {
            let lines = row_lines(&head, &ws, aligns);
            self.emit(lines);
            let rule = ws
                .iter()
                .map(|w| "─".repeat(*w))
                .collect::<Vec<_>>()
                .join("  ");
            let s = self.look.rule;
            self.emit(vec![vec![Span::styled(rule, s)]]);
        }
        for row in &rows {
            let lines = row_lines(row, &ws, aligns);
            self.emit(lines);
        }
    }

    /// A table with no room for columns: a block per row, `Heading: value`.
    fn table_blocks(&mut self, head: &[Cell], rows: &[Vec<Cell>]) {
        let muted = self.look.rule;
        let avail = self.avail();
        for (r, row) in rows.iter().enumerate() {
            if r > 0 {
                self.blank_line();
            }
            for (c, cell) in row.iter().enumerate() {
                if cell.is_empty() {
                    continue;
                }
                let label: String = head
                    .get(c)
                    .map(|h| {
                        h.iter()
                            .map(|p| match p {
                                Piece::Text(s, _) => s.as_str(),
                                _ => " ",
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let mut p = Vec::new();
                if !label.trim().is_empty() {
                    p.push(Piece::Text(format!("{}:", label.trim()), muted));
                    p.push(Piece::Space(muted));
                }
                p.extend(cell.iter().cloned());
                let lines = wrap(&p, avail);
                self.emit(lines);
            }
        }
    }
}

/// One table row as lines: each cell wrapped in its column, aligned, the
/// columns two spaces apart.
fn row_lines(cells: &[Cell], ws: &[usize], aligns: &[Align]) -> Vec<Vec<Span<'static>>> {
    let wrapped: Vec<Vec<Vec<Span<'static>>>> = ws
        .iter()
        .enumerate()
        .map(|(c, w)| cells.get(c).map(|cell| wrap(cell, *w)).unwrap_or_default())
        .collect();
    let height = wrapped.iter().map(Vec::len).max().unwrap_or(0).max(1);
    (0..height)
        .map(|i| {
            let mut line: Vec<Span<'static>> = Vec::new();
            for (c, w) in ws.iter().enumerate() {
                if c > 0 {
                    line.push(Span::raw("  "));
                }
                let seg = wrapped[c].get(i).cloned().unwrap_or_default();
                let pad = w.saturating_sub(spans_width(&seg));
                let (left, right) = match aligns.get(c) {
                    Some(Align::Right) => (pad, 0),
                    Some(Align::Center) => (pad / 2, pad - pad / 2),
                    _ => (0, pad),
                };
                if left > 0 {
                    line.push(Span::raw(" ".repeat(left)));
                }
                line.extend(seg);
                if right > 0 {
                    line.push(Span::raw(" ".repeat(right)));
                }
            }
            line
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_are_available_without_layout_and_keep_document_order() {
        let doc = Doc::new(vec![
            Block::para(vec![Inline::link("first", "https://example.test/one")]),
            Block::Quote {
                blocks: vec![Block::para(vec![
                    Inline::link("again", "https://example.test/one"),
                    Inline::link("second", "https://example.test/two"),
                ])],
            },
            Block::Media(Media {
                name: "attachment".into(),
                url: "https://example.test/file".into(),
                ..Media::default()
            }),
        ]);
        assert_eq!(
            doc.links()
                .iter()
                .map(|link| link.text.as_str())
                .collect::<Vec<_>>(),
            ["first", "second", "attachment"]
        );
    }
    use crate::design::theme::Tier;

    fn para(text: Vec<Inline>) -> Doc {
        Doc::new(vec![Block::para(text)])
    }

    fn at(doc: &Doc, w: u16) -> Vec<String> {
        plain(doc, w).lines().map(String::from).collect()
    }

    fn style_of(doc: &Doc, look: &Look, word: &str) -> Style {
        layout(doc, look, 60)
            .lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.contains(word))
            .unwrap_or_else(|| panic!("no span with {word}"))
            .style
    }

    #[test]
    fn runs_take_the_look_a_part_gives_them() {
        let t = Theme::new(Tier::TrueColor);
        let mut look = Look::new(&t);
        look.mention = t.s(Slot::StatusInfo);
        look.me = t.bold(Slot::StatusWarning);
        look.emoji = t.s(Slot::TextMuted);
        look.link = t.s(Slot::DataNumber);
        let doc = para(vec![
            Inline::mention("@alice", false),
            Inline::text(" and "),
            Inline::mention("@me", true),
            Inline::text(" "),
            Inline::emoji(":party-parrot:"),
            Inline::text(" "),
            Inline::link("plan", "https://x/plan"),
            Inline::text(" "),
            Inline::text("loud").bold(),
        ]);
        assert_eq!(style_of(&doc, &look, "@alice"), look.mention);
        assert_eq!(style_of(&doc, &look, "@me"), look.me);
        assert_eq!(style_of(&doc, &look, "parrot"), look.emoji);
        assert_eq!(style_of(&doc, &look, "plan"), look.link);
        assert!(
            style_of(&doc, &look, "loud")
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert_eq!(at(&doc, 80), ["@alice and @me :party-parrot: plan loud"]);
    }

    #[test]
    fn a_badge_is_a_chip_kept_whole_when_lines_break() {
        let doc = para(vec![
            Inline::text("state"),
            Inline::text(" "),
            Inline::badge("in   progress", Tone::Info),
            Inline::text(" now"),
        ]);
        assert_eq!(at(&doc, 40), ["state  in progress  now"]);
        assert_eq!(at(&doc, 16), ["state", " in progress", "now"]);
        let t = Theme::new(Tier::TrueColor);
        let look = Look::new(&t);
        assert_eq!(
            style_of(&doc, &look, "progress"),
            widgets::chip(&t, "x", Tone::Info).style
        );
    }

    #[test]
    fn a_callout_has_a_bar_and_its_symbol_on_the_first_line() {
        let doc = Doc::new(vec![
            Block::para(vec![Inline::text("before")]),
            Block::Callout {
                tone: Tone::Warn,
                symbol: "⚠ ".into(),
                blocks: vec![
                    Block::para(vec![Inline::text("mind the step that wraps")]),
                    Block::para(vec![Inline::text("two")]),
                ],
            },
            Block::para(vec![Inline::text("after")]),
        ]);
        assert_eq!(
            at(&doc, 16),
            [
                "before",
                "",
                "▌ ⚠ mind the",
                "▌ step that",
                "▌ wraps",
                "▌",
                "▌ two",
                "",
                "after"
            ]
        );
        let t = Theme::new(Tier::TrueColor);
        let look = Look::new(&t);
        let bar = &layout(&doc, &look, 40).lines[2].spans[0];
        assert_eq!(bar.style, look.tone(Tone::Warn));
    }

    #[test]
    fn lists_take_every_marker_and_an_empty_item_shows_its_marker() {
        let item = |marker, text: &str| Item {
            marker,
            blocks: if text.is_empty() {
                vec![]
            } else {
                vec![Block::para(vec![Inline::text(text)])]
            },
        };
        let doc = Doc::new(vec![Block::List {
            items: vec![
                item(Marker::Todo, "write"),
                item(Marker::Done, "ship"),
                item(Marker::Decision, "use the kit"),
                item(Marker::Bullet, ""),
                Item {
                    marker: Marker::Number(9),
                    blocks: vec![Block::List {
                        items: vec![item(Marker::Bullet, "inner")],
                    }],
                },
            ],
        }]);
        assert_eq!(
            at(&doc, 40),
            ["☐ write", "☑ ship", "◇ use the kit", "•", "9. ◦ inner"]
        );
    }

    #[test]
    fn media_without_graphics_is_a_line_naming_it_and_its_link() {
        let doc = Doc::new(vec![
            Block::Media(Media {
                image: true,
                name: "screen shot.png".into(),
                url: "https://x/f/1".into(),
                src: "https://files/1".into(),
                size: Some((800, 600)),
            }),
            Block::Media(Media {
                name: "notes.csv".into(),
                ..Media::default()
            }),
        ]);
        assert_eq!(
            at(&doc, 60),
            [
                "▣ image screen shot.png https://x/f/1",
                "",
                "▤ file notes.csv"
            ]
        );
        let t = Theme::new(Tier::TrueColor);
        let laid = layout(&doc, &Look::new(&t), 60);
        assert_eq!(
            laid.links,
            [Link {
                text: "screen shot.png".into(),
                url: "https://x/f/1".into()
            }]
        );
    }

    #[test]
    fn links_are_collected_by_their_run_of_words() {
        let doc = para(vec![
            Inline::link("the ", "https://x/a"),
            Inline::link("plan", "https://x/a").bold(),
            Inline::text(" and "),
            Inline::link("", "https://x/b"),
            Inline::link("again", "https://x/a"),
        ]);
        let laid = layout(&doc, &Look::new(&Theme::new(Tier::Mono)), 60);
        assert_eq!(
            laid.links,
            [
                Link {
                    text: "the plan".into(),
                    url: "https://x/a".into()
                },
                Link {
                    text: "https://x/b".into(),
                    url: "https://x/b".into()
                },
            ]
        );
    }

    #[test]
    fn a_look_can_show_where_links_go_after_their_words() {
        let doc = para(vec![
            Inline::text("see "),
            Inline::link("the ", "https://x/a"),
            Inline::link("plan", "https://x/a").bold(),
            Inline::text(", "),
            Inline::link("https://x/b", "https://x/b"),
            Inline::text(" and "),
            Inline::link("notes", "https://x/c"),
        ]);
        let t = Theme::new(Tier::Mono);
        let mut look = Look::new(&t);
        assert_eq!(
            text_of(&layout(&doc, &look, 80).lines),
            "see the plan, https://x/b and notes"
        );
        look.urls = true;
        assert_eq!(
            text_of(&layout(&doc, &look, 80).lines),
            "see the plan (https://x/a), https://x/b and notes (https://x/c)"
        );
    }

    #[test]
    fn with_links_on_their_words_carry_where_they_go_and_the_text_is_the_same() {
        let doc = Doc::new(vec![
            Block::para(vec![
                Inline::text("see "),
                Inline::link("the plan", "https://x/plan"),
            ]),
            Block::Media(Media {
                name: "f.csv".into(),
                url: "https://x/f.csv".into(),
                ..Media::default()
            }),
        ]);
        let look = Look::new(&Theme::new(Tier::TrueColor));
        let before = layout(&doc, &look, 40);
        super::super::link::set(true);
        let after = layout(&doc, &look, 40);
        super::super::link::set(false);
        assert_eq!(text_of(&before.lines), text_of(&after.lines));
        let carried = |word: &str| {
            after
                .lines
                .iter()
                .flat_map(|l| l.spans.iter())
                .find(|s| s.content.contains(word))
                .and_then(|s| s.style.underline_color)
        };
        assert!(carried("plan").is_some());
        assert!(carried("https://x/f.csv").is_some());
        assert_eq!(carried("see"), None);
        assert!(
            before
                .lines
                .iter()
                .flat_map(|l| l.spans.iter())
                .all(|s| s.style.underline_color.is_none())
        );
    }

    #[test]
    fn known_emoji_become_one_wide_glyph_and_the_rest_keep_their_name() {
        assert_eq!(emoji("tada").as_deref(), Some("🎉"));
        assert_eq!(emoji(":+1:").as_deref(), Some("👍"));
        assert_eq!(emoji("white_check_mark").as_deref(), Some("✅"));
        assert_eq!(emoji("no-such-emoji-here"), None);
        // A flag is two characters, a family is joined: neither is safe.
        assert_eq!(emoji("jp"), None);
        assert_eq!(emoji("family_man_woman_girl"), None);
        // A text symbol made an emoji by a selector is two cells only where
        // the terminal agrees; not risked.
        assert_eq!(safe_emoji("❤\u{fe0f}"), None);
        assert_eq!(safe_emoji("😄"), Some("😄".into()));
        for e in [emoji("tada").unwrap(), emoji("+1").unwrap()] {
            assert_eq!(UnicodeWidthStr::width(e.as_str()), 2);
        }
    }

    #[test]
    fn a_document_survives_a_round_trip_through_json() {
        let doc = Doc::new(vec![
            Block::Heading {
                level: 2,
                text: vec![Inline::text("Plan")],
            },
            Block::para(vec![
                Inline::mention("@a", true),
                Inline::badge("done", Tone::Good),
                Inline::line_break(),
                Inline::text("x").code(),
            ]),
            Block::Table {
                aligns: vec![Align::Right],
                head: vec![vec![Inline::text("n")]],
                rows: vec![vec![vec![Inline::text("1")]]],
            },
            Block::Media(Media {
                image: true,
                src: "s".into(),
                ..Media::default()
            }),
        ]);
        let v = serde_json::to_value(&doc).unwrap();
        assert_eq!(
            v[1]["text"][0],
            serde_json::json!({"text": "@a", "kind": {"mention": {"me": true}}})
        );
        let back: Doc = serde_json::from_value(v).unwrap();
        assert_eq!(back, doc);
        assert!(Doc::new(vec![Block::para(vec![Inline::text("  ")])]).is_empty());
    }

    #[test]
    fn no_line_is_wider_than_the_panel() {
        let doc = Doc::new(vec![
            Block::Callout {
                tone: Tone::Bad,
                symbol: "✗ ".into(),
                blocks: vec![Block::Quote {
                    blocks: vec![Block::para(vec![
                        Inline::badge("a long badge with words", Tone::Bad),
                        Inline::text(" averyveryverylongunbreakableword"),
                    ])],
                }],
            },
            Block::Media(Media {
                image: true,
                name: "x".into(),
                url: format!("https://x/{}", "y".repeat(80)),
                ..Media::default()
            }),
        ]);
        for w in 1..=90u16 {
            for l in layout(&doc, &Look::new(&Theme::new(Tier::Mono)), w).lines {
                assert!(l.width() <= w as usize, "{w}: {l:?}");
            }
        }
    }
}
