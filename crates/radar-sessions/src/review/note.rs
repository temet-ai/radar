//! The local note a finished review is saved as, and the record of which
//! sha it reviewed so the change request's row can say "reviewed" or "stale".
//!
//! A note is a Markdown file with an OKF-style front matter (`type`,
//! `resource`, `title`, `status`) plus what the review needs to be read later:
//! the reviewed sha, the provider, the model and the recipe version. Nothing
//! here posts anything; `posted: false` is written so a reader can tell.

use radar_core::agent_context::ChangeResolution;
use serde::{Deserialize, Serialize};

/// A finished review, ready to be written.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewNote {
    /// The change request's scoped key, as `provider/instance/kind/id`.
    pub subject: String,
    /// The change request's id within its service (`group/api!263`): names the file.
    pub id: String,
    pub title: String,
    pub link: Option<String>,
    pub source_branch: String,
    pub target_branch: String,
    /// The head sha the worktree held: what was reviewed.
    pub reviewed_sha: String,
    pub provider: String,
    pub model: Option<String>,
    pub recipe: String,
    pub recipe_version: u32,
    pub template_version: u32,
    /// Unix seconds, from the injected clock.
    pub reviewed_at: i64,
    /// The review, as the model wrote it.
    pub body: String,
}

impl ReviewNote {
    /// The file's name: the change request, the short sha.
    pub fn file_name(&self) -> String {
        let short: String = self.reviewed_sha.chars().take(8).collect();
        let slug: String = self
            .id
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
            .collect();
        let slug = slug.trim_matches('-');
        format!("review-{slug}-{short}.md")
    }

    /// The file's text.
    pub fn render(&self) -> String {
        let mut out = String::from("---\ntype: review\nstatus: local\nposted: false\n");
        let mut line = |k: &str, v: &str| {
            out.push_str(&format!("{k}: {}\n", quote(v)));
        };
        line("title", &format!("Review of {}", self.title));
        line("resource", &self.subject);
        if let Some(link) = &self.link {
            line("link", link);
        }
        line("source_branch", &self.source_branch);
        line("target_branch", &self.target_branch);
        line("reviewed_sha", &self.reviewed_sha);
        line("provider", &self.provider);
        if let Some(model) = &self.model {
            line("model", model);
        }
        line("recipe", &self.recipe);
        out.push_str(&format!("recipe_version: {}\n", self.recipe_version));
        out.push_str(&format!("template_version: {}\n", self.template_version));
        out.push_str(&format!("reviewed_at: {}\n", self.reviewed_at));
        out.push_str("---\n\n");
        out.push_str(&format!("# Review of {}\n\n", one_line(&self.title)));
        out.push_str(&format!(
            "Local review of `{}` at `{}`, by {}{}, recipe {} v{}. Not posted anywhere.\n\n",
            self.source_branch,
            self.reviewed_sha,
            self.provider,
            self.model.as_ref().map(|m| format!(" ({m})")).unwrap_or_default(),
            self.recipe,
            self.recipe_version
        ));
        out.push_str(self.body.trim());
        out.push('\n');
        out
    }

    /// Read the facts back from a rendered note (for the list of reviews
    /// after a restart). None for any other file.
    pub fn parse(text: &str) -> Option<ReviewRecord> {
        let rest = text.strip_prefix("---\n")?;
        let (front, _) = rest.split_once("\n---\n")?;
        let get = |key: &str| {
            front.lines().find_map(|l| {
                let (k, v) = l.split_once(": ")?;
                (k == key).then(|| unquote(v))
            })
        };
        if get("type")? != "review" {
            return None;
        }
        Some(ReviewRecord {
            subject: get("resource")?,
            reviewed_sha: get("reviewed_sha")?,
            provider: get("provider")?,
            model: get("model"),
            recipe: get("recipe")?,
            recipe_version: get("recipe_version")?.parse().ok()?,
            reviewed_at: get("reviewed_at")?.parse().ok()?,
            path: None,
        })
    }

    pub fn record(&self, path: Option<String>) -> ReviewRecord {
        ReviewRecord {
            subject: self.subject.clone(),
            reviewed_sha: self.reviewed_sha.clone(),
            provider: self.provider.clone(),
            model: self.model.clone(),
            recipe: self.recipe.clone(),
            recipe_version: self.recipe_version,
            reviewed_at: self.reviewed_at,
            path,
        }
    }
}

/// What the row needs to know about a saved review.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewRecord {
    pub subject: String,
    pub reviewed_sha: String,
    pub provider: String,
    pub model: Option<String>,
    pub recipe: String,
    pub recipe_version: u32,
    pub reviewed_at: i64,
    pub path: Option<String>,
}

/// How a saved review stands against the change's head now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    /// Reviewed the head that is there now.
    Current,
    /// The head moved after the review.
    Stale,
}

impl ReviewRecord {
    pub fn standing(&self, head_sha: &str) -> Standing {
        if self.reviewed_sha == head_sha { Standing::Current } else { Standing::Stale }
    }
}

/// The reviews saved so far, newest per change request.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reviews {
    records: Vec<ReviewRecord>,
}

impl Reviews {
    pub fn add(&mut self, record: ReviewRecord) {
        match self.records.iter_mut().find(|r| r.subject == record.subject) {
            Some(held) if held.reviewed_at > record.reviewed_at => {}
            Some(held) => *held = record,
            None => self.records.push(record),
        }
    }
    /// The review of the change request whose id in its service is `id`
    /// (`group/api!263`), whichever instance it is on.
    pub fn of_id(&self, id: &str) -> Option<&ReviewRecord> {
        self.records.iter().find(|r| r.subject.ends_with(&format!("/{id}")))
    }
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
    pub fn of(&self, subject: &str) -> Option<&ReviewRecord> {
        self.records.iter().find(|r| r.subject == subject)
    }
    /// The row's word: `reviewed abc1234` or `review stale (abc1234)`; None
    /// when the change request was never reviewed.
    pub fn chip(&self, subject: &str, head_sha: Option<&str>) -> Option<String> {
        let r = self.of(subject)?;
        let short: String = r.reviewed_sha.chars().take(7).collect();
        Some(match head_sha.map(|h| r.standing(h)) {
            Some(Standing::Stale) => format!("review stale ({short})"),
            _ => format!("reviewed {short}"),
        })
    }
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn quote(value: &str) -> String {
    serde_json::to_string(&one_line(value)).unwrap_or_else(|_| "\"\"".into())
}

fn unquote(value: &str) -> String {
    serde_json::from_str::<String>(value.trim()).unwrap_or_else(|_| value.trim().to_owned())
}

impl From<&ChangeResolution> for ReviewNote {
    /// A note with the facts of the change; the journey fills in the rest.
    fn from(c: &ChangeResolution) -> Self {
        let s = &c.subject;
        ReviewNote {
            subject: format!("{}/{}/{}/{}", s.provider, s.instance, s.kind, s.id),
            id: s.id.clone(),
            title: c.title.clone(),
            link: c.link.clone(),
            source_branch: c.source_branch.clone(),
            target_branch: c.target_branch.clone(),
            reviewed_sha: c.head_sha.clone(),
            provider: String::new(),
            model: None,
            recipe: String::new(),
            recipe_version: 0,
            template_version: 0,
            reviewed_at: 0,
            body: String::new(),
        }
    }
}
