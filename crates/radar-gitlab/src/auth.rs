//! A token for each request, and a fresh one once when GitLab turns it down.
//!
//! A token borrowed from glab can expire while radar is open (glab's OAuth
//! tokens last hours). When GitLab answers 401 and the token came from glab,
//! radar asks glab once to bring its login up to date, reads the token again
//! and tries the request once more. Any other 401, or a second one, is shown
//! as it is: never a loop.

use std::future::Future;

use anyhow::{Result, anyhow};
use radar_core::log;

use crate::client::{Client, is_rejected};
use crate::config;

/// Where tokens come from.
pub trait Tokens: Send + Sync {
    /// The token to use now.
    fn token(&self) -> Result<String>;
    /// Ask the source to bring its token up to date. `false` when it cannot
    /// (an environment variable or a typed-in token stays what it is).
    fn renew(&self) -> Result<bool>;
}

/// The configured token spec (`env:NAME`, `glab`, or a literal) for a server.
pub struct Configured {
    pub spec: String,
    pub url: String,
}

impl Configured {
    pub fn new(spec: &str, url: &str) -> Self {
        Self {
            spec: spec.trim().to_string(),
            url: url.to_string(),
        }
    }
}

impl Tokens for Configured {
    fn token(&self) -> Result<String> {
        let token = config::resolve_token(&self.spec, &self.url)?;
        log::secret(&token);
        Ok(token)
    }

    fn renew(&self) -> Result<bool> {
        if self.spec != "glab" {
            return Ok(false);
        }
        // glab refreshes an expired login whenever it talks to the server;
        // its status check is the smallest such talk. What it prints is
        // not needed (and is never shown or logged).
        let host = config::host_of(&self.url);
        let status = std::process::Command::new("glab")
            .args(["auth", "status", "--hostname", &host])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|e| anyhow!("running glab: {e}"))?;
        log::info(format!(
            "gitlab asked glab to renew its login for {host}: {status}"
        ));
        Ok(true)
    }
}

/// Run `op` with a token; if GitLab rejects it and the source can renew it,
/// renew once and run `op` once more.
pub async fn with_token<T, F, Fut>(tokens: &dyn Tokens, op: F) -> Result<T>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let err = match op(tokens.token()?).await {
        Err(e) if is_rejected(&e) => e,
        other => return other,
    };
    if !tokens.renew()? {
        return Err(anyhow!("{err}: check the token under settings, connection"));
    }
    log::info("gitlab: GitLab rejected the token; trying once more with a fresh one from glab");
    match op(tokens.token()?).await {
        Err(e) if is_rejected(&e) => {
            log::warn("gitlab: GitLab rejected glab's renewed token too");
            Err(anyhow!(
                "{e}, also after glab renewed its login once: run glab auth login"
            ))
        }
        other => other,
    }
}

/// The usual way in: a client for the configured server and token, with
/// one renewal on a 401.
pub async fn run<T, F, Fut>(url: &str, spec: &str, op: F) -> Result<T>
where
    F: Fn(Client) -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let tokens = Configured::new(spec, url);
    with_token(&tokens, |token| op(Client::new(url, token))).await
}

/// Captured connection and optional explicitly installed fixture. Reads retain
/// the production renewal policy; writes deliberately obtain only one client.
#[derive(Clone)]
pub(crate) struct Provider {
    pub url: String,
    pub spec: String,
    #[cfg(any(test, feature = "test-support"))]
    pub scripted: bool,
    #[cfg(any(test, feature = "test-support"))]
    pub fixture: Option<crate::test_support::HostedFixture>,
}

impl Provider {
    pub fn client(&self) -> Result<Client> {
        #[cfg(any(test, feature = "test-support"))]
        if self.scripted {
            return self.fixture.as_ref()
                .map(|fixture| fixture.client(&self.url))
                .ok_or_else(|| anyhow!("scripted GitLab fixture has no transport"));
        }
        // Single-send mutations do not renew or retry after rejection.
        let token = config::resolve_token(&self.spec, &self.url)?;
        Ok(Client::new(&self.url, token))
    }

    pub async fn run<T, F, Fut>(&self, op: F) -> Result<T>
    where
        F: Fn(Client) -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        #[cfg(any(test, feature = "test-support"))]
        if self.scripted {
            // Synthetic auth has no renewable credentials or CLI fallback.
            return op(self.client()?).await;
        }
        run(&self.url, &self.spec, op).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::Rejected;
    use std::sync::Mutex;

    /// Tokens `t1`, `t2`, … each renewal; counts renewals.
    struct Fake {
        renewable: bool,
        version: Mutex<u32>,
    }

    impl Tokens for Fake {
        fn token(&self) -> Result<String> {
            Ok(format!("t{}", self.version.lock().unwrap()))
        }
        fn renew(&self) -> Result<bool> {
            if self.renewable {
                *self.version.lock().unwrap() += 1;
            }
            Ok(self.renewable)
        }
    }

    fn fake(renewable: bool) -> Fake {
        Fake {
            renewable,
            version: Mutex::new(1),
        }
    }

    fn block<T>(f: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(f)
    }

    #[test]
    fn an_expired_glab_token_is_renewed_once_and_the_request_retried() {
        let tokens = fake(true);
        let tried = Mutex::new(Vec::new());
        let out = block(with_token(&tokens, |t| {
            tried.lock().unwrap().push(t.clone());
            async move {
                if t == "t1" {
                    Err(Rejected.into())
                } else {
                    Ok("data")
                }
            }
        }));
        assert_eq!(out.unwrap(), "data");
        assert_eq!(*tried.lock().unwrap(), vec!["t1", "t2"]);
    }

    #[test]
    fn a_second_rejection_is_shown_not_retried_again() {
        let tokens = fake(true);
        let calls = Mutex::new(0);
        let out: Result<()> = block(with_token(&tokens, |_| {
            *calls.lock().unwrap() += 1;
            async { Err(Rejected.into()) }
        }));
        let e = out.unwrap_err();
        assert_eq!(
            *calls.lock().unwrap(),
            2,
            "once, then once more, never a loop"
        );
        assert!(format!("{e:#}").contains("glab auth login"), "{e:#}");
        assert_eq!(
            e.to_string(),
            "GitLab rejected the token (401), also after glab renewed its login once: run glab auth login"
        );
    }

    #[test]
    fn a_token_that_cannot_be_renewed_fails_at_once() {
        let tokens = fake(false);
        let calls = Mutex::new(0);
        let out: Result<()> = block(with_token(&tokens, |_| {
            *calls.lock().unwrap() += 1;
            async { Err(Rejected.into()) }
        }));
        assert_eq!(*calls.lock().unwrap(), 1);
        assert!(format!("{:#}", out.unwrap_err()).contains("settings, connection"));
    }

    #[test]
    fn other_errors_are_not_retried() {
        let tokens = fake(true);
        let calls = Mutex::new(0);
        let out: Result<()> = block(with_token(&tokens, |_| {
            *calls.lock().unwrap() += 1;
            async { Err(anyhow!("GitLab unreachable")) }
        }));
        assert_eq!(*calls.lock().unwrap(), 1);
        assert!(out.is_err());
    }
}
