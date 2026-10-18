//! What radar holds of Slack: who you are, names for ids, each channel's
//! recent messages and the threads under them, what is for you, the
//! caught-up marks (local, moved only by you) and the review asks being
//! followed. All of it is the cache, so the first frame is never empty.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::client::{ChannelInfo, Found, Me, Message, Reaction, UserGroup};
use crate::config::Settings;
use crate::mrkdwn::{self, Names};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Data {
    /// Local automation checkpoints, partitioned with the authenticated account.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub auto_reviews: BTreeMap<String, crate::auto_review::Checkpoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub me: Option<Me>,
    /// User id → the name shown.
    #[serde(default)]
    pub users: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<UserGroup>,
    /// By channel id.
    #[serde(default)]
    pub channels: BTreeMap<String, ChannelData>,
    /// Mentions of you and your groups, and messages to you, newest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub for_you: Vec<Found>,
    /// Channel id → the ts you are caught up to. Moved only by you.
    #[serde(default)]
    pub marks: BTreeMap<String, String>,
    /// Review asks radar posted, followed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub asked: Vec<Asked>,
    /// `channel/thread` → the ts its thread was read to in radar (opened).
    /// Local, never sent to Slack.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub opened: BTreeMap<String, String>,
    /// When the user groups were last read (unix secs; 0 never), so a
    /// restart does not read them again within the hour.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub groups_at: i64,
    /// When For you was last asked (unix secs; 0 never).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub for_you_at: i64,
    /// When For you was last asked for everything (unix secs; 0 never).
    /// Between those, a refresh asks only whether anything is newer than
    /// what is held and folds it in.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub for_you_swept_at: i64,
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

fn opened_key(channel: &str, thread: &str) -> String {
    format!("{channel}/{thread}")
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChannelData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info: Option<ChannelInfo>,
    /// Top-level messages, oldest first.
    #[serde(default)]
    pub messages: Vec<Message>,
    /// Thread ts → its replies (not the parent), oldest first.
    #[serde(default)]
    pub replies: BTreeMap<String, Vec<Message>>,
    /// When the channel's whole window was last read (unix secs; 0 never).
    /// Between those reads a refresh asks only for what is newer than the
    /// newest message held (`oldest=`), and folds it in.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub swept_at: i64,
    /// When the last read of the channel was asked (unix secs; 0 never). A
    /// channel read less than a refresh ago is not asked again.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub read_at: i64,
}

impl ChannelData {
    /// The newest top-level message held: where the next small read starts.
    pub fn newest(&self) -> Option<&str> {
        self.messages
            .iter()
            .map(|m| m.ts.as_str())
            .max_by_key(|ts| ts_key(ts))
    }
}

/// Fold `new` into `held` (oldest first): one message per ts, the copy just
/// read kept, still oldest first.
pub fn merge_messages(held: &mut Vec<Message>, new: Vec<Message>) {
    for m in new {
        match held.iter_mut().find(|h| h.ts == m.ts) {
            Some(h) => *h = m,
            None => held.push(m),
        }
    }
    held.sort_by_key(|m| ts_key(&m.ts));
}

/// A review ask radar posted, and what has happened to it since.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Asked {
    /// The merge requests asked about: subject id and link.
    pub mrs: Vec<(String, String)>,
    pub channel: String,
    pub ts: String,
    /// Posted in the test channel.
    #[serde(default)]
    pub test: bool,
    #[serde(default)]
    pub replies: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reactions: Vec<Reaction>,
}

/// `1700000000.123456` → seconds.
pub fn ts_secs(ts: &str) -> i64 {
    ts.split('.').next().and_then(|s| s.parse().ok()).unwrap_or(0)
}

/// Orders timestamps: seconds, then the part after the dot.
pub fn ts_key(ts: &str) -> (i64, i64) {
    let mut it = ts.split('.');
    let s = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let f = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    (s, f)
}

pub fn newer(a: &str, b: &str) -> bool {
    ts_key(a) > ts_key(b)
}

/// A message's link in Slack: `<workspace>archives/<channel>/p<ts>`, with
/// the thread for a reply.
pub fn permalink(workspace: &str, channel: &str, ts: &str, thread: Option<&str>) -> String {
    let base = workspace.trim_end_matches('/');
    let p: String = ts.chars().filter(|c| *c != '.').collect();
    match thread.filter(|t| *t != ts) {
        Some(t) => format!("{base}/archives/{channel}/p{p}?thread_ts={t}&cid={channel}"),
        None => format!("{base}/archives/{channel}/p{p}"),
    }
}

/// `p1700000000123456` → `1700000000.123456`.
pub fn ts_of_p(p: &str) -> Option<String> {
    let digits = p.strip_prefix('p')?;
    if digits.len() < 7 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (s, f) = digits.split_at(digits.len() - 6);
    Some(format!("{s}.{f}"))
}

/// A thread: a top-level message and its replies, in a channel.
#[derive(Debug, Clone, Copy)]
pub struct Thread<'a> {
    pub channel: &'a str,
    pub parent: &'a Message,
    pub replies: &'a [Message],
}

impl<'a> Thread<'a> {
    /// When anything last happened in it.
    pub fn last(&self) -> &'a str {
        self.parent
            .latest_reply
            .as_deref()
            .into_iter()
            .chain(self.replies.last().map(|m| m.ts.as_str()))
            .chain([self.parent.ts.as_str()])
            .max_by_key(|t| ts_key(t))
            .unwrap_or(&self.parent.ts)
    }

    pub fn reply_count(&self) -> u32 {
        self.parent.reply_count.max(self.replies.len() as u32)
    }

    /// Every message's text and the messages shared into it, the parent
    /// first.
    pub fn texts(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.parent)
            .chain(self.replies)
            .flat_map(|m| std::iter::once(m.text.as_str()).chain(m.shared.iter().map(|s| s.text.as_str())))
    }

    /// What can be opened from it, the parent's first, each once: its links
    /// by their words (`<url|words>`), the messages shared into it, and its
    /// files and images by name.
    pub fn openable(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        let mut add = |label: String, url: String| {
            if !url.is_empty() && !out.iter().any(|(_, u)| *u == url) {
                out.push((label, url));
            }
        };
        for m in std::iter::once(self.parent).chain(self.replies) {
            for (label, url) in mrkdwn::labelled_links(&m.text) {
                add(label, url);
            }
            for s in &m.shared {
                let who = if s.author.is_empty() { "a message".to_string() } else { format!("{}'s message", s.author) };
                add(format!("shared: {who}"), s.url.clone());
                for (label, url) in mrkdwn::labelled_links(&s.text) {
                    add(label, url);
                }
            }
            for f in &m.files {
                add(format!("{} {}", if f.image { "image" } else { "file" }, f.name), f.url.clone());
            }
        }
        out
    }

    /// Every link in it, the parent's first.
    pub fn links(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for t in self.texts() {
            for l in mrkdwn::links(t) {
                if !out.contains(&l) {
                    out.push(l);
                }
            }
        }
        out
    }

    /// Everyone who wrote in it, the starter first.
    pub fn people(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for m in std::iter::once(self.parent).chain(self.replies) {
            if let Some(u) = m.user.as_deref().or(m.bot_name.as_deref())
                && !out.contains(&u)
            {
                out.push(u);
            }
        }
        for u in &self.parent.reply_users {
            if !out.contains(&u.as_str()) {
                out.push(u);
            }
        }
        out
    }
}

/// Whether one of `links` goes to `target` or under it (`…/263/diffs`,
/// `…/263#note_1`), not to a neighbour (`…/2630`).
pub fn links_to(links: &[String], target: &str) -> bool {
    let target = target.trim_end_matches('/');
    !target.is_empty()
        && links.iter().any(|l| {
            l.strip_prefix(target)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(['/', '#', '?']))
        })
}

/// `word` in `text` with no letter, digit, `-` or `_` right before or after.
pub fn has_word(text: &str, word: &str) -> bool {
    let joins = |c: char| c.is_alphanumeric() || c == '-' || c == '_';
    let mut from = 0;
    while let Some(at) = text[from..].find(word) {
        let start = from + at;
        let end = start + word.len();
        let before = text[..start].chars().next_back();
        let after = text[end..].chars().next();
        if !before.is_some_and(joins) && !after.is_some_and(joins) {
            return true;
        }
        from = start + word.len().max(1);
        while !text.is_char_boundary(from) {
            from += 1;
        }
    }
    false
}

impl Data {
    /// The threads of a channel, oldest first.
    pub fn threads<'a>(&'a self, channel: &'a str) -> Vec<Thread<'a>> {
        let Some(c) = self.channels.get(channel) else {
            return vec![];
        };
        c.messages
            .iter()
            .map(|m| Thread {
                channel,
                parent: m,
                replies: c.replies.get(&m.ts).map_or(&[][..], Vec::as_slice),
            })
            .collect()
    }

    /// One thread, by its channel and the ts of its first message.
    pub fn thread<'a>(&'a self, channel: &'a str, ts: &str) -> Option<Thread<'a>> {
        let c = self.channels.get(channel)?;
        let parent = c.messages.iter().find(|m| m.ts == ts)?;
        Some(Thread {
            channel,
            parent,
            replies: c.replies.get(ts).map_or(&[][..], Vec::as_slice),
        })
    }

    /// Every thread of the watched channels, the most recently active first.
    pub fn all_threads<'a>(&'a self, cfg: &'a Settings) -> Vec<Thread<'a>> {
        let mut out: Vec<Thread<'a>> = cfg
            .watched()
            .flat_map(|c| self.threads(&c.id))
            .collect();
        out.sort_by_key(|t| std::cmp::Reverse(ts_key(t.last())));
        out
    }

    /// Whether a message (`ts`, in the thread started at `thread`) is new:
    /// past the channel's caught-up mark, and not read in radar since (its
    /// thread opened).
    pub fn is_new(&self, channel: &str, thread: &str, ts: &str) -> bool {
        self.marks.get(channel).is_none_or(|m| newer(ts, m))
            && self.opened.get(&opened_key(channel, thread)).is_none_or(|m| newer(ts, m))
    }

    /// A thread was read in radar up to `ts`: what is in it up to then is no
    /// longer new. Local only, never sent to Slack.
    pub fn read_to(&mut self, channel: &str, thread: &str, ts: &str) {
        let key = opened_key(channel, thread);
        if self.opened.get(&key).is_none_or(|m| newer(ts, m)) {
            self.opened.insert(key, ts.to_string());
        }
    }

    /// Forget what was read in threads older than `oldest` seconds: they are
    /// past any mark that matters by then.
    pub fn forget_read_before(&mut self, oldest: i64) {
        self.opened.retain(|_, ts| ts_secs(ts) >= oldest);
    }

    /// What is new in a channel: top-level messages and replies past its mark.
    pub fn new_count(&self, channel: &str) -> usize {
        self.threads(channel)
            .iter()
            .map(|t| {
                usize::from(self.is_new(channel, &t.parent.ts, &t.parent.ts))
                    + t.replies.iter().filter(|r| self.is_new(channel, &t.parent.ts, &r.ts)).count()
            })
            .sum()
    }

    /// The user groups you are in.
    pub fn my_groups(&self) -> Vec<&UserGroup> {
        let Some(me) = &self.me else { return vec![] };
        self.groups
            .iter()
            .filter(|g| g.users.contains(&me.user_id))
            .collect()
    }

    /// Threads in watched channels you wrote in, with something new since.
    pub fn yours<'a>(&'a self, cfg: &'a Settings) -> Vec<Thread<'a>> {
        let Some(me) = &self.me else { return vec![] };
        self.all_threads(cfg)
            .into_iter()
            .filter(|t| {
                t.parent.reply_count > 0
                    && t.people().contains(&me.user_id.as_str())
                    && self.is_new(t.channel, &t.parent.ts, t.last())
            })
            .collect()
    }

    /// The name shown for a user id, or the id.
    pub fn name(&self, user: &str) -> String {
        self.users.get(user).cloned().unwrap_or_else(|| user.to_string())
    }

    /// Who wrote a message, as shown.
    pub fn author(&self, m: &Message) -> String {
        match (&m.user, &m.bot_name) {
            (Some(u), _) => self.name(u),
            (None, Some(b)) => b.clone(),
            _ => "someone".into(),
        }
    }

    /// The review ask radar posted for a merge request, newest first.
    pub fn asked_for(&self, id: &str) -> Option<&Asked> {
        self.asked
            .iter()
            .filter(|a| a.mrs.iter().any(|(m, _)| m == id))
            .max_by_key(|a| ts_key(&a.ts))
    }
}

/// Names for ids: users and groups from the cache, channels from the
/// config (as you call them) and what Slack said.
pub struct Namer<'a> {
    pub data: &'a Data,
    pub cfg: &'a Settings,
}

impl Names for Namer<'_> {
    fn user(&self, id: &str) -> Option<String> {
        self.data.users.get(id).cloned()
    }
    fn group(&self, id: &str) -> Option<String> {
        self.data
            .groups
            .iter()
            .find(|g| g.id == id)
            .map(|g| g.handle.clone())
            .or_else(|| (self.cfg.review.group == id && !self.cfg.review.handle.is_empty()).then(|| self.cfg.review.handle.clone()))
    }
    fn channel(&self, id: &str) -> Option<String> {
        self.cfg
            .by_id(id)
            .map(|c| if c.name.is_empty() { c.alias.clone() } else { c.name.clone() })
            .or_else(|| self.data.channels.get(id)?.info.as_ref().map(|i| i.name.clone()))
    }
}

fn weekday(day: i64) -> i64 {
    // 1970-01-01 was a Thursday: 0 Monday … 6 Sunday.
    (day + 3).rem_euclid(7)
}

/// Seconds from `from` to `to` that fall on working days (Monday to
/// Friday); both already in local time.
pub fn working_secs(from: i64, to: i64) -> i64 {
    let mut out = 0;
    let mut t = from;
    while t < to {
        let day = t.div_euclid(86_400);
        let end = ((day + 1) * 86_400).min(to);
        if weekday(day) < 5 {
            out += end - t;
        }
        t = end;
    }
    out
}

/// Whether an ask made at `from` with no reply is due to be asked again at
/// `now` (both seconds, UTC), counting in a time zone `offset` seconds east
/// of UTC.
pub fn reask_due(rule: crate::config::Reask, from: i64, now: i64, offset: i64) -> bool {
    use crate::config::Reask;
    let (from, now) = (from + offset, now + offset);
    match rule {
        Reask::Never => false,
        Reask::Days(n) => working_days(from, now) >= i64::from(n),
        Reask::Hours(n) => working_secs(from, now) >= i64::from(n) * 3600,
        Reask::At(minutes) => {
            let mut day = from.div_euclid(86_400);
            if day * 86_400 + i64::from(minutes) * 60 <= from {
                day += 1;
            }
            while weekday(day) >= 5 {
                day += 1;
            }
            now >= day * 86_400 + i64::from(minutes) * 60
        }
    }
}

/// Working days (Monday to Friday) from `from` to `to`, in whole days.
pub fn working_days(from: i64, to: i64) -> i64 {
    if to <= from {
        return 0;
    }
    let mut days = 0;
    let mut t = from;
    while t + 86_400 <= to {
        t += 86_400;
        // 1970-01-01 was a Thursday: day 0 is Thursday.
        let weekday = (t.div_euclid(86_400) + 3).rem_euclid(7); // 0 Monday … 6 Sunday
        if weekday < 5 {
            days += 1;
        }
    }
    days
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(ts: &str, user: &str, text: &str) -> Message {
        Message {
            ts: ts.into(),
            user: Some(user.into()),
            text: text.into(),
            ..Message::default()
        }
    }

    #[test]
    fn timestamps_order_by_seconds_then_the_rest() {
        assert!(newer("1700000001.000001", "1700000000.999999"));
        assert!(newer("1700000000.000010", "1700000000.000009"));
        assert_eq!(ts_secs("1700000000.5"), 1_700_000_000);
    }

    #[test]
    fn permalinks_name_the_thread_for_a_reply() {
        assert_eq!(
            permalink("https://acme.slack.com/", "C1", "1700000000.123456", None),
            "https://acme.slack.com/archives/C1/p1700000000123456"
        );
        assert_eq!(
            permalink("https://acme.slack.com", "C1", "1700000001.000001", Some("1700000000.123456")),
            "https://acme.slack.com/archives/C1/p1700000001000001?thread_ts=1700000000.123456&cid=C1"
        );
        assert_eq!(ts_of_p("p1700000000123456").as_deref(), Some("1700000000.123456"));
        assert_eq!(ts_of_p("pxyz"), None);
    }

    #[test]
    fn a_thread_names_a_link_and_what_is_under_it_but_not_its_neighbour() {
        let parent = msg("1.0", "U1", "see <https://git.example/acme/api/-/merge_requests/263/diffs|mr>");
        let t = Thread { channel: "C1", parent: &parent, replies: &[] };
        let links = t.links();
        assert!(links_to(&links, "https://git.example/acme/api/-/merge_requests/263"));
        assert!(!links_to(&links, "https://git.example/acme/api/-/merge_requests/26"));
        assert!(!links_to(&links, ""));
    }

    #[test]
    fn a_key_is_found_as_a_word_only() {
        assert!(has_word("ABC-12: add it", "ABC-12"));
        assert!(has_word("for (ABC-12)", "ABC-12"));
        assert!(!has_word("ABC-123", "ABC-12"));
        assert!(!has_word("XABC-12", "ABC-12"));
        assert!(has_word("é ABC-12", "ABC-12"));
    }

    #[test]
    fn what_is_new_is_past_the_mark_and_counts_replies() {
        let mut d = Data::default();
        let c = d.channels.entry("C1".into()).or_default();
        c.messages = vec![msg("10.0", "U1", "old"), msg("20.0", "U2", "new")];
        c.replies.insert("10.0".into(), vec![msg("25.0", "U2", "late reply")]);
        assert_eq!(d.new_count("C1"), 3, "no mark: all of it");
        d.marks.insert("C1".into(), "15.0".into());
        assert_eq!(d.new_count("C1"), 2);
        assert!(!d.is_new("C1", "10.0", "10.0"));
        // Read in radar: the thread up to its late reply is no longer new.
        d.read_to("C1", "10.0", "25.0");
        assert_eq!(d.new_count("C1"), 1, "only the other thread");
        assert!(d.is_new("C1", "10.0", "26.0"), "a later reply is new again");
        d.read_to("C1", "10.0", "12.0");
        assert!(!d.is_new("C1", "10.0", "25.0"), "never moved back");
    }

    #[test]
    fn working_days_skip_the_weekend() {
        // 2026-10-02 was a Friday; the Monday after is 3 days later.
        let friday = 1_791_244_800; // 2026-10-06 is a Tuesday: go back 4 days
        let friday = friday - 4 * 86_400;
        assert_eq!(working_days(friday, friday + 3 * 86_400), 1);
        assert_eq!(working_days(friday, friday + 5 * 86_400), 3);
        assert_eq!(working_days(friday, friday), 0);
    }

    #[test]
    fn asking_again_counts_working_hours_days_or_waits_for_the_time() {
        use crate::config::Reask;
        // 2026-10-02, a Friday, 00:00 UTC.
        let fri = 1_791_244_800 - 4 * 86_400;
        let h = 3600;
        // Asked Friday 15:00: 4 working hours is Friday 19:00; the weekend
        // does not count, so 12 working hours is Monday 03:00.
        let asked = fri + 15 * h;
        assert!(!reask_due(Reask::Hours(4), asked, fri + 18 * h, 0));
        assert!(reask_due(Reask::Hours(4), asked, fri + 19 * h, 0));
        assert!(!reask_due(Reask::Hours(12), asked, fri + 2 * 86_400 + 23 * h, 0));
        assert!(reask_due(Reask::Hours(12), asked, fri + 3 * 86_400 + 3 * h, 0));
        // At 10:00: the next working day's, Monday.
        assert!(!reask_due(Reask::At(600), asked, fri + 3 * 86_400 + 9 * h, 0));
        assert!(reask_due(Reask::At(600), asked, fri + 3 * 86_400 + 10 * h, 0));
        // Asked at 08:00 on a Friday: 10:00 the same day.
        assert!(reask_due(Reask::At(600), fri + 8 * h, fri + 10 * h, 0));
        // In a time zone an hour east, 10:00 local is 09:00 UTC.
        assert!(reask_due(Reask::At(600), fri + 7 * h, fri + 9 * h, h));
        assert!(!reask_due(Reask::At(600), fri + 7 * h, fri + 9 * h, 0));
        // One working day from Friday is Monday.
        assert!(!reask_due(Reask::Days(1), asked, fri + 2 * 86_400 + 16 * h, 0));
        assert!(reask_due(Reask::Days(1), asked, fri + 3 * 86_400 + 16 * h, 0));
        assert!(!reask_due(Reask::Never, asked, fri + 30 * 86_400, 0));
    }
}
