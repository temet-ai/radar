//! What a Jira ticket tells the host so an agent can be briefed on it (send
//! to agent).
//!
//! Everything here is read from what the plugin already holds (the ticket in
//! the list, and its page when one was read); nothing is fetched to answer
//! and nothing is written. The summary, description, acceptance criteria and
//! comments are given as they are.

use radar_core::agent_context::{AgentContext, AgentContextRequest, ContextField};
use radar_core::journal::SubjectKey;
use radar_core::plugin::Effect;
use radar_core::ui::rich;

use crate::config::Site;
use crate::model::Ticket;
use crate::{JiraPlugin, join, screen};

/// The words the brief heads the quoted material with.
pub(crate) const KIND: &str = "Jira ticket";
/// Comments a context carries, the newest ones; the rest are counted.
pub(crate) const MAX_COMMENTS: usize = 5;
/// The key a row binds, the same on every part's rows.
pub(crate) const KEY: char = 'A';

/// The scoped identity of a ticket: the site's address, and its key (the
/// same one the journal uses for it).
pub(crate) fn key(site: &Site, ticket: &str) -> Option<SubjectKey> {
    SubjectKey::new("jira", &site.base_url().ok()?, join::TICKET, ticket).ok()
}

/// A document's words, laid out wide enough that nothing wraps.
fn words(doc: &rich::Doc) -> String {
    rich::plain(doc, 1000).trim().to_string()
}

/// The ticket held for `site`/`key`, from the list the plugin keeps.
fn held<'a>(p: &'a JiraPlugin, site: &Site, key: &str) -> Option<&'a Ticket> {
    p.data.get(&site.name)?.tickets.get(key)
}

/// The context for one ticket.
pub(crate) fn context(p: &JiraPlugin, site: &Site, t: &Ticket) -> Option<AgentContext> {
    let subject = key(site, &t.key)?;
    let mut c = AgentContext::new(KIND, subject, &format!("{} {}", t.key, t.summary))
        .link(site.browse(&t.key))
        .field(ContextField::new("key", &t.key))
        .field(ContextField::new("summary", &t.summary))
        .field(ContextField::new("type", &t.kind))
        .field(ContextField::new("status", &t.status));
    for (name, value) in [
        ("priority", &t.priority),
        ("assignee", &t.assignee),
        ("reporter", &t.reporter),
        ("project", &t.project),
    ] {
        if !value.is_empty() {
            c = c.field(ContextField::new(name, value));
        }
    }
    if let Some(parent) = &t.parent {
        c = c.field(ContextField::new("parent", format!("{} {}", parent.key, parent.summary)));
    }
    for (name, list) in [("labels", &t.labels), ("components", &t.components)] {
        if !list.is_empty() {
            c = c.field(ContextField::new(name, list.join(", ")));
        }
    }
    if !t.description.is_empty() {
        c = c.field(ContextField::new("description", words(&t.description)));
    }
    if !t.ac.is_empty() {
        c = c.field(ContextField::new("acceptance_criteria", words(&t.ac)));
    }
    // The page's reading adds the newest comments and the merge requests Jira
    // links; a ticket whose page was never opened has neither.
    if let Some(detail) = p.details.get(&format!("{}\0{}", site.name, t.key)).and_then(|f| f.value.as_ref()) {
        if let Some(sprint) = &detail.sprint {
            c = c.field(ContextField::new("sprint", sprint));
        }
        let shown = detail.comments.iter().take(MAX_COMMENTS);
        let lines: Vec<String> = shown.map(|m| format!("{}: {}", m.author, words(&m.body))).collect();
        if !lines.is_empty() {
            c = c.field(ContextField::new("newest_comments", lines.join("\n\n")));
            let left_out = detail.comments_total.saturating_sub(lines.len());
            if left_out > 0 {
                c = c.field(ContextField::new("older_comments_left_out", left_out.to_string()));
            }
        }
        let code: Vec<String> = detail
            .code
            .iter()
            .map(|l| format!("{} {} ({}): {}", l.id, l.title, l.status, l.url))
            .collect();
        if !code.is_empty() {
            c = c.field(ContextField::new("linked_merge_requests", code.join("\n")));
        }
    }
    Some(c)
}

/// Asked of the plugin by the host for a ticket key of one of its sites.
pub(crate) fn agent_context(p: &JiraPlugin, request: &AgentContextRequest) -> Option<AgentContext> {
    let s = &request.subject;
    if s.provider != "jira" || s.kind != join::TICKET {
        return None;
    }
    // Compare as keys: the instance is stored in its safe form.
    let site = p.cfg.sites.iter().find(|site| key(site, &s.id).is_some_and(|k| k == *s))?;
    let t = held(p, site, &s.id)?;
    context(p, site, t)
}

/// The ticket the keys act on: the open page's, else the selected row's.
fn current(p: &JiraPlugin) -> Option<(&Site, &Ticket)> {
    if let Some(page) = p.pages.last() {
        let site = p.cfg.site(&page.site)?;
        return Some((site, held(p, site, &page.key)?));
    }
    match screen::selected(p)? {
        screen::Selected::Ticket(it) => Some((it.site, it.t)),
        _ => None,
    }
}

/// Whether a ticket is in hand, so the key is offered.
pub(crate) fn can_send(p: &JiraPlugin) -> bool {
    current(p).is_some_and(|(site, t)| key(site, &t.key).is_some())
}

/// What the key asks of the host.
pub(crate) fn send_selected(p: &JiraPlugin) -> Vec<Effect> {
    match current(p).and_then(|(site, t)| key(site, &t.key)) {
        Some(subject) => vec![Effect::SendToAgent(AgentContextRequest { subject })],
        None => vec![Effect::Notify("select a ticket to ask an agent about it".into())],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::demo;

    fn plugin() -> JiraPlugin {
        let mut p = demo::sample();
        p.clamp();
        p
    }

    fn first_ticket(p: &JiraPlugin) -> (&Site, &Ticket) {
        let site = &p.cfg.sites[0];
        let t = p.data.get(&site.name).unwrap().tickets.values().next().expect("a held ticket");
        (site, t)
    }

    fn field<'a>(c: &'a AgentContext, name: &str) -> &'a ContextField {
        c.fields.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("no {name}"))
    }

    #[test]
    fn a_ticket_context_carries_the_ticket_fields() {
        let p = plugin();
        let (site, t) = first_ticket(&p);
        let c = context(&p, site, t).unwrap();
        assert_eq!(c.kind, KIND);
        assert_eq!(c.subject, key(site, &t.key).unwrap());
        assert_eq!(c.link, site.browse(&t.key));
        assert_eq!(field(&c, "key").value, t.key);
        assert_eq!(field(&c, "status").value, t.status);
        assert_eq!(field(&c, "summary").value, t.summary);
        if !t.description.is_empty() {
            assert!(c.fields.iter().any(|f| f.name == "description"));
        }
    }

    #[test]
    fn the_host_asks_by_key_and_gets_only_a_held_ticket_of_a_known_site() {
        let p = plugin();
        let (site, t) = first_ticket(&p);
        let ask = |k: SubjectKey| agent_context(&p, &AgentContextRequest { subject: k });
        assert_eq!(ask(key(site, &t.key).unwrap()).unwrap().title, format!("{} {}", t.key, t.summary));
        assert!(ask(key(site, "NOPE-1").unwrap()).is_none(), "a ticket nothing holds");
        let other = SubjectKey::new("jira", "https://other.example.test", join::TICKET, &t.key).unwrap();
        assert!(ask(other).is_none(), "a site that is not configured");
        let slack = SubjectKey::new("chat", "T1", "message", "C1/1.2").unwrap();
        assert!(ask(slack).is_none());
    }

    #[test]
    fn the_key_sends_the_selected_ticket_and_nothing_else() {
        let p = plugin();
        let site = &p.cfg.sites[0];
        p.tree
            .borrow_mut()
            .select(vec!["s:acme".into(), "g:acme:In Progress".into(), "t:acme:ABC-1".into()]);
        assert!(can_send(&p));
        match send_selected(&p).as_slice() {
            [Effect::SendToAgent(r)] => assert_eq!(r.subject, key(site, "ABC-1").unwrap()),
            other => panic!("{other:?}"),
        }
        // a site row has no ticket
        p.tree.borrow_mut().select(vec![screen::site_id(&site.name)]);
        assert!(!can_send(&p));
        assert!(matches!(send_selected(&p).as_slice(), [Effect::Notify(_)]));
    }

    #[test]
    fn the_key_is_offered_on_a_ticket_and_runs_by_the_shared_action_id() {
        let mut p = plugin();
        p.tree
            .borrow_mut()
            .select(vec!["s:acme".into(), "g:acme:In Progress".into(), "t:acme:ABC-1".into()]);
        let rows = p.screen_rows(radar_core::plugin::Pane::Middle);
        let (id, b) = rows.iter().find(|(_, b)| b.keys == "A").expect("A is listed on a ticket");
        assert_eq!(id, radar_core::agent_context::SEND_TO_AGENT_ACTION);
        assert_eq!(b.what, "ask an agent about it…");
        let out = radar_core::Plugin::intent(
            &mut p,
            crate::SCREEN.id,
            radar_core::plugin::Pane::Middle,
            &radar_core::view::Intent::Invoke { action: id.clone() },
        );
        let radar_core::plugin::Outcome::Handled(fx) = out else { panic!("{out:?}") };
        assert!(matches!(fx.as_slice(), [Effect::SendToAgent(_)]), "{fx:?}");
        // on a site row it is not offered
        p.tree.borrow_mut().select(vec!["s:acme".into()]);
        assert!(p.screen_rows(radar_core::plugin::Pane::Middle).iter().all(|(_, b)| b.keys != "A"));
    }
}
