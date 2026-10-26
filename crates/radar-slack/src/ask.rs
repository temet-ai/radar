//! Asking for review: the team's message drafted from the template in
//! settings, shown exactly as it will post and where, and posted only after
//! a yes. The user group is mentioned only in the review channel.

use std::collections::BTreeMap;

use radar_core::Subject;
use radar_core::input::TextInput;

use crate::client::UserGroup;
use crate::config::Settings;

/// A merge request in a draft: what it is, its link, and the words after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// The subject's id, so the ask can be followed on its row.
    pub id: String,
    pub url: String,
    /// `api !263`.
    pub label: String,
    pub words: String,
    /// Ticket keys and their URLs, supplied by the part that owns them.
    pub tickets: Vec<(String, String)>,
    /// What else the part said about it, for the template's placeholders:
    /// `project`, `iid`, `branch`, `target`, `author`.
    pub facts: BTreeMap<String, String>,
}

/// The facts of a merge request a template can name, as its subject holds them.
const FACTS: [&str; 5] = ["project", "iid", "branch", "target", "author"];

/// What each merge request can be named by, in the template and in the
/// line for each one.
pub const ITEM_PLACEHOLDERS: [&str; 10] =
    ["link", "url", "repo", "project", "iid", "title", "branch", "target", "author", "tickets"];

impl Item {
    /// From a merge request subject: its link, its label and its title.
    pub fn of(s: &Subject) -> Option<Self> {
        Some(Self {
            id: s.id.clone(),
            url: s.url.clone()?,
            label: s.label.clone(),
            words: s.field("title").unwrap_or_default().trim().to_string(),
            tickets: s.related.iter().filter(|s| s.is("ticket"))
                .filter_map(|s| s.url.as_ref().map(|url| (s.id.clone(), url.clone())))
                .collect(),
            facts: FACTS.iter()
                .filter_map(|f| s.field(f).filter(|v| !v.trim().is_empty())
                    .map(|v| ((*f).to_string(), v.trim().to_string())))
                .collect(),
        })
    }

    /// The repo's own name: the end of its project path, else the start of
    /// its label (`api` of `api !263`).
    pub fn repo(&self) -> &str {
        match self.facts.get("project") {
            Some(project) => project.rsplit('/').next().unwrap_or(project),
            None => self.label.split(" !").next().unwrap_or(&self.label),
        }
    }

    /// One placeholder's value, as Slack text. `link` is the link's text;
    /// `tickets_shown` leaves out of `{title}` the tickets `{tickets}` lists.
    fn value(&self, name: &str, link: &str, tickets_shown: bool) -> Option<String> {
        Some(match name {
            "link" => format!("<{}|{}>", self.url, escape(link)),
            "url" => self.url.clone(),
            "repo" => escape(self.repo()),
            "title" => linked_words(self, !tickets_shown),
            "tickets" => self.tickets.iter()
                .filter(|(key, url)| !key.is_empty() && !url.is_empty())
                .map(|(key, url)| format!("<{}|{}>", escape(url), escape(key)))
                .collect::<Vec<_>>()
                .join(", "),
            f if FACTS.contains(&f) => escape(self.facts.get(f).map_or("", String::as_str)),
            _ => return None,
        })
    }
}

/// A review ask being written.
#[derive(Debug, Default)]
pub struct Draft {
    pub(crate) stamp: crate::ask_context::DraftStamp,
    pub items: Vec<Item>,
    /// The alias of the channel it goes to.
    pub channel: String,
    /// The item the cursor is on.
    pub row: usize,
    /// The words of `row` being edited.
    pub edit: Option<TextInput>,
    /// The operation sending this draft now.
    pub sending: Option<u64>,
    /// What Slack said when it would not take it.
    pub error: Option<String>,
}

impl Draft {
    pub(crate) fn changed(&mut self) {
        self.stamp.changed();
    }

    pub fn new(cfg: &Settings, item: Item) -> Self {
        Self {
            items: vec![item],
            channel: cfg.review.channel.clone(),
            ..Self::default()
        }
    }

    /// Add a merge request, unless it is in already. True when added.
    pub fn add(&mut self, item: Item) -> bool {
        if self.items.iter().any(|i| i.id == item.id) {
            return false;
        }
        self.items.push(item);
        self.changed();
        true
    }

    /// Whether a user group is mentioned: only in the review channel, and
    /// only when the template names one.
    pub fn mentions_group(&self, cfg: &Settings) -> bool {
        let names_one = placeholders(&cfg.review.template)
            .iter()
            .any(|p| (*p == "group" && !cfg.review.group.is_empty()) || p.starts_with('@'));
        names_one && !self.channel.is_empty() && self.channel == cfg.review.channel
    }

    /// Why it cannot be posted now, in words.
    pub fn blocked(&self, cfg: &Settings, groups: &[UserGroup]) -> Option<String> {
        if self.items.is_empty() {
            return Some("no merge request in it".into());
        }
        if self.channel.is_empty() {
            return Some("no channel: set the review channel under review".into());
        }
        if cfg.channel(&self.channel).is_none() {
            return Some(format!("no channel called {} any more", self.channel));
        }
        if let Err(why) = check_template(&cfg.review.template, groups) {
            return Some(format!("the template: {why}"));
        }
        if let Err(why) = check_line(&cfg.review.line) {
            return Some(format!("each merge request: {why}"));
        }
        if self.sending.is_some() {
            return Some("being sent".into());
        }
        None
    }
}

/// Slack's text escapes: `&`, `<` and `>` in words someone typed.
pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// The user groups a review ask mentions, by handle: the one picked under
/// user group for `{group}`, and each `{@handle}` Slack has.
pub fn mentioned(cfg: &Settings, groups: &[UserGroup]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in placeholders(&cfg.review.template) {
        let handle = match p {
            "group" if !cfg.review.group.is_empty() => groups
                .iter()
                .find(|g| g.id == cfg.review.group)
                .map(|g| g.handle.clone())
                .or_else(|| (!cfg.review.handle.is_empty()).then(|| cfg.review.handle.clone()))
                .unwrap_or_else(|| cfg.review.group.clone()),
            _ => match p.strip_prefix('@').and_then(|h| group_by_handle(groups, h)) {
                Some(g) => g.handle.clone(),
                None => continue,
            },
        };
        if !out.contains(&handle) {
            out.push(handle);
        }
    }
    out
}

/// The `{…}` placeholders in a template, without their braces, in order.
pub fn placeholders(template: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else { break };
        out.push(&after[..close]);
        rest = &after[close + 1..];
    }
    out
}

/// The user group a handle names (`review-squad` or `@review-squad`), as
/// Slack listed them.
pub fn group_by_handle<'a>(groups: &'a [UserGroup], handle: &str) -> Option<&'a UserGroup> {
    let handle = handle.trim().trim_start_matches('@');
    groups.iter().find(|g| g.handle.eq_ignore_ascii_case(handle))
}

/// Whether a template can be sent as it is: it has `{mrs}` or `{link}`;
/// every placeholder is `{mrs}`, `{count}`, `{group}`, `{@handle}` of a user
/// group Slack has, or one of [`ITEM_PLACEHOLDERS`]; and no group is named
/// as bare `@handle`, which Slack would post as plain words that ping no
/// one. `Ok` says what it mentions.
pub fn check_template(template: &str, groups: &[UserGroup]) -> Result<String, String> {
    let names = placeholders(template);
    if !names.iter().any(|p| *p == "mrs" || *p == "link") {
        return Err("needs {mrs} or {link}: where the merge requests go".into());
    }
    let mut mentions = Vec::new();
    for p in names {
        match p {
            "mrs" | "count" => {}
            p if ITEM_PLACEHOLDERS.contains(&p) => {}
            "group" => mentions.push("the group picked under user group".to_string()),
            _ => match p.strip_prefix('@') {
                Some(h) if groups.is_empty() => {
                    return Err(format!("{{@{h}}}: Slack's user groups are not read yet; pick the group under user group, or wait for a read"));
                }
                Some(h) => match group_by_handle(groups, h) {
                    Some(g) => mentions.push(format!("@{}", g.handle)),
                    None => return Err(format!("{{@{h}}}: Slack has no user group with that handle")),
                },
                None => return Err(format!(
                    "{{{p}}} is not a placeholder: {{mrs}}, {{count}}, {{group}}, {{@handle}}, {}",
                    item_placeholders()
                )),
            },
        }
    }
    for g in groups {
        let bare = format!("@{}", g.handle);
        let plain = template.replace(&format!("{{{bare}}}"), "");
        if crate::model::has_word(&plain, &bare) {
            return Err(format!("{bare} would post as plain words that ping no one: write {{{bare}}}, or {{group}} for the group picked under user group"));
        }
    }
    Ok(if mentions.is_empty() {
        "mentions no one".into()
    } else {
        format!("mentions {} (in the review channel)", mentions.join(" and "))
    })
}

/// Whether the line for each merge request can be sent: it links the merge
/// request with `{link}`, and names it only by [`ITEM_PLACEHOLDERS`].
pub fn check_line(line: &str) -> Result<String, String> {
    let names = placeholders(line);
    if !names.contains(&"link") {
        return Err("needs {link}: the merge request's link".into());
    }
    for p in &names {
        if !ITEM_PLACEHOLDERS.contains(p) {
            return Err(format!("{{{p}}} is not a placeholder here: {}", item_placeholders()));
        }
    }
    Ok("names each merge request".into())
}

/// The placeholders that name a merge request, as they are typed.
fn item_placeholders() -> String {
    ITEM_PLACEHOLDERS.iter().map(|p| format!("{{{p}}}")).collect::<Vec<_>>().join(", ")
}

/// `template` with each `{…}` that `value` knows replaced, in one pass, so
/// words put in are never read as placeholders themselves.
fn fill(template: &str, value: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else { break };
        out.push_str(&rest[..open]);
        match value(&after[..close]) {
            Some(v) => out.push_str(&v),
            None => out.push_str(&rest[open..open + close + 2]),
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

/// The message exactly as it will be sent to `channel`: the template with
/// `{group}` (the user group picked in settings) and `{@handle}` (a user
/// group by its handle), each as Slack's mention `<!subteam^ID>` and only
/// in the review channel; `{mrs}` (the line for each merge request, or a
/// bullet each); `{count}`; and each of [`ITEM_PLACEHOLDERS`], which with
/// several merge requests is their different values joined. A `{@handle}`
/// Slack has no group for is left as it is, and [`Draft::blocked`] says so.
pub fn text(cfg: &Settings, groups: &[UserGroup], channel: &str, items: &[Item]) -> String {
    let r = &cfg.review;
    let review = !r.channel.is_empty() && channel == r.channel;
    let group = if !r.group.is_empty() && review {
        format!("<!subteam^{}>", r.group)
    } else {
        String::new()
    };
    let link = if r.link.trim().is_empty() { "mr" } else { r.link.trim() };
    // One is linked by the link text; several each by their label.
    let link_of = |i: &Item| if items.len() == 1 { link.to_string() } else { i.label.clone() };
    let line = if r.line.trim().is_empty() { crate::config::default_line() } else { r.line.clone() };
    let line_tickets = placeholders(&line).contains(&"tickets");
    let line_of = |i: &Item| fill(&line, |p| i.value(p, &link_of(i), line_tickets));
    let mrs = match items {
        [one] => line_of(one),
        many => many.iter().map(|i| format!("\n• {}", line_of(i))).collect(),
    };
    let template = if r.template.trim().is_empty() {
        crate::config::default_template()
    } else {
        r.template.clone()
    };
    let tickets = placeholders(&template).contains(&"tickets");
    let filled = fill(&template, |p| match p {
        "group" => Some(group.clone()),
        "mrs" => Some(mrs.clone()),
        "count" => Some(items.len().to_string()),
        p if ITEM_PLACEHOLDERS.contains(&p) => {
            let mut seen: Vec<String> = Vec::new();
            for v in items.iter().filter_map(|i| i.value(p, &link_of(i), tickets)) {
                if !v.is_empty() && !seen.contains(&v) {
                    seen.push(v);
                }
            }
            Some(seen.join(", "))
        }
        p => {
            let g = p.strip_prefix('@').and_then(|h| group_by_handle(groups, h))?;
            Some(if review { format!("<!subteam^{}>", g.id) } else { String::new() })
        }
    });
    // An empty mention leaves its spaces behind: tidy each line.
    filled
        .lines()
        .map(|l| l.split(' ').filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// Keep editable words plain; insert Slack links only when formatting the
/// preview/confirmation/post. A branch or development-panel ticket that is
/// absent from the title is appended so that relationship is still visible,
/// unless `append` is false because `{tickets}` lists them.
fn linked_words(item: &Item, append: bool) -> String {
    let words = item.words.trim();
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let mut matches = Vec::new();
    let mut missing = Vec::new();
    for (key, url) in &item.tickets {
        if key.is_empty() || url.is_empty() {
            continue;
        }
        let mut found = false;
        for (start, _) in words.match_indices(key) {
            let end = start + key.len();
            if words[..start].chars().next_back().is_some_and(word)
                || words[end..].chars().next().is_some_and(word) {
                continue;
            }
            matches.push((start, end, url));
            found = true;
        }
        if !found {
            missing.push(format!("<{}|{}>", escape(url), escape(key)));
        }
    }
    matches.sort_by_key(|(start, end, _)| (*start, std::cmp::Reverse(*end)));
    let mut out = String::new();
    let mut at = 0;
    for (start, end, url) in matches {
        if start < at {
            continue;
        }
        out.push_str(&escape(&words[at..start]));
        out.push_str(&format!("<{}|{}>", escape(url), escape(&words[start..end])));
        at = end;
    }
    out.push_str(&escape(&words[at..]));
    if append && !missing.is_empty() {
        if !out.is_empty() {
            out.push_str(" · ");
        }
        out.push_str(&missing.join(", "));
    }
    out
}

/// A sample merge request, for the template's live preview in settings.
pub fn sample() -> Vec<Item> {
    vec![Item {
        id: "acme/team-a/api!263".into(),
        url: "https://git.example/acme/team-a/api/-/merge_requests/263".into(),
        label: "api !263".into(),
        words: "Add a stats endpoint for the dashboard".into(),
        tickets: vec![],
        facts: sample_facts("acme/team-a/api", "263", "stats-endpoint", "vimes"),
    }]
}

/// Each placeholder that names a merge request, with what it gives for
/// `item` linked as `link`, as words to read rather than Slack's markup.
pub fn examples(item: &Item, link: &str) -> Vec<(String, String)> {
    let fact = |f: &str| item.facts.get(f).cloned().unwrap_or_default();
    ITEM_PLACEHOLDERS
        .iter()
        .map(|p| {
            let example = match *p {
                "link" => format!("{link}, linked; {} for several", item.label),
                "url" => "its address".to_string(),
                "repo" => item.repo().to_string(),
                "title" => item.words.clone(),
                "tickets" => "its ticket keys, linked".to_string(),
                f => fact(f),
            };
            (format!("{{{p}}}"), example)
        })
        .collect()
}

/// A sample merge request's facts, for the previews in settings.
pub fn sample_facts(project: &str, iid: &str, branch: &str, author: &str) -> BTreeMap<String, String> {
    [("project", project), ("iid", iid), ("branch", branch), ("target", "main"), ("author", author)]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Channel, Kind};

    fn cfg() -> Settings {
        let mut s = Settings::default();
        for (alias, id, kind) in [("team", "C0000TEAM", Kind::Team), ("test", "C0000TEST", Kind::Test)] {
            s.channels.push(Channel {
                alias: alias.into(),
                id: id.into(),
                name: alias.into(),
                purpose: String::new(),
                kind,
                color: "blue".into(),
                watched: true,
                reader: None,
            });
        }
        s.review.channel = "team".into();
        s.review.group = "S0REVIEW1".into();
        s.review.handle = "review-squad".into();
        s.review.test_channel = "test".into();
        s
    }

    fn item(iid: u32, words: &str) -> Item {
        Item {
            id: format!("acme/team-a/api!{iid}"),
            url: format!("https://git.example/acme/team-a/api/-/merge_requests/{iid}"),
            label: format!("api !{iid}"),
            words: words.into(),
            tickets: vec![],
            facts: Default::default(),
        }
    }

    fn groups() -> Vec<UserGroup> {
        vec![
            UserGroup { id: "S0REVIEW1".into(), handle: "review-squad".into(), name: "Review squad".into(), users: vec![] },
            UserGroup { id: "S0ACMEDEV".into(), handle: "acme-devs".into(), name: "Acme devs".into(), users: vec![] },
        ]
    }

    /// What was posted live: `{@acme-devs} :gitlab: mr chore: …`, the
    /// placeholder sent as typed and no one pinged. A group named by its
    /// handle is sent as Slack's mention of its id.
    #[test]
    fn a_group_named_by_its_handle_is_sent_as_slacks_mention_of_its_id() {
        let mut c = cfg();
        c.review.group.clear();
        c.review.template = "{@acme-devs} :gitlab: {mrs}".into();
        let t = text(&c, &groups(), "team", &[item(198, "chore: renovate the images")]);
        assert_eq!(
            t,
            "<!subteam^S0ACMEDEV> :gitlab: <https://git.example/acme/team-a/api/-/merge_requests/198|mr> chore: renovate the images"
        );
        assert_eq!(mentioned(&c, &groups()), ["acme-devs"]);
        // Outside the review channel, no one.
        let t = text(&c, &groups(), "test", &[item(198, "chore")]);
        assert!(t.starts_with(":gitlab: <"), "{t}");
        // Slack's groups not read: it is not posted as it stands.
        let d = Draft::new(&c, item(198, ""));
        assert!(d.blocked(&c, &[]).unwrap().contains("user groups are not read yet"), "{:?}", d.blocked(&c, &[]));
        assert_eq!(d.blocked(&c, &groups()), None);
    }

    #[test]
    fn a_template_is_checked_for_what_slack_would_not_understand() {
        let g = groups();
        assert!(check_template("{group} {mrs}", &g).unwrap().contains("the group picked under user group"));
        assert_eq!(check_template("{@review-squad} {mrs}", &g).unwrap(), "mentions @review-squad (in the review channel)");
        assert!(check_template("{@nobody} {mrs}", &g).unwrap_err().contains("no user group with that handle"));
        assert!(check_template("{grop} {mrs}", &g).unwrap_err().contains("{grop} is not a placeholder"));
        assert!(check_template("@acme-devs please {mrs}", &g).unwrap_err().contains("plain words that ping no one"));
        assert!(check_template("{group}", &g).unwrap_err().contains("needs {mrs} or {link}"));
        assert!(check_template("{group} {link} {title}", &g).is_ok());
        assert_eq!(check_template(":gitlab: {mrs}", &g).unwrap(), "mentions no one");
    }

    #[test]
    fn one_merge_request_is_linked_as_mr_after_the_group_and_the_emoji() {
        let t = text(&cfg(), &[], "team", &[item(263, "Add a stats endpoint")]);
        assert_eq!(
            t,
            "<!subteam^S0REVIEW1> :gitlab: <https://git.example/acme/team-a/api/-/merge_requests/263|mr> Add a stats endpoint"
        );
    }

    #[test]
    fn ticket_keys_link_to_the_reference_url_without_escaping_the_link_markup() {
        let mut subject = Subject::new("merge-request", "mr-1", "api !1")
            .with_url("https://git.example/mr/1")
            .with("title", "chore(ABC-1274): Update build & release tasks.");
        subject.related.push(Subject::new("ticket", "ABC-1274", "ABC-1274")
            .with_url("https://acme.atlassian.net/browse/ABC-1274"));
        let i = Item::of(&subject).unwrap();
        assert_eq!(text(&cfg(), &[], "test", std::slice::from_ref(&i)),
            ":gitlab: <https://git.example/mr/1|mr> chore(<https://acme.atlassian.net/browse/ABC-1274|ABC-1274>): Update build &amp; release tasks.");
        let mut edited = i;
        edited.words = "ABC-12740 <tag> ABC-1274 / ABC-1274".into();
        let words = linked_words(&edited, true);
        assert!(words.starts_with("ABC-12740 &lt;tag&gt; <"), "{words}");
        assert_eq!(words.matches("|ABC-1274>").count(), 2);
    }

    #[test]
    fn tickets_not_in_the_words_are_appended_for_single_and_multiple_requests() {
        let mut i = item(263, "Tidy the build");
        i.tickets = vec![("ABC-12".into(), "https://jira.example/browse/ABC-12".into())];
        let expected = "Tidy the build · <https://jira.example/browse/ABC-12|ABC-12>";
        assert!(text(&cfg(), &[], "test", std::slice::from_ref(&i)).ends_with(expected));
        assert!(text(&cfg(), &[], "test", &[item(264, "Other"), i]).ends_with(expected));
        let mut subject = Subject::new("merge-request", "mr-1", "api !1")
            .with_url("https://git.example/mr/1").with("title", "ABC-12");
        subject.related.push(Subject::new("ticket", "ABC-12", "ABC-12"));
        assert_eq!(linked_words(&Item::of(&subject).unwrap(), true), "ABC-12");
    }

    #[test]
    fn several_are_bullets_each_named() {
        let t = text(&cfg(), &[], "team", &[item(263, "Add it"), item(264, "Fix <b> & more")]);
        assert_eq!(
            t,
            "<!subteam^S0REVIEW1> :gitlab:\n\
             • <https://git.example/acme/team-a/api/-/merge_requests/263|api !263> Add it\n\
             • <https://git.example/acme/team-a/api/-/merge_requests/264|api !264> Fix &lt;b&gt; &amp; more"
        );
    }

    fn watch_item(iid: u32, words: &str, author: &str) -> Item {
        Item {
            id: format!("ankh-morpork/watch!{iid}"),
            url: format!("https://git.example/ankh-morpork/watch/-/merge_requests/{iid}"),
            label: format!("watch !{iid}"),
            words: words.into(),
            tickets: vec![("WATCH-12".into(), "https://jira.example/browse/WATCH-12".into())],
            facts: sample_facts("ankh-morpork/watch", &iid.to_string(), "night-shift", author),
        }
    }

    #[test]
    fn a_template_names_the_merge_request_by_its_facts() {
        let mut c = cfg();
        c.review.template = "{group} {repo} !{iid} by {author}: {link} {title} ({branch} → {target}) {tickets}".into();
        assert_eq!(check_template(&c.review.template, &groups()).unwrap(), "mentions the group picked under user group (in the review channel)");
        let t = text(&c, &[], "test", &[watch_item(12, "Add the night shift", "vimes")]);
        assert_eq!(
            t,
            "watch !12 by vimes: <https://git.example/ankh-morpork/watch/-/merge_requests/12|mr> Add the night shift \
             (night-shift → main) <https://jira.example/browse/WATCH-12|WATCH-12>"
        );
    }

    #[test]
    fn each_merge_request_is_shown_by_the_line_and_the_template_joins_their_facts() {
        let mut c = cfg();
        c.review.template = "{count} in {repo} from {author}:{mrs}".into();
        c.review.line = "{link} {title} · {author}".into();
        let t = text(&c, &[], "test", &[watch_item(12, "Add it", "vimes"), watch_item(13, "Fix it", "carrot")]);
        assert_eq!(
            t,
            "2 in watch from vimes, carrot:\n\
             • <https://git.example/ankh-morpork/watch/-/merge_requests/12|watch !12> Add it \
             · <https://jira.example/browse/WATCH-12|WATCH-12> · vimes\n\
             • <https://git.example/ankh-morpork/watch/-/merge_requests/13|watch !13> Fix it \
             · <https://jira.example/browse/WATCH-12|WATCH-12> · carrot"
        );
        // A {title} that holds braces is not read as placeholders.
        c.review.line = "{link} {title}".into();
        let t = text(&c, &[], "test", &[watch_item(12, "{mrs} {author}", "vimes")]);
        assert!(t.ends_with("|mr> {mrs} {author} · <https://jira.example/browse/WATCH-12|WATCH-12>"), "{t}");
        // Without a project, the repo is the start of the label.
        assert_eq!(item(1, "").repo(), "api");
    }

    #[test]
    fn the_line_needs_a_link_and_only_names_a_merge_request() {
        assert!(check_line("{link} {title}").is_ok());
        assert!(check_line("{title}").unwrap_err().contains("needs {link}"));
        assert!(check_line("{link} {group}").unwrap_err().contains("{group} is not a placeholder here"));
        let mut c = cfg();
        c.review.line = "{title}".into();
        let d = Draft::new(&c, item(1, ""));
        assert!(d.blocked(&c, &[]).unwrap().starts_with("each merge request: needs {link}"));
    }

    #[test]
    fn the_group_is_never_mentioned_outside_the_review_channel() {
        let c = cfg();
        let t = text(&c, &[], "test", &[item(263, "Add it")]);
        assert_eq!(
            t,
            ":gitlab: <https://git.example/acme/team-a/api/-/merge_requests/263|mr> Add it"
        );
        let mut d = Draft::new(&c, item(263, "Add it"));
        assert!(d.mentions_group(&c));
        d.channel = "test".into();
        assert!(!d.mentions_group(&c));
    }

    #[test]
    fn a_draft_says_why_it_cannot_be_posted() {
        let mut c = cfg();
        c.review.channel.clear();
        let d = Draft::new(&c, item(1, ""));
        assert!(d.blocked(&c, &[]).unwrap().contains("set the review channel"));
        let mut d = Draft::new(&cfg(), item(1, ""));
        assert_eq!(d.blocked(&cfg(), &[]), None);
        assert!(!d.add(item(1, "again")), "once only");
        assert!(d.add(item(2, "")));
        d.items.clear();
        assert!(d.blocked(&cfg(), &[]).is_some());
    }

    #[test]
    fn adding_an_item_changes_the_revision_only_when_it_is_new() {
        let mut d = Draft::new(&cfg(), item(1, "first"));
        let before = d.stamp;
        assert!(!d.add(item(1, "duplicate")));
        assert_eq!(d.stamp, before);
        assert_eq!(d.items[0].words, "first");

        assert!(d.add(item(2, "second")));
        assert_ne!(d.stamp, before);
        assert_eq!(d.items.len(), 2);
    }

    #[test]
    fn the_template_and_link_text_are_the_settings() {
        let mut c = cfg();
        c.review.template = "Reviews please {group}: {mrs} thanks".into();
        c.review.link = "this".into();
        let t = text(&c, &[], "team", &[item(5, "")]);
        assert_eq!(
            t,
            "Reviews please <!subteam^S0REVIEW1>: <https://git.example/acme/team-a/api/-/merge_requests/5|this> thanks"
        );
    }
}
