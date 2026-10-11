//! User playlists and the signed-in user.

use crate::id::{PlaylistId, UserId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaylistKind {
    /// Created by the user.
    Created,
    /// Collected (subscribed) by the user.
    Collected,
    /// The "liked songs" list, which the provider maps from the like list.
    Liked,
}

impl PlaylistKind {
    /// Whether playlists of this kind appear in the browsable grouped list.
    ///
    /// `Liked` is reachable through its own secondary item instead, so it is
    /// never a group of the playlist list.
    pub fn is_grouped(self) -> bool {
        matches!(self, PlaylistKind::Created | PlaylistKind::Collected)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Playlist {
    pub id: PlaylistId,
    pub name: String,
    pub kind: PlaylistKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: UserId,
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_created_and_collected_are_grouped() {
        assert!(PlaylistKind::Created.is_grouped());
        assert!(PlaylistKind::Collected.is_grouped());
        assert!(!PlaylistKind::Liked.is_grouped());
    }

    #[test]
    fn kind_round_trips_through_serde() {
        let json = serde_json::to_string(&PlaylistKind::Collected).unwrap();
        assert_eq!(json, "\"collected\"");
        assert_eq!(
            serde_json::from_str::<PlaylistKind>(&json).unwrap(),
            PlaylistKind::Collected
        );
    }
}
