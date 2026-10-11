//! The control channel, exercised over a real socket against a scripted player.
//!
//! A stand-in is used rather than the real thing so that the interesting cases
//! — a refused command, a track cut short, a player that dies mid-session — can
//! be produced on demand instead of waited for.

use niveus_player::{Player, PlayerError, PlayerEvent};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

/// Long enough that a slow machine does not fail the test, short enough that a
/// genuine hang does not stall the run.
const PATIENCE: Duration = Duration::from_secs(5);

/// What the stand-in is told to do next.
enum Outgoing {
    Say(Value),
    Close,
}

/// A player that answers commands and says whatever the test asks it to say.
struct StandIn {
    socket: std::path::PathBuf,
    commands: mpsc::UnboundedReceiver<Vec<Value>>,
    outgoing: mpsc::UnboundedSender<Outgoing>,
    /// Command names the stand-in should refuse, as a real player refuses a
    /// property it does not have.
    refusing: Arc<Mutex<Vec<String>>>,
}

impl StandIn {
    async fn start(name: &str) -> (Self, Player, mpsc::Receiver<PlayerEvent>) {
        let socket =
            std::env::temp_dir().join(format!("niveus-player-{name}-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).unwrap();

        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (outgoing_tx, outgoing_rx) = mpsc::unbounded_channel();
        let refusing = Arc::new(Mutex::new(Vec::new()));

        let refusals = Arc::clone(&refusing);
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            serve(stream, command_tx, outgoing_rx, refusals).await;
        });

        let (event_tx, event_rx) = mpsc::channel(64);
        let player = Player::connect(&socket, event_tx).await.unwrap();

        (
            Self {
                socket,
                commands: command_rx,
                outgoing: outgoing_tx,
                refusing,
            },
            player,
            event_rx,
        )
    }

    /// The next command, whichever it is.
    async fn next_command(&mut self) -> Vec<Value> {
        tokio::time::timeout(PATIENCE, self.commands.recv())
            .await
            .expect("the player was sent a command")
            .expect("the stand-in is still running")
    }

    /// The next property the session asked to be kept informed about.
    async fn next_observation(&mut self) -> Vec<Value> {
        let command = self.next_command().await;
        assert_eq!(command[0], json!("observe_property"), "{command:?}");
        command
    }

    /// Waits for the command with this name, ignoring any others.
    async fn expect(&mut self, name: &str) -> Vec<Value> {
        loop {
            let command = self.next_command().await;
            if command.first().and_then(Value::as_str) == Some(name) {
                return command;
            }
        }
    }

    fn say(&self, message: Value) {
        self.outgoing.send(Outgoing::Say(message)).unwrap();
    }

    fn hang_up(&self) {
        self.outgoing.send(Outgoing::Close).unwrap();
    }

    fn refuse(&self, command: &str) {
        self.refusing.lock().unwrap().push(command.to_string());
    }
}

impl Drop for StandIn {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket);
    }
}

/// Reads commands, answers them, and forwards whatever the test queues.
async fn serve(
    stream: UnixStream,
    commands: mpsc::UnboundedSender<Vec<Value>>,
    mut outgoing: mpsc::UnboundedReceiver<Outgoing>,
    refusing: Arc<Mutex<Vec<String>>>,
) {
    let (read, mut write) = tokio::io::split(stream);
    let mut lines = BufReader::new(read).lines();

    loop {
        tokio::select! {
            incoming = lines.next_line() => match incoming {
                Ok(Some(line)) => {
                    let Ok(message) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    let Some(id) = message.get("request_id").and_then(Value::as_u64) else {
                        continue;
                    };
                    let command = message
                        .get("command")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    let name = command
                        .first()
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();

                    let _ = commands.send(command);

                    let refused = refusing.lock().unwrap().contains(&name);
                    let error = if refused { "property unavailable" } else { "success" };
                    let reply = json!({ "request_id": id, "error": error }).to_string();
                    if write.write_all(format!("{reply}\n").as_bytes()).await.is_err() {
                        return;
                    }
                    let _ = write.flush().await;
                }
                _ => return,
            },
            message = outgoing.recv() => match message {
                Some(Outgoing::Say(value)) => {
                    let mut line = value.to_string();
                    line.push('\n');
                    if write.write_all(line.as_bytes()).await.is_err() {
                        return;
                    }
                    let _ = write.flush().await;
                }
                Some(Outgoing::Close) | None => return,
            },
        }
    }
}

#[tokio::test]
async fn the_session_watches_the_properties_the_interface_needs() {
    // Progress is pushed as it changes rather than requested at the end, so a
    // player that stopped reporting it would simply look frozen.
    let (mut stand_in, _player, _events) = StandIn::start("observe").await;

    let mut watched = Vec::new();
    let mut ids = Vec::new();
    for _ in 0..4 {
        let command = stand_in.next_observation().await;
        ids.push(command[1].clone());
        watched.push(command[2].clone());
    }

    for property in ["time-pos", "duration", "pause", "volume"] {
        assert!(
            watched.contains(&json!(property)),
            "{property} is never watched: {watched:?}"
        );
    }
    // Each observation needs its own id, or the player watches one property
    // several times over and reports the rest never.
    let mut unique = ids.clone();
    unique.sort_by_key(|id| id.to_string());
    unique.dedup();
    assert_eq!(
        unique.len(),
        ids.len(),
        "an observation reuses an id: {ids:?}"
    );
}

#[tokio::test]
async fn a_command_reaches_the_player_with_its_arguments() {
    let (mut stand_in, player, _events) = StandIn::start("play").await;

    player.play("https://example.invalid/a.mp3").await.unwrap();

    let command = stand_in.expect("loadfile").await;
    assert_eq!(command[1], json!("https://example.invalid/a.mp3"));
    // Replacing rather than appending: asking for a track means that track.
    assert_eq!(command[2], json!("replace"));
}

#[tokio::test]
async fn asking_for_a_track_also_asks_for_it_to_play() {
    // The player keeps its settings across a change of file, and whether it was
    // paused is one of them. Without this, a track chosen while the last one
    // was paused loads, sits there, and never makes a sound.
    let (mut stand_in, player, _events) = StandIn::start("play-clears-pause").await;

    player.play("https://example.invalid/a.mp3").await.unwrap();

    stand_in.expect("loadfile").await;
    let resume = stand_in.expect("set_property").await;
    assert_eq!(resume[1], json!("pause"));
    assert_eq!(resume[2], json!(false), "the track was left paused");
}

#[tokio::test]
async fn seeking_and_volume_carry_their_values() {
    let (mut stand_in, player, _events) = StandIn::start("values").await;

    player.seek_to(42.5).await.unwrap();
    let seek = stand_in.expect("seek").await;
    assert_eq!(seek[1], json!(42.5));
    assert_eq!(seek[2], json!("absolute"));

    player.seek_by(-10.0).await.unwrap();
    let back = stand_in.expect("seek").await;
    assert_eq!(back[1], json!(-10.0));
    assert_eq!(back[2], json!("relative"));

    player.set_volume(80).await.unwrap();
    let volume = stand_in.expect("set_property").await;
    assert_eq!(volume[1], json!("volume"));
    assert_eq!(volume[2], json!(80));
}

#[tokio::test]
async fn the_volume_is_never_asked_for_beyond_its_range() {
    let (mut stand_in, player, _events) = StandIn::start("clamped").await;

    player.set_volume(200).await.unwrap();
    let volume = stand_in.expect("set_property").await;
    assert_eq!(volume[2], json!(100));
}

#[tokio::test]
async fn a_refused_command_is_reported_as_such() {
    let (stand_in, player, _events) = StandIn::start("refused").await;
    stand_in.refuse("seek");

    let error = player.seek_to(10.0).await.unwrap_err();
    assert!(matches!(error, PlayerError::Rejected(_)), "{error:?}");
    // A refusal is a normal answer, not a broken connection: the session goes on.
    assert!(player.pause().await.is_ok());
}

#[tokio::test]
async fn only_a_natural_end_is_forwarded() {
    let (stand_in, _player, mut events) = StandIn::start("end-file").await;

    // Every way a track can be interrupted, none of which may advance a queue.
    // (`error` is interruption too, but it is reported in its own right — see
    // `a_failed_track_says_why` — so it is not in this list.)
    for reason in ["stop", "quit", "redirect", "loadfile"] {
        stand_in.say(json!({ "event": "end-file", "reason": reason }));
    }
    // Then a real end, which is the one that may.
    stand_in.say(json!({ "event": "end-file", "reason": "eof" }));

    let event = tokio::time::timeout(PATIENCE, events.recv())
        .await
        .expect("an event arrives")
        .expect("the channel is open");
    assert_eq!(event, PlayerEvent::Ended);
}

#[tokio::test]
async fn an_end_before_the_file_loaded_is_silent() {
    // Nothing was playing, so a stop is not the end of anything.
    let (stand_in, _player, mut events) = StandIn::start("silent-end").await;
    stand_in.say(json!({ "event": "end-file", "reason": "stop" }));
    stand_in.say(json!({ "event": "file-loaded" }));

    let event = tokio::time::timeout(PATIENCE, events.recv())
        .await
        .expect("an event arrives")
        .expect("the channel is open");
    assert_eq!(event, PlayerEvent::Loaded);
}

#[tokio::test]
async fn progress_and_playback_state_are_reported() {
    let (stand_in, _player, mut events) = StandIn::start("progress").await;

    stand_in.say(json!({ "event": "property-change", "name": "time-pos", "data": 12.5 }));
    stand_in.say(json!({ "event": "property-change", "name": "duration", "data": 269.0 }));
    stand_in.say(json!({ "event": "property-change", "name": "pause", "data": true }));
    stand_in.say(json!({ "event": "property-change", "name": "volume", "data": 72.0 }));

    let mut received = Vec::new();
    for _ in 0..4 {
        received.push(
            tokio::time::timeout(PATIENCE, events.recv())
                .await
                .expect("an event arrives")
                .unwrap(),
        );
    }

    assert_eq!(
        received,
        vec![
            PlayerEvent::Position(12.5),
            PlayerEvent::Duration(269.0),
            PlayerEvent::Paused(true),
            PlayerEvent::Volume(72.0),
        ]
    );
}

#[tokio::test]
async fn a_failed_track_says_why() {
    let (stand_in, _player, mut events) = StandIn::start("failed").await;
    stand_in.say(json!({
        "event": "end-file",
        "reason": "error",
        "file_error": "Failed to open file",
    }));

    let event = tokio::time::timeout(PATIENCE, events.recv())
        .await
        .expect("an event arrives")
        .unwrap();
    // The player's own wording is English and written for a log; what the
    // status bar shows has to be something a listener can act on.
    let PlayerEvent::Failed(explanation) = event else {
        panic!("a failed track was not reported as such: {event:?}")
    };
    assert!(explanation.contains("cannot open"), "{explanation}");
}

#[tokio::test]
async fn a_player_that_goes_away_is_noticed_and_commands_stop_hanging() {
    let (stand_in, player, mut events) = StandIn::start("gone").await;

    stand_in.hang_up();

    let event = tokio::time::timeout(PATIENCE, events.recv())
        .await
        .expect("the loss is reported")
        .unwrap();
    assert_eq!(event, PlayerEvent::Exited);

    // The connection is gone, so a command cannot be honoured — and must say so
    // rather than wait for an answer that will never come.
    let error = player.pause().await.unwrap_err();
    assert!(matches!(error, PlayerError::Closed), "{error:?}");
}

#[tokio::test]
async fn a_line_that_is_not_an_object_is_skipped() {
    let (mut stand_in, player, _events) = StandIn::start("garbage").await;

    stand_in.say(json!("not an object"));
    stand_in.say(json!({ "event": "unknown-event" }));

    assert!(player.pause().await.is_ok());
    let command = stand_in.expect("set_property").await;
    assert_eq!(command[1], json!("pause"));
}

#[tokio::test]
async fn shutting_down_asks_the_player_to_quit_first() {
    // A player this facade did not start is not one it stops: `connect` leaves
    // the process running and only drops the connection.
    let (mut stand_in, player, _events) = StandIn::start("shutdown").await;

    player.shutdown().await;

    // The polite request was sent before the connection closed.
    let command = stand_in.expect("quit").await;
    assert_eq!(command[0], json!("quit"));
}
