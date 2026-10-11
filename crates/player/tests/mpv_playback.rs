//! Playback against a real player process.
//!
//! The scripted tests pin the protocol down; this one pins down that the
//! protocol is the right one — that a real player, started the way the
//! application starts it, decodes a track and reports its end.
//!
//! The tone is generated rather than downloaded so the test needs nothing from
//! the network, and the sound is routed away from the speakers so it needs
//! nothing from the sound card either.

use niveus_player::{LaunchOptions, Player, PlayerEvent};
use std::path::PathBuf;
use std::time::Duration;

/// Generous: a loaded machine can take a while to start a process.
const PATIENCE: Duration = Duration::from_secs(20);
/// Length of the generated tone.
const TONE_SECONDS: u32 = 1;

#[tokio::test]
async fn a_real_track_plays_through_to_its_end() {
    if niveus_player::player_path().is_none() {
        eprintln!("mpv 未安装，跳过真实播放测试");
        return;
    }

    let scratch = std::env::temp_dir().join(format!("niveus-mpv-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    let socket = scratch.join("mpv.sock");
    let tone = scratch.join("tone.wav");
    write_tone(&tone, TONE_SECONDS);

    let (event_tx, mut events) = tokio::sync::mpsc::channel(64);
    let player = Player::launch(
        &socket,
        LaunchOptions {
            silent: true,
            volume: Some(50),
        },
        event_tx,
    )
    .await
    .expect("a real player starts");

    player.play(tone.to_str().unwrap()).await.unwrap();

    let mut seen = Vec::new();
    let deadline = tokio::time::Instant::now() + PATIENCE;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let Ok(Some(event)) = tokio::time::timeout(remaining, events.recv()).await else {
            break;
        };
        let done = event == PlayerEvent::Ended;
        seen.push(event);
        if done {
            break;
        }
    }

    // The track was decoded...
    assert!(
        seen.contains(&PlayerEvent::Loaded),
        "the track never loaded: {seen:?}"
    );
    // ...the position was pushed while it played, rather than asked for at the
    // end...
    assert!(
        seen.iter().any(|e| matches!(e, PlayerEvent::Position(_))),
        "no progress was reported: {seen:?}"
    );
    // ...and it reached its natural end.
    assert!(
        seen.iter().any(|e| matches!(e, PlayerEvent::Ended)),
        "playback did not finish: {seen:?}"
    );

    player.shutdown().await;
    assert!(!socket.exists(), "the socket was left behind");

    std::fs::remove_dir_all(&scratch).unwrap();
}

#[tokio::test]
async fn a_track_that_is_replaced_does_not_report_an_end() {
    // The behaviour the queue depends on: changing track is not finishing one.
    if niveus_player::player_path().is_none() {
        eprintln!("mpv 未安装，跳过真实播放测试");
        return;
    }

    let scratch = std::env::temp_dir().join(format!("niveus-mpv-swap-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    let socket = scratch.join("mpv.sock");
    let first = scratch.join("first.wav");
    let second = scratch.join("second.wav");
    write_tone(&first, 30);
    write_tone(&second, 30);

    let (event_tx, mut events) = tokio::sync::mpsc::channel(64);
    let player = Player::launch(
        &socket,
        LaunchOptions {
            silent: true,
            ..Default::default()
        },
        event_tx,
    )
    .await
    .unwrap();

    player.play(first.to_str().unwrap()).await.unwrap();
    // Let the first track actually start before cutting it off.
    wait_for(&mut events, |event| *event == PlayerEvent::Loaded).await;
    player.play(second.to_str().unwrap()).await.unwrap();
    wait_for(&mut events, |event| *event == PlayerEvent::Loaded).await;

    // Whatever arrived in between, none of it was a natural end. Bounded by the
    // clock rather than by the channel going quiet: progress keeps arriving for
    // as long as the track plays.
    let until = tokio::time::Instant::now() + Duration::from_millis(800);
    while let Ok(Some(event)) = tokio::time::timeout_at(until, events.recv()).await {
        assert_ne!(
            event,
            PlayerEvent::Ended,
            "a replaced track reported an end"
        );
    }

    player.shutdown().await;
    std::fs::remove_dir_all(&scratch).unwrap();
}

/// Waits for an event matching `predicate`, panicking if it never comes.
async fn wait_for(
    events: &mut tokio::sync::mpsc::Receiver<PlayerEvent>,
    predicate: impl Fn(&PlayerEvent) -> bool,
) {
    let deadline = tokio::time::Instant::now() + PATIENCE;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let event = tokio::time::timeout(remaining, events.recv())
            .await
            .expect("the expected event arrives")
            .expect("the channel stays open");
        if predicate(&event) {
            return;
        }
    }
}

/// Writes a mono 16-bit PCM tone, faded in and out so it does not click.
fn write_tone(path: &PathBuf, seconds: u32) {
    const RATE: u32 = 44_100;
    const FREQUENCY: f64 = 440.0;
    const FADE: u32 = 441; // 10 ms

    let samples = RATE * seconds;
    let mut pcm = Vec::with_capacity((samples * 2) as usize);
    for index in 0..samples {
        let envelope = (index.min(FADE) as f64 / FADE as f64)
            .min((samples - index).min(FADE) as f64 / FADE as f64);
        let phase = 2.0 * std::f64::consts::PI * FREQUENCY * index as f64 / RATE as f64;
        let value = (phase.sin() * envelope * 0.3 * i16::MAX as f64) as i16;
        pcm.extend_from_slice(&value.to_le_bytes());
    }

    let mut file = Vec::new();
    file.extend_from_slice(b"RIFF");
    file.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    file.extend_from_slice(b"WAVE");
    file.extend_from_slice(b"fmt ");
    file.extend_from_slice(&16u32.to_le_bytes());
    file.extend_from_slice(&1u16.to_le_bytes()); // PCM
    file.extend_from_slice(&1u16.to_le_bytes()); // mono
    file.extend_from_slice(&RATE.to_le_bytes());
    file.extend_from_slice(&(RATE * 2).to_le_bytes()); // byte rate
    file.extend_from_slice(&2u16.to_le_bytes()); // block align
    file.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    file.extend_from_slice(b"data");
    file.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    file.extend_from_slice(&pcm);

    std::fs::write(path, file).unwrap();
}
