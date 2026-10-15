//! The filter grammar shared by views, `/` and (later) the palette.
//!
//! `is:review,fix` · `author:@me` · `tenant:cm` · `repo:hub` ·
//! `state:merged,closed` (or `all`; open ones only when no filter says) ·
//! `label:bug` · `-term` negates · anything else matches title, branch and
//! repo, case-insensitively.

use crate::model::{Mr, MrState, Role};

#[derive(Debug, Clone, PartialEq)]
enum Term {
    Is(Vec<Role>),
    Author(Vec<String>),
    Tenant(Vec<String>),
    Repo(Vec<String>),
    State(Vec<MrState>),
    Label(Vec<String>),
    Text(String),
}

/// The states a `state:` value names: `all` is every one.
fn states(value: &str) -> Vec<MrState> {
    list(value)
        .iter()
        .flat_map(|w| {
            if w == "all" {
                MrState::ALL.to_vec()
            } else {
                MrState::from_word(w).into_iter().collect()
            }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Filter {
    terms: Vec<(bool, Term)>,
}

/// What a term can be checked against, beyond the MR itself.
pub struct Context<'a> {
    pub me: &'a str,
    pub role: Role,
    pub tenant: &'a str,
}

fn list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

impl Filter {
    pub fn parse(text: &str) -> Self {
        let terms = text
            .split_whitespace()
            .map(|raw| {
                let (negated, raw) = match raw.strip_prefix('-') {
                    Some(rest) if !rest.is_empty() => (true, rest),
                    _ => (false, raw),
                };
                let term = match raw.split_once(':') {
                    Some(("is", v)) => {
                        Term::Is(list(v).iter().filter_map(|w| Role::from_word(w)).collect())
                    }
                    Some(("author", v)) => Term::Author(list(v)),
                    Some(("tenant", v)) => Term::Tenant(list(v)),
                    Some(("repo", v)) => Term::Repo(list(v)),
                    Some(("state", v)) => Term::State(states(v)),
                    Some(("label", v)) => Term::Label(list(v)),
                    _ => Term::Text(raw.to_ascii_lowercase()),
                };
                (negated, term)
            })
            .collect();
        Self { terms }
    }

    pub fn matches(&self, mr: &Mr, cx: &Context<'_>) -> bool {
        self.terms
            .iter()
            .all(|(negated, term)| term_matches(term, mr, cx) != *negated)
    }

    /// Whether a repo with no merge request in view still answers the
    /// filter: each term must be about the repo itself (words, `repo:`,
    /// `tenant:`) and match its name or tenancy. A term about merge requests
    /// (`is:`, `author:`, `state:`, `label:`) leaves the repo out, unless it
    /// is negated: a repo with none has none to leave out.
    pub fn matches_repo(&self, name: &str, tenant: &str) -> bool {
        let name = name.to_ascii_lowercase();
        self.terms.iter().all(|(negated, term)| {
            let hit = match term {
                Term::Text(t) => name.contains(t.as_str()),
                Term::Repo(names) => names.iter().any(|n| name.contains(n.as_str())),
                Term::Tenant(codes) => codes.iter().any(|c| *c == tenant.to_ascii_lowercase()),
                _ => return *negated,
            };
            hit != *negated
        })
    }

    /// Whether it says which states it wants. Without one, only open merge
    /// requests are shown (see [`state_default`]).
    pub fn names_a_state(&self) -> bool {
        self.terms.iter().any(|(_, t)| matches!(t, Term::State(_)))
    }

    /// Whether it could show a merged or closed merge request: what decides
    /// whether those are fetched at all.
    pub fn wants_ended(&self) -> bool {
        self.terms.iter().any(|(negated, t)| match t {
            Term::State(s) if *negated => s.contains(&MrState::Opened),
            Term::State(s) => s.iter().any(|s| *s != MrState::Opened),
            _ => false,
        })
    }
}

/// The rule for filters that say nothing about state: open ones only. Among
/// `filters`, if none names a state, `mr` must be open.
pub fn state_default(filters: &[Filter], mr: &Mr) -> bool {
    filters.iter().any(Filter::names_a_state) || mr.state == MrState::Opened
}

fn term_matches(term: &Term, mr: &Mr, cx: &Context<'_>) -> bool {
    match term {
        Term::Is(roles) => roles.contains(&cx.role),
        Term::Author(names) => names.iter().any(|n| {
            if n == "@me" {
                mr.author.username == cx.me
            } else {
                mr.author.username.to_ascii_lowercase().contains(n)
                    || mr.author.name.to_ascii_lowercase().contains(n)
            }
        }),
        Term::Tenant(codes) => codes.iter().any(|c| c == &cx.tenant.to_ascii_lowercase()),
        Term::Repo(names) => names
            .iter()
            .any(|n| mr.project_name.to_ascii_lowercase().contains(n)),
        Term::State(states) => states.contains(&mr.state),
        Term::Label(names) => names.iter().any(|n| {
            mr.labels
                .iter()
                .any(|l| l.to_ascii_lowercase().contains(n.as_str()))
        }),
        Term::Text(t) => {
            mr.title.to_ascii_lowercase().contains(t)
                || mr.source_branch.to_ascii_lowercase().contains(t)
                || mr.project_name.to_ascii_lowercase().contains(t)
                || format!("!{}", mr.iid) == *t
        }
    }
}

// --- in words, and edited without writing syntax -----------------------------

/// The keys a condition can have, besides free text.
pub const KEYS: [&str; 6] = ["is", "author", "tenant", "repo", "state", "label"];

fn state_phrase(word: &str) -> String {
    match word {
        "open" | "opened" => "still open".into(),
        "merged" => "merged lately".into(),
        "closed" => "closed lately without merging".into(),
        "all" => "open, merged or closed".into(),
        other => format!("\"{other}\" (not a state)"),
    }
}

fn role_phrase(word: &str) -> String {
    match word {
        "review" => "open for your review".into(),
        "fix" => "needing a fix".into(),
        "ready" => "ready to merge".into(),
        "waiting" => "waiting on others".into(),
        "draft" => "drafts".into(),
        "team" => "teammates'".into(),
        "bot" => "from bots".into(),
        other => format!("\"{other}\" (not a state)"),
    }
}

/// One condition, as written, in words: `-author:@me` → "not opened by you".
pub fn describe_term(raw: &str) -> String {
    let (negated, raw) = match raw.strip_prefix('-') {
        Some(rest) if !rest.is_empty() => (true, rest),
        _ => (false, raw),
    };
    let or = |v: &str, f: &dyn Fn(&str) -> String| {
        list(v)
            .iter()
            .map(|w| f(w))
            .collect::<Vec<_>>()
            .join(" or ")
    };
    let body = match raw.split_once(':') {
        Some(("is", v)) => or(v, &role_phrase),
        Some(("author", v)) => or(v, &|n| {
            if n == "@me" {
                "opened by you".into()
            } else {
                format!("opened by {n}")
            }
        }),
        Some(("tenant", v)) => format!("in tenancy {}", or(v, &|n| n.to_string())),
        Some(("repo", v)) => format!("in a repo named like {}", or(v, &|n| n.to_string())),
        Some(("state", v)) => or(v, &state_phrase),
        Some(("label", v)) => format!("labelled {}", or(v, &|n| n.to_string())),
        _ => format!("mentioning \"{raw}\""),
    };
    if negated { format!("not {body}") } else { body }
}

/// A whole filter in words: "open for your review, not from bots".
pub fn describe(text: &str) -> String {
    let terms: Vec<String> = text.split_whitespace().map(describe_term).collect();
    if terms.is_empty() {
        "everything".into()
    } else {
        terms.join(", ")
    }
}

/// What is wrong with a filter, if anything, in words.
pub fn problems(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in text.split_whitespace() {
        let raw = raw.trim_start_matches('-');
        match raw.split_once(':') {
            Some(("is", v)) => {
                for w in list(v) {
                    if Role::from_word(&w).is_none() {
                        out.push(format!("\"{w}\" is not a state; states are review fix ready waiting draft team bot"));
                    }
                }
            }
            Some(("state", v)) => {
                for w in list(v) {
                    if w != "all" && MrState::from_word(&w).is_none() {
                        out.push(format!(
                            "\"{w}\" is not a state; states are open merged closed all"
                        ));
                    }
                }
            }
            Some((k, v)) if KEYS.contains(&k) && list(v).is_empty() => {
                out.push(format!("{k}: needs a value"))
            }
            Some((k, _)) if !KEYS.contains(&k) => out.push(format!(
                "\"{k}:\" is not a condition; use is: author: tenant: repo: state: label:, or plain words"
            )),
            _ => {}
        }
    }
    out
}

/// Add a condition, merging into an existing one of the same kind and sign.
pub fn add(text: &str, negated: bool, key: &str, value: &str) -> String {
    let prefix = format!("{}{key}:", if negated { "-" } else { "" });
    let mut terms: Vec<String> = text.split_whitespace().map(String::from).collect();
    match terms.iter_mut().find(|t| t.starts_with(&prefix)) {
        Some(t) if !list(&t[prefix.len()..]).contains(&value.to_ascii_lowercase()) => {
            t.push(',');
            t.push_str(value);
        }
        Some(_) => {}
        None => terms.push(format!("{prefix}{value}")),
    }
    terms.join(" ")
}

/// Remove the condition at `index`.
pub fn remove(text: &str, index: usize) -> String {
    text.split_whitespace()
        .enumerate()
        .filter(|(i, _)| *i != index)
        .map(|(_, t)| t)
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::fixtures::mr;

    fn cx<'a>(role: Role) -> Context<'a> {
        Context {
            me: "me",
            role,
            tenant: "cm",
        }
    }

    #[test]
    fn is_matches_any_listed_role() {
        let m = mr(1, "alice", "g/api");
        let f = Filter::parse("is:review,fix");
        assert!(f.matches(&m, &cx(Role::Review)));
        assert!(f.matches(&m, &cx(Role::Fix)));
        assert!(!f.matches(&m, &cx(Role::Team)));
    }

    #[test]
    fn negation_author_me_and_free_text_combine() {
        let mine = mr(2, "me", "g/web");
        let theirs = mr(3, "alice", "g/api");
        let team = Filter::parse("-author:@me -is:bot");
        assert!(!team.matches(&mine, &cx(Role::Waiting)));
        assert!(team.matches(&theirs, &cx(Role::Team)));
        assert!(!team.matches(&theirs, &cx(Role::Bot)));
        let text = Filter::parse("WEB tenant:cm");
        assert!(text.matches(&mine, &cx(Role::Waiting)));
        assert!(!text.matches(&theirs, &cx(Role::Team)));
        assert!(Filter::parse("!3").matches(&theirs, &cx(Role::Team)));
    }

    #[test]
    fn a_repo_without_merge_requests_answers_only_the_terms_about_it() {
        let f = |s: &str| Filter::parse(s);
        assert!(f("").matches_repo("billing-hub", "acm"));
        assert!(f("hub").matches_repo("billing-hub", "acm"), "a word in its name");
        assert!(f("HUB").matches_repo("billing-hub", "acm"), "any case");
        assert!(!f("renovate").matches_repo("billing-hub", "acm"), "a word not in it");
        assert!(f("repo:hub tenant:acm").matches_repo("billing-hub", "acm"));
        assert!(!f("tenant:xyz").matches_repo("billing-hub", "acm"));
        assert!(!f("-hub").matches_repo("billing-hub", "acm"), "negated and matching");
        assert!(!f("is:review").matches_repo("billing-hub", "acm"), "it has nothing to review");
        assert!(!f("hub author:@me").matches_repo("billing-hub", "acm"));
        assert!(f("hub -is:bot").matches_repo("billing-hub", "acm"), "nothing to leave out");
    }

    #[test]
    fn filters_read_as_words() {
        assert_eq!(describe(""), "everything");
        assert_eq!(
            describe("-author:@me -is:bot"),
            "not opened by you, not from bots"
        );
        assert_eq!(
            describe("is:review,fix,ready"),
            "open for your review or needing a fix or ready to merge"
        );
        assert_eq!(
            describe("tenant:a deploy"),
            "in tenancy a, mentioning \"deploy\""
        );
    }

    #[test]
    fn problems_say_what_is_wrong() {
        assert!(problems("is:review author:@me").is_empty());
        assert!(problems("is:reveiw")[0].contains("not a state"));
        assert!(problems("owner:bob")[0].contains("not a condition"));
    }

    #[test]
    fn conditions_are_added_merged_and_removed() {
        let f = add("", false, "is", "review");
        let f = add(&f, false, "is", "fix");
        let f = add(&f, true, "author", "@me");
        assert_eq!(f, "is:review,fix -author:@me");
        assert_eq!(add(&f, false, "is", "fix"), f, "no duplicates");
        assert_eq!(remove(&f, 0), "-author:@me");
    }

    #[test]
    fn state_terms_pick_merged_and_closed_and_without_one_only_open_ones_show() {
        let open = mr(1, "alice", "g/api");
        let mut merged = mr(2, "alice", "g/api");
        merged.state = MrState::Merged;
        let mut closed = mr(3, "alice", "g/api");
        closed.state = MrState::Closed;
        let all = [&open, &merged, &closed];
        let shown = |text: &str, extra: &str| -> Vec<u64> {
            let fs = [Filter::parse(text), Filter::parse(extra)];
            all.iter()
                .filter(|m| state_default(&fs, m) && fs.iter().all(|f| f.matches(m, &cx(Role::Team))))
                .map(|m| m.iid)
                .collect()
        };
        assert_eq!(shown("", ""), [1], "open only by default");
        assert_eq!(shown("state:merged", ""), [2]);
        assert_eq!(shown("", "state:merged"), [2], "a state in either filter counts");
        assert_eq!(shown("state:merged,closed", ""), [2, 3]);
        assert_eq!(shown("state:all", ""), [1, 2, 3]);
        assert_eq!(shown("-state:open", ""), [2, 3]);
        assert!(Filter::parse("state:merged").wants_ended());
        assert!(Filter::parse("-state:open").wants_ended());
        assert!(!Filter::parse("state:open is:review").wants_ended());
        assert!(!Filter::parse("-state:merged").wants_ended());
    }

    #[test]
    fn label_terms_match_part_of_any_label() {
        let mut m = mr(1, "alice", "g/api");
        m.labels = vec!["dependencies".into(), "type::Bug".into()];
        assert!(Filter::parse("label:bug").matches(&m, &cx(Role::Team)));
        assert!(Filter::parse("label:dependencies,x").matches(&m, &cx(Role::Team)));
        assert!(!Filter::parse("label:security").matches(&m, &cx(Role::Team)));
        assert!(!Filter::parse("-label:dependencies").matches(&m, &cx(Role::Team)));
        assert_eq!(describe("label:bug state:merged"), "labelled bug, merged lately");
        assert_eq!(describe("state:merged,closed"), "merged lately or closed lately without merging");
        assert!(problems("state:done")[0].contains("not a state"));
        assert!(problems("state:all label:x").is_empty());
    }

    #[test]
    fn empty_filter_matches_everything() {
        assert!(Filter::parse("").matches(&mr(4, "x", "g/r"), &cx(Role::Bot)));
    }
}
