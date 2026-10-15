//! The Alerts view of the Slack screen: what the alerts channels' readers
//! made of their messages (see `alerts.rs`), firing first, then failed jobs,
//! then what resolved lately, each under its service and environment (what
//! no rule in settings placed under `unmapped`), with how long each has
//! fired. `⏎` opens an alert's thread, `o` its console link, `m` maps an
//! unmapped one. Nothing here is sent to Slack: the thread is read, the
//! mapping is a line of local settings.


use radar_core::input::Edit;
use radar_core::plugin::{Binding, Effect, Outcome};
use radar_core::ratatui::crossterm::event::KeyEvent;

use crate::alerts::{Alert, State};
use crate::config::Rule;
use crate::{Mapping, SlackPlugin, page};

/// How long, in the kit's form: `45s`, `4m 05s`, `3h 05m`.
pub(crate) fn lasted(secs: i64) -> String {
    radar_core::ui::duration(secs.max(0) as u64)
}

/// `24 hours`, `1 hour`, `3 days`.
pub(crate) fn hours(h: u64) -> String {
    match h {
        1 => "1 hour".into(),
        h if h >= 48 && h % 24 == 0 => format!("{} days", h / 24),
        h => format!("{h} hours"),
    }
}

/// The alerts worth showing now: firing, and the rest while recent.
pub(crate) fn shown(p: &SlackPlugin) -> Vec<Alert> {
    let hours = p.cfg.alerts.recent_hours;
    p.alerts().alerts.iter().filter(|a| a.recent(p.now, hours)).cloned().collect()
}

/// How many are firing now.
pub(crate) fn firing(p: &SlackPlugin) -> usize {
    p.alerts().alerts.iter().filter(|a| a.state == State::Firing).count()
}

/// Whether the nav pane has an Alerts row: some watched channel is for alerts.
pub(crate) fn has_channel(p: &SlackPlugin) -> bool {
    p.cfg.watched().any(|c| c.kind == crate::config::Kind::Alerts)
}

/// The alert a row is, as held.
pub(crate) fn find(p: &SlackPlugin, channel: &str, ts: &str) -> Option<Alert> {
    p.alerts().alerts.iter().find(|a| a.channel == channel && a.ts == ts).cloned()
}

/// What the nav pane's footer and the empty tree say about what is read.
pub(crate) fn about(p: &SlackPlugin) -> Vec<String> {
    let c = p.alerts();
    let mut out = Vec::new();
    if !c.without_reader.is_empty() {
        out.push(format!(
            "#{} has no reader: choose one under channels in settings.",
            c.without_reader.join(", #")
        ));
    }
    if !c.refused.is_empty() {
        out.push(format!(
            "{} bot message{} not read as alerts: the log says why.",
            c.refused.len(),
            if c.refused.len() == 1 { "" } else { "s" }
        ));
    }
    out
}

/// `x` rows of an alert.
pub(crate) fn rows(p: &SlackPlugin, channel: &str, ts: &str) -> Vec<crate::Row> {
    use crate::{act, head};
    let Some(a) = find(p, channel, ts) else {
        return vec![head(Binding::heading("alert"))];
    };
    let mut b = vec![
        head(Binding::heading(a.name.clone()).icon(a.state.icon())),
        act("slack.feed.open_alert_thread", Binding::new("⏎", "open its thread")),
    ];
    if a.link.is_some() {
        b.push(act("slack.feed.open_alert_console", Binding::new("o", "open its console link")));
        b.push(act("slack.feed.copy_alert_console", Binding::new("y", "copy its console link")));
    }
    if !a.place.mapped() {
        b.push(act("slack.feed.map_alert", Binding::new("m", "map it to a repo or service…")));
    }
    b
}

/// `o`, `y` and `m` on an alert.
pub(crate) fn key(p: &mut SlackPlugin, channel: &str, ts: &str, key: KeyEvent) -> Option<Outcome> {
    use radar_core::ratatui::crossterm::event::KeyCode;
    let a = find(p, channel, ts)?;
    let no_link = || Outcome::Handled(vec![Effect::Notify("this alert has no console link".into())]);
    match key.code {
        KeyCode::Char('o') => Some(match a.link {
            Some(url) => Outcome::Handled(vec![Effect::Open(url)]),
            None => no_link(),
        }),
        KeyCode::Char('y') => Some(match a.link {
            Some(url) => Outcome::Handled(vec![Effect::Copy(url), Effect::Notify("console link copied".into())]),
            None => no_link(),
        }),
        KeyCode::Char('m') if !a.place.mapped() => {
            p.mapping = Some(Mapping { name: a.name, input: radar_core::input::TextInput::new("") });
            Some(Outcome::done())
        }
        _ => None,
    }
}

// --- mapping an alert -------------------------------------------------------------------------

/// What was typed or pasted as a repo or a service: a path or a name, from
/// a link to the repo or to something in it.
pub(crate) fn repo_path(typed: &str) -> Result<String, String> {
    let typed = typed.trim().trim_end_matches('/');
    if typed.is_empty() {
        return Err("a repo's path (acme/team-a/api) or a service's name".into());
    }
    if typed.contains(char::is_whitespace) {
        return Err("no spaces: a repo's path (acme/team-a/api) or a service's name".into());
    }
    let path = match typed.strip_prefix("https://").or_else(|| typed.strip_prefix("http://")) {
        Some(rest) => rest.split_once('/').map_or("", |(_, path)| path).trim_end_matches('/'),
        None => typed,
    };
    let path = path.split("/-/").next().unwrap_or(path);
    if path.is_empty() {
        return Err("a repo's path (acme/team-a/api) or a service's name".into());
    }
    Ok(path.to_string())
}

/// What typed text maps to: a repo (a path with a slash, or a link to it)
/// or a service (a name).
pub(crate) fn rule_for(name: &str, typed: &str) -> Result<Rule, String> {
    let path = repo_path(typed)?;
    let mut rule = Rule { prefix: name.to_string(), ..Rule::default() };
    if path.contains('/') {
        rule.repo = path.to_string();
    } else {
        rule.service = path.to_string();
    }
    Ok(rule)
}

/// Put a rule in force: the newest first, so it wins over a broader one; a
/// rule for the same prefix alone is replaced.
pub(crate) fn add_rule(p: &mut SlackPlugin, rule: Rule) {
    p.cfg
        .alerts
        .rules
        .retain(|r| !(r.prefix == rule.prefix && r.label.is_empty() && r.environment.is_empty()));
    p.cfg.alerts.rules.insert(0, rule);
}

/// A key while the mapping is typed.
pub(crate) fn map_key(p: &mut SlackPlugin, key: KeyEvent) -> Outcome {
    let Some(m) = p.mapping.as_mut() else {
        return Outcome::Ignored;
    };
    match m.input.key(key) {
        Edit::Cancel => {
            p.mapping = None;
            Outcome::Handled(vec![Effect::Notify("nothing mapped".into())])
        }
        Edit::Typing => Outcome::done(),
        Edit::Submit(text) => match rule_for(&m.name, &text) {
            Err(why) => Outcome::Handled(vec![Effect::Notify(format!("not yet: {why}"))]),
            Ok(rule) => {
                p.mapping = None;
                let said = match (&rule.repo, &rule.service) {
                    (r, _) if !r.is_empty() => format!("repo {r}"),
                    (_, s) => format!("service {s}"),
                };
                let name = rule.prefix.clone();
                add_rule(p, rule);
                Outcome::Handled(vec![Effect::Notify(format!(
                    "alerts named {name}… now belong to {said}; settings, alerts has the rule"
                ))])
            }
        },
    }
}

/// The first place a page can open from: used by the page code for a row.
pub(crate) fn open(p: &mut SlackPlugin, channel: &str, ts: &str) -> Vec<Effect> {
    let fx = page::open_thread(p, channel, ts);
    p.data.read_to(channel, ts, ts);
    fx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn how_long_reads_short() {
        assert_eq!(lasted(10), "10s");
        assert_eq!(lasted(4 * 60 + 5), "4m 05s");
        assert_eq!(lasted(3 * 3600 + 5 * 60), "3h 05m");
        assert_eq!(lasted(-5), "0s");
    }

    #[test]
    fn typing_a_repo_or_a_service_makes_the_rule_and_refuses_what_is_neither() {
        let r = rule_for("api-HighErrors", " acme/team-a/api/ ").unwrap();
        assert_eq!((r.prefix.as_str(), r.repo.as_str(), r.service.as_str()), ("api-HighErrors", "acme/team-a/api", ""));
        let r = rule_for("db-Full", "database").unwrap();
        assert_eq!((r.repo.as_str(), r.service.as_str()), ("", "database"));
        // A link to the repo, as pasted from the browser.
        let r = rule_for("x", "https://git.example/acme/team-a/api/-/merge_requests/3").unwrap();
        assert_eq!(r.repo, "acme/team-a/api");
        let r = rule_for("x", "https://git.example/acme/team-a/api").unwrap();
        assert_eq!(r.repo, "acme/team-a/api");
        assert!(rule_for("x", "").is_err());
        assert!(rule_for("x", "two words").unwrap_err().contains("no spaces"));
        assert!(rule_for("x", "https://git.example/").is_err());
    }
}
