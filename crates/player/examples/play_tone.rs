//! Plays a tone through the real audio output and prints what the player
//! reports, so playback can be heard and followed without the interface.
//!
//! ```text
//! cargo run -p niveus-player --example play_tone            # a generated tone
//! cargo run -p niveus-player --example play_tone -- <file>  # any local file
//! cargo run -p niveus-player --example play_tone -- <file> --silent
//! ```
//!
//! This is the audible check for the player crate: if you hear a tone and then
//! see `Ended`, the process, the socket, the protocol and the end-of-file
//! report are all working together.

use niveus_player::{LaunchOptions, Player, PlayerEvent};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

#[tokio::main]
async fn main() -> ExitCode {
    if niveus_player::player_path().is_none() {
        eprintln!("未找到 mpv，请用你的发行版包管理器安装 mpv。");
        return ExitCode::FAILURE;
    }

    let mut args = std::env::args().skip(1);
    let path = args.next().map(PathBuf::from);
    let silent = args.any(|arg| arg == "--silent");

    let scratch = std::env::temp_dir().join(format!("niveus-play-tone-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    let socket = scratch.join("mpv.sock");

    let source = match &path {
        Some(path) => path.clone(),
        None => {
            let tone = scratch.join("tone.wav");
            write_tone(&tone, 2);
            tone
        }
    };

    println!("socket  : {}", socket.display());
    println!("playing : {}", source.display());
    println!(
        "output  : {}",
        if silent {
            "静默 (--ao=null)"
        } else {
            "扬声器"
        }
    );
    println!();

    let (event_tx, mut events) = tokio::sync::mpsc::channel(64);
    let player = match Player::launch(
        &socket,
        LaunchOptions {
            silent,
            volume: Some(70),
        },
        event_tx,
    )
    .await
    {
        Ok(player) => player,
        Err(error) => {
            eprintln!("无法启动播放器：{error}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(error) = player.play(source.to_str().unwrap_or_default()).await {
        eprintln!("无法播放：{error}");
        player.shutdown().await;
        return ExitCode::FAILURE;
    }

    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut ended = false;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let Ok(Some(event)) = tokio::time::timeout(remaining, events.recv()).await else {
            break;
        };
        println!("{event:?}");
        if event == PlayerEvent::Ended {
            ended = true;
            break;
        }
        if event == PlayerEvent::Exited {
            break;
        }
    }

    println!();
    if ended {
        println!("播放到自然结束（end-file reason=eof）");
    } else {
        println!("未在超时内结束");
    }

    player.shutdown().await;
    println!("socket 已清理：{}", !socket.exists());

    let _ = std::fs::remove_dir_all(&scratch);
    if ended {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
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
