//! Evidence for the selected fields of one settings-check reply. The permissive
//! display parser is preserved; these bounded counters never carry returned text.
//! Empty/whitespace strings are still type-valid, not proof of meaningful identity.

use super::{Client, PATH_QUERY, Result, Value, bail, json, str_of, u32_of};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CheckEvidence {
    pub incomplete_objects: u32,
    pub incomplete_fields: u32,
    pub defaulted_fields: u32,
    pub lossy_numbers: u32,
    pub ambiguous_path: bool,
    /// Values reported by this response, never an enumerated inventory.
    pub reported_projects: Option<u32>,
    pub reported_open_merge_requests: Option<u32>,
}

impl CheckEvidence {
    fn object(&mut self, value: &Value) {
        self.incomplete_objects += u32::from(!value.is_object());
    }

    fn text(&mut self, value: &Value) {
        if !value.is_string() {
            self.incomplete_fields += 1;
            self.defaulted_fields += 1;
        }
    }

    fn number(&mut self, value: &Value) -> Option<u32> {
        // Exactly u32_of's accepted values, without treating its wrapping cast
        // as evidence of a smaller response-reported count.
        let number = value
            .as_u64()
            .or_else(|| value.as_str().and_then(|s| s.parse::<u64>().ok()));
        match number {
            Some(number) => match u32::try_from(number) {
                Ok(number) => Some(number),
                Err(_) => {
                    self.incomplete_fields += 1;
                    self.lossy_numbers += 1;
                    None
                }
            },
            None => {
                self.incomplete_fields += 1;
                self.defaulted_fields += 1;
                None
            }
        }
    }

    pub fn incomplete(&self) -> bool {
        self.incomplete_objects != 0 || self.incomplete_fields != 0 || self.ambiguous_path
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CheckRead {
    pub value: String,
    pub evidence: CheckEvidence,
}

impl Client {
    pub(crate) async fn check_user_observed(&self) -> Result<CheckRead> {
        let d = self
            .query("query { currentUser { username name } }", json!({}))
            .await?;
        let user = &d["currentUser"];
        if user.is_null() {
            bail!(
                "GitLab answered, but not as anyone: the token is missing or has no read_api scope"
            );
        }
        let mut evidence = CheckEvidence::default();
        evidence.object(&d);
        evidence.object(user);
        evidence.text(&user["username"]);
        evidence.text(&user["name"]);
        Ok(CheckRead {
            value: format!(
                "signed in as {} ({})",
                str_of(&user["username"]),
                str_of(&user["name"])
            ),
            evidence,
        })
    }

    /// The token's scopes, as GitLab says them for a personal, group or
    /// project access token; `None` when it does not say (an OAuth token
    /// such as glab's, an older server, or any refusal). Never an error:
    /// a check that signed in stays signed in.
    pub(crate) async fn token_scopes(&self) -> Option<Vec<String>> {
        let body = self.get_rest("personal_access_tokens/self").await.ok()?;
        let scopes = body.get("scopes")?.as_array()?;
        Some(scopes.iter().filter_map(|s| s.as_str().map(str::to_owned)).collect())
    }

    pub(crate) async fn check_path_observed(&self, full_path: &str) -> Result<CheckRead> {
        let d = self.query(PATH_QUERY, json!({ "path": full_path })).await?;
        let (g, p) = (&d["group"], &d["project"]);
        let mut evidence = CheckEvidence::default();
        evidence.object(&d);
        evidence.ambiguous_path = !g.is_null() && !p.is_null();
        if !g.is_null() {
            evidence.object(g);
            evidence.object(&g["projects"]);
            evidence.object(&g["mergeRequests"]);
            evidence.text(&g["fullName"]);
            evidence.reported_projects = evidence.number(&g["projects"]["count"]);
            evidence.reported_open_merge_requests = evidence.number(&g["mergeRequests"]["count"]);
            return Ok(CheckRead {
                value: format!(
                    "found the group {}: {} repos, {} open merge requests",
                    str_of(&g["fullName"]),
                    u32_of(&g["projects"]["count"]),
                    u32_of(&g["mergeRequests"]["count"])
                ),
                evidence,
            });
        }
        if !p.is_null() {
            evidence.object(p);
            evidence.object(&p["mergeRequests"]);
            evidence.text(&p["nameWithNamespace"]);
            evidence.reported_open_merge_requests = evidence.number(&p["mergeRequests"]["count"]);
            return Ok(CheckRead {
                value: format!(
                    "found the repo {}: {} open merge requests",
                    str_of(&p["nameWithNamespace"]),
                    u32_of(&p["mergeRequests"]["count"])
                ),
                evidence,
            });
        }
        bail!("no group or repo {full_path}, or this token cannot see it")
    }
}

#[cfg(test)]
mod tests;
