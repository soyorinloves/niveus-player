//! Types mirroring the upstream payloads.
//!
//! These are this crate's own types. They must not escape into `ncm-provider`;
//! that crate works with `niveus-music-core` types instead.
//!
//! Field names follow the wire format, hence the `serde(rename)` calls. A name
//! that does not match fails silently — the field is simply `None` — so every
//! one of these needs a test holding a real payload, and the ones that could
//! only be observed while signed in need one written from a captured response.

use serde::Deserialize;

// ---------------------------------------------------------------------------
// Authentication
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct QrKeyDto {
    /// The QR session key. Note this sits at the top level of the body, not
    /// under `data`.
    pub unikey: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QrCheckDto {
    /// 800 expired, 801 awaiting scan, 802 scanned, 803 confirmed.
    pub code: i64,
    #[serde(default)]
    pub message: Option<String>,
}

/// Unscanned QR sessions come back with this code when the key is not usable.
pub const QR_CHECK_INVALID: i64 = 400;

#[derive(Debug, Clone, Deserialize)]
pub struct LoginStatusDto {
    /// Present only while signed in. The service reports a signed-out session as
    /// `profile: null` with `code: 200` rather than as an error.
    #[serde(default)]
    pub profile: Option<ProfileDto>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProfileDto {
    /// The service spells this `userId`; the all-lowercase form is accepted as
    /// a fallback because other endpoints in this family use it.
    ///
    /// Getting this wrong is not a visible failure: the field simply comes back
    /// `None`, and a perfectly valid session is then reported as signed out.
    #[serde(rename = "userId", alias = "userid")]
    pub user_id: Option<u64>,
    #[serde(default)]
    pub nickname: Option<String>,
}

// ---------------------------------------------------------------------------
// Catalogue
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct ArtistDto {
    pub id: u64,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AlbumDto {
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(rename = "picUrl", default)]
    pub pic_url: Option<String>,
}

/// A song as returned by `song_detail` / `playlist_track_all`.
#[derive(Debug, Clone, Deserialize)]
pub struct SongDto {
    pub id: u64,
    #[serde(default)]
    pub name: String,
    /// Artists. Named `ar` in v3 payloads.
    #[serde(default)]
    pub ar: Vec<ArtistDto>,
    /// Album. Absent for some catalogue entries.
    #[serde(default)]
    pub al: Option<AlbumDto>,
    /// Duration in milliseconds.
    #[serde(default)]
    pub dt: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserPlaylistDto {
    pub id: u64,
    #[serde(default)]
    pub name: String,
    /// The playlist's owner.
    #[serde(rename = "userId", default)]
    pub user_id: Option<u64>,
    #[serde(default)]
    pub creator: Option<CreatorDto>,
    /// `5` marks the "liked songs" list.
    #[serde(rename = "specialType", default)]
    pub special_type: Option<i64>,
    #[serde(rename = "trackCount", default)]
    pub track_count: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreatorDto {
    #[serde(rename = "userId", default)]
    pub user_id: Option<u64>,
    #[serde(default)]
    pub nickname: Option<String>,
}

/// The special type the service uses for "我喜欢的音乐".
pub const SPECIAL_TYPE_LIKED: i64 = 5;

// ---------------------------------------------------------------------------
// Playback sources and lyrics
// ---------------------------------------------------------------------------

/// One entry of `song_url_v1`'s `data` array.
#[derive(Debug, Clone, Deserialize)]
pub struct SongUrlDto {
    pub id: u64,
    #[serde(default)]
    pub url: Option<String>,
    /// The tier actually served, e.g. `exhigh`. `None` when nothing was served.
    #[serde(default)]
    pub level: Option<String>,
    /// Container format, e.g. `mp3` or `flac`.
    #[serde(rename = "type", default)]
    pub format: Option<String>,
    /// Bitrate in bits per second.
    #[serde(default)]
    pub br: Option<u32>,
    /// Per-entry status. `404` means the track is not playable for this
    /// account, which arrives alongside HTTP 200 and a null `url`.
    #[serde(default)]
    pub code: Option<i64>,
    #[serde(rename = "freeTrialInfo", default)]
    pub free_trial_info: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LyricsDto {
    #[serde(default)]
    pub lrc: Option<LyricBodyDto>,
    /// Translation; not used in the first release.
    #[serde(default)]
    pub tlyric: Option<LyricBodyDto>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LyricBodyDto {
    #[serde(default)]
    pub lyric: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LikelistDto {
    #[serde(default)]
    pub ids: Vec<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn song_dto_matches_the_observed_wire_shape() {
        let raw = r#"{
            "id": 186016,
            "name": "晴天",
            "ar": [{"id": 6452, "name": "周杰伦"}],
            "al": {"id": 18905, "name": "叶惠美", "picUrl": "https://p3.music.126.net/x.jpg"},
            "dt": 269000
        }"#;
        let song: SongDto = serde_json::from_str(raw).unwrap();
        assert_eq!(song.id, 186016);
        assert_eq!(song.name, "晴天");
        assert_eq!(song.ar[0].name, "周杰伦");
        assert!(!song
            .al
            .as_ref()
            .unwrap()
            .pic_url
            .as_deref()
            .unwrap()
            .is_empty());
        assert_eq!(song.dt, Some(269000));
    }

    #[test]
    fn a_song_without_album_or_duration_still_parses() {
        let raw = r#"{"id": 1, "name": "x", "ar": []}"#;
        let song: SongDto = serde_json::from_str(raw).unwrap();
        assert!(song.al.is_none());
        assert!(song.dt.is_none());
    }

    #[test]
    fn an_unplayable_url_entry_parses_with_a_null_url() {
        let raw =
            r#"{"id": 186016, "url": null, "level": null, "type": null, "br": 0, "code": 404}"#;
        let entry: SongUrlDto = serde_json::from_str(raw).unwrap();
        assert_eq!(entry.url, None);
        assert_eq!(entry.code, Some(404));
    }

    #[test]
    fn a_signed_out_status_parses_as_a_missing_profile() {
        let raw = r#"{"account": null, "code": 200, "profile": null}"#;
        let status: LoginStatusDto = serde_json::from_str(raw).unwrap();
        assert!(status.profile.is_none());
    }

    /// The shape the service actually returns once signed in.
    ///
    /// Abridged from a real response, but the field spellings are exact — a
    /// wrong name here is invisible, because the value simply disappears.
    #[test]
    fn a_signed_in_status_yields_the_account_id() {
        let raw = r#"{
            "account": {"id": 3885698826, "type": 1, "vipType": 11},
            "code": 200,
            "profile": {
                "accountStatus": 0,
                "avatarUrl": "https://p4.music.126.net/x.jpg",
                "birthday": 1136044800000,
                "city": 1007700,
                "createTime": 1599561868916,
                "nickname": "listener",
                "userId": 3885698826,
                "vipType": 11
            }
        }"#;
        let status: LoginStatusDto = serde_json::from_str(raw).unwrap();
        let profile = status.profile.expect("the profile is present");
        assert_eq!(profile.user_id, Some(3885698826));
        assert_eq!(profile.nickname.as_deref(), Some("listener"));
    }

    #[test]
    fn the_lowercase_spelling_is_still_accepted() {
        let raw = r#"{"profile": {"userid": 7, "nickname": "x"}}"#;
        let status: LoginStatusDto = serde_json::from_str(raw).unwrap();
        assert_eq!(status.profile.unwrap().user_id, Some(7));
    }

    #[test]
    fn playlist_owner_fields_accept_the_wire_names() {
        let raw = r#"{
            "id": 123, "name": "我的歌单", "userId": 42,
            "creator": {"userId": 42, "nickname": "me"},
            "specialType": 5, "trackCount": 17
        }"#;
        let playlist: UserPlaylistDto = serde_json::from_str(raw).unwrap();
        assert_eq!(playlist.user_id, Some(42));
        assert_eq!(playlist.creator.unwrap().nickname.as_deref(), Some("me"));
        assert_eq!(playlist.special_type, Some(SPECIAL_TYPE_LIKED));
        assert_eq!(playlist.track_count, Some(17));
    }
}
