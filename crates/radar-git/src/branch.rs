// T4 uses these functions when it wires the branch actions; until then nothing
// in the part calls them.
#![allow(dead_code)]

//! Branch names: the per-repo convention (`[[git.convention]]`), the template
//! variables that fill it, and the rules git applies to a name. Plain functions;
//! nothing here runs git.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The ticket a branch name is rendered for.
pub struct Ticket<'a> {
    pub key: &'a str,
    pub title: &'a str,
    pub kind: &'a str,
}

/// One `[[git.convention]]`: the repos under `under` (a repo path or a folder
/// of them) get `branch` as their template, and `base` as the branch to start
/// from. Empty fields mean "not set here".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Convention {
    #[serde(default)]
    pub under: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub base: String,
}

const SLUG_MAX: usize = 40;

/// The title as a branch-name part: lower case ASCII, every other run of
/// characters one `-`, no leading or trailing `-`, cut at a `-` to at most 40.
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.len() <= SLUG_MAX {
        return out;
    }
    // Words end at a `-`: keep whole words when one ends exactly at the limit
    // or somewhere before it; a single word longer than the limit is cut.
    if out.as_bytes()[SLUG_MAX] == b'-' {
        return out[..SLUG_MAX].to_string();
    }
    match out[..SLUG_MAX].rfind('-') {
        Some(i) => out[..i].to_string(),
        None => out[..SLUG_MAX].to_string(),
    }
}

/// Fill a template. `{project}` and `{num}` split the key at its last `-`;
/// `{type}` is the ticket type mapped through `types` (keys lower case), or
/// the lower-case type itself. An unknown `{name}` is left as written.
pub fn render(template: &str, me: &str, t: &Ticket, types: &BTreeMap<String, String>) -> String {
    let (project, num) = match t.key.rfind('-') {
        Some(i) => (&t.key[..i], &t.key[i + 1..]),
        None => (t.key, ""),
    };
    let lower = t.kind.to_lowercase();
    let kind = types.get(&lower).cloned().unwrap_or(lower);
    let value = |name: &str| -> Option<String> {
        Some(match name {
            "me" => me.to_string(),
            "key" => t.key.to_string(),
            "project" => project.to_string(),
            "num" => num.to_string(),
            "slug" => slug(t.title),
            "type" => kind.clone(),
            _ => return None,
        })
    };

    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open..];
        let close = after.find('}');
        let next_open = after[1..].find('{').map(|i| i + 1);
        match (close, next_open) {
            // A `{` before the `}` starts a new variable: this one is literal.
            (Some(c), Some(n)) if n < c => {
                out.push('{');
                rest = &after[1..];
            }
            (Some(c), _) => {
                match value(&after[1..c]) {
                    Some(v) => out.push_str(&v),
                    None => out.push_str(&after[..=c]),
                }
                rest = &after[c + 1..];
            }
            (None, _) => {
                out.push_str(after);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Why a template cannot name a branch, or `None` when it can: it has to
/// contain `{key}`, and a sample ticket must render to a name git accepts.
pub fn problem(template: &str) -> Option<&'static str> {
    if template.is_empty() {
        return Some("the template is empty");
    }
    if !template.contains("{key}") {
        return Some("the template has no {key}, so two tickets would share a branch");
    }
    let sample = Ticket {
        key: "ABC-12",
        title: "Add a stats endpoint",
        kind: "Story",
    };
    let name = render(template, "alice", &sample, &BTreeMap::new());
    valid_ref(&name).err()
}

/// git's rules for a branch name (`git check-ref-format --branch`).
pub fn valid_ref(name: &str) -> Result<(), &'static str> {
    if name.is_empty() {
        return Err("the name is empty");
    }
    if name == "@" {
        return Err("the name cannot be @ alone");
    }
    if name.contains("..") {
        return Err("the name has \"..\" in it");
    }
    if name.bytes().any(|b| b < 0x20 || b == 0x7f) {
        return Err("the name has a control character in it");
    }
    if name.chars().any(|c| " ~^:?*[\\".contains(c)) {
        return Err("the name has a space or one of ~ ^ : ? * [ \\");
    }
    if name.starts_with('-') || name.starts_with('/') {
        return Err("the name starts with - or /");
    }
    if name.ends_with('/') || name.ends_with(".lock") || name.ends_with('.') {
        return Err("the name ends with / or .lock or .");
    }
    if name.contains("//") {
        return Err("the name has // in it");
    }
    if name.contains("@{") {
        return Err("the name has @{ in it");
    }
    if name.split('/').any(|part| part.starts_with('.')) {
        return Err("a part of the name starts with .");
    }
    if name.split('/').any(|part| part.ends_with(".lock")) {
        return Err("a part of the name ends with .lock");
    }
    Ok(())
}

/// The template and the base for a repo. The template is the link's branch
/// when it has one, else the matching convention's, else `default`. The base
/// is the matching convention's when it has one. The matching convention is
/// the one with the longest `under` that is the repo or a folder above it.
pub fn pick<'a>(
    repo: &str,
    link_branch: Option<&'a str>,
    conventions: &'a [Convention],
    default: &'a str,
) -> (&'a str, Option<&'a str>) {
    let matched = conventions
        .iter()
        .filter(|c| {
            !c.under.is_empty() && (repo == c.under || repo.starts_with(&format!("{}/", c.under)))
        })
        .max_by_key(|c| c.under.len());
    let template = link_branch
        .filter(|b| !b.is_empty())
        .or_else(|| matched.map(|c| c.branch.as_str()).filter(|b| !b.is_empty()))
        .unwrap_or(default);
    let base = matched.map(|c| c.base.as_str()).filter(|b| !b.is_empty());
    (template, base)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket() -> Ticket<'static> {
        Ticket {
            key: "ABC-12",
            title: "Add a stats endpoint",
            kind: "Story",
        }
    }

    fn conv(under: &str, branch: &str, base: &str) -> Convention {
        Convention {
            under: under.into(),
            branch: branch.into(),
            base: base.into(),
        }
    }

    #[test]
    fn slug_lowercases_and_joins_runs_of_other_characters() {
        assert_eq!(
            slug("Add a stats endpoint, for the DASHBOARD!!"),
            "add-a-stats-endpoint-for-the-dashboard"
        );
        assert_eq!(slug("  --Fix: ünïcode & spaces--  "), "fix-n-code-spaces");
    }

    #[test]
    fn slug_cuts_at_a_word_to_at_most_forty() {
        let title = "one two three four five six seven eight nine ten eleven";
        let s = slug(title);
        assert!(s.len() <= 40, "{s}");
        assert_eq!(s, "one-two-three-four-five-six-seven-eight");
        // A word ending exactly at 40 is kept whole.
        let exact = format!("{} tail", "a".repeat(39));
        assert_eq!(slug(&exact), "a".repeat(39));
        // A single word longer than the limit is cut at the limit.
        assert_eq!(slug(&"b".repeat(50)), "b".repeat(40));
    }

    #[test]
    fn render_fills_key_me_and_splits_the_key() {
        let types = BTreeMap::new();
        assert_eq!(
            render("{me}/{key}", "alice", &ticket(), &types),
            "alice/ABC-12"
        );
        assert_eq!(
            render("{project}-{num}-{slug}", "alice", &ticket(), &types),
            "ABC-12-add-a-stats-endpoint"
        );
    }

    #[test]
    fn render_maps_the_type_and_leaves_unknown_variables() {
        let mut types = BTreeMap::new();
        types.insert("bug".to_string(), "fix".to_string());
        assert_eq!(
            render("{type}/{key}-{slug}", "alice", &ticket(), &types),
            "story/ABC-12-add-a-stats-endpoint"
        );
        let bug = Ticket {
            kind: "Bug",
            ..ticket()
        };
        assert_eq!(render("{type}/{key}", "alice", &bug, &types), "fix/ABC-12");
        assert_eq!(
            render("{me}/{nope}/{key}", "alice", &ticket(), &types),
            "alice/{nope}/ABC-12"
        );
        assert_eq!(
            render("{me}/{key", "alice", &ticket(), &types),
            "alice/{key"
        );
        assert_eq!(render("{{me}}", "alice", &ticket(), &types), "{alice}");
    }

    #[test]
    fn valid_ref_refuses_what_git_refuses() {
        for bad in [
            "",
            "@",
            "a..b",
            "a b",
            "a~b",
            "a^b",
            "a:b",
            "a?b",
            "a*b",
            "a[b",
            "a\\b",
            "a\tb",
            "-a",
            "/a",
            "a/",
            "a.",
            "a.lock",
            "a.lock/b",
            "a/b.lock/c",
            "a//b",
            "a@{b",
            ".a",
            "a/.b",
        ] {
            assert!(valid_ref(bad).is_err(), "{bad:?} should be refused");
        }
        assert!(valid_ref("alice/ABC-12").is_ok());
        assert!(valid_ref("story/ABC-12-add-a-stats-endpoint").is_ok());
        assert!(valid_ref("a.b").is_ok());
    }

    #[test]
    fn problem_refuses_templates_without_key_or_that_name_nothing_valid() {
        assert_eq!(problem("{me}/{key}"), None);
        assert!(problem("{me}/feature").unwrap().contains("{key}"));
        assert!(problem("").is_some());
        assert!(problem("{me} {key}").is_some());
        assert!(problem("{me}/{key}..x").is_some());
        assert!(problem("{key}.lock").is_some());
    }

    #[test]
    fn pick_prefers_the_link_then_the_longest_prefix_then_the_default() {
        let conventions = [
            conv("acme", "acme-{key}", ""),
            conv("acme/team-b", "{type}/{key}-{slug}", "develop"),
        ];
        // A link's branch wins over any convention.
        assert_eq!(
            pick(
                "acme/team-b/api",
                Some("{me}/{key}"),
                &conventions,
                "{me}/{key}"
            ),
            ("{me}/{key}", Some("develop"))
        );
        // The longest matching `under` wins.
        assert_eq!(
            pick("acme/team-b/api", None, &conventions, "{me}/{key}"),
            ("{type}/{key}-{slug}", Some("develop"))
        );
        // A repo equal to `under` matches it.
        assert_eq!(
            pick("acme/team-b", None, &conventions, "{me}/{key}"),
            ("{type}/{key}-{slug}", Some("develop"))
        );
        // A shorter folder matches a repo that is not under the longer one.
        assert_eq!(
            pick("acme/team-c", None, &conventions, "{me}/{key}"),
            ("acme-{key}", None)
        );
        // `acme/team-bb` is not under `acme/team-b`; it is under `acme`.
        assert_eq!(
            pick("acme/team-bb", None, &conventions, "{me}/{key}"),
            ("acme-{key}", None)
        );
        // No match: the default, no base.
        assert_eq!(
            pick("other/repo", None, &conventions, "{me}/{key}"),
            ("{me}/{key}", None)
        );
        // An empty link branch is not a branch: fall through.
        assert_eq!(
            pick("other/repo", Some(""), &conventions, "{me}/{key}"),
            ("{me}/{key}", None)
        );
        // An empty convention branch leaves the default in place but keeps its base.
        let base_only = [conv("acme", "", "develop")];
        assert_eq!(
            pick("acme/api", None, &base_only, "{me}/{key}"),
            ("{me}/{key}", Some("develop"))
        );
    }
}
