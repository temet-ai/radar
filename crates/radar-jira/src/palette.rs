//! Site-qualified cached tickets, without URL or credential resolution.

use std::collections::BTreeSet;

use radar_core::{palette::PaletteTarget, ui::icon::Icon};

use crate::{JiraPlugin, join};

pub(crate) fn targets(p: &JiraPlugin) -> Vec<PaletteTarget> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let mut add = |site: &str, key: &str, ticket: &crate::model::Ticket| {
        if ticket.key != key
            || !crate::config::is_ticket_key(key)
            || p.cfg.site(site).is_none()
            || !seen.insert((site.to_string(), key.to_string()))
        {
            return;
        }
        let mut subject = join::cached_ticket(p, site, key);
        subject.label = format!("{key} · {site} · {}", ticket.summary);
        let mut target = PaletteTarget::scoped(subject, site);
        target.words = vec![
            ticket.summary.clone(),
            ticket.project.clone(),
            site.into(),
            "ticket".into(),
        ];
        target.icon = Some(Icon::Issue);
        out.push(target);
    };
    // Pages carry the freshest copy; persisted pages count only for their
    // original configured address, just as page::fetch restores them.
    for (id, fetched) in &p.details {
        if let (Some((site, key)), Some(detail)) = (id.split_once('\0'), fetched.value.as_ref()) {
            add(site, key, &detail.ticket);
        }
    }
    for (site, data) in &p.data {
        for (key, ticket) in &data.tickets {
            add(site, key, ticket);
        }
        for (key, stored) in &data.details {
            if p.cfg
                .site(site)
                .is_some_and(|configured| configured.url == stored.url)
            {
                add(site, key, &stored.detail.ticket);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SiteData, config::Site, test_support::TicketBuilder};
    use radar_core::Plugin;

    fn site(name: &str) -> Site {
        Site {
            name: name.into(),
            url: "env:RADAR_PALETTE_MUST_NOT_RESOLVE".into(),
            email: "env:RADAR_PALETTE_MUST_NOT_RESOLVE".into(),
            token: "env:RADAR_PALETTE_MUST_NOT_RESOLVE".into(),
            color: "blue".into(),
            projects: vec![],
            from: String::new(),
        }
    }

    #[test]
    fn cached_ticket_keys_remain_distinct_by_site_without_resolving_urls() {
        let mut p = JiraPlugin::default();
        p.cfg.sites = vec![site("watch"), site("lancre")];
        let ticket = TicketBuilder::default().build();
        for name in ["watch", "lancre"] {
            p.data.insert(
                name.into(),
                SiteData {
                    tickets: [(ticket.key.clone(), ticket.clone())].into(),
                    ..Default::default()
                },
            );
        }
        p.details.insert(
            crate::detail_key("watch", &ticket.key),
            crate::Fetched {
                value: Some(crate::model::Detail {
                    ticket: ticket.clone(),
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
        let targets = p.palette_targets();
        assert_eq!(targets.len(), 2);
        assert_ne!(targets[0].identity, targets[1].identity);
        let identities: Vec<_> = targets
            .iter()
            .map(|target| target.identity.clone())
            .collect();
        p.details
            .get_mut(&crate::detail_key("watch", &ticket.key))
            .unwrap()
            .value
            .as_mut()
            .unwrap()
            .ticket
            .summary = "Inspect the Watch bell".into();
        p.data
            .get_mut("lancre")
            .unwrap()
            .tickets
            .get_mut(&ticket.key)
            .unwrap()
            .status = "Done".into();
        assert_eq!(
            p.palette_targets()
                .into_iter()
                .map(|target| target.identity)
                .collect::<Vec<_>>(),
            identities
        );
        let sites: BTreeSet<_> = targets
            .iter()
            .map(|target| target.subject.field("site").unwrap())
            .collect();
        assert_eq!(sites, BTreeSet::from(["watch", "lancre"]));
        for target in targets {
            assert_eq!(target.subject.id, "WATCH-12");
            assert!(target.subject.url.is_none());
            assert!(target.words.contains(&"Patrol the Shades".into()));
            let route = crate::subject_scope::resolve(&p, &target.subject).unwrap();
            assert_eq!(Some(route.name.as_str()), target.subject.field("site"));
        }
    }

    #[test]
    fn removed_sites_invalid_keys_and_unloaded_tickets_are_absent() {
        let mut p = JiraPlugin::default();
        p.cfg.sites = vec![site("watch")];
        assert!(p.palette_targets().is_empty());
        let ticket = TicketBuilder::default().build();
        p.data.insert(
            "removed".into(),
            SiteData {
                tickets: [(ticket.key.clone(), ticket.clone())].into(),
                ..Default::default()
            },
        );
        p.data.insert(
            "watch".into(),
            SiteData {
                tickets: [("WATCH-99".into(), ticket)].into(),
                ..Default::default()
            },
        );
        assert!(p.palette_targets().is_empty());
    }
    #[test]
    fn stored_ticket_page_uses_its_exact_configured_site_route() {
        let mut p = JiraPlugin::default();
        let mut configured = site("watch");
        configured.url = "https://watch.example.test".into();
        p.cfg.sites.push(configured.clone());
        let ticket = TicketBuilder::default().build();
        p.data.insert(
            "watch".into(),
            SiteData {
                details: [(
                    ticket.key.clone(),
                    crate::DetailEntry {
                        url: configured.url,
                        detail: crate::model::Detail {
                            ticket,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                )]
                .into(),
                ..Default::default()
            },
        );
        let targets = p.palette_targets();
        assert_eq!(targets.len(), 1);
        let radar_core::Showing::Page { screen, effects } = p.show(&targets[0].subject) else {
            panic!("cached ticket opens")
        };
        assert_eq!(screen, crate::SCREEN.id);
        let page = p.pages.last().unwrap();
        assert_eq!(page.site, "watch");
        assert_eq!(page.key, "WATCH-12");
        // Any refresh is an unevaluated effect; this test executes no provider.
        drop(effects);
        p.cfg.sites[0].url = "https://lancre.example.test".into();
        p.details.clear();
        assert!(p.palette_targets().is_empty());
    }
}
