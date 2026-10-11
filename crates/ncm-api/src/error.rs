//! The error type this crate exposes.
//!
//! The four variants are the ones the design fixes for the API layer. Every
//! upstream failure is funnelled into one of them so callers never have to know
//! what the upstream crate's own error taxonomy looks like.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    /// The request never produced a usable answer: transport failure or timeout.
    #[error("网络请求失败: {0}")]
    Network(String),

    /// The session is missing, expired, or rejected by the service.
    #[error("需要登录: {0}")]
    Authentication(String),

    /// An answer arrived but was not what the endpoint promises: an unexpected
    /// shape, a malformed payload, or a service-level rejection.
    #[error("接口返回异常: {0}")]
    InvalidResponse(String),

    /// The service is throttling us.
    #[error("请求过于频繁: {0}")]
    RateLimited(String),
}

pub type Result<T> = std::result::Result<T, ApiError>;

impl ApiError {
    /// Whether re-running the request after a fresh login could help.
    pub fn is_authentication(&self) -> bool {
        matches!(self, ApiError::Authentication(_))
    }
}

impl From<ncm_api_rs::NcmError> for ApiError {
    fn from(error: ncm_api_rs::NcmError) -> Self {
        use ncm_api_rs::NcmError;
        match error {
            NcmError::Http(e) => ApiError::Network(e.to_string()),
            NcmError::Timeout(m) => ApiError::Network(m),
            NcmError::AuthRequired(m) => ApiError::Authentication(m),
            NcmError::RateLimited(m) => ApiError::RateLimited(m),
            NcmError::Api { code, msg } => {
                ApiError::InvalidResponse(format!("服务返回 code={code}: {msg}"))
            }
            NcmError::InvalidParam(m) => ApiError::InvalidResponse(m),
            NcmError::Crypto(m) => ApiError::InvalidResponse(format!("加密失败: {m}")),
            NcmError::Json(e) => ApiError::InvalidResponse(format!("JSON 解析失败: {e}")),
            NcmError::Unknown(m) => ApiError::Network(m),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_failures_map_to_network_and_auth_is_distinguishable() {
        let network: ApiError = ncm_api_rs::NcmError::Timeout("slow".into()).into();
        assert!(matches!(network, ApiError::Network(_)));
        assert!(!network.is_authentication());

        let auth: ApiError = ncm_api_rs::NcmError::AuthRequired("需要登录".into()).into();
        assert!(auth.is_authentication());

        let throttled: ApiError = ncm_api_rs::NcmError::RateLimited("slow down".into()).into();
        assert!(matches!(throttled, ApiError::RateLimited(_)));

        let rejected: ApiError = ncm_api_rs::NcmError::Api {
            code: 404,
            msg: "not found".into(),
        }
        .into();
        assert!(matches!(rejected, ApiError::InvalidResponse(_)));
    }
}
