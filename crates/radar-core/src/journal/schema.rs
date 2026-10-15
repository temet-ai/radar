// All changes are additive, inside a transaction. Never discard history to
// recover from an unsupported version, malformed file, or failed migration.
pub(super) const V1: &str = r#"
CREATE TABLE sessions (
    id TEXT PRIMARY KEY NOT NULL,
    started_at_ms INTEGER NOT NULL,
    ended_at_ms INTEGER,
    process_id INTEGER NOT NULL
);
CREATE TABLE actions (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    parent_action_id TEXT REFERENCES actions(id),
    root_action_id TEXT NOT NULL REFERENCES actions(id) DEFERRABLE INITIALLY DEFERRED,
    correlation_id TEXT NOT NULL REFERENCES actions(id) DEFERRABLE INITIALLY DEFERRED,
    kind TEXT NOT NULL CHECK(length(kind) BETWEEN 1 AND 96),
    origin_kind TEXT NOT NULL CHECK(origin_kind IN ('host','plugin')),
    origin_id TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
);
CREATE TABLE action_events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    action_id TEXT NOT NULL REFERENCES actions(id),
    occurred_at_ms INTEGER NOT NULL,
    outcome TEXT NOT NULL CHECK(outcome IN
        ('requested','confirmed','started','succeeded','failed','cancelled','blocked','partial','unknown')),
    metadata_json TEXT NOT NULL CHECK(length(metadata_json)<=4096)
);
CREATE INDEX actions_session ON actions(session_id,created_at_ms);
CREATE INDEX actions_kind ON actions(kind,created_at_ms);
CREATE INDEX actions_root ON actions(root_action_id,created_at_ms);
CREATE INDEX actions_parent ON actions(parent_action_id);
CREATE INDEX actions_correlation ON actions(correlation_id);
CREATE INDEX events_time ON action_events(occurred_at_ms,sequence);
CREATE INDEX events_action ON action_events(action_id,sequence);
CREATE INDEX sessions_time ON sessions(started_at_ms);
CREATE TRIGGER actions_no_update BEFORE UPDATE ON actions BEGIN SELECT RAISE(ABORT,'actions are immutable'); END;
CREATE TRIGGER actions_no_delete BEFORE DELETE ON actions BEGIN SELECT RAISE(ABORT,'actions are immutable'); END;
CREATE TRIGGER events_no_update BEFORE UPDATE ON action_events BEGIN SELECT RAISE(ABORT,'events are immutable'); END;
CREATE TRIGGER events_no_delete BEFORE DELETE ON action_events BEGIN SELECT RAISE(ABORT,'events are immutable'); END;
"#;

pub(super) const V2: &str = r#"
CREATE TABLE subjects (
    id INTEGER PRIMARY KEY,
    provider TEXT NOT NULL,
    instance TEXT NOT NULL,
    kind TEXT NOT NULL,
    external_id TEXT NOT NULL,
    UNIQUE(provider,instance,kind,external_id)
);
CREATE TABLE action_subjects (
    action_id TEXT NOT NULL REFERENCES actions(id),
    ordinal INTEGER NOT NULL CHECK(ordinal>=0 AND ordinal<32),
    subject_id INTEGER NOT NULL REFERENCES subjects(id),
    role TEXT NOT NULL CHECK(role IN ('primary','source','destination','context','result')),
    url_snapshot TEXT,
    PRIMARY KEY(action_id,ordinal)
);
CREATE INDEX subject_actions ON action_subjects(subject_id,action_id);
CREATE TRIGGER subjects_no_update BEFORE UPDATE ON subjects BEGIN SELECT RAISE(ABORT,'subjects are immutable'); END;
CREATE TRIGGER subjects_no_delete BEFORE DELETE ON subjects BEGIN SELECT RAISE(ABORT,'subjects are immutable'); END;
CREATE TRIGGER action_subjects_no_update BEFORE UPDATE ON action_subjects BEGIN SELECT RAISE(ABORT,'subject snapshots are immutable'); END;
CREATE TRIGGER action_subjects_no_delete BEFORE DELETE ON action_subjects BEGIN SELECT RAISE(ABORT,'subject snapshots are immutable'); END;
"#;

pub(super) const V3: &str = r#"
ALTER TABLE sessions ADD COLUMN build_version TEXT NOT NULL DEFAULT 'unknown';
ALTER TABLE sessions ADD COLUMN origin TEXT NOT NULL DEFAULT 'unknown' CHECK(origin IN ('tui','cli','unknown'));
ALTER TABLE sessions ADD COLUMN event_version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE action_events ADD COLUMN event_version INTEGER NOT NULL DEFAULT 1;
"#;

pub(super) const V4: &str = r#"
ALTER TABLE action_events ADD COLUMN recorded_at_ms INTEGER;
ALTER TABLE sessions ADD COLUMN recording_degraded INTEGER NOT NULL DEFAULT 0 CHECK(recording_degraded IN (0,1));
ALTER TABLE sessions ADD COLUMN recording_gap_count INTEGER NOT NULL DEFAULT 0 CHECK(recording_gap_count>=0);
ALTER TABLE sessions ADD COLUMN dropped_records INTEGER NOT NULL DEFAULT 0 CHECK(dropped_records>=0);
ALTER TABLE sessions ADD COLUMN write_errors INTEGER NOT NULL DEFAULT 0 CHECK(write_errors>=0);
CREATE TABLE recording_gaps (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    observed_at_ms INTEGER NOT NULL,
    recorded_at_ms INTEGER NOT NULL,
    dropped_records INTEGER NOT NULL CHECK(dropped_records>=0),
    write_errors INTEGER NOT NULL CHECK(write_errors>=0),
    CHECK(dropped_records>0 OR write_errors>0)
);
CREATE INDEX gaps_session ON recording_gaps(session_id,sequence);
CREATE TRIGGER recording_gaps_no_update BEFORE UPDATE ON recording_gaps BEGIN SELECT RAISE(ABORT,'recording gaps are immutable'); END;
CREATE TRIGGER recording_gaps_no_delete BEFORE DELETE ON recording_gaps BEGIN SELECT RAISE(ABORT,'recording gaps are immutable'); END;
"#;

pub(super) const V5: &str = r#"
CREATE TABLE work_map_store (
 singleton INTEGER PRIMARY KEY CHECK(singleton=1), id TEXT NOT NULL UNIQUE
);
CREATE TABLE work_changes (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
 version INTEGER NOT NULL CHECK(version>0),
 id TEXT NOT NULL UNIQUE,
 recording_action_id TEXT NOT NULL REFERENCES actions(id),
 evidence_event_id TEXT NOT NULL REFERENCES action_events(id),
 basis TEXT NOT NULL CHECK(basis IN ('observed','user_asserted','inferred')),
 kind TEXT NOT NULL CHECK(kind IN ('create_item','bind_subject','create_tag','rename_tag','assign_tag','relate_items','retract')),
 observed_at_ms INTEGER NOT NULL,
 recorded_at_ms INTEGER NOT NULL,
 request_json TEXT NOT NULL CHECK(length(CAST(request_json AS BLOB))<=8192),
 retracts_id TEXT REFERENCES work_changes(id)
);
CREATE TABLE work_items (
 id TEXT PRIMARY KEY NOT NULL,
 kind TEXT NOT NULL CHECK(length(kind) BETWEEN 1 AND 96),
 created_change_id TEXT NOT NULL UNIQUE REFERENCES work_changes(id)
);
CREATE TABLE work_item_bindings (
 change_id TEXT PRIMARY KEY NOT NULL REFERENCES work_changes(id),
 item_id TEXT NOT NULL REFERENCES work_items(id),
 subject_id INTEGER NOT NULL REFERENCES subjects(id),
 from_sequence INTEGER NOT NULL CHECK(from_sequence>0),
 through_sequence INTEGER CHECK(through_sequence>=from_sequence)
);
CREATE TABLE work_tags (
 id TEXT PRIMARY KEY NOT NULL,
 namespace TEXT NOT NULL CHECK(length(namespace) BETWEEN 1 AND 96),
 stable_key TEXT NOT NULL CHECK(length(stable_key) BETWEEN 1 AND 96),
 created_change_id TEXT NOT NULL UNIQUE REFERENCES work_changes(id),
 UNIQUE(namespace,stable_key)
);
CREATE TABLE work_tag_labels (
 change_id TEXT PRIMARY KEY NOT NULL REFERENCES work_changes(id),
 tag_id TEXT NOT NULL REFERENCES work_tags(id),
 label TEXT NOT NULL CHECK(length(CAST(label AS BLOB)) BETWEEN 1 AND 128)
);
CREATE TABLE work_tag_assignments (
 change_id TEXT PRIMARY KEY NOT NULL REFERENCES work_changes(id),
 tag_id TEXT NOT NULL REFERENCES work_tags(id),
 item_id TEXT REFERENCES work_items(id),
 action_id TEXT REFERENCES actions(id),
 CHECK((item_id IS NOT NULL)+(action_id IS NOT NULL)=1)
);
CREATE TABLE work_relations (
 change_id TEXT PRIMARY KEY NOT NULL REFERENCES work_changes(id),
 from_item_id TEXT NOT NULL REFERENCES work_items(id),
 to_item_id TEXT NOT NULL REFERENCES work_items(id),
 kind TEXT NOT NULL CHECK(kind IN ('references','part_of','depends_on')),
 CHECK(from_item_id<>to_item_id)
);
CREATE INDEX work_changes_recording ON work_changes(recording_action_id,sequence);
CREATE INDEX work_changes_evidence ON work_changes(evidence_event_id,sequence);
CREATE INDEX work_changes_retracts ON work_changes(retracts_id,sequence);
CREATE INDEX work_bindings_item ON work_item_bindings(item_id,from_sequence);
CREATE INDEX work_bindings_subject ON work_item_bindings(subject_id,from_sequence);
CREATE INDEX work_labels_tag ON work_tag_labels(tag_id,change_id);
CREATE INDEX work_assignments_item ON work_tag_assignments(item_id,tag_id);
CREATE INDEX work_assignments_action ON work_tag_assignments(action_id,tag_id);
CREATE INDEX work_assignments_tag ON work_tag_assignments(tag_id,item_id,action_id);
CREATE INDEX work_relations_from ON work_relations(from_item_id,change_id);
CREATE INDEX work_relations_to ON work_relations(to_item_id,change_id);
CREATE TRIGGER work_map_store_no_update BEFORE UPDATE ON work_map_store BEGIN SELECT RAISE(ABORT,'work map identity is immutable'); END;
CREATE TRIGGER work_map_store_no_delete BEFORE DELETE ON work_map_store BEGIN SELECT RAISE(ABORT,'work map identity is immutable'); END;
CREATE TRIGGER work_changes_no_update BEFORE UPDATE ON work_changes BEGIN SELECT RAISE(ABORT,'work map history is immutable'); END;
CREATE TRIGGER work_changes_no_delete BEFORE DELETE ON work_changes BEGIN SELECT RAISE(ABORT,'work map history is immutable'); END;
CREATE TRIGGER work_items_no_update BEFORE UPDATE ON work_items BEGIN SELECT RAISE(ABORT,'work items are immutable'); END;
CREATE TRIGGER work_items_no_delete BEFORE DELETE ON work_items BEGIN SELECT RAISE(ABORT,'work items are immutable'); END;
CREATE TRIGGER work_item_bindings_no_update BEFORE UPDATE ON work_item_bindings BEGIN SELECT RAISE(ABORT,'work bindings are immutable'); END;
CREATE TRIGGER work_item_bindings_no_delete BEFORE DELETE ON work_item_bindings BEGIN SELECT RAISE(ABORT,'work bindings are immutable'); END;
CREATE TRIGGER work_tags_no_update BEFORE UPDATE ON work_tags BEGIN SELECT RAISE(ABORT,'work tags are immutable'); END;
CREATE TRIGGER work_tags_no_delete BEFORE DELETE ON work_tags BEGIN SELECT RAISE(ABORT,'work tags are immutable'); END;
CREATE TRIGGER work_tag_labels_no_update BEFORE UPDATE ON work_tag_labels BEGIN SELECT RAISE(ABORT,'work labels are immutable'); END;
CREATE TRIGGER work_tag_labels_no_delete BEFORE DELETE ON work_tag_labels BEGIN SELECT RAISE(ABORT,'work labels are immutable'); END;
CREATE TRIGGER work_tag_assignments_no_update BEFORE UPDATE ON work_tag_assignments BEGIN SELECT RAISE(ABORT,'work assignments are immutable'); END;
CREATE TRIGGER work_tag_assignments_no_delete BEFORE DELETE ON work_tag_assignments BEGIN SELECT RAISE(ABORT,'work assignments are immutable'); END;
CREATE TRIGGER work_relations_no_update BEFORE UPDATE ON work_relations BEGIN SELECT RAISE(ABORT,'work relations are immutable'); END;
CREATE TRIGGER work_relations_no_delete BEFORE DELETE ON work_relations BEGIN SELECT RAISE(ABORT,'work relations are immutable'); END;
"#;
