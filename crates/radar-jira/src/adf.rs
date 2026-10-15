//! Atlassian Document Format (ADF) as the kit's rich text, and text typed in
//! radar to ADF (`from_text`, further down).
//!
//! ADF is the JSON Jira Cloud REST v3 uses for descriptions, comments and rich
//! custom fields. `doc` turns a document, a single node, or an old-style
//! plain-text string into a [`Doc`] the kit lays out: paragraphs, headings,
//! bullet, ordered, task and decision lists, code blocks, quotes, panels (a
//! call-out with the panel's feeling), tables, rules, expands (shown open),
//! media (an image or a file), cards, extensions, and the inline nodes (text
//! with its marks and links, mentions, emoji as glyphs, dates, statuses as
//! badges). Unknown nodes are walked for their content; odd input never
//! panics.

use radar_core::Tone;
use radar_core::ui::rich::{self, Block, Doc, Inline, Item as ListItem, Kind, Marker, Marks, Media};
use serde_json::Value;

/// An ADF document, a single ADF node, or a plain-text string, as rich text.
/// Anything else is an empty document.
pub fn doc(v: &Value) -> Doc {
    match v {
        Value::String(s) => Doc::new(text_blocks(s)),
        Value::Object(_) => Doc::new(block(v)),
        _ => Doc::default(),
    }
}

/// A file attached to a ticket: its name, where its content is (Jira's
/// REST address, fetched with the site's credentials) and whether it is a
/// picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attached {
    pub name: String,
    pub content: String,
    pub image: bool,
}

/// A ticket's attachments, from its issue JSON's `attachment` field.
pub fn attachments(fields: &Value) -> Vec<Attached> {
    fields
        .get("attachment")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|a| {
            Some(Attached {
                name: a.get("filename")?.as_str()?.to_string(),
                content: a.get("content")?.as_str()?.to_string(),
                image: a
                    .get("mimeType")
                    .and_then(Value::as_str)
                    .is_some_and(|m| m.starts_with("image/")),
            })
        })
        .collect()
}

/// The document's files and images found among `files` by name: each then
/// opens its content, and an image is drawn from it.
pub fn attach(doc: &mut Doc, files: &[Attached]) {
    fn walk(blocks: &mut [Block], files: &[Attached]) {
        for b in blocks {
            match b {
                Block::Media(m) if m.src.is_empty() && m.url.is_empty() => {
                    if let Some(f) = files.iter().find(|f| f.name == m.name) {
                        m.url = f.content.clone();
                        if f.image {
                            m.image = true;
                            m.src = f.content.clone();
                        }
                    }
                }
                Block::Quote { blocks } | Block::Callout { blocks, .. } | Block::Details { blocks, .. } => {
                    walk(blocks, files)
                }
                Block::List { items } => {
                    for i in items {
                        walk(&mut i.blocks, files);
                    }
                }
                _ => {}
            }
        }
    }
    if !files.is_empty() {
        walk(&mut doc.blocks, files);
    }
}

/// The document's words as the page lays them out, at a width nothing
/// wraps in: for tests.
#[cfg(test)]
pub fn plain(v: &Value) -> String {
    rich::plain(&doc(v), 1000)
}

/// Old-style text: each run of lines between blank ones a paragraph, its
/// line breaks kept.
fn text_blocks(s: &str) -> Vec<Block> {
    let s = s.replace('\r', "");
    let mut out = Vec::new();
    for para in s.split("\n\n") {
        let mut runs = Vec::new();
        for (i, l) in para.lines().filter(|l| !l.trim().is_empty()).enumerate() {
            if i > 0 {
                runs.push(Inline::line_break());
            }
            runs.push(Inline::text(l.trim_end()));
        }
        if !runs.is_empty() {
            out.push(Block::para(runs));
        }
    }
    out
}

// ---- helpers ----

fn node_type(node: &Value) -> &str {
    node.get("type").and_then(Value::as_str).unwrap_or("")
}

fn content(node: &Value) -> &[Value] {
    node.get("content")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn attr<'a>(node: &'a Value, key: &str) -> Option<&'a Value> {
    node.get("attrs").and_then(|a| a.get(key))
}

fn attr_str<'a>(node: &'a Value, key: &str) -> Option<&'a str> {
    attr(node, key).and_then(Value::as_str)
}

fn attr_nonempty<'a>(node: &'a Value, key: &str) -> Option<&'a str> {
    attr_str(node, key).filter(|s| !s.trim().is_empty())
}

fn is_inline_type(t: &str) -> bool {
    matches!(
        t,
        "text"
            | "hardBreak"
            | "mention"
            | "emoji"
            | "date"
            | "status"
            | "inlineCard"
            | "mediaInline"
            | "inlineExtension"
            | "placeholder"
    )
}

#[cfg(test)]
fn is_list_type(t: &str) -> bool {
    matches!(
        t,
        "bulletList" | "orderedList" | "taskList" | "decisionList"
    )
}

// ---- inline ----

fn runs(node: &Value) -> Vec<Inline> {
    let mut out = Vec::new();
    for c in content(node) {
        inline(c, &mut out);
    }
    out
}

fn inline(node: &Value, out: &mut Vec<Inline>) {
    match node_type(node) {
        "text" => out.push(text_node(node)),
        "hardBreak" => out.push(Inline::line_break()),
        "mention" => {
            let t = attr_nonempty(node, "text").unwrap_or("someone");
            let t = if t.starts_with('@') { t.to_string() } else { format!("@{t}") };
            out.push(Inline::mention(t, false));
        }
        "emoji" => {
            let short = attr_nonempty(node, "shortName");
            let glyph = attr_nonempty(node, "text")
                .and_then(rich::safe_emoji)
                .or_else(|| short.and_then(rich::emoji));
            match (glyph, short.or_else(|| attr_nonempty(node, "text"))) {
                (Some(g), _) => out.push(Inline::emoji(g)),
                (None, Some(name)) => out.push(Inline::emoji(name)),
                (None, None) => {}
            }
        }
        "date" => {
            if let Some(d) = date(node) {
                out.push(Inline::text(d));
            }
        }
        "status" => {
            let text = attr_str(node, "text").unwrap_or("").to_uppercase();
            if !text.trim().is_empty() {
                out.push(Inline::badge(text, status_tone(attr_str(node, "color"))));
            }
        }
        "placeholder" => {}
        "inlineCard" => {
            let url = card_url(node);
            if !url.is_empty() {
                out.push(Inline::link(url.clone(), url));
            }
        }
        "mediaInline" => {
            let name = attr_nonempty(node, "alt").or_else(|| attr_nonempty(node, "name"));
            out.push(Inline {
                text: format!("▤ {}", name.unwrap_or("attachment")),
                kind: Kind::Image { src: String::new() },
                marks: Marks::default(),
            });
        }
        "media" => out.push(Inline::text(media(node, false).name)),
        "inlineExtension" | "extension" | "bodiedExtension" => {
            out.push(Inline::text(macro_text(node)));
        }
        _ => {
            if let Some(t) = node.get("text").and_then(Value::as_str) {
                out.push(Inline::text(t.replace('\r', "")));
            } else {
                for c in content(node) {
                    inline(c, out);
                }
            }
        }
    }
}

/// A status's colour as a feeling.
fn status_tone(colour: Option<&str>) -> Tone {
    match colour {
        Some("green") => Tone::Good,
        Some("yellow") => Tone::Warn,
        Some("red") => Tone::Bad,
        Some("blue") => Tone::Info,
        Some("purple") => Tone::Accent,
        _ => Tone::Muted,
    }
}

fn text_node(node: &Value) -> Inline {
    let text = node
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .replace('\r', "");
    let marks_of = node
        .get("marks")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let has = |name: &str| marks_of.iter().find(|m| node_type(m) == name);
    let marks = Marks {
        bold: has("strong").is_some(),
        italic: has("em").is_some(),
        strike: has("strike").is_some(),
        underline: has("underline").is_some(),
        code: has("code").is_some(),
    };
    let kind = match has("link").and_then(|m| attr_nonempty(m, "href")) {
        Some(href) => Kind::Link { url: href.to_string() },
        None => Kind::Text,
    };
    Inline { text, kind, marks }
}

fn date(node: &Value) -> Option<String> {
    let ts = attr(node, "timestamp")?;
    let ms: i64 = match ts {
        Value::String(s) => s.trim().parse().ok()?,
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64))?,
        _ => return None,
    };
    radar_core::time::to_rfc3339(ms.div_euclid(1000))
        .get(..10)
        .map(str::to_string)
}

fn card_url(node: &Value) -> String {
    attr_str(node, "url")
        .or_else(|| {
            attr(node, "data")
                .and_then(|d| d.get("url"))
                .and_then(Value::as_str)
        })
        .unwrap_or("")
        .to_string()
}

/// A media node: an external image by its address, or a file kept by Jira
/// by its name (its address is found from the ticket's attachments).
fn media(node: &Value, image: bool) -> Media {
    let size = match (
        attr(node, "width").and_then(Value::as_u64),
        attr(node, "height").and_then(Value::as_u64),
    ) {
        (Some(w), Some(h)) if w > 0 && h > 0 => Some((w as u32, h as u32)),
        _ => None,
    };
    if attr_str(node, "type") == Some("external")
        && let Some(url) = attr_nonempty(node, "url")
    {
        return Media {
            image: true,
            name: attr_nonempty(node, "alt").unwrap_or_default().to_string(),
            url: url.to_string(),
            src: url.to_string(),
            size,
        };
    }
    Media {
        image,
        name: attr_nonempty(node, "alt")
            .unwrap_or("attachment")
            .to_string(),
        size,
        ..Media::default()
    }
}

fn macro_text(node: &Value) -> String {
    match attr_nonempty(node, "extensionKey") {
        Some(k) => format!("[macro: {k}]"),
        None => "[macro]".to_string(),
    }
}

// ---- blocks ----

fn blocks(nodes: &[Value]) -> Vec<Block> {
    nodes.iter().flat_map(block).collect()
}

fn para(node: &Value) -> Vec<Block> {
    let r = runs(node);
    if r.is_empty() {
        Vec::new()
    } else {
        vec![Block::para(r)]
    }
}

fn block(node: &Value) -> Vec<Block> {
    let t = node_type(node);
    match t {
        "paragraph" => para(node),
        "heading" => {
            let level = attr(node, "level")
                .and_then(Value::as_u64)
                .unwrap_or(1)
                .clamp(1, 6) as u8;
            vec![Block::Heading {
                level,
                text: runs(node),
            }]
        }
        "bulletList" | "orderedList" => vec![list(node)],
        "taskList" | "decisionList" => vec![task_list(node)],
        "codeBlock" => {
            let mut text = String::new();
            for c in content(node) {
                if let Some(t) = c.get("text").and_then(Value::as_str) {
                    text.push_str(t);
                }
            }
            vec![Block::Code {
                lang: attr_str(node, "language").unwrap_or_default().to_string(),
                text,
            }]
        }
        "blockquote" => vec![Block::Quote {
            blocks: blocks(content(node)),
        }],
        "panel" => {
            let (tone, symbol) = match attr_str(node, "panelType") {
                Some("info") => (Tone::Info, "ℹ "),
                Some("note") => (Tone::Accent, "✎ "),
                Some("warning") => (Tone::Warn, "⚠ "),
                Some("error") => (Tone::Bad, "✗ "),
                Some("success") => (Tone::Good, "✓ "),
                _ => (Tone::Muted, ""),
            };
            vec![Block::Callout {
                tone,
                symbol: symbol.to_string(),
                blocks: blocks(content(node)),
            }]
        }
        "table" => vec![table(node)],
        "rule" => vec![Block::Rule],
        "expand" | "nestedExpand" => vec![Block::Details {
            summary: vec![Inline::text(attr_nonempty(node, "title").unwrap_or("more"))],
            blocks: blocks(content(node)),
        }],
        "mediaSingle" => content(node)
            .iter()
            .map(|m| match node_type(m) {
                "media" => vec![Block::Media(media(m, true))],
                _ => block(m),
            })
            .collect::<Vec<_>>()
            .concat(),
        "mediaGroup" => blocks(content(node)),
        "media" => vec![Block::Media(media(node, false))],
        "blockCard" | "embedCard" => {
            let url = card_url(node);
            if url.is_empty() {
                Vec::new()
            } else {
                vec![Block::para(vec![Inline::link(url.clone(), url)])]
            }
        }
        "extension" | "bodiedExtension" => {
            let mut out = vec![Block::para(vec![Inline::text(macro_text(node))])];
            out.extend(blocks(content(node)));
            out
        }
        _ => {
            let kids = content(node);
            if is_inline_type(t) {
                // A lone inline node: show it as a paragraph.
                let mut r = Vec::new();
                inline(node, &mut r);
                if r.is_empty() { Vec::new() } else { vec![Block::para(r)] }
            } else if !kids.is_empty() && kids.iter().all(|k| is_inline_type(node_type(k))) {
                para(node)
            } else if let Some(s) = node.get("text").and_then(Value::as_str) {
                text_blocks(s)
            } else {
                blocks(kids)
            }
        }
    }
}

fn table(node: &Value) -> Block {
    let mut head = Vec::new();
    let mut rows = Vec::new();
    for (i, row) in content(node).iter().enumerate() {
        if node_type(row) != "tableRow" {
            continue;
        }
        let cells = content(row);
        let texts: Vec<Vec<Inline>> = cells
            .iter()
            .map(|cell| {
                // A cell's paragraphs, one after another.
                let mut r: Vec<Inline> = Vec::new();
                for b in content(cell) {
                    let more = runs(b);
                    if more.is_empty() {
                        continue;
                    }
                    if !r.is_empty() {
                        r.push(Inline::text(" "));
                    }
                    r.extend(more);
                }
                r
            })
            .collect();
        let is_head = !cells.is_empty() && cells.iter().all(|c| node_type(c) == "tableHeader");
        if i == 0 && is_head {
            head = texts;
        } else {
            rows.push(texts);
        }
    }
    Block::Table {
        aligns: Vec::new(),
        head,
        rows,
    }
}

// ---- lists ----

fn list(node: &Value) -> Block {
    let ordered = node_type(node) == "orderedList";
    let start = attr(node, "order")
        .and_then(Value::as_u64)
        .unwrap_or(1);
    let mut items: Vec<ListItem> = Vec::new();
    let mut n = start;
    for child in content(node) {
        if node_type(child) == "listItem" {
            let marker = if ordered { Marker::Number(n) } else { Marker::Bullet };
            n = n.saturating_add(1);
            items.push(ListItem {
                marker,
                blocks: blocks(content(child)),
            });
        } else {
            nest(&mut items, block(child));
        }
    }
    Block::List { items }
}

/// Blocks that sit among a list's items go under the item before them.
fn nest(items: &mut Vec<ListItem>, more: Vec<Block>) {
    if more.is_empty() {
        return;
    }
    match items.last_mut() {
        Some(last) => last.blocks.extend(more),
        None => items.push(ListItem {
            marker: Marker::Bullet,
            blocks: more,
        }),
    }
}

fn task_list(node: &Value) -> Block {
    let mut items: Vec<ListItem> = Vec::new();
    for child in content(node) {
        let t = node_type(child);
        let marker = match t {
            "taskItem" if attr_str(child, "state") == Some("DONE") => Marker::Done,
            "taskItem" => Marker::Todo,
            "decisionItem" => Marker::Decision,
            _ => {
                nest(&mut items, block(child));
                continue;
            }
        };
        items.push(ListItem {
            marker,
            blocks: para(child),
        });
    }
    Block::List { items }
}

// ---- text → ADF, for what radar posts ----
//
// What people type in radar (a comment, a description) is sent as ADF, which
// Jira's v3 API requires. The text is a small Markdown: `# ` headings (1 to
// 6), `- ` or `* ` bullets and `1. ` numbers (two spaces deeper per level),
// ``` fenced code blocks (a language after the fence), paragraphs split by a
// blank line (a line break inside one is kept), and inline `code`, **bold**,
// [words](https://…) and bare https:// links. `to_text` is the inverse: it
// writes a document back in the same Markdown, so what will post can be
// shown, and so the two can be tested against each other.

use serde_json::json;

/// A document Jira's v3 API accepts, from the text typed in radar.
pub fn from_text(text: &str) -> Value {
    let lines: Vec<&str> = text.split('\n').map(|l| l.trim_end_matches('\r')).collect();
    let mut content = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if line.trim().is_empty() {
            i += 1;
            continue;
        }
        if let Some(lang) = line.trim_start().strip_prefix("```") {
            let mut code = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                code.push(lines[i]);
                i += 1;
            }
            i += 1; // the closing fence, if there is one
            let mut node = json!({ "type": "codeBlock" });
            if !lang.trim().is_empty() {
                node["attrs"] = json!({ "language": lang.trim() });
            }
            if !code.is_empty() {
                node["content"] = json!([{ "type": "text", "text": code.join("\n") }]);
            }
            content.push(node);
            continue;
        }
        if let Some((level, words)) = heading_of(line) {
            content.push(json!({
                "type": "heading",
                "attrs": { "level": level },
                "content": inline_nodes(words),
            }));
            i += 1;
            continue;
        }
        if item_of(line).is_some() {
            let start = i;
            while i < lines.len() && item_of(lines[i]).is_some() {
                i += 1;
            }
            let items: Vec<Item> = lines[start..i].iter().filter_map(|l| item_of(l)).collect();
            let mut at = 0;
            while at < items.len() {
                content.push(list_node(&items, &mut at));
            }
            continue;
        }
        // A paragraph runs to a blank line or to the start of another block.
        let start = i;
        while i < lines.len()
            && !lines[i].trim().is_empty()
            && !lines[i].trim_start().starts_with("```")
            && heading_of(lines[i]).is_none()
            && (i == start || item_of(lines[i]).is_none())
        {
            i += 1;
        }
        let mut nodes = Vec::new();
        for (n, l) in lines[start..i].iter().enumerate() {
            if n > 0 {
                nodes.push(json!({ "type": "hardBreak" }));
            }
            nodes.extend(inline_nodes(l.trim()));
        }
        content.push(json!({ "type": "paragraph", "content": nodes }));
    }
    json!({ "type": "doc", "version": 1, "content": content })
}

/// `## Words` → (2, "Words").
fn heading_of(line: &str) -> Option<(usize, &str)> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    let rest = line[hashes..].strip_prefix(' ')?;
    ((1..=6).contains(&hashes) && !rest.trim().is_empty()).then(|| (hashes, rest.trim()))
}

/// A list item line: how deep, numbered (from what), its words.
#[derive(Debug, Clone, Copy)]
struct Item<'a> {
    depth: usize,
    number: Option<u64>,
    words: &'a str,
}

fn item_of(line: &str) -> Option<Item<'_>> {
    let spaces = line.chars().take_while(|c| *c == ' ').count();
    let rest = &line[spaces..];
    let (number, words) = if let Some(w) = rest.strip_prefix("- ").or_else(|| rest.strip_prefix("* ")) {
        (None, w)
    } else {
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        let w = rest[digits..].strip_prefix(". ")?;
        (Some(rest[..digits].parse().ok()?), w)
    };
    (digits_ok(number) && !words.trim().is_empty()).then_some(Item {
        depth: spaces / 2,
        number,
        words: words.trim(),
    })
}

fn digits_ok(n: Option<u64>) -> bool {
    n.is_none_or(|n| n < 1_000_000)
}

/// The list starting at `items[*at]`: its run of items of one kind at that
/// depth, each with the deeper items after it as a list inside it.
fn list_node(items: &[Item<'_>], at: &mut usize) -> Value {
    let depth = items[*at].depth;
    let numbered = items[*at].number;
    let mut list_items = Vec::new();
    while *at < items.len() && items[*at].depth == depth && items[*at].number.is_some() == numbered.is_some() {
        let mut item = vec![json!({ "type": "paragraph", "content": inline_nodes(items[*at].words) })];
        *at += 1;
        while *at < items.len() && items[*at].depth > depth {
            item.push(list_node(items, at));
        }
        list_items.push(json!({ "type": "listItem", "content": item }));
    }
    let mut node = json!({
        "type": if numbered.is_some() { "orderedList" } else { "bulletList" },
        "content": list_items,
    });
    if let Some(n) = numbered.filter(|n| *n != 1) {
        node["attrs"] = json!({ "order": n });
    }
    node
}

fn text_value(text: &str, marks: &[Value]) -> Value {
    let mut node = json!({ "type": "text", "text": text });
    if !marks.is_empty() {
        node["marks"] = Value::Array(marks.to_vec());
    }
    node
}

fn link_mark(href: &str) -> Value {
    json!({ "type": "link", "attrs": { "href": href } })
}

/// Where a bare link ends: before a space, or a sentence's own punctuation.
fn bare_link_end(s: &str) -> usize {
    let mut end = s.find(char::is_whitespace).unwrap_or(s.len());
    while end > 0 && s[..end].ends_with(['.', ',', ';', ':', '!', '?', ')']) {
        end -= 1;
    }
    end
}

/// A line's words as text nodes: `code`, **bold**, [words](link), bare
/// links; anything unclosed stays as typed.
fn inline_nodes(s: &str) -> Vec<Value> {
    inline_marked(s, &[])
}

fn inline_marked(s: &str, outer: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut plain = String::new();
    let mut rest = s;
    let flush = |plain: &mut String, out: &mut Vec<Value>| {
        if !plain.is_empty() {
            out.push(text_value(plain, outer));
            plain.clear();
        }
    };
    while let Some(c) = rest.chars().next() {
        if c == '`'
            && let Some(end) = rest[1..].find('`').filter(|e| *e > 0)
        {
            flush(&mut plain, &mut out);
            let mut marks = vec![json!({ "type": "code" })];
            marks.extend(outer.iter().cloned());
            out.push(text_value(&rest[1..1 + end], &marks));
            rest = &rest[end + 2..];
            continue;
        }
        if rest.starts_with("**")
            && !outer.iter().any(|m| m["type"] == "strong")
            && let Some(end) = rest[2..].find("**").filter(|e| *e > 0)
        {
            flush(&mut plain, &mut out);
            let mut marks = vec![json!({ "type": "strong" })];
            marks.extend(outer.iter().cloned());
            out.extend(inline_marked(&rest[2..2 + end], &marks));
            rest = &rest[end + 4..];
            continue;
        }
        if c == '['
            && !outer.iter().any(|m| m["type"] == "link")
            && let Some(close) = rest.find("](")
            && close > 1
            && let Some(end) = rest[close + 2..].find(')')
        {
            let href = &rest[close + 2..close + 2 + end];
            if href.starts_with("http://") || href.starts_with("https://") {
                flush(&mut plain, &mut out);
                let mut marks: Vec<Value> = outer.to_vec();
                marks.push(link_mark(href));
                out.extend(inline_marked(&rest[1..close], &marks));
                rest = &rest[close + 3 + end..];
                continue;
            }
        }
        if (rest.starts_with("https://") || rest.starts_with("http://"))
            && !outer.iter().any(|m| m["type"] == "link")
            && plain.chars().last().is_none_or(char::is_whitespace)
        {
            let end = bare_link_end(rest);
            if end > "https://".len() {
                flush(&mut plain, &mut out);
                let mut marks: Vec<Value> = outer.to_vec();
                marks.push(link_mark(&rest[..end]));
                out.push(text_value(&rest[..end], &marks));
                rest = &rest[end..];
                continue;
            }
        }
        plain.push(c);
        rest = &rest[c.len_utf8()..];
    }
    flush(&mut plain, &mut out);
    out
}

/// A document as the Markdown `from_text` reads, so the two are tested
/// against each other. Nodes outside that Markdown are written as their
/// display text.
#[cfg(test)]
pub fn to_text(doc: &Value) -> String {
    let mut blocks_out: Vec<String> = Vec::new();
    for node in content(doc) {
        let b = block_text(node, 0);
        if !b.is_empty() {
            blocks_out.push(b);
        }
    }
    blocks_out.join("\n\n")
}

#[cfg(test)]
fn block_text(node: &Value, depth: usize) -> String {
    match node_type(node) {
        "paragraph" => inline_text(content(node)),
        "heading" => {
            let level = attr(node, "level").and_then(Value::as_u64).unwrap_or(1).clamp(1, 6) as usize;
            format!("{} {}", "#".repeat(level), inline_text(content(node)))
        }
        "bulletList" | "orderedList" => list_text(node, depth),
        "codeBlock" => {
            let lang = attr_str(node, "language").unwrap_or("");
            let code: String = content(node)
                .iter()
                .filter_map(|c| c.get("text").and_then(Value::as_str))
                .collect();
            if code.is_empty() {
                format!("```{lang}\n```")
            } else {
                format!("```{lang}\n{code}\n```")
            }
        }
        _ => plain(node),
    }
}

#[cfg(test)]
fn list_text(node: &Value, depth: usize) -> String {
    let ordered = node_type(node) == "orderedList";
    let start = attr(node, "order").and_then(Value::as_u64).unwrap_or(1);
    let pad = "  ".repeat(depth);
    let mut out = Vec::new();
    for (n, item) in (start..).zip(content(node)) {
        let marker = if ordered { format!("{n}. ") } else { "- ".to_string() };
        let mut first = true;
        for child in content(item) {
            if is_list_type(node_type(child)) {
                out.push(list_text(child, depth + 1));
            } else if first {
                out.push(format!("{pad}{marker}{}", block_text(child, depth)));
                first = false;
            } else {
                out.push(format!("{pad}  {}", block_text(child, depth)));
            }
        }
        if first {
            out.push(format!("{pad}{}", marker.trim_end()));
        }
    }
    out.join("\n")
}

/// A mark's name and, for a link, where it goes.
#[cfg(test)]
fn marks_of(node: &Value) -> (bool, bool, Option<String>) {
    let marks = node.get("marks").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
    let has = |name: &str| marks.iter().any(|m| node_type(m) == name);
    let href = marks
        .iter()
        .find(|m| node_type(m) == "link")
        .and_then(|m| attr_str(m, "href"))
        .map(str::to_string);
    (has("code"), has("strong"), href)
}

#[cfg(test)]
fn inline_text(nodes: &[Value]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < nodes.len() {
        let node = &nodes[i];
        if node_type(node) != "text" {
            let mut r = Vec::new();
            inline(node, &mut r);
            out.extend(r.iter().map(|i| if i.kind == Kind::Break { "\n" } else { i.text.as_str() }));
            i += 1;
            continue;
        }
        let (_, _, href) = marks_of(node);
        match href {
            Some(href) => {
                // The run of nodes with this link, as one.
                let start = i;
                while i < nodes.len() && node_type(&nodes[i]) == "text" && marks_of(&nodes[i]).2.as_deref() == Some(&href) {
                    i += 1;
                }
                let inner: String = nodes[start..i].iter().map(marked_text).collect();
                let bare = i == start + 1 && nodes[start].get("text").and_then(Value::as_str) == Some(href.as_str());
                let (code, strong, _) = marks_of(&nodes[start]);
                if bare && !code && !strong {
                    out.push_str(&href);
                } else {
                    out.push_str(&format!("[{inner}]({href})"));
                }
            }
            None => {
                out.push_str(&marked_text(node));
                i += 1;
            }
        }
    }
    out
}

/// A text node with its code and bold marks written out.
#[cfg(test)]
fn marked_text(node: &Value) -> String {
    let text = node.get("text").and_then(Value::as_str).unwrap_or("");
    let (code, strong, _) = marks_of(node);
    let text = if code { format!("`{text}`") } else { text.to_string() };
    if strong { format!("**{text}**") } else { text }
}

/// Text nodes next to each other with the same marks, as one: what Jira
/// itself stores, and what `from_text` gives for the same words.
#[cfg(test)]
fn merged(doc: &Value) -> Value {
    match doc {
        Value::Object(o) => {
            let mut out = serde_json::Map::new();
            for (k, v) in o {
                if k == "content"
                    && let Some(a) = v.as_array()
                {
                    let mut items: Vec<Value> = Vec::new();
                    for c in a.iter().map(merged) {
                        if let Some(last) = items.last_mut()
                            && node_type(last) == "text"
                            && node_type(&c) == "text"
                            && last.get("marks") == c.get("marks")
                        {
                            let joined = format!(
                                "{}{}",
                                last["text"].as_str().unwrap_or(""),
                                c["text"].as_str().unwrap_or("")
                            );
                            last["text"] = Value::String(joined);
                            continue;
                        }
                        items.push(c);
                    }
                    out.insert(k.clone(), Value::Array(items));
                } else {
                    out.insert(k.clone(), merged(v));
                }
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(merged).collect()),
        v => v.clone(),
    }
}

#[cfg(test)]
mod text_tests {
    use super::*;

    /// Texts in the Markdown `from_text` reads, written the way `to_text`
    /// writes them: each comes back unchanged.
    const SAMPLES: [&str; 9] = [
        "Looks right.",
        "First line\nsecond line of the same paragraph\n\nA new paragraph.",
        "# Why\n\nThe dashboard needs a count.\n\n## How\n\n### Detail",
        "- one\n- two with `code`\n  - nested\n    - deeper\n- three",
        "1. first\n2. second\n  - a bullet inside\n3. third",
        "4. starts at four\n5. five",
        "Run it:\n\n```sh\ncargo test -p radar-jira\n  indented\n```\n\nThen **check** the [design notes](https://example.com/design) and https://acme.atlassian.net/browse/ABC-12.",
        "```\nno language\n```",
        "[**bold link**](https://example.com/b) and [`code` link](https://example.com/c)",
    ];

    #[test]
    fn text_comes_back_from_its_document_unchanged() {
        for s in SAMPLES {
            let doc = from_text(s);
            assert_eq!(to_text(&doc), s, "{doc:#}");
        }
    }

    #[test]
    fn a_document_comes_back_from_its_text_unchanged() {
        let docs = [
            json!({ "type": "doc", "version": 1, "content": [
                { "type": "heading", "attrs": { "level": 2 }, "content": [{ "type": "text", "text": "Plan" }] },
                { "type": "paragraph", "content": [
                    { "type": "text", "text": "Use " },
                    { "type": "text", "text": "GET /stats", "marks": [{ "type": "code" }] },
                    { "type": "text", "text": ", " },
                    { "type": "text", "text": "not", "marks": [{ "type": "strong" }] },
                    { "type": "hardBreak" },
                    { "type": "text", "text": "the old one: " },
                    { "type": "text", "text": "notes", "marks": [{ "type": "link", "attrs": { "href": "https://example.com/n" } }] },
                ]},
                { "type": "bulletList", "content": [
                    { "type": "listItem", "content": [
                        { "type": "paragraph", "content": [{ "type": "text", "text": "a" }] },
                        { "type": "orderedList", "attrs": { "order": 3 }, "content": [
                            { "type": "listItem", "content": [
                                { "type": "paragraph", "content": [{ "type": "text", "text": "b" }] }
                            ]}
                        ]}
                    ]}
                ]},
                { "type": "codeBlock", "attrs": { "language": "rust" }, "content": [{ "type": "text", "text": "fn a() {}\n" }] },
            ]}),
            from_text(SAMPLES[6]),
        ];
        for d in docs {
            let d = merged(&d);
            assert_eq!(merged(&from_text(&to_text(&d))), d);
        }
    }

    #[test]
    fn what_jira_receives_for_a_comment() {
        let doc = from_text("Fixed in **!263**: see https://example.com/x.\n\n- tests\n- docs");
        assert_eq!(
            doc,
            json!({ "type": "doc", "version": 1, "content": [
                { "type": "paragraph", "content": [
                    { "type": "text", "text": "Fixed in " },
                    { "type": "text", "text": "!263", "marks": [{ "type": "strong" }] },
                    { "type": "text", "text": ": see " },
                    { "type": "text", "text": "https://example.com/x", "marks": [{ "type": "link", "attrs": { "href": "https://example.com/x" } }] },
                    { "type": "text", "text": "." },
                ]},
                { "type": "bulletList", "content": [
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "tests" }] }] },
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "docs" }] }] },
                ]},
            ]})
        );
    }

    #[test]
    fn it_reads_as_the_page_shows_it() {
        let shown = plain(&from_text(SAMPLES[4]));
        assert_eq!(shown, "1. first\n2. second\n   ◦ a bullet inside\n3. third");
        let shown = plain(&from_text("# Why\n\nsee [notes](https://example.com/n)"));
        assert_eq!(shown, "Why\n───\n\nsee notes");
    }

    #[test]
    fn odd_text_is_kept_as_typed() {
        for s in ["a ` lone tick", "**not closed", "[no link](here)", "#nospace", "1.no space", "-no space", "```\nunclosed"] {
            let doc = from_text(s);
            let back = to_text(&doc);
            assert!(back.starts_with(s.split('\n').next().unwrap()) || back.contains("unclosed"), "{s:?} → {back:?}");
        }
        assert_eq!(from_text("  \n\n "), json!({ "type": "doc", "version": 1, "content": [] }));
        assert_eq!(to_text(&Value::Null), "");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The document as the page lays it out, a line each.
    fn lines(v: &Value) -> Vec<String> {
        let p = plain(v);
        if p.is_empty() {
            return Vec::new();
        }
        p.lines().map(String::from).collect()
    }
    fn doc(content: Value) -> Value {
        json!({"type": "doc", "version": 1, "content": content})
    }
    fn tx(s: &str) -> Value {
        json!({"type": "text", "text": s})
    }
    fn p(s: &str) -> Value {
        json!({"type": "paragraph", "content": [tx(s)]})
    }
    fn li(s: &str) -> Value {
        json!({"type": "listItem", "content": [p(s)]})
    }

    #[test]
    fn paragraphs_and_headings_are_separated_by_one_blank() {
        let d = doc(json!([
            {"type": "paragraph", "content": []},
            {"type": "heading", "attrs": {"level": 1}, "content": [tx("Title")]},
            p("One"),
            {"type": "paragraph"},
            p("Two"),
            {"type": "heading", "attrs": {"level": 3}, "content": [tx("End")]},
            {"type": "paragraph", "content": [tx("  ")]},
        ]));
        assert_eq!(lines(&d), ["Title", "─────", "", "One", "", "Two", "", "End"]);
    }

    #[test]
    fn hard_break_starts_a_new_line() {
        let d = doc(json!([
            {"type": "paragraph", "content": [tx("a"), {"type": "hardBreak"}, tx("b  ")]}
        ]));
        assert_eq!(lines(&d), ["a", "b"]);
    }

    #[test]
    fn bullet_list_nested_three_deep() {
        let d = doc(json!([
            {"type": "bulletList", "content": [
                {"type": "listItem", "content": [
                    p("a"),
                    {"type": "bulletList", "content": [
                        {"type": "listItem", "content": [
                            p("b"),
                            {"type": "bulletList", "content": [li("c")]}
                        ]}
                    ]}
                ]},
                li("d"),
            ]}
        ]));
        assert_eq!(lines(&d), ["• a", "  ◦ b", "    ◦ c", "• d"]);
    }

    #[test]
    fn ordered_list_starts_at_order() {
        let d = doc(json!([
            {"type": "orderedList", "attrs": {"order": 3}, "content": [li("x"), li("y")]},
            {"type": "orderedList", "content": [li("z")]},
        ]));
        assert_eq!(lines(&d), ["3. x", "4. y", "", "1. z"]);
    }

    #[test]
    fn list_item_with_two_paragraphs_aligns() {
        let d = doc(json!([
            {"type": "orderedList", "attrs": {"order": 9}, "content": [
                {"type": "listItem", "content": [p("first"), p("second")]},
                li("next"),
            ]},
            {"type": "bulletList", "content": [
                {"type": "listItem", "content": [
                    {"type": "paragraph", "content": [tx("a"), {"type": "hardBreak"}, tx("b")]}
                ]}
            ]}
        ]));
        assert_eq!(
            lines(&d),
            ["9. first", "", "   second", "10. next", "", "• a", "  b"]
        );
    }

    #[test]
    fn empty_item_is_just_the_marker() {
        let d = doc(json!([
            {"type": "bulletList", "content": [{"type": "listItem", "content": []}]}
        ]));
        assert_eq!(lines(&d), ["•"]);
    }

    #[test]
    fn task_list_done_and_todo() {
        let d = doc(json!([
            {"type": "taskList", "content": [
                {"type": "taskItem", "attrs": {"state": "TODO"}, "content": [tx("write")]},
                {"type": "taskItem", "attrs": {"state": "DONE"}, "content": [tx("ship")]},
                {"type": "taskList", "content": [
                    {"type": "taskItem", "attrs": {"state": "TODO"}, "content": [tx("nested")]},
                ]},
            ]}
        ]));
        assert_eq!(lines(&d), ["☐ write", "☑ ship", "  ☐ nested"]);
    }

    #[test]
    fn decision_list() {
        let d = doc(json!([
            {"type": "decisionList", "content": [
                {"type": "decisionItem", "attrs": {"state": "DECIDED"}, "content": [tx("Use Rust")]},
            ]}
        ]));
        assert_eq!(lines(&d), ["◇ Use Rust"]);
    }

    #[test]
    fn code_block_lines_without_marks() {
        let d = doc(json!([
            p("before"),
            {"type": "codeBlock", "attrs": {"language": "rust"}, "content": [
                {"type": "text", "text": "fn a() {\n    1\n}", "marks": [{"type": "code"}]}
            ]},
        ]));
        assert_eq!(lines(&d), ["before", "", "  fn a() {", "      1", "  }"]);
    }

    #[test]
    fn blockquote_keeps_a_continuous_bar() {
        let d = doc(json!([
            {"type": "blockquote", "content": [p("q1"), p("q2")]},
            p("after"),
        ]));
        assert_eq!(lines(&d), ["│ q1", "│", "│ q2", "", "after"]);
    }

    #[test]
    fn panel_symbols() {
        for (ty, sym, tone) in [
            ("info", "ℹ ", Tone::Info),
            ("note", "✎ ", Tone::Accent),
            ("warning", "⚠ ", Tone::Warn),
            ("error", "✗ ", Tone::Bad),
            ("success", "✓ ", Tone::Good),
            ("custom", "", Tone::Muted),
        ] {
            let d = doc(json!([
                {"type": "panel", "attrs": {"panelType": ty}, "content": [p("one"), p("two")]}
            ]));
            assert_eq!(lines(&d), [format!("▌ {sym}one"), "▌".into(), "▌ two".into()], "{ty}");
            assert!(matches!(&super::doc(&d).blocks[0], Block::Callout { tone: t, .. } if *t == tone));
        }
    }

    #[test]
    fn table_with_header_and_body_rows() {
        let cell = |ty: &str, paras: Value| json!({"type": ty, "content": paras});
        let d = doc(json!([
            {"type": "table", "content": [
                {"type": "tableRow", "content": [
                    cell("tableHeader", json!([p("Name")])),
                    cell("tableHeader", json!([p("Role")])),
                ]},
                {"type": "tableRow", "content": [
                    cell("tableCell", json!([p("Alice Example")])),
                    cell("tableCell", json!([p("Dev")])),
                ]},
                {"type": "tableRow", "content": [
                    cell("tableCell", json!([p("Bob"), p("Example")])),
                    cell("tableHeader", json!([p("Ops")])),
                ]},
            ]}
        ]));
        assert_eq!(
            lines(&d),
            [
                "Name           Role",
                "─────────────  ────",
                "Alice Example  Dev",
                "Bob Example    Ops"
            ]
        );
    }

    #[test]
    fn rule_between_paragraphs() {
        let d = doc(json!([p("a"), {"type": "rule"}, p("b")]));
        assert_eq!(lines(&d), ["a", "", &"─".repeat(24), "", "b"]);
    }

    #[test]
    fn expand_with_and_without_title() {
        let d = doc(json!([
            {"type": "expand", "attrs": {"title": "Details"}, "content": [p("inner")]},
            {"type": "nestedExpand", "content": [p("more text")]},
        ]));
        assert_eq!(
            lines(&d),
            ["Details", "", "  inner", "", "more", "", "  more text"]
        );
    }

    #[test]
    fn media_variants() {
        let d = doc(json!([
            {"type": "mediaSingle", "content": [
                {"type": "media", "attrs": {"type": "external", "url": "https://acme.atlassian.net/a.png"}}
            ]},
            {"type": "mediaGroup", "content": [
                {"type": "media", "attrs": {"type": "file", "id": "1", "alt": "spec.pdf"}},
                {"type": "media", "attrs": {"type": "file", "id": "2"}},
            ]},
            {"type": "mediaSingle", "content": [
                {"type": "media", "attrs": {"type": "file", "id": "3", "alt": "shot.png", "width": 640, "height": 480}}
            ]},
        ]));
        assert_eq!(
            lines(&d),
            [
                "▣ image https://acme.atlassian.net/a.png",
                "",
                "▤ file spec.pdf",
                "",
                "▤ file attachment",
                "",
                "▣ image shot.png",
            ]
        );
        let Block::Media(m) = &super::doc(&d).blocks[3] else { panic!() };
        assert_eq!((m.image, m.size), (true, Some((640, 480))));
    }

    #[test]
    fn media_finds_its_attachment_by_name() {
        let fields = json!({ "attachment": [
            { "filename": "shot.png", "mimeType": "image/png", "content": "https://acme.atlassian.net/rest/api/3/attachment/content/10001" },
            { "filename": "spec.pdf", "mimeType": "application/pdf", "content": "https://acme.atlassian.net/rest/api/3/attachment/content/10002" },
            { "filename": "no-content.txt" },
        ]});
        let files = attachments(&fields);
        assert_eq!(files.len(), 2);
        let mut d = super::doc(&doc(json!([
            {"type": "mediaSingle", "content": [
                {"type": "media", "attrs": {"type": "file", "id": "u-1", "alt": "shot.png", "width": 640, "height": 480}}
            ]},
            {"type": "blockquote", "content": [{"type": "mediaGroup", "content": [
                {"type": "media", "attrs": {"type": "file", "id": "u-2", "alt": "spec.pdf"}},
                {"type": "media", "attrs": {"type": "file", "id": "u-3", "alt": "gone.png"}},
            ]}]},
        ])));
        attach(&mut d, &files);
        let Block::Media(shot) = &d.blocks[0] else { panic!() };
        assert!(shot.image);
        assert_eq!(shot.src, "https://acme.atlassian.net/rest/api/3/attachment/content/10001");
        let Block::Quote { blocks } = &d.blocks[1] else { panic!() };
        let Block::Media(spec) = &blocks[0] else { panic!() };
        assert!(!spec.image && spec.src.is_empty());
        assert_eq!(spec.url, "https://acme.atlassian.net/rest/api/3/attachment/content/10002");
        let Block::Media(gone) = &blocks[1] else { panic!() };
        assert!(gone.url.is_empty() && gone.src.is_empty());
    }

    #[test]
    fn media_inline() {
        let d = doc(json!([
            {"type": "paragraph", "content": [
                tx("see "), {"type": "mediaInline", "attrs": {"id": "1"}},
                tx(" and "), {"type": "mediaInline", "attrs": {"alt": "log.txt"}},
            ]}
        ]));
        assert_eq!(lines(&d), ["see ▤ attachment and ▤ log.txt"]);
    }

    #[test]
    fn cards() {
        let url = "https://acme.atlassian.net/browse/ABC-12";
        let d = doc(json!([
            {"type": "blockCard", "attrs": {"url": url}},
            {"type": "paragraph", "content": [tx("see "), {"type": "inlineCard", "attrs": {"url": url}}]},
            {"type": "embedCard", "attrs": {"url": "https://acme.atlassian.net/e"}},
        ]));
        assert_eq!(
            lines(&d),
            [url.to_string(), "".into(), format!("see {url}"), "".into(), "https://acme.atlassian.net/e".into()]
        );
    }

    #[test]
    fn extensions() {
        let d = doc(json!([
            {"type": "extension", "attrs": {"extensionKey": "toc"}},
            {"type": "bodiedExtension", "attrs": {"extensionKey": "info"}, "content": [p("body")]},
            {"type": "paragraph", "content": [tx("x "), {"type": "inlineExtension", "attrs": {"extensionKey": "jira"}}]},
        ]));
        assert_eq!(
            lines(&d),
            ["[macro: toc]", "", "[macro: info]", "", "body", "", "x [macro: jira]"]
        );
    }

    #[test]
    fn marks_code_and_link() {
        let href = "https://acme.atlassian.net/wiki";
        let d = doc(json!([
            {"type": "paragraph", "content": [
                {"type": "text", "text": "x", "marks": [{"type": "code"}]},
                tx(" "),
                {"type": "text", "text": href, "marks": [{"type": "link", "attrs": {"href": href}}]},
                tx(" "),
                {"type": "text", "text": "docs", "marks": [{"type": "strong"}, {"type": "link", "attrs": {"href": href}}]},
            ]}
        ]));
        // Words show; where they go is collected, once.
        assert_eq!(lines(&d), [format!("x {href} docs")]);
        let Block::Para { text } = &super::doc(&d).blocks[0] else { panic!() };
        assert!(text[0].marks.code);
        assert!(text[4].marks.bold);
        assert_eq!(text[4].kind, Kind::Link { url: href.into() });
    }

    #[test]
    fn mentions_emoji_date_status_placeholder() {
        let d = doc(json!([
            {"type": "paragraph", "content": [
                {"type": "mention", "attrs": {"id": "1", "text": "@Alice Example"}},
                tx(", "),
                {"type": "mention", "attrs": {"id": "2", "text": "Bob Example"}},
                tx(", "),
                {"type": "mention", "attrs": {"id": "3"}},
            ]},
            {"type": "paragraph", "content": [
                {"type": "emoji", "attrs": {"shortName": ":smile:", "text": "😄"}},
                {"type": "emoji", "attrs": {"shortName": ":tada:"}},
                {"type": "emoji", "attrs": {"shortName": ":our-team-logo:"}},
                tx(" due "),
                {"type": "date", "attrs": {"timestamp": "1791256553000"}},
                tx(" "),
                {"type": "date", "attrs": {"timestamp": 1791256553000_i64}},
                tx(" "),
                {"type": "status", "attrs": {"text": "in progress", "color": "blue"}},
                {"type": "placeholder", "attrs": {"text": "type here"}},
            ]},
        ]));
        assert_eq!(
            lines(&d),
            [
                "@Alice Example, @Bob Example, @someone",
                "",
                "😄🎉:our-team-logo: due 2026-10-06 2026-10-06  IN PROGRESS"
            ]
        );
        let Block::Para { text } = &super::doc(&d).blocks[1] else { panic!() };
        assert!(text.iter().any(|i| i.kind == Kind::Badge { tone: Tone::Info }));
    }

    #[test]
    fn plain_string_input() {
        let v = json!("one  \r\ntwo\r\n\r\n\r\nthree\n");
        assert_eq!(lines(&v), ["one", "two", "", "three"]);
    }

    #[test]
    fn null_and_odd_input_give_nothing() {
        assert!(lines(&Value::Null).is_empty());
        assert!(lines(&json!(3)).is_empty());
        assert!(lines(&json!([1, 2])).is_empty());
        assert!(lines(&json!({})).is_empty());
        assert!(lines(&json!({"type": "doc", "version": 1})).is_empty());
        assert_eq!(plain(&Value::Null), "");
    }

    #[test]
    fn garbage_does_not_panic() {
        assert!(lines(&json!({"type": "doc", "content": "nope"})).is_empty());
        let weird = doc(json!([
            null, 5, "x", [],
            {"type": 7, "content": {"a": 1}},
            {"type": "bulletList", "content": [1, {"type": "listItem", "content": "x"}, {"type": "paragraph"}]},
            {"type": "table", "content": [{"type": "tableRow", "content": [null]}, {"type": "tableRow"}]},
            {"type": "panel", "attrs": {"panelType": 4}},
            {"type": "orderedList", "attrs": {"order": "x"}, "content": [li("k")]},
            {"type": "date", "attrs": {"timestamp": {"a": 1}}},
            {"type": "codeBlock"},
        ]));
        let _ = lines(&weird);
        let _ = plain(&weird);
    }

    #[test]
    fn unknown_nodes_are_walked() {
        let d = doc(json!([
            {"type": "layoutSection", "content": [
                {"type": "layoutColumn", "content": [p("left")]},
                {"type": "layoutColumn", "content": [p("right")]},
            ]},
            {"type": "caption", "content": [tx("cap "), tx("tion")]},
            {"type": "weirdInline", "text": "odd"},
        ]));
        assert_eq!(lines(&d), ["left", "", "right", "", "cap tion", "", "odd"]);
    }

    #[test]
    fn single_node_input() {
        assert_eq!(lines(&p("solo")), ["solo"]);
        assert_eq!(lines(&tx("bare")), ["bare"]);
    }

    #[test]
    fn plain_output_pins_the_format() {
        let d = doc(json!([
            {"type": "heading", "attrs": {"level": 2}, "content": [tx("Plan")]},
            {"type": "bulletList", "content": [
                {"type": "listItem", "content": [
                    p("one"),
                    {"type": "orderedList", "content": [li("sub")]}
                ]},
            ]},
            {"type": "blockquote", "content": [p("quoted")]},
            p("end"),
        ]));
        assert_eq!(plain(&d), "Plan\n\n• one\n  1. sub\n\n│ quoted\n\nend");
    }
}
