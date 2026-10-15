//! Contextual labels and their destinations, without opening links or polling tasks.

use super::*;
use crate::model::fixtures;
use crate::tests::sample;
use radar_core::ratatui::crossterm::event::KeyModifiers;

/// The page's menu rows as bindings.
fn keys(p: &GitlabPlugin, pane: Pane) -> Vec<Binding> {
    crate::bindings_of(super::keys(p, pane))
}

/// The tree's menu rows as bindings.
fn screen_keys(p: &GitlabPlugin, pane: Pane) -> Vec<Binding> {
    crate::bindings_of(screen::keys(p, pane))
}

const REPO: &str = "acme/team-a/web";
const PIPELINE: &str = "https://gitlab.com/acme/team-a/web/-/pipelines/8001";

fn with_default_branch(branch: &str) -> GitlabPlugin {
    let mut p = sample();
    let project = p
        .data
        .get_mut("acme/team-a")
        .unwrap()
        .projects
        .iter_mut()
        .find(|project| project.full_path == REPO)
        .unwrap();
    project.default_branch = branch.into();
    p.tree
        .borrow_mut()
        .select(vec!["t:team-a".into(), format!("r:{REPO}")]);
    p
}

fn pipeline_label(bindings: &[Binding]) -> Option<&str> {
    bindings
        .iter()
        .find(|binding| binding.keys == "p")
        .map(|binding| binding.what.as_str())
}

fn assert_opens(outcome: Outcome, expected: &str) {
    assert!(
        matches!(outcome, Outcome::Handled(ref effects)
        if matches!(effects.as_slice(), [Effect::Open(url)] if url == expected)),
        "unexpected pipeline action: {outcome:?}"
    );
}

#[test]
fn contextual_repo_pipeline_actions_name_the_known_default_branch() {
    for (branch, expected) in [
        ("develop", "open develop's latest pipeline"),
        ("main", "open main's latest pipeline"),
        ("", "open the default branch's latest pipeline"),
    ] {
        let mut p = with_default_branch(branch);
        for pane in [Pane::Middle, Pane::Detail] {
            assert_eq!(pipeline_label(&screen_keys(&p, pane)), Some(expected));
            assert_opens(
                screen::key(
                    &mut p,
                    pane,
                    KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
                ),
                PIPELINE,
            );
        }
        p.pages.push(Page::new(Kind::Repo(REPO.into())));
        assert_eq!(pipeline_label(&keys(&p, Pane::Middle)), Some(expected));
        assert_opens(
            key(
                &mut p,
                Pane::Middle,
                KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
            ),
            PIPELINE,
        );
    }
}

#[test]
fn contextual_repo_pipeline_tab_names_and_opens_the_selected_run() {
    let mut p = with_default_branch("develop");
    p.pages.push(Page::new(Kind::Repo(REPO.into())));
    p.pages[0].tab = repo::PIPELINES;
    let mut runs = fixtures::runs();
    for run in &mut runs {
        run.url = run.url.as_ref().map(|url| url.replace("/api/", "/web/"));
    }
    crate::runs_ownership::begin(&mut p, REPO, "develop");
    on_runs(&mut p, REPO.into(), Ok(runs));
    let expected = repo::run(&p, REPO, 0).unwrap().url.clone().unwrap();
    assert_ne!(
        expected, PIPELINE,
        "selected run differs from the default-branch link"
    );
    assert_eq!(
        pipeline_label(&keys(&p, Pane::Middle)),
        Some("open this pipeline in GitLab")
    );
    assert_opens(
        key(
            &mut p,
            Pane::Middle,
            KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
        ),
        &expected,
    );
}

#[test]
fn contextual_repo_pipeline_actions_are_absent_without_a_pipeline_link() {
    let mut p = with_default_branch("develop");
    p.data
        .get_mut("acme/team-a")
        .unwrap()
        .projects
        .iter_mut()
        .find(|project| project.full_path == REPO)
        .unwrap()
        .main_pipeline_url = None;
    for pane in [Pane::Middle, Pane::Detail] {
        assert_eq!(pipeline_label(&screen_keys(&p, pane)), None);
    }
    p.pages.push(Page::new(Kind::Repo(REPO.into())));
    assert_eq!(pipeline_label(&keys(&p, Pane::Middle)), None);
}
