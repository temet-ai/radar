//! Provider-neutral context for agents. Sources stay with their owning parts;
//! the host joins cached facts and retains bounded snapshots for continuation.
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::hash::{Hash, Hasher};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::agent_context::{AgentContext, AgentContextRequest};
use crate::journal::SubjectKey;
use crate::{Plugin, Setup, Subject};

pub const TOOLS: &[&str] = &["search", "context", "read", "changes"];
pub fn arguments(tool: &str) -> &'static [&'static str] {
    match tool {
        "search" => &[
            "query",
            "kind",
            "project",
            "assignee",
            "cursor",
            "max_bytes",
        ],
        "context" => &["roots", "selection_ref", "depth", "cursor", "max_bytes"],
        "read" => &["refs", "sections", "snapshot_id", "cursor", "max_bytes"],
        "changes" => &["snapshot_id", "cursor", "max_bytes"],
        _ => &[],
    }
}
const MAX_ITEMS: usize = 256;
const MAX_SNAPSHOTS: usize = 32;
const MAX_SNAPSHOT_BYTES: usize = 8 * 1024 * 1024;
const MAX_STORE_BYTES: usize = 32 * 1024 * 1024;
const TTL: i64 = 3600;
/// Relations to what holds an item rather than what it is about.
const CONTAINERS: &[&str] = &["repository"];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Section {
    pub name: String,
    pub text: String,
    /// True when the provider itself has more than its cache holds.
    pub truncated: bool,
}
impl Section {
    pub fn new(name: &str, text: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            text: text.into(),
            truncated: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Document {
    pub context: AgentContext,
    pub sections: Vec<Section>,
    pub observed_at: Option<i64>,
    pub missing: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_revision: Option<String>,
}
impl Document {
    pub fn from_context(context: AgentContext) -> Self {
        let sections = context
            .fields
            .iter()
            .map(|f| Section {
                name: f.name.clone(),
                text: f.value.clone(),
                truncated: f.truncated,
            })
            .collect();
        Self {
            context,
            sections,
            observed_at: None,
            missing: vec![],
            source_revision: None,
        }
    }
    pub fn section(mut self, name: &str, text: impl Into<String>) -> Self {
        self.sections.retain(|s| s.name != name);
        self.sections.push(Section::new(name, text));
        self
    }
    pub fn revision(&self) -> String {
        if let Some(revision) = &self.source_revision {
            return revision.clone();
        }
        let mut h = std::collections::hash_map::DefaultHasher::new();
        // Observation time alone is not a content change.
        serde_json::to_string(&(&self.context, &self.sections, &self.missing))
            .unwrap_or_default()
            .hash(&mut h);
        format!("{:016x}", h.finish())
    }
}

/// An owner's cached relationship. Unknown basis is kept unknown, not promoted
/// to an explicit provider link by the host.
pub struct Related {
    pub subject: Subject,
    pub relation: String,
    pub basis: String,
    pub evidence: String,
}
impl Related {
    pub fn new(subject: Subject, relation: &str, basis: &str, evidence: &str) -> Self {
        Self {
            subject,
            relation: relation.into(),
            basis: basis.into(),
            evidence: evidence.into(),
        }
    }
}

pub struct Call {
    pub id: u64,
    pub tool: String,
    pub arguments: Value,
}

#[derive(Clone, Debug, Default)]
pub struct Bundle {
    pub contexts: Vec<AgentContext>,
    pub snapshot_id: Option<String>,
    pub missing: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Query {
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub roots: Vec<String>,
    pub selection_ref: Option<String>,
    #[serde(default)]
    pub refs: Vec<String>,
    #[serde(default)]
    pub sections: Vec<String>,
    pub kind: Option<String>,
    pub project: Option<String>,
    pub assignee: Option<String>,
    pub snapshot_id: Option<String>,
    pub cursor: Option<String>,
    pub max_bytes: Option<usize>,
    pub depth: Option<usize>,
}

#[derive(Clone, Serialize)]
struct Edge {
    from: String,
    to: String,
    relation: String,
    basis: String,
    evidence: String,
}

struct Snapshot {
    created: i64,
    depth: usize,
    roots: Vec<String>,
    documents: Vec<Document>,
    edges: Vec<Edge>,
    missing: Vec<String>,
    omitted: usize,
    bytes: usize,
}

#[derive(Default)]
pub struct Service {
    scope: Option<u64>,
    snapshots: BTreeMap<String, Snapshot>,
}

pub fn reference(key: &SubjectKey) -> String {
    format!(
        "radar:{}",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(key).expect("subject serializes"))
    )
}

fn owner(plugins: &[Box<dyn Plugin>], subject: &Subject) -> Option<(usize, SubjectKey)> {
    plugins.iter().enumerate().find_map(|(i, p)| {
        if p.set_up() != Setup::Ready || !p.owns().iter().any(|o| o.kind == subject.kind) {
            return None;
        }
        p.journal_subject(subject)
            .filter(|s| p.subject_from_journal(&s.key).is_some())
            .map(|s| (i, s.key))
    })
}

pub fn resolve(
    plugins: &[Box<dyn Plugin>],
    value: &str,
) -> Result<(usize, Subject, SubjectKey), String> {
    if let Some(encoded) = value.trim().strip_prefix("radar:") {
        // A model copying a long ref back sometimes breaks it with a space.
        let encoded: String = encoded.chars().filter(|c| !c.is_whitespace()).collect();
        let key: SubjectKey = URL_SAFE_NO_PAD
            .decode(encoded)
            .ok()
            .and_then(|v| serde_json::from_slice(&v).ok())
            .ok_or("invalid radar ref")?;
        for (i, p) in plugins.iter().enumerate() {
            if p.set_up() != Setup::Ready {
                continue;
            }
            if let Some(s) = p.subject_from_journal(&key)
                && p.journal_subject(&s).is_some_and(|r| r.key == key)
            {
                return Ok((i, s, key));
            }
        }
        return Err("source is unavailable in the current configured scope".into());
    }
    let mut found = Vec::new();
    for p in plugins.iter().filter(|p| p.set_up() == Setup::Ready) {
        for (_, s) in p.recognise(value) {
            if let Some((i, key)) = owner(plugins, &s)
                && !found.iter().any(|(_, _, k)| k == &key)
            {
                found.push((i, s, key));
            }
        }
    }
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err("no configured source recognizes this ref; use search".into()),
        _ => Err("ambiguous ref; use a scoped ref from search".into()),
    }
}

pub fn catalog(plugins: &[Box<dyn Plugin>]) -> Vec<(Subject, SubjectKey)> {
    let mut seen = BTreeSet::new();
    let mut out = vec![];
    for p in plugins.iter().filter(|p| p.set_up() == Setup::Ready) {
        for s in p.agent_subjects() {
            if let Some((_, key)) = owner(plugins, &s)
                && seen.insert(reference(&key))
            {
                out.push((s, key));
            }
        }
    }
    out
}

fn document(
    plugins: &mut [Box<dyn Plugin>],
    i: usize,
    subject: &Subject,
    key: SubjectKey,
) -> Document {
    plugins[i]
        .agent_document(&AgentContextRequest {
            subject: key.clone(),
        })
        .unwrap_or_else(|| {
            let mut d = Document::from_context(
                AgentContext::new(&subject.kind, key, &subject.label).link(subject.url.clone()),
            );
            d.missing
                .push("source content is not cached or this owner does not expose it yet".into());
            d
        })
}

impl Service {
    pub fn clear(&mut self) {
        self.snapshots.clear();
    }
    pub fn brief(&self, id: &str) -> Bundle {
        match self.snapshots.get(id) {
            Some(s) => Bundle {
                contexts: s.documents.iter().map(|d| d.context.clone()).collect(),
                snapshot_id: Some(id.into()),
                missing: s
                    .missing
                    .iter()
                    .cloned()
                    .chain(s.documents.iter().flat_map(|d| {
                        d.missing
                            .iter()
                            .map(|m| format!("{}: {m}", d.context.title))
                    }))
                    .collect(),
            },
            None => Bundle::default(),
        }
    }

    pub fn execute(
        &mut self,
        plugins: &mut [Box<dyn Plugin>],
        tool: &str,
        arguments: Value,
        now: i64,
    ) -> Result<Value, String> {
        // Only parts that own subjects decide what a ref resolves to. Another
        // part's settings (an allow rule added from the dock) keep snapshots.
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        for p in plugins.iter().filter(|p| !p.owns().is_empty()) {
            p.id().hash(&mut hash);
            p.agent_scope().hash(&mut hash);
            p.config().map(|v| v.to_string()).hash(&mut hash);
        }
        let scope = hash.finish();
        if self.scope != Some(scope) {
            self.snapshots.clear();
            self.scope = Some(scope);
        }
        if let Some(fields) = arguments.as_object() {
            for field in fields.keys() {
                if !self::arguments(tool).contains(&field.as_str()) {
                    return Err(format!("{tool} does not accept {field}"));
                }
            }
        }
        let q: Query =
            serde_json::from_value(arguments).map_err(|e| format!("invalid query: {e}"))?;
        if !TOOLS.contains(&tool) {
            return Err("unknown context tool".into());
        }
        self.snapshots
            .retain(|_, s| now.saturating_sub(s.created) < TTL);
        let budget = q.max_bytes.unwrap_or(24 * 1024).clamp(1024, 256 * 1024);
        if let Some(cursor) = &q.cursor {
            return self.continue_page(plugins, tool, cursor, budget);
        }
        if tool == "read" {
            return self.read(plugins, &q, budget, now);
        }
        if tool == "changes" {
            return self.changes(plugins, &q, now, budget);
        }
        let mut snapshot = Snapshot {
            created: now,
            depth: if tool == "search" {
                0
            } else {
                q.depth.unwrap_or(2).min(4)
            },
            roots: vec![],
            documents: vec![],
            edges: vec![],
            missing: vec![],
            omitted: 0,
            bytes: 0,
        };
        if tool == "search" {
            let needle = q.query.to_lowercase();
            // Exact identities and URLs are useful even before a list is cached.
            let exact = resolve(plugins, &q.query).ok().map(|(_, _, key)| key);
            let mut subjects = catalog(plugins);
            if !q.query.is_empty()
                && let Ok((_, s, k)) = resolve(plugins, &q.query)
                && !subjects.iter().any(|(_, key)| key == &k)
            {
                subjects.insert(0, (s, k));
            }
            for (s, key) in subjects {
                if q.kind.as_ref().is_some_and(|k| &key.kind != k) {
                    continue;
                }
                if q.project
                    .as_ref()
                    .is_some_and(|p| !key.id.contains(p) && !s.fields.values().any(|f| f == p))
                {
                    continue;
                }
                let words = format!(
                    "{} {} {} {}",
                    s.label,
                    key.id,
                    s.url.as_deref().unwrap_or_default(),
                    s.fields.values().cloned().collect::<Vec<_>>().join(" ")
                )
                .to_lowercase();
                let Some((i, _)) = owner(plugins, &s) else {
                    continue;
                };
                let d = document(plugins, i, &s, key.clone());
                if !needle.is_empty()
                    && exact.as_ref() != Some(&key)
                    && !words.contains(&needle)
                    && !d
                        .sections
                        .iter()
                        .any(|s| s.text.to_lowercase().contains(&needle))
                {
                    continue;
                }
                if let Some(want) = &q.assignee {
                    let want = if want == "self" {
                        plugins[i]
                            .agent_identity(&d.context.subject)
                            .unwrap_or_default()
                    } else {
                        want.clone()
                    };
                    if want.is_empty()
                        || !d.context.fields.iter().any(|f| {
                            (f.name == "assignee" || f.name == "assignee_id")
                                && f.value.eq_ignore_ascii_case(&want)
                        })
                    {
                        continue;
                    }
                }
                let r = reference(&d.context.subject);
                if Self::add(&mut snapshot, d) {
                    snapshot.roots.push(r);
                }
            }
            snapshot.missing.push("search covers cached subjects only; unqueried sources and older items are not counted".into());
        } else {
            let roots = if let Some(id) = &q.selection_ref {
                self.get(id)?.roots.clone()
            } else {
                q.roots.clone()
            };
            if roots.is_empty() {
                return Err("context needs roots or a selection_ref from search".into());
            }
            if roots.len() > MAX_ITEMS {
                return Err("too many roots; use a smaller selection".into());
            }
            let depth = q.depth.unwrap_or(2).min(4);
            let mut queue: VecDeque<_> = roots.iter().map(|r| (r.clone(), 0usize)).collect();
            snapshot.roots = roots;
            let mut seen = BTreeSet::new();
            while let Some((r, level)) = queue.pop_front() {
                let (i, s, key) = match resolve(plugins, &r) {
                    Ok(v) => v,
                    Err(e) => {
                        if snapshot.missing.len() < 64 {
                            snapshot.missing.push(format!("{r}: {e}"));
                        }
                        continue;
                    }
                };
                let from = reference(&key);
                if !seen.insert(from.clone()) {
                    continue;
                }
                if snapshot.documents.len() >= MAX_ITEMS {
                    snapshot.omitted += 1;
                    continue;
                }
                let d = document(plugins, i, &s, key);
                let mut related: Vec<Related> = plugins
                    .iter()
                    .filter(|p| p.set_up() == Setup::Ready)
                    .flat_map(|p| p.agent_related(&s))
                    .collect();
                // Owners recognize links and item keys in authored fields; this is
                // evidence of a mention, never a claim that the link authorizes work.
                for section in &d.sections {
                    for p in plugins.iter().filter(|p| p.set_up() == Setup::Ready) {
                        for (_, target) in p.recognise(&section.text) {
                            related.push(Related::new(
                                target,
                                "mentions",
                                "observed",
                                &section.name,
                            ));
                        }
                    }
                }
                for edge in related {
                    let Some((_, key)) = owner(plugins, &edge.subject) else {
                        continue;
                    };
                    let to = reference(&key);
                    if to == from {
                        continue;
                    }
                    // A container is listed but not walked: through an MR's
                    // repository lies every other MR and every message about them.
                    let next = if CONTAINERS.contains(&edge.relation.as_str()) { depth } else { level + 1 };
                    if snapshot.edges.len() < 2048
                        && !snapshot.edges.iter().any(|e| {
                            e.from == from
                                && e.to == to
                                && e.relation == edge.relation
                                && e.evidence == edge.evidence
                        })
                    {
                        snapshot.edges.push(Edge {
                            from: from.clone(),
                            to: to.clone(),
                            relation: edge.relation,
                            basis: edge.basis,
                            evidence: edge.evidence,
                        });
                    }
                    if level < depth && !seen.contains(&to) && !queue.iter().any(|(r, _)| r == &to)
                    {
                        queue.push_back((to, next));
                    } else if level >= depth && !seen.contains(&to) {
                        snapshot.omitted += 1;
                    }
                }
                Self::add(&mut snapshot, d);
            }
        }
        if snapshot.edges.len() == 2048 {
            snapshot
                .missing
                .push("relationship limit reached; narrow the roots or depth".into());
        }
        let id = self.keep(snapshot);
        self.page(plugins, tool, &id, 0, budget)
    }

    fn add(s: &mut Snapshot, mut d: Document) -> bool {
        fn clean(s: &mut String) {
            *s = crate::agent_text::redact_secrets(&crate::log::redact_registered(s)).0;
        }
        clean(&mut d.context.title);
        if let Some(url) = &mut d.context.link {
            clean(url);
        }
        for f in &mut d.context.fields {
            clean(&mut f.value);
        }
        for section in &mut d.sections {
            clean(&mut section.text);
        }
        for gap in &mut d.missing {
            clean(gap);
        }
        let size = serde_json::to_vec(&d).map_or(0, |v| v.len());
        if s.documents.len() >= MAX_ITEMS || s.bytes + size > MAX_SNAPSHOT_BYTES {
            s.omitted += 1;
            if s.missing.len() < 32 {
                s.missing.push(format!(
                    "{} exceeds this snapshot's capacity; narrow the query",
                    reference(&d.context.subject)
                ));
            }
            false
        } else {
            s.bytes += size;
            s.documents.push(d);
            true
        }
    }
    fn keep(&mut self, snapshot: Snapshot) -> String {
        while self.snapshots.len() >= MAX_SNAPSHOTS
            || self.snapshots.values().map(|s| s.bytes).sum::<usize>() + snapshot.bytes
                > MAX_STORE_BYTES
        {
            let Some(old) = self
                .snapshots
                .iter()
                .min_by_key(|(_, s)| s.created)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            self.snapshots.remove(&old);
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.snapshots.insert(id.clone(), snapshot);
        id
    }
    fn get(&self, id: &str) -> Result<&Snapshot, String> {
        self.snapshots
            .get(id)
            .ok_or_else(|| "snapshot expired or unknown; repeat the original query".into())
    }
    fn page(
        &self,
        plugins: &[Box<dyn Plugin>],
        tool: &str,
        id: &str,
        offset: usize,
        budget: usize,
    ) -> Result<Value, String> {
        let s = self.get(id)?;
        if offset > s.documents.len() {
            return Err("invalid cursor offset".into());
        }
        let mut items = vec![];
        let mut bytes = 0;
        for d in s.documents.iter().skip(offset) {
            let r = reference(&d.context.subject);
            if resolve(plugins, &r).is_err() {
                return Err("snapshot scope changed; repeat the original query".into());
            }
            let fields: Vec<_> = d
                .context
                .fields
                .iter()
                .map(|f| {
                    let mut value = f.value.clone();
                    let cut = crate::agent_context::cap_text(&mut value, 512);
                    json!({"name":f.name,"value":value,"truncated":cut || f.truncated})
                })
                .collect();
            let item = json!({"ref":r,"subject":d.context.subject,"kind":d.context.kind,"title":d.context.title,"url":d.context.link,
                "revision":d.revision(),"observed_at":d.observed_at,"fields":fields,"sections":d.sections.iter().map(|s| json!({"name":s.name,"bytes":s.text.len(),"truncated":s.truncated})).collect::<Vec<_>>(),"missing":d.missing});
            let size = serde_json::to_vec(&item).map_or(0, |v| v.len());
            if !items.is_empty() && bytes + size > budget {
                break;
            }
            bytes += size;
            items.push(item);
        }
        let end = offset + items.len();
        let refs: BTreeSet<_> = items.iter().filter_map(|i| i["ref"].as_str()).collect();
        let edges: Vec<_> = s
            .edges
            .iter()
            .filter(|e| refs.contains(e.from.as_str()))
            .collect();
        Ok(
            json!({"schema_version":1,"snapshot_id":id,"selection_ref":id,"as_of":s.created,"items":items,"relations":edges,
            "known_items":s.documents.len(),"omitted":s.omitted,"coverage":"cached","missing":s.missing,
            "next_cursor":(end < s.documents.len()).then(|| format!("{tool}:{id}:{end}"))}),
        )
    }
    fn continue_page(
        &self,
        plugins: &[Box<dyn Plugin>],
        tool: &str,
        cursor: &str,
        budget: usize,
    ) -> Result<Value, String> {
        let parts: Vec<_> = cursor.split(':').collect();
        if parts.len() != 3 || parts[0] != tool {
            return Err("cursor belongs to another query".into());
        }
        let offset = parts[2].parse().map_err(|_| "invalid cursor")?;
        if tool == "read" {
            self.read_page(plugins, parts[1], offset, budget)
        } else {
            self.page(plugins, tool, parts[1], offset, budget)
        }
    }
    fn read(
        &mut self,
        plugins: &mut [Box<dyn Plugin>],
        q: &Query,
        budget: usize,
        now: i64,
    ) -> Result<Value, String> {
        if q.refs.is_empty() || q.refs.len() > 32 {
            return Err("read needs 1 to 32 refs".into());
        }
        let mut s = Snapshot {
            created: now,
            depth: 0,
            roots: q.refs.clone(),
            documents: vec![],
            edges: vec![],
            missing: vec![],
            omitted: 0,
            bytes: 0,
        };
        for r in &q.refs {
            let (i, subject, key) = resolve(plugins, r)?;
            let mut d = if let Some(id) = &q.snapshot_id {
                self.get(id)?
                    .documents
                    .iter()
                    .find(|d| d.context.subject == key)
                    .cloned()
                    .ok_or("ref is absent from that snapshot")?
            } else {
                document(plugins, i, &subject, key)
            };
            d.source_revision = Some(d.revision());
            if !q.sections.is_empty() {
                for name in &q.sections {
                    if !d.sections.iter().any(|s| &s.name == name) {
                        d.missing.push(format!("section {name} is unavailable"));
                    }
                }
                d.sections.retain(|s| q.sections.contains(&s.name));
            }
            Self::add(&mut s, d);
        }
        let id = self.keep(s);
        self.read_page(plugins, &id, 0, budget)
    }
    fn read_page(
        &self,
        plugins: &[Box<dyn Plugin>],
        id: &str,
        offset: usize,
        budget: usize,
    ) -> Result<Value, String> {
        let s = self.get(id)?;
        let mut all = String::new();
        let mut sources = vec![];
        for d in &s.documents {
            let r = reference(&d.context.subject);
            if resolve(plugins, &r).is_err() {
                return Err("source scope changed; repeat the query".into());
            }
            all.push_str(&format!("\n# {}\nSource: {}\n", d.context.title, r));
            for section in &d.sections {
                all.push_str(&format!("\n## {}\n{}\n", section.name, section.text));
                if section.truncated {
                    all.push_str("[The provider cache contains only part of this section.]\n");
                }
            }
            sources.push(json!({"ref":r,"revision":d.revision(),"observed_at":d.observed_at,"missing":d.missing}));
        }
        if offset > all.len() || !all.is_char_boundary(offset) {
            return Err("invalid read cursor".into());
        }
        let mut end = (offset + budget).min(all.len());
        while !all.is_char_boundary(end) {
            end -= 1;
        }
        Ok(
            json!({"schema_version":1,"snapshot_id":id,"sources":sources,"text":&all[offset..end],"bytes":end-offset,
            "total_bytes":all.len(),"omitted":s.omitted,"missing":s.missing,"next_cursor":(end < all.len()).then(|| format!("read:{id}:{end}"))}),
        )
    }
    fn changes(
        &mut self,
        plugins: &mut [Box<dyn Plugin>],
        q: &Query,
        now: i64,
        budget: usize,
    ) -> Result<Value, String> {
        let old_id = q
            .snapshot_id
            .as_deref()
            .ok_or("changes needs snapshot_id")?;
        let old = self.get(old_id)?;
        let before: BTreeMap<_, _> = old
            .documents
            .iter()
            .map(|d| (reference(&d.context.subject), d.revision()))
            .collect();
        let roots = old.roots.clone();
        let depth = old.depth;
        let mut result = self.execute(
            plugins,
            "context",
            json!({"roots":roots,"depth":depth,"max_bytes":budget}),
            now,
        )?;
        let new_id = result["snapshot_id"].as_str().ok_or("missing snapshot")?;
        let after: BTreeMap<_, _> = self
            .get(new_id)?
            .documents
            .iter()
            .map(|d| (reference(&d.context.subject), d.revision()))
            .collect();
        result["changed_refs"] = json!(
            after
                .iter()
                .filter(|(r, v)| before.get(*r) != Some(*v))
                .map(|(r, _)| r)
                .collect::<Vec<_>>()
        );
        if let Some(cursor) = result["next_cursor"].as_str() {
            result["next_cursor"] = json!(cursor.replacen("context:", "changes:", 1));
        }
        result["unavailable_refs"] = json!(
            before
                .keys()
                .filter(|r| !after.contains_key(*r))
                .collect::<Vec<_>>()
        );
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
