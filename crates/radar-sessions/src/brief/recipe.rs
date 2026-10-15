//! Recipes: `agents/recipes/<name>.md`. A recipe is a Markdown file that
//! starts with a front-matter block of `key: value` lines between `---`
//! lines, then the instructions for the agent. Format in `agents/recipes/README.md`.

/// A parsed recipe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipe {
    pub name: String,
    pub version: u32,
    /// What the recipe needs radar to supply (`context: one or more`).
    pub inputs: String,
    pub scope: String,
    pub permissions: String,
    pub expected_output: String,
    pub stopping_point: String,
    /// The agent a session of this recipe runs as (`agent:`): one name for
    /// every provider, or `provider=name` pairs (`copilot=reviewer,
    /// claude=code-reviewer`). None: the agent Settings → Agents names.
    pub agent: Option<String>,
    /// Skills the session is asked to use (`skills:`, comma separated).
    pub skills: Vec<String>,
    /// The text after the front matter: what the agent is asked to do.
    pub instructions: String,
}

/// The recipes shipped with radar, by name.
pub const BUILT_IN: [(&str, &str); 2] = [
    ("ask", include_str!("../../../../agents/recipes/ask.md")),
    ("review-mr", include_str!("../../../../agents/recipes/review-mr.md")),
];

/// Where a recipe came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    BuiltIn,
    /// The operator's own file, in the recipes folder.
    File(std::path::PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecipeError {
    NoFrontMatter,
    Missing(&'static str),
    BadVersion(String),
}

impl std::fmt::Display for RecipeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoFrontMatter => f.write_str("a recipe starts with a --- front matter block"),
            Self::Missing(k) => write!(f, "a recipe needs `{k}:` in its front matter"),
            Self::BadVersion(v) => write!(f, "recipe version must be a whole number, not `{v}`"),
        }
    }
}

impl std::error::Error for RecipeError {}

impl Recipe {
    pub fn parse(source: &str) -> Result<Recipe, RecipeError> {
        let rest = source.strip_prefix("---\n").ok_or(RecipeError::NoFrontMatter)?;
        let (front, body) = rest.split_once("\n---\n").ok_or(RecipeError::NoFrontMatter)?;
        let get = |key: &'static str| -> Option<String> {
            front.lines().find_map(|l| {
                let (k, v) = l.split_once(':')?;
                (k.trim() == key).then(|| v.trim().to_owned())
            })
        };
        let need = |key: &'static str| get(key).filter(|v| !v.is_empty()).ok_or(RecipeError::Missing(key));
        let version = need("version")?;
        Ok(Recipe {
            name: need("name")?,
            version: version.parse().map_err(|_| RecipeError::BadVersion(version))?,
            inputs: need("inputs")?,
            scope: need("scope")?,
            permissions: need("permissions")?,
            expected_output: need("expected-output")?,
            stopping_point: need("stopping-point")?,
            agent: get("agent").filter(|v| !v.is_empty()),
            skills: get("skills")
                .map(|v| v.split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect())
                .unwrap_or_default(),
            instructions: body.trim_start_matches('\n').to_owned(),
        })
    }

    /// The general "ask about this" recipe shipped with radar.
    pub fn ask() -> Recipe {
        Recipe::built_in("ask").expect("agents/recipes/ask.md is a valid recipe")
    }

    pub fn built_in(name: &str) -> Option<Recipe> {
        BUILT_IN.iter().find(|(n, _)| *n == name).and_then(|(_, text)| Recipe::parse(text).ok())
    }

    /// The recipe `name`: the operator's `<dir>/<name>.md` when there is one
    /// and it parses, else the built-in. A file that does not parse is not
    /// used, and the error says why (the built-in stands in for it).
    pub fn load(name: &str, dir: Option<&std::path::Path>) -> (Option<Recipe>, Source, Option<String>) {
        if let Some(path) = dir.map(|d| d.join(format!("{name}.md")))
            && let Ok(text) = std::fs::read_to_string(&path)
        {
            match Recipe::parse(&text) {
                Ok(r) => return (Some(r), Source::File(path), None),
                Err(e) => return (Recipe::built_in(name), Source::BuiltIn, Some(format!("{}: {e}", path.display()))),
            }
        }
        (Recipe::built_in(name), Source::BuiltIn, None)
    }

    /// Every recipe: the built-ins and the operator's own, by name; the
    /// operator's file replaces a built-in of the same name.
    pub fn names(dir: Option<&std::path::Path>) -> Vec<String> {
        let mut names: Vec<String> = BUILT_IN.iter().map(|(n, _)| (*n).to_owned()).collect();
        if let Some(entries) = dir.and_then(|d| std::fs::read_dir(d).ok()) {
            for e in entries.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "md")
                    && let Some(stem) = p.file_stem().and_then(|s| s.to_str())
                    && stem != "README"
                    && !names.iter().any(|n| n == stem)
                {
                    names.push(stem.to_owned());
                }
            }
        }
        names
    }

    /// The agent for `provider`: `provider=name` when the recipe has such a
    /// pair, else its one plain name.
    pub fn agent_for(&self, provider: &str) -> Option<String> {
        let a = self.agent.as_deref()?;
        let pairs: Vec<(&str, &str)> = a.split(',').filter_map(|p| p.split_once('=')).map(|(k, v)| (k.trim(), v.trim())).collect();
        if pairs.is_empty() {
            return Some(a.trim().to_owned()).filter(|s| !s.is_empty());
        }
        let native = provider.strip_prefix("pty:").unwrap_or(provider);
        pairs.iter().find(|(k, _)| *k == native).map(|(_, v)| (*v).to_owned())
    }

    /// The recipe as a file, with `agent:` and `skills:` set to these
    /// (removed when empty); the rest of the file as it is.
    pub fn with_agent_and_skills(source: &str, agent: Option<&str>, skills: &[String]) -> String {
        let Some(rest) = source.strip_prefix("---\n") else { return source.to_owned() };
        let Some((front, body)) = rest.split_once("\n---\n") else { return source.to_owned() };
        let mut lines: Vec<String> = front
            .lines()
            .filter(|l| !matches!(l.split_once(':').map(|(k, _)| k.trim()), Some("agent" | "skills")))
            .map(str::to_owned)
            .collect();
        if let Some(a) = agent.filter(|a| !a.trim().is_empty()) {
            lines.push(format!("agent: {}", a.trim()));
        }
        if !skills.is_empty() {
            lines.push(format!("skills: {}", skills.join(", ")));
        }
        format!("---\n{}\n---\n{body}", lines.join("\n"))
    }
}

#[cfg(test)]
mod own_tests {
    use super::*;

    #[test]
    fn agent_and_skills_are_read_per_provider_and_written_back() {
        let src = BUILT_IN[1].1;
        let set = Recipe::with_agent_and_skills(src, Some("copilot=reviewer, claude=code-reviewer"), &["notes".into(), "pdf".into()]);
        let r = Recipe::parse(&set).unwrap();
        assert_eq!(r.agent_for("copilot").as_deref(), Some("reviewer"));
        assert_eq!(r.agent_for("pty:claude").as_deref(), Some("code-reviewer"));
        assert_eq!(r.agent_for("codex"), None);
        assert_eq!(r.skills, vec!["notes", "pdf"]);
        assert_eq!(r.instructions, Recipe::parse(src).unwrap().instructions, "the instructions are untouched");
        let plain = Recipe::parse(&Recipe::with_agent_and_skills(&set, Some("reviewer"), &[])).unwrap();
        assert_eq!((plain.agent_for("codex").as_deref(), plain.skills.len()), (Some("reviewer"), 0));
        let none = Recipe::parse(&Recipe::with_agent_and_skills(&set, None, &[])).unwrap();
        assert_eq!((none.agent, none.skills.len()), (None, 0));
    }

    #[test]
    fn an_own_recipe_replaces_the_built_in_and_a_broken_one_says_why() {
        let dir = std::env::temp_dir().join(format!("radar-recipes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let own = Recipe::with_agent_and_skills(BUILT_IN[0].1, Some("asker"), &[]);
        std::fs::write(dir.join("ask.md"), own).unwrap();
        std::fs::write(dir.join("triage.md"), BUILT_IN[0].1.replace("name: ask", "name: triage")).unwrap();
        std::fs::write(dir.join("review-mr.md"), "not a recipe").unwrap();
        let (ask, source, err) = Recipe::load("ask", Some(&dir));
        assert_eq!((ask.unwrap().agent.as_deref(), matches!(source, Source::File(_)), err), (Some("asker"), true, None));
        let (review, source, err) = Recipe::load("review-mr", Some(&dir));
        assert!(review.is_some() && source == Source::BuiltIn && err.unwrap().contains("front matter"));
        assert_eq!(Recipe::names(Some(&dir)), vec!["ask", "review-mr", "triage"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
