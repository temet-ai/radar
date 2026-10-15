//! Transient ownership of dedicated MR detail/diff reads. Neither the request
//! sequence nor the configured-connection stamp belongs in persisted history.

use crate::{GitlabPlugin, write_context::ConnectionStamp};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Read {
    pub id: u64,
    pub connection: ConnectionStamp,
}

pub(crate) fn next(p: &mut GitlabPlugin) -> Read {
    p.next_page_read = p
        .next_page_read
        .checked_add(1)
        .expect("GitLab page read sequence exhausted");
    Read {
        id: p.next_page_read,
        connection: p.connection_stamp(),
    }
}

/// A reply consumes only its exact pending owner, never a newer request's slot.
pub(crate) fn detail(p: &mut GitlabPlugin, key: &str, read: Read) -> bool {
    if !p.is_current_connection(read.connection)
        || p.mr_page_reads.get(key) != Some(&read)
        || !p.mr_pages.contains_key(key)
    {
        return false;
    }
    p.mr_page_reads.remove(key);
    true
}

pub(crate) fn diff(p: &mut GitlabPlugin, key: &str, head: &str, page: usize, read: Read) -> bool {
    if !p.is_current_connection(read.connection) {
        return false;
    }
    let Some(diffs) = p.diffs.get_mut(key) else {
        return false;
    };
    if diffs.head != head || diffs.requests.get(&page) != Some(&read) {
        return false;
    }
    diffs.requests.remove(&page);
    true
}

#[cfg(test)]
mod tests;
