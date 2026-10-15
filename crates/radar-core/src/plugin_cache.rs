//! Plugins' caches in radar's SQLite cache database (`<state>/radar/cache.sqlite3`,
//! table `plugin_cache`), in place of one JSON file each in the state folder.
//! The plugin contract is unchanged: the host stores what `snapshot()` returns
//! and hands it back to `restore()` at start.
//!
//! Not the action journal's file: the journal never holds cache blobs
//! and its tests scan the journal folder for provider text. This database follows the journal's
//! pattern: private 0600 files, WAL, a versioned schema migrated in one
//! immediate transaction, a newer or foreign database refused.
//!
//! A snapshot that is a JSON object is kept as one row per member (key
//! `/<member>`, a JSON pointer), so a refresh rewrites only the members that
//! changed; any other snapshot is one row under key "". A row is written only
//! when its payload's hash differs from the stored one. Writes run on the
//! cache's own thread, never the UI's.
//!
//! An old `<plugin>.json` is imported once, when the database has no rows for
//! that plugin, and renamed to `<plugin>.json.imported`. A row that cannot be
//! read back drops that plugin's cache: the part starts empty and refills it.
//! Where the database cannot be opened, the JSON files are read and written
//! as before.

use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, mpsc};
use std::thread::JoinHandle;

use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, params};
use serde_json::Value;

use crate::log;

/// The payload encoding stored with each row: 1 is JSON text.
const FORMAT: i64 = 1;
/// The database's schema; a newer one is refused, not migrated.
pub(crate) const SCHEMA_VERSION: u32 = 1;
/// "GGCA": marks the file as radar's cache database.
const APPLICATION_ID: i64 = 0x47474341;

/// `key` is "" for a whole snapshot, or a JSON pointer ("/data") to one
/// member of an object snapshot. `hash` is the payload's, so a save that
/// changes nothing writes nothing. Large payloads: a rowid table, not
/// WITHOUT ROWID, and the payload last, so reading the other columns does
/// not walk its overflow pages.
const V1: &str = r#"
CREATE TABLE plugin_cache (
 plugin TEXT NOT NULL CHECK(length(plugin) BETWEEN 1 AND 96),
 key TEXT NOT NULL,
 format INTEGER NOT NULL,
 hash INTEGER NOT NULL,
 saved_at_ms INTEGER NOT NULL,
 payload TEXT NOT NULL,
 PRIMARY KEY(plugin,key)
);
"#;

fn migrate(connection: &mut Connection) -> Result<()> {
    // The version is read under the write lock: two radar processes can start
    // together, and the second must see the first one's committed schema.
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let version: u32 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
    ensure!(
        version <= SCHEMA_VERSION,
        "the cache database is newer than this radar"
    );
    let application: i64 = tx.pragma_query_value(None, "application_id", |row| row.get(0))?;
    ensure!(
        application == APPLICATION_ID || (application == 0 && version == 0),
        "file is not radar's cache database"
    );
    if version == 0 {
        let tables: i64 = tx.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )?;
        ensure!(tables == 0, "refusing to initialize an unrelated database");
    }
    if version < 1 {
        tx.execute_batch(V1)?;
    }
    tx.pragma_update(None, "application_id", APPLICATION_ID)?;
    tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    tx.commit()?;
    Ok(())
}

/// Open (creating and migrating) the cache database at `path`.
fn connect(path: &Path) -> Result<Connection> {
    use crate::journal::{private_file, private_sidecars};
    ensure!(path.is_absolute(), "cache database path must be absolute");
    std::fs::create_dir_all(path.parent().context("cache path needs a directory")?)?;
    private_file(path, true)?;
    private_sidecars(path)?;
    let mut connection = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .context("cannot open the cache database")?;
    connection.busy_timeout(std::time::Duration::from_secs(2))?;
    connection.pragma_update(None, "trusted_schema", false)?;
    // Large pages for large payloads (fewer overflow pages to chain); only a
    // new, empty database takes it.
    connection.pragma_update(None, "page_size", 16384)?;
    migrate(&mut connection)?;
    // Payloads are read through memory maps, not a read call per page.
    connection.pragma_update(None, "mmap_size", 64 * 1024 * 1024)?;
    connection
        .prepare("SELECT plugin,key,format,hash,saved_at_ms,payload FROM plugin_cache LIMIT 0")
        .context("incomplete cache schema")?;
    let mode: String = connection.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
    ensure!(
        mode.eq_ignore_ascii_case("wal"),
        "the cache database requires WAL mode"
    );
    // A cache lost to a power cut is refetched: no fsync on every commit.
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    private_sidecars(path)?;
    Ok(connection)
}

/// The cache rows of every plugin, through one connection.
pub(crate) struct CacheDb {
    connection: Connection,
    /// The stored hash of each plugin's rows by key: what a save compares to.
    known: HashMap<String, BTreeMap<String, i64>>,
}

/// What one save did, row by row.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Saved {
    pub written: usize,
    pub unchanged: usize,
    pub removed: usize,
}

impl CacheDb {
    pub fn open(path: &Path) -> Result<Self> {
        Self::with(connect(path)?)
    }

    fn with(connection: Connection) -> Result<Self> {
        let mut known: HashMap<String, BTreeMap<String, i64>> = HashMap::new();
        {
            let mut rows = connection.prepare("SELECT plugin,key,hash FROM plugin_cache")?;
            let rows = rows.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })?;
            for row in rows {
                let (plugin, key, hash) = row?;
                known.entry(plugin).or_default().insert(key, hash);
            }
        }
        Ok(Self { connection, known })
    }

    /// The plugin's stored snapshot, `None` when it has none; `Err` when a
    /// row cannot be read back.
    #[cfg(test)]
    pub fn load(&self, plugin: &str) -> Result<Option<Value>> {
        parse(self.rows(plugin)?)
    }

    /// The plugin's rows as stored: key, format, payload.
    fn rows(&self, plugin: &str) -> Result<Vec<Row>> {
        let mut statement = self
            .connection
            .prepare_cached("SELECT key,format,payload FROM plugin_cache WHERE plugin=?1")?;
        let rows = statement
            .query_map([plugin], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<Vec<Row>>>()?;
        Ok(rows)
    }

    /// Store this snapshot, writing only the rows whose payload changed and
    /// removing the members it no longer has.
    pub fn save(&mut self, plugin: &str, value: &Value, saved_at_ms: i64) -> Result<Saved> {
        let rows = rows_of(value);
        let known = self.known.get(plugin);
        let mut saved = Saved::default();
        let mut changed = Vec::new();
        for (key, payload) in &rows {
            let hash = hash_of(payload);
            if known.and_then(|k| k.get(key)) == Some(&hash) {
                saved.unchanged += 1;
            } else {
                changed.push((key.as_str(), payload.as_str(), hash));
            }
        }
        let stale: Vec<String> = known
            .map(|k| {
                k.keys()
                    .filter(|key| !rows.iter().any(|(row, _)| row == *key))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        if changed.is_empty() && stale.is_empty() {
            return Ok(saved);
        }
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        {
            let mut upsert = tx.prepare_cached(
                "INSERT INTO plugin_cache(plugin,key,format,payload,hash,saved_at_ms) VALUES (?1,?2,?3,?4,?5,?6) \
                 ON CONFLICT(plugin,key) DO UPDATE SET format=excluded.format,payload=excluded.payload,\
                 hash=excluded.hash,saved_at_ms=excluded.saved_at_ms",
            )?;
            for (key, payload, hash) in &changed {
                upsert.execute(params![plugin, key, FORMAT, payload, hash, saved_at_ms])?;
            }
            let mut remove =
                tx.prepare_cached("DELETE FROM plugin_cache WHERE plugin=?1 AND key=?2")?;
            for key in &stale {
                remove.execute(params![plugin, key])?;
            }
        }
        tx.commit()?;
        let known = self.known.entry(plugin.to_string()).or_default();
        for key in &stale {
            known.remove(key);
        }
        for (key, _, hash) in &changed {
            known.insert((*key).to_string(), *hash);
        }
        saved.written = changed.len();
        saved.removed = stale.len();
        Ok(saved)
    }

    /// Remove every row of this plugin's cache.
    pub fn forget(&mut self, plugin: &str) -> Result<()> {
        self.connection
            .execute("DELETE FROM plugin_cache WHERE plugin=?1", [plugin])?;
        self.known.remove(plugin);
        Ok(())
    }
}

/// A stored row: key, format, payload.
type Row = (String, i64, String);

/// A plugin's rows as its snapshot again, `None` for no rows.
fn parse(rows: Vec<Row>) -> Result<Option<Value>> {
    if rows.is_empty() {
        return Ok(None);
    }
    let mut whole = None;
    let mut members = serde_json::Map::new();
    for (key, format, payload) in rows {
        ensure!(format == FORMAT, "cache row {key:?} has format {format}");
        let value: Value = serde_json::from_str(&payload)
            .with_context(|| format!("cache row {key:?} is not JSON"))?;
        if key.is_empty() {
            whole = Some(value);
        } else {
            let member = member_of(&key).with_context(|| format!("cache key {key:?}"))?;
            members.insert(member, value);
        }
    }
    ensure!(
        whole.is_none() || members.is_empty(),
        "cache rows hold both a whole snapshot and members"
    );
    Ok(Some(whole.unwrap_or(Value::Object(members))))
}

/// A snapshot as rows: an object's members under their JSON pointers, any
/// other value (and an empty object) whole under "".
fn rows_of(value: &Value) -> Vec<(String, String)> {
    match value {
        Value::Object(map) if !map.is_empty() => map
            .iter()
            .map(|(member, v)| (pointer(member), v.to_string()))
            .collect(),
        _ => vec![(String::new(), value.to_string())],
    }
}

fn pointer(member: &str) -> String {
    format!("/{}", member.replace('~', "~0").replace('/', "~1"))
}

fn member_of(key: &str) -> Option<String> {
    key.strip_prefix('/')
        .map(|m| m.replace("~1", "/").replace("~0", "~"))
}

/// The payload's hash. `DefaultHasher::new` uses fixed keys, so it is stable
/// from one run to the next; a Rust release that changes it costs one write.
fn hash_of(payload: &str) -> i64 {
    let mut hasher = std::hash::DefaultHasher::new();
    payload.hash(&mut hasher);
    hasher.finish() as i64
}

/// Where a plugin's restored cache came from.
#[derive(Debug, PartialEq)]
pub(crate) enum Restored {
    /// The database's rows.
    Stored(Value),
    /// The old JSON file, now in the database and renamed.
    Imported(Value),
    /// The old JSON file, the database being unavailable.
    File(Value),
    /// Nothing stored.
    Missing,
    /// Stored but unreadable, and dropped.
    Dropped(String),
}

/// The old cache file of this plugin.
pub(crate) fn legacy_file(dir: &Path, plugin: &str) -> PathBuf {
    dir.join(format!("{plugin}.json"))
}

fn renamed(file: &Path, suffix: &str) -> PathBuf {
    let mut name = file.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// This plugin's cache: the database's rows, else its old JSON file once.
#[cfg(test)]
pub(crate) fn restore(db: Option<&mut CacheDb>, dir: &Path, plugin: &str, now_ms: i64) -> Restored {
    let mut all = restore_all(db, dir, &[plugin], now_ms);
    all.pop().unwrap_or(Restored::Missing)
}

/// These plugins' caches, in their order. The rows are read in one pass and
/// parsed on a thread each: parsing JSON is most of a start's restore time.
pub(crate) fn restore_all(
    mut db: Option<&mut CacheDb>,
    dir: &Path,
    plugins: &[&str],
    now_ms: i64,
) -> Vec<Restored> {
    let loaded: Vec<Option<Result<Option<Value>>>> = match db.as_deref() {
        None => plugins.iter().map(|_| None).collect(),
        Some(db) => {
            let rows: Vec<_> = plugins.iter().map(|p| db.rows(p)).collect();
            std::thread::scope(|scope| {
                let parsing: Vec<_> = rows
                    .into_iter()
                    .map(|rows| scope.spawn(move || rows.and_then(parse)))
                    .collect();
                parsing
                    .into_iter()
                    .map(|p| {
                        Some(
                            p.join()
                                .unwrap_or_else(|_| Err(anyhow::anyhow!("parsing panicked"))),
                        )
                    })
                    .collect()
            })
        }
    };
    plugins
        .iter()
        .zip(loaded)
        .map(|(plugin, loaded)| restore_one(db.as_deref_mut(), dir, plugin, loaded, now_ms))
        .collect()
}

/// One plugin's cache from its parsed rows (`None` without a database),
/// else from its old JSON file.
fn restore_one(
    db: Option<&mut CacheDb>,
    dir: &Path,
    plugin: &str,
    loaded: Option<Result<Option<Value>>>,
    now_ms: i64,
) -> Restored {
    let file = legacy_file(dir, plugin);
    let (Some(db), Some(loaded)) = (db, loaded) else {
        return match std::fs::read_to_string(&file) {
            Err(_) => Restored::Missing,
            Ok(text) => match serde_json::from_str(&text) {
                Ok(value) => Restored::File(value),
                Err(e) => Restored::Dropped(format!("{}: {e}", file.display())),
            },
        };
    };
    let mut dropped = None;
    match loaded {
        Ok(Some(value)) => return Restored::Stored(value),
        Ok(None) => {}
        Err(e) => {
            if let Err(e) = db.forget(plugin) {
                log::warn(format!(
                    "cache {plugin}: unreadable rows not removed: {e:#}"
                ));
            }
            dropped = Some(format!("{plugin} rows: {e:#}"));
        }
    }
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(_) => return dropped.map_or(Restored::Missing, Restored::Dropped),
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(value) => match db.save(plugin, &value, now_ms) {
            Ok(_) => {
                if let Err(e) = std::fs::rename(&file, renamed(&file, ".imported")) {
                    log::warn(format!("cache {} not renamed: {e}", file.display()));
                }
                Restored::Imported(value)
            }
            Err(e) => {
                log::warn(format!("cache {plugin} not imported: {e:#}"));
                Restored::File(value)
            }
        },
        Err(e) => {
            // Renamed, so the next start does not read it again.
            let _ = std::fs::rename(&file, renamed(&file, ".unreadable"));
            Restored::Dropped(format!("{}: {e}", file.display()))
        }
    }
}

/// Called with whether a save was stored.
pub(crate) type Done = Box<dyn FnOnce(bool) + Send>;

enum Job {
    Save {
        plugin: &'static str,
        value: Value,
        done: Option<Done>,
    },
    Finish,
}

/// The cache's writer thread. Saves queued while it writes are folded: only
/// each plugin's latest snapshot is stored.
pub(crate) struct Writer {
    sender: mpsc::Sender<Job>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl Writer {
    /// Write to `db`, or to JSON files in `dir` without one.
    pub fn start(db: Option<CacheDb>, dir: PathBuf) -> Self {
        let (sender, receiver) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("radar-cache".into())
            .spawn(move || write_loop(db, &dir, &receiver))
            .ok();
        Self {
            sender,
            worker: Mutex::new(worker),
        }
    }

    pub fn save(&self, plugin: &'static str, value: Value, done: Option<Done>) {
        let job = Job::Save {
            plugin,
            value,
            done,
        };
        if let Err(mpsc::SendError(job)) = self.sender.send(job) {
            log::warn(format!(
                "cache {plugin} not written: the cache writer stopped"
            ));
            if let Job::Save {
                done: Some(done), ..
            } = job
            {
                done(false);
            }
        }
    }

    /// Store what is queued and stop the thread.
    pub fn finish(&self) {
        let worker = self.worker.lock().ok().and_then(|mut w| w.take());
        if let Some(worker) = worker {
            let _ = self.sender.send(Job::Finish);
            let _ = worker.join();
        }
    }
}

fn write_loop(mut db: Option<CacheDb>, dir: &Path, receiver: &mpsc::Receiver<Job>) {
    while let Ok(job) = receiver.recv() {
        let mut finish = false;
        let mut pending: Vec<(&'static str, Value, Vec<Done>)> = Vec::new();
        for job in std::iter::once(job).chain(receiver.try_iter()) {
            match job {
                Job::Finish => finish = true,
                Job::Save {
                    plugin,
                    value,
                    done,
                } => {
                    let at = match pending.iter().position(|(p, ..)| *p == plugin) {
                        Some(at) => {
                            pending[at].1 = value;
                            at
                        }
                        None => {
                            pending.push((plugin, value, Vec::new()));
                            pending.len() - 1
                        }
                    };
                    pending[at].2.extend(done);
                }
            }
        }
        for (plugin, value, done) in pending {
            let stored = match db.as_mut() {
                Some(db) => db
                    .save(plugin, &value, crate::time::now_secs() * 1000)
                    .map(|_| ())
                    .map_err(|e| format!("{e:#}")),
                None => write_file(&legacy_file(dir, plugin), &value).map_err(|e| e.to_string()),
            };
            if let Err(e) = &stored {
                log::warn(format!("cache {plugin} not written: {e}"));
            }
            for done in done {
                done(stored.is_ok());
            }
        }
        if finish {
            break;
        }
    }
}

fn write_file(path: &Path, value: &Value) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, value.to_string())
}

static WRITER: OnceLock<Writer> = OnceLock::new();

/// Make `writer` the one [`save`] uses; the first one installed stays.
pub(crate) fn install(writer: Writer) {
    if let Err(extra) = WRITER.set(writer) {
        extra.finish();
    }
}

/// Queue a plugin's snapshot on the installed writer.
pub(crate) fn save(plugin: &'static str, value: Value, done: Option<Done>) {
    match WRITER.get() {
        Some(writer) => writer.save(plugin, value, done),
        None => {
            log::warn(format!("cache {plugin} not written: no cache writer"));
            if let Some(done) = done {
                done(false);
            }
        }
    }
}

/// Store the queued saves before radar exits.
pub(crate) fn finish() {
    if let Some(writer) = WRITER.get() {
        writer.finish();
    }
}

/// The folder of the old JSON caches: `$XDG_STATE_HOME/radar` when set, else
/// where they were written (on macOS `~/Library/Caches/radar`).
pub(crate) fn legacy_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(dirs::state_dir)
        .or_else(dirs::cache_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("radar")
}

/// `<state>/radar/cache.sqlite3`: beside the journal's folder, not in it.
pub(crate) fn default_path() -> Result<PathBuf> {
    let journal = crate::journal::Journal::default_path()?;
    let radar = journal
        .parent()
        .and_then(Path::parent)
        .context("no state folder for the cache database")?;
    Ok(radar.join("cache.sqlite3"))
}

/// radar's database for the caches, `None` (logged) where it cannot be opened.
pub(crate) fn open_default() -> Option<CacheDb> {
    let opened = default_path().and_then(|path| CacheDb::open(&path));
    match opened {
        Ok(db) => Some(db),
        Err(e) => {
            log::warn(format!(
                "cache database unavailable, JSON files instead: {e:#}"
            ));
            None
        }
    }
}

#[cfg(all(test, unix))]
mod tests;
