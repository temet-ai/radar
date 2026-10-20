//! Pictures in messages and descriptions. A part supplies a [`Fetch`] for the
//! sources its views name; the host fetches and caches the bytes
//! ([`crate::fetch`]). Ink draws the `▣ image name url` line until pictures
//! are drawn there (#14).

pub use crate::fetch::{Bytes, DISK_CAP, Fetch, fetch};

/// Whether images are drawn as pictures: not yet (#14).
pub fn on() -> bool {
    false
}
