//! An environment reference is configuration; the resolved secret never is.
//! `gh` is a reference too: the gh program's sign-in, asked of gh (`gh auth
//! token`) when a request needs it and held in memory for a minute at most,
//! never written to the config or the cache.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::model::{ApiError, ApiErrorKind};
use reqwest::header::HeaderValue;

/// The reference that reuses the gh program's sign-in.
pub const GH: &str = "gh";

/// How long a token gh gave is used before gh is asked again.
const GH_HELD: Duration = Duration::from_secs(60);

pub fn validate_reference(reference: &str) -> Result<(), ApiError> {
    if reference.is_empty() || reference == GH {
        return Ok(());
    }
    let variable = reference.strip_prefix("env:").is_some_and(|name| {
        !name.is_empty()
            && name.len() <= 128
            && name
                .bytes()
                .enumerate()
                .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
    });
    let typed = !reference.starts_with("env:")
        && (8..=4096).contains(&reference.len())
        && reference.bytes().all(|b| b.is_ascii_graphic());
    if variable || typed {
        Ok(())
    } else {
        Err(ApiError::new(ApiErrorKind::InvalidTokenReference))
    }
}

/// Whether the reference is a secret typed into the config (not a name).
pub fn is_typed(reference: &str) -> bool {
    !reference.is_empty() && reference != GH && !reference.starts_with("env:")
}

/// The authorization for a request to github.com.
#[cfg(test)]
pub(crate) fn resolve(reference: &str) -> Result<Option<HeaderValue>, ApiError> {
    resolve_at(reference, "github.com")
}

/// The authorization for a request to `host` (the host gh signs in to).
pub(crate) fn resolve_at(reference: &str, host: &str) -> Result<Option<HeaderValue>, ApiError> {
    resolve_with(reference, |name| std::env::var(name).ok(), || gh_token(host))
}

/// gh's token for `host`, from memory while it is fresh, else from gh.
fn gh_token(host: &str) -> Option<String> {
    static HELD: Mutex<Option<(String, Instant, String)>> = Mutex::new(None);
    let mut held = HELD.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((h, at, token)) = held.as_ref()
        && h == host
        && at.elapsed() < GH_HELD
    {
        return Some(token.clone());
    }
    let out = std::process::Command::new("gh")
        .args(["auth", "token", "--hostname", host])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|out| out.status.success())?;
    let token = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if token.is_empty() {
        *held = None;
        return None;
    }
    radar_core::log::secret(&token);
    *held = Some((host.to_string(), Instant::now(), token.clone()));
    Some(token)
}

fn resolve_with(
    reference: &str,
    read: impl FnOnce(&str) -> Option<String>,
    gh: impl FnOnce() -> Option<String>,
) -> Result<Option<HeaderValue>, ApiError> {
    validate_reference(reference)?;
    if reference.is_empty() {
        return Ok(None);
    }
    let secret = if let Some(name) = reference.strip_prefix("env:") {
        read(name)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ApiError::new(ApiErrorKind::MissingToken))?
    } else if reference == GH {
        gh().filter(|s| !s.is_empty())
            .ok_or_else(|| ApiError::new(ApiErrorKind::NoGhSignIn))?
    } else {
        reference.to_string()
    };
    radar_core::log::secret(&secret);
    if secret.len() < 8 || secret.len() > 4096 || !secret.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(ApiError::new(ApiErrorKind::InvalidToken));
    }
    let mut header = HeaderValue::from_str(&format!("Bearer {secret}"))
        .map_err(|_| ApiError::new(ApiErrorKind::InvalidToken))?;
    header.set_sensitive(true);
    Ok(Some(header))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_gh() -> Option<String> {
        panic!("only gh asks gh")
    }

    #[test]
    fn public_environment_or_typed_tokens_are_accepted() {
        for good in [
            "",
            "env:GITHUB_TOKEN",
            "env:_TOKEN2",
            "github_pat_example_token_123",
            "gh",
        ] {
            assert!(validate_reference(good).is_ok());
        }
        for bad in [
            "GH",
            " gh",
            "env:",
            "env:2TOKEN",
            "env:A-B",
            "env:TOKEN\n",
            " env:TOKEN",
            "tiny",
            "token with spaces",
        ] {
            assert_eq!(
                validate_reference(bad).unwrap_err().kind,
                ApiErrorKind::InvalidTokenReference
            );
        }
    }

    #[test]
    fn secrets_are_sensitive_and_never_in_errors_or_debug() {
        let secret = "synthetic-token-redaction-test-71";
        let value = resolve_with("env:TOKEN", |_| Some(secret.into()), no_gh)
            .unwrap()
            .unwrap();
        assert!(value.is_sensitive());
        assert!(!format!("{value:?}").contains(secret));
        assert!(radar_core::log::contains_secret(secret));
        let typed = resolve_with(
            "github_pat_synthetic_direct_123",
            |_| panic!("typed token must not read environment"),
            no_gh,
        )
        .unwrap()
        .unwrap();
        assert!(typed.is_sensitive());
        assert!(!format!("{typed:?}").contains("github_pat_synthetic_direct_123"));
        for invalid in ["tiny", "secret with spaces", "secret\r\ninjected"] {
            let error = resolve_with("env:TOKEN", |_| Some(invalid.into()), no_gh).unwrap_err();
            assert!(!error.to_string().contains(invalid));
            assert!(!format!("{error:?}").contains(invalid));
        }
        assert_eq!(
            resolve_with("env:TOKEN", |_| None, no_gh).unwrap_err().kind,
            ApiErrorKind::MissingToken
        );
        assert!(
            resolve_with("", |_| panic!("public reads must not inspect credentials"), no_gh)
                .unwrap()
                .is_none()
        );
    }

    /// `gh` asks gh for its token when a request is made: the token goes
    /// in the header, sensitive; the config keeps only the word.
    #[test]
    fn gh_is_asked_at_use_and_its_token_is_never_the_reference() {
        let token = "gho_synthetic_gh_login_token_42";
        let value = resolve_with(GH, |_| panic!("gh reads no variable"), || Some(token.into()))
            .unwrap()
            .unwrap();
        assert!(value.is_sensitive());
        assert!(!format!("{value:?}").contains(token));
        assert!(radar_core::log::contains_secret(token));
        assert!(!is_typed(GH) && !is_typed("env:GH_TOKEN") && is_typed("github_pat_x_12345678"));
        for none in [None, Some(String::new())] {
            assert_eq!(
                resolve_with(GH, |_| None, || none).unwrap_err().kind,
                ApiErrorKind::NoGhSignIn,
                "not signed in: said so, never public by surprise"
            );
        }
    }
}
