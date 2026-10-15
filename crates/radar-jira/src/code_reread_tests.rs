//! A merge request linked to a ticket without touching the ticket shows up
//! because the development panels of the tickets in view are read again
//! every half hour (open tickets only), and all of them on `r`.

use super::*;
use crate::model::Category;

/// Every ticket in the current view has its panel held, read at `at`, as of
/// its `updated`. One of them (ABC-20) is done. Their `site\0key`s.
fn held_panels(p: &mut JiraPlugin, at: i64) -> Vec<String> {
    p.data.get_mut("acme").unwrap().tickets.get_mut("ABC-20").unwrap().category = Category::Done;
    p.sync.in_flight.clear();
    let v = p.current_view().cloned().unwrap();
    let mut ids = Vec::new();
    for i in 0..p.cfg.sites.len() {
        let Some(jql) = p.jql_for(i, &v) else { continue };
        let name = p.cfg.sites[i].name.clone();
        let data = p.data.get_mut(&name).unwrap();
        for k in data.found.get(&jql).cloned().unwrap_or_default() {
            let updated = data.tickets[&k].updated.clone();
            data.code.insert(k.clone(), CodeEntry { at, updated, links: vec![] });
            ids.push(detail_key(&name, &k));
        }
    }
    ids
}

fn is_open(p: &JiraPlugin, id: &str) -> bool {
    let (site, key) = id.split_once('\0').unwrap();
    p.data[site].tickets[key].category != Category::Done
}

/// Answer every panel in flight, and the ones that start as those finish,
/// with the links `links`; who was asked, in order.
fn answer_all(p: &mut JiraPlugin, links: &[model::CodeLink]) -> Vec<String> {
    let mut asked = Vec::new();
    while let Some(id) = p.sync.code_in_flight.iter().next().cloned() {
        asked.push(id.clone());
        let (site, key) = id.split_once('\0').unwrap();
        let updated = p.data[site].tickets[key].updated.clone();
        p.on_msg(Box::new(JiraMsg::Code {
            site: site.into(),
            key: key.into(),
            updated,
            action: code_history::action(p.cfg.site(site).unwrap(), key),
            result: Ok(client::CodeRead { links: links.to_vec(), ..Default::default() }),
        }));
    }
    asked
}

#[test]
fn panels_in_view_are_read_again_after_half_an_hour_open_tickets_only() {
    let mut p = numbered();
    let ids = held_panels(&mut p, NOW);
    let open: Vec<String> = ids.iter().filter(|id| is_open(&p, id)).cloned().collect();
    assert!(open.len() > CODE_AT_ONCE && open.len() < ids.len(), "{} open of {}", open.len(), ids.len());
    p.now = NOW + CODE_REREAD_SECS - 1;
    assert!(p.sync_code().is_empty(), "held and young: nothing asked");
    p.now = NOW + CODE_REREAD_SECS;
    assert_eq!(p.sync_code().len(), CODE_AT_ONCE, "a few at a time");
    // The merge request that was linked meanwhile arrives with the answer.
    let url = "https://gitlab.example/api/-/merge_requests/265";
    let mut asked = answer_all(&mut p, &code_link(url));
    asked.sort();
    let mut open = open;
    open.sort();
    assert_eq!(asked, open, "every open ticket in view once, the done one not");
    assert_eq!(p.data["acme"].code["ABC-12"].links.len(), 1);
    assert_eq!(p.data["acme"].code["ABC-12"].at, p.now);
    assert!(p.sync_code().is_empty(), "read just now: young again");
    p.now += CODE_REREAD_SECS - 1;
    assert!(p.sync_code().is_empty());
}

#[test]
fn a_ticket_outside_the_view_is_not_read_again_by_time() {
    let mut p = numbered();
    held_panels(&mut p, NOW);
    p.data.get_mut("acme").unwrap().code.insert(
        "ABC-50".into(),
        CodeEntry { at: NOW, updated: String::new(), links: vec![] },
    );
    p.now = NOW + 10 * CODE_REREAD_SECS;
    p.sync_code();
    answer_all(&mut p, &[]);
    assert_eq!(p.data["acme"].code["ABC-50"].at, NOW, "no ticket in view, no read");
}

#[test]
fn r_reads_every_panel_in_view_once_whatever_the_ticket_and_not_twice_in_a_row() {
    let mut p = numbered();
    let ids = held_panels(&mut p, NOW);
    let every = p.cfg.refresh_secs as i64;
    p.now = NOW + every + 1;
    assert!(p.sync_code().is_empty(), "a refresh on the cycle leaves them");
    let fx = p.refresh();
    assert_eq!(fx.len(), p.cfg.sites.len() + CODE_AT_ONCE, "the lists, and a few panels");
    let mut asked = answer_all(&mut p, &[]);
    asked.sort();
    let mut all = ids.clone();
    all.sort();
    assert_eq!(asked, all, "the done ticket's too: r is asked for");
    // Pressed again at once: the panels were just read.
    p.sync.in_flight.clear();
    let fx = p.refresh();
    assert_eq!(fx.len(), p.cfg.sites.len(), "only the lists");
    assert!(p.sync.code_refresh.is_empty());
}

#[test]
fn a_held_site_has_its_panels_left_even_on_r_and_they_come_when_it_is_free() {
    let mut p = numbered();
    held_panels(&mut p, NOW);
    let every = p.cfg.refresh_secs as i64;
    p.now = NOW + every + 1;
    let rate = crate::rate::Rate { limit: Some(1000), remaining: Some(5), reset: Some(NOW + 4000), reason: None };
    p.limits.note("acme", &crate::client::Resp { status: 200, retry_after: None, rate, body: String::new() });
    p.refresh();
    assert!(p.sync.code_in_flight.iter().all(|id| id.starts_with("beta\0")), "{:?}", p.sync.code_in_flight);
    answer_all(&mut p, &[]);
    assert!(p.data["acme"].code["ABC-12"].at == NOW, "acme's were not read");
    p.now = NOW + 4000;
    p.sync_code();
    let asked = answer_all(&mut p, &[]);
    assert!(asked.iter().any(|id| id == "acme\0ABC-12"), "free again: {asked:?}");
}
