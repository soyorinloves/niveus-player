//! Sign-in surface, expressed in this crate's own types.
//!
//! The shaped mirror of the API layer's QR flow exists so callers never import
//! anything from `niveus-ncm-api`.

use niveus_music_core::{User, UserId};
use niveus_ncm_api::{QrLoginState as ApiQrState, SignedInUser};

/// A QR code to display and poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrLogin {
    pub key: String,
    pub url: String,
}

/// Where a QR sign-in stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QrLoginState {
    Waiting,
    Scanned,
    Confirmed,
    Expired,
}

impl From<ApiQrState> for QrLoginState {
    fn from(state: ApiQrState) -> Self {
        match state {
            ApiQrState::Waiting => QrLoginState::Waiting,
            ApiQrState::Scanned => QrLoginState::Scanned,
            ApiQrState::Confirmed => QrLoginState::Confirmed,
            ApiQrState::Expired => QrLoginState::Expired,
        }
    }
}

/// Converts a signed-in account into a domain user.
///
/// A free function rather than a `From` impl: both types are foreign to this
/// crate, so the orphan rule forbids the impl.
pub(crate) fn to_domain_user(user: SignedInUser) -> User {
    User {
        id: UserId(user.id),
        name: user.name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_api_state_has_a_counterpart() {
        for (api, expected) in [
            (ApiQrState::Waiting, QrLoginState::Waiting),
            (ApiQrState::Scanned, QrLoginState::Scanned),
            (ApiQrState::Confirmed, QrLoginState::Confirmed),
            (ApiQrState::Expired, QrLoginState::Expired),
        ] {
            assert_eq!(QrLoginState::from(api), expected);
        }
    }

    #[test]
    fn a_signed_in_user_becomes_a_domain_user() {
        let user = to_domain_user(SignedInUser {
            id: 42,
            name: "someone".into(),
        });
        assert_eq!(user.id, UserId(42));
        assert_eq!(user.name, "someone");
    }
}
