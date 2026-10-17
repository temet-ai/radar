//! Account-owned cache storage. Persisted identity is a lookup key, never a
//! login: only a fresh successful auth response can activate a partition.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    SlackPlugin,
    client::Me,
    model::{Asked, Data},
};

const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AccountOwner {
    pub team_id: String,
    pub user_id: String,
}

impl AccountOwner {
    pub(crate) fn of(me: &Me) -> Option<Self> {
        fn id(value: &str, prefixes: &[char]) -> bool {
            value.len() > 1
                && value.starts_with(prefixes)
                && value
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        }
        // Ownership validation is independent of journal redaction. Redaction
        // may omit a valid identifier from evidence without changing a login.
        (id(&me.team_id, &['T']) && id(&me.user_id, &['U', 'W'])).then(|| Self {
            team_id: me.team_id.clone(),
            user_id: me.user_id.clone(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Partition {
    owner: AccountOwner,
    data: Data,
    /// A one-way print of the token the account last signed in with (see
    /// [`token_print`]). It decides only whether a start may draw this
    /// account's data before Slack has said who the token is; it is never
    /// a login, and nothing is written or moved on it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    token: Option<String>,
}

/// A one-way print of a token: FNV-1a over a radar prefix and the token, as
/// hex. It tells two tokens apart; it cannot be turned back into one.
pub(crate) fn token_print(token: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in b"radar-slack-token\0".iter().chain(token.trim().as_bytes()) {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AccountCache {
    version: u32,
    accounts: Vec<Partition>,
    /// Original JSON, including unknown future formats. Never inferred-owned,
    /// merged into active data, or replayed. Export with `cache-recovery`.
    quarantine: Vec<Value>,
}

impl Default for AccountCache {
    fn default() -> Self {
        Self {
            version: VERSION,
            accounts: vec![],
            quarantine: vec![],
        }
    }
}

impl From<Data> for AccountCache {
    fn from(data: Data) -> Self {
        // A bare Data value has no verified provenance, even when Me matches.
        Self::restore(serde_json::json!({"data": data}))
    }
}

impl AccountCache {
    pub(crate) fn restore(value: Value) -> Self {
        if let Ok(cache) = serde_json::from_value::<Self>(value.clone())
            && cache.version == VERSION
            && cache.snapshot() == value
            && cache
                .accounts
                .iter()
                .all(|p| p.data.me.as_ref().and_then(AccountOwner::of).as_ref() == Some(&p.owner))
            && cache.accounts.iter().enumerate().all(|(i, p)| {
                !cache.accounts[..i]
                    .iter()
                    .any(|earlier| earlier.owner == p.owner)
            })
        {
            return cache;
        }
        Self {
            quarantine: vec![value],
            ..Self::default()
        }
    }

    pub(crate) fn snapshot(&self) -> Value {
        serde_json::to_value(self).expect("Slack cache is JSON")
    }

    pub(crate) fn recovery(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string_pretty(&self.quarantine)? + "\n")
    }

    fn take(&mut self, owner: &AccountOwner) -> Data {
        self.accounts
            .iter()
            .position(|p| p.owner == *owner)
            .map(|i| self.accounts.remove(i).data)
            .unwrap_or_default()
    }

    fn put(&mut self, owner: AccountOwner, data: Data, token: Option<String>) {
        if data.me.as_ref().and_then(AccountOwner::of).as_ref() != Some(&owner) {
            self.quarantine.push(serde_json::json!({"data": data}));
            return;
        }
        if token.is_some() {
            // A token is one account's: no other partition keeps its print.
            for p in self.accounts.iter_mut().filter(|p| p.owner != owner) {
                if p.token == token {
                    p.token = None;
                }
            }
        }
        if let Some(p) = self.accounts.iter_mut().find(|p| p.owner == owner) {
            p.data = data;
            if token.is_some() {
                p.token = token;
            }
        } else {
            self.accounts.push(Partition { owner, data, token });
        }
    }

    /// The data of the one account last signed in with the token of this
    /// print, to draw before Slack confirms who the token is.
    fn drawn_for(&self, print: &str) -> Option<&Data> {
        let mut with = self
            .accounts
            .iter()
            .filter(|p| p.token.as_deref() == Some(print));
        match (with.next(), with.next()) {
            (Some(p), None) => Some(&p.data),
            _ => None,
        }
    }

    pub(crate) fn authenticated_data(mut self, me: Me) -> anyhow::Result<Data> {
        let owner = AccountOwner::of(&me)
            .ok_or_else(|| anyhow::anyhow!("Slack returned an incomplete account identity"))?;
        let mut data = self.take(&owner);
        data.me = Some(me);
        Ok(data)
    }
}

impl SlackPlugin {
    pub(crate) fn has_quarantined_cache(&self) -> bool {
        !self.account_cache.quarantine.is_empty()
    }

    pub(crate) fn authenticated(&self) -> bool {
        self.session_owner.is_some()
            && self.data.me.as_ref().and_then(AccountOwner::of) == self.session_owner
    }

    /// At start, before Slack says who the token is: the saved data of the
    /// one account last signed in with this very token, drawn and marked
    /// cached. Nothing is written or moved while it shows (none of it is
    /// signed in), and a sign-in as anyone else puts it away.
    pub(crate) fn draw_cached(&mut self) {
        if self.session_owner.is_some() || self.previewing || self.data != Data::default() {
            return;
        }
        let Ok(token) = self.session_credential() else {
            return;
        };
        if let Some(data) = self.account_cache.drawn_for(&token_print(&token)) {
            self.data = data.clone();
            self.previewing = true;
            self.changed();
        }
    }

    pub(crate) fn cache_snapshot(&self) -> AccountCache {
        let mut cache = self.account_cache.clone();
        if let Some(owner) = &self.session_owner {
            cache.put(owner.clone(), self.data.clone(), self.session_token.clone());
        } else if self.previewing {
            // What is drawn is a copy of a partition the cache still holds.
        } else if self.data != Data::default() {
            // Defensive recovery for unowned local state; never stamp it.
            cache
                .quarantine
                .push(serde_json::json!({"data": self.data}));
        }
        cache
    }

    pub(crate) fn park_account(&mut self) {
        let data = std::mem::take(&mut self.data);
        let token = self.session_token.take();
        if std::mem::take(&mut self.previewing) {
            // A drawn copy: the cache still holds the partition itself.
            if self.session_owner.is_none() {
                return;
            }
        }
        if let Some(owner) = self.session_owner.take() {
            self.account_cache.put(owner, data, token);
        } else if data != Data::default() {
            self.account_cache
                .quarantine
                .push(serde_json::json!({"data": data}));
        }
    }

    pub(crate) fn activate_account(&mut self, me: Me) -> bool {
        let Some(owner) = AccountOwner::of(&me) else {
            self.revoke_session();
            return false;
        };
        let drawn_is_this_account = self.previewing
            && self.session_owner.is_none()
            && self.data.me.as_ref().and_then(AccountOwner::of).as_ref() == Some(&owner);
        if drawn_is_this_account {
            // What is drawn is this account's: it stays on screen as it is,
            // and becomes the account's held data.
            self.previewing = false;
            self.account_cache.take(&owner);
        } else if self.session_owner.as_ref() != Some(&owner) || !self.authenticated() {
            self.revoke_session();
            self.data = self.account_cache.take(&owner);
        }
        self.session_token = self
            .session_credential()
            .ok()
            .map(|t| token_print(&t));
        self.session_owner = Some(owner);
        self.data.me = Some(me);
        self.changed();
        true
    }

    /// Successful delayed writes belong to their dispatch account, even after
    /// a token change. Keeping them never follows them with another account.
    pub(crate) fn remember_asked(&mut self, owner: &AccountOwner, asked: Asked) {
        if self.authenticated() && self.session_owner.as_ref() == Some(owner) {
            self.data.asked.push(asked);
        } else if let Some(p) = self
            .account_cache
            .accounts
            .iter_mut()
            .find(|p| p.owner == *owner)
        {
            p.data.asked.push(asked);
        } else {
            // A missing partition must not manufacture a new account identity.
            self.account_cache
                .quarantine
                .push(serde_json::json!({"owner": owner, "asked": asked}));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{Client, Recorded, Reply};
    use crate::{SlackMsg, Via, demo};
    use radar_core::Plugin;
    use radar_core::plugin::Effect;
    use serde_json::json;

    fn me(team: &str, user: &str) -> Me {
        Me {
            team_id: team.into(),
            user_id: user.into(),
            url: format!("https://{team}.slack.com/"),
            tz_offset: Some(0),
            ..Default::default()
        }
    }
    fn ok(value: Value) -> anyhow::Result<Reply> {
        Ok(Reply {
            status: 200,
            body: value.to_string(),
            retry_after: None,
            scopes: None,
        })
    }
    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }
    fn task(fx: Vec<Effect>) -> radar_core::plugin::Msg {
        let mut tasks = fx.into_iter().filter_map(|e| match e {
            Effect::Spawn(t) | Effect::SpawnAction { task: t, .. } => Some(t),
            _ => None,
        });
        let t = tasks.next().expect("bootstrap task");
        assert!(tasks.next().is_none(), "auth must precede all scoped work");
        rt().block_on(t)
    }

    #[test]
    fn account_switches_preserve_and_isolate_all_data_even_with_overlapping_ids() {
        for second in [me("TB", "UA"), me("TA", "UB"), me("TB", "UB")] {
            let mut p = SlackPlugin::new();
            assert!(p.install_me(me("TA", "UA")));
            p.data.users.insert("U1".into(), "A name".into());
            p.data.marks.insert("C1".into(), "100.001".into());
            p.data.opened.insert("C1/100.001".into(), "101.001".into());
            p.data.groups.push(crate::client::UserGroup {
                id: "S1".into(),
                ..Default::default()
            });
            p.data.asked.push(Asked {
                channel: "C1".into(),
                ts: "100.001".into(),
                ..Default::default()
            });
            p.data.channels.insert("C1".into(), Default::default());
            let a = p.data.clone();
            assert!(p.install_me(second.clone()));
            assert_eq!(
                p.data,
                Data {
                    me: Some(second.clone()),
                    ..Default::default()
                }
            );
            p.data.users.insert("U1".into(), "B name".into());
            p.data.marks.insert("C1".into(), "200.001".into());
            let b = p.data.clone();
            assert!(p.install_me(me("TA", "UA")));
            assert_eq!(p.data, a);
            assert!(p.install_me(second));
            assert_eq!(p.data, b);
            let restored = AccountCache::restore(p.snapshot().unwrap());
            assert!(restored.quarantine.is_empty());
            assert_eq!(restored.accounts.len(), 2);
        }
    }

    #[test]
    fn startup_restore_never_uses_cached_authority_in_either_config_order() {
        let source = demo::sample();
        for configure_first in [false, true] {
            let mut p = SlackPlugin::new();
            let cfg = source.config().unwrap();
            if configure_first {
                p.configure(Some(&cfg)).unwrap();
            }
            p.restore(source.snapshot().unwrap());
            if !configure_first {
                p.configure(Some(&cfg)).unwrap();
            }
            assert!(!p.authenticated());
            assert_eq!(p.data, Data::default());
            assert!(p.workspace().is_none());
            let restored = p.snapshot().unwrap();
            assert_eq!(restored["accounts"], source.snapshot().unwrap()["accounts"]);
            assert!(p.install_me(source.data.me.clone().unwrap()));
            assert_eq!(p.data, source.data);
        }
    }

    #[test]
    fn legacy_matching_me_is_not_proof_and_all_unknown_json_is_recoverable() {
        let source = demo::sample();
        let mut known_with_unknown_data = source.snapshot().unwrap();
        known_with_unknown_data["accounts"][0]["data"]["future_authored_content"] =
            json!({"words":"keep these"});
        for original in [
            json!({"data": source.data, "unknown_kept_work": {"draft":"my words"}}),
            json!({"version": 99, "accounts": [{"future":true}], "drafts":["future words"]}),
            known_with_unknown_data,
            json!(["malformed but recoverable", {"words":"keep"}]),
        ] {
            let mut p = SlackPlugin::new();
            p.restore(original.clone());
            assert!(p.install_me(source.data.me.clone().unwrap()));
            assert!(p.data.users.is_empty());
            assert!(p.data.marks.is_empty());
            assert!(p.data.asked.is_empty());
            let saved = p.snapshot().unwrap();
            assert_eq!(saved["quarantine"], json!([original]));
            let mut reopened = SlackPlugin::new();
            reopened.restore(saved);
            let recovered: Value =
                serde_json::from_str(&reopened.cache_snapshot().recovery().unwrap()).unwrap();
            assert_eq!(recovered, json!([original]));
            let command = reopened
                .cli()
                .unwrap()
                .try_get_matches_from(["slack", "cache-recovery"])
                .unwrap();
            assert_eq!(
                rt().block_on(reopened.run_cli(&command)).unwrap(),
                reopened.cache_snapshot().recovery().unwrap()
            );
        }
    }

    const TOKEN: &str = "xoxp-the-token-it-was-read-with";

    fn configured(token: &str) -> toml::Value {
        let mut cfg = demo::sample().cfg;
        cfg.token = token.into();
        toml::Value::try_from(&cfg).unwrap()
    }

    /// Account A signed in with TOKEN, something read, then saved.
    fn saved_by_a() -> (Data, Value) {
        let mut p = SlackPlugin::new();
        p.env = Some(vec![]);
        p.configure(Some(&configured(TOKEN))).unwrap();
        assert!(p.install_me(me("TA", "UA")));
        p.data.users.insert("U1".into(), "A name".into());
        p.data.channels.insert("C1".into(), Default::default());
        (p.data.clone(), p.snapshot().unwrap())
    }

    fn started(token: &str, saved: &Value, configure_first: bool) -> SlackPlugin {
        let mut p = SlackPlugin::new();
        p.env = Some(vec![]);
        if configure_first {
            p.configure(Some(&configured(token))).unwrap();
        }
        p.restore(saved.clone());
        if !configure_first {
            p.configure(Some(&configured(token))).unwrap();
        }
        p
    }

    #[test]
    fn a_start_draws_the_account_last_read_with_this_token_at_once_marked_cached_and_signed_out() {
        let (a, saved) = saved_by_a();
        assert!(!saved.to_string().contains(TOKEN), "a print, never the token");
        for configure_first in [false, true] {
            let mut p = started(TOKEN, &saved, configure_first);
            assert_eq!(p.data, a, "drawn before Slack is asked anything");
            assert!(!p.authenticated());
            assert!(crate::screen::top_right(&p).unwrap().starts_with("cached · "));
            // Nothing moves on it, and the first read signs in first.
            assert!(matches!(&p.catch_up("C1", "1.000001")[..], [Effect::Notify(_)]));
            assert!(p.data.marks.is_empty());
            let recording = Recorded::new(vec![
                ok(json!({"ok":true,"team_id":"TA","user_id":"UA","url":"https://TA.slack.com/"})),
                ok(json!({"ok":true,"user":{"id":"UA","tz_offset":0}})),
            ]);
            p.via = Via(Some(recording.clone()));
            let message = task(p.read());
            assert_eq!(
                recording.requests().iter().map(|r| r.method).collect::<Vec<_>>(),
                ["auth.test", "users.info"]
            );
            // Signed in as that account: what is drawn stays, now held.
            p.via = Via(Some(Recorded::new(vec![])));
            let _ = p.on_msg(message);
            assert!(p.authenticated());
            assert!(!p.previewing);
            assert_eq!(p.data.users, a.users);
            assert!(!crate::screen::top_right(&p).unwrap().starts_with("cached"));
            let again = p.snapshot().unwrap();
            assert_eq!(again["accounts"].as_array().unwrap().len(), 1);
            assert_eq!(again["quarantine"], json!([]));
        }
    }

    #[test]
    fn another_token_or_an_older_cache_draws_nothing_before_sign_in() {
        let (_, saved) = saved_by_a();
        for configure_first in [false, true] {
            let p = started("xoxp-another-token", &saved, configure_first);
            assert_eq!(p.data, Data::default());
            assert!(!p.previewing);
            assert!(!crate::screen::top_right(&p).unwrap().starts_with("cached"));
        }
        let mut older = saved.clone();
        older["accounts"][0].as_object_mut().unwrap().remove("token");
        let p = started(TOKEN, &older, false);
        assert!(!p.has_quarantined_cache(), "still this account's, read whole once signed in");
        assert_eq!(p.data, Data::default());
    }

    #[test]
    fn a_token_that_turns_out_to_be_another_account_puts_the_drawn_data_away_untouched() {
        let (a, saved) = saved_by_a();
        let mut p = started(TOKEN, &saved, true);
        assert!(p.previewing);
        // A token edited between the save and now: Slack says it is B.
        assert!(p.install_me(me("TB", "UB")));
        assert!(!p.previewing);
        assert_eq!(
            p.data,
            Data {
                me: Some(me("TB", "UB")),
                ..Default::default()
            }
        );
        let snapshot = p.snapshot().unwrap();
        assert_eq!(snapshot["quarantine"], json!([]));
        assert!(p.install_me(me("TA", "UA")));
        assert_eq!(p.data, a, "A's data is still A's, whole");
        // The token is B's now: A's partition no longer claims it.
        let after = p.snapshot().unwrap();
        let claims = after["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p.get("token").is_some())
            .count();
        assert_eq!(claims, 1);
    }

    #[test]
    fn conflicting_partitions_are_preserved_but_not_selected() {
        let mut raw = demo::sample().snapshot().unwrap();
        let partition = raw["accounts"][0].clone();
        raw["accounts"].as_array_mut().unwrap().push(partition);
        let cache = AccountCache::restore(raw.clone());
        assert!(cache.accounts.is_empty());
        assert_eq!(cache.quarantine, vec![raw]);
    }

    #[test]
    fn restart_with_changed_secret_authenticates_before_reading_or_scoping() {
        let original = demo::sample();
        let mut p = SlackPlugin::new();
        p.configure(original.config().as_ref()).unwrap();
        p.restore(original.snapshot().unwrap());
        let recording = Recorded::new(vec![
            ok(json!({"ok":true,"team_id":"TB","user_id":"UB","url":"https://b.slack.com/"})),
            ok(json!({"ok":true,"user":{"id":"UB","tz_offset":0}})),
        ]);
        p.via = Via(Some(recording.clone()));
        let message = task(p.read());
        assert_eq!(
            recording
                .requests()
                .iter()
                .map(|r| r.method)
                .collect::<Vec<_>>(),
            ["auth.test", "users.info"]
        );
        assert_eq!(p.data, Data::default());
        let fx = p.on_msg(message);
        assert_eq!(
            p.session_owner.as_ref().unwrap(),
            &AccountOwner {
                team_id: "TB".into(),
                user_id: "UB".into()
            }
        );
        assert!(p.data.users.is_empty());
        assert!(p.data.groups.is_empty());
        assert!(p.data.asked.is_empty());
        assert!(p.data.marks.is_empty());
        for e in fx {
            if let Effect::SpawnAction { action, .. } = e {
                assert!(!format!("{action:?}").contains("T000ACME1"));
            }
        }
    }

    #[test]
    fn failed_or_incomplete_auth_has_no_downstream_requests_or_cached_fallback() {
        for response in [
            json!({"ok":false,"error":"invalid_auth"}),
            json!({"ok":true,"team_id":"TA"}),
            json!({"ok":true,"user_id":"UA"}),
            json!({"ok":true,"team_id":"TA","user_id":""}),
        ] {
            let mut p = demo::sample();
            p.data.me.as_mut().unwrap().tz_offset = None; // forces a real validation
            let recording = Recorded::new(vec![ok(response)]);
            p.via = Via(Some(recording.clone()));
            let message = task(p.read());
            assert!(matches!(
                message.downcast_ref::<SlackMsg>().unwrap(),
                SlackMsg::Read { .. }
            ));
            let fx = p.on_msg(message);
            assert!(!p.authenticated());
            assert_eq!(p.data, Data::default());
            assert!(p.sync.errors.contains_key(&crate::SyncErrorKey::Bootstrap));
            assert!(
                !fx.iter()
                    .any(|e| matches!(e, Effect::Spawn(_) | Effect::SpawnAction { .. }))
            );
            assert_eq!(recording.requests().len(), 1);
        }
    }

    #[test]
    fn cli_auth_selects_only_exact_owner_marks_names_and_urls() {
        let mut source = demo::sample();
        source.cfg.channels.truncate(1);
        source
            .data
            .marks
            .insert(demo::TEAM.into(), "123.000001".into());
        for same_account in [false, true] {
            for catchup in [false, true] {
                let identity = if same_account {
                    source.data.me.clone().unwrap()
                } else {
                    me("T000ACME1", "UOTHER")
                };
                let url = if same_account {
                    "https://acme.slack.com/"
                } else {
                    "https://other.slack.com/"
                };
                let recording = Recorded::new(vec![
                    ok(
                        json!({"ok":true,"team_id":identity.team_id,"user_id":identity.user_id,"url":url}),
                    ),
                    ok(
                        json!({"ok":true,"messages":[{"ts":"500.001","user":demo::BOB,"text":"hello"}]}),
                    ),
                    ok(json!({"ok":false,"error":"missing_scope"})),
                ]);
                let client = Client::new(recording.clone());
                let text = if catchup {
                    rt().block_on(crate::catchup(
                        &client,
                        source.cfg.clone(),
                        source.cache_snapshot(),
                        None,
                        true,
                        demo::NOW,
                    ))
                } else {
                    rt().block_on(crate::list(
                        &client,
                        source.cfg.clone(),
                        source.cache_snapshot(),
                        None,
                        None,
                        true,
                        demo::NOW,
                    ))
                }
                .unwrap();
                let output: Value = serde_json::from_str(&text).unwrap();
                assert!(output[0]["url"].as_str().unwrap().starts_with(url));
                assert_eq!(
                    output[0]["author"]["name"],
                    if same_account {
                        "Bob Builder"
                    } else {
                        demo::BOB
                    }
                );
                let requests = recording.requests();
                assert_eq!(requests[0].method, "auth.test");
                assert_eq!(requests[1].method, "conversations.history");
                let oldest = requests[1]
                    .params
                    .iter()
                    .find(|(k, _)| k == "oldest")
                    .unwrap()
                    .1
                    .clone();
                assert_eq!(
                    oldest,
                    if catchup && same_account {
                        "123.000001".into()
                    } else {
                        format!("{}.000000", demo::NOW - source.cfg.days as i64 * 86400)
                    }
                );
            }
        }
    }
}
