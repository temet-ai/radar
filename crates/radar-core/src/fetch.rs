//! How a part fetches an image, and the cache of what it fetched on disk.
//! Neutral: no drawing, no terminal type. The kit decides which images are
//! wanted and draws them (`ui::image`); a part supplies only a [`Fetch`].

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use crate::log;

/// An image's bytes, on their way.
pub type Bytes = Pin<Box<dyn Future<Output = anyhow::Result<Vec<u8>>> + Send>>;

/// How a part fetches an image: given its `src`, the bytes. The part keeps
/// its credentials inside; the kit never sees them.
pub type Fetch = Arc<dyn Fn(String) -> Bytes + Send + Sync>;

/// A [`Fetch`] from an async function or closure.
pub fn fetch<F, Fut>(f: F) -> Fetch
where
    F: Fn(String) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = anyhow::Result<Vec<u8>>> + Send + 'static,
{
    Arc::new(move |src| Box::pin(f(src)))
}

/// The disk cache's size cap.
pub const DISK_CAP: u64 = 200 * 1024 * 1024;

/// What the disk cache holds for `src`, if anything.
pub fn cached(dir: &Path, src: &str) -> Option<Vec<u8>> {
    std::fs::read(file_for(dir, src)).ok()
}

/// Where `src` is kept on disk: a name from its address, never the address.
pub(crate) fn file_for(dir: &Path, src: &str) -> PathBuf {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in src.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    dir.join(format!("{h:016x}"))
}

/// Write `bytes` for `src` into the cache (readable by you alone), then
/// let the oldest files go until the cache is under its cap.
pub fn keep(dir: &Path, src: &str, bytes: &[u8]) {
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    let path = file_for(dir, src);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options
        .open(&path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, bytes));
    if let Err(e) = written {
        log::warn(format!("image cache {} not written: {e}", path.display()));
        return;
    }
    prune(dir, DISK_CAP);
}

/// The oldest files in `dir` removed until what is left is at most `cap`
/// bytes.
pub(crate) fn prune(dir: &Path, cap: u64) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            m.is_file()
                .then(|| (m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort();
    for (_, len, path) in files {
        if total <= cap {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}
