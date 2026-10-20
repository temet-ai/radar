//! The rich document model: one tree for text written in a service's own
//! format (markdown, a tracker's JSON documents, a chat's markup). A part
//! converts its format into a [`Doc`]; the renderers lay it out
//! (`ui::rich::layout`) or send it as it is. This module holds
//! the data and names no renderer.

use serde::{Deserialize, Serialize};
use unicode_width::UnicodeWidthChar;

use crate::plugin::Tone;

// ---------------------------------------------------------------------
// The model.

/// A document: blocks, top to bottom. Its wire form is the bare list of
/// blocks; the protocol types say so with `#[ts(as = "Vec<Block>")]` where a
/// `Doc` is sent, so it derives no `TS` of its own.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Doc {
    pub blocks: Vec<Block>,
}

impl Doc {
    pub fn new(blocks: Vec<Block>) -> Self {
        Self { blocks }
    }

    /// Nothing that would draw.
    pub fn is_empty(&self) -> bool {
        self.blocks.iter().all(Block::is_empty)
    }

    /// Explicit links in document order, without laying the document out.
    /// This lets an interaction layer build link actions before any frontend
    /// chooses its width or paints a line.
    pub fn links(&self) -> Vec<Link> {
        fn inlines(runs: &[Inline], out: &mut Vec<Link>) {
            for run in runs {
                if let Kind::Link { url } = &run.kind
                    && !out.iter().any(|link| link.url == *url)
                {
                    out.push(Link {
                        text: if run.text.trim().is_empty() {
                            url.clone()
                        } else {
                            run.text.clone()
                        },
                        url: url.clone(),
                    });
                }
            }
        }
        fn walk_blocks(blocks: &[Block], out: &mut Vec<Link>) {
            for block in blocks {
                match block {
                    Block::Para { text } | Block::Heading { text, .. } => inlines(text, out),
                    Block::List { items } => {
                        for item in items {
                            walk_blocks(&item.blocks, out);
                        }
                    }
                    Block::Quote { blocks: children }
                    | Block::Callout {
                        blocks: children, ..
                    } => walk_blocks(children, out),
                    Block::Table { head, rows, .. } => {
                        for cell in head {
                            inlines(cell, out);
                        }
                        for row in rows {
                            for cell in row {
                                inlines(cell, out);
                            }
                        }
                    }
                    Block::Details {
                        summary,
                        blocks: children,
                    } => {
                        inlines(summary, out);
                        walk_blocks(children, out);
                    }
                    Block::Media(media)
                        if !media.url.is_empty()
                            && !out.iter().any(|link| link.url == media.url) =>
                    {
                        out.push(Link {
                            text: media.name.clone(),
                            url: media.url.clone(),
                        });
                    }
                    Block::Code { .. } | Block::Rule | Block::Media(_) => {}
                }
            }
        }
        let mut out = Vec::new();
        walk_blocks(&self.blocks, &mut out);
        out
    }
}

/// A block of a document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "t", rename_all = "snake_case")]
#[ts(rename = "RichBlock")]
pub enum Block {
    /// Words, wrapped to the width.
    Para { text: Vec<Inline> },
    /// A heading; level 1 is underlined.
    Heading { level: u8, text: Vec<Inline> },
    /// Items with a marker each; their later lines hang under the text.
    List { items: Vec<Item> },
    /// Quoted blocks, behind a bar.
    Quote { blocks: Vec<Block> },
    /// Blocks set apart with a feeling (a note, a warning): a bar in the
    /// tone's style, `symbol` before the first line.
    Callout {
        tone: Tone,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        symbol: String,
        blocks: Vec<Block>,
    },
    /// Code kept as written.
    Code {
        #[serde(default, skip_serializing_if = "String::is_empty")]
        lang: String,
        text: String,
    },
    /// Columns with a heading row, or a block per row where they cannot fit.
    Table {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        aligns: Vec<Align>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        head: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
    },
    /// A line across.
    Rule,
    /// A summary and the blocks under it, shown open.
    Details {
        summary: Vec<Inline>,
        blocks: Vec<Block>,
    },
    /// An image or a file.
    Media(Media),
}

impl Block {
    pub fn para(text: Vec<Inline>) -> Self {
        Block::Para { text }
    }

    fn is_empty(&self) -> bool {
        match self {
            Block::Para { text } | Block::Heading { text, .. } => !visible(text),
            Block::Quote { blocks } => blocks.iter().all(Block::is_empty),
            Block::List { items } => items.is_empty(),
            Block::Code { text, .. } => text.trim().is_empty(),
            _ => false,
        }
    }
}

/// A list item: its marker and its blocks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct Item {
    pub marker: Marker,
    pub blocks: Vec<Block>,
}

/// What stands before a list item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum Marker {
    /// `•`, and `◦` in a list inside another.
    Bullet,
    /// `3.`
    Number(u64),
    /// `☐`
    Todo,
    /// `☑`
    Done,
    /// `◇`
    Decision,
}

/// How a table column's cells sit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

/// An image or a file in a document.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct Media {
    #[serde(default, skip_serializing_if = "is_false")]
    pub image: bool,
    /// What it is called: a file's name, an image's words.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Where it opens; may be empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    /// What to fetch to draw it (an image's own address); may be empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub src: String,
    /// Its size in pixels, when the service says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<(u32, u32)>,
}

/// A run of text and what it is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct Inline {
    pub text: String,
    #[serde(default, skip_serializing_if = "Kind::is_text")]
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "Marks::is_plain")]
    pub marks: Marks,
}

/// What a run is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "InlineKind")]
pub enum Kind {
    #[default]
    Text,
    /// Words that go somewhere.
    Link { url: String },
    /// Someone, a group or a place named; `me` when it is the reader.
    Mention {
        #[serde(default, skip_serializing_if = "is_false")]
        me: bool,
    },
    /// An emoji: its glyph, or `:name:` when none is known.
    Emoji,
    /// A short label set apart (a state), drawn as a chip.
    Badge { tone: Tone },
    /// An image's words, where the image sits in a line of text.
    Image { src: String },
    /// A line break.
    Break,
}

impl Kind {
    fn is_text(&self) -> bool {
        *self == Kind::Text
    }
}

/// How a run's letters look.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct Marks {
    #[serde(default, skip_serializing_if = "is_false")]
    pub bold: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub italic: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub strike: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub underline: bool,
    /// Inline code.
    #[serde(default, skip_serializing_if = "is_false")]
    pub code: bool,
}

impl Marks {
    fn is_plain(&self) -> bool {
        *self == Marks::default()
    }
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Inline {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }

    pub fn link(text: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: Kind::Link { url: url.into() },
            marks: Marks::default(),
        }
    }

    pub fn mention(text: impl Into<String>, me: bool) -> Self {
        Self {
            text: text.into(),
            kind: Kind::Mention { me },
            marks: Marks::default(),
        }
    }

    /// An emoji by its glyph, or its `:name:` when it has none that is
    /// safe to draw ([`emoji`]).
    pub fn emoji(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: Kind::Emoji,
            marks: Marks::default(),
        }
    }

    pub fn badge(text: impl Into<String>, tone: Tone) -> Self {
        Self {
            text: text.into(),
            kind: Kind::Badge { tone },
            marks: Marks::default(),
        }
    }

    pub fn line_break() -> Self {
        Self {
            kind: Kind::Break,
            ..Self::default()
        }
    }

    pub fn with(mut self, marks: Marks) -> Self {
        self.marks = marks;
        self
    }

    pub fn code(mut self) -> Self {
        self.marks.code = true;
        self
    }

    pub fn bold(mut self) -> Self {
        self.marks.bold = true;
        self
    }
}

/// Whether runs hold anything that shows.
fn visible(text: &[Inline]) -> bool {
    text.iter().any(|i| match i.kind {
        Kind::Break => false,
        Kind::Badge { .. } => true,
        _ => !i.text.trim().is_empty(),
    })
}

// ---------------------------------------------------------------------
// Emoji.

/// The glyph for an emoji's short name (`tada`, `+1`, `white_check_mark`),
/// when one is known and is safe to draw: one wide character, which every
/// terminal shows two cells wide, so columns stay where they are. Joined
/// sequences, flags and skin tones are not; their name stays.
pub fn emoji(name: &str) -> Option<String> {
    let name = name.trim_matches(':');
    let found = emojis::get_by_shortcode(name)?;
    safe_emoji(found.as_str())
}

/// `glyph` when it draws as one wide character (a trailing emoji
/// presentation selector dropped), else nothing.
pub fn safe_emoji(glyph: &str) -> Option<String> {
    let base = glyph.trim_end_matches('\u{fe0f}');
    let mut chars = base.chars();
    let c = chars.next()?;
    (chars.next().is_none() && UnicodeWidthChar::width(c) == Some(2)).then(|| c.to_string())
}

/// A link found in the text: the words it showed (or its url when it had
/// none) and where it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub text: String,
    pub url: String,
}
