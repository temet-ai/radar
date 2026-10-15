//! An environment reference is configuration; the resolved secret never is.

use crate::model::{ApiError, ApiErrorKind};
use reqwest::header::HeaderValue;

pub fn validate_reference(reference: &str) -> Result<(), ApiError> {
    if reference.is_empty() {
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

pub(crate) fn resolve(reference: &str) -> Result<Option<HeaderValue>, ApiError> {
    resolve_with(reference, |name| std::env::var(name).ok())
}

fn resolve_with(
    reference: &str,
    read: impl FnOnce(&str) -> Option<String>,
) -> Result<Option<HeaderValue>, ApiError> {
    validate_reference(reference)?;
    if reference.is_empty() {
        return Ok(None);
    }
    let secret = if let Some(name) = reference.strip_prefix("env:") {
        read(name)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ApiError::new(ApiErrorKind::MissingToken))?
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

    #[test]
    fn public_environment_or_typed_tokens_are_accepted() {
        for good in [
            "",
            "env:GITHUB_TOKEN",
            "env:_TOKEN2",
            "github_pat_example_token_123",
        ] {
            assert!(validate_reference(good).is_ok());
        }
        for bad in [
            "gh",
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
        let value = resolve_with("env:TOKEN", |_| Some(secret.into()))
            .unwrap()
            .unwrap();
        assert!(value.is_sensitive());
        assert!(!format!("{value:?}").contains(secret));
        assert!(radar_core::log::contains_secret(secret));
        let typed = resolve_with("github_pat_synthetic_direct_123", |_| {
            panic!("typed token must not read environment")
        })
        .unwrap()
        .unwrap();
        assert!(typed.is_sensitive());
        assert!(!format!("{typed:?}").contains("github_pat_synthetic_direct_123"));
        for invalid in ["tiny", "secret with spaces", "secret\r\ninjected"] {
            let error = resolve_with("env:TOKEN", |_| Some(invalid.into())).unwrap_err();
            assert!(!error.to_string().contains(invalid));
            assert!(!format!("{error:?}").contains(invalid));
        }
        assert_eq!(
            resolve_with("env:TOKEN", |_| None).unwrap_err().kind,
            ApiErrorKind::MissingToken
        );
        assert!(
            resolve_with("", |_| panic!("public reads must not inspect credentials"))
                .unwrap()
                .is_none()
        );
    }
}
