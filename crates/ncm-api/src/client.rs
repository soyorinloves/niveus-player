//! The HTTP client wrapper and its cookie jar.

use crate::error::{ApiError, Result};
use ncm_api_rs::{ApiClient, ApiResponse, Query};
use serde::de::DeserializeOwned;
use std::sync::{Arc, Mutex};

/// Attributes that may accompany a cookie but are not cookies themselves.
///
/// They are skipped so a raw `Set-Cookie` line can be fed to
/// [`NcmApiClient::set_cookie`] as readily as the `name=value; name=value`
/// string this crate persists.
const COOKIE_ATTRIBUTES: [&str; 9] = [
    "path",
    "domain",
    "expires",
    "max-age",
    "secure",
    "httponly",
    "samesite",
    "priority",
    "partitioned",
];

/// A cookie store keyed by name, preserving the order names were first seen.
///
/// The upstream returns `Set-Cookie` lines complete with attributes
/// (`Path=/`, `Max-Age=…`); only the leading `name=value` pair is meaningful to
/// us, so the rest is dropped here rather than being forwarded.
#[derive(Debug, Default, Clone)]
struct CookieJar {
    entries: Vec<(String, String)>,
}

impl CookieJar {
    fn from_string(cookie: &str) -> Self {
        let mut jar = Self::default();
        for pair in cookie.split(';') {
            if let Some((name, value)) = pair.split_once('=') {
                let name = name.trim();
                if name.is_empty()
                    || COOKIE_ATTRIBUTES.contains(&name.to_ascii_lowercase().as_str())
                {
                    continue;
                }
                jar.set(name, value.trim());
            }
        }
        jar
    }

    fn set(&mut self, name: &str, value: &str) {
        match self.entries.iter_mut().find(|(k, _)| k == name) {
            Some(entry) => entry.1 = value.to_string(),
            None => self.entries.push((name.to_string(), value.to_string())),
        }
    }

    /// Absorbs the `Set-Cookie` headers of a response.
    fn absorb(&mut self, lines: &[String]) {
        for line in lines {
            let Some(first) = line.split(';').next() else {
                continue;
            };
            if let Some((name, value)) = first.split_once('=') {
                let name = name.trim();
                if !name.is_empty() {
                    self.set(name, value.trim());
                }
            }
        }
    }
}

/// Renders the jar back into a cookie header value.
impl std::fmt::Display for CookieJar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let joined = self
            .entries
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ");
        f.write_str(&joined)
    }
}

/// Talks to the NetEase service.
///
/// The cookie jar lives here so every request carries the current session
/// without callers having to thread it through, and so a rotation pushed by the
/// service is picked up automatically.
#[derive(Debug, Clone)]
pub struct NcmApiClient {
    /// The upstream client. Kept crate-private so no `ncm-api-rs` type can
    /// appear in this crate's public surface.
    pub(crate) inner: ApiClient,
    jar: Arc<Mutex<CookieJar>>,
}

impl Default for NcmApiClient {
    fn default() -> Self {
        Self::new()
    }
}

impl NcmApiClient {
    pub fn new() -> Self {
        Self {
            inner: ApiClient::new(None),
            jar: Arc::new(Mutex::new(CookieJar::default())),
        }
    }

    /// Builds a client that already holds a persisted session.
    pub fn with_cookie(cookie: impl Into<String>) -> Self {
        let client = Self::new();
        client.set_cookie(&cookie.into());
        client
    }

    /// The session cookie, ready to be persisted.
    pub fn cookie(&self) -> String {
        self.jar.lock().expect("cookie jar poisoned").to_string()
    }

    pub fn set_cookie(&self, cookie: &str) {
        *self.jar.lock().expect("cookie jar poisoned") = CookieJar::from_string(cookie);
    }

    pub fn clear_cookie(&self) {
        *self.jar.lock().expect("cookie jar poisoned") = CookieJar::default();
    }

    /// Whether a session cookie is present.
    ///
    /// This is only a hint; the service is the authority on whether the session
    /// is still valid, which is what [`crate::NcmApiClient::login_status`] is for.
    pub fn has_session(&self) -> bool {
        self.jar
            .lock()
            .expect("cookie jar poisoned")
            .entries
            .iter()
            .any(|(k, v)| k == "MUSIC_U" && !v.is_empty())
    }

    /// A request builder carrying the current session.
    pub(crate) fn query(&self) -> Query {
        Query::new().cookie(&self.cookie())
    }

    /// Folds a newly received response's cookies into the jar.
    pub(crate) fn record(&self, response: &ApiResponse) {
        self.jar
            .lock()
            .expect("cookie jar poisoned")
            .absorb(&response.cookie);
    }

    /// Turns an upstream failure into this crate's error and logs it.
    pub(crate) fn fail(endpoint: &str, error: ncm_api_rs::NcmError) -> ApiError {
        let error: ApiError = error.into();
        tracing::warn!(endpoint, error = %error, "request failed");
        error
    }

    /// Deserialises a response body, reporting the endpoint on a shape mismatch.
    pub(crate) fn decode<T: DeserializeOwned>(
        endpoint: &str,
        body: serde_json::Value,
    ) -> Result<T> {
        serde_json::from_value(body).map_err(|e| {
            tracing::warn!(endpoint, error = %e, "unexpected payload shape");
            ApiError::InvalidResponse(format!("{endpoint}: {e}"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_cookie_attributes_are_stripped() {
        let mut jar = CookieJar::from_string("MUSIC_U=abc; Path=/; Max-Age=100");
        assert_eq!(jar.to_string(), "MUSIC_U=abc");

        jar.absorb(&[
            "NMTID=xyz; Max-Age=315360000; Expires=Sat, 04 Oct 2036 01:05:55 GMT; Path=/;".into(),
        ]);
        assert_eq!(jar.to_string(), "MUSIC_U=abc; NMTID=xyz");
    }

    #[test]
    fn a_repeated_name_is_replaced_in_place() {
        let mut jar = CookieJar::from_string("a=1; b=2");
        jar.set("a", "9");
        assert_eq!(jar.to_string(), "a=9; b=2");
    }

    #[test]
    fn an_empty_session_is_recognised_as_signed_out() {
        let client = NcmApiClient::new();
        assert!(!client.has_session());

        client.set_cookie("MUSIC_U=token; __csrf=csrf");
        assert!(client.has_session());
        assert!(client.cookie().contains("__csrf=csrf"));

        client.clear_cookie();
        assert!(!client.has_session());
    }
}
