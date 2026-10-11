//! Diagnostic for the sign-in handshake.
//!
//! Prints exactly what the service sends, which is what is needed when a scan
//! is accepted on the phone but the session is then not recognised.
//!
//! ```text
//! cargo run -p niveus-ncm-api --example session_probe            # inspect the saved cookie
//! cargo run -p niveus-ncm-api --example session_probe -- qr      # run a fresh scan
//! ```
//!
//! Cookie *names* are printed, never values, so the output is safe to share.

use ncm_api_rs::{ApiClient, Query};
use niveus_ncm_api::NcmApiClient;
use std::path::PathBuf;
use std::time::Duration;

const POLL: Duration = Duration::from_secs(2);

fn cookie_path() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("niveus").join("cookie")
}

fn names_of(cookie: &str) -> Vec<String> {
    cookie
        .split(';')
        .filter_map(|pair| {
            pair.split_once('=')
                .map(|(name, _)| name.trim().to_string())
        })
        .collect()
}

fn describe(label: &str, set_cookie_lines: &[String]) {
    println!("\n--- Set-Cookie received at {label} ---");
    if set_cookie_lines.is_empty() {
        println!("  (none)");
    }
    for line in set_cookie_lines {
        let first = line.split(';').next().unwrap_or(line);
        match first.split_once('=') {
            Some((name, value)) => {
                println!("  {name} = <{} chars>", value.trim().len())
            }
            None => println!("  {first}"),
        }
    }
}

async fn raw(
    label: &str,
    call: impl std::future::Future<Output = Result<ncm_api_rs::ApiResponse, ncm_api_rs::NcmError>>,
) {
    println!("\n================ {label} ================");
    match call.await {
        Ok(response) => {
            println!("status = {}", response.status);
            describe(label, &response.cookie);
            println!(
                "body = {}",
                serde_json::to_string_pretty(&response.body).unwrap_or_default()
            );
        }
        Err(error) => println!("error = {error}"),
    }
}

async fn qr_flow() -> Option<String> {
    let client = ApiClient::new(None);

    let key = client.login_qr_key(&Query::new()).await.ok()?.body["unikey"]
        .as_str()?
        .to_string();
    println!(">> unikey = {key}");
    println!(
        ">> scan this URL with the NetEase app:\n   https://music.163.com/login?codekey={key}"
    );

    loop {
        tokio::time::sleep(POLL).await;
        let response = match client
            .login_qr_check(&Query::new().param("key", &key))
            .await
        {
            Ok(response) => response,
            Err(error) => {
                println!("!! poll failed: {error}");
                return None;
            }
        };
        let code = response.body["code"].as_i64().unwrap_or(0);
        println!(">> poll code = {code}");

        match code {
            803 => {
                println!("\n================ 803 granted ================");
                println!(
                    "full body = {}",
                    serde_json::to_string_pretty(&response.body).unwrap_or_default()
                );

                println!("\n--- every Set-Cookie line, verbatim ---");
                for line in &response.cookie {
                    // Values are redacted; the attributes are what matter here.
                    let mut parts = line.split(';');
                    let first = parts.next().unwrap_or("");
                    let redacted = match first.split_once('=') {
                        Some((name, value)) => {
                            format!("{name} = <{} chars>", value.trim().len())
                        }
                        None => first.to_string(),
                    };
                    println!("  {redacted}");
                    for attribute in parts {
                        println!("      ;{}", attribute.trim());
                    }
                }

                let merged = merge(&response.cookie);
                println!("\n>> merged cookie names = {:?}", names_of(&merged));

                let path = cookie_path();
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                match std::fs::write(&path, &merged) {
                    Ok(()) => println!(">> saved to {}", path.display()),
                    Err(error) => println!("!! could not save: {error}"),
                }
                return Some(merged);
            }
            800 | 400 => {
                println!("!! code expired, request a new one");
                return None;
            }
            _ => {}
        }
    }
}

/// Reduces `Set-Cookie` lines to a `name=value` cookie string.
fn merge(lines: &[String]) -> String {
    lines
        .iter()
        .filter_map(|line| line.split(';').next())
        .map(str::trim)
        .filter(|pair| pair.contains('='))
        .collect::<Vec<_>>()
        .join("; ")
}

#[tokio::main]
async fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();

    let cookie = if mode == "qr" {
        match qr_flow().await {
            Some(cookie) => cookie,
            None => {
                eprintln!("!! no session obtained");
                return;
            }
        }
    } else {
        let path = cookie_path();
        match std::fs::read_to_string(&path) {
            Ok(cookie) if !cookie.trim().is_empty() => {
                println!(">> cookie from {}", path.display());
                cookie.trim().to_string()
            }
            _ => {
                eprintln!("!! no saved cookie at {}", path.display());
                eprintln!("   run with `-- qr` to sign in first");
                return;
            }
        }
    };

    println!(">> cookie names = {:?}", names_of(&cookie));
    println!(
        ">> has MUSIC_U = {}",
        cookie.split(';').any(|p| p.trim().starts_with("MUSIC_U="))
    );

    // The layering the app uses, so the reported result matches what the app
    // would see.
    let typed = NcmApiClient::with_cookie(&cookie);
    println!("\n================ typed login_status ================");
    match typed.login_status().await {
        Ok(Some(user)) => println!("signed in as {} ({})", user.name, user.id),
        Ok(None) => println!("the service does not recognise the session (profile is null)"),
        Err(error) => println!("error = {error}"),
    }

    // And the raw calls, for the exact payloads.
    let raw_client = ApiClient::new(Some(cookie.clone()));
    raw(
        "raw login_status (weapi)",
        raw_client.login_status(&Query::new()),
    )
    .await;

    // Which playlists exist, and whether any is large enough to need paging.
    let typed = NcmApiClient::with_cookie(&cookie);
    let uid = match typed.login_status().await {
        Ok(Some(user)) => user.id,
        _ => {
            eprintln!("!! cannot continue without a signed-in account");
            return;
        }
    };

    let playlists = match typed.user_playlists(uid).await {
        Ok(playlists) => playlists,
        Err(error) => {
            eprintln!("!! user_playlists failed: {error}");
            return;
        }
    };
    println!("\n================ playlists ================");
    println!("count = {}", playlists.len());
    let mut by_size: Vec<_> = playlists
        .iter()
        .map(|p| (p.track_count.unwrap_or(0), p.id, p.name.as_str()))
        .collect();
    by_size.sort_unstable_by_key(|entry| std::cmp::Reverse(entry.0));
    for (count, id, name) in by_size.iter().take(5) {
        println!("  {count:>6} tracks  [{id}] {name}");
    }

    // Paging: the largest playlist is fetched in two requests, which is the
    // path a playlist over a thousand tracks takes.
    if let Some((count, id, name)) = by_size.first().copied() {
        if count > 1000 {
            println!("\n================ paging {name} ================");
            let first = typed.playlist_tracks(id, 1000, 0).await;
            let second = typed.playlist_tracks(id, 1000, 1000).await;
            match (first, second) {
                (Ok(a), Ok(b)) => println!(
                    "page 1 = {}, page 2 = {} (declared {count})",
                    a.len(),
                    b.len()
                ),
                (a, b) => println!("paging failed: {:?} / {:?}", a.err(), b.err()),
            }
        } else {
            println!("\n(no playlist is over a thousand tracks, so paging stays untested)");
        }
    }
}
