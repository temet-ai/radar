//! `.okf.yaml`: where a workspace keeps its knowledge and where new notes
//! go. Only the few keys radar needs are read, with a small reader for the
//! subset of YAML such a file uses (block maps and lists, quoted or plain
//! scalars, comments):
//!
//! - `sources.documents[].include` / `exclude` and `sources.artifacts[]…`:
//!   the globs of the files that are knowledge;
//! - `create.locations.artifacts.path`: where a new ticket's notes go
//!   (`artifacts/{date}`).

use std::path::Path;

/// A value of the YAML subset.
#[derive(Debug, Clone, PartialEq)]
pub enum Y {
    Map(Vec<(String, Y)>),
    List(Vec<Y>),
    Str(String),
}

impl Y {
    pub fn get(&self, key: &str) -> Option<&Y> {
        match self {
            Y::Map(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn str(&self) -> Option<&str> {
        match self {
            Y::Str(s) => Some(s),
            _ => None,
        }
    }

    /// A list's scalars (a lone scalar is a list of one).
    pub fn strs(&self) -> Vec<String> {
        match self {
            Y::List(l) => l
                .iter()
                .filter_map(|v| v.str().map(str::to_string))
                .collect(),
            Y::Str(s) if !s.is_empty() => vec![s.clone()],
            _ => vec![],
        }
    }

    pub fn items(&self) -> &[Y] {
        match self {
            Y::List(l) => l,
            _ => &[],
        }
    }
}

/// `text` without a trailing comment (a `#` after a space, outside quotes).
fn uncomment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut prev = ' ';
    for (i, c) in line.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '#' && prev.is_whitespace() => return &line[..i],
            None => {}
        }
        prev = c;
    }
    line
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    for q in ['"', '\''] {
        if s.len() >= 2 && s.starts_with(q) && s.ends_with(q) {
            return s[1..s.len() - 1].to_string();
        }
    }
    s.to_string()
}

/// `key: rest` (the key unquoted), when `content` is a map entry.
fn entry(content: &str) -> Option<(String, String)> {
    if content.starts_with(['"', '\'']) {
        return None;
    }
    let at = content
        .find(": ")
        .or_else(|| content.ends_with(':').then(|| content.len() - 1))?;
    Some((
        unquote(&content[..at]),
        content[at + 1..].trim().to_string(),
    ))
}

/// A flow list (`[a, b]`) or a scalar.
fn scalar(rest: &str) -> Y {
    let r = rest.trim();
    if let Some(inner) = r.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
        return Y::List(
            inner
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| Y::Str(unquote(s)))
                .collect(),
        );
    }
    Y::Str(unquote(r))
}

type Lines = Vec<(usize, String)>;

fn block(lines: &mut Lines, i: &mut usize, indent: usize) -> Y {
    let list = lines
        .get(*i)
        .is_some_and(|(_, c)| c == "-" || c.starts_with("- "));
    if list {
        let mut items = Vec::new();
        while let Some((ind, content)) = lines.get(*i).cloned() {
            if ind != indent || !(content == "-" || content.starts_with("- ")) {
                break;
            }
            let rest = content[1..].trim_start().to_string();
            if rest.is_empty() {
                *i += 1;
                items.push(match lines.get(*i) {
                    Some((n, _)) if *n > indent => block(lines, i, *n),
                    _ => Y::Str(String::new()),
                });
            } else if entry(&rest).is_some() {
                // The item is a map: its first entry sits after the dash.
                let inner = indent + (content.len() - rest.len());
                lines[*i] = (inner, rest);
                items.push(block(lines, i, inner));
            } else {
                *i += 1;
                items.push(scalar(&rest));
            }
        }
        return Y::List(items);
    }
    let mut map = Vec::new();
    while let Some((ind, content)) = lines.get(*i).cloned() {
        if ind != indent {
            break;
        }
        let Some((key, rest)) = entry(&content) else {
            break;
        };
        *i += 1;
        if !rest.is_empty() {
            map.push((key, scalar(&rest)));
            continue;
        }
        let value = match lines.get(*i) {
            Some((n, c)) if *n > indent => block(lines, i, *n),
            // A list may sit at its key's own indent.
            Some((n, c)) if *n == indent && (c == "-" || c.starts_with("- ")) => {
                block(lines, i, *n)
            }
            _ => Y::Str(String::new()),
        };
        map.push((key, value));
    }
    Y::Map(map)
}

/// The YAML subset in `text`.
pub fn parse_yaml(text: &str) -> Y {
    let mut lines: Lines = text
        .lines()
        .filter(|l| !l.trim_start().starts_with('#') && l.trim() != "---")
        .map(|l| {
            let l = uncomment(l).trim_end();
            let content = l.trim_start();
            (l.len() - content.len(), content.to_string())
        })
        .filter(|(_, c)| !c.is_empty())
        .collect();
    let first = lines.first().map_or(0, |(n, _)| *n);
    let mut i = 0;
    block(&mut lines, &mut i, first)
}

// --- what a workspace needs ------------------------------------------------------

/// What radar reads of a workspace's `.okf.yaml`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Okf {
    /// Globs of the knowledge files, from `sources.documents` and `sources.artifacts`.
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    /// `create.locations.artifacts.path`: where a ticket's notes are made.
    pub artifacts_path: Option<String>,
}

pub fn parse(text: &str) -> Okf {
    let y = parse_yaml(text);
    let mut okf = Okf::default();
    if let Some(sources) = y.get("sources") {
        for group in ["documents", "artifacts"] {
            let Some(g) = sources.get(group) else {
                continue;
            };
            for item in g.items() {
                okf.include
                    .extend(item.get("include").map(Y::strs).unwrap_or_default());
                okf.exclude
                    .extend(item.get("exclude").map(Y::strs).unwrap_or_default());
            }
        }
    }
    okf.artifacts_path = y
        .get("create")
        .and_then(|c| c.get("locations"))
        .and_then(|l| l.get("artifacts"))
        .and_then(|a| a.get("path"))
        .and_then(Y::str)
        .map(str::to_string)
        .filter(|p| !p.is_empty());
    okf
}

/// The workspace's `.okf.yaml`, if it has one.
pub fn read(top: &Path) -> std::io::Result<Okf> {
    let text = std::fs::read_to_string(top.join(".okf.yaml"))?;
    Ok(parse(&text))
}

/// The folder a glob starts in: its leading components before any wildcard
/// (`artifacts/**/*.md` → `artifacts`; `*.md` → ``).
pub fn static_prefix(glob: &str) -> String {
    let mut parts = Vec::new();
    for c in glob.trim_start_matches("./").split('/') {
        if c.contains(['*', '?', '[', '{']) {
            break;
        }
        parts.push(c);
    }
    // A glob with no wildcard names a file: its folder.
    if parts.len() == glob.split('/').count() {
        parts.pop();
    }
    parts.join("/")
}

impl Okf {
    /// The folders knowledge lives in, relative to the workspace: each
    /// include glob's static prefix, a folder inside another one dropped.
    pub fn roots(&self) -> Vec<String> {
        let mut roots: Vec<String> = Vec::new();
        for g in &self.include {
            let p = static_prefix(g);
            if !roots.contains(&p) {
                roots.push(p);
            }
        }
        let all = roots.clone();
        roots.retain(|r| {
            !all.iter()
                .any(|o| o != r && (o.is_empty() || r.starts_with(&format!("{o}/"))))
        });
        roots.sort();
        roots
    }

    /// Whether `rel` (a path under the workspace, `/`-separated) is knowledge
    /// by the globs. Excludes that name `done` or `archive` are not applied:
    /// a ticket's finished notes are still its notes.
    pub fn holds(&self, rel: &str) -> bool {
        self.include.iter().any(|g| glob(g, rel))
            && !self
                .exclude
                .iter()
                .filter(|g| !finished(g))
                .any(|g| glob(g, rel))
    }
}

fn finished(pattern: &str) -> bool {
    pattern.split('/').any(|c| c == "done" || c == "archive")
}

/// Whether `path` matches `pattern`: `*` within a component, `?` one
/// character, `**` any number of components.
pub fn glob(pattern: &str, path: &str) -> bool {
    let pat: Vec<&str> = pattern.trim_start_matches("./").split('/').collect();
    let parts: Vec<&str> = path.split('/').collect();
    fn walk(pat: &[&str], parts: &[&str]) -> bool {
        match pat.split_first() {
            None => parts.is_empty(),
            Some((&"**", rest)) => (0..=parts.len()).any(|n| walk(rest, &parts[n..])),
            Some((p, rest)) => match parts.split_first() {
                Some((c, tail)) => within(p, c) && walk(rest, tail),
                None => false,
            },
        }
    }
    walk(&pat, &parts)
}

/// `*` and `?` inside one path component.
fn within(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    fn go(p: &[char], n: &[char]) -> bool {
        match p.split_first() {
            None => n.is_empty(),
            Some(('*', rest)) => (0..=n.len()).any(|k| go(rest, &n[k..])),
            Some(('?', rest)) => !n.is_empty() && go(rest, &n[1..]),
            Some((c, rest)) => n.first() == Some(c) && go(rest, &n[1..]),
        }
    }
    go(&p, &n)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
version: 1
bundle: docs/knowledge/bundles/acme   # generated

sources:
  repositories:
    enabled: true
    path: repos.yaml
  documents:
    - name: workspace-documents
      include:
        - docs/**/*.md
      exclude:
        - docs/knowledge/**
        - "**/archive/**"
      tags:
        - workspace
    - name: tenant-docs
      include:
        - repositories/api/docs/**/*.md
  artifacts:
    - name: active-artifacts
      include:
        - artifacts/**/*.md
      exclude:
        - artifacts/archive/**
        - "**/done/**"
create:
  locations:
    artifacts:
      path: artifacts/{date}
      filename: "{slug}.md"
"#;

    #[test]
    fn the_keys_radar_needs_are_read_from_a_real_shaped_file() {
        let okf = parse(SAMPLE);
        assert_eq!(
            okf.include,
            [
                "docs/**/*.md",
                "repositories/api/docs/**/*.md",
                "artifacts/**/*.md"
            ]
        );
        assert_eq!(
            okf.exclude,
            [
                "docs/knowledge/**",
                "**/archive/**",
                "artifacts/archive/**",
                "**/done/**"
            ]
        );
        assert_eq!(okf.artifacts_path.as_deref(), Some("artifacts/{date}"));
        assert_eq!(okf.roots(), ["artifacts", "docs", "repositories/api/docs"]);
    }

    #[test]
    fn the_yaml_subset_handles_maps_lists_flow_lists_and_comments() {
        let y = parse_yaml("a:\n  b: 1 # one\n  c: [x, \"y z\"]\nd:\n- p\n- q: 2\n  r: 3\n");
        assert_eq!(y.get("a").unwrap().get("b").unwrap().str(), Some("1"));
        assert_eq!(y.get("a").unwrap().get("c").unwrap().strs(), ["x", "y z"]);
        let d = y.get("d").unwrap().items();
        assert_eq!(d[0].str(), Some("p"));
        assert_eq!(d[1].get("r").unwrap().str(), Some("3"));
        assert_eq!(parse_yaml("").get("x"), None);
    }

    #[test]
    fn globs_match_by_component_and_finished_notes_stay_knowledge() {
        assert!(glob(
            "artifacts/**/*.md",
            "artifacts/2026-01-01/ABC-12/README.md"
        ));
        assert!(glob("artifacts/**/*.md", "artifacts/README.md"));
        assert!(!glob("artifacts/**/*.md", "docs/README.md"));
        assert!(glob("**/done/**", "artifacts/done/x/y.md"));
        assert!(glob("a?c/*.md", "abc/n.md"));
        let okf = parse(SAMPLE);
        assert!(
            okf.holds("artifacts/done/2026-01-01/ABC-12/README.md"),
            "done/ is still notes"
        );
        assert!(okf.holds("artifacts/archive/old.md"));
        assert!(
            !okf.holds("docs/knowledge/bundle/x.md"),
            "generated knowledge is not notes"
        );
        assert!(!okf.holds("src/main.md"));
    }

    #[test]
    fn a_static_prefix_is_the_folder_before_a_wildcard() {
        assert_eq!(static_prefix("artifacts/**/*.md"), "artifacts");
        assert_eq!(static_prefix("*.md"), "");
        assert_eq!(static_prefix("docs/README.md"), "docs");
        assert_eq!(static_prefix("a/b/{key}/x"), "a/b");
    }
}
