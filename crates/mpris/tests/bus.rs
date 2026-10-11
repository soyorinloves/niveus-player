//! The service, driven over a real session bus.
//!
//! A stand-in bus would only prove the code agrees with itself; what matters is
//! whether a desktop's own tools can see and drive Niveus. These tests use the
//! same interface a media card does, over whatever session bus the machine has.
//!
//! Skipped, with a note, where there is no session bus — that is the ordinary
//! state of affairs under a bare container or a serial console, and it is a
//! case the program itself is expected to survive.

use futures_lite::StreamExt;
use niveus_mpris::{MprisCommand, MprisService, Snapshot, SERVICE_NAME};
use niveus_music_core::{Album, AlbumId, Artist, ArtistId, PlaybackStatus, Song, SongId};
use std::collections::HashMap;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};
use tokio::sync::Mutex;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue};

/// One object name can be owned once, so the tests take turns.
static BUS: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();

/// Takes the turn.
///
/// An asynchronous lock, so that holding it across the bus calls the test makes
/// is what it looks like rather than a hazard.
async fn take_turn() -> tokio::sync::MutexGuard<'static, ()> {
    BUS.get_or_init(|| Mutex::new(())).lock().await
}

/// The playback half, declared the way a desktop client would declare it.
#[zbus::proxy(
    interface = "org.mpris.MediaPlayer2.Player",
    default_service = "org.mpris.MediaPlayer2.niveus",
    default_path = "/org/mpris/MediaPlayer2"
)]
trait Player {
    fn play(&self) -> zbus::Result<()>;
    fn pause(&self) -> zbus::Result<()>;
    fn play_pause(&self) -> zbus::Result<()>;
    fn stop(&self) -> zbus::Result<()>;
    fn next(&self) -> zbus::Result<()>;
    fn previous(&self) -> zbus::Result<()>;
    fn seek(&self, offset: i64) -> zbus::Result<()>;
    fn set_position(&self, track_id: ObjectPath<'_>, position: i64) -> zbus::Result<()>;

    #[zbus(property)]
    fn playback_status(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn metadata(&self) -> zbus::Result<HashMap<String, OwnedValue>>;
    #[zbus(property)]
    fn position(&self) -> zbus::Result<i64>;
    #[zbus(property)]
    fn volume(&self) -> zbus::Result<f64>;
    #[zbus(property)]
    fn set_volume(&self, value: f64) -> zbus::Result<()>;
    #[zbus(property)]
    fn can_seek(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn identity(&self) -> zbus::Result<String>;
}

/// The application half, likewise.
#[zbus::proxy(
    interface = "org.mpris.MediaPlayer2",
    default_service = "org.mpris.MediaPlayer2.niveus",
    default_path = "/org/mpris/MediaPlayer2"
)]
trait MediaPlayer2 {
    fn quit(&self) -> zbus::Result<()>;

    #[zbus(property)]
    fn identity(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn desktop_entry(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn has_track_list(&self) -> zbus::Result<bool>;
}

fn song(id: u64) -> Song {
    Song {
        id: SongId(id),
        title: "海阔天空".into(),
        artists: vec![Artist {
            id: ArtistId(1),
            name: "Beyond".into(),
        }],
        album: Some(Album {
            id: AlbumId(2),
            name: "乐与怒".into(),
            cover_url: Some("https://example.invalid/cover.jpg".into()),
        }),
        duration_ms: Some(326_000),
    }
}

/// Starts the service, or explains why it cannot be started here.
async fn service() -> Option<(MprisService, UnboundedReceiver<MprisCommand>)> {
    let (commands, inbox) = unbounded_channel();
    match MprisService::start(commands).await {
        Ok(service) => Some((service, inbox)),
        Err(error) => {
            eprintln!("没有可用的会话总线，跳过 MPRIS 测试：{error}");
            None
        }
    }
}

/// The next command, or a panic naming what was expected.
async fn next_command(inbox: &mut UnboundedReceiver<MprisCommand>) -> MprisCommand {
    tokio::time::timeout(std::time::Duration::from_secs(5), inbox.recv())
        .await
        .expect("a command arrives")
        .expect("the channel is open")
}

#[tokio::test]
async fn the_desktop_can_see_who_is_playing() {
    let _bus = take_turn().await;
    let Some((service, _inbox)) = service().await else {
        return;
    };

    let connection = zbus::Connection::session().await.unwrap();
    let player = PlayerProxy::new(&connection).await.unwrap();
    let application = MediaPlayer2Proxy::new(&connection).await.unwrap();

    assert_eq!(application.identity().await.unwrap(), "Niveus");
    assert_eq!(application.desktop_entry().await.unwrap(), "niveus");
    assert!(!application.has_track_list().await.unwrap());

    service
        .update(Snapshot {
            status: PlaybackStatus::Playing,
            song: Some(song(1)),
            position_ms: 97_000,
            volume: 72,
        })
        .await
        .unwrap();

    assert_eq!(player.playback_status().await.unwrap(), "Playing");
    assert_eq!(player.position().await.unwrap(), 97_000_000);
    assert_eq!(player.volume().await.unwrap(), 0.72);
    assert!(player.can_seek().await.unwrap());
}

#[tokio::test]
async fn the_metadata_carries_the_track_the_card_shows() {
    let _bus = take_turn().await;
    let Some((service, _inbox)) = service().await else {
        return;
    };

    service
        .update(Snapshot {
            status: PlaybackStatus::Playing,
            song: Some(song(1)),
            position_ms: 0,
            volume: 72,
        })
        .await
        .unwrap();

    let connection = zbus::Connection::session().await.unwrap();
    let player = PlayerProxy::new(&connection).await.unwrap();
    let metadata = player.metadata().await.unwrap();

    let text = |key: &str| -> String {
        String::try_from(metadata.get(key).expect(key).clone()).expect("a string")
    };
    assert_eq!(text("xesam:title"), "海阔天空");
    assert_eq!(text("xesam:album"), "乐与怒");
    // A remote address, handed over as it is: nothing is written to disk.
    assert_eq!(text("mpris:artUrl"), "https://example.invalid/cover.jpg");

    let artists: Vec<String> =
        Vec::try_from(metadata.get("xesam:artist").expect("artists").clone())
            .expect("a list of strings");
    assert_eq!(artists, vec!["Beyond".to_string()]);

    let length = i64::try_from(metadata.get("mpris:length").expect("length").clone()).unwrap();
    assert_eq!(length, 326_000_000, "the length is not in microseconds");

    let track = OwnedObjectPath::try_from(metadata.get("mpris:trackid").expect("trackid").clone())
        .expect("an object path");
    assert_eq!(track.as_str(), "/org/niveus/track/s1");
}

#[tokio::test]
async fn with_nothing_playing_the_card_has_a_track_id_and_nothing_else() {
    let _bus = take_turn().await;
    let Some((_service, _inbox)) = service().await else {
        return;
    };

    let connection = zbus::Connection::session().await.unwrap();
    let player = PlayerProxy::new(&connection).await.unwrap();

    let metadata = player.metadata().await.unwrap();
    // The id is an object path on the wire, not a string.
    let track = OwnedObjectPath::try_from(metadata.get("mpris:trackid").expect("trackid").clone())
        .expect("an object path");
    assert_eq!(track.as_str(), "/org/mpris/MediaPlayer2/TrackList/NoTrack");
    assert!(!metadata.contains_key("xesam:title"));
    assert_eq!(player.playback_status().await.unwrap(), "Stopped");
    assert!(!player.can_seek().await.unwrap());
}

#[tokio::test]
async fn the_media_keys_arrive_as_commands() {
    let _bus = take_turn().await;
    let Some((_service, mut inbox)) = service().await else {
        return;
    };

    let connection = zbus::Connection::session().await.unwrap();
    let player = PlayerProxy::new(&connection).await.unwrap();

    player.play_pause().await.unwrap();
    assert_eq!(next_command(&mut inbox).await, MprisCommand::PlayPause);

    player.play().await.unwrap();
    assert_eq!(next_command(&mut inbox).await, MprisCommand::Play);

    player.pause().await.unwrap();
    assert_eq!(next_command(&mut inbox).await, MprisCommand::Pause);

    player.next().await.unwrap();
    assert_eq!(next_command(&mut inbox).await, MprisCommand::Next);

    player.previous().await.unwrap();
    assert_eq!(next_command(&mut inbox).await, MprisCommand::Previous);

    player.stop().await.unwrap();
    assert_eq!(next_command(&mut inbox).await, MprisCommand::Stop);

    player.seek(-5_000_000).await.unwrap();
    assert_eq!(
        next_command(&mut inbox).await,
        MprisCommand::Seek(-5_000_000)
    );

    MediaPlayer2Proxy::new(&connection)
        .await
        .unwrap()
        .quit()
        .await
        .unwrap();
    assert_eq!(next_command(&mut inbox).await, MprisCommand::Quit);
}

#[tokio::test]
async fn the_volume_comes_back_as_a_percentage() {
    let _bus = take_turn().await;
    let Some((_service, mut inbox)) = service().await else {
        return;
    };

    let connection = zbus::Connection::session().await.unwrap();
    let player = PlayerProxy::new(&connection).await.unwrap();

    // The interface counts 0.0–1.0; Niveus counts 0–100.
    player.set_volume(0.42).await.unwrap();
    assert_eq!(next_command(&mut inbox).await, MprisCommand::SetVolume(42));
}

#[tokio::test]
async fn a_seek_to_another_track_is_refused() {
    let _bus = take_turn().await;
    let Some((service, mut inbox)) = service().await else {
        return;
    };
    service
        .update(Snapshot {
            status: PlaybackStatus::Playing,
            song: Some(song(7)),
            position_ms: 0,
            volume: 72,
        })
        .await
        .unwrap();

    let connection = zbus::Connection::session().await.unwrap();
    let player = PlayerProxy::new(&connection).await.unwrap();

    // The right track: accepted, in microseconds.
    player
        .set_position(
            ObjectPath::try_from("/org/niveus/track/s7").unwrap(),
            30_000_000,
        )
        .await
        .unwrap();
    assert_eq!(
        next_command(&mut inbox).await,
        MprisCommand::SetPosition(30_000_000)
    );

    // A track that is not playing: refused, and nothing is asked of the
    // application.
    let refused = player
        .set_position(ObjectPath::try_from("/org/niveus/track/s8").unwrap(), 0)
        .await;
    assert!(refused.is_err(), "a seek for another track was accepted");
    assert!(
        inbox.try_recv().is_err(),
        "the application was asked anyway"
    );
}

#[tokio::test]
async fn the_desktop_is_told_when_something_changes() {
    let _bus = take_turn().await;
    let Some((service, _inbox)) = service().await else {
        return;
    };

    let connection = zbus::Connection::session().await.unwrap();
    let player = PlayerProxy::new(&connection).await.unwrap();
    let mut changes = player.receive_playback_status_changed().await;

    service
        .update(Snapshot {
            status: PlaybackStatus::Paused,
            song: Some(song(1)),
            position_ms: 0,
            volume: 72,
        })
        .await
        .unwrap();

    let changed = tokio::time::timeout(std::time::Duration::from_secs(5), changes.next())
        .await
        .expect("a change is announced")
        .expect("the signal stream is open");
    assert_eq!(changed.get().await.unwrap(), "Paused");

    // A second update that changes nothing must not wake anybody: a progress
    // tick would otherwise reach the desktop several times a second.
    let mut quiet = player.receive_playback_status_changed().await;
    service
        .update(Snapshot {
            status: PlaybackStatus::Paused,
            song: Some(song(1)),
            position_ms: 30_000,
            volume: 72,
        })
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(400), quiet.next())
            .await
            .is_err(),
        "an unchanged status was announced again"
    );
}

#[tokio::test]
async fn a_second_instance_does_not_take_over_the_name() {
    let _bus = take_turn().await;
    let Some((_first, _inbox)) = service().await else {
        return;
    };

    let (commands, _second_inbox) = unbounded_channel();
    let second = MprisService::start(commands).await;
    assert!(
        second.is_err(),
        "a second instance claimed the name the first is serving"
    );
}

/// The bus name this test's proxies look for, checked so that a rename cannot
/// silently leave the tests talking to something else.
#[test]
fn the_service_name_is_the_one_the_desktop_expects() {
    assert_eq!(SERVICE_NAME, "org.mpris.MediaPlayer2.niveus");
}
