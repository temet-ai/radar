//! Alerts: what a bot's messages in an alerts channel mean, as plain
//! functions with no I/O.
//!
//! - **Readers** turn one message into alerts, each for one shape of message
//!   (chosen per channel in config): [`Reader::Alertmanager`] (one
//!   attachment per alert, titled `[FIRING]` or `[RESOLVED] <AlertName>`, its
//!   environment, priority, summary and console link in its text or
//!   fields), [`Reader::JobFailed`] (a header saying a job failed, details
//!   in a context) and [`Reader::Status`] (any bot message with a status
//!   word). A reader never guesses: a bot's message it cannot read comes
//!   back as a refusal with the reason, and is logged; what a person wrote
//!   is not an alert and is passed over without a word.
//! - **Pairing** joins the firing and the resolved of one alert (same name
//!   and environment in a channel) into one [`Alert`] with how long it fired.
//! - **Mapping** places an alert on a repo, a service and an environment by
//!   the rules in settings; an alert no rule matches is `unmapped` and still
//!   shows.

use std::collections::BTreeMap;

use crate::client::Message;
use crate::config::{Alerts as AlertSettings, Kind, Reader, Rule, Settings};
use crate::model::{Data, ts_key, ts_secs};
use crate::mrkdwn::{self, NoNames};

/// Where an alert stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Firing,
    Resolved,
    /// A job that failed: it happened, and does not resolve.
    Failed,
}

impl State {
    pub fn word(self) -> &'static str {
        match self {
            State::Firing => "firing",
            State::Resolved => "resolved",
            State::Failed => "failed",
        }
    }

    /// Its icon: firing, resolved, a job that failed.
    pub fn icon(self) -> radar_core::ui::icon::Icon {
        use radar_core::ui::icon::Icon;
        match self {
            State::Firing => Icon::AlertFiring,
            State::Resolved => Icon::AlertResolved,
            State::Failed => Icon::RunFailed,
        }
    }

    pub fn mark(self) -> &'static str {
        match self {
            State::Firing => "▲",
            State::Resolved => "✓",
            State::Failed => "✗",
        }
    }
}

/// What a reader made of one alert in a message.
#[derive(Debug, Clone, PartialEq)]
pub struct Seen {
    pub name: String,
    pub state: State,
    pub environment: String,
    pub priority: String,
    pub summary: String,
    /// The console link.
    pub link: Option<String>,
    /// Labels the message carried: `service=billing`.
    pub labels: BTreeMap<String, String>,
    pub source: Reader,
}

/// What a reader found in one message.
#[derive(Debug, Default, PartialEq)]
pub struct Reading {
    pub alerts: Vec<Seen>,
    /// Why a bot's attachment or message could not be read, one per piece.
    pub refused: Vec<String>,
    /// Written by a person: not an alert, and nothing to say about it.
    pub person: bool,
}

/// Whether a message is a bot's.
fn is_bot(m: &Message) -> bool {
    m.bot_name.is_some() || m.subtype.as_deref() == Some("bot_message") || !m.cards.is_empty() || !m.blocks.is_empty()
}

/// Read one message with `reader`.
pub fn read(reader: Reader, m: &Message) -> Reading {
    if m.user.is_some() && m.bot_name.is_none() && m.subtype.as_deref() != Some("bot_message") {
        return Reading { person: true, ..Reading::default() };
    }
    if !is_bot(m) {
        return Reading { person: true, ..Reading::default() };
    }
    let mut out = Reading::default();
    match reader {
        Reader::Alertmanager => {
            for card in cards_or_text(m) {
                match alertmanager(&card) {
                    Ok(s) => out.alerts.push(s),
                    Err(why) => out.refused.push(why),
                }
            }
            if out.alerts.is_empty() && out.refused.is_empty() {
                out.refused.push("no attachment and no text to read".into());
            }
        }
        Reader::JobFailed => match job_failed(m) {
            Ok(s) => out.alerts.push(s),
            Err(why) => out.refused.push(why),
        },
        Reader::Status => match status(m) {
            Ok(s) => out.alerts.push(s),
            Err(why) => out.refused.push(why),
        },
    }
    out
}

// --- text helpers ------------------------------------------------------------------------

/// A bot's attachments, or its text as one when it sent none.
fn cards_or_text(m: &Message) -> Vec<crate::client::Card> {
    if !m.cards.is_empty() {
        return m.cards.clone();
    }
    let text = m.text.trim();
    if text.is_empty() {
        return vec![];
    }
    let (first, rest) = text.split_once('\n').unwrap_or((text, ""));
    vec![crate::client::Card {
        title: first.to_string(),
        text: rest.to_string(),
        ..Default::default()
    }]
}

/// The text with Slack's markup gone, one string per line.
fn plain_lines(text: &str) -> Vec<String> {
    mrkdwn::plain(text, &NoNames)
        .split('\n')
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// Without the markup characters round it and the colon a label ends with.
fn bare(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '*' | '`' | '~'))
        .collect::<String>()
        .trim()
        .trim_matches('_')
        .trim()
        .trim_end_matches(':')
        .trim()
        .to_string()
}

/// `key: value`, `*key:* value`, `key = value` and `*key*` over `value` on
/// the next line, as lowercase keys with their raw values (a link stays a
/// link). Lines that are not that are passed over.
fn pairs(texts: &[&str]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for text in texts {
        let lines: Vec<&str> = text
            .lines()
            .flat_map(|l| l.split(" | "))
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        let mut i = 0;
        while i < lines.len() {
            let line = lines[i].trim_start_matches(['•', '-', '·', ' ']);
            i += 1;
            // `key = value`
            if let Some((k, v)) = line.split_once(" = ").or_else(|| line.split_once('='))
                && !k.contains("://")
                && !k.contains(':')
                && !k.trim().is_empty()
                && k.chars().count() <= 30
            {
                out.push((bare(k).to_lowercase(), v.trim().trim_matches('`').to_string()));
                continue;
            }
            // `key: value`, the key short and not a link
            if let Some((k, v)) = line.split_once(':')
                && !v.starts_with("//")
                && bare(k).chars().count() <= 30
                && !bare(k).is_empty()
                && !k.contains("://")
                && !v.trim_start_matches(['*', '_']).trim().is_empty()
            {
                out.push((bare(k).to_lowercase(), v.trim_start_matches(['*', '_']).trim().to_string()));
                continue;
            }
            // `*key:*` with its value on the next line
            if line.trim_end_matches(['*', '_']).ends_with(':')
                && bare(line).chars().count() <= 30
                && let Some(next) = lines.get(i)
                && !next.starts_with(['•', '-', '·'])
                && !next.contains(" = ")
            {
                out.push((bare(line).to_lowercase(), (*next).to_string()));
                i += 1;
            }
        }
    }
    out
}

/// What the pairs of a message say.
#[derive(Debug, Default)]
struct Details {
    environment: String,
    priority: String,
    summary: String,
    link: Option<String>,
    labels: BTreeMap<String, String>,
}

fn details(pairs: &[(String, String)]) -> Details {
    let mut d = Details::default();
    let plain = |v: &str| mrkdwn::plain(v, &NoNames).trim().to_string();
    for (k, v) in pairs {
        let k = k.as_str();
        match k {
            "environment" | "env" => set(&mut d.environment, plain(v)),
            "priority" | "severity" | "prio" => {
                set(&mut d.priority, plain(v));
                d.labels.entry(k.to_string()).or_insert_with(|| plain(v));
            }
            "summary" | "description" | "message" => set(&mut d.summary, plain(v)),
            "console" | "console link" | "link" | "url" | "dashboard" | "runbook" | "source" | "details" | "logs" => {
                if d.link.is_none() {
                    d.link = mrkdwn::links(v).into_iter().next();
                }
            }
            _ => {
                let v = plain(v);
                if !k.contains(' ') && !v.is_empty() && v.chars().count() <= 60 {
                    d.labels.entry(k.to_string()).or_insert(v);
                }
            }
        }
    }
    d
}

fn set(into: &mut String, v: String) {
    if into.is_empty() && !v.is_empty() {
        *into = v;
    }
}

/// `:fire:`-style emoji at the start of a line, and the marks around it.
fn without_lead_emoji(s: &str) -> String {
    let mut rest = s.trim();
    while let Some(r) = rest.strip_prefix(':') {
        match r.split_once(':') {
            Some((name, after)) if !name.is_empty() && !name.contains(char::is_whitespace) => rest = after.trim_start(),
            _ => break,
        }
    }
    rest.to_string()
}

/// `key=value` tokens in a line, with the brackets and commas round them gone.
fn labels_in(rest: &str) -> BTreeMap<String, String> {
    rest.split_whitespace()
        .filter_map(|w| {
            let w = w.trim_matches(|c| matches!(c, '(' | ')' | ',' | '"' | '\'' | '{' | '}' | '`' | '*'));
            let (k, v) = w.split_once('=')?;
            let v = v.trim_matches(|c| matches!(c, '"' | '\'' | ','));
            (!k.is_empty() && !v.is_empty()).then(|| (k.to_lowercase(), v.to_string()))
        })
        .collect()
}

/// What an attachment's colour says: `good` or green is a resolved alert,
/// `danger` or red a firing one, anything else nothing.
fn colour_says(color: &str) -> Option<State> {
    let c = color.trim().to_lowercase();
    match c.as_str() {
        "good" => return Some(State::Resolved),
        "danger" => return Some(State::Firing),
        _ => {}
    }
    let hex = c.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(hex, 16).ok()?;
    let (r, g, b) = ((n >> 16) as u8, (n >> 8) as u8, n as u8);
    if r >= 0xc0 && g < 0x60 && b < 0x60 {
        Some(State::Firing)
    } else if g >= 0x80 && r < 0x80 && b < 0x90 {
        Some(State::Resolved)
    } else {
        None
    }
}

// --- the Alertmanager reader ----------------------------------------------------------------

/// `[FIRING:2] HighErrors prod` → the state, the name and what follows it.
fn firing_title(title: &str) -> Result<(State, String, String), String> {
    let title = without_lead_emoji(title);
    let open = title
        .find('[')
        .filter(|i| *i <= 12)
        .ok_or_else(|| format!("the title has no [FIRING] or [RESOLVED]: “{}”", radar_core::ui::fit(&title, 60).trim_end()))?;
    let close = title[open..]
        .find(']')
        .map(|i| open + i)
        .ok_or_else(|| "the title's [ is never closed".to_string())?;
    let word = title[open + 1..close].split(':').next().unwrap_or("").trim().to_uppercase();
    let state = match word.as_str() {
        "FIRING" => State::Firing,
        "RESOLVED" => State::Resolved,
        other => return Err(format!("the title's [{other}] is neither FIRING nor RESOLVED")),
    };
    let after = title[close + 1..].trim().trim_matches(['*', '`', '_']).trim().to_string();
    let (name, rest) = after.split_once(char::is_whitespace).unwrap_or((after.as_str(), ""));
    let name = name.trim_matches(|c: char| matches!(c, ':' | ',' | '"' | '`'));
    if name.is_empty() {
        return Err("the title has no alert name after the state".into());
    }
    Ok((state, name.to_string(), rest.trim().to_string()))
}

fn alertmanager(card: &crate::client::Card) -> Result<Seen, String> {
    let title = [&card.title, &card.pretext]
        .into_iter()
        .find(|t| !t.trim().is_empty())
        .map_or("", |t| t.as_str());
    if title.trim().is_empty() {
        return Err("an attachment with no title".into());
    }
    let (state, name, rest) = firing_title(title)?;
    // The colour must not say the opposite of the title.
    match (state, colour_says(&card.color)) {
        (State::Firing, Some(State::Resolved)) | (State::Resolved, Some(State::Firing)) => {
            return Err(format!(
                "{name}: the title says {} but the colour says the opposite",
                state.word()
            ));
        }
        _ => {}
    }
    let mut p = pairs(&[card.text.as_str()]);
    p.extend(
        card.fields
            .iter()
            .map(|(k, v)| (bare(k).to_lowercase(), v.trim().to_string()))
            .filter(|(k, v)| !k.is_empty() && !v.is_empty()),
    );
    let mut d = details(&p);
    for (k, v) in labels_in(&rest) {
        d.labels.entry(k).or_insert(v);
    }
    if d.environment.is_empty() {
        d.environment = d.labels.get("env").or_else(|| d.labels.get("environment")).cloned().unwrap_or_default();
    }
    if d.priority.is_empty() {
        d.priority = d.labels.get("severity").or_else(|| d.labels.get("priority")).cloned().unwrap_or_default();
    }
    if d.summary.is_empty() {
        d.summary = plain_lines(&card.text).into_iter().find(|l| !l.contains(": ") && !l.contains(" = ")).unwrap_or_default();
    }
    let link = d
        .link
        .or_else(|| mrkdwn::links(&card.text).into_iter().next())
        .or_else(|| (!card.title_link.is_empty()).then(|| card.title_link.clone()));
    Ok(Seen {
        name,
        state,
        environment: d.environment,
        priority: d.priority,
        summary: d.summary,
        link,
        labels: d.labels,
        source: Reader::Alertmanager,
    })
}

// --- the job-failed reader --------------------------------------------------------------------

fn job_failed(m: &Message) -> Result<Seen, String> {
    let header = m
        .blocks
        .iter()
        .find(|b| b.kind == "header")
        .and_then(|b| b.texts.first())
        .ok_or_else(|| "no header block".to_string())?;
    let header = mrkdwn::plain(&without_lead_emoji(header), &NoNames).trim().to_string();
    let says_failed = |s: &str| {
        s.split(|c: char| !c.is_alphanumeric())
            .any(|w| w.eq_ignore_ascii_case("failed") || w.eq_ignore_ascii_case("failure"))
    };
    if !says_failed(&header) {
        return Err(format!("the header does not say a job failed: “{}”", radar_core::ui::fit(&header, 60).trim_end()));
    }
    let rest: Vec<&str> = m
        .blocks
        .iter()
        .filter(|b| b.kind != "header")
        .flat_map(|b| b.texts.iter().map(String::as_str))
        .collect();
    let p = pairs(&rest);
    let d = details(&p);
    let from_pairs = p
        .iter()
        .find(|(k, _)| matches!(k.as_str(), "job" | "job name" | "task" | "workflow" | "pipeline"))
        .map(|(_, v)| mrkdwn::plain(v, &NoNames).trim().to_string())
        .filter(|v| !v.is_empty());
    let name = from_pairs.unwrap_or_else(|| name_from_header(&header));
    if name.is_empty() {
        return Err("no job name in the header or the details".into());
    }
    let mut summary = d.summary.clone();
    if summary.is_empty() {
        for (k, v) in &p {
            if matches!(k.as_str(), "error" | "reason" | "failure") {
                summary = mrkdwn::plain(v, &NoNames).trim().to_string();
                break;
            }
        }
    }
    let link = d.link.clone().or_else(|| rest.iter().flat_map(|t| mrkdwn::links(t)).next());
    Ok(Seen {
        name,
        state: State::Failed,
        environment: d.environment,
        priority: d.priority,
        summary,
        link,
        labels: d.labels,
        source: Reader::JobFailed,
    })
}

/// The job's name out of a header that says it failed: `Nightly export
/// failed`, `Job failed: nightly-export`, `Build nightly-export failed`.
fn name_from_header(header: &str) -> String {
    let is_failed = |w: &str| {
        let w = w.trim_matches(|c: char| !c.is_alphanumeric());
        w.eq_ignore_ascii_case("failed") || w.eq_ignore_ascii_case("failure")
    };
    if let Some((before, after)) = header.split_once(':') {
        if before.split_whitespace().any(is_failed) && !after.trim().is_empty() {
            return after.trim().to_string();
        }
        if after.split_whitespace().any(is_failed) && !before.trim().is_empty() {
            return before.trim().to_string();
        }
    }
    let words: Vec<&str> = header.split_whitespace().filter(|w| !is_failed(w)).collect();
    let words = match words.first() {
        Some(w) if w.eq_ignore_ascii_case("job") && words.len() > 1 => &words[1..],
        _ => &words[..],
    };
    words.join(" ")
}

// --- the status-word reader --------------------------------------------------------------------

const FIRING_WORDS: [&str; 10] = [
    "FIRING", "ALERT", "ALERTING", "FAILED", "FAILURE", "DOWN", "CRITICAL", "ERROR", "TRIGGERED", "BROKEN",
];
const RESOLVED_WORDS: [&str; 10] = [
    "RESOLVED", "OK", "RECOVERED", "RECOVERY", "UP", "FIXED", "CLEARED", "SUCCEEDED", "PASSED", "HEALTHY",
];

/// The status words of a line: which kind and the word, each once.
fn status_words(line: &str) -> Vec<(State, String)> {
    let mut out: Vec<(State, String)> = Vec::new();
    for w in line.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()) {
        let up = w.to_uppercase();
        let state = if FIRING_WORDS.contains(&up.as_str()) {
            State::Firing
        } else if RESOLVED_WORDS.contains(&up.as_str()) {
            State::Resolved
        } else {
            continue;
        };
        if !out.iter().any(|(_, x)| *x == up) {
            out.push((state, up));
        }
    }
    out
}

fn status(m: &Message) -> Result<Seen, String> {
    // The first line that has a status word: a title, else the text.
    let mut candidates: Vec<String> = Vec::new();
    for c in &m.cards {
        candidates.extend([c.title.clone(), c.pretext.clone()]);
        candidates.extend(plain_lines(&c.text).into_iter().take(2));
    }
    candidates.extend(plain_lines(&m.text).into_iter().take(2));
    let line = candidates
        .into_iter()
        .map(|c| mrkdwn::plain(&without_lead_emoji(&c), &NoNames).trim().to_string())
        .find(|c| !status_words(c).is_empty())
        .ok_or_else(|| "no status word (firing, resolved, failed, ok…) in it".to_string())?;
    let words = status_words(&line);
    let kinds: Vec<State> = words.iter().map(|(s, _)| *s).collect();
    if kinds.iter().any(|k| *k != kinds[0]) {
        return Err(format!(
            "the line says both firing and resolved words ({}): “{}”",
            words.iter().map(|(_, w)| w.to_lowercase()).collect::<Vec<_>>().join(", "),
            radar_core::ui::fit(&line, 60).trim_end()
        ));
    }
    let state = kinds[0];
    // The name: the line without its status words and the marks round them.
    let name: String = line
        .split_whitespace()
        .filter(|w| status_words(w).is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let name = name
        .trim_matches(|c: char| matches!(c, '[' | ']' | '(' | ')' | ':' | '-' | '—' | '·' | '|' | '*' | '`' | ' '))
        .to_string();
    if name.is_empty() {
        return Err(format!("no name beside the status word: “{}”", radar_core::ui::fit(&line, 60).trim_end()));
    }
    let mut texts: Vec<&str> = m.cards.iter().map(|c| c.text.as_str()).collect();
    texts.push(&m.text);
    let mut p = pairs(&texts);
    for c in &m.cards {
        p.extend(c.fields.iter().map(|(k, v)| (bare(k).to_lowercase(), v.trim().to_string())));
    }
    let d = details(&p);
    let link = d
        .link
        .or_else(|| texts.iter().flat_map(|t| mrkdwn::links(t)).next())
        .or_else(|| m.cards.iter().map(|c| c.title_link.clone()).find(|l| !l.is_empty()));
    Ok(Seen {
        name,
        state,
        environment: d.environment,
        priority: d.priority,
        summary: d.summary,
        link,
        labels: d.labels,
        source: Reader::Status,
    })
}

// --- pairing --------------------------------------------------------------------------------------

/// One alert: its firing and its resolved joined, or a job that failed.
#[derive(Debug, Clone, PartialEq)]
pub struct Alert {
    pub name: String,
    pub state: State,
    pub environment: String,
    pub priority: String,
    pub summary: String,
    pub link: Option<String>,
    pub labels: BTreeMap<String, String>,
    pub source: Reader,
    /// The channel's id, and the ts of the message that opened it: its thread.
    pub channel: String,
    pub ts: String,
    /// The newest message about it.
    pub last_ts: String,
    /// When it began (seconds): its first message.
    pub since: i64,
    /// When it resolved.
    pub ended: Option<i64>,
    /// Only its resolving was seen: when it began is not known.
    pub start_unknown: bool,
    /// How many times it was notified while firing, the first included.
    pub notices: u32,
    pub place: Place,
}

impl Alert {
    /// How long it fired (so far, if it still does). `None` for a job that
    /// failed, or a resolved alert whose beginning was not seen.
    pub fn lasted(&self, now: i64) -> Option<i64> {
        match self.state {
            State::Firing => Some((now - self.since).max(0)),
            State::Resolved if !self.start_unknown => self.ended.map(|e| (e - self.since).max(0)),
            _ => None,
        }
    }

    /// When it last changed: its newest message.
    pub fn changed(&self) -> i64 {
        ts_secs(&self.last_ts)
    }

    /// Whether it is still worth showing at `now`: firing always, the rest
    /// for `hours` after they happened.
    pub fn recent(&self, now: i64, hours: u64) -> bool {
        self.state == State::Firing || now - self.changed() <= hours as i64 * 3600
    }
}

/// Where an alert belongs, by the rules in settings.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Place {
    /// The repo (its path, or the last part of it, as the rule has it).
    pub repo: Option<String>,
    /// The service's name; empty when no rule matched.
    pub service: String,
}

impl Place {
    pub fn mapped(&self) -> bool {
        self.repo.is_some() || !self.service.is_empty()
    }
}

/// The word groups of the view use for what no rule placed.
pub const UNMAPPED: &str = "unmapped";

fn last_part(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Whether `rule` takes the alert: its prefix starts the name and its label
/// is carried; a rule with neither takes nothing.
fn matches(rule: &Rule, name: &str, labels: &BTreeMap<String, String>) -> bool {
    if rule.prefix.is_empty() && rule.label.is_empty() {
        return false;
    }
    let prefix = rule.prefix.is_empty() || name.to_lowercase().starts_with(&rule.prefix.to_lowercase());
    let label = match rule.label.split_once('=') {
        _ if rule.label.is_empty() => true,
        Some((k, v)) => labels
            .get(&k.trim().to_lowercase())
            .is_some_and(|got| got.eq_ignore_ascii_case(v.trim())),
        None => false,
    };
    prefix && label
}

/// The rule that places an alert, and where it puts it (the environment, if
/// the alert did not say, is the rule's).
pub fn place(settings: &AlertSettings, name: &str, labels: &BTreeMap<String, String>) -> (Place, Option<String>) {
    match settings.rules.iter().find(|r| matches(r, name, labels)) {
        Some(r) => {
            let service = if r.service.is_empty() { last_part(&r.repo).to_string() } else { r.service.clone() };
            let repo = (!r.repo.is_empty()).then(|| r.repo.clone());
            (Place { repo, service }, (!r.environment.is_empty()).then(|| r.environment.clone()))
        }
        None => (Place::default(), None),
    }
}

/// Whether a rule's repo is `id`, a repo's path: the same, or the last part
/// of it.
pub fn repo_is(rule_repo: &str, id: &str) -> bool {
    let (r, id) = (rule_repo.trim_matches('/').to_lowercase(), id.trim_matches('/').to_lowercase());
    !r.is_empty() && (id == r || id.ends_with(&format!("/{r}")))
}

/// What the held messages of the alerts channels make: the alerts, and each
/// message a reader refused with why.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Collected {
    /// Firing first (the longest first), then the rest, newest first.
    pub alerts: Vec<Alert>,
    /// Channel id, ts and why, for bot messages no reader could read.
    pub refused: Vec<(String, String, String)>,
    /// Alerts channels with no reader chosen, by alias.
    pub without_reader: Vec<String>,
}

pub fn collect(data: &Data, cfg: &Settings) -> Collected {
    let mut out = Collected::default();
    for ch in cfg.watched().filter(|c| c.kind == Kind::Alerts) {
        let Some(reader) = ch.reader else {
            out.without_reader.push(ch.alias.clone());
            continue;
        };
        let Some(held) = data.channels.get(&ch.id) else { continue };
        let mut msgs: Vec<&Message> = held.messages.iter().collect();
        msgs.sort_by_key(|m| ts_key(&m.ts));
        // Alerts firing now, by name and environment.
        let mut open: BTreeMap<(String, String), usize> = BTreeMap::new();
        for m in msgs {
            let r = read(reader, m);
            out.refused.extend(r.refused.into_iter().map(|why| (ch.id.clone(), m.ts.clone(), why)));
            for s in r.alerts {
                let (placed, env) = place(&cfg.alerts, &s.name, &s.labels);
                let environment = if s.environment.is_empty() { env.unwrap_or_default() } else { s.environment.clone() };
                let key = (s.name.to_lowercase(), environment.to_lowercase());
                let new = |start_unknown: bool, ended: Option<i64>| Alert {
                    name: s.name.clone(),
                    state: s.state,
                    environment: environment.clone(),
                    priority: s.priority.clone(),
                    summary: s.summary.clone(),
                    link: s.link.clone(),
                    labels: s.labels.clone(),
                    source: s.source,
                    channel: ch.id.clone(),
                    ts: m.ts.clone(),
                    last_ts: m.ts.clone(),
                    since: ts_secs(&m.ts),
                    ended,
                    start_unknown,
                    notices: 1,
                    place: placed.clone(),
                };
                match s.state {
                    State::Failed => out.alerts.push(new(false, None)),
                    State::Firing => match open.get(&key) {
                        Some(&i) => {
                            let a = &mut out.alerts[i];
                            a.last_ts = m.ts.clone();
                            a.notices += 1;
                            for (mine, theirs) in [(&mut a.priority, &s.priority), (&mut a.summary, &s.summary)] {
                                if !theirs.is_empty() {
                                    mine.clone_from(theirs);
                                }
                            }
                            if s.link.is_some() {
                                a.link.clone_from(&s.link);
                            }
                        }
                        None => {
                            open.insert(key, out.alerts.len());
                            out.alerts.push(new(false, None));
                        }
                    },
                    State::Resolved => match open.remove(&key) {
                        Some(i) => {
                            let a = &mut out.alerts[i];
                            a.state = State::Resolved;
                            a.ended = Some(ts_secs(&m.ts));
                            a.last_ts = m.ts.clone();
                        }
                        None => out.alerts.push(new(true, Some(ts_secs(&m.ts)))),
                    },
                }
            }
        }
    }
    out.alerts.sort_by_key(|a| match a.state {
        State::Firing => (0, a.since, 0),
        _ => (1, 0, -a.changed()),
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{Block, Card};
    use crate::config::{Channel, Kind};
    use crate::model::ChannelData;

    fn bot(ts: &str, cards: Vec<Card>) -> Message {
        Message { ts: ts.into(), bot_name: Some("alertbot".into()), cards, ..Message::default() }
    }

    fn card(color: &str, title: &str, text: &str) -> Card {
        Card { color: color.into(), title: title.into(), text: text.into(), ..Card::default() }
    }

    fn firing(env: &str) -> Card {
        card(
            "danger",
            "[FIRING:1] api-HighErrorRate",
            &format!(
                "*Summary:* Error rate above 5% for 10m\n*Environment:* {env}\n*Priority:* P1\n*Console:* <https://console.acme.example/alerts/42|open in the console>"
            ),
        )
    }

    fn alertmanager_one(c: Card) -> Result<Seen, String> {
        let r = read(Reader::Alertmanager, &bot("1.0", vec![c]));
        assert!(!r.person);
        match (r.alerts.into_iter().next(), r.refused.into_iter().next()) {
            (Some(s), _) => Ok(s),
            (None, why) => Err(why.expect("a reason")),
        }
    }

    #[test]
    fn the_alertmanager_reader_reads_state_name_and_the_fields_in_the_text() {
        let s = alertmanager_one(firing("prod")).unwrap();
        assert_eq!(
            (s.name.as_str(), s.state, s.environment.as_str(), s.priority.as_str()),
            ("api-HighErrorRate", State::Firing, "prod", "P1")
        );
        assert_eq!(s.summary, "Error rate above 5% for 10m");
        assert_eq!(s.link.as_deref(), Some("https://console.acme.example/alerts/42"));
        let r = alertmanager_one(card("good", "[RESOLVED] api-HighErrorRate", "*Environment:* prod")).unwrap();
        assert_eq!((r.state, r.environment.as_str()), (State::Resolved, "prod"));
    }

    #[test]
    fn the_alertmanager_reader_reads_real_fields_labels_and_a_count_in_the_state() {
        let mut c = card("#e01e5a", ":fire: [FIRING:3] DiskFull env=staging service=billing", "");
        c.fields = vec![("Priority".into(), "P2".into()), ("Console".into(), "<https://console.acme.example/d/9|dash>".into())];
        c.title_link = "https://alertmanager.acme.example/#/alerts".into();
        let s = alertmanager_one(c).unwrap();
        assert_eq!((s.name.as_str(), s.state, s.environment.as_str(), s.priority.as_str()), ("DiskFull", State::Firing, "staging", "P2"));
        assert_eq!(s.labels.get("service").map(String::as_str), Some("billing"));
        assert_eq!(s.link.as_deref(), Some("https://console.acme.example/d/9"), "the console field, not the manager's own page");
        // No console in the text: the title's link is what there is.
        let s = alertmanager_one({
            let mut c = card("danger", "[FIRING] X", "");
            c.title_link = "https://alertmanager.acme.example/x".into();
            c
        })
        .unwrap();
        assert_eq!(s.link.as_deref(), Some("https://alertmanager.acme.example/x"));
        // The labels list Alertmanager prints.
        let s = alertmanager_one(card("danger", "[FIRING:1] Y", "*Labels:*\n • alertname = Y\n • severity = critical\n • env = prod")).unwrap();
        assert_eq!((s.environment.as_str(), s.priority.as_str()), ("prod", "critical"));
    }

    #[test]
    fn the_alertmanager_reader_refuses_what_it_cannot_read_and_says_why() {
        let why = |c: Card| alertmanager_one(c).unwrap_err();
        assert!(why(card("danger", "Deploy finished", "")).contains("no [FIRING] or [RESOLVED]"));
        assert!(why(card("danger", "[PENDING] X", "")).contains("neither FIRING nor RESOLVED"));
        assert!(why(card("danger", "[FIRING]", "")).contains("no alert name"));
        assert!(why(card("danger", "", "text only")).contains("no title"));
        assert!(why(card("good", "[FIRING:1] X", "")).contains("the colour says the opposite"));
        assert!(why(card("#36a64f", "[FIRING:1] X", "")).contains("the colour says the opposite"));
        // A colour it does not know says nothing either way.
        assert!(alertmanager_one(card("#123456", "[FIRING:1] X", "")).is_ok());
        // A person's message is not an alert, and not a refusal.
        let person = Message { ts: "1.0".into(), user: Some("U1".into()), text: "[FIRING] looks bad".into(), ..Message::default() };
        assert_eq!(read(Reader::Alertmanager, &person), Reading { person: true, ..Reading::default() });
        // A bot with nothing in it.
        let r = read(Reader::Alertmanager, &Message { ts: "1.0".into(), bot_name: Some("b".into()), ..Message::default() });
        assert_eq!(r.refused.len(), 1);
        // Two attachments: one read, one refused.
        let r = read(Reader::Alertmanager, &bot("1.0", vec![firing("prod"), card("danger", "Hello", "")]));
        assert_eq!((r.alerts.len(), r.refused.len()), (1, 1));
    }

    fn job_block(header: &str, rest: &[&str]) -> Message {
        let mut blocks = vec![Block { kind: "header".into(), texts: vec![header.into()] }];
        blocks.extend(rest.iter().map(|t| Block { kind: "context".into(), texts: vec![t.to_string()] }));
        Message { ts: "1.0".into(), bot_name: Some("jobbot".into()), blocks, ..Message::default() }
    }

    fn job(m: &Message) -> Result<Seen, String> {
        let r = read(Reader::JobFailed, m);
        r.alerts.into_iter().next().ok_or_else(|| r.refused.into_iter().next().unwrap_or_default())
    }

    #[test]
    fn the_job_failed_reader_reads_the_header_and_the_context() {
        let s = job(&job_block(
            ":x: Nightly export failed",
            &["Environment: prod | <https://ci.acme.example/runs/9|run 9>", "Error: disk quota exceeded"],
        ))
        .unwrap();
        assert_eq!((s.name.as_str(), s.state, s.environment.as_str()), ("Nightly export", State::Failed, "prod"));
        assert_eq!(s.link.as_deref(), Some("https://ci.acme.example/runs/9"));
        assert_eq!(s.summary, "disk quota exceeded");
        // The job named in the details wins; other headers' shapes.
        assert_eq!(job(&job_block("Job failed: nightly-export", &[])).unwrap().name, "nightly-export");
        assert_eq!(job(&job_block("Build nightly-export failed", &[])).unwrap().name, "Build nightly-export");
        assert_eq!(job(&job_block("Job nightly-export failed", &[])).unwrap().name, "nightly-export");
        assert_eq!(job(&job_block("Something failed", &["Job: reindex"])).unwrap().name, "reindex");
    }

    #[test]
    fn the_job_failed_reader_refuses_what_is_not_a_failure_header() {
        assert!(job(&job_block("Nightly export succeeded", &[])).unwrap_err().contains("does not say a job failed"));
        assert!(job(&job_block("failed", &[])).unwrap_err().contains("no job name"));
        assert!(job(&Message { ts: "1.0".into(), bot_name: Some("b".into()), text: "export failed".into(), ..Message::default() })
            .unwrap_err()
            .contains("no header block"));
        // `failed` inside another word is not a failure.
        assert!(job(&job_block("Unfailed run", &[])).is_err());
        // A person pasting a header is not a bot.
        let mut person = job_block("X failed", &[]);
        person.bot_name = None;
        person.user = Some("U1".into());
        person.blocks.clear();
        assert!(read(Reader::JobFailed, &person).person);
    }

    fn status_of(text: &str) -> Result<Seen, String> {
        let m = Message { ts: "1.0".into(), bot_name: Some("statusbot".into()), text: text.into(), ..Message::default() };
        let r = read(Reader::Status, &m);
        r.alerts.into_iter().next().ok_or_else(|| r.refused.into_iter().next().unwrap_or_default())
    }

    #[test]
    fn the_status_reader_reads_a_bot_message_with_a_status_word() {
        let s = status_of("[FIRING] checkout latency\nEnvironment: prod\nhttps://grafana.acme.example/d/1").unwrap();
        assert_eq!((s.name.as_str(), s.state, s.environment.as_str()), ("checkout latency", State::Firing, "prod"));
        assert_eq!(s.link.as_deref(), Some("https://grafana.acme.example/d/1"));
        let s = status_of("checkout latency: RECOVERED").unwrap();
        assert_eq!((s.name.as_str(), s.state), ("checkout latency", State::Resolved));
        assert_eq!(status_of(":rotating_light: Deploy of web FAILED").unwrap().state, State::Firing);
        assert_eq!(status_of("Deploy of web succeeded").unwrap().name, "Deploy of web");
    }

    #[test]
    fn the_status_reader_refuses_no_word_two_kinds_of_word_no_name_and_a_word_inside_a_word() {
        assert!(status_of("lunch is at noon").unwrap_err().contains("no status word"));
        assert!(status_of("download finished").unwrap_err().contains("no status word"), "down is not in download");
        assert!(status_of("api is DOWN and then UP").unwrap_err().contains("both firing and resolved"));
        assert!(status_of("[FIRING]").unwrap_err().contains("no name"));
        assert!(status_of("").is_err());
        let person = Message { ts: "1.0".into(), user: Some("U1".into()), text: "api is down".into(), ..Message::default() };
        assert!(read(Reader::Status, &person).person);
    }

    fn held(msgs: Vec<Message>) -> (Data, Settings) {
        let mut cfg = Settings::default();
        cfg.channels.push(Channel {
            alias: "team-alerts".into(),
            id: "C0000ALRT1".into(),
            name: "team-alerts".into(),
            purpose: String::new(),
            kind: Kind::Alerts,
            color: "orange".into(),
            watched: true,
            reader: Some(Reader::Alertmanager),
        });
        let mut data = Data::default();
        data.channels.insert("C0000ALRT1".into(), ChannelData { messages: msgs, ..ChannelData::default() });
        (data, cfg)
    }

    #[test]
    fn firing_and_resolved_of_one_alert_pair_with_how_long_it_fired() {
        let (data, cfg) = held(vec![
            bot("1000.000001", vec![firing("prod")]),
            bot("1600.000001", vec![firing("prod")]),
            bot("4600.000001", vec![card("good", "[RESOLVED] api-HighErrorRate", "*Environment:* prod")]),
        ]);
        let c = collect(&data, &cfg);
        assert_eq!(c.alerts.len(), 1, "{:?}", c.alerts);
        let a = &c.alerts[0];
        assert_eq!((a.state, a.since, a.ended, a.notices), (State::Resolved, 1000, Some(4600), 2));
        assert_eq!(a.lasted(9999), Some(3600));
        assert_eq!(a.ts, "1000.000001", "its thread is the first message");
        assert_eq!(a.link.as_deref(), Some("https://console.acme.example/alerts/42"));
    }

    #[test]
    fn the_same_name_in_another_environment_is_another_alert_and_a_firing_one_keeps_counting() {
        let (data, cfg) = held(vec![
            bot("1000.000001", vec![firing("prod")]),
            bot("1100.000001", vec![firing("staging")]),
            bot("1200.000001", vec![card("good", "[RESOLVED] api-HighErrorRate", "*Environment:* staging")]),
        ]);
        let c = collect(&data, &cfg);
        assert_eq!(c.alerts.len(), 2);
        assert_eq!((c.alerts[0].state, c.alerts[0].environment.as_str()), (State::Firing, "prod"), "firing first");
        assert_eq!(c.alerts[0].lasted(1300), Some(300));
        assert_eq!((c.alerts[1].state, c.alerts[1].environment.as_str()), (State::Resolved, "staging"));
    }

    #[test]
    fn a_resolved_alert_whose_firing_was_not_read_has_no_duration_and_a_refusal_is_collected() {
        let (data, cfg) = held(vec![
            bot("2000.000001", vec![card("good", "[RESOLVED] OldAlert", "")]),
            bot("2100.000001", vec![card("danger", "Hello from the bot", "")]),
        ]);
        let c = collect(&data, &cfg);
        assert_eq!(c.alerts.len(), 1);
        assert!(c.alerts[0].start_unknown && c.alerts[0].lasted(9999).is_none());
        assert_eq!(c.refused.len(), 1);
        assert_eq!((c.refused[0].0.as_str(), c.refused[0].1.as_str()), ("C0000ALRT1", "2100.000001"));
        assert!(c.refused[0].2.contains("no [FIRING] or [RESOLVED]"));
    }

    #[test]
    fn jobs_that_failed_are_alerts_of_their_own_and_a_channel_with_no_reader_reads_nothing() {
        let (mut data, mut cfg) = held(vec![job_block("Nightly export failed", &["Environment: prod"])]);
        cfg.channels[0].reader = Some(Reader::JobFailed);
        let c = collect(&data, &cfg);
        assert_eq!((c.alerts.len(), c.alerts[0].state), (1, State::Failed));
        assert!(c.alerts[0].lasted(9999).is_none());
        cfg.channels[0].reader = None;
        let c = collect(&data, &cfg);
        assert!(c.alerts.is_empty() && c.without_reader == ["team-alerts"]);
        // Not an alerts channel: not read.
        cfg.channels[0].kind = Kind::Team;
        cfg.channels[0].reader = Some(Reader::JobFailed);
        data.channels.get_mut("C0000ALRT1").unwrap().messages.push(job_block("X failed", &[]));
        assert!(collect(&data, &cfg).alerts.is_empty());
    }

    #[test]
    fn rules_place_an_alert_by_prefix_or_label_first_match_wins_and_the_rest_are_unmapped() {
        let rule = |prefix: &str, label: &str, repo: &str, service: &str, env: &str| Rule {
            prefix: prefix.into(),
            label: label.into(),
            repo: repo.into(),
            service: service.into(),
            environment: env.into(),
        };
        let cfg = AlertSettings {
            recent_hours: 24,
            rules: vec![
                rule("api-", "", "acme/team-a/api", "", ""),
                rule("", "service=billing", "acme/team-a/billing", "billing", "prod"),
                rule("db-", "env=prod", "", "database", ""),
                rule("", "", "acme/nothing", "", ""),
            ],
        };
        let labels = |kv: &[(&str, &str)]| kv.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<BTreeMap<_, _>>();
        let (p, env) = place(&cfg, "API-HighErrors", &labels(&[]));
        assert_eq!((p.repo.as_deref(), p.service.as_str(), env), (Some("acme/team-a/api"), "api", None), "any case, a repo names its own service");
        let (p, env) = place(&cfg, "Slow", &labels(&[("service", "Billing")]));
        assert_eq!((p.repo.as_deref(), p.service.as_str(), env.as_deref()), (Some("acme/team-a/billing"), "billing", Some("prod")));
        let (p, _) = place(&cfg, "db-Full", &labels(&[("env", "prod")]));
        assert_eq!((p.repo, p.service.as_str()), (None, "database"), "a service with no repo");
        assert!(place(&cfg, "db-Full", &labels(&[("env", "dev")])).0.service.is_empty(), "both must hold");
        let (p, _) = place(&cfg, "Unknown", &labels(&[]));
        assert!(!p.mapped(), "a rule with neither matches nothing");
    }

    #[test]
    fn an_environment_a_rule_gives_fills_in_only_when_the_alert_does_not_say() {
        let (data, mut cfg) = held(vec![
            bot("1000.000001", vec![card("danger", "[FIRING] api-A", "")]),
            bot("1001.000001", vec![firing("staging")]),
        ]);
        cfg.alerts.rules.push(Rule { prefix: "api-".into(), repo: "acme/team-a/api".into(), environment: "prod".into(), ..Rule::default() });
        let c = collect(&data, &cfg);
        let env = |name: &str| c.alerts.iter().find(|a| a.name == name).unwrap().environment.clone();
        assert_eq!((env("api-A").as_str(), env("api-HighErrorRate").as_str()), ("prod", "staging"));
    }

    #[test]
    fn a_rules_repo_is_a_repos_path_or_the_last_part_of_it() {
        assert!(repo_is("acme/team-a/api", "acme/team-a/api"));
        assert!(repo_is("api", "acme/team-a/api"));
        assert!(repo_is("Team-A/API", "acme/team-a/api"));
        assert!(!repo_is("pi", "acme/team-a/api"));
        assert!(!repo_is("acme/api", "acme/team-a/api"));
        assert!(!repo_is("", "acme/team-a/api"));
    }
}
