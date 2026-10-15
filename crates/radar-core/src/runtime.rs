//! radar's JavaScript runtime: one executable (runtime/entry.mjs, built by
//! scripts/build-runtime) that runs the Ink renderer as `<runtime> ink` and
//! the Claude Code sidecar as `<runtime> claude-agent`, with no `node`
//! installed. A release binary carries it (the radar binary's build.rs) and
//! writes it into the cache folder when it starts; [`VAR`] then names it for
//! radar and the processes it starts, and is how a developer points radar at one
//! built elsewhere.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The variable naming the runtime's executable.
pub const VAR: &str = "RADAR_RUNTIME";

const FILE: &str = if cfg!(windows) { "radar-js.exe" } else { "radar-js" };

/// The runtime [`VAR`] names, when that file exists.
pub fn find(var: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    var(VAR).filter(|p| !p.is_empty()).map(PathBuf::from).filter(|p| p.is_file())
}

/// Writes an embedded runtime to `<cache>/radar/runtime/<id>/` unless it is
/// there already, and removes the ones older radar builds wrote. `id` changes
/// with the runtime's bytes, so the same id is the same file.
pub fn unpack(bytes: &[u8], id: &str, cache: &Path) -> io::Result<PathBuf> {
    let base = cache.join("radar").join("runtime");
    let dir = base.join(id);
    let path = dir.join(FILE);
    if fs::metadata(&path).is_ok_and(|m| m.len() == bytes.len() as u64) {
        return Ok(path);
    }
    fs::create_dir_all(&dir)?;
    // Written beside it and renamed: two radar starting at once never run half a file.
    let tmp = dir.join(format!("{FILE}.new.{}", std::process::id()));
    fs::write(&tmp, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755))?;
    }
    fs::rename(&tmp, &path)?;
    for entry in fs::read_dir(&base)?.flatten() {
        if entry.file_name() != id {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
    Ok(path)
}

/// [`unpack`] into the user's cache folder.
pub fn unpack_into_cache(bytes: &[u8], id: &str) -> io::Result<PathBuf> {
    let cache = crate::system::cache_home().ok_or_else(|| io::Error::other("no cache folder"))?;
    unpack(bytes, id, &cache)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_runtime_is_written_once_and_older_ones_are_removed() {
        let cache = std::env::temp_dir().join(format!("radar-runtime-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&cache);
        let old = unpack(b"old", "0.1-aaaa", &cache).unwrap();
        let new = unpack(b"newer", "0.1-bbbb", &cache).unwrap();
        assert_eq!(fs::read(&new).unwrap(), b"newer");
        assert!(!old.exists());
        // The same id again: the file there is kept, not rewritten.
        let again = unpack(b"other", "0.1-bbbb", &cache).unwrap();
        assert_eq!(fs::read(&again).unwrap(), b"newer");
        let var = |k: &str| (k == VAR).then(|| again.display().to_string());
        assert_eq!(find(&var), Some(again.clone()));
        let gone = |k: &str| (k == VAR).then(|| cache.join("missing").display().to_string());
        assert_eq!(find(&gone), None);
        fs::remove_dir_all(&cache).unwrap();
    }
}
