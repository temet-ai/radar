use super::*;
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("radar-plugin-cache-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn db_path(&self) -> PathBuf {
        self.0.join("cache.sqlite3")
    }

    fn open(&self) -> CacheDb {
        CacheDb::open(&self.db_path()).unwrap()
    }

    fn raw(&self) -> Connection {
        Connection::open(self.db_path()).unwrap()
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        // Only this unique test fixture is removed, never a user's state.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn snapshot() -> Value {
    json!({
        "data": {"site": [1, 2, 3]},
        "me": "alice",
        "a/b~c": {"escaped": true},
        "sync": {"saved_at": 10},
    })
}

fn saved_at(dir: &TestDir, plugin: &str, key: &str) -> i64 {
    dir.raw()
        .query_row(
            "SELECT saved_at_ms FROM plugin_cache WHERE plugin=?1 AND key=?2",
            params![plugin, key],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn a_snapshot_reads_back_as_it_was_saved_after_a_restart() {
    let dir = TestDir::new();
    let mut db = dir.open();
    db.save("alpha", &snapshot(), 1).unwrap();
    db.save("beta", &json!([1, "two", null]), 1).unwrap();
    db.save("delta", &json!({}), 1).unwrap();
    drop(db);

    let db = dir.open();
    assert_eq!(db.load("alpha").unwrap(), Some(snapshot()));
    assert_eq!(db.load("beta").unwrap(), Some(json!([1, "two", null])));
    assert_eq!(db.load("delta").unwrap(), Some(json!({})));
    assert_eq!(db.load("gamma").unwrap(), None);
    // One row per member of an object snapshot, under its JSON pointer.
    let keys: Vec<String> = dir
        .raw()
        .prepare("SELECT key FROM plugin_cache WHERE plugin='alpha' ORDER BY key")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(keys, ["/a~1b~0c", "/data", "/me", "/sync"]);
}

#[test]
fn a_save_writes_only_the_members_that_changed() {
    let dir = TestDir::new();
    let mut db = dir.open();
    let first = db.save("alpha", &snapshot(), 1).unwrap();
    assert_eq!(
        first,
        Saved {
            written: 4,
            unchanged: 0,
            removed: 0
        }
    );

    let same = db.save("alpha", &snapshot(), 2).unwrap();
    assert_eq!(
        same,
        Saved {
            written: 0,
            unchanged: 4,
            removed: 0
        }
    );
    assert_eq!(
        saved_at(&dir, "alpha", "/data"),
        1,
        "an unchanged row is not rewritten"
    );

    let mut next = snapshot();
    next["sync"]["saved_at"] = json!(11);
    next.as_object_mut().unwrap().remove("me");
    let changed = db.save("alpha", &next, 3).unwrap();
    assert_eq!(
        changed,
        Saved {
            written: 1,
            unchanged: 2,
            removed: 1
        }
    );
    assert_eq!(saved_at(&dir, "alpha", "/sync"), 3);
    assert_eq!(saved_at(&dir, "alpha", "/data"), 1);
    drop(db);

    // The stored hashes are what a new run compares against.
    let mut db = dir.open();
    assert_eq!(db.save("alpha", &next, 4).unwrap().written, 0);
    assert_eq!(db.load("alpha").unwrap(), Some(next));
}

#[test]
fn an_old_json_file_is_imported_once_and_renamed() {
    let dir = TestDir::new();
    let file = legacy_file(&dir.0, "gamma");
    std::fs::write(&file, snapshot().to_string()).unwrap();

    let mut db = dir.open();
    assert_eq!(
        restore(Some(&mut db), &dir.0, "gamma", 1),
        Restored::Imported(snapshot())
    );
    assert!(!file.exists());
    assert!(dir.0.join("gamma.json.imported").exists());

    // An old radar that writes the file again does not replace the stored rows.
    std::fs::write(&file, json!({"stale": true}).to_string()).unwrap();
    let mut db = dir.open();
    assert_eq!(
        restore(Some(&mut db), &dir.0, "gamma", 2),
        Restored::Stored(snapshot())
    );
    assert!(
        file.exists(),
        "a file is imported only when no rows are stored"
    );
}

#[test]
fn without_the_database_the_json_files_are_read_and_written() {
    let dir = TestDir::new();
    std::fs::write(legacy_file(&dir.0, "beta"), "[1]").unwrap();
    assert_eq!(restore(None, &dir.0, "beta", 1), Restored::File(json!([1])));
    assert_eq!(restore(None, &dir.0, "gamma", 1), Restored::Missing);

    let writer = Writer::start(None, dir.0.clone());
    writer.save("gamma", json!({"data": 1}), None);
    writer.finish();
    let text = std::fs::read_to_string(legacy_file(&dir.0, "gamma")).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&text).unwrap(),
        json!({"data": 1})
    );
}

#[test]
fn a_corrupt_row_drops_that_plugins_cache_and_nothing_else() {
    let dir = TestDir::new();
    let mut db = dir.open();
    db.save("alpha", &snapshot(), 1).unwrap();
    db.save("beta", &json!([1]), 1).unwrap();
    dir.raw()
        .execute(
            "UPDATE plugin_cache SET payload='{not json' WHERE plugin='alpha' AND key='/data'",
            [],
        )
        .unwrap();

    let mut db = dir.open();
    assert!(matches!(
        restore(Some(&mut db), &dir.0, "alpha", 2),
        Restored::Dropped(_)
    ));
    assert_eq!(db.load("alpha").unwrap(), None, "its rows are removed");
    assert_eq!(
        restore(Some(&mut db), &dir.0, "alpha", 3),
        Restored::Missing
    );
    assert_eq!(
        restore(Some(&mut db), &dir.0, "beta", 3),
        Restored::Stored(json!([1]))
    );
    // A save after the drop stores every member again.
    assert_eq!(db.save("alpha", &snapshot(), 4).unwrap().written, 4);
}

#[test]
fn a_corrupt_json_file_is_dropped_and_not_read_again() {
    let dir = TestDir::new();
    let file = legacy_file(&dir.0, "gamma");
    std::fs::write(&file, "{\"data\":").unwrap();
    let mut db = dir.open();
    assert!(matches!(
        restore(Some(&mut db), &dir.0, "gamma", 1),
        Restored::Dropped(_)
    ));
    assert!(!file.exists());
    assert_eq!(
        restore(Some(&mut db), &dir.0, "gamma", 2),
        Restored::Missing
    );
}

#[test]
fn the_writer_stores_each_save_and_reports_it() {
    let dir = TestDir::new();
    let stored = Arc::new(AtomicUsize::new(0));
    let writer = Writer::start(Some(dir.open()), dir.0.clone());
    for n in 0..3 {
        let stored = stored.clone();
        writer.save(
            "alpha",
            json!({ "n": n }),
            Some(Box::new(move |ok| {
                assert!(ok);
                stored.fetch_add(1, Ordering::SeqCst);
            })),
        );
    }
    writer.finish();
    assert_eq!(
        stored.load(Ordering::SeqCst),
        3,
        "every queued save is reported"
    );
    assert_eq!(dir.open().load("alpha").unwrap(), Some(json!({ "n": 2 })));
    // A save after finish is reported as not stored.
    let refused = Arc::new(AtomicUsize::new(0));
    let seen = refused.clone();
    writer.save(
        "alpha",
        json!({}),
        Some(Box::new(move |ok| {
            assert!(!ok);
            seen.fetch_add(1, Ordering::SeqCst);
        })),
    );
    assert_eq!(refused.load(Ordering::SeqCst), 1);
}

#[test]
fn a_newer_or_foreign_database_is_refused_and_left_as_it_was() {
    let dir = TestDir::new();
    dir.open().save("beta", &json!([1]), 1).unwrap();
    dir.raw()
        .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
        .unwrap();
    assert!(CacheDb::open(&dir.db_path()).is_err());
    let version: u32 = dir
        .raw()
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION + 1);

    // The action journal is not a cache database, nor is any other file.
    let journal = dir.0.join("journal").join("actions.sqlite3");
    drop(crate::journal::Journal::open(&journal).unwrap());
    assert!(CacheDb::open(&journal).is_err());
    let other = dir.0.join("other.sqlite3");
    Connection::open(&other)
        .unwrap()
        .execute_batch("CREATE TABLE t(x)")
        .unwrap();
    assert!(CacheDb::open(&other).is_err());
}

#[test]
fn the_cache_database_sits_beside_the_journal_folder_not_in_it() {
    let journal = crate::journal::Journal::default_path().unwrap();
    let cache = default_path().unwrap();
    assert_eq!(
        Some(cache.parent().unwrap()),
        journal.parent().unwrap().parent()
    );
    assert_eq!(cache.file_name().unwrap(), "cache.sqlite3");
}

/// File against database on real caches: copy them into a scratch folder,
/// then `RADAR_CACHE_MEASURE_DIR=<folder> cargo test --release -p radar-core --lib
/// plugin_cache::tests::measure -- --ignored --nocapture`.
#[test]
#[ignore = "measures real cache files named by RADAR_CACHE_MEASURE_DIR"]
fn measure() {
    use std::time::Instant;
    let Some(src) = std::env::var_os("RADAR_CACHE_MEASURE_DIR").map(PathBuf::from) else {
        return;
    };
    let dir = TestDir::new();
    let mut names: Vec<String> = std::fs::read_dir(&src)
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_suffix(".json").map(str::to_string))
        .collect();
    names.sort();
    // The best of 30: other work on the machine only ever adds time.
    let best = |f: &mut dyn FnMut(usize)| {
        (0..30)
            .map(|n| {
                let t = Instant::now();
                f(n);
                t.elapsed()
            })
            .min()
            .unwrap()
    };
    let mut values = Vec::new();
    for name in &names {
        let text = std::fs::read_to_string(src.join(format!("{name}.json"))).unwrap();
        std::fs::write(dir.0.join(format!("{name}.json")), &text).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        let plugin: &'static str = Box::leak(name.clone().into_boxed_str());
        dir.open().save(plugin, &value, 0).unwrap();
        values.push((plugin, text.len(), value));
    }

    // Start: every cache read and parsed, the database opened once.
    let files_start = best(&mut |_| {
        for (plugin, ..) in &values {
            let text = std::fs::read_to_string(dir.0.join(format!("{plugin}.json"))).unwrap();
            serde_json::from_str::<Value>(&text).unwrap();
        }
    });
    let plugins: Vec<&str> = values.iter().map(|(plugin, ..)| *plugin).collect();
    let none = dir.0.join("none");
    let db_start = best(&mut |_| {
        let mut db = dir.open();
        let restored = restore_all(Some(&mut db), &none, &plugins, 0);
        assert!(restored.iter().all(|r| matches!(r, Restored::Stored(_))));
    });
    let db_open = best(&mut |_| drop(dir.open()));
    println!(
        "start, all caches: files {files_start:.2?}, database {db_start:.2?} (open {db_open:.2?})"
    );

    println!(
        "plugin       bytes   file load    db load  file save  db save same  db save 1 changed"
    );
    let mut db = dir.open();
    for (plugin, bytes, value) in &values {
        let scratch = dir.0.join(format!("{plugin}.json"));
        let file_load = best(&mut |_| {
            serde_json::from_str::<Value>(&std::fs::read_to_string(&scratch).unwrap()).unwrap();
        });
        let db_load = best(&mut |_| assert!(db.load(plugin).unwrap().is_some()));
        let file_save = best(&mut |_| std::fs::write(&scratch, value.to_string()).unwrap());
        let db_same = best(&mut |_| assert_eq!(db.save(plugin, value, 1).unwrap().written, 0));

        // One small member changed, as a refresh that touched one part of
        // the snapshot; the two versions are built before the clock starts.
        let variants: Vec<Value> = (0..2)
            .map(|n| {
                let mut next = value.clone();
                if let Some(map) = next.as_object_mut() {
                    map.insert("measure".into(), json!(n));
                }
                next
            })
            .collect();
        let db_changed = best(&mut |n| {
            assert_eq!(db.save(plugin, &variants[n % 2], 2).unwrap().written, 1);
        });
        println!(
            "{plugin:<10} {bytes:>8}  {file_load:>9.2?}  {db_load:>9.2?}  {file_save:>9.2?}  {db_same:>12.2?}  {db_changed:>12.2?}"
        );
    }
}
