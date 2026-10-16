use super::*;
use crate::client::{Synced, Want};
use radar_core::Host;
use radar_core::host::{Event, HostEffect, JournalRecord, View};
use radar_core::plugin::{Msg, Pane, ScreenSpec, SectionSpec};
use std::sync::{Arc, Mutex};

struct Harness(Arc<Mutex<GitlabPlugin>>);
impl Plugin for Harness {
    fn id(&self) -> &'static str {
        "gitlab"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        self.0.lock().unwrap().config()
    }
    fn screens(&self) -> Vec<ScreenSpec> {
        self.0.lock().unwrap().screens()
    }
    fn sections(&self) -> Vec<SectionSpec> {
        self.0.lock().unwrap().sections()
    }
    fn section_view(&self, section: &str, cx: &radar_core::view::ViewCx<'_>) -> Option<radar_core::view::SectionView> {
        self.0.lock().unwrap().section_view(section, cx)
    }
    fn intent(&mut self, scope: &str, pane: Pane, intent: &radar_core::view::Intent) -> Outcome {
        self.0.lock().unwrap().intent(scope, pane, intent)
    }
    fn on_menu(&mut self, menu: &str, value: &str) -> Vec<Effect> {
        self.0.lock().unwrap().on_menu(menu, value)
    }
    fn on_msg(&mut self, msg: Msg) -> Vec<Effect> {
        self.0.lock().unwrap().on_msg(msg)
    }
}
fn hosted(p: GitlabPlugin, section: &str) -> (Host, Arc<Mutex<GitlabPlugin>>) {
    let shared = Arc::new(Mutex::new(p));
    let mut host = Host::new(vec![Box::new(Harness(shared.clone()))], 1).unwrap();
    host.enable_journal();
    host.view = View::Settings;
    host.focus = Pane::Middle;
    host.section = host
        .sections
        .iter()
        .position(|(_, s)| s.id == section && s.group == "gitlab")
        .unwrap();
    (host, shared)
}
fn key(host: &mut Host, code: KeyCode) -> Vec<HostEffect> {
    host.update(Event::Key(KeyEvent::from(code)))
}
fn loaded(host: &mut Host, group: &str) {
    let want = Want {
        projects: true,
        ..Default::default()
    };
    host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(GlMsg::Loaded {
            group: group.into(),
            action: crate::sync_history::action("https://git.example", group, &want),
            want,
            result: Ok(Synced {
                me: "me".into(),
                projects: Some(vec![]),
                ..Default::default()
            }),
            net: Default::default(),
        }),
    });
}
fn checks(effects: &[HostEffect]) -> Vec<(String, String)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            HostEffect::Journal(JournalRecord::Begin { descriptor, .. })
                if [
                    "gitlab.settings.check.auth",
                    "gitlab.settings.check.base",
                    "gitlab.settings.check.source",
                ]
                .contains(&descriptor.kind.as_str()) =>
            {
                Some((
                    descriptor.kind.as_str().into(),
                    descriptor.subjects[0].key.id.clone(),
                ))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn x_menu_base_check_keeps_surviving_field_when_loaded_removes_token_offer() {
    let mut p = demo::sample();
    p.me.clear();
    p.cfg.token = "glab".into();
    p.env = Some(vec![(
        "GITLAB_TOKEN".into(),
        "fake-token-long-enough-for-offer".into(),
    )]);
    assert_eq!(p.offers().len(), 1);
    p.section_rows[0] = 2; // Offer, Url, Base, Token.
    assert_eq!(
        settings::conn_rows(&p)[2],
        settings::ConnRow::Field(settings::Field::Base)
    );
    let (mut host, shared) = hosted(p, "connection");
    key(&mut host, KeyCode::Char('x'));
    assert!(
        host.menu
            .as_ref()
            .unwrap()
            .menu
            .items
            .iter()
            .any(|i| i.label == "check the base group")
    );
    loaded(&mut host, "acme/team-a");
    let fx = key(&mut host, KeyCode::Char('c'));
    assert_eq!(
        checks(&fx),
        [("gitlab.settings.check.base".into(), "acme".into())]
    );
    let p = shared.lock().unwrap();
    assert!(p.offers().is_empty());
    assert_eq!(
        settings::conn_rows(&p)[p.section_rows[0]],
        settings::ConnRow::Field(settings::Field::Base)
    );
    assert_eq!(p.check_owners.pending.len(), 1);
    assert!(p.check_owners.pending.contains_key(&Target::Base));
    drop(fx); // The spawned provider task is never run.
}

#[test]
fn x_menu_source_check_keeps_surviving_tenancy_when_loaded_removes_prior_repos() {
    let mut p = demo::sample();
    p.cfg.sources = ["a", "b", "c"]
        .map(|group| crate::config::Source {
            group: group.into(),
            code: group.into(),
            color: "cyan".into(),
            hide: vec![],
        })
        .into();
    let project = p.data["acme/team-a"].projects[0].clone();
    p.data.clear();
    p.data.insert(
        "acme/a".into(),
        crate::GroupData {
            projects: vec![crate::model::Project {
                full_path: "acme/a/repo".into(),
                ..project
            }],
            ..Default::default()
        },
    );
    p.section_rows[1] = 3; // Add, A, A/repo, B, C.
    assert_eq!(
        settings::source_rows(&p)[3],
        settings::SourceRow::Tenancy(1)
    );
    let (mut host, shared) = hosted(p, "tenancies");
    key(&mut host, KeyCode::Char('x'));
    assert!(
        host.menu
            .as_ref()
            .unwrap()
            .menu
            .items
            .iter()
            .any(|i| i.label == "check it")
    );
    loaded(&mut host, "acme/a");
    let fx = key(&mut host, KeyCode::Char('c'));
    assert_eq!(
        checks(&fx),
        [("gitlab.settings.check.source".into(), "acme/b".into())]
    );
    let p = shared.lock().unwrap();
    assert_eq!(p.section_rows[1], 2);
    assert_eq!(
        settings::source_rows(&p)[p.section_rows[1]],
        settings::SourceRow::Tenancy(1)
    );
    assert!(
        p.check_owners
            .pending
            .contains_key(&Target::Source("acme/b".into()))
    );
    assert!(
        !p.check_owners
            .pending
            .contains_key(&Target::Source("acme/c".into()))
    );
    drop(fx);
}

#[test]
fn explicit_source_menu_check_captures_original_path_across_reorder() {
    let mut p = demo::sample();
    p.section_rows[1] = 1;
    let Outcome::Handled(fx) = settings::key(&mut p, "tenancies", KeyEvent::from(KeyCode::Enter))
    else {
        panic!()
    };
    let Effect::Menu(menu) = &fx[0] else { panic!() };
    let item = menu
        .items
        .iter()
        .find(|item| item.key == Some('c'))
        .unwrap();
    settings::on_menu(&mut p, "tenancy:0", "down");
    let fx = settings::on_menu(&mut p, &menu.id, &item.value);
    assert!(
        matches!(&fx[..], [Effect::SpawnAction { action, .. }] if action.subjects[0].key.id == "acme/team-b")
    );
    assert!(
        p.check_owners
            .pending
            .contains_key(&Target::Source("acme/team-b".into()))
    );
}
