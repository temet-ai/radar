//! The independence rules, checked: no part depends on another part, no
//! part names another, and radar-core names none of them. A part is a
//! workspace crate `radar-<id>` other than radar-core; its plugin id is `<id>`.

use std::path::{Path, PathBuf};
use std::process::Command;

struct Krate {
    name: String,
    dir: PathBuf,
    deps: Vec<String>,
}

fn workspace() -> Vec<Krate> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo metadata runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let meta: serde_json::Value = serde_json::from_slice(&out.stdout).expect("metadata is JSON");
    meta["packages"]
        .as_array()
        .expect("packages")
        .iter()
        .map(|p| Krate {
            name: p["name"].as_str().unwrap_or_default().to_string(),
            dir: Path::new(p["manifest_path"].as_str().unwrap_or_default())
                .parent()
                .expect("a manifest is in a folder")
                .to_path_buf(),
            deps: p["dependencies"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|d| d["name"].as_str().map(String::from))
                .collect(),
        })
        .collect()
}

fn is_part(name: &str) -> bool {
    name.starts_with("radar-") && name != "radar-core"
}

/// Every `.rs` file under `dir/src`, with its text.
fn sources(dir: &Path) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    let mut todo = vec![dir.join("src")];
    while let Some(d) = todo.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                todo.push(path);
            } else if path.extension().is_some_and(|x| x == "rs") {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                out.push((path, text));
            }
        }
    }
    out
}

/// How a part is named in code: its crate (`radar-x`, `radar_x`) and its id as a
/// string (`"x"`).
fn names_of(part: &str) -> Vec<String> {
    let id = part.trim_start_matches("radar-");
    let mut names = vec![part.to_string(), part.replace('-', "_")];
    // A part id that is also an ordinary word other parts use for their own
    // things (Slack's workspace, the `workspace` field of a bus payload) is
    // not named by that word: only its crate name counts.
    if !WORDS.contains(&id) {
        names.push(format!("\"{id}\""));
    }
    names
}

/// An adapter crate (`radar-sessions-copilot`) is a part of the part it is named
/// for (`radar-sessions`): it implements that part's trait, so it depends on it
/// and names it. Nothing else may.
fn is_adapter_of(adapter: &str, part: &str) -> bool {
    adapter.starts_with(&format!("{part}-"))
}

/// Part ids that are everyday words in other parts' own vocabulary.
const WORDS: [&str; 1] = ["workspace"];

#[test]
fn no_part_depends_on_another_and_core_depends_on_none() {
    let ws = workspace();
    let parts: Vec<&str> = ws
        .iter()
        .map(|k| k.name.as_str())
        .filter(|n| is_part(n))
        .collect();
    assert!(
        !parts.is_empty() || ws.len() <= 2,
        "no parts found in {:?}",
        ws.iter().map(|k| &k.name).collect::<Vec<_>>()
    );
    for k in ws
        .iter()
        .filter(|k| is_part(&k.name) || k.name == "radar-core")
    {
        for d in &k.deps {
            assert!(
                !(is_part(d) && *d != k.name && !is_adapter_of(&k.name, d)),
                "{} depends on the part {d}: parts meet only through radar-core",
                k.name
            );
        }
    }
}

/// Whether `line` has `name` as a whole name: `radar-git` is not named by
/// `radar-gitlab`, nor `radar_git` by `radar_gitlab`.
fn names_whole(line: &str, name: &str) -> bool {
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    line.match_indices(name).any(|(at, _)| {
        let end = at + name.len();
        // A quoted id ("git") carries its own edges.
        name.ends_with('"') || !line[end..].starts_with(word)
    })
}

#[test]
fn a_name_is_named_only_whole() {
    assert!(names_whole("use radar_git::X;", "radar_git"));
    assert!(!names_whole("use radar_gitlab::X;", "radar_git"));
    assert!(!names_whole("//! radar-gitlab: merge requests", "radar-git"));
    assert!(names_whole("see radar-git.", "radar-git"));
    assert!(names_whole("p.id() == \"git\"", "\"git\""));
}

/// The named exemptions: the reviewer's command policy in radar-sessions matches
/// the program `git` that an agent runs, and the always-allow rules name it
/// among the programs whose subcommand makes a rule (`shell(git status)`).
/// That is a program name, not the radar-git part; every other part name in
/// radar-sessions is still caught.
fn is_program_name(path: &Path, name: &str) -> bool {
    name == "\"git\""
        && (path.ends_with("radar-sessions/src/review/policy.rs")
            || path.ends_with("radar-sessions/src/settings/config.rs"))
}

#[test]
fn no_part_names_another_and_core_names_none() {
    let ws = workspace();
    let parts: Vec<String> = ws
        .iter()
        .map(|k| k.name.clone())
        .filter(|n| is_part(n))
        .collect();
    let mut found = Vec::new();
    for k in ws
        .iter()
        .filter(|k| is_part(&k.name) || k.name == "radar-core")
    {
        let others: Vec<&String> = parts
            .iter()
            .filter(|p| **p != k.name && !is_adapter_of(&k.name, p))
            .collect();
        for (path, text) in sources(&k.dir) {
            for other in &others {
                for name in names_of(other) {
                    for (n, line) in text.lines().enumerate() {
                        if names_whole(line, &name) && !is_program_name(&path, &name) {
                            found.push(format!(
                                "{}:{}: names {other} ({name})",
                                path.display(),
                                n + 1
                            ));
                        }
                    }
                }
            }
        }
    }
    assert!(
        found.is_empty(),
        "parts must not name each other:\n{}",
        found.join("\n")
    );
}

/// Words for tools, services and their things. radar-core is a clean interface
/// that knows none of them: the parts integrate the tools.
const TOOL_WORDS: [&str; 10] = [
    "gitlab",
    "github",
    "glab",
    "jira",
    "slack",
    "flux",
    "merge request",
    "merge-request",
    "merge_request",
    "ticket",
];

#[test]
fn core_names_no_tool_in_code_comments_or_tests() {
    let ws = workspace();
    let core = ws.iter().find(|k| k.name == "radar-core").expect("radar-core");
    let mut found = Vec::new();
    for (path, text) in sources(&core.dir) {
        for (n, line) in text.lines().enumerate() {
            let lower = line.to_lowercase();
            for w in TOOL_WORDS {
                if lower.contains(w) {
                    found.push(format!(
                        "{}:{}: {w}: {}",
                        path.display(),
                        n + 1,
                        line.trim()
                    ));
                }
            }
        }
    }
    assert!(
        found.is_empty(),
        "radar-core names a tool:\n{}",
        found.join("\n")
    );
}

#[test]
fn the_binary_registers_each_part_behind_its_feature() {
    let ws = workspace();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let lib = std::fs::read_to_string(root.join("src/lib.rs")).unwrap();
    for part in ws.iter().map(|k| k.name.as_str()).filter(|n| is_part(n)) {
        let id = part.trim_start_matches("radar-");
        // An adapter's feature also turns on the part it is an adapter of:
        // src/lib.rs registers the adapter inside that part's cfg.
        let owner = ws
            .iter()
            .map(|k| k.name.as_str())
            .find(|p| is_part(p) && is_adapter_of(part, p));
        let want = match owner {
            Some(o) => format!(
                "{id} = [\"{}\", \"dep:{part}\"]",
                o.trim_start_matches("radar-")
            ),
            None => format!("{id} = [\"dep:{part}\"]"),
        };
        assert!(
            manifest.contains(&want),
            "Cargo.toml: the feature `{want}` is missing"
        );
        assert!(
            lib.contains(&format!("#[cfg(feature = \"{id}\")]")),
            "src/lib.rs: {part} is not registered behind `#[cfg(feature = \"{id}\")]`"
        );
    }
}
