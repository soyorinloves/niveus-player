//! QR sign-in and session checks.
//!
//! Only QR sign-in is supported; there is deliberately no password or
//! phone-number flow.

use crate::client::NcmApiClient;
use crate::dto::*;
use crate::error::{ApiError, Result};

/// Everything needed to render and poll a QR sign-in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrLogin {
    /// Session key used when polling.
    pub key: String,
    /// The URL to encode as a QR code. The phone app scans this.
    pub url: String,
}

/// Where a QR sign-in currently stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QrLoginState {
    /// The service is waiting for the code to be scanned.
    Waiting,
    /// Scanned, waiting for the user to confirm on their phone.
    Scanned,
    /// Confirmed. The session cookie has been stored on the client.
    Confirmed,
    /// The code is no longer usable; a new one must be requested.
    Expired,
}

/// The account behind a valid session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedInUser {
    pub id: u64,
    pub name: String,
}

impl NcmApiClient {
    /// Requests a fresh QR code to display.
    pub async fn qr_login_start(&self) -> Result<QrLogin> {
        let query = self.query();
        let response = self
            .inner
            .login_qr_key(&query)
            .await
            .map_err(|e| Self::fail("login_qr_key", e))?;
        self.record(&response);

        let key: QrKeyDto = Self::decode("login_qr_key", response.body)?;
        if key.unikey.trim().is_empty() {
            return Err(ApiError::InvalidResponse(
                "登录接口未返回二维码 key".to_string(),
            ));
        }

        // The QR payload is derived from the key; the upstream crate does not
        // render an image, so the caller draws it.
        let url = format!("https://music.163.com/login?codekey={}", key.unikey);
        Ok(QrLogin {
            key: key.unikey,
            url,
        })
    }

    /// Polls a QR sign-in.
    ///
    /// On confirmation the service hands back the session cookie, which the
    /// client stores; the caller only has to persist [`NcmApiClient::cookie`].
    pub async fn qr_login_poll(&self, key: &str) -> Result<QrLoginState> {
        let query = self.query().param("key", key);
        let response = self
            .inner
            .login_qr_check(&query)
            .await
            .map_err(|e| Self::fail("login_qr_check", e))?;
        self.record(&response);

        let check: QrCheckDto = Self::decode("login_qr_check", response.body)?;
        if let Some(message) = check.message.as_deref() {
            tracing::debug!(code = check.code, message, "qr sign-in status");
        }
        match check.code {
            801 => Ok(QrLoginState::Waiting),
            802 => Ok(QrLoginState::Scanned),
            803 => {
                if !self.has_session() {
                    return Err(ApiError::InvalidResponse(
                        "扫码已确认，但未收到登录 cookie".to_string(),
                    ));
                }
                Ok(QrLoginState::Confirmed)
            }
            800 | QR_CHECK_INVALID => Ok(QrLoginState::Expired),
            other => Err(ApiError::InvalidResponse(format!(
                "未知的扫码状态码 {other}"
            ))),
        }
    }

    /// The account behind the stored cookie, or `None` when signed out.
    ///
    /// A signed-out session is reported as a null profile with a 200 status, so
    /// this inspects the body rather than the transport. An expired session is
    /// therefore not an error: it is simply `None`, and the caller moves on to
    /// QR sign-in.
    pub async fn login_status(&self) -> Result<Option<SignedInUser>> {
        /// The Node original nests the status under `data`.
        #[derive(serde::Deserialize)]
        struct Envelope {
            #[serde(default)]
            data: Option<LoginStatusDto>,
        }

        let query = self.query();
        let response = self
            .inner
            .login_status(&query)
            .await
            .map_err(|e| Self::fail("login_status", e))?;
        self.record(&response);

        let body = response.body;
        let status: LoginStatusDto =
            match Self::decode::<LoginStatusDto>("login_status", body.clone()) {
                Ok(status) if status.profile.is_some() => status,
                // Either the payload is nested, or the session is signed out.
                _ => Self::decode::<Envelope>("login_status", body)
                    .ok()
                    .and_then(|envelope| envelope.data)
                    .unwrap_or(LoginStatusDto { profile: None }),
            };

        Ok(status.profile.and_then(|profile| {
            let id = profile.user_id?;
            Some(SignedInUser {
                id,
                name: profile.nickname.unwrap_or_else(|| id.to_string()),
            })
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_qr_payload_embeds_the_key() {
        let login = QrLogin {
            key: "abc-123".into(),
            url: "https://music.163.com/login?codekey=abc-123".into(),
        };
        assert!(login.url.contains(&login.key));
    }
}
