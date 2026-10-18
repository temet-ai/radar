//! Tickets, comments and links as radar holds them, read from Jira's JSON.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::adf;
use radar_core::ui::icon::Icon;
use radar_core::ui::rich::Doc;

/// Jira's three status categories, in radar's words.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    /// Being worked on: listed first.
    Doing,
    #[default]
    Todo,
    Done,
}

impl Category {
    /// From a status's `statusCategory.key`: `new`, `indeterminate`, `done`.
    pub fn from_key(key: &str) -> Self {
        match key {
            "indeterminate" => Self::Doing,
            "done" => Self::Done,
            _ => Self::Todo,
        }
    }

    /// The look word, also how it is said.
    pub const fn word(self) -> &'static str {
        match self {
            Self::Todo => "to do",
            Self::Doing => "doing",
            Self::Done => "done",
        }
    }

    /// So it reads without colour: `○ to do`, `◐ doing`, `● done`.
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Todo => "○",
            Self::Doing => "◐",
            Self::Done => "●",
        }
    }

    /// Its icon: to do, in progress, done.
    pub const fn icon(self) -> Icon {
        match self {
            Self::Todo => Icon::Todo,
            Self::Doing => Icon::InProgress,
            Self::Done => Icon::Done,
        }
    }
}

/// `✗ words` on the status line, or the error icon (in its own colour)
/// before them when icons are on.
pub fn failed_notice(words: &str) -> radar_core::Effect {
    let (icon, mark) = radar_core::ui::icon::plain_or(Icon::Error, "✗");
    radar_core::Effect::error(icon, format!("{mark}{words}"))
}

/// A category's icon to name when icons are on, else its old symbol and a
/// space to write before the words.
pub fn mark_or(c: Category) -> (Option<Icon>, String) {
    radar_core::ui::icon::plain_or(c.icon(), c.symbol())
}

/// A category's mark as text: its icon, or the old symbol when icons are off.
pub fn mark(c: Category) -> String {
    let lead = radar_core::ui::icon::plain(c.icon());
    if lead.is_empty() { format!("{} ", c.symbol()) } else { lead }
}

/// A ticket's kind as an icon, from its issue type's name (any case).
pub fn kind_icon(kind: &str, subtask: bool) -> Icon {
    if subtask {
        return Icon::Subtask;
    }
    match kind.trim().to_ascii_lowercase().as_str() {
        "epic" => Icon::Epic,
        "story" | "user story" => Icon::Story,
        "task" => Icon::Task,
        "bug" | "defect" => Icon::Bug,
        "sub-task" | "subtask" | "sub task" => Icon::Subtask,
        _ => Icon::Issue,
    }
}

/// A priority's name as an icon; none for a name it does not know.
pub fn priority_icon(priority: &str) -> Option<Icon> {
    Some(match priority.trim().to_ascii_lowercase().as_str() {
        "highest" | "blocker" | "critical" => Icon::PriorityHighest,
        "high" | "major" => Icon::PriorityHigh,
        "medium" | "normal" => Icon::PriorityMedium,
        "low" | "minor" => Icon::PriorityLow,
        "lowest" | "trivial" => Icon::PriorityLowest,
        _ => return None,
    })
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Parent {
    pub key: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub status: String,
    #[serde(default)]
    pub category: Category,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ticket {
    pub key: String,
    /// Jira's number for it, which its development panel is asked by.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    pub summary: String,
    /// The issue type's name: `Story`, `Bug`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub subtask: bool,
    /// The status's own name: `In Review`.
    pub status: String,
    pub category: Category,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub priority: String,
    /// Display name; empty when nobody is assigned.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub assignee: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub assignee_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reporter: String,
    pub created: String,
    pub updated: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<Parent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fix_versions: Vec<String>,
    /// The project's key and name.
    pub project: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub project_name: String,
    #[serde(default, skip_serializing_if = "Doc::is_empty", deserialize_with = "lenient")]
    pub description: Doc,
    /// The acceptance criteria, from the field set for its project.
    #[serde(default, skip_serializing_if = "Doc::is_empty", deserialize_with = "lenient")]
    pub ac: Doc,
}

/// A document from the cache; one kept in an older shape is read as empty
/// (the next sync fills it), so the rest of the cache still loads.
fn lenient<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Doc, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(serde_json::from_value(v).unwrap_or_default())
}

/// Who a site's token belongs to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Me {
    /// Missing in a cache from before it was kept: read as none, and the
    /// next sync asks `myself` again.
    #[serde(default)]
    pub account_id: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    pub author: String,
    pub created: String,
    #[serde(default, deserialize_with = "lenient")]
    pub body: Doc,
}

/// What a link points at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    /// Another ticket on the same site: its key, status and category.
    Ticket {
        key: String,
        status: String,
        category: Category,
    },
    /// A web page (a remote link).
    Web { url: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    /// How it relates, as Jira says it: `blocks`, `is blocked by`, `parent`,
    /// `child`, `web link`.
    pub relation: String,
    pub title: String,
    pub target: Target,
}

/// Everything a ticket's page shows beyond the list's fields.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Detail {
    pub ticket: Ticket,
    pub sprint: Option<String>,
    /// Parent, children, issue links, then web links.
    pub links: Vec<Link>,
    /// Newest first, as many as were paged in.
    pub comments: Vec<Comment>,
    /// How many there are in all.
    pub comments_total: usize,
    /// Merge requests Jira's development panel links to it.
    pub code: Vec<CodeLink>,
}

/// A move a ticket can make now: Jira's transition, and where it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub id: String,
    /// The transition's own name: `Start review`.
    pub name: String,
    /// The status it moves to: `In Review`.
    pub to: String,
    pub category: Category,
}

impl Transition {
    pub fn from_json(v: &Value) -> Option<Self> {
        Some(Self {
            id: v.get("id")?.as_str()?.to_string(),
            name: s(v, "/name"),
            to: s(v, "/to/name"),
            category: Category::from_key(&s(v, "/to/statusCategory/key")),
        })
    }
}

/// A kind of ticket a project can have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueType {
    pub id: String,
    pub name: String,
    pub subtask: bool,
}

/// A merge request Jira's development panel (`dev-status`) knows for a
/// ticket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeLink {
    /// As the code host names it: `!263`.
    pub id: String,
    pub title: String,
    pub url: String,
    /// `open`, `merged` or `declined`.
    pub status: String,
    pub repo: String,
    pub branch: String,
}

/// The merge requests in a `dev-status` detail reply.
pub fn code_links(reply: &Value) -> Vec<CodeLink> {
    let Some(details) = reply.get("detail").and_then(Value::as_array) else {
        return vec![];
    };
    details
        .iter()
        .flat_map(|d| d.get("pullRequests").and_then(Value::as_array).cloned().unwrap_or_default())
        .filter_map(|pr| {
            let url = pr.get("url")?.as_str()?.to_string();
            // GitLab's ids come as the project's path and the number
            // (`acme/team-a/api!187`): the number is enough beside the repo.
            let id = s(&pr, "/id");
            let id = match id.rsplit_once('!') {
                Some((_, n)) => format!("!{n}"),
                None => id,
            };
            Some(CodeLink {
                id,
                title: s(&pr, "/name"),
                status: s(&pr, "/status").to_lowercase(),
                repo: s(&pr, "/repositoryName"),
                branch: s(&pr, "/source/branch"),
                url,
            })
        })
        .collect()
}

fn s(v: &Value, ptr: &str) -> String {
    v.pointer(ptr)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn names(v: &Value, ptr: &str, field: &str) -> Vec<String> {
    v.pointer(ptr)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| match x {
                    Value::String(s) => Some(s.clone()),
                    o => o.get(field).and_then(Value::as_str).map(str::to_string),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The fields a list search asks for, plus each acceptance-criteria field.
pub fn list_fields(ac_fields: &[String]) -> Vec<String> {
    let mut f: Vec<String> = [
        "summary",
        "issuetype",
        "status",
        "priority",
        "assignee",
        "reporter",
        "created",
        "updated",
        "parent",
        "labels",
        "components",
        "fixVersions",
        "project",
        "description",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    for a in ac_fields {
        if !f.contains(a) {
            f.push(a.clone());
        }
    }
    f
}

impl Ticket {
    /// From an issue in a search or issue reply. `ac` picks the
    /// acceptance-criteria field by the issue's project.
    pub fn from_issue(issue: &Value, ac: &dyn Fn(&str) -> Option<String>) -> Option<Self> {
        let key = issue.get("key")?.as_str()?.to_string();
        let f = issue.get("fields").unwrap_or(&Value::Null);
        let project = s(f, "/project/key");
        // Its pictures and files, found by name among its attachments.
        let files = adf::attachments(f);
        let text = |v: &Value| {
            let mut d = adf::doc(v);
            adf::attach(&mut d, &files);
            d
        };
        let ac_lines = ac(&project)
            .and_then(|id| f.get(&id).map(text))
            .unwrap_or_default();
        let parent = f.get("parent").and_then(|p| {
            Some(Parent {
                key: p.get("key")?.as_str()?.to_string(),
                summary: s(p, "/fields/summary"),
                status: s(p, "/fields/status/name"),
                category: Category::from_key(&s(p, "/fields/status/statusCategory/key")),
            })
        });
        Some(Self {
            key,
            id: s(issue, "/id"),
            summary: s(f, "/summary"),
            kind: s(f, "/issuetype/name"),
            subtask: f
                .pointer("/issuetype/subtask")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            status: s(f, "/status/name"),
            category: Category::from_key(&s(f, "/status/statusCategory/key")),
            priority: s(f, "/priority/name"),
            assignee: s(f, "/assignee/displayName"),
            assignee_id: s(f, "/assignee/accountId"),
            reporter: s(f, "/reporter/displayName"),
            created: s(f, "/created"),
            updated: s(f, "/updated"),
            parent,
            labels: names(f, "/labels", "name"),
            components: names(f, "/components", "name"),
            fix_versions: names(f, "/fixVersions", "name"),
            project,
            project_name: s(f, "/project/name"),
            description: f.get("description").map(text).unwrap_or_default(),
            ac: ac_lines,
        })
    }

    /// Its kind's icon.
    pub fn icon(&self) -> Icon {
        kind_icon(&self.kind, self.subtask)
    }

    /// The look word of its type: `bug`, `story`, `task`, `epic`,
    /// `subtask`, or `other`.
    pub fn type_word(&self) -> &'static str {
        if self.subtask {
            return "subtask";
        }
        match self.kind.to_ascii_lowercase().as_str() {
            "bug" | "defect" => "bug",
            "story" | "user story" => "story",
            "task" => "task",
            "epic" => "epic",
            "sub-task" | "subtask" => "subtask",
            _ => "other",
        }
    }
}

impl Me {
    pub fn from_json(v: &Value) -> Self {
        Self {
            account_id: s(v, "/accountId"),
            name: s(v, "/displayName"),
        }
    }
}

impl Comment {
    pub fn from_json(v: &Value) -> Self {
        Self {
            author: s(v, "/author/displayName"),
            created: s(v, "/created"),
            body: v.get("body").map(adf::doc).unwrap_or_default(),
        }
    }
}

/// The sprint a ticket is in, by the field Jira names `Sprint`: the active
/// one, else the last.
pub fn sprint_of(issue: &Value) -> Option<String> {
    let names = issue.get("names")?.as_object()?;
    let id = names.iter().find(|(_, n)| n.as_str() == Some("Sprint"))?.0;
    let sprints = issue.pointer(&format!("/fields/{id}"))?.as_array()?;
    let pick = sprints
        .iter()
        .find(|s| s.get("state").and_then(Value::as_str) == Some("active"))
        .or_else(|| sprints.last())?;
    pick.get("name").and_then(Value::as_str).map(str::to_string)
}

/// A ticket's issue links, from its `issuelinks` field.
pub fn issue_links(issue: &Value) -> Vec<Link> {
    let Some(links) = issue.pointer("/fields/issuelinks").and_then(Value::as_array) else {
        return vec![];
    };
    links
        .iter()
        .filter_map(|l| {
            let (other, relation) = match (l.get("outwardIssue"), l.get("inwardIssue")) {
                (Some(o), _) => (o, s(l, "/type/outward")),
                (_, Some(i)) => (i, s(l, "/type/inward")),
                _ => return None,
            };
            Some(Link {
                relation,
                title: s(other, "/fields/summary"),
                target: Target::Ticket {
                    key: other.get("key")?.as_str()?.to_string(),
                    status: s(other, "/fields/status/name"),
                    category: Category::from_key(&s(other, "/fields/status/statusCategory/key")),
                },
            })
        })
        .collect()
}

/// The web links of a ticket, from `/remotelink`.
pub fn web_links(reply: &Value) -> Vec<Link> {
    reply
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| {
                    let url = r.pointer("/object/url")?.as_str()?.to_string();
                    let title = r
                        .pointer("/object/title")
                        .and_then(Value::as_str)
                        .filter(|t| !t.is_empty())
                        .unwrap_or(&url)
                        .to_string();
                    Some(Link {
                        relation: r
                            .get("relationship")
                            .and_then(Value::as_str)
                            .unwrap_or("web link")
                            .to_string(),
                        title,
                        target: Target::Web { url },
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A ticket as a link: its parent or a child.
pub fn ticket_link(relation: &str, t: &Ticket) -> Link {
    Link {
        relation: relation.into(),
        title: t.summary.clone(),
        target: Target::Ticket {
            key: t.key.clone(),
            status: t.status.clone(),
            category: t.category,
        },
    }
}

#[cfg(any(test, feature = "demo"))]
pub mod fixtures {
    use super::*;

    /// A ticket in project ABC, updated an hour before the samples' "now".
    pub fn ticket(key: &str, summary: &str, status: &str, category: Category) -> Ticket {
        Ticket {
            key: key.into(),
            summary: summary.into(),
            kind: "Story".into(),
            status: status.into(),
            category,
            priority: "Medium".into(),
            assignee: "Alice Example".into(),
            assignee_id: "acc-alice".into(),
            reporter: "Bob Example".into(),
            created: "2026-09-20T09:00:00.000+0000".into(),
            updated: "2026-10-06T06:00:00.000+0000".into(),
            project: key.split('-').next().unwrap_or_default().into(),
            project_name: format!("Acme {}", key.split('-').next().unwrap_or_default()),
            description: adf::doc(&Value::String(format!("What {key} is about."))),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    mod notice_severity_tests {
        use super::*;

        #[test]
        fn failed_notice_keeps_error_tone_with_icons_on_and_off() {
            use radar_core::ui::icon as packs;
            // Restore the thread-local pack even if this helper contract fails.
            struct Restore(packs::Icons);
            impl Drop for Restore {
                fn drop(&mut self) { packs::set_current(&self.0); }
            }
            let _restore = Restore(packs::current());
            for pack in ["off", "unicode"] {
                let (icons, warnings) = packs::resolve(pack, &[], &Default::default());
                assert!(warnings.is_empty());
                packs::set_current(&icons);
                let radar_core::Effect::Notice(message) = failed_notice("request refused") else {
                    panic!("failure must carry semantic error severity");
                };
                assert_eq!(message.tone, radar_core::view::chrome::NoticeTone::Error);
                assert_eq!(message.icon, (pack != "off").then_some(Icon::Error));
                assert_eq!(message.text, if pack == "off" { "✗ request refused" } else { "request refused" });
            }
        }
    }

    #[test]
    fn a_kind_and_a_priority_pick_their_icons() {
        assert_eq!(kind_icon("Epic", false), Icon::Epic);
        assert_eq!(kind_icon("STORY", false), Icon::Story);
        assert_eq!(kind_icon("task", false), Icon::Task);
        assert_eq!(kind_icon("Bug", false), Icon::Bug);
        assert_eq!(kind_icon("Sub-task", false), Icon::Subtask);
        assert_eq!(kind_icon("Subtask", false), Icon::Subtask);
        assert_eq!(kind_icon("Task", true), Icon::Subtask);
        assert_eq!(kind_icon("Spike", false), Icon::Issue);
        assert_eq!(priority_icon("Highest"), Some(Icon::PriorityHighest));
        assert_eq!(priority_icon("Blocker"), Some(Icon::PriorityHighest));
        assert_eq!(priority_icon("high"), Some(Icon::PriorityHigh));
        assert_eq!(priority_icon("Major"), Some(Icon::PriorityHigh));
        assert_eq!(priority_icon("Medium"), Some(Icon::PriorityMedium));
        assert_eq!(priority_icon("Minor"), Some(Icon::PriorityLow));
        assert_eq!(priority_icon("Trivial"), Some(Icon::PriorityLowest));
        assert_eq!(priority_icon(""), None);
        assert_eq!(Category::Doing.icon(), Icon::InProgress);
    }

    pub fn issue() -> Value {
        json!({
            "key": "ABC-12",
            "fields": {
                "summary": "Add a stats endpoint",
                "issuetype": { "name": "Story", "subtask": false },
                "status": { "name": "In Review", "statusCategory": { "key": "indeterminate" } },
                "priority": { "name": "High" },
                "assignee": { "displayName": "Alice Example", "accountId": "acc-alice" },
                "reporter": { "displayName": "Bob Example" },
                "created": "2026-09-20T09:00:00.000+0000",
                "updated": "2026-10-06T09:00:00.000+0000",
                "parent": { "key": "ABC-1", "fields": { "summary": "Dashboard" } },
                "labels": ["backend"],
                "components": [{ "name": "api" }],
                "fixVersions": [{ "name": "1.4" }],
                "project": { "key": "ABC", "name": "Acme Backend" },
                "description": { "type": "doc", "version": 1, "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "Count requests." }] }
                ]},
                "customfield_10050": { "type": "doc", "version": 1, "content": [
                    { "type": "bulletList", "content": [
                        { "type": "listItem", "content": [
                            { "type": "paragraph", "content": [{ "type": "text", "text": "GET /stats answers" }] }
                        ]}
                    ]}
                ]},
                "customfield_10020": [
                    { "name": "Sprint 40", "state": "closed" },
                    { "name": "Sprint 41", "state": "active" }
                ],
                "issuelinks": [
                    { "type": { "inward": "is blocked by", "outward": "blocks" },
                      "outwardIssue": { "key": "ABC-15", "fields": { "summary": "Ship it",
                        "status": { "name": "To Do", "statusCategory": { "key": "new" } } } } },
                    { "type": { "inward": "is blocked by", "outward": "blocks" },
                      "inwardIssue": { "key": "ABC-9", "fields": { "summary": "Schema",
                        "status": { "name": "Done", "statusCategory": { "key": "done" } } } } }
                ]
            },
            "names": { "customfield_10020": "Sprint", "customfield_10050": "Acceptance Criteria" }
        })
    }

    #[test]
    fn a_ticket_is_read_with_its_acceptance_criteria_from_the_projects_field() {
        let ac = |p: &str| (p == "ABC").then(|| "customfield_10050".to_string());
        let t = Ticket::from_issue(&issue(), &ac).unwrap();
        assert_eq!(t.key, "ABC-12");
        assert_eq!(t.category, Category::Doing);
        assert_eq!(t.status, "In Review");
        assert_eq!(t.parent.as_ref().unwrap().key, "ABC-1");
        assert_eq!(t.components, ["api"]);
        assert_eq!(t.fix_versions, ["1.4"]);
        assert_eq!(adf::plain(&issue()["fields"]["description"]), "Count requests.");
        assert!(radar_core::ui::rich::plain(&t.ac, 80).starts_with("• GET /stats answers"));
        assert_eq!(t.type_word(), "story");
        let none = |_: &str| None;
        assert!(Ticket::from_issue(&issue(), &none).unwrap().ac.is_empty());
        assert_eq!(Ticket::from_issue(&json!({"fields": {}}), &none), None);
    }

    #[test]
    fn sprint_and_links_are_read() {
        assert_eq!(sprint_of(&issue()).as_deref(), Some("Sprint 41"));
        let links = issue_links(&issue());
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].relation, "blocks");
        assert_eq!(links[1].relation, "is blocked by");
        assert!(matches!(&links[1].target, Target::Ticket { key, category: Category::Done, .. } if key == "ABC-9"));
        let web = web_links(&json!([
            { "relationship": "mentioned in", "object": { "url": "https://example.com/doc", "title": "Design" } },
            { "object": { "url": "https://example.com/x" } },
            { "object": {} }
        ]));
        assert_eq!(web.len(), 2);
        assert_eq!(web[1].relation, "web link");
        assert_eq!(web[1].title, "https://example.com/x");
    }

    #[test]
    fn categories_order_doing_first() {
        let mut c = vec![Category::Done, Category::Todo, Category::Doing];
        c.sort();
        assert_eq!(c, [Category::Doing, Category::Todo, Category::Done]);
        assert_eq!(Category::from_key("new"), Category::Todo);
    }
}
