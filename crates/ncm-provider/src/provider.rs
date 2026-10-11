//! The NetEase-backed provider.

use crate::auth::{QrLogin, QrLoginState};
use crate::error::{ProviderError, Result};
use crate::mapping;
use niveus_music_core::{
    AudioQuality, AudioSource, Playlist, PlaylistId, RawLyrics, Song, SongId, User,
};
use niveus_ncm_api::NcmApiClient;
use std::sync::RwLock;
use std::time::Duration;

/// How many items a single page request asks for.
const PAGE_SIZE: u32 = 1000;
/// Upper bound on pages walked, so a misbehaving service cannot spin forever.
const MAX_PAGES: u32 = 20;

/// Music service backed by the NetEase API.
///
/// Holds the session and the signed-in account. Nothing outside this crate
/// learns which service is behind it.
pub struct NcmProvider {
    api: NcmApiClient,
    /// `None` until a session is confirmed.
    user: RwLock<Option<User>>,
}

impl NcmProvider {
    /// Constructs the provider around a client. Kept private so the concrete
    /// client type stays out of this crate's public API.
    fn new(api: NcmApiClient) -> Self {
        Self {
            api,
            user: RwLock::new(None),
        }
    }

    /// A provider with no session yet, ready for QR sign-in.
    pub fn anonymous() -> Self {
        Self::new(NcmApiClient::new())
    }

    /// Restores a session from a persisted cookie.
    pub fn with_cookie(cookie: impl Into<String>) -> Self {
        Self::new(NcmApiClient::with_cookie(cookie))
    }

    /// The session cookie, ready to be persisted.
    pub fn cookie(&self) -> String {
        self.api.cookie()
    }

    pub fn set_cookie(&self, cookie: &str) {
        self.api.set_cookie(cookie);
        // The account behind a replaced cookie is unknown until checked again.
        self.clear_user();
    }

    pub fn user(&self) -> Option<User> {
        self.user.read().expect("user lock poisoned").clone()
    }

    pub fn set_user(&self, user: User) {
        *self.user.write().expect("user lock poisoned") = Some(user);
    }

    pub fn clear_user(&self) {
        *self.user.write().expect("user lock poisoned") = None;
    }

    /// Asks the service who the stored cookie belongs to.
    ///
    /// `Ok(None)` means the session is gone — expired, revoked, or never
    /// established. That is a normal outcome the caller answers with a QR
    /// sign-in, not an error.
    pub async fn restore_session(&self) -> Result<Option<User>> {
        match self.api.login_status().await? {
            Some(signed_in) => {
                let user = crate::auth::to_domain_user(signed_in);
                self.set_user(user.clone());
                Ok(Some(user))
            }
            None => {
                self.clear_user();
                Ok(None)
            }
        }
    }

    /// Begins a QR sign-in.
    pub async fn start_qr_login(&self) -> Result<QrLogin> {
        let login = self.api.qr_login_start().await?;
        Ok(QrLogin {
            key: login.key,
            url: login.url,
        })
    }

    /// Polls a QR sign-in. On confirmation the session is stored.
    pub async fn poll_qr_login(&self, key: &str) -> Result<QrLoginState> {
        let state = self.api.qr_login_poll(key).await?;
        Ok(state.into())
    }

    /// Confirms the freshly-established session and records the account.
    ///
    /// Retries briefly before giving up. This is a guard, not a known fix: the
    /// cost of retrying is a few seconds, while the cost of a false negative is
    /// a user having to fetch their phone and scan again. Only a session the
    /// service positively recognises is accepted.
    pub async fn finish_qr_login(&self) -> Result<User> {
        const ATTEMPTS: usize = 5;
        const RETRY_DELAY: Duration = Duration::from_millis(600);

        let mut last_error = None;
        for attempt in 0..ATTEMPTS {
            match self.restore_session().await {
                Ok(Some(user)) => return Ok(user),
                Ok(None) => {
                    last_error = None;
                    tracing::debug!(attempt, "session not recognised yet");
                }
                Err(error) => last_error = Some(error),
            }
            if attempt + 1 < ATTEMPTS {
                tokio::time::sleep(RETRY_DELAY).await;
            }
        }

        // Reached only when the service keeps returning a null profile, which
        // means it does not recognise the session rather than that anything
        // went wrong on the wire. Saying so points at the right thing.
        Err(last_error.unwrap_or_else(|| {
            ProviderError::MissingResource("服务端未确认本次登录，请重新扫码".to_string())
        }))
    }

    /// The signed-in account, or an authentication failure.
    fn signed_in(&self) -> Result<User> {
        self.user().ok_or_else(|| {
            ProviderError::Api(crate::error::ApiFailure::Authentication(
                "尚未登录".to_string(),
            ))
        })
    }

    /// Every track of a playlist, in playlist order.
    async fn fetch_playlist_tracks(&self, playlist_id: u64) -> Result<Vec<Song>> {
        let mut songs = Vec::new();
        for page in 0..MAX_PAGES {
            let offset = page * PAGE_SIZE;
            let batch = self
                .api
                .playlist_tracks(playlist_id, PAGE_SIZE, offset)
                .await?;
            let received = batch.len();
            songs.extend(mapping::map_songs(batch));

            if received < PAGE_SIZE as usize {
                break;
            }
        }
        Ok(songs)
    }

    /// Details for a set of ids, in the order given.
    async fn fetch_songs(&self, ids: Vec<u64>) -> Result<Vec<Song>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let dtos = self.api.song_details(&ids).await?;
        Ok(mapping::map_songs(dtos))
    }
}

impl crate::MusicProvider for NcmProvider {
    async fn get_user_playlists(&self) -> Result<Vec<Playlist>> {
        let user = self.signed_in()?;
        let dtos = self.api.user_playlists(user.id.get()).await?;
        Ok(mapping::map_playlists(dtos, user.id.get()))
    }

    async fn get_playlist_songs(&self, playlist_id: PlaylistId) -> Result<Vec<Song>> {
        self.fetch_playlist_tracks(playlist_id.get()).await
    }

    async fn get_liked_songs(&self) -> Result<Vec<Song>> {
        let user = self.signed_in()?;
        let ids = self.api.liked_song_ids(user.id.get()).await?;
        self.fetch_songs(ids).await
    }

    /// Resolves a playable source.
    ///
    /// The tier returned is whatever the account can actually get: asking for
    /// `lossless` without a subscription yields a lower tier, or a trial clip,
    /// and the caller must show what it got rather than what it asked for.
    async fn get_song_url(&self, song_id: SongId, level: AudioQuality) -> Result<AudioSource> {
        match self
            .api
            .song_url(song_id.get(), level.as_api_level())
            .await?
        {
            Some(dto) => mapping::map_audio_source(dto),
            None => Err(ProviderError::MissingResource(format!(
                "该歌曲暂不可播放 (id={})",
                song_id.get()
            ))),
        }
    }

    async fn get_lyrics(&self, song_id: SongId) -> Result<RawLyrics> {
        let dto = self.api.lyrics(song_id.get()).await?;
        Ok(mapping::map_lyrics(dto))
    }
}
