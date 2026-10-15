//! Signing in from Settings → Agents, as a state machine the section draws.
//! radar never holds a credential: Copilot's own `login --device-code` shows a
//! code and a link, which radar only displays; Codex and Claude Code run their
//! own login command in a terminal tab (T7), and radar re-reads their status when
//! it ends. Every transition here carries words for the screen and nothing
//! the CLI keeps.

use crate::settings::config::PtyCli;

/// One sign-in, from the operator's point of view.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Login {
    #[default]
    Idle,
    /// Asked the CLI for a code.
    Starting,
    /// Copilot: type `code` at `url`; radar waits.
    DeviceCode { url: String, code: String },
    /// Codex, Claude Code and PTY CLIs: its login command runs in this
    /// terminal tab (a tab id the dock gave).
    InTab { tab: String },
    /// Ended well; the status is read again.
    Done,
    Cancelled,
    Failed(String),
}

pub use crate::adapter::LoginEvent;

impl Login {
    /// Whether the flow is running (a second start is refused, cancel is offered).
    pub fn active(&self) -> bool {
        matches!(self, Login::Starting | Login::DeviceCode { .. } | Login::InTab { .. })
    }

    pub fn start(&mut self) -> bool {
        if self.active() {
            return false;
        }
        *self = Login::Starting;
        true
    }

    pub fn open_tab(&mut self, tab: impl Into<String>) {
        *self = Login::InTab { tab: tab.into() };
    }

    pub fn cancel(&mut self) {
        if self.active() {
            *self = Login::Cancelled;
        }
    }

    /// Apply an event. Returns whether the CLI's status must be read again.
    pub fn on_event(&mut self, event: LoginEvent) -> bool {
        // A flow that was cancelled ignores what the child said before it died.
        if !self.active() {
            return false;
        }
        match event {
            LoginEvent::DeviceCode { url, code } => {
                *self = Login::DeviceCode { url, code };
                false
            }
            LoginEvent::Completed => {
                *self = Login::Done;
                true
            }
            LoginEvent::Failed(why) => {
                *self = Login::Failed(why);
                true
            }
            LoginEvent::Cancelled => {
                *self = Login::Cancelled;
                false
            }
        }
    }

    /// The terminal tab running the login command ended. Returns whether the
    /// CLI's status must be read again.
    pub fn tab_exited(&mut self, code: Option<i32>) -> bool {
        if !matches!(self, Login::InTab { .. }) {
            return false;
        }
        *self = match code {
            Some(0) | None => Login::Done,
            Some(c) => Login::Failed(format!("the login command ended with status {c}")),
        };
        true
    }

    /// Lines for the detail pane.
    pub fn words(&self) -> Vec<String> {
        match self {
            Login::Idle => vec![],
            Login::Starting => vec!["Asking the CLI for a sign-in code…".into()],
            Login::DeviceCode { url, code } => vec![
                format!("Open {url}"),
                format!("and enter the code {code}"),
                "Waiting for you to finish. Cancel with esc.".into(),
            ],
            Login::InTab { .. } => vec!["The CLI's own login is running in a tab of the agents dock, under this section. ` hides and shows the dock.".into()],
            Login::Done => vec!["Sign-in finished; reading the status again.".into()],
            Login::Cancelled => vec!["Sign-in cancelled. Nothing was changed.".into()],
            Login::Failed(why) => vec![format!("Sign-in did not finish: {why}")],
        }
    }
}

/// How a provider signs in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Method {
    /// The adapter runs the flow and reports the code (Copilot).
    DeviceCode,
    /// The CLI's own login command, in a terminal tab.
    Command(Vec<String>),
    /// Nothing to run (a PTY CLI with no login configured).
    None,
}

pub fn method(provider: &str, pty: &[PtyCli]) -> Method {
    match provider {
        "copilot" => Method::DeviceCode,
        "codex" => Method::Command(vec!["codex".into(), "login".into()]),
        "claude" => Method::Command(vec!["claude".into(), "auth".into(), "login".into()]),
        other => match pty.iter().find(|c| c.id == other).and_then(|c| c.login.clone()) {
            Some(cmd) if !cmd.is_empty() => Method::Command(cmd),
            _ => Method::None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_code_flow_waits_then_cancels_without_a_status_read() {
        let mut l = Login::default();
        assert!(l.start());
        assert!(!l.start(), "a second start is refused while one runs");
        assert!(!l.on_event(LoginEvent::DeviceCode { url: "https://github.com/login/device".into(), code: "ABCD-1234".into() }));
        assert!(l.words().join(" ").contains("ABCD-1234"));
        l.cancel();
        assert_eq!(l, Login::Cancelled);
        // The child's late word after cancel changes nothing.
        assert!(!l.on_event(LoginEvent::Completed));
        assert_eq!(l, Login::Cancelled);
    }

    #[test]
    fn completion_and_failure_ask_for_the_status_again() {
        let mut l = Login::default();
        l.start();
        assert!(l.on_event(LoginEvent::Completed));
        assert_eq!(l, Login::Done);
        let mut l = Login::default();
        l.open_tab("t1");
        assert!(l.tab_exited(Some(2)));
        assert!(matches!(l, Login::Failed(_)));
    }

    #[test]
    fn each_provider_signs_in_its_own_way() {
        let pty = [PtyCli { id: "aider".into(), name: "Aider".into(), command: "aider".into(), args: vec![], login: None }];
        assert_eq!(method("copilot", &pty), Method::DeviceCode);
        assert_eq!(method("codex", &pty), Method::Command(vec!["codex".into(), "login".into()]));
        assert_eq!(method("claude", &pty), Method::Command(vec!["claude".into(), "auth".into(), "login".into()]));
        assert_eq!(method("aider", &pty), Method::None);
    }
}
