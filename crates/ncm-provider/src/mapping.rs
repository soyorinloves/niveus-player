//! Translates API payloads into domain objects.
//!
//! Kept as free functions over plain data so they can be tested without a
//! network or a client.

use niveus_music_core::{
    Album, AlbumId, Artist, ArtistId, AudioQuality, AudioSource, Playlist, PlaylistId,
    PlaylistKind, RawLyrics, Song, SongId,
};
use niveus_ncm_api::{LyricsDto, SongDto, SongUrlDto, UserPlaylistDto, SPECIAL_TYPE_LIKED};

use crate::error::{ProviderError, Result};
use crate::lyric_text;

/// Cover art is requested at this edge length.
///
/// Matches the reference geometry the design uses for album art (a 600×600
/// source rendered into a 40×12 cell region); the renderer downsamples locally,
/// so one size covers every layout tier and the URL stays cacheable.
pub const COVER_EDGE: u32 = 600;

/// Builds the cover URL the artwork pipeline will fetch.
///
/// The sizing parameter is applied here, centrally, so no other layer has to
/// know how the service expects it.
fn cover_url(raw: &str, edge: u32) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Replace any pre-existing query rather than appending a second one.
    let base = trimmed.split('?').next().unwrap_or(trimmed);
    Some(format!("{base}?param={edge}y{edge}"))
}

fn map_song(dto: SongDto) -> Option<Song> {
    if dto.id == 0 {
        // Placeholder entries appear for tracks that were removed from the
        // catalogue; they carry no usable data.
        tracing::debug!("skipping placeholder song entry");
        return None;
    }

    let artists = dto
        .ar
        .into_iter()
        .filter(|a| a.id != 0)
        .map(|a| Artist {
            id: ArtistId(a.id),
            name: a.name,
        })
        .collect();

    let album = dto.al.and_then(|al| {
        if al.id == 0 && al.name.trim().is_empty() {
            return None;
        }
        Some(Album {
            id: AlbumId(al.id),
            name: al.name,
            cover_url: al.pic_url.as_deref().and_then(|u| cover_url(u, COVER_EDGE)),
        })
    });

    Some(Song {
        id: SongId(dto.id),
        title: dto.name,
        artists,
        album,
        // A zero duration means the service has no duration for this track, not
        // that it lasts no time at all.
        duration_ms: dto.dt.filter(|ms| *ms > 0),
    })
}

/// Maps a page of songs, dropping unusable entries.
pub fn map_songs(dtos: Vec<SongDto>) -> Vec<Song> {
    dtos.into_iter().filter_map(map_song).collect()
}

/// Classifies a playlist from the signed-in user's own list.
///
/// Ownership decides created vs collected; the "liked songs" list is recognised
/// by its special type and is reachable through its own page instead.
pub fn map_playlist(dto: UserPlaylistDto, signed_in_uid: u64) -> Playlist {
    let owner = dto
        .user_id
        .or_else(|| dto.creator.as_ref().and_then(|c| c.user_id));

    let kind = if dto.special_type == Some(SPECIAL_TYPE_LIKED) {
        PlaylistKind::Liked
    } else if owner == Some(signed_in_uid) {
        PlaylistKind::Created
    } else {
        PlaylistKind::Collected
    };

    Playlist {
        id: PlaylistId(dto.id),
        name: dto.name,
        kind,
    }
}

pub fn map_playlists(dtos: Vec<UserPlaylistDto>, signed_in_uid: u64) -> Vec<Playlist> {
    dtos.into_iter()
        .filter(|dto| dto.id != 0)
        .map(|dto| map_playlist(dto, signed_in_uid))
        .collect()
}

/// Maps a resolved URL entry into a playable source.
pub fn map_audio_source(dto: SongUrlDto) -> Result<AudioSource> {
    let url = dto
        .url
        .filter(|u| !u.trim().is_empty())
        .ok_or_else(|| ProviderError::MissingResource("no audio source available".to_string()))?;

    Ok(AudioSource {
        url,
        // The tier actually served, which may be lower than the one requested.
        level: dto.level.as_deref().and_then(AudioQuality::from_api_level),
        format: dto.format.filter(|f| !f.trim().is_empty()),
        bitrate: dto.br.filter(|br| *br > 0),
        is_trial: dto.free_trial_info.is_some_and(|info| !info.is_null()),
    })
}

/// Maps a lyric payload to what the panel can show of it.
///
/// Three outcomes, and the difference between them is the point (SPEC §13.4):
/// real lyrics, only a translation, or nothing. A body that turns out to hold
/// nothing but credits is "nothing" — a track with no words.
///
/// A body that does have lyrics keeps only the lines that are words to sing:
/// credits go, because they are metadata about the track and the panel is three
/// lines tall (SPEC §13.7). The lines that stay are the service's own bytes —
/// filtering drops lines, it never rewrites them (SPEC §13.8).
pub fn map_lyrics(dto: LyricsDto) -> RawLyrics {
    let original = dto
        .lrc
        .and_then(|lrc| lrc.lyric)
        .filter(|text| !text.trim().is_empty());

    match original {
        Some(body) if lyric_text::has_real_lyrics(&body) => {
            RawLyrics::Original(lyric_text::without_placeholders(&body))
        }
        // The field was filled in, but everything in it is a credit or a note
        // that the track has no words. The panel shows this exactly as it shows
        // a track the service had nothing for.
        Some(_) => RawLyrics::Unavailable,
        None => {
            // Translations are never displayed, but their presence is what
            // tells the two kinds of "no original" apart.
            let translated = dto
                .tlyric
                .and_then(|translation| translation.lyric)
                .filter(|text| !text.trim().is_empty());
            if translated.is_some() {
                RawLyrics::TranslationOnly
            } else {
                RawLyrics::Unavailable
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use niveus_ncm_api::{AlbumDto, ArtistDto};

    fn song_dto(id: u64, with_album: bool, duration: Option<u32>) -> SongDto {
        SongDto {
            id,
            name: format!("song {id}"),
            ar: vec![ArtistDto {
                id: 7,
                name: "Artist".into(),
            }],
            al: with_album.then(|| AlbumDto {
                id: 9,
                name: "Album".into(),
                pic_url: Some("https://p3.music.126.net/cover.jpg".into()),
            }),
            dt: duration,
        }
    }

    #[test]
    fn a_song_maps_with_its_album_and_resized_cover() {
        let song = map_songs(vec![song_dto(1, true, Some(269_000))]).remove(0);
        assert_eq!(song.id, SongId(1));
        assert_eq!(song.title, "song 1");
        assert_eq!(song.artist_names(), "Artist");
        assert_eq!(song.duration_ms, Some(269_000));
        let album = song.album.unwrap();
        assert_eq!(album.id, AlbumId(9));
        assert_eq!(
            album.cover_url.as_deref(),
            Some("https://p3.music.126.net/cover.jpg?param=600y600")
        );
    }

    #[test]
    fn an_existing_query_string_is_replaced_not_appended() {
        let mut dto = song_dto(1, true, None);
        dto.al.as_mut().unwrap().pic_url =
            Some("https://p3.music.126.net/cover.jpg?param=100y100".into());
        let song = map_songs(vec![dto]).remove(0);
        assert_eq!(
            song.cover_url(),
            Some("https://p3.music.126.net/cover.jpg?param=600y600")
        );
    }

    #[test]
    fn a_blank_cover_url_becomes_no_cover_rather_than_an_empty_url() {
        let mut dto = song_dto(1, true, None);
        dto.al.as_mut().unwrap().pic_url = Some("   ".into());
        let song = map_songs(vec![dto]).remove(0);
        assert_eq!(song.cover_url(), None);
    }

    #[test]
    fn placeholder_and_zero_duration_entries_are_normalised() {
        let songs = map_songs(vec![song_dto(0, false, None), song_dto(2, false, Some(0))]);
        assert_eq!(songs.len(), 1);
        assert_eq!(songs[0].id, SongId(2));
        assert_eq!(songs[0].duration_ms, None);
        assert!(songs[0].album.is_none());
    }

    fn playlist_dto(id: u64, owner: u64, special: Option<i64>) -> UserPlaylistDto {
        UserPlaylistDto {
            id,
            name: format!("playlist {id}"),
            user_id: Some(owner),
            creator: None,
            special_type: special,
            track_count: None,
        }
    }

    #[test]
    fn ownership_decides_created_from_collected() {
        let playlists = map_playlists(
            vec![playlist_dto(1, 42, Some(0)), playlist_dto(2, 99, Some(0))],
            42,
        );
        assert_eq!(playlists[0].kind, PlaylistKind::Created);
        assert_eq!(playlists[1].kind, PlaylistKind::Collected);
    }

    #[test]
    fn the_liked_list_is_recognised_by_its_special_type_even_when_owned() {
        let playlists = map_playlists(vec![playlist_dto(1, 42, Some(SPECIAL_TYPE_LIKED))], 42);
        assert_eq!(playlists[0].kind, PlaylistKind::Liked);
        assert!(!playlists[0].kind.is_grouped());
    }

    #[test]
    fn a_missing_owner_falls_back_to_collected() {
        let mut dto = playlist_dto(1, 42, None);
        dto.user_id = None;
        let playlists = map_playlists(vec![dto], 42);
        assert_eq!(playlists[0].kind, PlaylistKind::Collected);
    }

    fn url_dto(url: Option<&str>, level: Option<&str>, trial: bool) -> SongUrlDto {
        SongUrlDto {
            id: 1,
            url: url.map(str::to_string),
            level: level.map(str::to_string),
            format: Some("flac".into()),
            br: Some(999_000),
            code: Some(200),
            free_trial_info: trial.then(|| serde_json::json!({"start": 0, "end": 30})),
        }
    }

    #[test]
    fn a_resolved_source_reports_the_served_tier_not_the_requested_one() {
        let source = map_audio_source(url_dto(
            Some("https://m8.music.126.net/a.flac"),
            Some("lossless"),
            false,
        ))
        .unwrap();
        assert_eq!(source.level, Some(AudioQuality::Lossless));
        assert_eq!(source.format.as_deref(), Some("flac"));
        assert_eq!(source.bitrate, Some(999_000));
        assert!(!source.is_trial);
    }

    #[test]
    fn a_trial_clip_is_flagged() {
        let source =
            map_audio_source(url_dto(Some("https://m8.music.126.net/t.mp3"), None, true)).unwrap();
        assert!(source.is_trial);
        // The served tier was not reported, so it stays unknown rather than
        // being guessed.
        assert_eq!(source.level, None);
    }

    #[test]
    fn an_unmodelled_tier_leaves_the_level_unknown() {
        let source =
            map_audio_source(url_dto(Some("https://x/y.flac"), Some("hires"), false)).unwrap();
        assert_eq!(source.level, None);
    }

    #[test]
    fn a_missing_url_is_a_missing_resource_not_a_mapping_failure() {
        let error = map_audio_source(url_dto(None, None, false)).unwrap_err();
        assert!(matches!(error, ProviderError::MissingResource(_)));
    }

    fn lyrics(payload: &str) -> RawLyrics {
        map_lyrics(serde_json::from_str(payload).unwrap())
    }

    #[test]
    fn lyrics_without_credits_are_passed_on_untouched() {
        let body = "[00:01.00]晴天\n[00:05.00]故事的小黄花";
        let mapped = lyrics(&format!(r#"{{"lrc":{{"lyric":{}}}}}"#, json_string(body)));
        assert_eq!(mapped, RawLyrics::Original(body.to_string()));
    }

    #[test]
    fn a_body_that_opens_with_credits_starts_at_the_first_line_of_lyrics() {
        // The panel is three lines tall; two of them spent on who wrote the
        // song is two rows of lyrics missing (SPEC §13.7).
        let body = "[00:00.00] 作词 : 田島貴男\n[00:01.00] 作曲 : 田島貴男\n[00:03.96]長く甘い口づけを交わす\n[00:13.57]深く果てしなくあなたを知りたい";
        let payload = format!(r#"{{"lrc":{{"lyric":{}}}}}"#, json_string(body));
        assert_eq!(
            lyrics(&payload),
            RawLyrics::Original(
                "[00:03.96]長く甘い口づけを交わす\n[00:13.57]深く果てしなくあなたを知りたい".into()
            )
        );
    }

    #[test]
    fn a_body_that_is_only_credits_is_still_no_lyrics_at_all() {
        // Filtering must not turn "this track has no words" into "this track has
        // words, there are just none left after filtering".
        let payload = r#"{"lrc":{"lyric":"[00:00.00]作曲 : BOY\n[00:00.00]编曲 : BOY"}}"#;
        assert_eq!(lyrics(payload), RawLyrics::Unavailable);
    }

    #[test]
    fn credits_in_the_middle_leave_the_rest_in_order_with_their_timestamps() {
        let payload =
            r#"{"lrc":{"lyric":"[00:01.00]第一句\n[00:02.00]编曲 : 某人\n[00:03.00]第二句"}}"#;
        assert_eq!(
            lyrics(payload),
            RawLyrics::Original("[00:01.00]第一句\n[00:03.00]第二句".into())
        );
    }

    #[test]
    fn blank_lyrics_are_no_lyrics_and_not_an_error() {
        assert_eq!(
            lyrics(r#"{"lrc":{"lyric":"   \n "}}"#),
            RawLyrics::Unavailable
        );
    }

    #[test]
    fn a_body_of_nothing_but_credits_is_no_lyrics() {
        let payload = r#"{"lrc":{"lyric":"[00:00.00]作曲 : BOY\n[00:00.00]编曲 : BOY"}}"#;
        assert_eq!(lyrics(payload), RawLyrics::Unavailable);
    }

    #[test]
    fn a_translation_without_an_original_is_told_apart_from_having_nothing() {
        let payload = r#"{"tlyric":{"lyric":"[00:01.00]Sunny day"}}"#;
        assert_eq!(lyrics(payload), RawLyrics::TranslationOnly);

        let payload = r#"{"lrc":{"lyric":""},"tlyric":{"lyric":"[00:01.00]Sunny day"}}"#;
        assert_eq!(lyrics(payload), RawLyrics::TranslationOnly);
    }

    #[test]
    fn an_original_that_is_all_placeholder_is_no_lyrics_even_with_a_translation() {
        let payload =
            r#"{"lrc":{"lyric":"[00:00.00]纯音乐，请欣赏"},"tlyric":{"lyric":"[00:00.00]x"}}"#;
        assert_eq!(lyrics(payload), RawLyrics::Unavailable);
    }

    #[test]
    fn a_payload_without_an_lrc_block_is_not_an_error() {
        assert_eq!(lyrics(r#"{"code":200}"#), RawLyrics::Unavailable);
    }

    /// A JSON string literal for a body, so the tests above can spell one out.
    fn json_string(text: &str) -> String {
        serde_json::to_string(text).unwrap()
    }
}
