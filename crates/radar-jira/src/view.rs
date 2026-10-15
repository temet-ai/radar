//! A view's conditions, in words, and the JQL they make for each site.
//!
//! A condition is one string in `when`: `assignee:me`, `assignee:none`,
//! `reporter:me`, `status:open` (not done) or `status:todo|doing|done`,
//! `sprint:open`, `project:watched` or `project:ABC`, `type:Bug`,
//! `label:backend`, `updated:14d`, `text:some words`; a leading `-` turns
//! it round. Nobody types these: they are built from menus, and the nav pane
//! and settings show them as words. JQL typed by hand is the other way.

use crate::config::{Site, View, is_project_key};

/// One condition, read.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Cond {
    AssigneeMe,
    Unassigned,
    ReporterMe,
    Open,
    Category(&'static str),
    OpenSprint,
    Watched,
    Project(String),
    Type(String),
    Label(String),
    UpdatedDays(u32),
    Text(String),
}

fn parse(raw: &str) -> Result<(bool, Cond), String> {
    let raw = raw.trim();
    let (not, raw) = match raw.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, raw),
    };
    let (k, v) = raw
        .split_once(':')
        .ok_or_else(|| format!("\"{raw}\" is not a condition"))?;
    let v = v.trim();
    let cond = match (k, v) {
        ("assignee", "me") => Cond::AssigneeMe,
        ("assignee", "none") => Cond::Unassigned,
        ("reporter", "me") => Cond::ReporterMe,
        ("status", "open") => Cond::Open,
        ("status", "todo") => Cond::Category("To Do"),
        ("status", "doing") => Cond::Category("In Progress"),
        ("status", "done") => Cond::Category("Done"),
        ("sprint", "open") => Cond::OpenSprint,
        ("project", "watched") => Cond::Watched,
        ("project", key) if is_project_key(key) => Cond::Project(key.into()),
        ("type", t) if !t.is_empty() => Cond::Type(t.into()),
        ("label", l) if !l.is_empty() && !l.contains(char::is_whitespace) => Cond::Label(l.into()),
        ("updated", d) => match d.trim_end_matches('d').parse::<u32>() {
            Ok(n) if (1..=365).contains(&n) => Cond::UpdatedDays(n),
            _ => return Err(format!("\"{d}\" is not a number of days (1 to 365)")),
        },
        ("text", w) if !w.is_empty() => Cond::Text(w.into()),
        _ => return Err(format!("\"{raw}\" is not a condition")),
    };
    Ok((not, cond))
}

/// A string for JQL: quoted, with `"` and `\` escaped.
pub fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// One condition in words: `assigned to you`, `not done`.
pub fn describe(raw: &str) -> String {
    let Ok((not, c)) = parse(raw) else {
        return format!("? {raw}");
    };
    let (yes, no) = match &c {
        Cond::AssigneeMe => ("assigned to you".into(), "not assigned to you".into()),
        Cond::Unassigned => ("not assigned to anyone".into(), "assigned to someone".into()),
        Cond::ReporterMe => ("reported by you".into(), "reported by someone else".into()),
        Cond::Open => ("not done".into(), "done".into()),
        Cond::Category(c) => {
            let w = category_word(c);
            (w.to_string(), format!("not {w}"))
        }
        Cond::OpenSprint => ("in an open sprint".into(), "not in an open sprint".into()),
        Cond::Watched => (
            "in your watched projects".into(),
            "outside your watched projects".into(),
        ),
        Cond::Project(k) => (format!("in {k}"), format!("not in {k}")),
        Cond::Type(t) => (format!("of type {t}"), format!("not of type {t}")),
        Cond::Label(l) => (format!("labelled {l}"), format!("not labelled {l}")),
        Cond::UpdatedDays(n) => (
            format!("updated in the last {}", days(*n)),
            format!("not updated for {}", days(*n)),
        ),
        Cond::Text(w) => (format!("mentioning \"{w}\""), format!("not mentioning \"{w}\"")),
    };
    if not { no } else { yes }
}

fn days(n: u32) -> String {
    if n == 1 {
        "day".into()
    } else {
        format!("{n} days")
    }
}

fn category_word(c: &str) -> &'static str {
    match c {
        "To Do" => "to do",
        "In Progress" => "doing",
        _ => "done",
    }
}

/// What a view shows, one phrase per condition; the JQL itself is never
/// shown in words, only said to be there.
pub fn describe_view(v: &View) -> Vec<String> {
    let mut out = if !v.jql.trim().is_empty() {
        vec!["what its JQL asks for (settings, views)".to_string()]
    } else if v.when.is_empty() {
        vec!["every ticket updated in the last 30 days".into()]
    } else {
        v.when.iter().map(|c| describe(c)).collect()
    };
    if !v.site.is_empty() {
        out.push(format!("on {} only", v.site));
    }
    out
}

/// Why a view asks `site` nothing, in words.
pub fn why_not(site: &Site, v: &View) -> String {
    if !v.site.is_empty() && v.site != site.name {
        format!("the view asks {} only", v.site)
    } else {
        "it watches no project this needs".into()
    }
}

/// Why a condition cannot be used, if it cannot.
pub fn problem(raw: &str) -> Option<String> {
    parse(raw).err()
}

/// The JQL a view sends to `site`, or `None` when the view has nothing to
/// ask that site (it names watched projects and the site has none, or a
/// project the site does not watch while another site does).
pub fn jql(site: &Site, others_watch: &dyn Fn(&str) -> bool, v: &View) -> Option<String> {
    let order = " ORDER BY updated DESC";
    if !v.site.is_empty() && v.site != site.name {
        return None;
    }
    if !v.jql.trim().is_empty() {
        let q = v.jql.trim().to_string();
        return Some(if q.to_ascii_lowercase().contains("order by") {
            q
        } else {
            format!("{q}{order}")
        });
    }
    let mut parts = Vec::new();
    for raw in &v.when {
        let Ok((not, c)) = parse(raw) else { continue };
        let part = match c {
            Cond::AssigneeMe if not => "(assignee != currentUser() OR assignee is EMPTY)".into(),
            Cond::AssigneeMe => "assignee = currentUser()".into(),
            Cond::Unassigned if not => "assignee is not EMPTY".into(),
            Cond::Unassigned => "assignee is EMPTY".into(),
            Cond::ReporterMe if not => "reporter != currentUser()".into(),
            Cond::ReporterMe => "reporter = currentUser()".into(),
            Cond::Open if not => "statusCategory = Done".into(),
            Cond::Open => "statusCategory != Done".into(),
            Cond::Category(c) => {
                format!("statusCategory {} {}", if not { "!=" } else { "=" }, quote(c))
            }
            Cond::OpenSprint if not => "(sprint not in openSprints() OR sprint is EMPTY)".into(),
            Cond::OpenSprint => "sprint in openSprints()".into(),
            Cond::Watched => {
                if site.projects.is_empty() {
                    if not {
                        continue;
                    }
                    return None;
                }
                let keys: Vec<&str> = site.projects.iter().map(|p| p.key.as_str()).collect();
                format!(
                    "project {} ({})",
                    if not { "not in" } else { "in" },
                    keys.join(", ")
                )
            }
            Cond::Project(k) => {
                if !not && !site.watches(&k) && others_watch(&k) {
                    return None;
                }
                format!("project {} {k}", if not { "!=" } else { "=" })
            }
            Cond::Type(t) => format!("issuetype {} {}", if not { "!=" } else { "=" }, quote(&t)),
            Cond::Label(l) if not => format!("(labels != {} OR labels is EMPTY)", quote(&l)),
            Cond::Label(l) => format!("labels = {}", quote(&l)),
            Cond::UpdatedDays(n) => format!("updated {} -{n}d", if not { "<" } else { ">=" }),
            Cond::Text(w) if not => format!("NOT text ~ {}", quote(&w)),
            Cond::Text(w) => format!("text ~ {}", quote(&w)),
        };
        parts.push(part);
    }
    // Jira refuses a search with nothing to narrow it.
    if parts.is_empty() {
        parts.push("updated >= -30d".into());
    }
    Some(format!("{}{order}", parts.join(" AND ")))
}

/// `jql` asking only what changed in the last `minutes`: its conditions
/// AND `updated >= -Nm`, its order kept. A relative time counts on Jira's
/// clock, so neither the account's time zone nor this machine's clock moves it.
pub fn changed_since(jql: &str, minutes: i64) -> String {
    let (cond, order) = split_order(jql);
    let since = format!("updated >= -{}m", minutes.max(1));
    match cond.trim() {
        "" => format!("{since}{order}"),
        cond => format!("({cond}) AND {since}{order}"),
    }
}

/// A query's conditions, and its ` ORDER BY …` (with one leading space) or
/// nothing: the last `order by` outside quotes.
fn split_order(jql: &str) -> (&str, String) {
    // ASCII lowering keeps every byte where it was.
    let lower = jql.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let (mut quote, mut escaped, mut at) = (None, false, None);
    for (i, &b) in bytes.iter().enumerate() {
        if let Some(q) = quote {
            match b {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                _ if b == q => quote = None,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' | b'\'' => quote = Some(b),
            b'o' if lower[i..].starts_with("order")
                && (i == 0 || bytes[i - 1].is_ascii_whitespace() || bytes[i - 1] == b')') =>
            {
                let rest = &lower[i + 5..];
                let by = rest.trim_start();
                if by.len() < rest.len()
                    && by.starts_with("by")
                    && by[2..].starts_with(|c: char| c.is_ascii_whitespace())
                {
                    at = Some(i);
                }
            }
            _ => {}
        }
    }
    match at {
        Some(i) => (&jql[..i], format!(" {}", jql[i..].trim())),
        None => (jql, String::new()),
    }
}

/// Add a condition, replacing one about the same thing (`status:` or
/// `assignee:`), or the same condition turned round.
pub fn add(when: &[String], cond: &str) -> Vec<String> {
    let key = |c: &str| {
        let c = c.trim_start_matches('-');
        match c.split_once(':') {
            Some(("status" | "assignee" | "sprint" | "updated", _)) => {
                c.split(':').next().unwrap_or(c).to_string()
            }
            _ => c.to_string(),
        }
    };
    let mut out: Vec<String> = when
        .iter()
        .filter(|c| key(c) != key(cond))
        .cloned()
        .collect();
    out.push(cond.to_string());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Project, default_views};

    fn site(projects: &[&str]) -> Site {
        Site {
            name: "acme".into(),
            url: "https://acme.atlassian.net".into(),
            email: String::new(),
            token: String::new(),
            color: "blue".into(),
            projects: projects
                .iter()
                .map(|k| Project {
                    key: k.to_string(),
                    ac_field: String::new(),
                })
                .collect(),
            from: String::new(),
        }
    }

    fn view(when: &[&str]) -> View {
        View {
            name: "v".into(),
            when: when.iter().map(|s| s.to_string()).collect(),
            jql: String::new(),
            site: String::new(),
        }
    }

    const NOBODY: &dyn Fn(&str) -> bool = &|_| false;

    #[test]
    fn the_default_views_say_what_they_show_and_make_jql() {
        let views = default_views();
        let s = site(&["ABC", "DEF"]);
        let words: Vec<Vec<String>> = views.iter().map(describe_view).collect();
        assert_eq!(words[0], ["assigned to you", "not done"]);
        assert_eq!(words[1], ["in an open sprint", "in your watched projects"]);
        assert_eq!(
            words[2],
            ["in your watched projects", "updated in the last 14 days"]
        );
        assert_eq!(
            jql(&s, NOBODY, &views[0]).unwrap(),
            "assignee = currentUser() AND statusCategory != Done ORDER BY updated DESC"
        );
        assert_eq!(
            jql(&s, NOBODY, &views[1]).unwrap(),
            "sprint in openSprints() AND project in (ABC, DEF) ORDER BY updated DESC"
        );
        assert_eq!(
            jql(&site(&[]), NOBODY, &views[1]),
            None,
            "no watched projects: nothing to ask"
        );
    }

    #[test]
    fn turned_round_conditions_and_quoting() {
        let s = site(&["ABC"]);
        let q = jql(
            &s,
            NOBODY,
            &view(&["-assignee:me", "type:User \"Story\"", "-label:ops", "text:a\\b", "-updated:7d"]),
        )
        .unwrap();
        assert_eq!(
            q,
            "(assignee != currentUser() OR assignee is EMPTY) AND issuetype = \"User \\\"Story\\\"\" AND (labels != \"ops\" OR labels is EMPTY) AND text ~ \"a\\\\b\" AND updated < -7d ORDER BY updated DESC"
        );
        assert_eq!(describe("-assignee:me"), "not assigned to you");
        assert_eq!(describe("status:doing"), "doing");
        assert_eq!(describe("-status:done"), "not done");
        assert_eq!(describe("updated:1d"), "updated in the last day");
    }

    #[test]
    fn a_project_goes_to_the_site_that_watches_it() {
        let a = site(&["ABC"]);
        let b = site(&["XYZ"]);
        let watched_somewhere = |k: &str| k == "ABC" || k == "XYZ";
        let v = view(&["project:ABC"]);
        assert!(jql(&a, &watched_somewhere, &v).is_some());
        assert_eq!(jql(&b, &watched_somewhere, &v), None);
        // Watched nowhere: asked of every site.
        assert!(jql(&b, NOBODY, &v).is_some());
    }

    #[test]
    fn hand_written_jql_is_kept_and_ordered() {
        let s = site(&[]);
        let mut v = view(&[]);
        v.jql = "project = ABC".into();
        assert_eq!(jql(&s, NOBODY, &v).unwrap(), "project = ABC ORDER BY updated DESC");
        v.jql = "project = ABC order by created".into();
        assert_eq!(jql(&s, NOBODY, &v).unwrap(), "project = ABC order by created");
        assert_eq!(describe_view(&v).len(), 1);
        assert!(!describe_view(&v)[0].contains("project ="), "never JQL in words");
        assert_eq!(
            jql(&s, NOBODY, &view(&[])).unwrap(),
            "updated >= -30d ORDER BY updated DESC",
            "never unbounded"
        );
        // A view can ask one site only.
        v.site = "beta".into();
        assert_eq!(jql(&s, NOBODY, &v), None);
        assert_eq!(why_not(&s, &v), "the view asks beta only");
        assert_eq!(describe_view(&v).last().unwrap(), "on beta only");
    }

    #[test]
    fn a_changed_since_query_adds_the_updated_clause_before_the_order() {
        assert_eq!(
            changed_since("assignee = currentUser() AND statusCategory != Done ORDER BY updated DESC", 4),
            "(assignee = currentUser() AND statusCategory != Done) AND updated >= -4m ORDER BY updated DESC"
        );
        // Hand-written: any case, no order, an order only, `order by` quoted.
        assert_eq!(changed_since("project = ABC order  by created", 3), "(project = ABC) AND updated >= -3m order  by created");
        assert_eq!(changed_since("project = ABC", 3), "(project = ABC) AND updated >= -3m");
        assert_eq!(changed_since("ORDER BY rank", 3), "updated >= -3m ORDER BY rank");
        assert_eq!(
            changed_since("text ~ \"order by x\" OR summary ~ 'border by'", 2),
            "(text ~ \"order by x\" OR summary ~ 'border by') AND updated >= -2m"
        );
        assert_eq!(
            changed_since("text ~ \"a \\\" order by b\" ORDER BY key", 2),
            "(text ~ \"a \\\" order by b\") AND updated >= -2m ORDER BY key"
        );
        assert_eq!(changed_since("project = ABC", 0), "(project = ABC) AND updated >= -1m", "never an empty window");
    }

    #[test]
    fn adding_replaces_a_condition_about_the_same_thing() {
        let when = vec!["assignee:me".to_string(), "status:open".to_string()];
        assert_eq!(add(&when, "status:doing"), ["assignee:me", "status:doing"]);
        assert_eq!(add(&when, "-assignee:me"), ["status:open", "-assignee:me"]);
        assert_eq!(add(&when, "label:x").len(), 3);
        assert!(problem("updated:soon").is_some());
        assert!(problem("project:abc").is_some());
        assert!(problem("sprint:open").is_none());
    }
}
