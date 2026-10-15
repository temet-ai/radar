//! Which of the distinct states a CLI is in, from what its adapter reported
//! ([`AdapterStatus`]). It holds no token and asks no store for one: the CLI
//! says who is signed in, and radar shows exactly that.

use crate::adapter::{AdapterStatus, AuthStatus};

/// The one state a CLI is shown in. Each asks something different of the
/// operator: install it, sign in, ask the organisation, or look at the error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Not asked yet.
    Unread,
    /// This build has no adapter for it (its provider crate is not built in).
    NoAdapter,
    Missing,
    /// Installed, but radar could not talk to it (also a protocol mismatch).
    Error(String),
    SignedOut,
    PolicyBlocked(String),
    Ready { account: String },
    /// Installed and answering; sign-in not known (a PTY CLI).
    Installed,
}

pub fn state(status: Option<&AdapterStatus>) -> State {
    let Some(s) = status else { return State::Unread };
    if !s.installed {
        return State::Missing;
    }
    if let Some(e) = &s.error {
        return State::Error(e.clone());
    }
    match &s.auth {
        AuthStatus::SignedOut => State::SignedOut,
        AuthStatus::PolicyBlocked { message } => State::PolicyBlocked(message.clone()),
        AuthStatus::SignedIn { as_user } => State::Ready { account: as_user.clone() },
        AuthStatus::Unknown => State::Installed,
    }
}

impl State {
    /// The short word on the row.
    pub fn word(&self) -> &'static str {
        match self {
            State::Unread => "checking",
            State::NoAdapter => "not in this build",
            State::Missing => "not installed",
            State::Error(_) => "cannot connect",
            State::SignedOut => "signed out",
            State::PolicyBlocked(_) => "blocked by policy",
            State::Ready { .. } => "signed in",
            State::Installed => "installed",
        }
    }

    /// What to do about it, for the detail pane.
    pub fn advice(&self) -> String {
        match self {
            State::Unread => "Reading the CLI's status…".into(),
            State::NoAdapter => "This build of radar has no adapter for it yet, so it cannot be used or signed in to from here.".into(),
            State::Missing => "The executable was not found on PATH. Install it, or set where it is in [sessions.executables].".into(),
            State::Error(e) => format!("The CLI is installed but radar could not use it: {e}"),
            State::SignedOut => "Not signed in. Sign in from here; the CLI keeps the login, radar never sees it.".into(),
            State::PolicyBlocked(why) => format!("Your organisation does not allow this: {why}"),
            State::Ready { account } => format!("Signed in as {account}."),
            State::Installed => "Installed. It has no sign-in radar can read.".into(),
        }
    }

    /// A session can start with it.
    pub fn usable(&self) -> bool {
        matches!(self, State::Ready { .. } | State::Installed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::Capabilities;

    fn st(installed: bool, auth: AuthStatus, error: Option<&str>) -> AdapterStatus {
        AdapterStatus {
            installed,
            version: Some("0.158.0".into()),
            protocol: None,
            auth,
            error: error.map(Into::into),
            capabilities: Capabilities::default(),
        }
    }

    #[test]
    fn a_missing_executable_is_not_a_signed_out_cli() {
        let missing = state(Some(&st(false, AuthStatus::SignedOut, None)));
        let out = state(Some(&st(true, AuthStatus::SignedOut, None)));
        assert_eq!(missing, State::Missing);
        assert_eq!(out, State::SignedOut);
        assert_ne!(missing.word(), out.word());
    }

    #[test]
    fn every_state_has_its_own_word() {
        let all = [
            state(None),
            state(Some(&st(false, AuthStatus::Unknown, None))),
            state(Some(&st(true, AuthStatus::Unknown, Some("no answer")))),
            state(Some(&st(true, AuthStatus::SignedOut, None))),
            state(Some(&st(true, AuthStatus::PolicyBlocked { message: "off".into() }, None))),
            state(Some(&st(true, AuthStatus::SignedIn { as_user: "me".into() }, None))),
            state(Some(&st(true, AuthStatus::Unknown, None))),
        ];
        let mut words: Vec<_> = all.iter().map(State::word).collect();
        words.sort();
        words.dedup();
        assert_eq!(words.len(), all.len());
        assert!(all[5].usable() && !all[3].usable() && !all[1].usable());
    }
}
