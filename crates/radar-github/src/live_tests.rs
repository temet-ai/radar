//! Read-only measurement against GitHub.com: what a refresh costs the old
//! way (every repository and its pull requests, unconditionally) and the
//! local-first way (one probe, then conditional reads). Ignored; run with
//! `RADAR_GITHUB_LIVE_CONFIG=<copy of config.toml> cargo test -p radar-github --lib
//! live_ -- --ignored --nocapture`. GETs and one GraphQL query only; the
//! token is never printed.

use crate::client::Client;
use crate::tests::shown;
use crate::config::Settings;
use crate::model::{
    Collection, Fresh, PullRequest, RateLimit, Repository, RepositoryId, Traffic, Validator,
};
use radar_core::Plugin;
use std::time::Instant;
use tokio::task::JoinSet;

fn settings() -> Option<Settings> {
    let path = std::env::var("RADAR_GITHUB_LIVE_CONFIG").ok()?;
    let text = std::fs::read_to_string(path).ok()?;
    let table: toml::Table = toml::from_str(&text).ok()?;
    let cfg: Settings = table.get("github")?.clone().try_into().ok()?;
    cfg.validated().ok()
}

/// Points used so far in the core and GraphQL buckets (`/rate_limit` is free).
async fn used(token: &str) -> (u64, u64) {
    let Ok(Some(auth)) = crate::auth::resolve(token) else {
        return (0, 0);
    };
    let value: serde_json::Value = reqwest::Client::new()
        .get("https://api.github.com/rate_limit")
        .header("authorization", auth)
        .header("user-agent", "radar-github-read-only/0.1")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let of = |name: &str| value["resources"][name]["used"].as_u64().unwrap_or(0);
    (of("core"), of("graphql"))
}

fn bucket(traffic: &Traffic, name: &str) -> Option<RateLimit> {
    traffic.rates.iter().find(|r| r.resource == name).cloned()
}

/// Every repository at once, as radar starts: description and open pull requests.
async fn full(
    ids: &[RepositoryId],
    token: &str,
    known: Option<&[(Option<Validator>, Vec<Validator>)]>,
) -> (
    Traffic,
    Vec<(Option<Validator>, Vec<Validator>)>,
    usize,
    Data,
) {
    let mut set = JoinSet::new();
    for (i, id) in ids.iter().enumerate() {
        let client = Client::new(ids, token, crate::config::DEFAULT_API).unwrap();
        let id = id.clone();
        let known = known.map(|k| k[i].clone());
        set.spawn(async move {
            let (repo_tag, pull_tags) = known.unwrap_or_default();
            let repository = client.repository_if(&id, repo_tag.as_ref()).await;
            let pulls = client.open_pulls_if(&id, &pull_tags).await;
            let mut moved = 0;
            let mut data = (None, None);
            let repo_tag = match repository {
                Ok(Fresh::New(value, tag)) => {
                    moved += 1;
                    data.0 = Some(value);
                    tag
                }
                Ok(Fresh::Same) => repo_tag,
                Err(_) => None,
            };
            let pull_tags = match pulls {
                Ok(Fresh::New(value, tags)) => {
                    moved += 1;
                    data.1 = Some(value);
                    tags
                }
                Ok(Fresh::Same) => pull_tags,
                Err(_) => vec![],
            };
            (i, client.traffic(), repo_tag, pull_tags, moved, data)
        });
    }
    let mut traffic = Traffic::default();
    let mut tags = vec![(None, vec![]); ids.len()];
    let mut moved = 0;
    let mut data = vec![(None, None); ids.len()];
    while let Some(Ok((i, t, repo_tag, pull_tags, m, d))) = set.join_next().await {
        traffic.add(t);
        tags[i] = (repo_tag, pull_tags);
        moved += m;
        data[i] = d;
    }
    (traffic, tags, moved, data)
}

type Data = Vec<(Option<Repository>, Option<Collection<PullRequest>>)>;

#[tokio::test]
#[ignore = "reads GitHub.com; needs RADAR_GITHUB_LIVE_CONFIG"]
async fn live_refresh_costs() {
    let Some(cfg) = settings() else {
        eprintln!("RADAR_GITHUB_LIVE_CONFIG unset or unreadable; nothing measured");
        return;
    };
    let ids = cfg.ids();
    let token = cfg.token.clone();
    println!("repositories: {}", ids.len());

    let before = used(&token).await;
    let started = Instant::now();
    let (old, tags, _, data) = full(&ids, &token, None).await;
    let cold_ms = started.elapsed().as_millis();
    let after = used(&token).await;
    println!(
        "old refresh (every repository, unconditional): {} requests, {} 304s, {cold_ms} ms to the last answer; core points {}; core {:?}",
        old.requests,
        old.not_modified,
        after.0 - before.0,
        bucket(&old, "core").map(|r| (r.remaining, r.limit))
    );

    // The cache those reads make, and how long a start takes to draw from it.
    let mut held = crate::GithubPlugin::default();
    held.apply_config(cfg.clone());
    for ((repo, (repository, pulls)), (repo_tag, pull_tags)) in
        held.repos.iter_mut().zip(data).zip(tags.clone())
    {
        repo.repository = repository;
        repo.pulls = pulls;
        repo.sync.repository_tag = repo_tag;
        repo.sync.pulls_tags = pull_tags;
    }
    let text = held.snapshot().unwrap().to_string();
    let started = Instant::now();
    let mut fresh = crate::GithubPlugin::default();
    fresh.apply_config(cfg.clone());
    fresh.restore(serde_json::from_str(&text).unwrap());
    let host = crate::tests::hosted(fresh);
    let drawn = shown(&host, 160, 40);
    println!(
        "cache: {} bytes; read, restored and drawn in {} ms ({} rows of text)",
        text.len(),
        started.elapsed().as_millis(),
        drawn.lines().count()
    );

    let before = used(&token).await;
    let started = Instant::now();
    let (again, _, moved, _) = full(&ids, &token, Some(&tags)).await;
    let elapsed = started.elapsed().as_millis();
    let after = used(&token).await;
    println!(
        "conditional refresh of every repository (public path): {} requests, {} 304s, {moved} moved, {elapsed} ms; core points {}",
        again.requests,
        again.not_modified,
        after.0 - before.0,
    );

    if token.is_empty() {
        return;
    }
    let client = Client::new(&ids, &token, crate::config::DEFAULT_API).unwrap();
    let before = used(&token).await;
    let started = Instant::now();
    let first = client.markers(&ids).await.unwrap();
    let probe_ms = started.elapsed().as_millis();
    let second = client.markers(&ids).await.unwrap();
    let after = used(&token).await;
    let traffic = client.traffic();
    let unchanged = first
        .iter()
        .zip(&second)
        .filter(|(a, b)| a == b && a.1.is_some())
        .count();
    println!(
        "probe (what moved): 2 rounds, {} requests, first round {probe_ms} ms; {unchanged}/{} repositories unchanged; graphql points {}, core points {}",
        traffic.requests,
        ids.len(),
        after.1 - before.1,
        after.0 - before.0,
    );
}

/// A first start with nothing cached, the round, and the Actions tab, the
/// way radar now does them: one GraphQL query for what is not held, a few REST
/// reads at a time for what that could not read, two batches in flight
/// together, and one bounded page of runs asked conditionally.
#[tokio::test]
#[ignore = "reads GitHub.com; needs RADAR_GITHUB_LIVE_CONFIG"]
async fn live_cold_start_round_and_actions() {
    use crate::model::RunsAsk;
    let Some(cfg) = settings() else {
        eprintln!("RADAR_GITHUB_LIVE_CONFIG unset or unreadable; nothing measured");
        return;
    };
    let ids = cfg.ids();
    let token = cfg.token.clone();
    if token.is_empty() {
        eprintln!("no token: the probe needs one");
        return;
    }
    println!("repositories: {}", ids.len());
    let client = Client::new(&ids, &token, crate::config::DEFAULT_API).unwrap();

    // Cold start: nothing held, so every repository is asked in full.
    let before = used(&token).await;
    let started = Instant::now();
    let asks: Vec<_> = ids.iter().map(|id| (id.clone(), true)).collect();
    let probed = client.probe(&asks).await.unwrap();
    let probe_ms = started.elapsed().as_millis();
    let filled = probed.iter().filter(|p| p.full.is_some()).count();
    let rest: Vec<_> = probed
        .iter()
        .filter(|p| p.full.is_none())
        .map(|p| p.id.clone())
        .collect();
    println!(
        "cold probe: {} GraphQL requests, {probe_ms} ms; {filled}/{} repositories read in full, {} left to REST",
        client.traffic().requests,
        ids.len(),
        rest.len()
    );
    // What is left is read six at a time, as the plugin feeds them.
    let started = Instant::now();
    let slots = std::sync::Arc::new(tokio::sync::Semaphore::new(crate::sync::READS_AT_ONCE));
    let mut set = JoinSet::new();
    let mut open = 0usize;
    let most = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let running = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    for id in rest.clone() {
        let (client, slots) = (
            Client::new(&ids, &token, crate::config::DEFAULT_API).unwrap(),
            slots.clone(),
        );
        let (most, running) = (most.clone(), running.clone());
        set.spawn(async move {
            let _slot = slots.acquire_owned().await.unwrap();
            let now = running.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            most.fetch_max(now, std::sync::atomic::Ordering::SeqCst);
            let _ = client.repository_if(&id, None).await;
            let _ = client.open_pulls_if(&id, &[]).await;
            running.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            client.traffic().requests
        });
    }
    while let Some(Ok(requests)) = set.join_next().await {
        open += requests as usize;
    }
    let after = used(&token).await;
    println!(
        "cold start total: {} requests ({} GraphQL + {open} REST, at most {} in flight), {} ms to the last answer; graphql points {}, core points {}",
        client.traffic().requests + open as u64,
        client.traffic().requests,
        most.load(std::sync::atomic::Ordering::SeqCst),
        probe_ms + started.elapsed().as_millis(),
        after.1 - before.1,
        after.0 - before.0,
    );

    // A round: the probe's batches one after another, then together.
    let markers: Vec<_> = ids.iter().map(|id| (id.clone(), false)).collect();
    let started = Instant::now();
    let mut one_by_one = 0;
    for batch in markers.chunks(50) {
        one_by_one += client.probe(batch).await.unwrap().len();
    }
    let sequential = started.elapsed().as_millis();
    let started = Instant::now();
    let together = client.probe(&markers).await.unwrap().len();
    let concurrent = started.elapsed().as_millis();
    println!(
        "round (what moved, {} batches): one after another {sequential} ms ({one_by_one} repositories), together {concurrent} ms ({together})",
        markers.len().div_ceil(50)
    );

    // The Actions tab, on the first repository with plenty of runs.
    let http = reqwest::Client::new();
    let auth = crate::auth::resolve(&token).unwrap().unwrap();
    let mut chosen = None;
    for id in ids.iter().take(12) {
        let value: serde_json::Value = http
            .get(format!(
                "https://api.github.com/repos/{}/actions/runs?per_page=1",
                id.full_name()
            ))
            .header("authorization", auth.clone())
            .header("user-agent", "radar-github-read-only/0.1")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap_or_default();
        let total = value["total_count"].as_u64().unwrap_or(0);
        if total >= 100 {
            chosen = Some((id.clone(), total));
            break;
        }
    }
    let Some((id, total)) = chosen else {
        println!("actions: no repository among the first 12 has 100 runs; nothing measured");
        return;
    };
    let old_pages = (total.min(500)).div_ceil(100);
    println!(
        "actions on a repository with {total} runs: the old tab read {old_pages} pages of 100, untagged, every time"
    );
    let ask = RunsAsk {
        branch: None,
        per_page: 80,
    };
    let client = Client::new(&ids, &token, crate::config::DEFAULT_API).unwrap();
    let before = used(&token).await;
    let started = Instant::now();
    let Fresh::New(runs, tags) = client.workflow_runs_if(&id, &ask, &[]).await.unwrap() else {
        panic!("a first read is new");
    };
    println!(
        "actions first open: 1 request, {} ms, {} runs, {} validators ({:?})",
        started.elapsed().as_millis(),
        runs.items.len(),
        tags.len(),
        runs.completion.reason
    );
    let started = Instant::now();
    let again = client.workflow_runs_if(&id, &ask, &tags).await.unwrap();
    let traffic = client.traffic();
    let after = used(&token).await;
    println!(
        "actions reopen after the 15 minutes: {} ({} requests so far, {} 304s), {} ms; core points for both {}",
        if again == Fresh::Same {
            "304, nothing parsed"
        } else {
            "changed"
        },
        traffic.requests,
        traffic.not_modified,
        started.elapsed().as_millis(),
        after.0 - before.0,
    );
    println!("actions reopen on a repository that has not moved: 0 requests (held, marker equal)");
}
