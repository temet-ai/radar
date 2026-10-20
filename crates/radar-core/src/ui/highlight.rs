//! Code coloured by language: a code string and its language in, styled
//! spans per line out. Built on `syntect` and its bundled grammars; the
//! grammars' scopes map onto the theme's slots (comments muted, strings,
//! numbers, keywords, types, functions each on a slot), so code follows
//! the user's palette. Keywords are bold and comments italic, so the shape
//! of the code still reads where there is no colour. A language syntect
//! does not know comes back plain.
//!
//! The grammars load once, on the first use, never at start-up. Feed a
//! [`Highlighter`] lines in order (it carries what an open comment or
//! string spans); [`highlight`] does a whole text.

use std::sync::OnceLock;

use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxSet};

/// What a stretch of code is, as far as colour goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    Plain,
    Comment,
    Text,
    Number,
    Keyword,
    Type,
    Function,
    Constant,
    /// Annotations, attributes, decorators, tags.
    Mark,
}

impl Token {
    /// Its name on the wire to a renderer that styles it itself.
    pub fn name(self) -> &'static str {
        match self {
            Token::Plain => "plain",
            Token::Comment => "comment",
            Token::Text => "text",
            Token::Number => "number",
            Token::Keyword => "keyword",
            Token::Type => "type",
            Token::Function => "function",
            Token::Constant => "constant",
            Token::Mark => "mark",
        }
    }

}

fn set() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

/// Scope prefixes and what they are; the first that matches a scope wins,
/// and the innermost scope that matches any decides.
fn rules() -> &'static [(Scope, Token)] {
    static RULES: OnceLock<Vec<(Scope, Token)>> = OnceLock::new();
    RULES.get_or_init(|| {
        [
            ("comment", Token::Comment),
            ("punctuation.definition.comment", Token::Comment),
            ("string", Token::Text),
            ("constant.character", Token::Text),
            ("constant.numeric", Token::Number),
            ("constant.language", Token::Constant),
            ("constant.other", Token::Constant),
            ("variable.language", Token::Constant),
            ("storage.type.annotation", Token::Mark),
            ("meta.annotation", Token::Mark),
            ("punctuation.definition.annotation", Token::Mark),
            ("entity.other.attribute-name", Token::Mark),
            ("meta.attribute", Token::Mark),
            ("entity.name.tag", Token::Mark),
            ("keyword", Token::Keyword),
            ("storage", Token::Keyword),
            ("entity.name.function", Token::Function),
            ("support.function", Token::Function),
            ("meta.function-call", Token::Function),
            ("entity.name.type", Token::Type),
            ("entity.name.class", Token::Type),
            ("entity.name.struct", Token::Type),
            ("entity.name.enum", Token::Type),
            ("entity.name.trait", Token::Type),
            ("entity.other.inherited-class", Token::Type),
            ("support.type", Token::Type),
            ("support.class", Token::Type),
        ]
        .into_iter()
        .filter_map(|(s, t)| Some((Scope::new(s).ok()?, t)))
        .collect()
    })
}

fn token_of(stack: &ScopeStack) -> Token {
    for scope in stack.as_slice().iter().rev() {
        if let Some((_, t)) = rules().iter().find(|(sel, _)| sel.is_prefix_of(*scope)) {
            return *t;
        }
    }
    Token::Plain
}

/// The language of a file, by its name or extension, as [`Highlighter`]
/// takes it; `None` when no grammar knows it.
pub fn language_of(path: &str) -> Option<String> {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let ext = match name.rsplit_once('.') {
        Some((_, ext)) => ext.to_ascii_lowercase(),
        None => name.to_string(),
    };
    // Close enough to a bundled grammar to read well.
    let ext = match ext.as_str() {
        "ts" | "tsx" | "mjs" | "cjs" | "jsx" => "js".to_string(),
        "kts" | "gradle" => "groovy".to_string(),
        "yml" => "yaml".to_string(),
        _ => ext,
    };
    let s = set();
    s.find_syntax_by_extension(&ext)
        .or_else(|| s.find_syntax_by_token(name))
        .filter(|syn| syn.name != "Plain Text")
        .map(|_| ext)
}

/// Code highlighted a line at a time, in order.
pub struct Highlighter {
    parse: ParseState,
    stack: ScopeStack,
}

impl Highlighter {
    /// For `lang` (an extension or a language's name: `rs`, `java`,
    /// `Python`); `None` when no grammar knows it.
    pub fn new(lang: &str) -> Option<Self> {
        let s = set();
        let syntax = s
            .find_syntax_by_token(lang)
            .or_else(|| s.find_syntax_by_extension(lang))
            .filter(|syn| syn.name != "Plain Text")?;
        Some(Self {
            parse: ParseState::new(syntax),
            stack: ScopeStack::new(),
        })
    }

    /// The next line (without its newline) as pieces of it and what each
    /// is, in order and not empty; `None` when the grammar cannot read it.
    /// The renderer styles each token itself.
    pub fn runs<'t>(&mut self, text: &'t str) -> Option<Vec<(&'t str, Token)>> {
        let with_nl = format!("{text}\n");
        let ops = self.parse.parse_line(&with_nl, set()).ok()?;
        let mut out: Vec<(&'t str, Token)> = Vec::new();
        let mut push = |from: usize, to: usize, token: Token| {
            let to = to.min(text.len());
            if from < to {
                out.push((&text[from..to], token));
            }
        };
        let mut at = 0;
        for (i, op) in ops {
            push(at, i, token_of(&self.stack));
            at = at.max(i);
            if self.stack.apply(&op).is_err() {
                break;
            }
        }
        push(at, text.len(), token_of(&self.stack));
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every line's pieces and what each is; a line the grammar cannot read,
    /// or an unknown language, is one plain piece.
    fn lines(code: &str, lang: &str) -> Vec<Vec<(String, Token)>> {
        let mut highlighter = Highlighter::new(lang);
        code.lines()
            .map(|l| {
                let runs = highlighter.as_mut().and_then(|h| h.runs(l));
                match runs {
                    Some(runs) if !runs.is_empty() => runs.into_iter().map(|(s, t)| (s.to_string(), t)).collect(),
                    _ => vec![(l.to_string(), Token::Plain)],
                }
            })
            .collect()
    }

    fn tokens(code: &str, lang: &str) -> Vec<(String, Token)> {
        lines(code, lang).into_iter().flatten().collect()
    }

    fn kind_of(spans: &[(String, Token)], word: &str) -> Token {
        spans
            .iter()
            .find(|(s, _)| s.contains(word))
            .map(|(_, t)| *t)
            .unwrap_or_else(|| panic!("{word} not in {spans:?}"))
    }

    #[test]
    fn code_is_coloured_by_what_each_part_is() {
        let spans = tokens("// sort them\nfn sort(n: u32) -> &'static str { \"by decade\" }\nlet x = 42;", "rs");
        assert_eq!(kind_of(&spans, "sort them"), Token::Comment);
        assert_eq!(kind_of(&spans, "fn"), Token::Keyword);
        assert_eq!(kind_of(&spans, "by decade"), Token::Text);
        assert_eq!(kind_of(&spans, "42"), Token::Number);
        let java = tokens("@RestController\npublic class Shelves {}", "java");
        assert_eq!(kind_of(&java, "public"), Token::Keyword);
        assert_eq!(kind_of(&java, "RestController"), Token::Mark);
    }

    #[test]
    fn every_line_keeps_its_text_and_an_unknown_language_is_plain() {
        let code = "/* open\n still open */\nx = 1\n\n";
        let lines = lines(code, "c");
        assert_eq!(lines.len(), 4);
        let text: Vec<String> = lines.iter().map(|l| l.iter().map(|(s, _)| s.as_str()).collect()).collect();
        assert_eq!(text, ["/* open", " still open */", "x = 1", ""]);
        // The open comment carries over to the next line.
        let spans = tokens(code, "c");
        assert_eq!(kind_of(&spans, "still open"), Token::Comment);
        assert_eq!(tokens("anything at all", "no-such-language"), vec![("anything at all".to_string(), Token::Plain)]);
    }

    #[test]
    fn the_language_comes_from_the_files_name() {
        assert_eq!(language_of("src/main/java/acme/Shelves.java").as_deref(), Some("java"));
        assert_eq!(language_of("crates/x/src/lib.rs").as_deref(), Some("rs"));
        assert_eq!(language_of("web/app.tsx").as_deref(), Some("js"));
        assert_eq!(language_of("ci/.pipeline.yml").as_deref(), Some("yaml"));
        assert_eq!(language_of("Makefile").as_deref(), Some("Makefile"));
        assert_eq!(language_of("notes.unknownext"), None);
    }
}
