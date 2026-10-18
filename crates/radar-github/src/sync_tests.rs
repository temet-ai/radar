//! Local-first sync: the cache is the reference, a round asks only what
//! moved, and GitHub's limits are waited out.

use super::*;
use crate::model::{ApiErrorKind, Completion, RateLimit};
use crate::test_support::RepositoryBuilder;
use radar_core::shell::to_text;
use serde_json::json;

const NOW: i64 = radar_core::plugin::DEMO_NOW;

fn spawns(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|e| matches!(e, Effect::SpawnAction { .. }))
        .count()
}
fn persists(effects: &[Effect]) -> bool {
    effects.iter().any(|e| matches!(e, Effect::PersistCache))
}
fn notifies(effects: &[Effect]) -> bool {
    effects
        .iter()
        .any(|e| matches!(e, Effect::Notify(_) | Effect::NotifyWith(..) | Effect::Notice(_)))
}

fn marker(open: u64, at: &str) -> Marker {
    Marker {
        open,
        latest: Some(at.into()),
    }
}

/// The demo plugin as a cache leaves it: every repository read completely,
/// with the marker it matched and a description read just now.
fn synced() -> GithubPlugin {
    let mut p = demo::sample();
    for (i, repo) in p.repos.iter_mut().enumerate() {
        repo.sync.marker = Some(marker(i as u64, "2026-10-06T10:00:00Z"));
        repo.sync.repository_at = Some(NOW - 60);
        repo.sync.pulls_tags = vec![Validator {
            etag: Some(format!("W/\"{i}\"")),
            last_modified: None,
        }];
    }
    p.sync.said_complete = Some(true);
    p.sync.last_started = Some(NOW - 60);
    p
}

/// A round started at `now`, its probe in flight.
fn probing(p: &mut GithubPlugin, now: i64) {
    p.now = now;
    p.sync.last_started = None;
    p.started = false;
    let effects = p.tick(now);
    assert_eq!(spawns(&effects), 1, "one probe for every repository");
    assert!(p.sync.probing);
}

fn probe(p: &GithubPlugin, result: Result<Vec<model::Probed>, ApiError>) -> Answer {
    Answer::Probe {
        generation: p.generation,
        request: p.sync.request,
        action: history::probe_action(),
        result,
        traffic: Traffic::default(),
    }
}

fn markers(p: &GithubPlugin) -> Vec<model::Probed> {
    p.repos
        .iter()
        .map(|r| model::Probed {
            id: r.id.clone(),
            marker: r.sync.marker.clone(),
            full: None,
        })
        .collect()
}

fn same_pulls(p: &GithubPlugin, index: usize, marker: Option<Marker>) -> Answer {
    let repo = &p.repos[index];
    Answer::Repository {
        generation: p.generation,
        request: repo.request,
        id: repo.id.clone(),
        action: history::action(
            "github.repository.read",
            &Target::Repository(repo.id.clone()),
        ),
        repository: None,
        pulls: Ok(Fresh::Same),
        reply: sync::Reply {
            marker,
            ..Default::default()
        },
    }
}

fn limited(retry_after: Option<u64>, reset: Option<u64>) -> ApiError {
    ApiError {
        kind: ApiErrorKind::RateLimit,
        status: Some(403),
        retry_after_secs: retry_after,
        rate_limit_reset: reset,
    }
}

#[test]
fn an_unchanged_repository_costs_no_detail_read() {
    let mut p = synced();
    probing(&mut p, NOW);
    let answer = probe(&p, Ok(markers(&p)));
    let effects = p.receive(answer);
    assert_eq!(spawns(&effects), 0, "nothing moved, nothing read");
    assert!(p.repos.iter().all(|r| !r.loading));
    assert_eq!(
        p.last_sync,
        Some(NOW),
        "the probe proved the held data current"
    );
    assert!(!persists(&effects), "nothing new to write");
    assert!(!notifies(&effects), "a quiet background round says nothing");
}

#[test]
fn only_the_repository_that_moved_is_read() {
    let mut p = synced();
    probing(&mut p, NOW);
    let mut moved = markers(&p);
    moved[1].marker = Some(marker(1, "2026-10-13T09:00:00Z"));
    let answer = probe(&p, Ok(moved));
    let effects = p.receive(answer);
    assert_eq!(spawns(&effects), 1);
    assert!(!p.repos[0].loading);
    assert!(p.repos[1].loading);
    // A background read keeps drawing what is held.
    let text = to_text(&tests_host(&p), 160, 40);
    assert!(!text.contains("loading"), "{text}");
}

fn tests_host(p: &GithubPlugin) -> radar_core::Host {
    let mut host = radar_core::Host::new(vec![Box::new(GithubPlugin::default())], NOW).unwrap();
    let mut table = toml::Table::new();
    table.insert("github".into(), toml::Value::try_from(&p.cfg).unwrap());
    host.load_config(Some(&toml::to_string(&table).unwrap()))
        .unwrap();
    let mut copy = demo::sample();
    copy.repos.iter_mut().zip(&p.repos).for_each(|(c, r)| {
        c.loading = r.loading;
        c.pulls = r.pulls.clone();
    });
    host.plugins[0] = Box::new(copy);
    host
}

#[test]
fn a_304_keeps_the_rows_held_and_takes_the_new_marker() {
    let mut p = synced();
    let held = p.repos[0].pulls.clone();
    let tags = p.repos[0].sync.pulls_tags.clone();
    let now_marker = marker(2, "2026-10-13T09:00:00Z");
    assert_eq!(
        spawns(&p.refresh_repository(0, Some(now_marker.clone()), false)),
        1
    );
    let answer = same_pulls(&p, 0, Some(now_marker.clone()));
    let effects = p.receive(answer);
    assert_eq!(p.repos[0].pulls, held, "a 304 changes nothing held");
    assert_eq!(p.repos[0].sync.pulls_tags, tags);
    assert_eq!(p.repos[0].sync.marker, Some(now_marker));
    assert!(
        persists(&effects),
        "the new marker saves a read at the next start"
    );
}

#[test]
fn a_new_list_replaces_the_rows_and_their_validators() {
    let mut p = synced();
    p.refresh_repository(0, Some(marker(1, "2026-10-13T09:00:00Z")), false);
    let id = p.repos[0].id.clone();
    let tags = vec![Validator {
        etag: Some("W/\"new\"".into()),
        last_modified: None,
    }];
    let answer = Answer::Repository {
        generation: p.generation,
        request: p.repos[0].request,
        id: id.clone(),
        action: history::action("github.repository.read", &Target::Repository(id.clone())),
        repository: None,
        pulls: Ok(Fresh::New(
            Collection {
                items: vec![demo::pull(&id, 44, "A new one")],
                completion: Completion::complete(1),
            },
            tags.clone(),
        )),
        reply: sync::Reply {
            marker: Some(marker(1, "2026-10-13T09:00:00Z")),
            ..Default::default()
        },
    };
    let effects = p.receive(answer);
    assert_eq!(p.repos[0].pulls.as_ref().unwrap().items[0].number, 44);
    assert_eq!(p.repos[0].sync.pulls_tags, tags);
    assert!(persists(&effects));
}

#[test]
fn a_rate_limited_round_waits_until_the_reset() {
    let mut p = synced();
    probing(&mut p, NOW);
    let reset = (NOW + 600) as u64;
    let answer = probe(&p, Err(limited(None, Some(reset))));
    let effects = p.receive(answer);
    assert_eq!(spawns(&effects), 0, "nothing else is read past a refusal");
    assert_eq!(
        p.last_sync,
        demo::sample().last_sync,
        "not counted as synced"
    );
    for now in [NOW + 60, NOW + 300, NOW + 599] {
        assert_eq!(spawns(&p.tick(now)), 0, "still waiting at {now}");
    }
    assert!(
        p.top_right("github")
            .unwrap()
            .contains("GitHub asked radar to wait")
    );
    let asked = p.refresh_all();
    assert_eq!(spawns(&asked), 0, "r waits too");
    assert!(notifies(&asked));
    assert_eq!(spawns(&p.tick(NOW + 600)), 1, "reads resume at the reset");
}

#[test]
fn retry_after_wins_and_server_errors_back_off() {
    let mut sync = sync::Sync::default();
    sync.stop(&limited(Some(30), Some((NOW + 600) as u64)), NOW);
    assert_eq!(sync.wait_until(NOW), Some(NOW + 30));
    let mut sync = sync::Sync::default();
    sync.stop(&limited(None, None), NOW);
    assert_eq!(
        sync.wait_until(NOW),
        Some(NOW + 60),
        "a secondary limit waits a minute"
    );
    let mut sync = sync::Sync::default();
    let down = ApiError::new(ApiErrorKind::Unavailable);
    let waits: Vec<_> = (0..7)
        .map(|_| {
            sync.stop(&down, NOW);
            let wait = sync.wait_until(NOW).unwrap() - NOW;
            sync.blocked_until = None;
            wait
        })
        .collect();
    assert_eq!(waits, [60, 120, 240, 480, 900, 900, 900]);
    sync.take(Traffic::default(), None, NOW);
    assert_eq!(
        sync.failures, 0,
        "a read that went through resets the backoff"
    );
}

#[test]
fn a_bucket_running_low_waits_for_its_reset() {
    let mut p = synced();
    p.sync.traffic.note(RateLimit {
        resource: "core".into(),
        limit: 5000,
        remaining: 400,
        reset: NOW + 900,
    });
    assert!(!sync::due(&p, NOW + 120));
    assert_eq!(spawns(&p.tick(NOW + 120)), 0);
    assert!(sync::due(&p, NOW + 900));
}

#[test]
fn the_cache_is_drawn_at_start_and_only_what_moved_is_asked() {
    let p = synced();
    let cache = p.snapshot().unwrap();
    let mut fresh = GithubPlugin::default();
    fresh.apply_config(p.cfg.clone());
    fresh.restore(cache);
    fresh.now = NOW;
    assert_eq!(
        fresh.repos[0].pulls, p.repos[0].pulls,
        "drawn before any read"
    );
    assert_eq!(fresh.repos[0].sync, p.repos[0].sync);
    assert_eq!(fresh.last_sync, p.last_sync);
    let effects = fresh.tick(NOW);
    assert_eq!(
        spawns(&effects),
        1,
        "one probe, not a read of every repository"
    );
    let answer = probe(&fresh, Ok(markers(&fresh)));
    assert_eq!(spawns(&fresh.receive(answer)), 0);
}

#[test]
fn an_old_or_unreadable_cache_is_discarded_once() {
    for cache in [
        json!({"version": 0, "repos": [{"id": {"owner": "acme", "name": "api"}}]}),
        json!({"repos": "not a list"}),
        json!([1, 2, 3]),
    ] {
        let mut p = GithubPlugin::default();
        p.apply_config(demo::sample().cfg);
        p.restore(cache);
        assert!(
            p.repos
                .iter()
                .all(|r| r.pulls.is_none() && r.repository.is_none())
        );
        assert!(p.sync.dirty, "the next round writes a cache of this layout");
        assert!(persists(&p.finish_round()));
        assert!(!persists(&p.finish_round()), "written once");
        assert_eq!(p.snapshot().unwrap()["version"], sync::VERSION);
    }
}

#[test]
fn a_cache_never_fills_a_repository_with_another_ones_rows() {
    let mut cache = synced().snapshot().unwrap();
    cache["repos"][1]["pulls"] = cache["repos"][0]["pulls"].clone();
    cache["repos"].as_array_mut().unwrap().push(json!({
        "id": {"owner": "elsewhere", "name": "repo"},
        "pulls": {"items": [], "completion": {"complete": true, "reason": null, "pages": 1}}
    }));
    let mut p = GithubPlugin::default();
    p.apply_config(demo::sample().cfg);
    p.restore(cache);
    assert!(p.repos[0].pulls.is_some());
    assert!(
        p.repos[1].pulls.is_none(),
        "acme/api rows are not acme/docs rows"
    );
    assert_eq!(p.repos.len(), 2, "an unconfigured repository is not added");
}

#[test]
fn public_reads_ask_each_repository_conditionally() {
    let mut p = synced();
    p.cfg.token = String::new();
    p.started = false;
    let effects = p.tick(NOW);
    assert_eq!(spawns(&effects), 2, "no GraphQL without a token");
    assert!(!p.sync.probing);
}

#[test]
fn a_pull_page_reopens_from_what_was_seen_and_polls_only_running_checks() {
    let mut p = demo::page();
    let pull = p.pull_detail.clone().unwrap();
    let target = Target::Pull(pull.repository.clone(), pull.number);
    let answer = Answer::Pull {
        generation: p.generation,
        request: p.page_request,
        action: history::action("github.pull-request.read", &target),
        target: target.clone(),
        pull: Box::new(Ok(Fresh::New(pull.clone(), None))),
        checks: Ok(Fresh::New(demo::checks(&pull), CheckTags::new())),
        reply: sync::Reply::default(),
    };
    p.page_loading = true;
    assert!(persists(&p.receive(answer)), "a read page is kept");
    screen::key(
        &mut p,
        Pane::Middle,
        chord(radar_core::ratatui::crossterm::event::KeyCode::Esc),
    );
    p.open_target(target.clone());
    assert_eq!(
        p.pull_detail.as_ref(),
        Some(&pull),
        "drawn before the read answers"
    );
    assert!(p.checks.is_some());
    assert!(p.page_loading);
    // Checks still running: asked again a minute later, not before.
    p.page_loading = false;
    let opened = p.now;
    p.tick(opened + sync::EVERY);
    assert!(
        !p.page_loading,
        "no rounds run (the demo, a deferred save): not asked"
    );
    p.sync.last_started = Some(opened);
    p.tick(opened + 30);
    assert!(!p.page_loading);
    p.tick(opened + sync::EVERY);
    assert!(p.page_loading, "asked again");
    // Every check finished: the page is not asked again.
    p.page_loading = false;
    for run in &mut p.checks.as_mut().unwrap().check_runs.items {
        run.status = "completed".into();
        run.conclusion = Some("success".into());
    }
    p.tick(opened + 10 * sync::EVERY);
    assert!(!p.page_loading);
}

#[test]
fn a_304_page_keeps_what_is_drawn() {
    let mut p = demo::page();
    let pull = p.pull_detail.clone().unwrap();
    let checks = p.checks.clone();
    let target = Target::Pull(pull.repository.clone(), pull.number);
    p.links.borrow_mut().push(radar_core::ui::rich::Link {
        text: "kept".into(),
        url: "https://example.com/kept".into(),
    });
    let answer = Answer::Pull {
        generation: p.generation,
        request: p.page_request,
        action: history::action("github.pull-request.read", &target),
        target,
        pull: Box::new(Ok(Fresh::Same)),
        checks: Ok(Fresh::Same),
        reply: sync::Reply::default(),
    };
    let effects = p.receive(answer);
    assert_eq!(p.pull_detail, Some(pull));
    assert_eq!(p.checks, checks);
    assert_eq!(
        p.links.borrow().len(),
        1,
        "nothing changed, nothing re-made"
    );
    assert!(!persists(&effects));
}

/// A first start: `count` repositories configured, nothing cached, nothing read.
fn cold(count: usize) -> GithubPlugin {
    let mut p = demo::sample();
    p.repos = (0..count)
        .map(|i| RepoState::new(RepositoryId::parse(&format!("acme/r{i}")).unwrap()))
        .collect();
    p.started = false;
    p.last_sync = None;
    p.sync = sync::Sync::default();
    p.seen.clear();
    p
}

fn loading(p: &GithubPlugin) -> usize {
    p.repos.iter().filter(|r| r.loading).count()
}

/// The answer of the repository read in flight at `index`: its data, new.
fn read_of(p: &GithubPlugin, index: usize, reply: sync::Reply) -> Answer {
    let repo = &p.repos[index];
    Answer::Repository {
        generation: p.generation,
        request: repo.request,
        id: repo.id.clone(),
        action: history::action(
            "github.repository.read",
            &Target::Repository(repo.id.clone()),
        ),
        repository: Some(Ok(Fresh::New(
            RepositoryBuilder::default().id(repo.id.clone()).build(),
            None,
        ))),
        pulls: Ok(Fresh::New(
            Collection {
                items: vec![],
                completion: Completion::complete(1),
            },
            vec![],
        )),
        reply,
    }
}

#[test]
fn a_first_start_feeds_its_reads_through_a_cap() {
    let mut p = cold(20);
    p.cfg.token = String::new();
    let effects = p.tick(NOW);
    assert_eq!(
        spawns(&effects),
        sync::READS_AT_ONCE,
        "not 20 reads at once, and not one after another"
    );
    let (mut started, mut most) = (spawns(&effects), loading(&p));
    while let Some(index) = p.repos.iter().position(|r| r.loading) {
        let answer = read_of(&p, index, sync::Reply::default());
        let effects = p.receive(answer);
        started += spawns(&effects);
        most = most.max(loading(&p));
    }
    assert_eq!((started, most), (20, sync::READS_AT_ONCE));
    assert!(p.sync.queue.is_empty());
    assert!(
        p.repos
            .iter()
            .all(|r| r.repository.is_some() && r.pulls.is_some()),
        "every repository was read"
    );
    assert_eq!(
        p.last_sync,
        Some(NOW),
        "the round counted as synced once the last answered"
    );
}

#[test]
fn a_repository_with_nothing_held_is_asked_for_in_the_probe_and_the_rest_are_not() {
    let mut p = synced();
    p.repos[1].pulls = None;
    let asks = sync::asks(&p);
    assert_eq!(
        asks.iter().map(|(_, full)| *full).collect::<Vec<_>>(),
        [false, true]
    );
    assert!(sync::asks(&cold(3)).iter().all(|(_, full)| *full));
    assert!(sync::asks(&synced()).iter().all(|(_, full)| !*full));
}

fn filled(p: &GithubPlugin, index: usize) -> model::Probed {
    let id = p.repos[index].id.clone();
    model::Probed {
        marker: Some(marker(0, "2026-10-13T09:00:00Z")),
        full: Some(model::Full {
            repository: RepositoryBuilder::default()
                .id(id.clone())
                .description("read in the probe")
                .build(),
            pulls: Collection {
                items: vec![demo::pull(&id, 5, "Read in the probe")],
                completion: Completion::complete(1),
            },
        }),
        id,
    }
}

#[test]
fn a_probe_that_read_the_repositories_in_full_leaves_no_rest_read_to_make() {
    let mut p = cold(76);
    p.cfg.token = "github_pat_synthetic".into();
    probing(&mut p, NOW);
    let all: Vec<_> = (0..76).map(|i| filled(&p, i)).collect();
    let answer = probe(&p, Ok(all));
    let effects = p.receive(answer);
    assert_eq!(spawns(&effects), 0, "152 requests became none");
    assert!(p.repos.iter().all(|r| {
        r.repository
            .as_ref()
            .is_some_and(|d| d.description.as_deref() == Some("read in the probe"))
            && r.pulls
                .as_ref()
                .is_some_and(|l| l.items[0].number == 5 && l.completion.complete)
            && r.sync.marker.is_some()
            && r.sync.repository_at == Some(NOW)
    }));
    assert_eq!(p.last_sync, Some(NOW));
    assert!(persists(&effects), "written for the next start");
    // The next round has a marker to compare, so asks nothing more.
    probing(&mut p, NOW + 60);
    let again: Vec<_> = p
        .repos
        .iter()
        .map(|r| model::Probed {
            id: r.id.clone(),
            marker: r.sync.marker.clone(),
            full: None,
        })
        .collect();
    let answer = probe(&p, Ok(again));
    assert_eq!(spawns(&p.receive(answer)), 0);
}

#[test]
fn what_the_probe_could_not_read_in_full_is_read_over_rest_a_few_at_a_time() {
    let mut p = cold(20);
    p.cfg.token = "github_pat_synthetic".into();
    probing(&mut p, NOW);
    let mut answered: Vec<_> = (0..20).map(|i| filled(&p, i)).collect();
    // Six had more pull requests than a full read takes.
    for probed in &mut answered[..6] {
        probed.full = None;
    }
    let answer = probe(&p, Ok(answered));
    let effects = p.receive(answer);
    assert_eq!(spawns(&effects), 6);
    assert!(p.repos[..6].iter().all(|r| r.loading));
    assert!(p.repos[6..].iter().all(|r| r.pulls.is_some()));
    // One probe answer for 20 whose full read failed: 20 queued, 6 at once.
    let mut p = cold(20);
    p.cfg.token = "github_pat_synthetic".into();
    probing(&mut p, NOW);
    let none: Vec<_> = (0..20)
        .map(|i| model::Probed {
            full: None,
            ..filled(&p, i)
        })
        .collect();
    let answer = probe(&p, Ok(none));
    assert_eq!(spawns(&p.receive(answer)), sync::READS_AT_ONCE);
    assert_eq!(p.sync.queue.len(), 14);
}

#[test]
fn a_refusal_mid_queue_stops_the_rest_and_waits() {
    let mut p = cold(20);
    p.cfg.token = String::new();
    assert_eq!(spawns(&p.tick(NOW)), sync::READS_AT_ONCE);
    let reply = sync::Reply {
        stop: Some(limited(Some(120), None)),
        ..Default::default()
    };
    let first = p.repos.iter().position(|r| r.loading).unwrap();
    let answer = read_of(&p, first, reply);
    assert_eq!(
        spawns(&p.receive(answer)),
        0,
        "nothing more is started into a limit"
    );
    assert!(p.sync.queue.is_empty());
    while let Some(index) = p.repos.iter().position(|r| r.loading) {
        let answer = read_of(&p, index, sync::Reply::default());
        assert_eq!(spawns(&p.receive(answer)), 0);
    }
    assert_eq!(
        p.repos.iter().filter(|r| r.pulls.is_some()).count(),
        sync::READS_AT_ONCE,
        "only what was already in flight finished"
    );
    assert_eq!(p.last_sync, None, "not counted as synced");
    assert_eq!(spawns(&p.tick(NOW + 60)), 0, "still waiting");
    assert!(spawns(&p.tick(NOW + 120)) > 0, "reads resume when asked to");
}

#[test]
fn a_saved_configuration_forgets_reads_still_queued() {
    let mut p = cold(20);
    p.cfg.token = String::new();
    p.tick(NOW);
    assert!(!p.sync.queue.is_empty());
    p.apply_config(p.cfg.clone());
    assert!(
        p.sync.queue.is_empty(),
        "indexes of a list that was rebuilt"
    );
}
