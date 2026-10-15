//! The terminal layers between radar and the user's mouse.

use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stack {
    pub herdr: bool,
    pub herdr_pane: bool,
    pub tmux: bool,
    pub cmux: bool,
    pub ghostty: bool,
    pub foot: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    Direct,
    Pending,
    Ready,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    pub stack: Stack,
    pub route: Route,
}

impl State {
    pub fn new(stack: Stack) -> Self {
        let route = if stack.herdr_pane {
            Route::Pending
        } else if stack.herdr {
            Route::Failed("Herdr pane identity unavailable".into())
        } else {
            Route::Direct
        };
        Self { stack, route }
    }

    pub fn right_click_words(&self) -> &'static str {
        match self.route {
            Route::Direct => "requested; terminal controls delivery",
            Route::Pending => "routing to radar pane…",
            Route::Ready => "routed to radar pane",
            Route::Failed(_) => "routing failed · use x",
        }
    }

    pub fn bar_words(&self) -> String {
        match self.route {
            Route::Pending => "mouse …".into(),
            Route::Ready => "mouse ✓".into(),
            Route::Failed(_) => "mouse !".into(),
            Route::Direct => self.stack.description(),
        }
    }
}

impl Stack {
    pub fn detect() -> Self {
        Self::from_env(&|name| std::env::var(name).ok())
    }

    fn from_env(var: &dyn Fn(&str) -> Option<String>) -> Self {
        let set = |name| var(name).is_some_and(|v| !v.is_empty());
        let herdr = var("HERDR_ENV").as_deref() == Some("1");
        let cmux = set("CMUX_SURFACE_ID") || set("CMUX_SOCKET_PATH") || set("CMUX_TUI_SOCKET");
        let ghostty = !cmux && (var("TERM_PROGRAM").as_deref() == Some("ghostty")
            || set("GHOSTTY_RESOURCES_DIR"));
        Self {
            herdr,
            herdr_pane: herdr && set("HERDR_PANE_ID"),
            tmux: set("TMUX"),
            cmux,
            ghostty,
            // Foot deliberately clears TERM_PROGRAM and has no dedicated
            // identity variable. Its default terminfo name is the best signal.
            foot: !cmux && !ghostty && matches!(var("TERM").as_deref(), Some("foot" | "foot-direct")),
        }
    }

    pub fn description(&self) -> String {
        let mut layers = Vec::new();
        if self.tmux { layers.push("tmux"); }
        if self.herdr { layers.push("Herdr"); }
        if self.cmux { layers.push("cmux"); }
        if self.ghostty { layers.push("Ghostty"); }
        if self.foot { layers.push("Foot"); }
        if layers.is_empty() { "terminal (unidentified)".into() } else { layers.join(" → ") }
    }
}

/// Only this pane is changed. Herdr falls back to its pane menu after radar
/// releases mouse reporting, so the setting remains useful for other TUIs.
pub async fn route_herdr_right_click() -> Result<(), String> {
    let binary = std::env::var_os("HERDR_BIN_PATH").unwrap_or_else(|| "herdr".into());
    let command = tokio::process::Command::new(binary)
        .args(["pane", "input", "--current", "--right-click", "pane"])
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(Duration::from_secs(2), command)
        .await
        .map_err(|_| "Herdr pane routing timed out".to_string())?
        .map_err(|e| format!("Herdr pane routing unavailable: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err("Herdr could not route right-clicks to this pane".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{Event, Host};

    fn stack(pairs: &[(&str, &str)]) -> Stack {
        Stack::from_env(&|key| pairs.iter().find(|(name, _)| *name == key).map(|(_, value)| (*value).into()))
    }

    #[test]
    fn names_the_actual_layers_without_mistaking_cmux_for_ghostty() {
        assert_eq!(stack(&[("TERM_PROGRAM", "ghostty")]).description(), "Ghostty");
        assert_eq!(stack(&[("TERM", "foot")]).description(), "Foot");
        assert_eq!(stack(&[("TERM", "foot-direct")]).description(), "Foot");
        assert_eq!(stack(&[("TERM", "foot"), ("HERDR_ENV", "1"), ("HERDR_PANE_ID", "w1:p1")]).description(), "Herdr → Foot");
        assert_eq!(stack(&[("TERM", "foot"), ("TERM_PROGRAM", "ghostty")]).description(), "Ghostty");
        assert_eq!(stack(&[("CMUX_SURFACE_ID", "surface"), ("TERM_PROGRAM", "ghostty")]).description(), "cmux");
        assert_eq!(stack(&[("CMUX_TUI_SOCKET", "socket")]).description(), "cmux");
        assert_eq!(stack(&[("TMUX", "socket"), ("HERDR_ENV", "1"), ("HERDR_PANE_ID", "w1:p1"), ("TERM_PROGRAM", "ghostty")]).description(), "tmux → Herdr → Ghostty");
        assert!(stack(&[("HERDR_ENV", "1"), ("HERDR_PANE_ID", "w1:p1")]).herdr_pane);
        assert!(!stack(&[("HERDR_ENV", "1")]).herdr_pane);
        assert_eq!(stack(&[]).description(), "terminal (unidentified)");
    }

    #[test]
    fn routing_result_is_visible_in_the_shared_host_ui() {
        let mut host = Host::new(vec![], 0).unwrap();
        host.terminal = Some(State::new(stack(&[("HERDR_ENV", "1"), ("HERDR_PANE_ID", "w1:p1")])));
        host.update(Event::TerminalRoute(Ok(())));
        assert!(crate::shell::to_text(&host, 80, 24).contains("mouse ✓"));
        host.update(Event::TerminalRoute(Err("pane unavailable".into())));
        let shown = crate::shell::to_text(&host, 80, 24);
        assert!(shown.contains("mouse !"));
        assert!(shown.contains("pane unavailable"));
    }
}
