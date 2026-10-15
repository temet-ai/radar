//! The same configured, bounded open-PR read in the command line.
use crate::client::Client;
use crate::config::Settings;
use crate::model::{ApiErrorKind, Collection, PullRequest, Repository, RepositoryId};
use clap::{Arg, ArgAction, ArgMatches, Command};
use radar_core::plugin::CliTask;
use serde::Serialize;

pub(crate) fn command() -> Command {
    Command::new("github")
        .about("Read-only GitHub repositories and open pull requests")
        .subcommand_required(true)
        .subcommand(
            Command::new("list")
                .about("Read configured repositories and their open pull requests")
                .arg(
                    Arg::new("json")
                        .long("json")
                        .action(ArgAction::SetTrue)
                        .help("JSON with explicit read and pagination completeness"),
                ),
        )
}

#[derive(Serialize)]
struct Row {
    id: RepositoryId,
    repository: Option<Repository>,
    pull_requests: Option<Collection<PullRequest>>,
    errors: Vec<String>,
}

impl Row {
    fn complete(&self) -> bool {
        self.repository.is_some()
            && self.errors.is_empty()
            && self
                .pull_requests
                .as_ref()
                .is_some_and(|p| p.completion.complete)
    }
}

fn stop_after(kind: ApiErrorKind) -> bool {
    matches!(
        kind,
        ApiErrorKind::Authentication
            | ApiErrorKind::RateLimit
            | ApiErrorKind::MissingToken
            | ApiErrorKind::InvalidToken
    )
}

pub(crate) fn run(cfg: &Settings, matches: &ArgMatches) -> CliTask {
    let cfg = cfg.clone();
    let json = matches
        .subcommand_matches("list")
        .map(|m| m.get_flag("json"));
    Box::pin(async move {
        let json = json.ok_or_else(|| anyhow::anyhow!("unknown GitHub command"))?;
        let repos: Vec<_> = cfg
            .repositories
            .iter()
            .map(|s| RepositoryId::parse(s))
            .collect::<Result<_, _>>()?;
        anyhow::ensure!(
            !repos.is_empty(),
            "No GitHub repositories configured; add owner/repo in settings, GitHub, repositories"
        );
        let client = Client::new(&repos, &cfg.token, &cfg.api)?;
        let mut rows = Vec::new();
        let mut stopped = false;
        for id in repos {
            let mut row = Row {
                id,
                repository: None,
                pull_requests: None,
                errors: vec![],
            };
            if stopped {
                row.errors.push(
                    "Not requested after authentication or rate-limit refusal; refresh later"
                        .into(),
                );
                rows.push(row);
                continue;
            }
            match client.repository(&row.id).await {
                Ok(repository) => row.repository = Some(repository),
                Err(error) => {
                    stopped = stop_after(error.kind);
                    row.errors.push(error.to_string());
                    rows.push(row);
                    continue;
                }
            }
            match client.open_pulls(&row.id).await {
                Ok(pulls) => {
                    stopped = pulls.completion.error.is_some_and(stop_after);
                    row.pull_requests = Some(pulls);
                }
                Err(error) => {
                    stopped = stop_after(error.kind);
                    row.errors.push(error.to_string());
                }
            }
            rows.push(row);
        }
        if json {
            Ok(serde_json::to_string_pretty(&serde_json::json!({
                "complete": rows.iter().all(Row::complete), "repositories": rows,
            }))? + "\n")
        } else {
            Ok(render(&rows))
        }
    })
}

fn one_line(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

fn render(rows: &[Row]) -> String {
    let mut out = String::new();
    if rows.iter().any(|r| !r.complete()) {
        out.push_str("Partial result: some requested data is incomplete or unavailable.\n");
    }
    for row in rows {
        out.push_str(&format!("{}\n", row.id.full_name()));
        for error in &row.errors {
            out.push_str(&format!("  {error}\n"));
        }
        let Some(pulls) = &row.pull_requests else {
            out.push_str("  Open pull requests were not loaded.\n");
            continue;
        };
        if !pulls.completion.complete {
            out.push_str("  Partial open pull request list: ");
            out.push_str(
                pulls
                    .completion
                    .reason
                    .as_deref()
                    .unwrap_or("read was incomplete"),
            );
            out.push('\n');
        } else if pulls.items.is_empty() {
            out.push_str("  No open pull requests\n");
        }
        for pull in &pulls.items {
            out.push_str(&format!(
                "  #{}  {}  {}\n",
                pull.number,
                if pull.draft { "draft" } else { "open" },
                one_line(&pull.title)
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Completion;

    #[test]
    fn json_flag_and_partial_empty_output_are_explicit() {
        let args = command()
            .try_get_matches_from(["github", "list", "--json"])
            .unwrap();
        assert!(args.subcommand_matches("list").unwrap().get_flag("json"));
        let repo = Repository {
            id: RepositoryId::parse("acme/api").unwrap(),
            description: None,
            default_branch: "main".into(),
            private: false,
            archived: false,
        };
        let rows = vec![Row {
            id: repo.id.clone(),
            repository: Some(repo),
            errors: vec![],
            pull_requests: Some(Collection {
                items: vec![],
                completion: Completion::partial(0, "page limit reached"),
            }),
        }];
        let text = render(&rows);
        assert!(text.contains("Partial open pull request list"));
        assert!(!text.contains("No open pull requests"));
        let value = serde_json::to_value(&rows).unwrap();
        assert_eq!(value[0]["pull_requests"]["completion"]["complete"], false);
        assert!(!rows[0].complete());
        assert_eq!(one_line("title\u{1b}[31m\nnext"), "title [31m next");
    }

    #[test]
    fn authentication_and_rate_refusals_stop_further_reads() {
        for kind in [
            ApiErrorKind::Authentication,
            ApiErrorKind::RateLimit,
            ApiErrorKind::MissingToken,
            ApiErrorKind::InvalidToken,
        ] {
            assert!(stop_after(kind));
        }
        for kind in [
            ApiErrorKind::NotFound,
            ApiErrorKind::Forbidden,
            ApiErrorKind::Body,
        ] {
            assert!(!stop_after(kind));
        }
        let row = Row {
            id: RepositoryId::parse("acme/api").unwrap(),
            repository: None,
            pull_requests: None,
            errors: vec!["Not requested after rate-limit refusal".into()],
        };
        assert!(!row.complete());
        let text = render(&[row]);
        assert!(text.contains("were not loaded"));
        assert!(!text.contains("No open pull requests"));
    }
}
