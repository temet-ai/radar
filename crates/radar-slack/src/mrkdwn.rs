//! Slack mrkdwn (as the Web API returns it) turned into display lines.
//!
//! Self-contained, std only. Never panics on any input.

/// What a run of text is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Text,
    /// A link; `text` is its label (or the URL itself when it has none).
    Link { url: String },
    /// `<@U123>` or `<@U123|name>`: text is `@name`.
    User { id: String },
    /// `<!subteam^S123>` or `<!subteam^S123|@handle>`: text is `@handle`.
    Group { id: String },
    /// `<#C123>` or `<#C123|name>`: text is `#name`.
    Channel { id: String },
    /// `<!here>`, `<!channel>`, `<!everyone>`: text is `@here` etc.
    Broadcast,
    /// `:eyes:`: text is `:eyes:` (emoji are shown as text).
    Emoji { name: String },
}

/// A run of text and how it looks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seg {
    pub text: String,
    pub kind: Kind,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    /// Inline `code`.
    pub code: bool,
}

/// One line as shown.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DLine {
    pub segs: Vec<Seg>,
    /// A `>` quote line (the marker removed).
    pub quote: bool,
    /// Inside a ``` block.
    pub pre: bool,
}

/// Names for ids, from what the caller has cached.
pub trait Names {
    fn user(&self, id: &str) -> Option<String>;
    /// The handle, without '@'.
    fn group(&self, id: &str) -> Option<String>;
    /// Without '#'.
    fn channel(&self, id: &str) -> Option<String>;
}

/// No names known.
pub struct NoNames;

impl Names for NoNames {
    fn user(&self, _id: &str) -> Option<String> {
        None
    }
    fn group(&self, _id: &str) -> Option<String> {
        None
    }
    fn channel(&self, _id: &str) -> Option<String> {
        None
    }
}

/// The user ids and user-group ids mentioned, and whether a broadcast is.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Mentions {
    pub users: Vec<String>,
    pub groups: Vec<String>,
    pub broadcast: bool,
}

// ---------------------------------------------------------------------------
// Low-level helpers
// ---------------------------------------------------------------------------

fn starts_with_at(cs: &[char], i: usize, pat: &str) -> bool {
    pat.chars()
        .enumerate()
        .all(|(d, p)| cs.get(i + d) == Some(&p))
}

/// A decoded entity at `i`: the char and how many chars it spans.
fn entity(cs: &[char], i: usize) -> Option<(char, usize)> {
    if cs.get(i) != Some(&'&') {
        return None;
    }
    [("&amp;", '&'), ("&lt;", '<'), ("&gt;", '>')]
        .into_iter()
        .find(|(pat, _)| starts_with_at(cs, i, pat))
        .map(|(pat, c)| (c, pat.len()))
}

fn decode(s: &str) -> String {
    let cs: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < cs.len() {
        if let Some((c, l)) = entity(&cs, i) {
            out.push(c);
            i += l;
        } else {
            out.push(cs[i]);
            i += 1;
        }
    }
    out
}

/// If `cs[start]` is `<` opening a token, the index of its closing `>`.
/// A token has no `<` inside, is not empty, and does not start with a space.
fn token_end(cs: &[char], start: usize) -> Option<usize> {
    if cs.get(start) != Some(&'<') {
        return None;
    }
    match cs.get(start + 1) {
        None => return None,
        Some(c) if c.is_whitespace() => return None,
        _ => {}
    }
    for (k, c) in cs.iter().enumerate().skip(start + 1) {
        match c {
            '>' => return if k > start + 1 { Some(k) } else { None },
            '<' => return None,
            _ => {}
        }
    }
    None
}

fn is_link_target(t: &str) -> bool {
    t.starts_with("http://") || t.starts_with("https://") || t.starts_with("mailto:")
}

fn split_label(inner: &str) -> (&str, Option<&str>) {
    match inner.split_once('|') {
        Some((t, l)) => (t, Some(l)),
        None => (inner, None),
    }
}

/// Resolve the inside of an angle token to a kind and its display text.
fn resolve(inner: &str, names: &dyn Names) -> (Kind, String) {
    let (target, label) = split_label(inner);
    let target = decode(target);
    let label = label.map(decode).filter(|l| !l.is_empty());

    if let Some(id) = target.strip_prefix('@') {
        let text = if let Some(n) = names.user(id) {
            format!("@{}", n.trim_start_matches('@'))
        } else if let Some(l) = label {
            format!("@{}", l.trim_start_matches('@'))
        } else {
            format!("@{id}")
        };
        return (Kind::User { id: id.to_string() }, text);
    }
    if let Some(id) = target.strip_prefix("!subteam^") {
        let text = if let Some(n) = names.group(id) {
            format!("@{}", n.trim_start_matches('@'))
        } else if let Some(l) = label {
            format!("@{}", l.trim_start_matches('@'))
        } else {
            format!("@{id}")
        };
        return (Kind::Group { id: id.to_string() }, text);
    }
    if let Some(id) = target.strip_prefix('#') {
        let text = if let Some(n) = names.channel(id) {
            format!("#{}", n.trim_start_matches('#'))
        } else if let Some(l) = label {
            format!("#{}", l.trim_start_matches('#'))
        } else {
            format!("#{id}")
        };
        return (Kind::Channel { id: id.to_string() }, text);
    }
    match target.as_str() {
        "!here" => return (Kind::Broadcast, "@here".into()),
        "!channel" => return (Kind::Broadcast, "@channel".into()),
        "!everyone" => return (Kind::Broadcast, "@everyone".into()),
        _ => {}
    }
    if target.starts_with("!date") {
        return (Kind::Text, label.unwrap_or_default());
    }
    if is_link_target(&target) {
        let text = label.unwrap_or_else(|| target.clone());
        return (Kind::Link { url: target }, text);
    }
    (Kind::Text, decode(inner))
}

fn is_emoji_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '+' | '-' | '\'')
}

/// An emoji at `i` (a `:`): its name and the index just past it (and past any skin tones).
fn emoji_at(cs: &[char], i: usize) -> Option<(String, usize)> {
    if cs.get(i) != Some(&':') {
        return None;
    }
    let mut j = i + 1;
    while j < cs.len() && is_emoji_char(cs[j]) {
        j += 1;
    }
    if j == i + 1 || cs.get(j) != Some(&':') {
        return None;
    }
    let name: String = cs[i + 1..j].iter().collect();
    // `12:00:30` is a time, not an emoji.
    if name.chars().all(|c| c.is_ascii_digit()) && i > 0 && cs[i - 1].is_ascii_digit() {
        return None;
    }
    let mut end = j + 1;
    while starts_with_at(cs, end, ":skin-tone-") {
        let mut k = end + ":skin-tone-".len();
        let digits_from = k;
        while cs.get(k).is_some_and(|c| c.is_ascii_digit()) {
            k += 1;
        }
        if k > digits_from && cs.get(k) == Some(&':') {
            end = k + 1;
        } else {
            break;
        }
    }
    Some((name, end))
}

// ---------------------------------------------------------------------------
// Inline parsing
// ---------------------------------------------------------------------------

enum Item {
    Ch(char),
    Atom { text: String, kind: Kind, code: bool },
}

#[derive(Clone, Copy, Default, PartialEq)]
struct Flags {
    bold: bool,
    italic: bool,
    strike: bool,
}

/// The items for the inside of an inline code span.
fn code_items(cs: &[char]) -> Vec<Item> {
    let mut items = Vec::new();
    let mut buf = String::new();
    let flush = |buf: &mut String, items: &mut Vec<Item>| {
        if !buf.is_empty() {
            items.push(Item::Atom {
                text: std::mem::take(buf),
                kind: Kind::Text,
                code: true,
            });
        }
    };
    let mut i = 0;
    while i < cs.len() {
        if let Some((c, l)) = entity(cs, i) {
            buf.push(c);
            i += l;
            continue;
        }
        if let Some(e) = token_end(cs, i) {
            let inner: String = cs[i + 1..e].iter().collect();
            let (kind, text) = resolve(&inner, &NoNames);
            if matches!(kind, Kind::Link { .. }) {
                flush(&mut buf, &mut items);
                items.push(Item::Atom { text, kind, code: true });
                i = e + 1;
                continue;
            }
        }
        buf.push(cs[i]);
        i += 1;
    }
    flush(&mut buf, &mut items);
    items
}

fn lex_inline(cs: &[char], names: &dyn Names) -> Vec<Item> {
    let n = cs.len();
    let mut items = Vec::new();
    let mut i = 0;
    while i < n {
        let c = cs[i];
        if c == '&'
            && let Some((d, l)) = entity(cs, i)
        {
            items.push(Item::Ch(d));
            i += l;
            continue;
        }
        if c == '<'
            && let Some(e) = token_end(cs, i)
        {
            let inner: String = cs[i + 1..e].iter().collect();
            let (kind, text) = resolve(&inner, names);
            items.push(Item::Atom { text, kind, code: false });
            i = e + 1;
            continue;
        }
        if c == '`'
            && let Some(e) = (i + 1..n).find(|&k| cs[k] == '`')
            && e > i + 1
        {
            items.extend(code_items(&cs[i + 1..e]));
            i = e + 1;
            continue;
        }
        if c == ':'
            && let Some((name, end)) = emoji_at(cs, i)
        {
            items.push(Item::Atom {
                text: format!(":{name}:"),
                kind: Kind::Emoji { name },
                code: false,
            });
            i = end;
            continue;
        }
        items.push(Item::Ch(c));
        i += 1;
    }
    items
}

const MARKERS: [char; 3] = ['*', '_', '~'];
const CLOSE_PUNCT: &str = ".,;:!?)]}\"'";
const OPEN_PUNCT: &str = "([{\"'";

fn marker_index(m: char) -> usize {
    match m {
        '*' => 0,
        '_' => 1,
        _ => 2,
    }
}

/// Which items are consumed markers, and the style of every item.
fn apply_styles(items: &[Item]) -> (Vec<Flags>, Vec<bool>) {
    let n = items.len();
    let ch = |i: usize| -> Option<char> {
        match items.get(i) {
            Some(Item::Ch(c)) => Some(*c),
            _ => None,
        }
    };
    let marker = |i: usize| ch(i).filter(|c| MARKERS.contains(c));

    let can_open = |i: usize, m: char| -> bool {
        let prev_ok = i == 0
            || match items[i - 1] {
                Item::Ch(p) => {
                    p.is_whitespace() || OPEN_PUNCT.contains(p) || (MARKERS.contains(&p) && p != m)
                }
                Item::Atom { .. } => false,
            };
        let next_ok = match items.get(i + 1) {
            None => false,
            Some(Item::Ch(c)) => !c.is_whitespace() && *c != m,
            Some(_) => true,
        };
        prev_ok && next_ok
    };
    let can_close = |i: usize, m: char| -> bool {
        if i == 0 {
            return false;
        }
        let prev_ok = match items[i - 1] {
            Item::Ch(p) => !p.is_whitespace() && p != m,
            Item::Atom { .. } => true,
        };
        let next_ok = match items.get(i + 1) {
            None => true,
            Some(Item::Ch(c)) => {
                c.is_whitespace()
                    || CLOSE_PUNCT.contains(*c)
                    || (MARKERS.contains(c) && *c != m)
            }
            Some(_) => false,
        };
        prev_ok && next_ok
    };

    let mut closers: [Vec<usize>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for i in 0..n {
        if let Some(m) = marker(i)
            && can_close(i, m)
        {
            closers[marker_index(m)].push(i);
        }
    }

    let mut consumed = vec![false; n];
    let mut diff = [vec![0i32; n + 1], vec![0i32; n + 1], vec![0i32; n + 1]];
    for i in 0..n {
        if consumed[i] {
            continue;
        }
        let Some(m) = marker(i) else { continue };
        if !can_open(i, m) {
            continue;
        }
        let list = &closers[marker_index(m)];
        let mut p = list.partition_point(|&j| j <= i);
        while p < list.len() && consumed[list[p]] {
            p += 1;
        }
        if let Some(&j) = list.get(p) {
            consumed[i] = true;
            consumed[j] = true;
            let d = &mut diff[marker_index(m)];
            d[i + 1] += 1;
            d[j] -= 1;
        }
    }

    let mut flags = vec![Flags::default(); n];
    let mut run = [0i32; 3];
    for (k, f) in flags.iter_mut().enumerate() {
        for (r, d) in run.iter_mut().zip(diff.iter()) {
            *r += d[k];
        }
        f.bold = run[0] > 0;
        f.italic = run[1] > 0;
        f.strike = run[2] > 0;
    }
    (flags, consumed)
}

fn push_seg(segs: &mut Vec<Seg>, new: Seg) {
    if new.text.is_empty() {
        return;
    }
    if let Some(last) = segs.last_mut()
        && last.kind == Kind::Text
        && new.kind == Kind::Text
        && last.bold == new.bold
        && last.italic == new.italic
        && last.strike == new.strike
        && last.code == new.code
    {
        last.text.push_str(&new.text);
        return;
    }
    segs.push(new);
}

fn inline_segs(s: &str, names: &dyn Names) -> Vec<Seg> {
    let cs: Vec<char> = s.chars().collect();
    let items = lex_inline(&cs, names);
    let (flags, consumed) = apply_styles(&items);
    let mut segs = Vec::new();
    for (k, item) in items.into_iter().enumerate() {
        if consumed[k] {
            continue;
        }
        let f = flags[k];
        let (text, kind, code) = match item {
            Item::Ch(c) => (c.to_string(), Kind::Text, false),
            Item::Atom { text, kind, code } => (text, kind, code),
        };
        push_seg(
            &mut segs,
            Seg {
                text,
                kind,
                bold: f.bold,
                italic: f.italic,
                strike: f.strike,
                code,
            },
        );
    }
    segs
}

fn normal_line(line: &str, names: &dyn Names) -> DLine {
    let (rest, quote) = if let Some(r) = line.strip_prefix("&gt;") {
        (r, true)
    } else if let Some(r) = line.strip_prefix('>') {
        (r, true)
    } else {
        (line, false)
    };
    let rest = if quote {
        rest.strip_prefix(' ').unwrap_or(rest)
    } else {
        rest
    };
    DLine {
        segs: inline_segs(rest, names),
        quote,
        pre: false,
    }
}

fn pre_line(line: &str) -> DLine {
    let cs: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < cs.len() {
        if let Some((c, l)) = entity(&cs, i) {
            out.push(c);
            i += l;
            continue;
        }
        if let Some(e) = token_end(&cs, i) {
            let inner: String = cs[i + 1..e].iter().collect();
            if let (Kind::Link { url }, _) = resolve(&inner, &NoNames) {
                out.push_str(&url);
                i = e + 1;
                continue;
            }
        }
        out.push(cs[i]);
        i += 1;
    }
    let segs = if out.is_empty() {
        Vec::new()
    } else {
        vec![Seg {
            text: out,
            kind: Kind::Text,
            bold: false,
            italic: false,
            strike: false,
            code: false,
        }]
    };
    DLine {
        segs,
        quote: false,
        pre: true,
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse Slack mrkdwn into display lines.
pub fn parse(text: &str, names: &dyn Names) -> Vec<DLine> {
    let mut out = Vec::new();
    let mut in_pre = false;
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if !line.contains("```") {
            out.push(if in_pre {
                pre_line(line)
            } else {
                normal_line(line, names)
            });
            continue;
        }
        for (k, part) in line.split("```").enumerate() {
            if k > 0 {
                in_pre = !in_pre;
            }
            if part.trim().is_empty() {
                continue;
            }
            out.push(if in_pre {
                pre_line(part)
            } else {
                normal_line(part, names)
            });
        }
    }
    out
}

/// The display text, lines joined with '\n'.
pub fn plain(text: &str, names: &dyn Names) -> String {
    parse(text, names)
        .iter()
        .map(|l| l.segs.iter().map(|s| s.text.as_str()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The first non-empty display line, trimmed.
pub fn first_line(text: &str, names: &dyn Names) -> String {
    plain(text, names)
        .split('\n')
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default()
        .to_string()
}

fn bare_links(seg: &[char], out: &mut Vec<String>) {
    let s: String = seg.iter().collect();
    for word in s.split_whitespace() {
        let word = decode(word);
        let word = word.trim_start_matches(['(', '[', '"', '\'']);
        if !(word.starts_with("http://") || word.starts_with("https://")) {
            continue;
        }
        let url = word.trim_end_matches(['.', ',', ';', ':', '!', '?', ')']);
        if url != "http://" && url != "https://" {
            out.push(url.to_string());
        }
    }
}

/// Every link's URL in the raw text, in order, entities decoded.
pub fn links(text: &str) -> Vec<String> {
    let cs: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < cs.len() {
        if cs[i] == '<'
            && let Some(e) = token_end(&cs, i)
        {
            bare_links(&cs[start..i], &mut out);
            let inner: String = cs[i + 1..e].iter().collect();
            let target = decode(split_label(&inner).0);
            if target.starts_with("http://") || target.starts_with("https://") {
                out.push(target);
            }
            i = e + 1;
            start = i;
            continue;
        }
        i += 1;
    }
    bare_links(&cs[start..], &mut out);
    out
}

/// Every web link in the text with the words it is shown as (`<url|words>`
/// gives the words; a bare link is its own words), in order.
pub fn labelled_links(text: &str) -> Vec<(String, String)> {
    let mut labels: Vec<(String, String)> = Vec::new();
    for l in parse(text, &NoNames) {
        for s in l.segs {
            if let Kind::Link { url } = s.kind {
                labels.push((url, s.text));
            }
        }
    }
    links(text)
        .into_iter()
        .map(|url| {
            let label = labels
                .iter()
                .find(|(u, _)| *u == url)
                .map_or_else(|| url.clone(), |(_, l)| l.clone());
            (label, url)
        })
        .collect()
}

/// The user and group ids mentioned, and whether a broadcast is.
pub fn mentions(text: &str) -> Mentions {
    let cs: Vec<char> = text.chars().collect();
    let mut m = Mentions::default();
    let mut i = 0;
    while i < cs.len() {
        if cs[i] == '<'
            && let Some(e) = token_end(&cs, i)
        {
            let inner: String = cs[i + 1..e].iter().collect();
            let target = split_label(&inner).0;
            if let Some(id) = target.strip_prefix('@') {
                if !id.is_empty() && !m.users.iter().any(|u| u == id) {
                    m.users.push(id.to_string());
                }
            } else if let Some(id) = target.strip_prefix("!subteam^") {
                if !id.is_empty() && !m.groups.iter().any(|g| g == id) {
                    m.groups.push(id.to_string());
                }
            } else if matches!(target, "!here" | "!channel" | "!everyone") {
                m.broadcast = true;
            }
            i = e + 1;
            continue;
        }
        i += 1;
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestNames;
    impl Names for TestNames {
        fn user(&self, id: &str) -> Option<String> {
            (id == "U1").then(|| "alice".to_string())
        }
        fn group(&self, id: &str) -> Option<String> {
            (id == "S1").then(|| "review-squad".to_string())
        }
        fn channel(&self, id: &str) -> Option<String> {
            (id == "C1").then(|| "general".to_string())
        }
    }

    fn sg(text: &str, kind: Kind) -> Seg {
        Seg {
            text: text.into(),
            kind,
            bold: false,
            italic: false,
            strike: false,
            code: false,
        }
    }
    fn t(text: &str) -> Seg {
        sg(text, Kind::Text)
    }
    fn one(text: &str) -> DLine {
        let mut v = parse(text, &NoNames);
        assert_eq!(v.len(), 1, "{v:?}");
        v.remove(0)
    }

    #[test]
    fn group_emoji_link_sentence() {
        let src = "<!subteam^S1> :ship: <https://git.example/acme/api/-/merge_requests/263|MR> to add the endpoint.";
        let url = "https://git.example/acme/api/-/merge_requests/263";
        let v = parse(src, &TestNames);
        assert_eq!(v.len(), 1);
        assert_eq!(
            v[0].segs,
            vec![
                sg("@review-squad", Kind::Group { id: "S1".into() }),
                t(" "),
                sg(":ship:", Kind::Emoji { name: "ship".into() }),
                t(" "),
                sg("MR", Kind::Link { url: url.into() }),
                t(" to add the endpoint."),
            ]
        );
        assert_eq!(links(src), vec![url.to_string()]);
        assert_eq!(mentions(src).groups, vec!["S1".to_string()]);
    }

    #[test]
    fn user_mentions() {
        let k = Kind::User { id: "U1".into() };
        assert_eq!(parse("<@U1>", &TestNames)[0].segs, vec![sg("@alice", k.clone())]);
        assert_eq!(parse("<@U1>", &NoNames)[0].segs, vec![sg("@U1", k.clone())]);
        assert_eq!(parse("<@U1|alice>", &NoNames)[0].segs, vec![sg("@alice", k.clone())]);
        // The cache wins over the label.
        assert_eq!(parse("<@U1|bob>", &TestNames)[0].segs, vec![sg("@alice", k)]);
        assert_eq!(mentions("<@U1> <@U2|x> <@U1>").users, vec!["U1", "U2"]);
    }

    #[test]
    fn group_labels() {
        let k = Kind::Group { id: "S9".into() };
        assert_eq!(one("<!subteam^S9|@team>").segs, vec![sg("@team", k.clone())]);
        assert_eq!(one("<!subteam^S9|team>").segs, vec![sg("@team", k.clone())]);
        assert_eq!(one("<!subteam^S9>").segs, vec![sg("@S9", k)]);
    }

    #[test]
    fn channels() {
        let k = Kind::Channel { id: "C1".into() };
        assert_eq!(parse("<#C1>", &TestNames)[0].segs, vec![sg("#general", k.clone())]);
        assert_eq!(one("<#C1|random>").segs, vec![sg("#random", Kind::Channel { id: "C1".into() })]);
        assert_eq!(one("<#C1>").segs, vec![sg("#C1", k)]);
    }

    #[test]
    fn bullets() {
        let v = parse("• <https://x.example/a|a> \n• <https://x.example/b|b>", &NoNames);
        assert_eq!(v.len(), 2);
        for l in &v {
            assert!(l.segs[0].text.starts_with("• "));
        }
        assert_eq!(v[0].segs[1].text, "a");
        assert_eq!(v[1].segs[1].text, "b");
    }

    #[test]
    fn bold_italic_strike() {
        let l = one("*bold* and _it_ and ~gone~ and snake_case_word");
        let mut b = t("bold");
        b.bold = true;
        let mut i = t("it");
        i.italic = true;
        let mut s = t("gone");
        s.strike = true;
        assert_eq!(
            l.segs,
            vec![b, t(" and "), i, t(" and "), s, t(" and snake_case_word")]
        );
    }

    #[test]
    fn nesting() {
        let l = one("*_both_*");
        assert_eq!(l.segs.len(), 1);
        assert!(l.segs[0].bold && l.segs[0].italic);
        assert_eq!(l.segs[0].text, "both");
    }

    #[test]
    fn link_inside_bold() {
        let l = one("*see <https://x|y>*");
        assert_eq!(l.segs.len(), 2);
        assert!(l.segs.iter().all(|s| s.bold));
        assert_eq!(l.segs[1].kind, Kind::Link { url: "https://x".into() });
    }

    #[test]
    fn quote() {
        let l = one("&gt; quoted &amp; fine");
        assert!(l.quote);
        assert_eq!(l.segs, vec![t("quoted & fine")]);
        let l = one("> raw");
        assert!(l.quote);
        assert_eq!(l.segs, vec![t("raw")]);
        let l = one("&gt;tight");
        assert_eq!(l.segs, vec![t("tight")]);
        assert!(!one("a &gt; b").quote);
    }

    #[test]
    fn pre_block() {
        let v = parse("```\nlet a = 1;\n<https://x|y> *no* :no:\nb &lt; c\n```", &NoNames);
        assert_eq!(v.len(), 3);
        assert!(v.iter().all(|l| l.pre));
        assert_eq!(v[0].segs, vec![t("let a = 1;")]);
        assert_eq!(v[1].segs, vec![t("https://x *no* :no:")]);
        assert_eq!(v[2].segs, vec![t("b < c")]);
    }

    #[test]
    fn pre_fence_with_text_on_same_line() {
        let v = parse("before ```code``` after", &NoNames);
        assert_eq!(v.len(), 3);
        assert!(!v[0].pre && v[1].pre && !v[2].pre);
        assert_eq!(v[0].segs, vec![t("before ")]);
        assert_eq!(v[1].segs, vec![t("code")]);
        assert_eq!(v[2].segs, vec![t(" after")]);
        // Blank line inside a block stays, as a pre line.
        let v = parse("```\na\n\nb\n```", &NoNames);
        assert_eq!(v.len(), 3);
        assert!(v[1].pre && v[1].segs.is_empty());
        // Unclosed block runs to the end.
        let v = parse("```\na\n*b*", &NoNames);
        assert!(v.iter().all(|l| l.pre));
        assert_eq!(v[1].segs, vec![t("*b*")]);
    }

    #[test]
    fn emoji_and_times() {
        let l = one("12:00 - 13:00 :rocket: :+1::skin-tone-2:");
        let emoji: Vec<&str> = l
            .segs
            .iter()
            .filter(|s| matches!(s.kind, Kind::Emoji { .. }))
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(emoji, vec![":rocket:", ":+1:"]);
        assert_eq!(plain("12:00 - 13:00 :rocket: :+1::skin-tone-2:", &NoNames), "12:00 - 13:00 :rocket: :+1:");
        assert!(!one("at 12:00:30 ok").segs.iter().any(|s| matches!(s.kind, Kind::Emoji { .. })));
        assert_eq!(one(":a::b:").segs.len(), 2);
    }

    #[test]
    fn link_entities() {
        let l = one("<https://a.example/x?b=1&amp;c=2|link>");
        assert_eq!(
            l.segs,
            vec![sg("link", Kind::Link { url: "https://a.example/x?b=1&c=2".into() })]
        );
        assert_eq!(links("<https://a.example/x?b=1&amp;c=2|link>"), vec!["https://a.example/x?b=1&c=2"]);
    }

    #[test]
    fn link_without_label_and_mailto() {
        let l = one("<https://x.example/p>");
        assert_eq!(l.segs, vec![sg("https://x.example/p", Kind::Link { url: "https://x.example/p".into() })]);
        let l = one("<mailto:a@b.c|a@b.c>");
        assert_eq!(l.segs, vec![sg("a@b.c", Kind::Link { url: "mailto:a@b.c".into() })]);
        assert!(links("<mailto:a@b.c|x>").is_empty());
    }

    #[test]
    fn broadcast() {
        let l = one("<!here> look");
        assert_eq!(l.segs, vec![sg("@here", Kind::Broadcast), t(" look")]);
        assert!(mentions("<!here> look").broadcast);
        assert!(mentions("<!channel>").broadcast);
        assert!(mentions("<!everyone|everyone>").broadcast);
        assert!(!mentions("here @here").broadcast);
        assert_eq!(one("<!here|here>").segs, vec![sg("@here", Kind::Broadcast)]);
    }

    #[test]
    fn dates() {
        assert_eq!(plain("at <!date^1700000000^{date_short}|Nov 14> ok", &NoNames), "at Nov 14 ok");
        assert_eq!(plain("at <!date^1700000000^{date_short}> ok", &NoNames), "at  ok");
    }

    #[test]
    fn other_tokens_lose_brackets() {
        assert_eq!(plain("a <foo|bar> b", &NoNames), "a foo|bar b");
        assert_eq!(plain("<!foo>", &NoNames), "!foo");
    }

    #[test]
    fn bare_links() {
        assert_eq!(links("see https://bare.example/path."), vec!["https://bare.example/path"]);
        assert_eq!(
            links("(http://a.example/x?a=1&amp;b=2), <https://t.example|t> https://z.example"),
            vec!["http://a.example/x?a=1&b=2", "https://t.example", "https://z.example"]
        );
        assert!(links("https:// nothing").is_empty());
    }

    #[test]
    fn links_come_with_the_words_they_are_shown_as() {
        assert_eq!(
            labelled_links("see <https://x.example/a|the plan> and https://y.example/b."),
            vec![
                ("the plan".to_string(), "https://x.example/a".to_string()),
                ("https://y.example/b".to_string(), "https://y.example/b".to_string()),
            ]
        );
    }

    #[test]
    fn unclosed_and_lone_markers() {
        assert_eq!(one("a <b c").segs, vec![t("a <b c")]);
        assert_eq!(one("1 < 2").segs, vec![t("1 < 2")]);
        assert_eq!(one("<").segs, vec![t("<")]);
        assert_eq!(one("*").segs, vec![t("*")]);
        assert_eq!(one("2 * 3 * 4").segs, vec![t("2 * 3 * 4")]);
        assert_eq!(one("*open only").segs, vec![t("*open only")]);
        assert_eq!(one("a*b*c").segs, vec![t("a*b*c")]);
    }

    #[test]
    fn inline_code() {
        let l = one("run `a *b* :x: <https://u|l> &amp; c` now *y*");
        assert_eq!(l.segs[0], t("run "));
        let c = &l.segs[1];
        assert!(c.code && !c.bold);
        assert_eq!(c.text, "a *b* :x: ");
        assert_eq!(l.segs[2].kind, Kind::Link { url: "https://u".into() });
        assert!(l.segs[2].code);
        assert_eq!(l.segs[2].text, "l");
        assert_eq!(l.segs[3].text, " & c");
        assert!(l.segs[3].code);
        assert!(l.segs.last().is_some_and(|s| s.bold && s.text == "y"));
        assert_eq!(one("a ` b").segs, vec![t("a ` b")]);
    }

    #[test]
    fn entities_in_text() {
        assert_eq!(plain("a &lt;b&gt; &amp;amp;", &NoNames), "a <b> &amp;");
    }

    #[test]
    fn first_line_skips_blanks() {
        assert_eq!(first_line("\n\n  hello *x*\nmore", &NoNames), "hello x");
        assert_eq!(first_line("", &NoNames), "");
        assert_eq!(first_line("   \n ", &NoNames), "");
    }

    #[test]
    fn plain_joins_lines() {
        assert_eq!(plain("&gt; q\nb *c*\n```\nx\n```", &NoNames), "q\nb c\nx");
    }

    #[test]
    fn lines_one_to_one() {
        assert_eq!(parse("a\n\nb", &NoNames).len(), 3);
        assert_eq!(parse("a\r\nb", &NoNames)[0].segs, vec![t("a")]);
    }

    #[test]
    fn no_panic_on_malformed_input() {
        let samples = [
            "", "<", ">", "<>", "< >", "<<<>>>", "<@", "<!subteam^", "<!date^", "`", "``", "```", "````", "`<`>`",
            "*", "**", "_*_*", "~~~", "&", "&amp", "&gt;&gt;", ":", "::", ":::", ":a", ":a::skin-tone-:", ":a::skin-tone-9",
            "é<é|é>é :é: *é*", "\u{0}\n\r\r\n", "<|>", "<a|b|c>", "http://", "https://<",
        ];
        for s in samples {
            let _ = parse(s, &TestNames);
            let _ = plain(s, &NoNames);
            let _ = first_line(s, &NoNames);
            let _ = links(s);
            let _ = mentions(s);
        }
        // Deterministic pseudo-random soup of the special characters.
        let alphabet: Vec<char> = "<>@!#|^&;:`*_~ \n\r.,(a1é+-'/%".chars().collect();
        let mut x: u64 = 0x2545_F491_4F6C_DD1D;
        for _ in 0..3000 {
            let mut s = String::new();
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let len = (x % 40) as usize;
            for _ in 0..len {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                s.push(alphabet[(x % alphabet.len() as u64) as usize]);
            }
            let _ = parse(&s, &TestNames);
            let _ = links(&s);
            let _ = mentions(&s);
        }
    }

    #[test]
    fn long_line_is_fast() {
        let s = "*a ".repeat(20000) + &"<a".repeat(20000);
        let _ = parse(&s, &NoNames);
        let _ = links(&s);
    }
}
