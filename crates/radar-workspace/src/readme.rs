//! A ticket's README: made from the workspace's template, and its
//! `## Branches` table filled from what git reports. Plain text in, plain
//! text out; nothing here touches the disk except [`write`].

use std::path::{Path, PathBuf};

use serde_json::Value;

/// radar's template, used when the workspace has no `artifacts/.template.md`.
pub const DEFAULT: &str = "---\ntitle: \"{key}: {title}\"\ntype: ticket\nstatus: planning\ndate: {date}\nticket: {key}\n---\n\n# {key}: {title}\n\n## What we know\n\n## Decisions\n\n## Branches\n\n## Evidence\n";

/// Where a workspace keeps its template, under its top folder.
pub const TEMPLATE: &str = "artifacts/.template.md";

/// The folder a new ticket's notes go in when nothing says otherwise.
pub const DEFAULT_FOLDER: &str = "artifacts/{date}/{key}-{slug}";

const HEADER: [&str; 5] = ["Repo", "Checkout", "Branch", "Base", "Stash"];

/// One row of the Branches table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Row {
    pub repo: String,
    pub checkout: String,
    pub branch: String,
    pub base: String,
    pub stash: String,
}

/// Lower case, every run of other characters one `-`, at most 40 characters
/// cut at a `-`.
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let mut out = out.trim_end_matches('-').to_string();
    if out.len() > 40 {
        out.truncate(40);
        if let Some(at) = out.rfind('-') {
            out.truncate(at);
        }
    }
    out
}

/// `YYYY-MM-DD` of a unix time in seconds (UTC).
pub fn date_of(epoch: i64) -> String {
    let z = epoch.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// A folder or text template filled: `{key}` `{ticket}` `{title}` `{slug}`
/// `{date}` `{type}`.
pub fn fill(template: &str, key: &str, title: &str, kind: &str, date: &str) -> String {
    template
        .replace("{key}", key)
        .replace("{ticket}", key)
        .replace("{title}", title)
        .replace("{slug}", &slug(title))
        .replace("{date}", date)
        .replace("{type}", kind)
}

/// The Branches table as lines: header, rule, one line a row.
pub fn table_lines(rows: &[Row]) -> Vec<String> {
    let line = |cells: [&str; 5]| format!("| {} |", cells.join(" | "));
    let mut out = vec![line(HEADER), line(["---"; 5])];
    out.extend(
        rows.iter()
            .map(|r| line([&r.repo, &r.checkout, &r.branch, &r.base, &r.stash])),
    );
    out
}

/// The rows of `git.started`'s `results`, the checkout shown under `top`.
pub fn rows_from(results: &Value, top: &Path) -> Vec<Row> {
    let text = |v: &Value, k: &str| v[k].as_str().unwrap_or("").to_string();
    results
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or_default()
        .iter()
        .map(|r| {
            let dir = text(r, "dir");
            let place = match text(r, "worktree") {
                w if w.is_empty() => dir.clone(),
                w => w,
            };
            let checkout = Path::new(&place)
                .strip_prefix(top)
                .map_or(place.clone(), |p| p.display().to_string());
            let repo = match text(r, "repo") {
                p if p.is_empty() => Path::new(&dir)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                p => p,
            };
            let ok = r["ok"].as_bool().unwrap_or(false);
            let branch = if ok {
                text(r, "branch")
            } else {
                format!("not started: {}", text(r, "why"))
            };
            Row {
                repo,
                checkout,
                branch,
                base: text(r, "base"),
                stash: text(r, "stash"),
            }
        })
        .collect()
}

fn cells(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|c| c.trim().to_string())
        .collect()
}

fn is_branches(line: &str) -> bool {
    line.trim()
        .strip_prefix("##")
        .is_some_and(|h| !h.starts_with('#') && h.trim().eq_ignore_ascii_case("branches"))
}

/// How many rows the README's Branches table already has.
pub fn rows_in(text: &str) -> usize {
    let mut lines = text.lines().skip_while(|l| !is_branches(l)).skip(1);
    let mut n: usize = 0;
    for l in lines.by_ref() {
        if l.starts_with("## ") {
            break;
        }
        if l.trim_start().starts_with('|') {
            n += 1;
        }
    }
    n.saturating_sub(2)
}

/// `text` with the Branches table holding `rows`: the section's other lines
/// and every other section stay as written. Rows already there for another
/// repo are kept; a row for the same repo is replaced. A README with no such
/// section gets one at the end.
pub fn merge(text: &str, rows: &[Row]) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let Some(start) = lines.iter().position(|l| is_branches(l)) else {
        let mut out = text.trim_end_matches('\n').to_string();
        out.push_str("\n\n## Branches\n\n");
        out.push_str(&table_lines(rows).join("\n"));
        out.push('\n');
        return out;
    };
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.starts_with("## "))
        .map_or(lines.len(), |i| start + 1 + i);
    let section = &lines[start + 1..end];
    let first = section.iter().position(|l| l.trim_start().starts_with('|'));
    let mut kept: Vec<Row> = Vec::new();
    let mut table_end = 0;
    if let Some(f) = first {
        let mut i = f;
        while i < section.len() && section[i].trim_start().starts_with('|') {
            if i >= f + 2 {
                let c = cells(section[i]);
                let get = |n: usize| c.get(n).cloned().unwrap_or_default();
                let row = Row {
                    repo: get(0),
                    checkout: get(1),
                    branch: get(2),
                    base: get(3),
                    stash: get(4),
                };
                if !rows.iter().any(|r| r.repo == row.repo) {
                    kept.push(row);
                }
            }
            i += 1;
        }
        table_end = i;
    }
    kept.extend(rows.iter().cloned());
    let table = table_lines(&kept);
    let mut out: Vec<String> = lines[..=start].iter().map(|l| l.to_string()).collect();
    match first {
        Some(f) => {
            out.extend(section[..f].iter().map(|l| l.to_string()));
            out.extend(table);
            out.extend(section[table_end..].iter().map(|l| l.to_string()));
        }
        None => {
            out.push(String::new());
            out.extend(table);
            // What the section already said follows, after a blank line.
            let rest: Vec<String> = section.iter().map(|l| l.to_string()).collect();
            if rest.iter().any(|l| !l.trim().is_empty()) {
                out.push(String::new());
                out.extend(rest.into_iter().skip_while(|l| l.trim().is_empty()));
            } else {
                out.push(String::new());
            }
        }
    }
    out.extend(lines[end..].iter().map(|l| l.to_string()));
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// A new README: the template filled, its Branches table holding `rows`.
pub fn render_new(
    template: Option<&str>,
    key: &str,
    title: &str,
    kind: &str,
    date: &str,
    rows: &[Row],
) -> String {
    let text = fill(template.unwrap_or(DEFAULT), key, title, kind, date);
    merge(&text, rows)
}

/// The ticket's folder under `top`, as a relative path: its notes' folder
/// when it has one, else the create location filled.
pub fn folder(existing: Option<&str>, create: &str, key: &str, title: &str, date: &str) -> String {
    match existing {
        Some(f) if !f.is_empty() => f.to_string(),
        _ => {
            let create = create.trim().trim_end_matches('/');
            // A location that does not name the ticket (`artifacts/{date}`)
            // holds every ticket of that day: the ticket gets its own folder in it.
            let path = if create.is_empty() {
                DEFAULT_FOLDER.to_string()
            } else if create.contains("{key}") || create.contains("{ticket}") {
                create.to_string()
            } else {
                format!("{create}/{{key}}-{{slug}}")
            };
            fill(&path, key, title, "", date)
                .trim_matches('/')
                .to_string()
        }
    }
}

/// The workspace's template text, if it has one.
pub fn template(top: &Path) -> Option<String> {
    std::fs::read_to_string(top.join(TEMPLATE)).ok()
}

/// Write `text` to `path`, its folders made.
pub fn write(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Whether `text` names the merge request `id` (`group/project!12`): by that
/// reference or by its link (`…/group/project/-/merge_requests/12`), the
/// number whole.
pub fn names_change(text: &str, id: &str) -> bool {
    let Some((project, iid)) = id.rsplit_once('!') else {
        return false;
    };
    if project.is_empty() || iid.is_empty() || !iid.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let forms = [
        (format!("{project}!{iid}"), false),
        (format!("{project}/-/merge_requests/{iid}"), true),
        (format!("{project}/merge_requests/{iid}"), true),
    ];
    forms.iter().any(|(needle, link)| {
        text.match_indices(needle.as_str()).any(|(at, _)| {
            let after = text[at + needle.len()..].chars().next();
            if after.is_some_and(|c| c.is_ascii_digit()) {
                return false;
            }
            // What stands in front must not be more of a path: a project
            // path starts at a word edge, or (a link) right after its host.
            let token = text[..at]
                .rsplit(|c: char| c.is_whitespace() || "([<\"'`,".contains(c))
                .next()
                .unwrap_or("");
            token.is_empty()
                || (*link
                    && token.split_once("://").is_some_and(|(_, host)| {
                        host.strip_suffix('/')
                            .is_some_and(|h| !h.is_empty() && !h.contains('/'))
                    }))
        })
    })
}

/// A file name with no folder in it.
pub fn plain_file_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0'])
}

/// The README's path for a ticket folder (relative to `top`).
pub fn path_in(top: &Path, folder: &str) -> PathBuf {
    top.join(folder).join("README.md")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(repo: &str, branch: &str) -> Row {
        Row {
            repo: repo.into(),
            checkout: format!("repositories/{repo}"),
            branch: branch.into(),
            base: "main".into(),
            stash: String::new(),
        }
    }

    #[test]
    fn slug_and_date() {
        assert_eq!(
            slug("Add a stats endpoint, for the DASHBOARD!!"),
            "add-a-stats-endpoint-for-the-dashboard"
        );
        assert_eq!(date_of(0), "1970-01-01");
        assert_eq!(date_of(1_791_271_000), "2026-10-06");
        assert_eq!(date_of(1_709_164_800), "2024-02-29");
    }

    #[test]
    fn a_new_readme_is_the_template_filled_with_the_table() {
        let text = render_new(
            None,
            "ABC-12",
            "Add stats",
            "Story",
            "2026-10-09",
            &[row("api", "alice/ABC-12")],
        );
        assert!(text.contains("title: \"ABC-12: Add stats\""));
        assert!(text.contains("ticket: ABC-12"));
        assert!(text.contains("## Branches\n\n| Repo | Checkout | Branch | Base | Stash |\n| --- | --- | --- | --- | --- |\n| api | repositories/api | alice/ABC-12 | main |  |\n"));
        assert!(text.ends_with("## Evidence\n"));
        // A workspace's own template with no Branches section gets one.
        let own = render_new(Some("# {key}\n\nnotes\n"), "ABC-12", "x", "Story", "d", &[]);
        assert_eq!(
            own,
            "# ABC-12\n\nnotes\n\n## Branches\n\n| Repo | Checkout | Branch | Base | Stash |\n| --- | --- | --- | --- | --- |\n"
        );
    }

    #[test]
    fn merging_changes_only_the_branches_table() {
        let before = "# ABC-12\n\n## What we know\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n## Branches\n\nSome words.\n\n| Repo | Checkout | Branch | Base | Stash |\n| --- | --- | --- | --- | --- |\n| api | repositories/api | old | main |  |\n| ui | repositories/ui | alice/ABC-12 | main |  |\n\nAfter the table.\n\n## Evidence\n\nkept\n";
        let after = merge(
            before,
            &[row("api", "alice/ABC-12"), row("docs", "alice/ABC-12")],
        );
        let want = "# ABC-12\n\n## What we know\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n## Branches\n\nSome words.\n\n| Repo | Checkout | Branch | Base | Stash |\n| --- | --- | --- | --- | --- |\n| ui | repositories/ui | alice/ABC-12 | main |  |\n| api | repositories/api | alice/ABC-12 | main |  |\n| docs | repositories/docs | alice/ABC-12 | main |  |\n\nAfter the table.\n\n## Evidence\n\nkept\n";
        assert_eq!(after, want);
        assert_eq!(rows_in(before), 2);
        assert_eq!(rows_in("## Branches\n\n| h |\n|---|\n"), 0);
        // Running it again changes nothing.
        assert_eq!(
            merge(
                &after,
                &[row("api", "alice/ABC-12"), row("docs", "alice/ABC-12")]
            ),
            after
        );
    }

    #[test]
    fn a_readme_without_the_section_gets_it_at_the_end() {
        let after = merge("# ABC-12\n\ntext\n", &[row("api", "b")]);
        assert!(after.starts_with("# ABC-12\n\ntext\n\n## Branches\n\n| Repo"));
    }

    #[test]
    fn results_become_rows_under_the_workspace() {
        let results = json!([
            {"dir": "/w/repositories/api", "repo": "acme/team-a/api", "branch": "alice/ABC-12", "base": "main", "worktree": "", "stash": "ABC-12-local-2026-10-09", "ok": true, "why": ""},
            {"dir": "/w/repositories/ui", "repo": "", "branch": "alice/ABC-12", "base": "main", "worktree": "/w/repositories/ui-abc-12", "stash": "", "ok": true, "why": ""},
            {"dir": "/w/repositories/docs", "repo": "", "branch": "x", "base": "main", "worktree": "", "stash": "", "ok": false, "why": "dirty"},
        ]);
        let rows = rows_from(&results, Path::new("/w"));
        assert_eq!(rows[0].repo, "acme/team-a/api");
        assert_eq!(rows[0].checkout, "repositories/api");
        assert_eq!(rows[0].stash, "ABC-12-local-2026-10-09");
        assert_eq!(
            (rows[1].repo.as_str(), rows[1].checkout.as_str()),
            ("ui", "repositories/ui-abc-12")
        );
        assert_eq!(rows[2].branch, "not started: dirty");
    }

    #[test]
    fn the_folder_is_the_existing_one_else_the_create_location() {
        assert_eq!(
            folder(Some("artifacts/d/ABC-12-x"), "", "ABC-12", "t", "d"),
            "artifacts/d/ABC-12-x"
        );
        assert_eq!(
            folder(
                None,
                "artifacts/{date}/{ticket}-{slug}",
                "ABC-12",
                "Add stats",
                "2026-10-09"
            ),
            "artifacts/2026-10-09/ABC-12-add-stats"
        );
        assert_eq!(
            folder(None, "", "ABC-12", "Add stats", "2026-10-09"),
            "artifacts/2026-10-09/ABC-12-add-stats"
        );
    }

    #[test]
    fn a_location_that_does_not_name_the_ticket_gets_a_folder_per_ticket() {
        // The shape .okf.yaml files have: the day's folder, a filename pattern.
        let a = folder(
            None,
            "artifacts/{date}",
            "ABC-12",
            "Add stats",
            "2026-10-09",
        );
        let b = folder(
            None,
            "artifacts/{date}/",
            "ABC-13",
            "Fix the list",
            "2026-10-09",
        );
        assert_eq!(a, "artifacts/2026-10-09/ABC-12-add-stats");
        assert_eq!(b, "artifacts/2026-10-09/ABC-13-fix-the-list");
        assert_ne!(a, b);
    }
}
