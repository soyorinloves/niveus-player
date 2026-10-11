//! End-to-end check of the provider: sign in, list playlists, load one, resolve
//! a playable URL and fetch lyrics.
//!
//! Headless — it prints the QR payload rather than drawing a code. Sign in once
//! through the TUI, and this then runs against the saved session:
//!
//! ```text
//! cargo run -p niveus-ncm-provider --example login_flow
//! cargo run -p niveus-ncm-provider --example login_flow -- /path/to/cookie
//! cargo run -p niveus-ncm-provider --example login_flow -- /path/to/cookie <playlist-id>
//! ```
//!
//! Given a playlist id it fetches that one instead of the first, which is how
//! a playlist long enough to need paging gets exercised.

use niveus_music_core::{AudioQuality, PlaylistId, PlaylistKind};
use niveus_ncm_provider::{MusicProvider, NcmProvider, QrLoginState};
use std::path::PathBuf;
use std::time::Duration;

/// How long to wait between QR polls.
const POLL_INTERVAL: Duration = Duration::from_secs(3);
/// Give up on a QR sign-in after this long so a headless run cannot hang.
const QR_TIMEOUT: Duration = Duration::from_secs(120);

/// `$XDG_STATE_HOME/niveus/cookie`, falling back to `~/.local/state/niveus`.
fn default_cookie_path() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("niveus").join("cookie")
}

fn save_cookie(path: &PathBuf, cookie: &str) {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("!! 无法创建状态目录 {}: {e}", parent.display());
            return;
        }
    }
    if let Err(e) = write_private(path, cookie) {
        eprintln!("!! 无法保存 cookie 到 {}: {e}", path.display());
    } else {
        println!(">> 登录态已保存到 {}", path.display());
    }
}

/// Writes the session cookie with owner-only permissions.
fn write_private(path: &PathBuf, contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(contents.as_bytes())
}

async fn sign_in(provider: &NcmProvider) -> bool {
    let deadline = tokio::time::Instant::now() + QR_TIMEOUT;

    loop {
        let login = match provider.start_qr_login().await {
            Ok(login) => login,
            Err(e) => {
                eprintln!("!! 无法获取二维码: {e}");
                return false;
            }
        };
        println!(
            "\n请扫描以下二维码完成登录（用网易云音乐 App）:\n  {}",
            login.url
        );
        println!("（此示例不绘制二维码图形，可在任意二维码生成器里编码上面的 URL）");

        loop {
            if tokio::time::Instant::now() >= deadline {
                eprintln!("!! 等待扫码超时");
                return false;
            }
            match provider.poll_qr_login(&login.key).await {
                Ok(QrLoginState::Waiting) => {}
                Ok(QrLoginState::Scanned) => println!(">> 已扫码，请在手机上确认"),
                Ok(QrLoginState::Confirmed) => {
                    println!(">> 登录成功");
                    return true;
                }
                Ok(QrLoginState::Expired) => {
                    println!(">> 二维码已过期，重新生成");
                    break;
                }
                Err(e) => {
                    eprintln!("!! 轮询失败: {e}");
                    return false;
                }
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cookie_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(default_cookie_path);

    let provider = match std::fs::read_to_string(&cookie_path) {
        Ok(cookie) if !cookie.trim().is_empty() => {
            println!(">> 使用已保存的登录态: {}", cookie_path.display());
            NcmProvider::with_cookie(cookie.trim())
        }
        _ => {
            println!(">> 未找到登录态 ({}), 走扫码流程", cookie_path.display());
            NcmProvider::anonymous()
        }
    };

    match provider.restore_session().await {
        Ok(Some(user)) => println!(">> 已登录: {} ({})", user.name, user.id),
        Ok(None) => {
            println!(">> 登录态无效或不存在");
            if !sign_in(&provider).await {
                anyhow::bail!("登录失败");
            }
            let user = provider.finish_qr_login().await?;
            println!(">> 已登录: {} ({})", user.name, user.id);
            save_cookie(&cookie_path, &provider.cookie());
        }
        Err(e) => anyhow::bail!("恢复登录态失败: {e}"),
    }

    // --- playlists -------------------------------------------------------
    let playlists = provider.get_user_playlists().await?;
    let created: Vec<_> = playlists
        .iter()
        .filter(|p| p.kind == PlaylistKind::Created)
        .collect();
    let collected: Vec<_> = playlists
        .iter()
        .filter(|p| p.kind == PlaylistKind::Collected)
        .collect();

    println!("\n=== 歌单 ({}) ===", playlists.len());
    println!("CREATED ({})", created.len());
    for p in created.iter().take(5) {
        println!("  [{}] {}", p.id, p.name);
    }
    println!("COLLECTED ({})", collected.len());
    for p in collected.iter().take(5) {
        println!("  [{}] {}", p.id, p.name);
    }

    // --- liked songs -----------------------------------------------------
    match provider.get_liked_songs().await {
        Ok(liked) => {
            println!("\n=== FAVORITES ({} 首) ===", liked.len());
            for song in liked.iter().take(3) {
                println!("  {}  —  {}", song.title, song.artist_names());
            }
        }
        Err(e) => println!("\n!! 获取喜欢的歌曲失败: {e}"),
    }

    // --- songs in one playlist ------------------------------------------
    let requested: Option<PlaylistId> = std::env::args()
        .nth(2)
        .and_then(|raw| raw.parse::<u64>().ok())
        .map(PlaylistId);

    let chosen = match requested {
        Some(id) => playlists.iter().find(|p| p.id == id),
        None => playlists
            .iter()
            .find(|p| p.kind == PlaylistKind::Created)
            .or_else(|| playlists.iter().find(|p| p.kind == PlaylistKind::Collected)),
    };

    let Some(first) = chosen else {
        println!("\n(没有可浏览的歌单，跳过歌曲检查)");
        return Ok(());
    };

    let songs = provider.get_playlist_songs(first.id).await?;
    println!("\n=== 「{}」 共 {} 首 ===", first.name, songs.len());
    for (i, song) in songs.iter().take(5).enumerate() {
        println!(
            "  {:02}  {}  —  {}  ({})",
            i + 1,
            song.title,
            song.artist_names(),
            song.duration_ms
                .map(|ms| format!("{:02}:{:02}", ms / 60_000, (ms / 1000) % 60))
                .unwrap_or_else(|| "—".into())
        );
    }

    let Some(song) = songs.first() else {
        println!("\n(歌单为空，跳过音源检查)");
        return Ok(());
    };

    // --- audio url ------------------------------------------------------
    println!("\n=== 音源: {} ===", song.title);
    match provider.get_song_url(song.id, AudioQuality::Exhigh).await {
        Ok(source) => {
            println!(
                "  level={:?} format={:?} bitrate={:?} trial={}",
                source.level, source.format, source.bitrate, source.is_trial
            );
            println!("  url={}", source.url);
        }
        Err(e) => println!("  不可播放: {e}"),
    }

    // --- lyrics ---------------------------------------------------------
    println!("\n=== 歌词: {} ===", song.title);
    let lyrics = provider.get_lyrics(song.id).await?;
    match lyrics.body() {
        Some(text) => {
            println!("  原文 {} 行，前 5 行：", text.lines().count());
            for line in text.lines().take(5) {
                println!("  {line}");
            }
        }
        // Which of the two it is decides what the panel says (SPEC §13.4).
        None => println!("  {lyrics:?}"),
    }

    Ok(())
}
