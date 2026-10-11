//! Identifier newtypes.
//!
//! Ids are kept in distinct types so that a playlist id can never be passed
//! where a song id is expected.

use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! define_id {
    ($name:ident, $inner:ty, $doc:literal) => {
        #[doc = $doc]
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub $inner);

        impl $name {
            pub fn get(self) -> $inner {
                self.0
            }
        }

        impl From<$inner> for $name {
            fn from(value: $inner) -> Self {
                Self(value)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

define_id!(SongId, u64, "Identifies a song.");
define_id!(PlaylistId, u64, "Identifies a playlist.");
define_id!(ArtistId, u64, "Identifies an artist.");
define_id!(AlbumId, u64, "Identifies an album.");
define_id!(UserId, u64, "Identifies a user.");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_ordered_and_transparent_when_serialised() {
        assert!(SongId(1) < SongId(2));
        assert_eq!(serde_json::to_string(&SongId(42)).unwrap(), "42");
        assert_eq!(serde_json::from_str::<SongId>("42").unwrap(), SongId(42));
        assert_eq!(PlaylistId::from(7u64).get(), 7);
    }
}
