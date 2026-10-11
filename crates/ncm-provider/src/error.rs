//! Errors surfaced by the provider.
//!
//! The API layer's error type stops here: `ProviderError::Api` carries its own
//! classification rather than naming a type from `niveus-ncm-api`, so nothing
//! above this crate depends on that crate's shape.

use thiserror::Error;

/// Why an API call failed, classified for the caller's purposes.
#[derive(Debug, Error)]
pub enum ApiFailure {
    #[error("网络错误: {0}")]
    Network(String),
    /// The session is missing or no longer accepted.
    #[error("需要登录: {0}")]
    Authentication(String),
    #[error("接口异常: {0}")]
    InvalidResponse(String),
    #[error("请求过于频繁: {0}")]
    RateLimited(String),
}

impl From<niveus_ncm_api::ApiError> for ApiFailure {
    fn from(error: niveus_ncm_api::ApiError) -> Self {
        use niveus_ncm_api::ApiError;
        match error {
            ApiError::Network(m) => ApiFailure::Network(m),
            ApiError::Authentication(m) => ApiFailure::Authentication(m),
            ApiError::InvalidResponse(m) => ApiFailure::InvalidResponse(m),
            ApiError::RateLimited(m) => ApiFailure::RateLimited(m),
        }
    }
}

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("{0}")]
    Api(#[from] ApiFailure),

    /// The service answered, but its data could not be expressed as a domain
    /// object.
    #[error("数据转换失败: {0}")]
    Mapping(String),

    /// The service answered normally and the requested thing does not exist or
    /// cannot be served — an unplayable track, an empty playlist.
    #[error("资源不可用: {0}")]
    MissingResource(String),
}

impl ProviderError {
    /// Whether signing in again could fix this.
    pub fn is_authentication(&self) -> bool {
        matches!(self, ProviderError::Api(ApiFailure::Authentication(_)))
    }
}

impl From<niveus_ncm_api::ApiError> for ProviderError {
    fn from(error: niveus_ncm_api::ApiError) -> Self {
        ProviderError::Api(error.into())
    }
}

pub type Result<T> = std::result::Result<T, ProviderError>;

#[cfg(test)]
mod tests {
    use super::*;
    use niveus_ncm_api::ApiError;

    #[test]
    fn authentication_failures_are_recognisable_across_the_boundary() {
        let error: ProviderError = ApiError::Authentication("过期".into()).into();
        assert!(error.is_authentication());

        let error: ProviderError = ApiError::Network("断网".into()).into();
        assert!(!error.is_authentication());
    }
}
