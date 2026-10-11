//! The service the desktop talks to.
//!
//! Two interfaces live on one object path, as the specification lays them out:
//! the application half (`org.mpris.MediaPlayer2`) and the playback half
//! (`.Player`).
//!
//! Nothing here decides anything. The playback half's methods put a
//! [`MprisCommand`] on a channel and return; the application takes it from
//! there and treats it exactly as it treats a key press. Everything outward —
//! the properties and the signals that carry them — is pushed in by
//! [`MprisService::update`], so the arrow runs one way whether the desktop is
//! reading or writing.

use crate::command::MprisCommand;
use crate::error::MprisError;
use crate::snapshot::{track_path, Snapshot};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::UnboundedSender;
use zbus::fdo::RequestNameFlags;
use zbus::fdo::RequestNameReply;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, Value};
use zbus::Connection;

/// The name the desktop looks for.
pub const SERVICE_NAME: &str = "org.mpris.MediaPlayer2.niveus";

/// Where both interfaces live.
pub const OBJECT_PATH: &str = "/org/mpris/MediaPlayer2";

/// What the desktop is told the program is called.
const IDENTITY: &str = "Niveus";

/// The state the interfaces read from, and the application writes to.
type Shared = Arc<Mutex<Snapshot>>;

/// The application half of the interface.
struct Root {
    commands: UnboundedSender<MprisCommand>,
}

#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    #[zbus(property)]
    fn identity(&self) -> &'static str {
        IDENTITY
    }

    /// The desktop entry this program is launched by, when the desktop wants
    /// to name it in a launcher.
    #[zbus(property)]
    fn desktop_entry(&self) -> &'static str {
        "niveus"
    }

    #[zbus(property)]
    fn can_quit(&self) -> bool {
        true
    }

    /// Niveus is a terminal program: there is no window to raise.
    #[zbus(property)]
    fn can_raise(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }

    fn quit(&self) {
        let _ = self.commands.send(MprisCommand::Quit);
    }

    fn raise(&self) -> zbus::fdo::Result<()> {
        Err(zbus::fdo::Error::NotSupported(
            "Niveus 是终端程序，没有可以置前的窗口".to_string(),
        ))
    }
}

/// The playback half.
#[derive(Clone)]
struct Player {
    state: Shared,
    commands: UnboundedSender<MprisCommand>,
}

#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl Player {
    #[zbus(property)]
    fn playback_status(&self) -> String {
        self.snapshot().playback_status().to_string()
    }

    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, Value<'static>> {
        let snapshot = self.snapshot();
        let mut metadata = HashMap::new();

        metadata.insert(
            "mpris:trackid".to_string(),
            Value::from(object_path(&track_path(snapshot.song.as_ref()))),
        );

        let Some(song) = snapshot.song else {
            return metadata;
        };

        metadata.insert("xesam:title".to_string(), Value::from(song.title.clone()));
        metadata.insert(
            "xesam:artist".to_string(),
            Value::from(
                song.artists
                    .iter()
                    .map(|artist| artist.name.clone())
                    .collect::<Vec<_>>(),
            ),
        );
        if let Some(album) = &song.album {
            metadata.insert("xesam:album".to_string(), Value::from(album.name.clone()));
        }
        if let Some(duration_ms) = song.duration_ms {
            // The interface counts microseconds.
            metadata.insert(
                "mpris:length".to_string(),
                Value::from(i64::from(duration_ms) * 1000),
            );
        }
        if let Some(cover) = song.cover_url() {
            // A remote address, given as it is: the desktop fetches the cover
            // itself, and nothing has to be written to a temporary file.
            metadata.insert("mpris:artUrl".to_string(), Value::from(cover.to_string()));
        }

        metadata
    }

    #[zbus(property)]
    fn position(&self) -> i64 {
        self.snapshot().position_us()
    }

    #[zbus(property)]
    fn volume(&self) -> f64 {
        self.snapshot().volume_fraction()
    }

    /// Sets the volume, as the desktop asks for it — a fraction, not a percent.
    #[zbus(property)]
    fn set_volume(&self, value: f64) {
        let percent = (value.clamp(0.0, 1.0) * 100.0).round() as u8;
        let _ = self.commands.send(MprisCommand::SetVolume(percent));
    }

    /// Niveus plays at one speed; the interface asks for the range it can take.
    #[zbus(property)]
    fn rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn can_play(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_pause(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_seek(&self) -> bool {
        self.snapshot().can_seek()
    }

    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }

    fn play(&self) {
        let _ = self.commands.send(MprisCommand::Play);
    }

    fn pause(&self) {
        let _ = self.commands.send(MprisCommand::Pause);
    }

    fn play_pause(&self) {
        let _ = self.commands.send(MprisCommand::PlayPause);
    }

    fn stop(&self) {
        let _ = self.commands.send(MprisCommand::Stop);
    }

    fn next(&self) {
        let _ = self.commands.send(MprisCommand::Next);
    }

    fn previous(&self) {
        let _ = self.commands.send(MprisCommand::Previous);
    }

    /// Moves the position by an offset in microseconds.
    fn seek(&self, offset: i64) {
        let _ = self.commands.send(MprisCommand::Seek(offset));
    }

    /// Jumps to a position in microseconds.
    ///
    /// The track id is checked, because the interface says a request for a
    /// track that is no longer playing is a mistake rather than a seek.
    fn set_position(&self, track_id: ObjectPath<'_>, position: i64) -> zbus::fdo::Result<()> {
        let snapshot = self.snapshot();
        let wanted = track_path(snapshot.song.as_ref());
        if track_id.as_str() != wanted {
            return Err(zbus::fdo::Error::InvalidArgs(format!(
                "请求的曲目 {track_id} 不是当前播放的曲目"
            )));
        }
        let _ = self.commands.send(MprisCommand::SetPosition(position));
        Ok(())
    }

    /// The position jumped, rather than advanced.
    #[zbus(signal)]
    async fn seeked(emitter: &SignalEmitter<'_>, position: i64) -> zbus::Result<()>;
}

/// The path the no-longer-playing track reports, which the interface names.
const NO_TRACK: &str = "/org/mpris/MediaPlayer2/TrackList/NoTrack";

/// Turns a path into the owned form the interface's metadata takes.
///
/// The no-track path is the fallback rather than an error: a malformed path can
/// only come from a song id, and a desktop that sees "no track" is no worse off
/// than one that sees nothing at all.
fn object_path(path: &str) -> OwnedObjectPath {
    OwnedObjectPath::try_from(path.to_string())
        .or_else(|_| OwnedObjectPath::try_from(NO_TRACK.to_string()))
        .expect("the no-track path is a valid object path")
}

impl Player {
    fn snapshot(&self) -> Snapshot {
        self.state
            .lock()
            .expect("the MPRIS snapshot lock is never held across a panic")
            .clone()
    }
}

/// Niveus, as the desktop sees it.
pub struct MprisService {
    connection: Connection,
    state: Shared,
    player: Player,
}

impl MprisService {
    /// Puts Niveus on the session bus.
    ///
    /// Fails — without disturbing anything — when there is no session bus,
    /// which is the ordinary state of affairs under a bare container or a
    /// serial console. The program runs without the desktop knowing about it.
    pub async fn start(
        commands: UnboundedSender<MprisCommand>,
    ) -> Result<MprisService, MprisError> {
        let connection = Connection::session().await.map_err(MprisError::Connect)?;

        // Asked for rather than queued: a second Niveus must not believe it is
        // the one the desktop is talking to when the first still owns the name.
        let reply = connection
            .request_name_with_flags(SERVICE_NAME, RequestNameFlags::DoNotQueue.into())
            .await
            .map_err(MprisError::Name)?;
        if reply != RequestNameReply::PrimaryOwner {
            return Err(MprisError::Name(zbus::Error::Failure(format!(
                "服务名 {SERVICE_NAME} 已被占用（{reply:?}）"
            ))));
        }

        let state: Shared = Arc::new(Mutex::new(Snapshot::default()));
        let player = Player {
            state: Arc::clone(&state),
            commands: commands.clone(),
        };

        connection
            .object_server()
            .at(OBJECT_PATH, Root { commands })
            .await
            .map_err(MprisError::Serve)?;
        connection
            .object_server()
            .at(OBJECT_PATH, player.clone())
            .await
            .map_err(MprisError::Serve)?;

        tracing::info!(service = SERVICE_NAME, "desktop integration is up");

        Ok(MprisService {
            connection,
            state,
            player,
        })
    }

    /// Publishes what is playing now.
    ///
    /// Only what actually changed is announced, so a desktop is not woken for
    /// a progress tick.
    pub async fn update(&self, snapshot: Snapshot) -> Result<(), MprisError> {
        let (status_changed, metadata_changed, volume_changed) = {
            let mut current = self
                .state
                .lock()
                .expect("the MPRIS snapshot lock is never held across a panic");
            let changed = (
                current.status != snapshot.status,
                current.song != snapshot.song,
                current.volume != snapshot.volume,
            );
            *current = snapshot;
            changed
        };

        let emitter =
            SignalEmitter::new(&self.connection, OBJECT_PATH).map_err(MprisError::Serve)?;

        if status_changed {
            self.player
                .playback_status_changed(&emitter)
                .await
                .map_err(MprisError::Serve)?;
        }
        if metadata_changed {
            self.player
                .metadata_changed(&emitter)
                .await
                .map_err(MprisError::Serve)?;
        }
        if volume_changed {
            self.player
                .volume_changed(&emitter)
                .await
                .map_err(MprisError::Serve)?;
        }
        Ok(())
    }

    /// Announces that the position jumped rather than ran on.
    ///
    /// The interface asks for this instead of a property change: a desktop
    /// that tried to follow the position by property would be woken several
    /// times a second.
    pub async fn seeked(&self, position_ms: u64) -> Result<(), MprisError> {
        let emitter =
            SignalEmitter::new(&self.connection, OBJECT_PATH).map_err(MprisError::Serve)?;
        Player::seeked(&emitter, (position_ms as i64) * 1000)
            .await
            .map_err(MprisError::Serve)
    }
}
