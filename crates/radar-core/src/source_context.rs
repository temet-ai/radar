//! Owner-validated source locators for explicit local work annotations.
//! Display subjects are transient context. Only each owner's scoped identity may
//! be bound; related candidates never become durable relations automatically.

use crate::compose::Subject;
use crate::journal::SubjectKey;

pub const MAX_SOURCES: usize = 32;

#[derive(Clone)]
pub struct ResolvedSource {
    pub subject: Subject,
    pub key: SubjectKey,
}

pub struct ResolvedSources {
    pub id: u64,
    pub action_id: Option<crate::journal::ActionId>,
    pub sources: Vec<ResolvedSource>,
    pub omitted: usize,
}

/// A runner response returned only to the plugin which made the request.
pub struct WorkReply {
    pub id: u64,
    pub result: Result<crate::work_api::Response, crate::work_api::Error>,
}
