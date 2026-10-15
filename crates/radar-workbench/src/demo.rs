//! Sample work for shots and tests: one local task with linked sources and a
//! group, made through the part's own menus and a scratch journal, so every
//! view of it is what a real save produces. Nothing here runs without the
//! `demo` feature (or tests); the scratch journal is deleted when it is done.
use super::*;
use radar_core::journal::{
    ActionDescriptor, Journal, Origin, SessionId, SubjectRole, SubjectSnapshot,
};
use radar_core::writes::Mode;
use std::collections::VecDeque;

pub(crate) const TITLE: &str = "Ship explicit work-item navigation";
pub(crate) const GROUP: &str = "release candidate";

fn source(kind: &str, id: &str) -> Subject {
    Subject::new(kind, id, format!("Selected {kind}"))
        .with("provider", "forge")
        .with("instance", "https://forge.example/")
}

fn key_for(s: &Subject) -> Option<SubjectKey> {
    SubjectKey::new(s.field("provider")?, s.field("instance")?, &s.kind, &s.id).ok()
}

struct Seed {
    p: WorkbenchPlugin,
    journal: Journal,
    session: SessionId,
    action: ActionId,
    menu: Option<Menu>,
}

impl Seed {
    fn pump(&mut self, effects: Vec<Effect>) {
        let mut queue: VecDeque<_> = effects.into();
        let mut steps = 0;
        while let Some(effect) = queue.pop_front() {
            steps += 1;
            assert!(steps < 100, "demo work seed does not settle");
            match effect {
                Effect::Work { id, request } => {
                    let result = work_api::test_support::execute(
                        &mut self.journal,
                        &self.session,
                        *request,
                        Mode::Edit,
                        radar_core::plugin::DEMO_NOW,
                    );
                    queue.extend(self.p.on_msg(Box::new(WorkReply { id, result })));
                }
                Effect::ResolveSources { id, subjects } => {
                    let sources = subjects
                        .into_iter()
                        .filter_map(|subject| {
                            key_for(&subject).map(|key| ResolvedSource { subject, key })
                        })
                        .collect();
                    queue.extend(self.p.on_msg(Box::new(ResolvedSources {
                        id,
                        action_id: Some(self.action.clone()),
                        sources,
                        omitted: 0,
                    })));
                }
                Effect::Menu(menu) => self.menu = Some(menu),
                Effect::Show(subject) => {
                    if let Showing::Page { effects, .. } = self.p.show(&subject) {
                        queue.extend(effects)
                    }
                }
                _ => {}
            }
        }
    }

    fn menu(&mut self, id: &str, value: &str) {
        let fx = self.p.on_menu(id, value);
        self.pump(fx);
    }
}

pub(crate) fn sample() -> WorkbenchPlugin {
    let dir = std::env::temp_dir().join(format!("radar-work-demo-{}", ActionId::new().as_str()));
    let mut journal = Journal::open(&dir.join("journal.sqlite3")).expect("scratch journal");
    let session = journal.start_session().expect("scratch session");
    let sources = [
        source("pull-request", "acme/api/7"),
        source("ticket", "ABC-12"),
        source("checkout", "/home/user/projects/api"),
        source("workflow-run", "acme/api/42/1"),
    ];
    let mut descriptor = ActionDescriptor::new("source.selected", Origin::Host);
    descriptor.subjects = sources
        .iter()
        .map(|s| SubjectSnapshot::new(key_for(s).expect("demo source key"), SubjectRole::Context))
        .collect();
    let action = ActionId::new();
    journal
        .begin_with_id_at(&session, &action, descriptor, None, radar_core::plugin::DEMO_NOW)
        .expect("scratch action");
    let mut seed = Seed {
        p: WorkbenchPlugin {
            // A fixed seed keeps the ids the part allocates the same every run.
            seed: "10000000-0000-4000-8000-000000000000".into(),
            ..Default::default()
        },
        journal,
        session,
        action,
        menu: None,
    };
    let fx = seed.p.tick(radar_core::plugin::DEMO_NOW);
    seed.pump(fx);
    let fx = seed
        .p
        .act_with_related(&sources[0], "work.link", &sources[1..]);
    seed.pump(fx);
    seed.menu("work.choose", "new");
    seed.menu("work.new", TITLE);
    for i in 1..sources.len() {
        seed.menu("work.sources", &format!("toggle:{i}"));
    }
    seed.menu("work.sources", "review");
    seed.menu("work.confirm", "yes");
    // A group: the second menu is the group chooser, "new" asks for its name.
    let fx = seed.p.group_menu();
    seed.pump(fx);
    let chooser = seed.menu.as_ref().expect("the group menu opens").id.clone();
    let fx = seed.p.on_menu(&chooser, "new");
    seed.pump(fx);
    let named = seed.menu.as_ref().expect("the name prompt opens").id.clone();
    seed.menu(&named, GROUP);
    seed.menu("work.confirm", "yes");
    assert!(seed.p.error.is_none(), "{:?}", seed.p.error);
    let Seed { p, journal, .. } = seed;
    drop(journal);
    let _ = std::fs::remove_dir_all(&dir);
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_core::headless::Headless;
    use radar_core::view::{Intent, Node, PaneId, Target};

    fn headless() -> Headless {
        let mut host = radar_core::Host::new(vec![Box::new(WorkbenchPlugin::default())], radar_core::plugin::DEMO_NOW).unwrap();
        host.load_config(Some("")).unwrap();
        for p in &mut host.plugins {
            p.demo();
        }
        Headless::new(host, 160, 40)
    }

    #[test]
    fn the_demo_work_item_opens_from_the_work_screen_and_by_its_subject() {
        let mut h = headless();
        h.show_screen(0);
        let view = h.screen().expect("the Work screen draws");
        let Some(Node::List { rows, .. }) = view.pane(PaneId::Middle).map(|p| &p.body) else { panic!("{view:#?}") };
        assert!(rows.len() > 1 && serde_json::to_string(&rows[0]).unwrap().contains(TITLE), "the task leads the list");
        let target: Target = rows[0].target.clone();
        h.focus(PaneId::Middle);
        h.send(Intent::Activate { target });
        let text = serde_json::to_string(&h.screen().unwrap()).unwrap();
        for want in [TITLE, "pull-request", "acme/api/7", "ABC-12", "authored", GROUP] {
            assert!(text.contains(want), "{want} missing: {text}");
        }
        // The route other parts take: show the work-item subject.
        let id = h.host().plugins[0].snapshot().unwrap()["selected"].as_str().unwrap().to_string();
        h.show_screen(0);
        h.send(Intent::Show { target: Target::new("none"), subject: (&Subject::new("work-item", id, "Work item")).into() });
        assert_eq!(h.host().screen().map(|(_, s)| s.id), Some("work"));
        assert!(serde_json::to_string(&h.screen().unwrap()).unwrap().contains(TITLE));
    }
}
