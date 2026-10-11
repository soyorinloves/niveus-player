//! The control channel to the player process.
//!
//! Two kinds of message travel in each direction. Requests carry an id and are
//! answered with the same id; everything else is an event the player raised on
//! its own. Matching a reply to its request is what lets several commands be in
//! flight without the answers being confused for one another.
//!
//! The session is written against any byte stream rather than a socket, so the
//! whole protocol can be exercised over an in-memory pipe in tests.

use crate::error::PlayerError;
use crate::events::PlayerEvent;
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

/// How long a command waits for its answer before the player is presumed stuck.
const REPLY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Properties watched so the application stays in step with the player.
///
/// Progress in particular is pushed as it changes rather than asked for at the
/// end, so the position survives an abrupt exit.
const OBSERVED: [&str; 4] = ["time-pos", "duration", "pause", "volume"];

/// Replies owed to commands that are still in flight.
type Pending = HashMap<u64, oneshot::Sender<Result<(), PlayerError>>>;

/// How many unanswered commands are kept before the oldest are given up on.
///
/// A command that times out is answered at the caller, but its entry can only
/// be dropped here when the reply it is waiting for finally arrives — and a
/// player that has stopped answering altogether would never send one. The cap
/// is what keeps that from growing for as long as the program runs.
const MAX_PENDING: usize = 32;

/// A request from the facade to the session task.
pub(crate) enum Request {
    Command {
        args: Vec<Value>,
        reply: oneshot::Sender<Result<(), PlayerError>>,
    },
    /// Leave without draining anything still in flight.
    Close,
}

/// Builds a command message's argument list.
pub(crate) fn command(name: &str, arguments: impl IntoIterator<Item = Value>) -> Vec<Value> {
    let mut args = vec![json!(name)];
    args.extend(arguments);
    args
}

/// Runs the control channel until the stream or the requests end.
pub(crate) fn spawn<S>(
    stream: S,
    requests: mpsc::Receiver<Request>,
    events: mpsc::Sender<PlayerEvent>,
) -> JoinHandle<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    tokio::spawn(run(stream, requests, events))
}

async fn run<S>(stream: S, mut requests: mpsc::Receiver<Request>, events: mpsc::Sender<PlayerEvent>)
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let (read, mut write) = tokio::io::split(stream);
    let mut lines = BufReader::new(read).lines();
    let mut pending = Pending::new();
    let mut next_id: u64 = 1;

    // Ask for the properties the interface needs, before anything is playing:
    // the first position, duration or volume arrives without anyone asking.
    for (index, property) in OBSERVED.iter().enumerate() {
        let args = command("observe_property", [json!(index + 1), json!(property)]);
        if send(&mut write, &args, next_id).await.is_err() {
            finish(pending, &events).await;
            return;
        }
        next_id += 1;
    }

    loop {
        tokio::select! {
            incoming = lines.next_line() => match incoming {
                Ok(Some(line)) => {
                    if !on_message(&line, &mut pending, &events).await {
                        break;
                    }
                }
                // Either end of a closed pipe: the player is gone.
                Ok(None) | Err(_) => break,
            },
            request = requests.recv() => match request {
                Some(Request::Command { args, reply }) => {
                    let id = next_id;
                    next_id += 1;
                    match send(&mut write, &args, id).await {
                        Ok(()) => {
                            if pending.len() >= MAX_PENDING {
                                let abandoned = std::mem::take(&mut pending);
                                tracing::warn!(
                                    count = abandoned.len(),
                                    "the player has stopped answering; giving up on what is in flight"
                                );
                                for (_, reply) in abandoned {
                                    let _ = reply.send(Err(PlayerError::IpcProtocol(
                                        "播放器长时间未回应".to_string(),
                                    )));
                                }
                            }
                            pending.insert(id, reply);
                        }
                        Err(error) => {
                            let _ = reply.send(Err(PlayerError::IpcConnection(error)));
                            break;
                        }
                    }
                }
                Some(Request::Close) | None => break,
            },
        }
    }

    // Leaving the write half to drop closes the connection; a player that is
    // still running then notices and shuts itself down.
    drop(write);
    finish(pending, &events).await;
}

/// Answers everything still in flight and reports the player as gone.
///
/// Nothing may be left waiting: a caller parked on a reply that will never come
/// is a hung interface.
async fn finish(pending: Pending, events: &mpsc::Sender<PlayerEvent>) {
    for (_, reply) in pending {
        let _ = reply.send(Err(PlayerError::Closed));
    }
    let _ = events.send(PlayerEvent::Exited).await;
}

/// Handles one message; returns `false` when the session should stop.
async fn on_message(line: &str, pending: &mut Pending, events: &mpsc::Sender<PlayerEvent>) -> bool {
    let Ok(message) = serde_json::from_str::<Value>(line) else {
        // A line that is not an object is not worth ending the session over,
        // but a run of them means the two ends disagree about the protocol,
        // which is impossible to diagnose without the line itself.
        tracing::warn!(line, "ignoring a line that is not a message");
        return true;
    };

    // A reply, which belongs to whichever command is waiting under that id.
    if let Some(id) = message.get("request_id").and_then(Value::as_u64) {
        if let Some(reply) = pending.remove(&id) {
            let _ = reply.send(verdict(&message));
        }
        return true;
    }

    match parse_event(&message) {
        Some(event) => events.send(event).await.is_ok(),
        None => true,
    }
}

/// Whether a reply reports success.
fn verdict(message: &Value) -> Result<(), PlayerError> {
    match message.get("error").and_then(Value::as_str) {
        Some("success") => Ok(()),
        Some(other) => Err(PlayerError::Rejected(other.to_string())),
        None => Err(PlayerError::IpcProtocol("回执缺少 error 字段".to_string())),
    }
}

/// Turns a message into an event, when it is one the application cares about.
pub(crate) fn parse_event(message: &Value) -> Option<PlayerEvent> {
    match message.get("event")?.as_str()? {
        "file-loaded" => Some(PlayerEvent::Loaded),
        "end-file" => match message.get("reason").and_then(Value::as_str) {
            // The only reason that means the track played to its end.
            Some("eof") => Some(PlayerEvent::Ended),
            Some("error") => Some(PlayerEvent::Failed(explain(message))),
            // `stop`, `quit`, `redirect` and `loadfile` all mean the track was
            // interrupted on purpose: seeking to another track or replacing it
            // must not read as having finished, or the queue would advance on
            // every manual change.
            _ => None,
        },
        "property-change" => {
            let name = message.get("name")?.as_str()?;
            let data = message.get("data")?;
            // A property with no value yet (nothing playing) reports null, which
            // is simply not an update.
            Some(match name {
                "time-pos" => PlayerEvent::Position(data.as_f64()?),
                "duration" => PlayerEvent::Duration(data.as_f64()?),
                "pause" => PlayerEvent::Paused(data.as_bool()?),
                "volume" => PlayerEvent::Volume(data.as_f64()?),
                _ => return None,
            })
        }
        _ => None,
    }
}

/// What went wrong with the file, in words the user can act on.
///
/// The player's own wording is English and written for a log rather than for
/// someone looking at a status bar; the raw text is still what reaches the log
/// through the caller. Anything not recognised is passed through unchanged,
/// because a strange message is more use than no message.
fn explain(message: &Value) -> String {
    let raw = message
        .get("error")
        .and_then(Value::as_str)
        .or_else(|| message.get("file_error").and_then(Value::as_str))
        .unwrap_or_default();

    match raw {
        "" => "无法播放该文件".to_string(),
        // The usual causes, in the order a listener is likely to meet them.
        "Failed to open file" => "无法打开音频文件，可能已过期或无权限".to_string(),
        "loading failed" => "音源加载失败，可能网络不通或该曲目受限".to_string(),
        "unrecognized file format" => "音频格式无法识别".to_string(),
        _ => raw.to_string(),
    }
}

async fn send<W: AsyncWrite + Unpin>(
    write: &mut W,
    args: &[Value],
    id: u64,
) -> std::io::Result<()> {
    let message = json!({ "command": args, "request_id": id });
    let mut line = serde_json::to_string(&message).expect("a command is always serialisable");
    line.push('\n');
    write.write_all(line.as_bytes()).await?;
    write.flush().await
}

/// Sends a command and waits for its answer.
pub(crate) async fn request(
    requests: &mpsc::Sender<Request>,
    args: Vec<Value>,
) -> Result<(), PlayerError> {
    let (reply, answer) = oneshot::channel();
    requests
        .send(Request::Command { args, reply })
        .await
        .map_err(|_| PlayerError::Closed)?;
    match tokio::time::timeout(REPLY_TIMEOUT, answer).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(PlayerError::Closed),
        Err(_) => Err(PlayerError::IpcProtocol("播放器未在超时内回应".to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_carries_its_name_first() {
        let args = command("loadfile", [json!("https://example.invalid/a.mp3")]);
        assert_eq!(args[0], json!("loadfile"));
        assert_eq!(args[1], json!("https://example.invalid/a.mp3"));
        assert_eq!(args.len(), 2);
    }

    #[test]
    fn only_a_natural_end_is_reported_as_ended() {
        for reason in ["stop", "quit", "error", "redirect", "loadfile"] {
            let message = json!({ "event": "end-file", "reason": reason });
            assert!(
                !matches!(parse_event(&message), Some(PlayerEvent::Ended)),
                "{reason} was reported as a natural end"
            );
        }
        let message = json!({ "event": "end-file", "reason": "eof" });
        assert_eq!(parse_event(&message), Some(PlayerEvent::Ended));
    }

    #[test]
    fn a_failed_file_is_reported_rather_than_silently_ignored() {
        let message = json!({
            "event": "end-file",
            "reason": "error",
            "file_error": "Failed to open file",
        });
        // The common failures arrive as English log text; what reaches the
        // status bar has to be something a listener can act on.
        let Some(PlayerEvent::Failed(explanation)) = parse_event(&message) else {
            panic!("a failed file was not reported")
        };
        assert!(explanation.contains("无法打开"), "{explanation}");
    }

    #[test]
    fn a_failure_nobody_recognises_is_passed_through_rather_than_replaced() {
        // A strange message is more use than none at all.
        let message = json!({
            "event": "end-file",
            "reason": "error",
            "error": "something entirely new",
        });
        assert_eq!(
            parse_event(&message),
            Some(PlayerEvent::Failed("something entirely new".to_string()))
        );

        // And a failure with no detail at all still says something.
        let message = json!({ "event": "end-file", "reason": "error" });
        assert_eq!(
            parse_event(&message),
            Some(PlayerEvent::Failed("无法播放该文件".to_string()))
        );
    }

    #[test]
    fn the_watched_properties_become_events() {
        let cases = [
            (json!("time-pos"), json!(12.5), PlayerEvent::Position(12.5)),
            (
                json!("duration"),
                json!(269.0),
                PlayerEvent::Duration(269.0),
            ),
            (json!("pause"), json!(true), PlayerEvent::Paused(true)),
            (json!("volume"), json!(72.0), PlayerEvent::Volume(72.0)),
        ];
        for (name, data, expected) in cases {
            let message = json!({ "event": "property-change", "name": name, "data": data });
            assert_eq!(parse_event(&message), Some(expected));
        }
    }

    #[test]
    fn an_unset_property_is_not_an_update() {
        // Nothing is playing, so the player reports the position as null.
        let message = json!({ "event": "property-change", "name": "time-pos", "data": null });
        assert_eq!(parse_event(&message), None);
    }

    #[test]
    fn unrelated_messages_are_ignored() {
        assert_eq!(parse_event(&json!({"event": "idle"})), None);
        assert_eq!(parse_event(&json!({"event": "seek"})), None);
        assert_eq!(
            parse_event(&json!({"request_id": 3, "error": "success"})),
            None
        );
    }

    #[test]
    fn a_reply_is_read_as_success_or_refusal() {
        assert!(verdict(&json!({"request_id": 1, "error": "success"})).is_ok());
        assert!(matches!(
            verdict(&json!({"request_id": 1, "error": "property unavailable"})),
            Err(PlayerError::Rejected(_))
        ));
        assert!(matches!(
            verdict(&json!({"request_id": 1})),
            Err(PlayerError::IpcProtocol(_))
        ));
    }
}
