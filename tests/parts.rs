//! The independence rules, checked: no part depends on another part, no
//! part names another, and radar-core names none of them. A part is a
//! workspace crate `radar-<id>` other than radar-core; its plugin id is `<id>`.
//! Dependency declarations are the strict architectural boundary; source-name
//! checks below are supplemental vocabulary checks, not dependency detection.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

struct Krate {
    name: String,
    dir: PathBuf,
    deps: Vec<Dependency>,
}

fn workspace_metadata() -> serde_json::Value {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = Command::new(cargo)
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--locked",
            "--offline",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo metadata runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("metadata is JSON")
}

fn workspace() -> Vec<Krate> {
    parse_workspace(&workspace_metadata())
}

/// Inspect declarations, not the resolved graph: disabled optional dependencies,
/// inactive targets, and dev/build dependencies all belong to the boundary.
fn parse_workspace(meta: &serde_json::Value) -> Vec<Krate> {
    meta["packages"]
        .as_array()
        .expect("packages")
        .iter()
        .map(|p| Krate {
            name: p["name"].as_str().expect("package name").to_string(),
            dir: Path::new(p["manifest_path"].as_str().expect("manifest path"))
                .parent()
                .expect("a manifest is in a folder")
                .to_path_buf(),
            deps: p["dependencies"]
                .as_array()
                .expect("dependency declarations")
                .iter()
                .map(Dependency::from_metadata)
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

/// Only these named adapters implement the sessions contract. A common name
/// prefix is not permission to depend on a sibling (or on an arbitrary owner).
const SESSION_ADAPTERS: [&str; 4] = [
    "radar-sessions-claude",
    "radar-sessions-codex",
    "radar-sessions-copilot",
    "radar-sessions-pty",
];

fn is_adapter_of(adapter: &str, part: &str) -> bool {
    part == "radar-sessions" && SESSION_ADAPTERS.contains(&adapter)
}

struct Dependency {
    // Cargo's `name` is the package name even when the manifest key is an alias.
    package: String,
    rename: Option<String>,
    kind: String,
    target: Option<String>,
    optional: bool,
}

impl Dependency {
    fn from_metadata(value: &serde_json::Value) -> Self {
        let optional_string = |field: &str| match &value[field] {
            serde_json::Value::Null => None,
            serde_json::Value::String(text) => Some(text.clone()),
            _ => panic!("dependency {field} must be a string or null"),
        };
        let kind = optional_string("kind").unwrap_or_else(|| "normal".into());
        assert!(
            matches!(kind.as_str(), "normal" | "dev" | "build"),
            "unknown dependency kind: {kind}"
        );
        Self {
            package: value["name"]
                .as_str()
                .expect("dependency package name")
                .into(),
            rename: optional_string("rename"),
            kind,
            target: optional_string("target"),
            optional: value["optional"]
                .as_bool()
                .expect("dependency optional flag"),
        }
    }
}

fn dependency_violations(workspace: &[Krate]) -> Vec<String> {
    let mut violations = Vec::new();
    for krate in workspace
        .iter()
        .filter(|krate| is_part(&krate.name) || krate.name == "radar-core")
    {
        for dependency in &krate.deps {
            if is_part(&dependency.package)
                && dependency.package != krate.name
                && !is_adapter_of(&krate.name, &dependency.package)
            {
                violations.push(format!(
                    "{} depends on the part {} (kind={}, alias={}, target={}, optional={}): parts meet only through radar-core",
                    krate.name,
                    dependency.package,
                    dependency.kind,
                    dependency.rename.as_deref().unwrap_or("<none>"),
                    dependency.target.as_deref().unwrap_or("<all>"),
                    dependency.optional,
                ));
            }
        }
    }
    violations
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
    let violations = dependency_violations(&ws);
    assert!(
        violations.is_empty(),
        "dependency boundary violations:\n{}",
        violations.join("\n")
    );
}

/// Minimal declaration metadata, deliberately without a resolve graph. These
/// unit tests exercise the same parser and boundary as the real workspace guard.
fn metadata_fixture(owner: &str, dependencies: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({
        "packages": [{
            "name": owner,
            "manifest_path": format!("/fixtures/{owner}/Cargo.toml"),
            "dependencies": dependencies,
        }],
        "resolve": null,
    })
}

fn dependency_fixture(package: &str) -> serde_json::Value {
    serde_json::json!({
        "name": package,
        "rename": null,
        "kind": null,
        "target": null,
        "optional": false,
    })
}

#[test]
fn forbidden_dependencies_are_checked_for_every_declaration_shape() {
    // Both the neutral host and a sibling part must reject every combination,
    // including renamed, disabled optional, and foreign-target declarations.
    for owner in ["radar-core", "radar-git"] {
        for kind in [None, Some("dev"), Some("build")] {
            for target in [None, Some("cfg(target_os = \"none\")")] {
                for optional in [false, true] {
                    for rename in [None, Some("innocent_fixture")] {
                        let mut dependency = dependency_fixture("radar-slack");
                        dependency["kind"] = serde_json::json!(kind);
                        dependency["target"] = serde_json::json!(target);
                        dependency["optional"] = serde_json::json!(optional);
                        dependency["rename"] = serde_json::json!(rename);
                        let metadata = metadata_fixture(owner, vec![dependency]);
                        let violations = dependency_violations(&parse_workspace(&metadata));
                        assert_eq!(violations.len(), 1, "missed declaration: {metadata}");
                        let diagnostic = &violations[0];
                        for expected in [
                            owner.to_string(),
                            "radar-slack".to_string(),
                            format!("kind={}", kind.unwrap_or("normal")),
                            format!("alias={}", rename.unwrap_or("<none>")),
                            format!("target={}", target.unwrap_or("<all>")),
                            format!("optional={optional}"),
                        ] {
                            assert!(diagnostic.contains(&expected), "{diagnostic}");
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn only_named_sessions_adapters_may_depend_on_their_owner() {
    for adapter in SESSION_ADAPTERS {
        for kind in [None, Some("dev"), Some("build")] {
            let mut dependency = dependency_fixture("radar-sessions");
            dependency["kind"] = serde_json::json!(kind);
            let metadata = metadata_fixture(adapter, vec![dependency]);
            assert!(dependency_violations(&parse_workspace(&metadata)).is_empty());
        }
    }
    // No implicit exception for arbitrary prefixes, a nested adapter name,
    // reversed ownership, or dependencies between actual adapters.
    for (owner, dependency) in [
        ("radar-git-helper", "radar-git"),
        ("radar-sessions-unknown", "radar-sessions"),
        ("radar-sessions-codex-helper", "radar-sessions-codex"),
        ("radar-sessions", "radar-sessions-codex"),
        ("radar-sessions-codex", "radar-sessions-claude"),
        ("radar-sessions-copilot", "radar-git"),
        ("radar-core", "radar-sessions"),
        ("radar-core", "radar-sessions-codex"),
    ] {
        let metadata = metadata_fixture(owner, vec![dependency_fixture(dependency)]);
        assert_eq!(
            dependency_violations(&parse_workspace(&metadata)).len(),
            1,
            "unexpected exception: {owner} -> {dependency}"
        );
    }
}

#[test]
fn legitimate_host_external_and_binary_dependencies_are_allowed() {
    for (owner, dependencies) in [
        ("radar-git", vec!["radar-core", "serde_json"]),
        ("radar-core", vec!["serde_json"]),
        ("radar-sessions-codex", vec!["radar-core", "radar-sessions"]),
        (
            "radar",
            vec!["radar-core", "radar-git", "radar-sessions-codex"],
        ),
    ] {
        let metadata = metadata_fixture(
            owner,
            dependencies.into_iter().map(dependency_fixture).collect(),
        );
        assert!(dependency_violations(&parse_workspace(&metadata)).is_empty());
    }
    // The actual package governs the boundary, never its manifest alias.
    let mut dependency = dependency_fixture("serde_json");
    dependency["rename"] = serde_json::json!("radar-slack");
    let metadata = metadata_fixture("radar-core", vec![dependency]);
    assert!(dependency_violations(&parse_workspace(&metadata)).is_empty());
}

#[test]
fn all_violations_are_reported_in_declaration_order() {
    let metadata = metadata_fixture(
        "radar-core",
        vec![
            dependency_fixture("radar-git"),
            dependency_fixture("serde_json"),
            dependency_fixture("radar-slack"),
        ],
    );
    let violations = dependency_violations(&parse_workspace(&metadata));
    assert_eq!(violations.len(), 2);
    assert!(violations[0].contains("part radar-git"));
    assert!(violations[1].contains("part radar-slack"));
}

#[test]
#[should_panic(expected = "dependency package name")]
fn malformed_dependencies_are_not_silently_ignored() {
    let mut dependency = dependency_fixture("radar-slack");
    dependency.as_object_mut().unwrap().remove("name");
    parse_workspace(&metadata_fixture("radar-core", vec![dependency]));
}

#[test]
#[should_panic(expected = "dependency declarations")]
fn missing_dependency_lists_are_not_treated_as_empty() {
    let mut metadata = metadata_fixture("radar-core", vec![]);
    metadata["packages"][0]
        .as_object_mut()
        .unwrap()
        .remove("dependencies");
    parse_workspace(&metadata);
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
    assert!(!names_whole(
        "//! radar-gitlab: merge requests",
        "radar-git"
    ));
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
    let core = ws
        .iter()
        .find(|k| k.name == "radar-core")
        .expect("radar-core");
    let mut found = Vec::new();
    for (path, text) in sources(&core.dir) {
        for (n, line) in text.lines().enumerate() {
            let mut lower = line.to_lowercase();
            // Shared credential redaction must recognize real token formats.
            // This literal is a signature, not a dependency on a provider.
            if path.ends_with("radar-core/src/agent_text.rs") {
                lower = lower.replace("\"github_pat_\"", "\"\"");
            }
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

// Root tests may compose owners, but fixture support must not select owners.
// Source cfgs alone cannot protect the dependency graph of an empty test binary.
const FIXTURE_PARTS: [&str; 6] = ["git", "github", "gitlab", "jira", "slack", "sessions"];

const FIXTURE_TARGETS: [(&str, &[&str]); 9] = [
    ("provider_scenarios", &["test-support"]),
    ("slack_scenarios", &["slack", "test-support"]),
    ("local_scenarios", &["git", "workbench", "test-support"]),
    ("agent_scenarios", &["sessions", "test-support"]),
    ("review_scenarios", &["gitlab", "sessions", "test-support"]),
    ("slack_cli_history", &["slack", "test-support"]),
    ("mix", &["test-support"]),
    ("reference_views", &["test-support"]),
    (
        "github_links",
        &["github", "git", "jira", "slack", "demo", "test-support"],
    ),
];

fn strings(value: &serde_json::Value) -> BTreeSet<&str> {
    value
        .as_array()
        .expect("feature list")
        .iter()
        .map(|v| v.as_str().expect("feature name"))
        .collect()
}

/// Expand root feature declarations only, not Cargo's resolved package graph.
/// This checks default/shot paths without requiring unrelated PNG crates cached.
fn root_feature_closure<'a>(
    features: &'a serde_json::Map<String, serde_json::Value>,
    start: &'a str,
) -> BTreeSet<&'a str> {
    let mut found = BTreeSet::new();
    let mut pending = vec![start];
    while let Some(feature) = pending.pop() {
        if found.insert(feature)
            && let Some(entries) = features.get(feature)
        {
            pending.extend(strings(entries));
        }
    }
    found
}

fn root_fixture_violations(root: &serde_json::Value) -> Vec<String> {
    let mut violations = Vec::new();
    for value in root["dependencies"].as_array().expect("root dependencies") {
        let dependency = Dependency::from_metadata(value);
        if is_part(&dependency.package) {
            if dependency.kind != "normal" || !dependency.optional {
                violations.push(format!(
                    "{} must be an optional normal dependency, not kind={} optional={} alias={} target={}",
                    dependency.package,
                    dependency.kind,
                    dependency.optional,
                    dependency.rename.as_deref().unwrap_or("<none>"),
                    dependency.target.as_deref().unwrap_or("<all>"),
                ));
            }
            if strings(&value["features"]).contains("test-support") {
                violations.push(format!(
                    "{} enables test-support in its dependency declaration",
                    dependency.package
                ));
            }
        }
    }
    let features = root["features"].as_object().expect("root features");
    let expected: BTreeSet<String> = std::iter::once("radar-core/test-support".to_string())
        .chain(FIXTURE_PARTS.map(|part| format!("radar-{part}?/test-support")))
        .collect();
    let actual = strings(&features["test-support"]);
    if actual != expected.iter().map(String::as_str).collect() {
        violations.push(format!(
            "root test-support must forward only core plus weak owner features: {actual:?}"
        ));
    }
    for (feature, entries) in features {
        for entry in strings(entries) {
            if let Some(owner) = entry.strip_suffix("/test-support")
                && is_part(owner)
                && !owner.ends_with('?')
            {
                violations.push(format!("{feature} strongly forwards {entry}"));
            }
        }
    }
    for start in ["default", "demo"] {
        for feature in root_feature_closure(features, start) {
            if feature == "test-support"
                || feature
                    .strip_suffix("/test-support")
                    .is_some_and(|owner| is_part(owner.trim_end_matches('?')))
            {
                violations.push(format!(
                    "shipped {start} declaration closure enables fixture support: {feature}"
                ));
            }
        }
    }
    let targets = root["targets"].as_array().expect("root targets");
    for (name, required) in FIXTURE_TARGETS {
        let target = targets.iter().find(|t| t["name"] == name);
        let actual = target
            .and_then(|t| t.get("required-features"))
            .map(strings)
            .unwrap_or_default();
        if actual != required.iter().copied().collect() {
            violations.push(format!(
                "{name} must require exactly {required:?}; got {actual:?}"
            ));
        }
    }
    let probes = targets
        .iter()
        .find(|t| t["name"] == "scenario_probes")
        .expect("pure probe target");
    if probes
        .get("required-features")
        .is_some_and(|value| !strings(value).is_empty())
    {
        violations
            .push("scenario_probes must remain runnable without parts or test-support".into());
    }
    violations
}

fn root_fixture_metadata() -> serde_json::Value {
    let dependencies: Vec<_> = FIXTURE_PARTS
        .map(|part| {
            let mut value = dependency_fixture(&format!("radar-{part}"));
            value["optional"] = serde_json::json!(true);
            value["features"] = serde_json::json!([]);
            value
        })
        .into();
    let support: Vec<_> = std::iter::once("radar-core/test-support".to_string())
        .chain(FIXTURE_PARTS.map(|part| format!("radar-{part}?/test-support")))
        .collect();
    let mut targets: Vec<_> = FIXTURE_TARGETS
        .map(|(name, required)| serde_json::json!({"name": name, "required-features": required}))
        .into();
    targets.push(serde_json::json!({"name": "scenario_probes"}));
    serde_json::json!({
        "dependencies": dependencies,
        "features": {
            "default": ["demo"], "demo": ["radar-core/test-support"],
            "test-support": support,
        },
        "targets": targets,
    })
}

#[test]
fn root_fixture_features_targets_and_shipped_declaration_closures_are_isolated() {
    let metadata = workspace_metadata();
    let root = metadata["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "radar")
        .expect("root package");
    let violations = root_fixture_violations(root);
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

#[test]
fn root_fixture_guard_rejects_strong_forwarding_and_unconditional_owners() {
    assert!(root_fixture_violations(&root_fixture_metadata()).is_empty());
    for part in FIXTURE_PARTS {
        let package = format!("radar-{part}");
        let mut root = root_fixture_metadata();
        root["dependencies"][0]["name"] = serde_json::json!(package);
        root["dependencies"][0]["features"] = serde_json::json!(["test-support"]);
        assert!(
            root_fixture_violations(&root)
                .iter()
                .any(|v| v.contains("dependency declaration"))
        );
        for kind in [None, Some("dev"), Some("build")] {
            for rename in [None, Some("innocent_fixture")] {
                for target in [None, Some("cfg(target_os = \"none\")")] {
                    let mut root = root_fixture_metadata();
                    let mut dependency = dependency_fixture(&package);
                    dependency["kind"] = serde_json::json!(kind);
                    dependency["rename"] = serde_json::json!(rename);
                    dependency["target"] = serde_json::json!(target);
                    dependency["features"] = serde_json::json!(["test-support"]);
                    root["dependencies"]
                        .as_array_mut()
                        .unwrap()
                        .push(dependency);
                    let violations = root_fixture_violations(&root);
                    assert!(violations.iter().any(|v| v.contains(&package)), "{root}");
                }
            }
        }
        for feature in ["test-support", "demo", part] {
            let mut root = root_fixture_metadata();
            root["features"][feature] = serde_json::json!([format!("{package}/test-support")]);
            assert!(
                root_fixture_violations(&root)
                    .iter()
                    .any(|v| v.contains("strongly forwards")),
                "missed strong forwarding: {root}"
            );
        }
    }
}

#[test]
fn shipped_declaration_closure_rejects_direct_transitive_strong_and_weak_fixture_paths() {
    for start in ["default", "demo"] {
        for entry in [
            "test-support",
            "radar-slack/test-support",
            "radar-slack?/test-support",
        ] {
            for transitive in [false, true] {
                let mut root = root_fixture_metadata();
                if transitive {
                    root["features"]["fixture_alias"] = serde_json::json!([entry]);
                    root["features"][start] = serde_json::json!(["fixture_alias"]);
                } else {
                    root["features"][start] = serde_json::json!([entry]);
                }
                assert!(
                    root_fixture_violations(&root)
                        .iter()
                        .any(|v| v.contains(&format!("shipped {start} declaration closure"))),
                    "missed shipped fixture path: {root}"
                );
            }
        }
    }
    // A root feature cycle terminates; core's non-optional support remains valid.
    let mut root = root_fixture_metadata();
    root["features"]["demo"] = serde_json::json!(["radar-core/test-support", "cycle"]);
    root["features"]["cycle"] = serde_json::json!(["demo"]);
    assert!(root_fixture_violations(&root).is_empty());
}

#[test]
fn root_fixture_guard_rejects_missing_target_flags_and_gated_pure_probes() {
    for (name, required) in FIXTURE_TARGETS {
        for missing in required {
            let mut root = root_fixture_metadata();
            let target = root["targets"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|t| t["name"] == name)
                .unwrap();
            target["required-features"] = serde_json::json!(
                required
                    .iter()
                    .filter(|feature| *feature != missing)
                    .collect::<Vec<_>>()
            );
            assert!(
                root_fixture_violations(&root)
                    .iter()
                    .any(|v| v.contains(name))
            );
        }
    }
    let mut root = root_fixture_metadata();
    root["targets"].as_array_mut().unwrap().last_mut().unwrap()["required-features"] =
        serde_json::json!(["test-support"]);
    assert!(
        root_fixture_violations(&root)
            .iter()
            .any(|v| v.contains("scenario_probes"))
    );
}

fn root_tree_owners(features: &str, edges: &str) -> BTreeMap<String, BTreeSet<String>> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = Command::new(cargo)
        .args([
            "tree",
            "--locked",
            "--offline",
            "-p",
            "radar",
            "--no-default-features",
            "--features",
            features,
            "--edges",
            edges,
            "--prefix",
            "none",
            "--format",
            "{p}|{f}",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo tree runs without building");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let tree = String::from_utf8(out.stdout).expect("tree is UTF-8");
    tree.lines()
        .filter_map(|line| {
            let (package, features) = line.split_once('|')?;
            let name = package.split_whitespace().next()?;
            is_part(name).then(|| {
                (
                    name.to_string(),
                    features
                        .trim_end_matches(" (*)")
                        .split(',')
                        .filter(|feature| !feature.is_empty())
                        .map(str::to_string)
                        .collect(),
                )
            })
        })
        .collect()
}

#[test]
fn root_fixture_cargo_graph_selects_no_unrelated_parts() {
    // Inspect the root's actual normal/build/dev graph, not the workspace-wide
    // metadata resolve graph (which includes unrelated workspace members).
    for part in std::iter::once(None).chain(FIXTURE_PARTS.map(Some)) {
        for support in [false, true] {
            let features = part
                .into_iter()
                .chain(support.then_some("test-support"))
                .collect::<Vec<_>>()
                .join(",");
            let owners = root_tree_owners(&features, "normal,build,dev");
            let expected: BTreeSet<_> = part.map(|p| format!("radar-{p}")).into_iter().collect();
            assert_eq!(
                owners.keys().cloned().collect::<BTreeSet<_>>(),
                expected,
                "features={features}; owners={owners:?}"
            );
            for (owner, enabled) in owners {
                assert_eq!(
                    enabled.contains("test-support"),
                    support,
                    "features={features}: {owner} has {enabled:?}"
                );
            }
        }
    }
}

#[test]
fn demo_cargo_graphs_do_not_enable_owner_fixtures() {
    for selection in ["demo", "slack,demo"] {
        let owners = root_tree_owners(selection, "normal,build");
        let expected: BTreeSet<_> = if selection == "demo" {
            BTreeSet::new()
        } else {
            BTreeSet::from(["radar-slack".to_string()])
        };
        assert_eq!(owners.keys().cloned().collect::<BTreeSet<_>>(), expected);
        for (owner, features) in owners {
            assert!(
                !features.contains("test-support"),
                "demo selection {selection} enables fixtures: {owner} {features:?}"
            );
        }
    }
}
