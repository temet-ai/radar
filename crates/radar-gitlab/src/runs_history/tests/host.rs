use super::*;
use radar_core::Host;
use radar_core::host::{Event, HostEffect, JournalRecord};
use radar_core::journal::{ActionId, Journal, Query};

fn records(effects: &[HostEffect]) -> Vec<JournalRecord> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            HostEffect::Journal(record) => Some(record.clone()),
            _ => None,
        })
        .collect()
}
fn tasks(effects: &[HostEffect]) -> Vec<(ActionId, Option<ActionId>)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            HostEffect::Tracked {
                action,
                semantic,
                effect,
            } if matches!(&**effect, HostEffect::Spawn { .. }) => {
                Some((action.clone(), semantic.clone()))
            }
            _ => None,
        })
        .collect()
}
fn descriptor(records: &[JournalRecord], id: &ActionId) -> ActionDescriptor {
    records
        .iter()
        .find_map(|record| match record {
            JournalRecord::Begin {
                id: got,
                descriptor,
                ..
            } if got == id => Some(descriptor.clone()),
            _ => None,
        })
        .unwrap()
}
fn terminal(records: &[JournalRecord], id: &ActionId) -> Vec<Outcome> {
    records
        .iter()
        .filter_map(|record| match record {
            JournalRecord::Outcome {
                id: got, outcome, ..
            } if got == id && outcome.is_terminal() => Some(*outcome),
            _ => None,
        })
        .collect()
}
fn deliver(host: &mut Host, task: &(ActionId, Option<ActionId>), msg: Msg) -> Vec<HostEffect> {
    host.update(Event::Tracked {
        action: task.0.clone(),
        semantic: task.1.clone(),
        event: Box::new(Event::Msg { plugin: 0, msg }),
    })
}

struct Begin;
type Captured = std::sync::Arc<std::sync::Mutex<Option<crate::page_ownership::Read>>>;
type RunsCaptured = std::sync::Arc<std::sync::Mutex<Option<crate::runs_ownership::Request>>>;
struct Reads(GitlabPlugin, Captured, RunsCaptured);
impl Plugin for Reads {
    fn id(&self) -> &'static str {
        "gitlab"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn on_msg(&mut self, msg: Msg) -> Vec<Effect> {
        match msg.downcast::<Begin>() {
            Ok(_) => {
                let effects = if self.0.pages.is_empty() {
                    pages::push(&mut self.0, pages::Kind::Repo(PROJECT.into()))
                } else {
                    refresh(&mut self.0)
                };
                *self.1.lock().unwrap() = self
                    .0
                    .failed_reads
                    .get(&crate::why::key(PROJECT, 901))
                    .copied();
                *self.2.lock().unwrap() = Some(crate::runs_ownership::pending(&self.0, PROJECT));
                effects
            }
            Err(msg) => self.0.on_msg(msg),
        }
    }
}

#[test]
fn actual_host_keeps_sibling_outcomes_private_and_interruption_unknown_without_map_facts() {
    for (expected, stale) in [Outcome::Succeeded, Outcome::Partial, Outcome::Failed]
        .into_iter().flat_map(|outcome| [(outcome, false), (outcome, true)]) {
        let mut p = page_plugin();
        for data in p.data.values_mut() {
            for project in &mut data.projects {
                if project.full_path == PROJECT {
                    project.main_pipeline = Some(crate::model::Pipeline::Failed);
                    project.main_pipeline_url = Some(format!("{BASE}/{PROJECT}/-/pipelines/901"));
                    project.default_branch = "feature/A".into();
                }
            }
        }
        let captured = Captured::default();
        let runs_captured = RunsCaptured::default();
        let mut host = Host::new(vec![Box::new(Reads(p, captured.clone(), runs_captured.clone()))], 1).unwrap();
        host.enable_journal();
        let initial = host.update(Event::Msg {
            plugin: 0,
            msg: Box::new(Begin),
        });
        let mut all = records(&initial);
        let pending = tasks(&initial);
        assert_eq!(pending.len(), 2);
        let runs = pending
            .iter()
            .find(|(_, semantic)| {
                descriptor(&all, semantic.as_ref().unwrap()).kind.as_str() == "gitlab.page.runs"
            })
            .unwrap();
        let diagnosis = pending
            .iter()
            .find(|(_, semantic)| {
                descriptor(&all, semantic.as_ref().unwrap()).kind.as_str()
                    == "gitlab.pipeline.diagnosis"
            })
            .unwrap();
        let runs_id = runs.1.as_ref().unwrap();
        let diagnosis_id = diagnosis.1.as_ref().unwrap();
        let scope = descriptor(&all, runs_id);
        assert_eq!(scope.subjects[1].key.id, format!("{PROJECT}:feature/A"));
        let failed = deliver(
            &mut host,
            diagnosis,
            Box::new(GlMsg::Failed {
                request: crate::log_ownership::PipelineRead {
                    project: PROJECT.into(),
                    pipeline: 901,
                    owner: captured.lock().unwrap().unwrap(),
                    activity: 0,
                },
                action: descriptor(&all, diagnosis_id),
                read: crate::why::DiagnosisRead {
                    result: Err("SECRET_DIAGNOSIS_ERROR".into()),
                    logs: Some(vec![]),
                },
            }),
        );
        let failed = records(&failed);
        assert_eq!(terminal(&failed, diagnosis_id), [Outcome::Failed]);
        assert!(terminal(&failed, runs_id).is_empty());
        all.extend(failed);
        let result = if expected == Outcome::Failed {
            Err("SECRET_RUNS_ERROR".into())
        } else {
            Ok(read(if expected == Outcome::Partial {
                json!({"nodes": [node(), null]})
            } else {
                json!({"nodes": [node()]})
            }))
        };
        let request = runs_captured.lock().unwrap().clone().unwrap();
        if stale {
            // Supersede the real dispatch before the original tracked read completes.
            let newer = host.update(Event::Msg { plugin: 0, msg: Box::new(Begin) });
            all.extend(records(&newer));
        }
        let completed = deliver(
            &mut host,
            runs,
            Box::new(GlMsg::Runs {
                request,
                action: scope.clone(),
                result,
            }),
        );
        let completed = records(&completed);
        assert_eq!(terminal(&completed, runs_id), [expected]);
        assert!(terminal(&completed, diagnosis_id).is_empty());
        for record in &completed {
            if let JournalRecord::Begin {
                descriptor, parent, ..
            } = record
                && descriptor.kind.as_str().starts_with("gitlab.page.runs.")
            {
                assert_eq!(parent.as_ref(), Some(&runs.0));
                assert_eq!(descriptor.subjects, scope.subjects);
            }
        }
        all.extend(completed);
        // A real forced refresh is dispatched but never polled or delivered.
        let interrupted = host.update(Event::Msg {
            plugin: 0,
            msg: Box::new(Begin),
        });
        all.extend(records(&interrupted));
        let dir =
            std::env::temp_dir().join(format!("radar-runs-history-{}", ActionId::new().as_str()));
        let mut journal = Journal::open(&dir.join("actions.sqlite3")).unwrap();
        let session = journal.start_session().unwrap();
        for record in all {
            match record {
                JournalRecord::Begin {
                    id,
                    descriptor,
                    parent,
                } => journal
                    .begin_with_id(&session, &id, descriptor, parent.as_ref())
                    .unwrap(),
                JournalRecord::Outcome {
                    id,
                    outcome,
                    metadata,
                } => {
                    journal.record(&id, outcome, metadata).unwrap();
                }
                JournalRecord::Gap { .. } => panic!("unexpected gap"),
            }
        }
        journal.finish_session(&session).unwrap();
        let events = journal
            .query(&Query {
                limit: 1000,
                ..Default::default()
            })
            .unwrap();
        let exported = serde_json::to_string(&events).unwrap();
        assert!(!exported.contains("SECRET"), "{exported}");
        for transient in ["runs_scopes", "ConnectionStamp", "pending", "request_id"] {
            assert!(!exported.contains(transient), "{transient}");
        }
        assert_eq!(events.iter().filter(|e| e.kind.as_str() == "gitlab.page.runs.apply.skipped.stale"
            && e.outcome == Outcome::Blocked).count(), usize::from(stale));
        assert!(
            events
                .iter()
                .any(|e| e.kind.as_str() == "gitlab.page.runs" && e.outcome == Outcome::Unknown)
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| e.action_id == *runs_id && e.outcome.is_terminal())
                .map(|e| e.outcome)
                .collect::<Vec<_>>(),
            [expected]
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| e.action_id == *diagnosis_id && e.outcome.is_terminal())
                .map(|e| e.outcome)
                .collect::<Vec<_>>(),
            [Outcome::Failed]
        );
        assert_eq!(events.iter().filter(|e| e.kind.as_str() == "gitlab.page.runs.entries" && e.outcome.is_terminal()).count(), usize::from(expected != Outcome::Failed));
        assert!(
            events
                .iter()
                .filter(|e| e.kind.as_str().starts_with("gitlab.page.runs"))
                .flat_map(|e| &e.subjects)
                .all(|s| s.role != SubjectRole::Result)
        );
        assert_eq!(journal.work_map_snapshot().unwrap().map_sequence(), 0);
        assert!(
            journal
                .query_items(&Default::default(), 10, None)
                .unwrap()
                .items
                .is_empty()
        );
        drop(journal);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
