//! Prints the default cover, and optionally a real one, as plain text.
//!
//! ```text
//! cargo run -p niveus-artwork --example covers
//! cargo run -p niveus-artwork --example covers -- <cover-url>
//! cargo run -p niveus-artwork --example covers -- <cover-url> 24 12
//! ```
//!
//! Colours are not printed — this is for judging the shape and the placement,
//! which is what is hard to get right and easy to check here.

use niveus_artwork::{
    load, render_default, ArtworkMode, ColourDepth, CoverKey, Frame, Options, Size,
};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if let Some(url) = args.first() {
        let cols = args.get(1).and_then(|v| v.parse().ok()).unwrap_or(40);
        let rows = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(12);
        preview_cover(url, Size::new(cols, rows)).await;
        return;
    }

    for size in [
        Size::new(53, 12),
        Size::new(31, 9),
        Size::new(24, 7),
        Size::new(16, 5),
        Size::new(11, 3),
        Size::new(4, 1),
    ] {
        show(
            &format!("default · {}×{}", size.cols, size.rows),
            render_default(size),
        );
    }
}

async fn preview_cover(url: &str, area: Size) {
    for (mode, depth) in [
        (ArtworkMode::HalfBlock, ColourDepth::TrueColor),
        (ArtworkMode::Characters, ColourDepth::TrueColor),
        (ArtworkMode::HalfBlock, ColourDepth::Monochrome),
    ] {
        let key = CoverKey::new(url, area, Options { mode, depth });
        match load(&key).await {
            Ok(frame) => show(
                &format!("{mode:?} · {depth:?} · {}×{}", area.cols, area.rows),
                frame,
            ),
            Err(error) => eprintln!("{mode:?} · {depth:?} 失败：{error}"),
        }
    }
}

fn show(title: &str, frame: Frame) {
    println!("┄┄ {title} · 实际 {}×{} ┄┄", frame.width(), frame.height());
    for line in frame.to_string().lines() {
        println!("|{line}|");
    }
    println!();
}
